use std::path::{Path, PathBuf};
use std::sync::Arc;

use iced::futures::channel::oneshot;
use iced::Task;
use loupe_engine::{bar_frames, Command, CommandError, Frames, Input, InputChannels, Note, Outcome, Source, TapedKey, TrackId};

use crate::selection::TAKES_FOLDER;
use crate::{App, Message, SETTLE_TICKS};

const UNNAMED_TAKE: &str = "Take";

pub struct Recording {
    pub from: Frames,
    tracks: Vec<(TrackId, usize)>,
    note_tracks: Vec<TrackId>,
    pub taped: Vec<TapedKey>,
    counted_in: bool,
    looping: Option<(Frames, Frames)>,
    pub began: Option<i64>,
    pub punch: Option<(Frames, Frames)>,
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
        let punch = match (self.punch, self.loop_range.filter(|(from, to)| to > from)) {
            (false, _) => None,
            (true, Some(range)) => Some(range),
            (true, None) => {
                self.notice = Some("Punch is on: mark the part to redo on the ruler first.".into());
                return Task::none();
            }
        };
        let mut tracks: Vec<(TrackId, usize)> = tracks.into_iter().map(|track| (track, 0)).collect();
        if !tracks.is_empty() {
            let Some(input) = &self.input else {
                self.notice = Some("The recording input is not open. Check Settings > Recording.".into());
                return Task::none();
            };
            let Some(folder) = self.path.as_deref().and_then(Path::parent).map(|project| project.join(TAKES_FOLDER)) else {
                let asked = self.save_as();
                self.entry_problem = Some("Save the project first. Takes are kept in its Audio folder.".into());
                return asked;
            };
            let inputs = input.inputs();
            let recorded: Vec<&loupe_engine::Track> = tracks.iter().filter_map(|(id, _)| self.project.track(*id)).collect();
            if let Some(track) = recorded.iter().find(|track| !track.input.fits(inputs)) {
                let has = if inputs == 1 { "1 input".to_string() } else { format!("{inputs} inputs") };
                self.problem = Some(format!("{} records from {}, but the recording input has {has}. Right click the track to choose another.", track.name, track.input.name()));
                return Task::none();
            }
            let (files, file_for) = take_files(&folder, &recorded);
            for ((_, file_at), at) in tracks.iter_mut().zip(file_for) {
                *file_at = at;
            }
            if let Err(why) = input.begin_takes(&files) {
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
        let looping = self.loop_range.filter(|(from, to)| punch.is_none() && to > from && (!self.playing || (*from..*to).contains(&self.playhead)));
        self.engine.set_endless(looping.is_none());
        let counted_in = !self.playing && self.count_in_bars > 0 && punch.is_none();
        if !self.playing {
            match (punch, self.loop_range) {
                (Some((from, _)), _) => {
                    let preroll = bar_frames(self.project.bpm, self.project.rate) * self.preroll_bars as Frames;
                    self.seek(from.saturating_sub(preroll));
                }
                (None, Some((from, _))) => self.seek(from),
                (None, None) => {}
            }
            if counted_in {
                self.engine.count_in(bar_frames(self.project.bpm, self.project.rate) * self.count_in_bars as Frames);
            }
            self.engine.play();
            self.playing = true;
        }
        self.recording = Some(Recording { from: self.playhead, tracks, note_tracks, taped: Vec::new(), counted_in, looping, began: None, punch });
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
        let began = recording.began.or_else(|| self.input.as_ref().and_then(Input::take_began).map(|at| self.engine.position_at(at)));
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
            Some(input) => input.finish_takes(),
            None => Err("the recording input closed".to_string()),
        };
        let takes = match finished {
            Ok(takes) => takes,
            Err(why) => {
                self.problem = Some(format!("Could not keep the take: {why}"));
                return Task::none();
            }
        };
        let (frames, lost, take_rate) = takes.first().map_or((0, 0, 1), |take| (take.frames, take.lost, take.rate));
        let (Some(start), true) = (began, frames > 0) else {
            for take in &takes {
                let _ = std::fs::remove_file(&take.path);
            }
            self.notice = Some("Nothing was recorded.".into());
            return Task::none();
        };
        let warning = (lost > 0).then(|| {
            let lost_ms = lost * 1000 / take_rate.max(1) as u64;
            format!("The disk fell behind: about {lost_ms} ms of the take were lost.")
        });
        let rate = self.project.rate;
        let tracks = recording.tracks;
        self.gather_fx_state();
        let printing: Vec<(TrackId, Vec<loupe_plugins::rack::Wanted>)> = tracks
            .iter()
            .filter_map(|(id, _)| self.project.track(*id))
            .filter(|track| track.print_takes)
            .map(|track| (track.id, crate::printing::wanted(&track.fx)))
            .filter(|(_, want)| !want.is_empty() && !self.plugins_are_off())
            .collect();
        let host = loupe_plugins::sandbox::host_beside_us();
        let keep_from = if recording.counted_in { recording.from as i64 } else { 0 };
        let punch = recording.punch;
        let passes = recording.looping.and_then(|(from, to)| loop_takes(start, from, to, frames, rate));
        let pad = passes.as_ref().map_or(0, |passes| passes.pad);
        self.loading += 1;
        let (done, loaded) = oneshot::channel();
        std::thread::spawn(move || {
            let dries: Result<Vec<Arc<Source>>, String> =
                takes.iter().map(|take| Source::load(&take.path, rate).and_then(|dry| padded(dry, &take.path, pad, rate)).map(Arc::new)).collect();
            let read = dries.map(|dries| {
                let mut trouble = None;
                let sources = tracks
                    .iter()
                    .map(|(id, at)| {
                        let dry = &dries[*at];
                        match printing.iter().find(|(track, _)| track == id) {
                            Some((_, want)) => match crate::printing::print_take(dry, &takes[*at].path, want, rate, &host) {
                                Ok(printed) => (*id, Arc::new(printed)),
                                Err(why) => {
                                    trouble = Some(why);
                                    (*id, dry.clone())
                                }
                            },
                            None => (*id, dry.clone()),
                        }
                    })
                    .collect();
                (sources, trouble)
            });
            let _ = done.send(read);
        });
        Task::perform(
            async move { loaded.await.unwrap_or_else(|_| Err("reading the take stopped unexpectedly".into())) },
            move |result| Message::TakeReady { start, keep_from, passes: passes.clone(), punch, warning: warning.clone(), result },
        )
    }

