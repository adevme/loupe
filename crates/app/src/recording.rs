use std::path::{Path, PathBuf};
use std::sync::Arc;

use iced::futures::channel::oneshot;
use iced::Task;
use loupe_engine::{bar_frames, Command, CommandError, Frames, Input, Outcome, Source, TrackId};

use crate::selection::TAKES_FOLDER;
use crate::{App, Message, SETTLE_TICKS};

const UNNAMED_TAKE: &str = "Take";

pub struct Recording {
    pub from: Frames,
    tracks: Vec<TrackId>,
    counted_in: bool,
}

impl App {
    pub(crate) fn toggle_recording(&mut self) -> Task<Message> {
        if self.recording.is_some() {
            return self.finish_recording();
        }
        let tracks: Vec<TrackId> =
            self.project.tracks.iter().map(|track| track.id).filter(|track| self.armed.contains(track)).collect();
        let (Some(first), Some(input)) = (tracks.first(), &self.input) else {
            self.notice = Some("Arm a track to record: the dot beside M.".into());
            return Task::none();
        };
        let Some(folder) = self.path.as_deref().and_then(Path::parent).map(|project| project.join(TAKES_FOLDER)) else {
            let asked = self.save_as();
            self.entry_problem = Some("Save the project first. Takes are kept in its Audio folder.".into());
            return asked;
        };
        let track_name = self.project.track(*first).map(|track| crate::files::file_safe(&track.name)).unwrap_or_default();
        let file = unused_take(&folder, if track_name.is_empty() { UNNAMED_TAKE } else { &track_name });
        if let Err(why) = input.begin_take(&file) {
            self.problem = Some(format!("Could not start recording: {why}"));
            return Task::none();
        }
        self.problem = None;
        self.notice = None;
        self.engine.set_endless(true);
        let counted_in = !self.playing && self.count_in_bars > 0;
        if !self.playing {
            if let Some((from, _)) = self.loop_range {
                self.seek(from);
            }
            if counted_in {
                self.engine.count_in(bar_frames(self.project.bpm, self.project.rate) * self.count_in_bars as Frames);
            }
            self.engine.play();
            self.playing = true;
        }
        self.recording = Some(Recording { from: self.playhead, tracks, counted_in });
        Task::none()
    }

    pub(crate) fn finish_recording(&mut self) -> Task<Message> {
        let Some(recording) = self.recording.take() else {
            return Task::none();
        };
        let began = self.input.as_ref().and_then(Input::take_began).map(|at| self.engine.position_at(at));
        self.engine.stop();
        self.engine.set_endless(false);
        self.playing = false;
        self.settle = SETTLE_TICKS;
        self.cache.clear();
        let finished = match &self.input {
            Some(input) => input.finish_take(),
            None => Err("the recording input closed".to_string()),
        };
        let take = match finished {
            Ok(take) => take,
            Err(why) => {
                self.problem = Some(format!("Could not keep the take: {why}"));
                return Task::none();
            }
        };
        let (Some(start), true) = (began, take.frames > 0) else {
            let _ = std::fs::remove_file(&take.path);
            self.notice = Some("Nothing was recorded.".into());
            return Task::none();
        };
        let warning = (take.lost > 0).then(|| {
            let lost_ms = take.lost * 1000 / take.rate.max(1) as u64;
            format!("The disk fell behind: about {lost_ms} ms of the take were lost.")
        });
        let rate = self.project.rate;
        let tracks = recording.tracks;
        let keep_from = if recording.counted_in { recording.from as i64 } else { 0 };
        self.loading += 1;
        let (done, loaded) = oneshot::channel();
        std::thread::spawn(move || {
            let _ = done.send(Source::load(&take.path, rate).map(Arc::new));
        });
        Task::perform(
            async move { loaded.await.unwrap_or_else(|_| Err("reading the take stopped unexpectedly".into())) },
            move |result| Message::TakeReady { tracks: tracks.clone(), start, keep_from, warning: warning.clone(), result },
        )
    }

    pub(crate) fn place_take(&mut self, tracks: Vec<TrackId>, start: i64, keep_from: i64, result: Result<Arc<Source>, String>) {
        self.loading = self.loading.saturating_sub(1);
        let source = match result {
            Ok(source) => source,
            Err(why) => {
                self.problem = Some(format!("Could not read the take: {why}"));
                return;
            }
        };
        let cut = (keep_from.max(0) - start).max(0) as Frames;
        let whole = source.frames.len() as Frames;
        if cut >= whole {
            return;
        }
        let start = (start + cut as i64) as Frames;
        let mut placed = Vec::new();
        self.transact(None, |project| {
            for track in tracks {
                if project.track(track).is_none() {
                    continue;
                }
                let Outcome::Clip(clip) = project.apply(Command::AddClip { track, source: source.clone(), start })? else {
                    return Err(CommandError::NoSuchClip);
                };
                if cut > 0 {
                    project.apply(Command::TrimClip { clip, offset: cut, len: whole - cut })?;
                }
                placed.push(clip);
            }
            Ok(Outcome::Done)
        });
        if !placed.is_empty() {
            self.choose(placed);
        }
    }
}

fn unused_take(folder: &Path, name: &str) -> PathBuf {
    (1..)
        .map(|number| folder.join(format!("{name} (take {number}).wav")))
        .find(|file| !file.exists())
        .unwrap_or_else(|| folder.join(format!("{name}.wav")))
}
