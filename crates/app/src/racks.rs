use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use loupe_engine::{Chains, ClipId, Project, TrackId};
use loupe_plugins::rack::{Rack, Wanted};
use loupe_plugins::sandbox::host_beside_us;
use loupe_stock::{History, Scopes};

#[derive(Clone, Default)]
pub struct Peek {
    pub scopes: Option<Arc<Scopes>>,
    pub history: Option<Arc<History>>,
    pub knobs: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Spot {
    Track(TrackId, usize),
    Clip(ClipId, usize),
}

pub type Peeks = Arc<Mutex<HashMap<Spot, Peek>>>;

pub struct Racks {
    host: PathBuf,
    rate: u32,
    block: usize,
    chains: HashMap<TrackId, Rack>,
    clips: HashMap<ClipId, Rack>,
    scratch: Vec<[f32; 2]>,
    peeks: Peeks,
}

impl Racks {
    pub fn new(rate: u32, block: usize, peeks: Peeks) -> Self {
        Self { host: host_beside_us(), rate, block, chains: HashMap::new(), clips: HashMap::new(), scratch: Vec::new(), peeks }
    }

    fn settle(&mut self, project: &Project) -> Vec<String> {
        let mut troubles = Vec::new();
        self.chains.retain(|id, _| project.tracks.iter().any(|track| track.id == *id));
        for track in &project.tracks {
            if track.fx.is_empty() {
                self.chains.remove(&track.id);
                continue;
            }
            let want: Vec<_> = track
                .fx
                .iter()
                .map(|fx| Wanted {
                    path: fx.path.clone(),
                    index: fx.index,
                    name: fx.name.clone(),
                    bypassed: fx.bypassed,
                    state: fx.state.clone(),
                })
                .collect();
            let rack = self
                .chains
                .entry(track.id)
                .or_insert_with(|| Rack::new(self.host.clone(), self.rate, self.block));
            troubles.extend(rack.reconcile(&want));
        }
        let alive: Vec<ClipId> = project.tracks.iter().flat_map(|track| track.clips.iter().map(|clip| clip.id)).collect();
        self.clips.retain(|id, _| alive.contains(id));
        for track in &project.tracks {
            for clip in &track.clips {
                if clip.fx.is_empty() {
                    self.clips.remove(&clip.id);
                    continue;
                }
                let want: Vec<_> = clip
                    .fx
                    .iter()
                    .map(|fx| Wanted {
                        path: fx.path.clone(),
                        index: fx.index,
                        name: fx.name.clone(),
                        bypassed: fx.bypassed,
                        state: fx.state.clone(),
                    })
                    .collect();
                let rack = self
                    .clips
                    .entry(clip.id)
                    .or_insert_with(|| Rack::new(self.host.clone(), self.rate, self.block));
                troubles.extend(rack.reconcile(&want));
            }
        }
        self.publish();
        troubles
    }

    fn publish(&mut self) {
        let mut found: Vec<(Spot, Peek)> = Vec::new();
        for (id, rack) in self.chains.iter_mut() {
            for slot in 0..rack.len() {
                let knobs = rack.knobs(slot);
                let (scopes, history) = match rack.built_at(slot) {
                    Some(made) => (made.scopes(), made.history()),
                    None => (None, None),
                };
                found.push((Spot::Track(*id, slot), Peek { scopes, history, knobs }));
            }
        }
        for (id, rack) in self.clips.iter_mut() {
            for slot in 0..rack.len() {
                let knobs = rack.knobs(slot);
                let (scopes, history) = match rack.built_at(slot) {
                    Some(made) => (made.scopes(), made.history()),
                    None => (None, None),
                };
                found.push((Spot::Clip(*id, slot), Peek { scopes, history, knobs }));
            }
        }
        let Ok(mut held) = self.peeks.lock() else { return };
        held.clear();
        held.extend(found);
    }

