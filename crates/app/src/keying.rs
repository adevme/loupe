use std::path::{Path, PathBuf};

use iced::futures::channel::oneshot;
use iced::Task;
use loupe_engine::{Command, Outcome, Project, Source};
use loupe_stock::{key_name, listen_to, Heard};
use loupe_stock_ui::KeyMessage;

use crate::{App, Message, Overlay};

const TUNE: &str = "Loupe Tune";
const TUNE_KEY: usize = 0;
const TUNE_SCALE: usize = 1;
const MAJOR_SCALE: f32 = 1.0;
const MINOR_SCALE: f32 = 2.0;

fn tune_index() -> Option<usize> {
    loupe_stock::NAMES.iter().position(|name| *name == TUNE)
}

fn is_tune(fx: &loupe_engine::Fx) -> bool {
    fx.path == Path::new(loupe_plugins::BUILT_IN) && Some(fx.index) == tune_index()
}

fn retuned(state: &[u8], key: u8) -> Vec<u8> {
    let mut knobs = Vec::new();
    if let Some(made) = loupe_stock::make(TUNE) {
        for index in 0..made.params().len() {
            let at = index * 4;
            let value = state.get(at..at + 4).map_or(made.value(index), |four| f32::from_le_bytes([four[0], four[1], four[2], four[3]]));
            knobs.push(value);
        }
    }
    if knobs.len() > TUNE_SCALE {
        knobs[TUNE_KEY] = (key % 12) as f32;
        knobs[TUNE_SCALE] = if key < 12 { MAJOR_SCALE } else { MINOR_SCALE };
    }
    knobs.iter().flat_map(|value| value.to_le_bytes()).collect()
}

pub fn send_to(project: &mut Project, bpm: Option<f32>, key: Option<u8>) -> Result<usize, loupe_engine::CommandError> {
    if let Some(bpm) = bpm {
        project.apply(Command::SetBpm(bpm as f64))?;
    }
    let Some(key) = key else { return Ok(0) };
    let mut changes = Vec::new();
    for track in &project.tracks {
        for (slot, fx) in track.fx.iter().enumerate().filter(|(_, fx)| is_tune(fx)) {
            changes.push(Command::SetFxState { track: track.id, slot, state: retuned(&fx.state, key) });
        }
        for clip in &track.clips {
            for (slot, fx) in clip.fx.iter().enumerate().filter(|(_, fx)| is_tune(fx)) {
                changes.push(Command::SetClipFxState { clip: clip.id, slot, state: retuned(&fx.state, key) });
            }
        }
    }
    for (slot, fx) in project.master_fx.iter().enumerate().filter(|(_, fx)| is_tune(fx)) {
        changes.push(Command::SetMasterFxState { slot, state: retuned(&fx.state, key) });
    }
    let count = changes.len();
    for change in changes {
        project.apply(change)?;
    }
    Ok(count)
}

impl App {
    pub(crate) fn key_told(&mut self, message: KeyMessage) {
        let Some(window) = self.stock.as_mut() else { return };
        let Some((bpm, key)) = window.told_key(message) else { return };
        let mut tuned = 0;
        self.transact(None, |project| {
            tuned = send_to(project, bpm, key)?;
            Ok(Outcome::Done)
        });
        self.bpm = crate::format_bpm(self.project.bpm);
        let mut said = Vec::new();
        if let Some(bpm) = bpm {
            said.push(format!("the song to {} BPM", crate::format_bpm(bpm as f64)));
        }
        if let Some(key) = key {
            said.push(match tuned {
                0 => format!("found {} but there is no Loupe Tune to send it to", key_name(key)),
                1 => format!("{} on 1 Loupe Tune", key_name(key)),
                many => format!("{} on {many} Loupe Tunes", key_name(key)),
            });
        }
        self.notice = Some(format!("Set {}.", said.join(", and ")));
    }

    pub(crate) fn key_wants_file(&self) -> bool {
        matches!(self.overlay, Overlay::Stock) && self.stock.as_ref().is_some_and(|window| matches!(window.face, crate::stockwin::Face::Key(_)))
    }

    pub(crate) fn read_key_from(&mut self, path: PathBuf) -> Task<Message> {
        let name = path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
        if let Some(editor) = self.stock.as_mut().and_then(|window| window.key_editor()) {
            editor.reading(name.clone());
        }
        let rate = self.project.rate;
        let (done, read) = oneshot::channel();
        std::thread::spawn(move || {
            let heard = Source::load(&path, rate).map(|source| listen_to(&source.frames, rate as f32));
            let _ = done.send(heard);
        });
        Task::perform(async move { read.await.unwrap_or_else(|_| Err("reading the file stopped".into())) }, move |result| Message::KeyFileHeard(name.clone(), result))
    }

    pub(crate) fn key_file_heard(&mut self, name: String, result: Result<Heard, String>) {
        let Some(editor) = self.stock.as_mut().and_then(|window| window.key_editor()) else { return };
        match result {
            Ok(heard) => editor.file_heard(name, heard),
            Err(why) => {
                editor.update(KeyMessage::ForgetFile);
                self.problem = Some(format!("Could not read {name}: {why}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loupe_engine::Fx;
    use std::path::PathBuf;

    #[test]
    fn send_sets_the_tempo_and_every_loupe_tune_but_nothing_else() {
        let mut project = Project::new(48_000);
        let Ok(Outcome::Track(vocal)) = project.apply(Command::AddTrack { name: "Vocal".into() }) else { panic!() };
        let tune = Fx { path: PathBuf::from(loupe_plugins::BUILT_IN), index: tune_index().unwrap(), name: TUNE.into(), bypassed: false, state: Vec::new(), record: false };
        let eq = Fx { path: PathBuf::from(loupe_plugins::BUILT_IN), index: 0, name: "Loupe EQ".into(), bypassed: false, state: vec![1, 2, 3, 4], record: false };
        project.apply(Command::AddFx { track: vocal, fx: eq }).unwrap();
        project.apply(Command::AddFx { track: vocal, fx: tune.clone() }).unwrap();
        project.apply(Command::AddMasterFx(tune)).unwrap();
        let a_minor = 21;
        assert_eq!(send_to(&mut project, Some(92.0), Some(a_minor)).unwrap(), 2);
        assert_eq!(project.bpm, 92.0);
        let track = project.track(vocal).unwrap();
        assert_eq!(track.fx[0].state, vec![1, 2, 3, 4], "the EQ was left alone");
        let mut made = loupe_stock::make(TUNE).unwrap();
        for (index, four) in track.fx[1].state.chunks_exact(4).enumerate() {
            made.set(index, f32::from_le_bytes([four[0], four[1], four[2], four[3]]));
        }
        assert_eq!((made.value(TUNE_KEY), made.value(TUNE_SCALE)), (9.0, MINOR_SCALE));
        assert_eq!(made.value(2), made.params()[2].default, "the other knobs keep their places");
        assert_eq!(project.master_fx[0].state, track.fx[1].state);
    }
}
