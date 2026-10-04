use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use loupe_engine::{Chains, Clip, ClipId, Project, TrackId};
use loupe_plugins::rack::{Rack, Wanted};
use loupe_plugins::wire::Region;
use loupe_plugins::sandbox::host_beside_us;
use loupe_stock::{History, Readings, Scopes};

#[derive(Clone, Default)]
pub struct Peek {
    pub scopes: Option<Arc<Scopes>>,
    pub history: Option<Arc<History>>,
    pub meter: Option<Arc<Readings>>,
    pub knobs: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Spot {
    Track(TrackId, usize),
    Clip(ClipId, usize),
    Master(usize),
}

pub type Peeks = Arc<Mutex<HashMap<Spot, Peek>>>;

pub struct Racks {
    host: PathBuf,
    rate: u32,
    block: usize,
    chains: HashMap<TrackId, Rack>,
    clips: HashMap<ClipId, Rack>,
    /// The plugins over the whole mix.
    master: Option<Rack>,
    scratch: Vec<[f32; 2]>,
    peeks: Peeks,
}

impl Racks {
    pub fn new(rate: u32, block: usize, peeks: Peeks) -> Self {
        Self {
            host: host_beside_us(),
            rate,
            block,
            chains: HashMap::new(),
            clips: HashMap::new(),
            master: None,
            scratch: Vec::new(),
            peeks,
        }
    }