    fn problems(&self) -> Vec<(TrackId, String, String)> {
        let mut out = Vec::new();
        for (id, rack) in &self.chains {
            for slot in rack.slots() {
                if let Some(why) = &slot.trouble {
                    out.push((*id, slot.name.clone(), why.clone()));
                }
            }
        }
        out
    }
}

impl Chains for Racks {
    fn follow(&mut self, project: &Project) -> Vec<String> {
        self.settle(project)
    }

    fn troubles(&self) -> Vec<(TrackId, String, String)> {
        self.problems()
    }

    fn show(&mut self, track: TrackId, slot: usize) -> Result<(), String> {
        match self.chains.get_mut(&track) {
            Some(rack) => rack.show(slot),
            None => Err("that track has no plugins".into()),
        }
    }

    fn tweak(&mut self, track: TrackId, slot: usize, knob: usize, value: f32) {
        if let Some(rack) = self.chains.get_mut(&track) {
            rack.tweak(slot, knob, value);
        }
    }

    fn latency(&self, track: TrackId) -> usize {
        self.chains.get(&track).map(|rack| rack.latency()).unwrap_or(0)
    }

    fn process_clip(&mut self, clip: ClipId, audio: &mut [[f32; 2]]) {
        let Some(rack) = self.clips.get_mut(&clip) else { return };
        if rack.is_empty() {
            return;
        }
        self.scratch.clear();
        self.scratch.extend_from_slice(audio);
        rack.process(&mut self.scratch);
        let shared = self.scratch.len().min(audio.len());
        audio[..shared].copy_from_slice(&self.scratch[..shared]);
    }

    fn clip_latency(&self, clip: ClipId) -> usize {
        self.clips.get(&clip).map(|rack| rack.latency()).unwrap_or(0)
    }

    fn tweak_clip(&mut self, clip: ClipId, slot: usize, knob: usize, value: f32) {
        if let Some(rack) = self.clips.get_mut(&clip) {
            rack.tweak(slot, knob, value);
        }
    }

    fn show_clip(&mut self, clip: ClipId, slot: usize) -> Result<(), String> {
        match self.clips.get_mut(&clip) {
            Some(rack) => rack.show(slot),
            None => Err("that clip has no plugins".into()),
        }
    }

    fn automate(&mut self, track: TrackId, slot: usize, knob: usize, value: f32) {
        if let Some(rack) = self.chains.get_mut(&track) {
            rack.automate(slot, knob, value);
        }
    }

    fn automate_clip(&mut self, clip: ClipId, slot: usize, knob: usize, value: f32) {
        if let Some(rack) = self.clips.get_mut(&clip) {
            rack.automate(slot, knob, value);
        }
    }

    fn harvest_clips(&mut self) -> Vec<(ClipId, usize, Vec<u8>)> {
        let mut out = Vec::new();
        for (id, rack) in self.clips.iter_mut() {
            for slot in 0..rack.len() {
                if let Some(state) = rack.save(slot) {
                    out.push((*id, slot, state));
                }
            }
        }
        out
    }

    fn harvest(&mut self) -> Vec<(TrackId, usize, Vec<u8>)> {
        let mut out = Vec::new();
        for (id, rack) in self.chains.iter_mut() {
            for slot in 0..rack.len() {
                if let Some(state) = rack.save(slot) {
                    out.push((*id, slot, state));
                }
            }
        }
        out
    }

    fn process_with(&mut self, track: TrackId, audio: &mut [[f32; 2]], side: &[[f32; 2]]) {
        let Some(rack) = self.chains.get_mut(&track) else { return };
        if rack.is_empty() {
            return;
        }
        self.scratch.clear();
        self.scratch.extend_from_slice(audio);
        rack.process_with(&mut self.scratch, side);
        let shared = self.scratch.len().min(audio.len());
        audio[..shared].copy_from_slice(&self.scratch[..shared]);
    }
}
