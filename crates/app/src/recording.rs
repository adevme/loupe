use std::path::{Path, PathBuf};
use std::sync::Arc;

use iced::futures::channel::oneshot;
use iced::Task;
use loupe_engine::{bar_frames, Command, CommandError, Frames, Input, Note, Outcome, Source, TapedKey, TrackId};

use crate::selection::TAKES_FOLDER;
use crate::{App, Message, SETTLE_TICKS};

const UNNAMED_TAKE: &str = "Take";

pub struct Recording {
    pub from: Frames,
    tracks: Vec<TrackId>,
    note_tracks: Vec<TrackId>,
    pub taped: Vec<TapedKey>,
    counted_in: bool,
}

impl App {
    pub(crate) fn toggle_recording(&mut self) -> Task<Message> {
        if self.recording.is_some() {
            return self.finish_recording();
        }
        let armed: Vec<&loupe_engine::Track> = self.project.tracks.iter().filter(|track| self.armed.contains(&track.id)).collect();
        let tracks: Vec<TrackId> = armed.iter().filter(|track| !track.records_notes).map(|track| track.id).collect();
        let note_tracks: Vec<TrackId> = armed.iter().filter(|track| track.records_notes).map(|track| track.id).collect();
        if tracks.is_empty() && note_tracks.is_empty() {
            self.notice = Some("Arm a track to record: the dot beside M.".into());
            return Task::none();
        }
        if let Some(first) = tracks.first() {
            let Some(input) = &self.input else {
                self.notice = Some("The recording input is not open. Check Settings > Recording.".into());
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
        }
        if !note_tracks.is_empty() {
            self.engine.taped_keys();
            self.engine.tape_keys(true);
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
        self.recording = Some(Recording { from: self.playhead, tracks, note_tracks, taped: Vec::new(), counted_in });
        self.listen_if_armed();
        Task::none()
    }

    pub(crate) fn finish_recording(&mut self) -> Task<Message> {
        let Some(recording) = self.recording.take() else {
            return Task::none();
        };
        let stopped_at = self.engine.position();
        self.engine.hear_on(&[]);
        self.engine.tape_keys(false);
        let began = self.input.as_ref().and_then(Input::take_began).map(|at| self.engine.position_at(at));
        self.engine.stop();
        self.engine.set_endless(false);
        self.playing = false;
        self.settle = SETTLE_TICKS;
        self.cache.clear();
        let mut recording = recording;
        recording.taped.extend(self.engine.taped_keys());
        self.place_notes(&recording, stopped_at);
        if recording.tracks.is_empty() {
            return Task::none();
        }
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
        self.gather_fx_state();
        let printing: Vec<(TrackId, Vec<loupe_plugins::rack::Wanted>)> = tracks
            .iter()
            .filter_map(|id| self.project.track(*id))
            .filter(|track| track.print_takes)
            .map(|track| (track.id, crate::printing::wanted(&track.fx)))
            .filter(|(_, want)| !want.is_empty())
            .collect();
        let host = loupe_plugins::sandbox::host_beside_us();
        let keep_from = if recording.counted_in { recording.from as i64 } else { 0 };
        self.loading += 1;
        let (done, loaded) = oneshot::channel();
        std::thread::spawn(move || {
            let read = Source::load(&take.path, rate).map(Arc::new).map(|dry| {
                let mut trouble = None;
                let sources = tracks
                    .iter()
                    .map(|id| match printing.iter().find(|(track, _)| track == id) {
                        Some((_, want)) => match crate::printing::print_take(&dry, &take.path, want, rate, &host) {
                            Ok(printed) => (*id, Arc::new(printed)),
                            Err(why) => {
                                trouble = Some(why);
                                (*id, dry.clone())
                            }
                        },
                        None => (*id, dry.clone()),
                    })
                    .collect();
                (sources, trouble)
            });
            let _ = done.send(read);
        });
        Task::perform(
            async move { loaded.await.unwrap_or_else(|_| Err("reading the take stopped unexpectedly".into())) },
            move |result| Message::TakeReady { start, keep_from, warning: warning.clone(), result },
        )
    }

    pub(crate) fn place_take(&mut self, start: i64, keep_from: i64, result: Result<Vec<(TrackId, Arc<Source>)>, String>) {
        self.loading = self.loading.saturating_sub(1);
        let sources = match result {
            Ok(sources) => sources,
            Err(why) => {
                self.problem = Some(format!("Could not read the take: {why}"));
                return;
            }
        };
        let cut = (keep_from.max(0) - start).max(0) as Frames;
        let Some(whole) = sources.first().map(|(_, source)| source.frames.len() as Frames) else {
            return;
        };
        if cut >= whole {
            return;
        }
        let start = (start + cut as i64) as Frames;
        let mut placed = Vec::new();
        self.transact(None, |project| {
            for (track, source) in sources {
                if project.track(track).is_none() {
                    continue;
                }
                let Outcome::Clip(clip) = project.apply(Command::AddClip { track, source, start })? else {
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

pub fn notes_from_keys(taped: &[TapedKey], track: TrackId, stopped_at: Frames) -> Vec<Note> {
    let mut held: Vec<(u8, Frames, f32)> = Vec::new();
    let mut notes = Vec::new();
    for key in taped.iter().filter(|key| key.track == track) {
        if let Some(at) = held.iter().position(|(held_key, _, _)| *held_key == key.key) {
            let (_, start, velocity) = held.remove(at);
            notes.push(Note { key: key.key, start, len: key.at.saturating_sub(start).max(1), velocity });
        }
        if key.velocity > 0.0 {
            held.push((key.key, key.at, key.velocity));
        }
    }
    for (key, start, velocity) in held {
        notes.push(Note { key, start, len: stopped_at.saturating_sub(start).max(1), velocity });
    }
    notes.sort_by_key(|note| (note.start, note.key));
    notes
}

impl App {
    fn place_notes(&mut self, recording: &Recording, stopped_at: Frames) {
        let bar = bar_frames(self.project.bpm, self.project.rate).max(1);
        let mut made = Vec::new();
        let mut clips = Vec::new();
        for track in &recording.note_tracks {
            let notes = notes_from_keys(&recording.taped, *track, stopped_at);
            let (Some(first), Some(last)) = (notes.iter().map(|note| note.start).min(), notes.iter().map(Note::end).max()) else {
                continue;
            };
            let start = first / bar * bar;
            let end = last.div_ceil(bar) * bar;
            let shifted = notes.into_iter().map(|note| Note { start: note.start - start, ..note }).collect();
            let name = self.project.track(*track).map_or("Notes".to_string(), |found| found.name.clone());
            clips.push(Command::AddNotesClip { track: *track, name, start, len: (end - start).max(bar), notes: shifted });
        }
        if clips.is_empty() {
            if recording.tracks.is_empty() {
                self.notice = Some("Nothing was played.".into());
            }
            return;
        }
        self.transact(None, |project| {
            for clip in clips {
                if let Outcome::Clip(id) = project.apply(clip)? {
                    made.push(id);
                }
            }
            Ok(Outcome::Done)
        });
        self.choose(made);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tapped(track: TrackId, key: u8, velocity: f32, at: Frames) -> TapedKey {
        TapedKey { track, key, velocity, at }
    }

    #[test]
    fn keys_pressed_and_released_become_notes_and_held_keys_end_at_stop() {
        let mine = TrackId(1);
        let other = TrackId(2);
        let taped = [
            tapped(mine, 60, 0.9, 100),
            tapped(other, 50, 1.0, 120),
            tapped(mine, 64, 0.5, 200),
            tapped(mine, 60, 0.0, 400),
            tapped(mine, 60, 0.7, 500),
        ];
        let notes = notes_from_keys(&taped, mine, 1_000);
        assert_eq!(
            notes,
            vec![
                Note { key: 60, start: 100, len: 300, velocity: 0.9 },
                Note { key: 64, start: 200, len: 800, velocity: 0.5 },
                Note { key: 60, start: 500, len: 500, velocity: 0.7 },
            ]
        );
        assert!(notes_from_keys(&taped, TrackId(9), 1_000).is_empty());
    }
}
