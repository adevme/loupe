use std::path::{Path, PathBuf};

use crate::sandbox::Sandbox;
use crate::wire::{Ask, Reply};

pub struct Slot {
    pub path: PathBuf,
    pub index: usize,
    pub name: String,
    pub bypassed: bool,
    pub trouble: Option<String>,
    host: Option<Sandbox>,
    built: Option<Box<dyn loupe_stock::Effect>>,
}

impl Slot {
    pub fn working(&self) -> bool {
        (self.host.is_some() || self.built.is_some()) && self.trouble.is_none()
    }

    pub fn built_in(&self) -> bool {
        self.built.is_some()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wanted {
    pub path: PathBuf,
    pub index: usize,
    pub name: String,
    pub bypassed: bool,
    pub state: Vec<u8>,
}

pub struct Rack {
    host: PathBuf,
    rate: u32,
    block: usize,
    slots: Vec<Slot>,
}

impl Rack {
    pub fn new(host: PathBuf, rate: u32, block: usize) -> Self {
        Self { host, rate, block, slots: Vec::new() }
    }

    pub fn slots(&self) -> &[Slot] {
        &self.slots
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn add(&mut self, path: &Path, index: usize, name: &str) -> Result<usize, String> {
        let mut slot = Slot {
            path: path.to_path_buf(),
            index,
            name: name.to_string(),
            bypassed: false,
            trouble: None,
            host: None,
            built: None,
        };
        if is_built_in(&slot.path) {
            match self.make_built(slot.index) {
                Ok(made) => slot.built = Some(made),
                Err(why) => slot.trouble = Some(why),
            }
        } else {
            match self.open(&slot.path, slot.index) {
                Ok(host) => slot.host = Some(host),
                Err(why) => slot.trouble = Some(why),
            }
        }
        let trouble = slot.trouble.clone();
        self.slots.push(slot);
        match trouble {
            Some(why) => Err(why),
            None => Ok(self.slots.len() - 1),
        }
    }

    pub fn remove(&mut self, slot: usize) {
        if slot < self.slots.len() {
            self.slots.remove(slot);
        }
    }

    pub fn shift(&mut self, slot: usize, to: usize) {
        if slot < self.slots.len() && to < self.slots.len() {
            let moved = self.slots.remove(slot);
            self.slots.insert(to, moved);
        }
    }

    pub fn bypass(&mut self, slot: usize, bypassed: bool) {
        if let Some(found) = self.slots.get_mut(slot) {
            found.bypassed = bypassed;
        }
    }

    pub fn revive(&mut self, slot: usize) -> Result<(), String> {
        let Some(found) = self.slots.get_mut(slot) else {
            return Err("there is no such slot".into());
        };
        found.host = None;
        found.trouble = None;
        let path = found.path.clone();
        let index = found.index;
        match self.open(&path, index) {
            Ok(host) => {
                self.slots[slot].host = Some(host);
                Ok(())
            }
            Err(why) => {
                self.slots[slot].trouble = Some(why.clone());
                Err(why)
            }
        }
    }

    pub fn show(&mut self, slot: usize) -> Result<(), String> {
        self.tell(slot, Ask::Show)
    }

    pub fn hide(&mut self, slot: usize) -> Result<(), String> {
        self.tell(slot, Ask::Hide)
    }

    fn tell(&mut self, slot: usize, ask: Ask) -> Result<(), String> {
        let found = self.slots.get_mut(slot).ok_or("there is no such slot")?;
        if found.built.is_some() {
            return Err("the built in plugins do not have their own window yet".into());
        }
        let host = found.host.as_mut().ok_or("that plugin is not loaded")?;
        match host.ask(ask) {
            Ok(Reply::Fine) => Ok(()),
            Ok(Reply::Trouble(why)) => Err(why),
            Ok(other) => Err(format!("the plugin host answered out of turn: {other:?}")),
            Err(why) => {
                found.trouble = Some(why.clone());
                found.host = None;
                Err(why)
            }
        }
    }

    pub fn save(&mut self, slot: usize) -> Option<Vec<u8>> {
        let found = self.slots.get_mut(slot)?;
        if let Some(made) = found.built.as_ref() {
            return Some(take_knobs(made.as_ref()));
        }
        let host = found.host.as_mut()?;
        match host.ask(Ask::Save) {
            Ok(Reply::State(state)) => Some(state),
            Ok(Reply::Trouble(why)) => {
                found.trouble = Some(why);
                found.host = None;
                None
            }
            Ok(_) => None,
            Err(why) => {
                found.trouble = Some(why);
                found.host = None;
                None
            }
        }
    }

    pub fn process(&mut self, audio: &mut Vec<[f32; 2]>) {
        for slot in self.slots.iter_mut() {
            if slot.bypassed || slot.trouble.is_some() {
                continue;
            }
            if let Some(made) = slot.built.as_mut() {
                made.process(audio);
                continue;
            }
            let Some(host) = slot.host.as_mut() else { continue };
            if let Err(why) = host.run(audio) {
                slot.trouble = Some(why);
                slot.host = None;
            }
        }
    }

    pub fn reconcile(&mut self, want: &[Wanted]) -> Vec<String> {
        let mut pool: Vec<Slot> = self.slots.drain(..).collect();
        let mut troubles = Vec::new();
        for Wanted { path, index, name, bypassed, state } in want {
            let found = pool.iter().position(|slot| &slot.path == path && slot.index == *index && slot.working());
            match found {
                Some(at) => {
                    let mut slot = pool.remove(at);
                    slot.name = name.clone();
                    slot.bypassed = *bypassed;
                    if let Some(made) = slot.built.as_mut() {
                        put_knobs(made.as_mut(), state);
                    }
                    self.slots.push(slot);
                }
                None => {
                    let mut slot = Slot {
                        path: path.clone(),
                        index: *index,
                        name: name.clone(),
                        bypassed: *bypassed,
                        trouble: None,
                        host: None,
                        built: None,
                    };
                    let opened = if is_built_in(path) {
                        self.make_built(*index).map(|mut made| {
                            put_knobs(made.as_mut(), state);
                            slot.built = Some(made);
                        })
                    } else {
                        self.open(path, *index).and_then(|mut host| {
                            settle(&mut host, state)?;
                            slot.host = Some(host);
                            Ok(())
                        })
                    };
                    if let Err(why) = opened {
                        troubles.push(format!("{name}: {why}"));
                        slot.trouble = Some(why);
                    }
                    self.slots.push(slot);
                }
            }
        }
        troubles
    }

    fn make_built(&self, index: usize) -> Result<Box<dyn loupe_stock::Effect>, String> {
        let name = loupe_stock::NAMES.get(index).ok_or("Loupe has no such built in plugin")?;
        let mut made = loupe_stock::make(name).ok_or("Loupe has no such built in plugin")?;
        made.prepare(self.rate as f32);
        Ok(made)
    }

    fn open(&self, path: &Path, index: usize) -> Result<Sandbox, String> {
        let mut host = Sandbox::start(&self.host)?;
        let ask = Ask::Load {
            path: path.to_string_lossy().to_string(),
            index,
            rate: self.rate,
            block: self.block,
        };
        match host.ask(ask)? {
            Reply::Loaded { .. } => Ok(host),
            Reply::Trouble(why) => Err(why),
            other => Err(format!("the plugin host answered out of turn: {other:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rack() -> Rack {
        Rack::new(PathBuf::from("loupe-host-that-is-not-there"), 48_000, 512)
    }

    #[test]
    fn an_empty_rack_leaves_audio_alone() {
        let mut rack = rack();
        let mut audio = vec![[0.5, -0.25]; 8];
        rack.process(&mut audio);
        assert_eq!(audio, vec![[0.5, -0.25]; 8]);
    }

    #[test]
    fn a_plugin_that_will_not_open_is_kept_as_trouble() {
        let mut rack = rack();
        assert!(rack.add(Path::new("nowhere.vst3"), 0, "Nowhere").is_err());
        assert_eq!(rack.len(), 1);
        assert!(rack.slots()[0].trouble.is_some());
        assert!(!rack.slots()[0].working());
    }

    #[test]
    fn trouble_does_not_stop_the_audio() {
        let mut rack = rack();
        let _ = rack.add(Path::new("nowhere.vst3"), 0, "Nowhere");
        let mut audio = vec![[1.0, 1.0]; 4];
        rack.process(&mut audio);
        assert_eq!(audio, vec![[1.0, 1.0]; 4]);
    }

    #[test]
    fn slots_can_be_shifted_and_removed() {
        let mut rack = rack();
        let _ = rack.add(Path::new("one.vst3"), 0, "One");
        let _ = rack.add(Path::new("two.vst3"), 0, "Two");
        let _ = rack.add(Path::new("three.vst3"), 0, "Three");
        rack.shift(2, 0);
        let names: Vec<_> = rack.slots().iter().map(|slot| slot.name.clone()).collect();
        assert_eq!(names, vec!["Three", "One", "Two"]);
        rack.remove(1);
        let names: Vec<_> = rack.slots().iter().map(|slot| slot.name.clone()).collect();
        assert_eq!(names, vec!["Three", "Two"]);
    }

    #[test]
    fn reconcile_keeps_what_is_still_wanted_and_drops_the_rest() {
        let mut rack = rack();
        let want = vec![
            Wanted { path: PathBuf::from("one.vst3"), index: 0, name: "One".into(), bypassed: false, state: Vec::new() },
            Wanted { path: PathBuf::from("two.vst3"), index: 1, name: "Two".into(), bypassed: true, state: Vec::new() },
        ];
        let troubles = rack.reconcile(&want);
        assert_eq!(troubles.len(), 2);
        let names: Vec<_> = rack.slots().iter().map(|slot| slot.name.clone()).collect();
        assert_eq!(names, vec!["One", "Two"]);
        assert!(rack.slots()[1].bypassed);
        let shorter = vec![Wanted { path: PathBuf::from("two.vst3"), index: 1, name: "Two".into(), bypassed: false, state: Vec::new() }];
        rack.reconcile(&shorter);
        let names: Vec<_> = rack.slots().iter().map(|slot| slot.name.clone()).collect();
        assert_eq!(names, vec!["Two"]);
        assert!(!rack.slots()[0].bypassed);
    }

    #[test]
    fn bypass_is_remembered() {
        let mut rack = rack();
        let _ = rack.add(Path::new("one.vst3"), 0, "One");
        rack.bypass(0, true);
        assert!(rack.slots()[0].bypassed);
        rack.bypass(0, false);
        assert!(!rack.slots()[0].bypassed);
    }
}

fn settle(host: &mut Sandbox, state: &[u8]) -> Result<(), String> {
    if state.is_empty() {
        return Ok(());
    }
    match host.ask(Ask::Restore(state.to_vec()))? {
        Reply::Fine => Ok(()),
        Reply::Trouble(why) => Err(why),
        other => Err(format!("the plugin host answered out of turn: {other:?}")),
    }
}

pub fn is_built_in(path: &Path) -> bool {
    path.as_os_str() == crate::BUILT_IN
}

fn take_knobs(effect: &dyn loupe_stock::Effect) -> Vec<u8> {
    let mut out = Vec::with_capacity(effect.params().len() * 4);
    for index in 0..effect.params().len() {
        out.extend_from_slice(&effect.value(index).to_le_bytes());
    }
    out
}

fn put_knobs(effect: &mut dyn loupe_stock::Effect, state: &[u8]) {
    if state.len() != effect.params().len() * 4 {
        return;
    }
    for (index, four) in state.chunks_exact(4).enumerate() {
        effect.set(index, f32::from_le_bytes([four[0], four[1], four[2], four[3]]));
    }
}

#[cfg(test)]
mod built_in_tests {
    use super::*;

    #[test]
    fn a_built_in_loads_and_changes_the_sound() {
        let mut rack = Rack::new(PathBuf::from("no-host-here"), 48_000, 512);
        let slot = rack.add(Path::new(crate::BUILT_IN), 1, "Loupe Compressor").expect("it loads");
        assert!(rack.slots()[slot].working());
        assert!(rack.slots()[slot].built_in());
        let mut audio = vec![[0.9, 0.9]; 512];
        rack.process(&mut audio);
        let loudest = audio.iter().fold(0.0f32, |top, frame| top.max(frame[0].abs()));
        assert!(loudest < 0.9, "the compressor left it at {loudest}");
    }

    #[test]
    fn a_built_in_remembers_its_knobs() {
        let mut rack = Rack::new(PathBuf::from("no-host-here"), 48_000, 512);
        rack.add(Path::new(crate::BUILT_IN), 0, "Loupe EQ").expect("it loads");
        let first = rack.save(0).expect("it saves");
        assert!(!first.is_empty());
        let want = vec![Wanted {
            path: PathBuf::from(crate::BUILT_IN),
            index: 0,
            name: "Loupe EQ".into(),
            bypassed: false,
            state: first.clone(),
        }];
        let mut fresh = Rack::new(PathBuf::from("no-host-here"), 48_000, 512);
        assert!(fresh.reconcile(&want).is_empty());
        assert_eq!(fresh.save(0), Some(first));
    }
}