    fn settle(&mut self, project: &Project) -> Vec<String> {
        let mut troubles = Vec::new();
        if project.master_fx.is_empty() {
            self.master = None;
        } else {
            let want: Vec<_> = project
                .master_fx
                .iter()
                .map(|fx| Wanted {
                    path: fx.path.clone(),
                    index: fx.index,
                    name: fx.name.clone(),
                    bypassed: fx.bypassed,
                    state: fx.state.clone(),
                    record: fx.record,
                })
                .collect();
            let rack = self.master.get_or_insert_with(|| Rack::new(self.host.clone(), self.rate, self.block));
            troubles.extend(rack.reconcile(&want));
        }
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
                    record: fx.record,
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
                        record: false,
                    })
                    .collect();
                let rack = self
                    .clips
                    .entry(clip.id)
                    .or_insert_with(|| Rack::new(self.host.clone(), self.rate, self.block));
                troubles.extend(rack.follow_region(region_of(project, clip)));
                troubles.extend(rack.reconcile(&want));
            }
        }
        self.publish();
        troubles
    }

    fn publish(&mut self) {
        // Runs on the thread that draws, so this is where a waiting window may open.
        for rack in self.chains.values_mut().chain(self.clips.values_mut()).chain(self.master.iter_mut()) {
            let _ = rack.open_waiting();
        }
        for rack in self.clips.values_mut() {
            let region = rack.region().cloned();
            let _ = rack.follow_region(region);
        }
        let mut found: Vec<(Spot, Peek)> = Vec::new();
        for (id, rack) in self.chains.iter_mut() {
            for slot in 0..rack.len() {
                let knobs = rack.knobs(slot);
                let (scopes, history, meter) = match rack.built_at(slot) {
                    Some(made) => (made.scopes(), made.history(), made.meter()),
                    None => (None, None, None),
                };
                found.push((Spot::Track(*id, slot), Peek { scopes, history, meter, knobs }));
            }
        }
        for (id, rack) in self.clips.iter_mut() {
            for slot in 0..rack.len() {
                let knobs = rack.knobs(slot);
                let (scopes, history, meter) = match rack.built_at(slot) {
                    Some(made) => (made.scopes(), made.history(), made.meter()),
                    None => (None, None, None),
                };
                found.push((Spot::Clip(*id, slot), Peek { scopes, history, meter, knobs }));
            }
        }
        if let Some(rack) = self.master.as_mut() {
            for slot in 0..rack.len() {
                let knobs = rack.knobs(slot);
                let (scopes, history, meter) = match rack.built_at(slot) {
                    Some(made) => (made.scopes(), made.history(), made.meter()),
                    None => (None, None, None),
                };
                found.push((Spot::Master(slot), Peek { scopes, history, meter, knobs }));
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

    fn show_master(&mut self, slot: usize) -> Result<(), String> {
        match self.master.as_mut() {
            Some(rack) => rack.show(slot),
            None => Err("the master has no plugins".into()),
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

    fn process_clip_at(&mut self, clip: ClipId, audio: &mut [[f32; 2]], at: loupe_engine::Frames) {
        let Some(rack) = self.clips.get_mut(&clip) else { return };
        if rack.is_empty() {
            return;
        }
        self.scratch.clear();
        self.scratch.extend_from_slice(audio);
        rack.process_at(&mut self.scratch, at as i64);
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

    fn automate_master(&mut self, slot: usize, knob: usize, value: f32) {
        if let Some(rack) = self.master.as_mut() {
            rack.automate(slot, knob, value);
        }
    }

    fn still_opening(&self) -> bool {
        self.chains.values().chain(self.clips.values()).chain(self.master.iter()).any(|rack| rack.still_opening())
    }

    fn nudge(&mut self) {
        self.publish();
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

    fn process_takes(&mut self, track: TrackId, audio: &mut [[f32; 2]]) {
        let Some(rack) = self.chains.get_mut(&track) else { return };
        if !rack.has_takes() {
            return;
        }
        self.scratch.clear();
        self.scratch.extend_from_slice(audio);
        rack.process_takes(&mut self.scratch);
        let shared = self.scratch.len().min(audio.len());
        audio[..shared].copy_from_slice(&self.scratch[..shared]);
    }

    fn process_master(&mut self, audio: &mut [[f32; 2]]) {
        let Some(rack) = self.master.as_mut() else { return };
        if rack.is_empty() {
            return;
        }
        self.scratch.clear();
        self.scratch.extend_from_slice(audio);
        rack.process(&mut self.scratch);
        let shared = self.scratch.len().min(audio.len());
        audio[..shared].copy_from_slice(&self.scratch[..shared]);
    }
}

fn region_of(project: &Project, clip: &Clip) -> Option<Region> {
    if clip.notes.is_some() || clip.source.path.as_os_str().is_empty() || project.rate == 0 {
        return None;
    }
    let rate = project.rate as f64;
    let stretch = if clip.stretch > 0.0 { clip.stretch } else { 1.0 };
    Some(Region {
        file: clip.source.path.to_string_lossy().into_owned(),
        name: clip.source.name.clone(),
        start: clip.start as f64 / rate,
        offset: clip.offset as f64 / rate / stretch,
        length: clip.len as f64 / rate,
        stretch,
        tempo: project.bpm,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use loupe_engine::{Command, Outcome, Source};

    fn project_with_clip(path: &str) -> (Project, ClipId) {
        let mut project = Project::new(48_000);
        project.bpm = 90.0;
        let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: "Vocal".into() }) else { panic!("no track") };
        let mut source = Source::from_frames("lead", vec![[0.0; 2]; 480_000]);
        source.path = PathBuf::from(path);
        let Ok(Outcome::Clip(clip)) = project.apply(Command::AddClip { track, source: Arc::new(source), start: 96_000 }) else {
            panic!("no clip")
        };
        (project, clip)
    }

    fn clip_in(project: &Project, id: ClipId) -> &Clip {
        project.tracks.iter().flat_map(|track| track.clips.iter()).find(|clip| clip.id == id).expect("the clip is there")
    }

    #[test]
    fn a_clip_becomes_a_region_in_seconds() {
        let (project, id) = project_with_clip("C:\\Songs\\lead.wav");
        let region = region_of(&project, clip_in(&project, id)).expect("an audio clip has a region");
        assert_eq!(region.file, "C:\\Songs\\lead.wav");
        assert_eq!(region.name, "lead");
        assert_eq!(region.start, 2.0);
        assert_eq!(region.offset, 0.0);
        assert_eq!(region.length, 10.0);
        assert_eq!(region.stretch, 1.0);
        assert_eq!(region.tempo, 90.0);
    }

    #[test]
    fn a_trimmed_and_stretched_clip_reads_its_audio_from_the_right_place() {
        let (project, id) = project_with_clip("lead.wav");
        let mut clip = clip_in(&project, id).clone();
        clip.offset = 96_000;
        clip.len = 48_000;
        clip.stretch = 2.0;
        let region = region_of(&project, &clip).expect("it has a region");
        assert_eq!(region.offset, 1.0);
        assert_eq!(region.length, 1.0);
        assert_eq!(region.stretch, 2.0);
    }

    #[test]
    fn melodyne_edits_on_a_clip_survive_a_save_and_open() {
        use loupe_engine::{Fx, SavedProject};
        use loupe_plugins::ara_document::{pack, unpack};
        let (mut project, id) = project_with_clip("C:\\Songs\\lead.wav");
        let archive = vec![0u8, 1, 2, 255, b'\n', b'\t', b' '];
        let state = pack(b"melodyne window", "com.celemony.ara.audiosourcedescription.13", &archive);
        let fx = Fx { path: PathBuf::from("C:\\VST3\\Melodyne.vst3"), index: 0, name: "Melodyne".into(), bypassed: false, state: state.clone(), record: false };
        project.apply(Command::AddClipFx { clip: id, fx }).expect("the plugin goes on the clip");
        let text = SavedProject::capture(&project, |_| None).to_text();
        let sources = vec![project.sources[0].clone()];
        let (back, _) = SavedProject::parse(&text).expect("it reads back").build(&sources, 48_000);
        let clip = back.tracks[0].clips.first().expect("the clip came back");
        assert_eq!(clip.fx[0].state, state);
        let unpacked = unpack(&clip.fx[0].state).expect("the edits are still packed in");
        assert_eq!(unpacked.settings, b"melodyne window");
        assert_eq!(unpacked.archive, &archive[..]);
        assert_eq!(region_of(&back, clip), region_of(&project, clip_in(&project, id)));
    }

    #[test]
    fn audio_without_a_file_has_no_region() {
        let (project, id) = project_with_clip("");
        assert_eq!(region_of(&project, clip_in(&project, id)), None);
    }
}