    pub(crate) fn place_take(&mut self, start: i64, keep_from: i64, passes: Option<Passes>, punch: Option<(Frames, Frames)>, result: Result<Vec<(TrackId, Arc<Source>)>, String>) {
        self.loading = self.loading.saturating_sub(1);
        let sources = match result {
            Ok(sources) => sources,
            Err(why) => {
                self.problem = Some(format!("Could not read the take: {why}"));
                return;
            }
        };
        if let Some(passes) = passes {
            self.place_passes(passes, sources);
            return;
        }
        if let Some((from, to)) = punch {
            self.place_punched(start, from, to, sources);
            return;
        }
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

fn take_files(folder: &Path, recorded: &[&loupe_engine::Track]) -> (Vec<(PathBuf, InputChannels)>, Vec<usize>) {
    let mut files: Vec<(PathBuf, InputChannels, bool)> = Vec::new();
    let mut file_for = Vec::new();
    for track in recorded {
        let shareable = !track.print_takes;
        if let Some(at) = files.iter().position(|(_, input, open)| shareable && *open && *input == track.input) {
            file_for.push(at);
            continue;
        }
        let name = crate::files::file_safe(&track.name);
        let file = unused_take(folder, if name.is_empty() { UNNAMED_TAKE } else { &name }, &files);
        files.push((file, track.input, shareable));
        file_for.push(files.len() - 1);
    }
    (files.into_iter().map(|(file, input, _)| (file, input)).collect(), file_for)
}

fn unused_take(folder: &Path, name: &str, taken: &[(PathBuf, InputChannels, bool)]) -> PathBuf {
    (1..)
        .map(|number| folder.join(format!("{name} (take {number}).wav")))
        .find(|file| !file.exists() && taken.iter().all(|(chosen, _, _)| chosen != file))
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
    fn a_punched_take_keeps_only_the_marked_part() {
        assert_eq!(punched_span(1_000, 10_000, 5_000, 7_000), Some((5_000, 4_000, 2_000)));
        assert_eq!(punched_span(6_000, 10_000, 5_000, 7_000), Some((6_000, 0, 1_000)), "started late, kept from where it began");
        assert_eq!(punched_span(1_000, 5_000, 5_000, 7_000), Some((5_000, 4_000, 1_000)), "stopped early, kept up to the stop");
        assert_eq!(punched_span(1_000, 3_000, 5_000, 7_000), None);
    }

    #[test]
    fn tracks_on_the_same_input_share_a_take_and_other_inputs_get_their_own() {
        let mut project = loupe_engine::Project::new(48_000);
        let mut add = |name: &str, input: InputChannels, print: bool| {
            let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: name.into() }) else { panic!() };
            project.apply(Command::SetTrackInput { track, input }).unwrap();
            project.apply(Command::SetPrintTakes { track, on: print }).unwrap();
        };
        add("Vocal", InputChannels::Mono(1), false);
        add("Double", InputChannels::Mono(1), false);
        add("Keys", InputChannels::Stereo(2), false);
        add("Printed", InputChannels::Mono(1), true);
        add("Vocal", InputChannels::Mono(0), false);
        let recorded: Vec<&loupe_engine::Track> = project.tracks.iter().collect();
        let folder = std::env::temp_dir().join(format!("loupe-take-files-{}", std::process::id()));
        let (files, file_for) = take_files(&folder, &recorded);
        assert_eq!(file_for, [0, 0, 1, 2, 3]);
        let named = |name: &str| folder.join(name);
        assert_eq!(
            files,
            [
                (named("Vocal (take 1).wav"), InputChannels::Mono(1)),
                (named("Keys (take 1).wav"), InputChannels::Stereo(2)),
                (named("Printed (take 1).wav"), InputChannels::Mono(1)),
                (named("Vocal (take 2).wav"), InputChannels::Mono(0)),
            ]
        );
    }

