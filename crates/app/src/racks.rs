use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use loupe_engine::{Chains, Clip, ClipId, Project, TrackId};
use loupe_plugins::rack::{Fallen, Rack, Wanted};
use loupe_plugins::sandbox::host_beside_us;
use loupe_plugins::wire::Region;
use loupe_stock::{Findings, History, Readings, Scopes};

#[derive(Clone, Default)]
pub struct Peek {
    pub scopes: Option<Arc<Scopes>>,
    pub history: Option<Arc<History>>,
    pub meter: Option<Arc<Readings>>,
    pub findings: Option<Arc<Findings>>,
    pub knobs: Vec<String>,
    pub held_back: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Spot {
    Track(TrackId, usize),
    Clip(ClipId, usize),
    Master(usize),
}

pub type Peeks = Arc<Mutex<HashMap<Spot, Peek>>>;
pub type PluginsOff = Arc<AtomicBool>;

#[derive(Clone, Debug, PartialEq)]
pub struct Fell {
    pub spot: Spot,
    pub name: String,
    pub why: String,
}

pub type Falls = Arc<Mutex<Vec<Fell>>>;

fn tell_falls(falls: &Falls, fallen: Vec<Fallen>, spot: impl Fn(usize) -> Spot) {
    let Ok(mut held) = falls.lock() else { return };
    held.extend(fallen.into_iter().map(|fell| Fell { spot: spot(fell.slot), name: fell.name, why: fell.why }));
}

pub struct Racks {
    host: PathBuf,
    rate: u32,
    block: usize,
    chains: HashMap<TrackId, Rack>,
    clips: HashMap<ClipId, Rack>,
    master: Option<Rack>,
    scratch: Vec<[f32; 2]>,
    peeks: Peeks,
    off: PluginsOff,
    falls: Falls,
    said_held_back: usize,
    off_the_audio_thread: bool,
}

impl Racks {
    pub fn new(rate: u32, block: usize, peeks: Peeks, off: PluginsOff, falls: Falls) -> Self {
        Self {
            host: host_beside_us(),
            rate,
            block,
            chains: HashMap::new(),
            clips: HashMap::new(),
            master: None,
            scratch: Vec::new(),
            peeks,
            off,
            falls,
            said_held_back: 0,
            off_the_audio_thread: false,
        }
    }

    pub fn away_from_the_audio_thread(mut self) -> Self {
        self.off_the_audio_thread = true;
        self
    }

    fn held_back(&self) -> usize {
        self.chains.values().chain(self.clips.values()).chain(self.master.iter()).map(Rack::held_back).sum()
    }

    fn word_about_the_ceiling(&mut self) -> Option<String> {
        let held_back = self.held_back();
        if held_back == self.said_held_back {
            return None;
        }
        self.said_held_back = held_back;
        if held_back == 0 {
            return None;
        }
        let most = loupe_plugins::ceiling::Ceiling::for_this_computer().most();
        let one = held_back == 1;
        Some(format!(
            "Loupe is running {most} plugins, as many as this computer can manage, so {held_back} {} not loaded. Open an FX chain and press Load to start {}.",
            if one { "is" } else { "are" },
            if one { "it" } else { "one" },
        ))
    }