    #[test]
    fn each_time_round_the_loop_is_a_take_and_the_last_whole_one_plays() {
        let counted = loop_takes(1_000 - 400, 1_000, 2_000, 400 + 3_500, 100).unwrap();
        assert_eq!(counted.offsets, vec![400, 1_400, 2_400, 3_400]);
        assert_eq!((counted.pad, counted.active, counted.len, counted.from), (0, 2, 1_000, 1_000));
        let late = loop_takes(1_250, 1_000, 2_000, 2_750, 100).unwrap();
        assert_eq!(late.pad, 250);
        assert_eq!(late.offsets, vec![0, 1_000, 2_000]);
        assert_eq!(late.active, 2);
        assert_eq!(loop_takes(1_000, 1_000, 2_000, 900, 100), None, "once round is an ordinary take");
        assert_eq!(loop_takes(1_000, 1_000, 2_000, 1_010, 100), None, "a stub after the loop comes round is not a take");
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

#[derive(Clone, Debug, PartialEq)]
pub struct Passes {
    pub from: Frames,
    pub len: Frames,
    pub pad: Frames,
    pub offsets: Vec<i64>,
    pub active: usize,
}

pub fn loop_takes(start: i64, from: Frames, to: Frames, frames: u64, rate: u32) -> Option<Passes> {
    let len = to.checked_sub(from).filter(|len| *len > 0)?;
    let first = from as i64 - start;
    let pad = (-first).max(0) as Frames;
    let first = first + pad as i64;
    let recorded = (frames + pad) as i64;
    let shortest = (rate / 4) as i64;
    let offsets: Vec<i64> = (0..).map(|pass| first + pass * len as i64).take_while(|at| *at < recorded - shortest).collect();
    if offsets.len() < 2 {
        return None;
    }
    let whole = offsets.iter().rposition(|at| at + len as i64 <= recorded).unwrap_or(offsets.len() - 1);
    Some(Passes { from, len, pad, offsets, active: whole })
}

fn padded(dry: Source, path: &Path, pad: Frames, rate: u32) -> Result<Source, String> {
    if pad == 0 {
        return Ok(dry);
    }
    let mut frames = vec![[0.0; 2]; pad as usize];
    frames.extend_from_slice(&dry.frames);
    loupe_engine::write_frames(path, &frames, rate).map_err(|why| format!("{}: {why}", path.display()))?;
    Source::load(path, rate)
}

impl App {
    fn place_passes(&mut self, passes: Passes, sources: Vec<(TrackId, Arc<Source>)>) {
        let Passes { from, len, offsets, active, .. } = passes;
        let count = offsets.len();
        let mut placed = Vec::new();
        self.transact(None, |project| {
            for (track, source) in sources {
                if project.track(track).is_none() {
                    continue;
                }
                let Outcome::Clip(clip) = project.apply(Command::AddClip { track, source, start: from })? else {
                    return Err(CommandError::NoSuchClip);
                };
                project.apply(Command::TrimClip { clip, offset: offsets[active].max(0) as Frames, len })?;
                project.apply(Command::SetTakes { clip, offsets: offsets.clone(), active })?;
                placed.push(clip);
            }
            Ok(Outcome::Done)
        });
        if !placed.is_empty() {
            self.choose(placed);
            self.notice = Some(format!("{count} takes. Click a lane to hear it, or use the comp tool (K) to pick the best parts."));
        }
    }
}

pub fn punched_span(start: i64, frames: Frames, from: Frames, to: Frames) -> Option<(Frames, Frames, Frames)> {
    let recorded_end = start + frames as i64;
    let begin = (from as i64).max(start);
    let end = (to as i64).min(recorded_end);
    (end > begin).then(|| (begin as Frames, (begin - start) as Frames, (end - begin) as Frames))
}

impl App {
    fn place_punched(&mut self, start: i64, from: Frames, to: Frames, sources: Vec<(TrackId, Arc<Source>)>) {
        let Some(frames) = sources.first().map(|(_, source)| source.frames.len() as Frames) else {
            return;
        };
        let Some((begin, offset, len)) = punched_span(start, frames, from, to) else {
            self.notice = Some("Nothing was recorded inside the punch range.".into());
            return;
        };
        let fade = crate::takes::short_fade(self.project.rate);
        let mut placed = Vec::new();
        self.transact(None, |project| {
            for (track, source) in sources {
                if project.track(track).is_none() {
                    continue;
                }
                crate::takes::clear_between(project, track, begin, begin + len, fade)?;
                let Outcome::Clip(clip) = project.apply(Command::AddClip { track, source, start: begin })? else {
                    return Err(CommandError::NoSuchClip);
                };
                project.apply(Command::TrimClip { clip, offset, len })?;
                project.apply(Command::SetClipFade { clip, edge: loupe_engine::Edge::In, fade })?;
                project.apply(Command::SetClipFade { clip, edge: loupe_engine::Edge::Out, fade })?;
                placed.push(clip);
            }
            Ok(Outcome::Done)
        });
        if !placed.is_empty() {
            self.choose(placed);
        }
    }
}