    fn settle(&mut self, project: &Project) -> Vec<String> {
        if self.off.load(Ordering::Relaxed) {
            self.chains.clear();
            self.clips.clear();
            self.master = None;
            self.said_held_back = 0;
            self.publish();
            return Vec::new();
        }
        let mut troubles = Vec::new();
        if project.master_fx.is_empty() {
            self.master = None;
        } else {
            let want: Vec<_> = project
                .master_fx
                .iter()
                .map(|fx| Wanted {
                    mix: fx.mix,
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
                    mix: fx.mix,
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
                        mix: fx.mix,
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
        self.gather();
        if let Some(word) = self.word_about_the_ceiling() {
            troubles.insert(0, word);
        }
        troubles
    }

    fn gather(&mut self) {
        for (id, rack) in self.chains.iter_mut() {
            if rack.has_fallen() {
                tell_falls(&self.falls, rack.take_fallen(), |slot| Spot::Track(*id, slot));
            }
        }
        for (id, rack) in self.clips.iter_mut() {
            if rack.has_fallen() {
                tell_falls(&self.falls, rack.take_fallen(), |slot| Spot::Clip(*id, slot));
            }
        }
        if let Some(rack) = self.master.as_mut().filter(|rack| rack.has_fallen()) {
            tell_falls(&self.falls, rack.take_fallen(), Spot::Master);
        }
    }

    fn publish(&mut self) {
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
                let held_back = rack.slots().get(slot).is_some_and(|found| found.held_back);
                let (scopes, history, meter, findings) = match rack.built_at(slot) {
                    Some(made) => (made.scopes(), made.history(), made.meter(), made.findings()),
                    None => (None, None, None, None),
                };
                found.push((Spot::Track(*id, slot), Peek { scopes, history, meter, findings, knobs, held_back }));
            }
        }
        for (id, rack) in self.clips.iter_mut() {
            for slot in 0..rack.len() {
                let knobs = rack.knobs(slot);
                let held_back = rack.slots().get(slot).is_some_and(|found| found.held_back);
                let (scopes, history, meter, findings) = match rack.built_at(slot) {
                    Some(made) => (made.scopes(), made.history(), made.meter(), made.findings()),
                    None => (None, None, None, None),
                };
                found.push((Spot::Clip(*id, slot), Peek { scopes, history, meter, findings, knobs, held_back }));
            }
        }
        if let Some(rack) = self.master.as_mut() {
            for slot in 0..rack.len() {
                let knobs = rack.knobs(slot);
                let held_back = rack.slots().get(slot).is_some_and(|found| found.held_back);
                let (scopes, history, meter, findings) = match rack.built_at(slot) {
                    Some(made) => (made.scopes(), made.history(), made.meter(), made.findings()),
                    None => (None, None, None, None),
                };
                found.push((Spot::Master(slot), Peek { scopes, history, meter, findings, knobs, held_back }));
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

    fn load(&mut self, track: TrackId, slot: usize) -> Result<(), String> {
        match self.chains.get_mut(&track) {
            Some(rack) => rack.load_held_back(slot),
            None => Err("that track has no plugins".into()),
        }
    }

    fn load_clip(&mut self, clip: ClipId, slot: usize) -> Result<(), String> {
        match self.clips.get_mut(&clip) {
            Some(rack) => rack.load_held_back(slot),
            None => Err("that clip has no plugins".into()),
        }
    }

    fn load_master(&mut self, slot: usize) -> Result<(), String> {
        match self.master.as_mut() {
            Some(rack) => rack.load_held_back(slot),
            None => Err("the master has no plugins".into()),
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
        if rack.has_fallen() {
            tell_falls(&self.falls, rack.take_fallen(), |slot| Spot::Clip(clip, slot));
        }
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
        self.gather();
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
        if rack.has_fallen() {
            tell_falls(&self.falls, rack.take_fallen(), |slot| Spot::Track(track, slot));
        }
        let shared = self.scratch.len().min(audio.len());
        audio[..shared].copy_from_slice(&self.scratch[..shared]);
    }

    fn whole_waves_at_once(&self) -> bool {
        self.off_the_audio_thread
    }

    fn process_wave(&mut self, jobs: &mut [loupe_engine::Job<'_>]) {
        let mut spare: Vec<&mut loupe_engine::Job<'_>> = jobs.iter_mut().collect();
        let mut pairs: Vec<(&mut Rack, &mut loupe_engine::Job<'_>)> = Vec::with_capacity(spare.len());
        for (id, rack) in self.chains.iter_mut() {
            if rack.is_empty() {
                continue;
            }
            if let Some(at) = spare.iter().position(|job| job.track == *id) {
                pairs.push((rack, spare.swap_remove(at)));
            }
        }
        if pairs.len() < 2 {
            for (rack, job) in pairs {
                run_one(rack, job, &self.falls);
            }
            return;
        }
        let queue = Mutex::new(pairs);
        let hands = queue.lock().map(|held| held.len()).unwrap_or(0).min(MOST_TRACKS_AT_ONCE);
        let falls = &self.falls;
        std::thread::scope(|scope| {
            for _ in 0..hands {
                let queue = &queue;
                scope.spawn(move || loop {
                    let next = queue.lock().ok().and_then(|mut held| held.pop());
                    let Some((rack, job)) = next else { return };
                    run_one(rack, job, falls);
                });
            }
        });
    }

    fn process_takes(&mut self, track: TrackId, audio: &mut [[f32; 2]]) {
        let Some(rack) = self.chains.get_mut(&track) else { return };
        if !rack.has_takes() {
            return;
        }
        self.scratch.clear();
        self.scratch.extend_from_slice(audio);
        rack.process_takes(&mut self.scratch);
        if rack.has_fallen() {
            tell_falls(&self.falls, rack.take_fallen(), |slot| Spot::Track(track, slot));
        }
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
        if rack.has_fallen() {
            tell_falls(&self.falls, rack.take_fallen(), Spot::Master);
        }
        let shared = self.scratch.len().min(audio.len());
        audio[..shared].copy_from_slice(&self.scratch[..shared]);
    }
}

const MOST_TRACKS_AT_ONCE: usize = 256;

fn run_one(rack: &mut Rack, job: &mut loupe_engine::Job<'_>, falls: &Falls) {
    let mut held: Vec<[f32; 2]> = job.audio.to_vec();
    rack.process_with(&mut held, job.side);
    if rack.has_fallen() {
        let track = job.track;
        tell_falls(falls, rack.take_fallen(), |slot| Spot::Track(track, slot));
    }
    let shared = held.len().min(job.audio.len());
    job.audio[..shared].copy_from_slice(&held[..shared]);
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
    use loupe_engine::{Command, Fx, Outcome, Source};

    use super::*;

    fn fx(path: &str, index: usize, state: &[u8]) -> Fx {
        Fx { path: PathBuf::from(path), index, name: format!("{path} {index}"), bypassed: false, state: state.to_vec(), record: false, mix: 1.0 }
    }

    fn song_with_plugins() -> (Project, TrackId, ClipId) {
        let mut project = Project::new(48_000);
        let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: "Vox".into() }) else { panic!() };
        let source = Arc::new(Source::from_frames("take", vec![[0.1, 0.1]; 4_800]));
        let Ok(Outcome::Clip(clip)) = project.apply(Command::AddClip { track, source, start: 0 }) else { panic!() };
        project.apply(Command::AddFx { track, fx: fx("/nowhere/Crasher.vst3", 0, &[1, 2, 3]) }).unwrap();
        project.apply(Command::AddFx { track, fx: fx(loupe_plugins::BUILT_IN, 1, &[]) }).unwrap();
        project.apply(Command::AddClipFx { clip, fx: fx("/nowhere/Clipper.clap", 0, &[4, 5]) }).unwrap();
        project.apply(Command::AddMasterFx(fx("/nowhere/Glue.vst3", 0, &[6]))).unwrap();
        (project, track, clip)
    }

    fn racks(off: bool) -> Racks {
        Racks::new(48_000, 512, Peeks::default(), Arc::new(AtomicBool::new(off)), Falls::default())
    }

    fn six_tracks_with_a_stock_plugin() -> Project {
        let mut project = Project::new(48_000);
        for at in 0..6 {
            let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: format!("T{at}") }) else {
                panic!("no track")
            };
            let level = 0.1 + at as f32 / 10.0;
            let source = Arc::new(Source::from_frames("take", vec![[level, -level]; 4_800]));
            project.apply(Command::AddClip { track, source, start: 0 }).unwrap();
            project.apply(Command::AddFx { track, fx: fx(loupe_plugins::BUILT_IN, 1, &[]) }).unwrap();
        }
        project
    }

    #[test]
    fn a_whole_wave_of_racks_at_once_sounds_like_one_rack_at_a_time() {
        let project = six_tracks_with_a_stock_plugin();
        let mut both = Vec::new();
        for at_once in [false, true] {
            let mut racks = racks(false);
            if at_once {
                racks = racks.away_from_the_audio_thread();
            }
            assert_eq!(racks.whole_waves_at_once(), at_once);
            racks.follow(&project);
            assert_eq!(racks.chains.len(), 6);
            let mut out = vec![[0.0f32; 2]; 512];
            let mut scratch = loupe_engine::Mixdown::default();
            loupe_engine::render_through(&project, 0, &mut out, &mut scratch, Some(&mut racks));
            both.push(out);
        }
        assert_eq!(both[0], both[1], "the racks gave a different mix when they ran together");
        assert!(both[0][200][0].abs() > 1e-6, "the mix is silent, so it proves nothing");
    }

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
    fn a_song_opened_with_plugins_off_loads_none_of_them() {
        let (project, _, _) = song_with_plugins();
        let mut racks = racks(true);
        assert!(racks.follow(&project).is_empty());
        assert!(racks.chains.is_empty() && racks.clips.is_empty() && racks.master.is_none());
        assert!(!racks.still_opening());
        assert!(racks.harvest().is_empty() && racks.harvest_clips().is_empty());
        assert!(racks.show(project.tracks[0].id, 0).is_err());
    }

    #[test]
    fn turning_plugins_back_on_loads_every_one_again() {
        let (project, track, clip) = song_with_plugins();
        let mut racks = racks(true);
        racks.follow(&project);
        racks.off.store(false, Ordering::Relaxed);
        racks.follow(&project);
        assert_eq!(racks.chains.get(&track).map(Rack::len), Some(2));
        assert_eq!(racks.clips.get(&clip).map(Rack::len), Some(1));
        assert_eq!(racks.master.as_ref().map(Rack::len), Some(1));
        assert!(racks.chains[&track].slots()[1].built_in());
    }

    #[test]
    fn saving_with_plugins_off_writes_the_same_song_as_saving_with_them_on() {
        let (mut project, _, _) = song_with_plugins();
        let saved = |song: &Project| loupe_engine::SavedProject::capture(song, |_| None).to_text();
        let as_opened = saved(&project);
        let mut racks = racks(true);
        racks.follow(&project);
        for (track, slot, state) in racks.harvest() {
            project.apply(Command::SetFxState { track, slot, state }).unwrap();
        }
        for (clip, slot, state) in racks.harvest_clips() {
            project.apply(Command::SetClipFxState { clip, slot, state }).unwrap();
        }
        assert_eq!(saved(&project), as_opened);
        assert!(as_opened.contains("Crasher"));
    }

    #[test]
    fn plugins_that_were_running_close_when_they_are_turned_off() {
        let (project, track, _) = song_with_plugins();
        let mut racks = racks(false);
        racks.follow(&project);
        assert!(racks.chains.contains_key(&track));
        racks.off.store(true, Ordering::Relaxed);
        racks.follow(&project);
        assert!(racks.chains.is_empty() && racks.clips.is_empty() && racks.master.is_none());
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
        use loupe_engine::SavedProject;
        use loupe_plugins::ara_document::{pack, unpack};
        let (mut project, id) = project_with_clip("C:\\Songs\\lead.wav");
        let archive = vec![0u8, 1, 2, 255, b'\n', b'\t', b' '];
        let state = pack(b"melodyne window", "com.celemony.ara.audiosourcedescription.13", &archive);
        let fx = Fx { path: PathBuf::from("C:\\VST3\\Melodyne.vst3"), index: 0, name: "Melodyne".into(), bypassed: false, state: state.clone(), record: false, mix: 1.0 };
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
