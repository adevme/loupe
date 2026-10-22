use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use loupe_engine::{Chains, Clip, ClipId, Engine, PluginSpending, Project, Saved, Touched, TrackId};
use loupe_plugins::rack::{let_them_go, Fallen, Rack, Wanted};
use loupe_plugins::sandbox::host_beside_us;
use loupe_plugins::wire::Region;
use loupe_stock::{Findings, History, Readings, Scopes, Spectra, Tuning};

#[derive(Clone, Default)]
pub struct Peek {
    pub scopes: Option<Arc<Scopes>>,
    pub history: Option<Arc<History>>,
    pub meter: Option<Arc<Readings>>,
    pub findings: Option<Arc<Findings>>,
    pub spectra: Option<Arc<Spectra>>,
    pub tuning: Option<Arc<Tuning>>,
    pub knobs: Vec<String>,
    pub held_back: bool,
    pub host: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Spot {
    Track(TrackId, usize),
    Clip(ClipId, usize),
    Master(usize),
    Monitor(usize),
}

pub type Peeks = Arc<Mutex<HashMap<Spot, Peek>>>;
pub type PluginsOff = Arc<AtomicBool>;

pub struct Lending<T> {
    held: Option<T>,
    out: bool,
}

impl<T> Default for Lending<T> {
    fn default() -> Self {
        Self { held: None, out: false }
    }
}

impl<T> Lending<T> {
    pub fn take(&mut self) -> Option<T> {
        self.held.take()
    }

    pub fn keep(&mut self, thing: T) {
        self.out = false;
        self.held = Some(thing);
    }

    pub fn gone_out(&mut self) {
        self.held = None;
        self.out = true;
    }

    pub fn is_out(&self) -> bool {
        self.out
    }

    pub fn came_back(&mut self, thing: T) -> T {
        self.out = false;
        thing
    }

    pub fn holding(&self) -> bool {
        self.held.is_some()
    }

    pub fn forget(&mut self) {
        self.held = None;
        self.out = false;
    }
}

/// Waves of tracks run on scoped threads, counted for `loupe bench`: how many waves went that
/// way and the time spent inside the scope, threads spawned and joined included.
pub static WAVES_SCOPED: AtomicU64 = AtomicU64::new(0);
pub static WAVE_SCOPE_NANOS: AtomicU64 = AtomicU64::new(0);

pub fn wave_timing() -> (u64, u64) {
    (WAVES_SCOPED.load(Ordering::Relaxed), WAVE_SCOPE_NANOS.load(Ordering::Relaxed))
}

pub const CHAIN_BLOCK: usize = 512;
const WAIT_FOR_CHAINS: u32 = 500;

pub fn lend(engine: &mut Engine, holder: &mut Lending<Box<dyn Chains>>, racks: Box<dyn Chains>) {
    if engine.running().is_none() {
        holder.keep(racks);
        return;
    }
    engine.use_chains(racks);
    holder.gone_out();
}

pub fn borrow(engine: &mut Engine, holder: &mut Lending<Box<dyn Chains>>) -> Option<Box<dyn Chains>> {
    if let Some(racks) = holder.take() {
        return Some(racks);
    }
    if !holder.is_out() {
        return None;
    }
    engine.drop_chains();
    for _ in 0..WAIT_FOR_CHAINS {
        if let Some(got) = engine.chains_back() {
            return Some(holder.came_back(got));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    None
}

pub fn borrow_or_make(engine: &mut Engine, holder: &mut Lending<Box<dyn Chains>>, peeks: &Peeks, off: &PluginsOff, falls: &Falls) -> Option<Box<dyn Chains>> {
    if let Some(racks) = borrow(engine, holder) {
        return Some(racks);
    }
    if holder.is_out() {
        return None;
    }
    Some(Box::new(Racks::new(engine.rate(), CHAIN_BLOCK, peeks.clone(), off.clone(), falls.clone())) as Box<dyn Chains>)
}

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

fn all_at_once<Of: Copy + Send>(racks: Vec<(Of, &mut Rack)>) -> Vec<(Of, usize, Vec<u8>)> {
    std::thread::scope(|scope| {
        let asking: Vec<_> = racks
            .into_iter()
            .map(|(of, rack)| scope.spawn(move || rack.all_they_remember().into_iter().map(|(slot, state)| (of, slot, state)).collect::<Vec<_>>()))
            .collect();
        asking.into_iter().flat_map(|one| one.join().unwrap_or_default()).collect()
    })
}

#[derive(Clone, Copy)]
enum Which {
    Track(TrackId),
    Clip(ClipId),
    Master,
    Monitor,
}

fn wanted_from(fx: &loupe_engine::Fx) -> Wanted {
    Wanted {
        mix: fx.mix,
        path: fx.path.clone(),
        index: fx.index,
        name: fx.name.clone(),
        bypassed: fx.bypassed,
        state: fx.state.clone(),
        record: fx.record,
    }
}

pub struct Racks {
    pub(crate) host: PathBuf,
    rate: u32,
    block: usize,
    chains: HashMap<TrackId, Rack>,
    clips: HashMap<ClipId, Rack>,
    master: Option<Rack>,
    monitor: Option<Rack>,
    scratch: Vec<[f32; 2]>,
    peeks: Peeks,
    off: PluginsOff,
    falls: Falls,
    said_held_back: usize,
    off_the_audio_thread: bool,
    own_ceiling: Option<std::sync::Arc<loupe_plugins::ceiling::Ceiling>>,
    spent_in_plugins: AtomicU64,
    waves: AtomicU64,
    tracks_in_waves: AtomicU64,
    crew: crate::crew::Crew,
}

type RunRack = fn(&mut Rack, &mut Vec<[f32; 2]>, &[[f32; 2]]);

impl Racks {
    fn through_track(&mut self, track: TrackId, audio: &mut [[f32; 2]], side: &[[f32; 2]], run: RunRack) {
        let Self { chains, scratch, falls, spent_in_plugins, .. } = self;
        let Some(rack) = chains.get_mut(&track) else { return };
        if rack.is_empty() {
            return;
        }
        let began = std::time::Instant::now();
        scratch.clear();
        scratch.extend_from_slice(audio);
        run(rack, scratch, side);
        note_plugin_time(spent_in_plugins, began);
        if rack.has_fallen() {
            tell_falls(falls, rack.take_fallen(), |slot| Spot::Track(track, slot));
        }
        let shared = scratch.len().min(audio.len());
        audio[..shared].copy_from_slice(&scratch[..shared]);
    }

    pub fn new(rate: u32, block: usize, peeks: Peeks, off: PluginsOff, falls: Falls) -> Self {
        Self {
            host: host_beside_us(),
            rate,
            block,
            chains: HashMap::new(),
            clips: HashMap::new(),
            master: None,
            monitor: None,
            scratch: Vec::new(),
            peeks,
            off,
            falls,
            said_held_back: 0,
            off_the_audio_thread: false,
            own_ceiling: None,
            spent_in_plugins: AtomicU64::new(0),
            waves: AtomicU64::new(0),
            tracks_in_waves: AtomicU64::new(0),
            crew: crate::crew::Crew::of(crew_hands()),
        }
    }

    pub fn away_from_the_audio_thread(mut self) -> Self {
        self.off_the_audio_thread = true;
        self.keep_time();
        self.own_ceiling = Some(loupe_plugins::ceiling::Ceiling::of(loupe_plugins::ceiling::Ceiling::for_this_computer().most()));
        self
    }

    fn a_new_rack(&self) -> Rack {
        let mut made = match &self.own_ceiling {
            Some(ceiling) => Rack::sharing(self.host.clone(), self.rate, self.block, ceiling.clone()),
            None => Rack::new(self.host.clone(), self.rate, self.block),
        };
        made.keep_time(!self.off_the_audio_thread);
        made
    }

    fn every_rack(&self) -> impl Iterator<Item = &Rack> {
        self.chains.values().chain(self.clips.values()).chain(self.master.iter()).chain(self.monitor.iter())
    }

    fn keep_time(&mut self) {
        let on_time = !self.off_the_audio_thread;
        for rack in self.chains.values_mut().chain(self.clips.values_mut()).chain(self.master.iter_mut()).chain(self.monitor.iter_mut()) {
            rack.keep_time(on_time);
        }
    }

    fn held_back(&self) -> usize {
        self.chains.values().chain(self.clips.values()).chain(self.master.iter()).chain(self.monitor.iter()).map(Rack::held_back).sum()
    }

    fn running(&self) -> usize {
        self.chains
            .values()
            .chain(self.clips.values())
            .chain(self.master.iter())
            .chain(self.monitor.iter())
            .flat_map(Rack::slots)
            .filter(|slot| !slot.held_back && slot.trouble.is_none() && !slot.bypassed)
            .count()
    }

    fn note_plugin_time(&self, began: std::time::Instant) {
        note_plugin_time(&self.spent_in_plugins, began);
    }

    fn run_a_whole_wave(&mut self, jobs: &mut [loupe_engine::Job<'_>]) {
        let Self { chains, falls, crew, waves, tracks_in_waves, .. } = self;
        let mut spare: Vec<&mut loupe_engine::Job<'_>> = jobs.iter_mut().collect();
        let mut pairs: Vec<(&mut Rack, &mut loupe_engine::Job<'_>)> = Vec::with_capacity(spare.len());
        for (id, rack) in chains.iter_mut() {
            if rack.is_empty() {
                continue;
            }
            if let Some(at) = spare.iter().position(|job| job.track == *id) {
                pairs.push((rack, spare.swap_remove(at)));
            }
        }
        waves.fetch_add(1, Ordering::Relaxed);
        tracks_in_waves.fetch_add(pairs.len() as u64, Ordering::Relaxed);
        if pairs.len() < 2 {
            for (rack, job) in pairs {
                run_one(rack, job, falls);
            }
            return;
        }
        let clock = Instant::now();
        let falls: &Falls = falls;
        let batch: Vec<Box<dyn FnMut() + Send + '_>> =
            pairs.into_iter().map(|(rack, job)| Box::new(move || run_one(rack, job, falls)) as Box<dyn FnMut() + Send + '_>).collect();
        crew.run_all(batch);
        WAVES_SCOPED.fetch_add(1, Ordering::Relaxed);
        WAVE_SCOPE_NANOS.fetch_add(clock.elapsed().as_nanos() as u64, Ordering::Relaxed);
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

    fn everything_leaving(&mut self) -> Vec<Rack> {
        let mut going: Vec<Rack> = self.chains.drain().map(|(_, rack)| rack).collect();
        going.extend(self.clips.drain().map(|(_, rack)| rack));
        going.extend(self.master.take());
        going.extend(self.monitor.take());
        going
    }

    fn who_is_leaving(&mut self, project: &Project) -> Vec<Rack> {
        if self.off.load(Ordering::Relaxed) {
            return self.everything_leaving();
        }
        let mut going = Vec::new();
        if project.master_fx.is_empty() {
            going.extend(self.master.take());
        }
        if project.monitor_fx.is_empty() {
            going.extend(self.monitor.take());
        }
        let staying: Vec<TrackId> = project.tracks.iter().filter(|track| !track.fx.is_empty()).map(|track| track.id).collect();
        let tracks: Vec<TrackId> = self.chains.keys().copied().filter(|id| !staying.contains(id)).collect();
        for track in tracks {
            going.extend(self.chains.remove(&track));
        }
        let kept: Vec<ClipId> =
            project.tracks.iter().flat_map(|track| track.clips.iter()).filter(|clip| !clip.fx.is_empty()).map(|clip| clip.id).collect();
        let clips: Vec<ClipId> = self.clips.keys().copied().filter(|id| !kept.contains(id)).collect();
        for clip in clips {
            going.extend(self.clips.remove(&clip));
        }
        going
    }

    fn settle(&mut self, project: &Project) -> Vec<String> {
        let going = self.who_is_leaving(project);
        let_them_go(going);
        if self.off.load(Ordering::Relaxed) {
            self.said_held_back = 0;
            self.publish();
            return Vec::new();
        }
        let mut troubles = Vec::new();
        if !project.master_fx.is_empty() {
            let want: Vec<_> = project.master_fx.iter().map(wanted_from).collect();
            let fresh = self.a_new_rack();
            let rack = self.master.get_or_insert(fresh);
            troubles.extend(rack.reconcile(&want));
            rack.bypass_chain(!project.all_fx_on || !project.master_fx_on);
        }
        if !project.monitor_fx.is_empty() {
            let want: Vec<_> = project.monitor_fx.iter().map(wanted_from).collect();
            let fresh = self.a_new_rack();
            let rack = self.monitor.get_or_insert(fresh);
            troubles.extend(rack.reconcile(&want));
            rack.bypass_chain(!project.all_fx_on || !project.monitor_fx_on);
        }
        for track in &project.tracks {
            if track.fx.is_empty() {
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
            let fresh = self.a_new_rack();
            let rack = self
                .chains
                .entry(track.id)
                .or_insert(fresh);
            troubles.extend(rack.reconcile(&want));
            rack.bypass_chain(!project.all_fx_on || !track.fx_on);
        }
        for track in &project.tracks {
            for clip in &track.clips {
                if clip.fx.is_empty() {
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
                let fresh = self.a_new_rack();
                let rack = self.clips.entry(clip.id).or_insert(fresh);
                troubles.extend(rack.follow_region(region_of(project, clip)));
                troubles.extend(rack.reconcile(&want));
                rack.bypass_chain(!project.all_fx_on);
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
        if let Some(rack) = self.monitor.as_mut().filter(|rack| rack.has_fallen()) {
            tell_falls(&self.falls, rack.take_fallen(), Spot::Monitor);
        }
    }

    fn publish(&mut self) {
        for rack in self.chains.values_mut().chain(self.clips.values_mut()).chain(self.master.iter_mut()).chain(self.monitor.iter_mut()) {
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
                let (scopes, history, meter, findings, spectra, tuning) = match rack.built_at(slot) {
                    Some(made) => (made.scopes(), made.history(), made.meter(), made.findings(), made.spectra(), made.tuning()),
                    None => (None, None, None, None, None, None),
                };

                found.push((Spot::Track(*id, slot), Peek { scopes, history, meter, findings, spectra, tuning, knobs, held_back, host: rack.host_pid(slot) }));


            }
        }
        for (id, rack) in self.clips.iter_mut() {
            for slot in 0..rack.len() {
                let knobs = rack.knobs(slot);
                let held_back = rack.slots().get(slot).is_some_and(|found| found.held_back);
                let (scopes, history, meter, findings, spectra, tuning) = match rack.built_at(slot) {
                    Some(made) => (made.scopes(), made.history(), made.meter(), made.findings(), made.spectra(), made.tuning()),
                    None => (None, None, None, None, None, None),
                };

                found.push((Spot::Clip(*id, slot), Peek { scopes, history, meter, findings, spectra, tuning, knobs, held_back, host: rack.host_pid(slot) }));


            }
        }
        if let Some(rack) = self.master.as_mut() {
            for slot in 0..rack.len() {
                let knobs = rack.knobs(slot);
                let held_back = rack.slots().get(slot).is_some_and(|found| found.held_back);
                let (scopes, history, meter, findings, spectra, tuning) = match rack.built_at(slot) {
                    Some(made) => (made.scopes(), made.history(), made.meter(), made.findings(), made.spectra(), made.tuning()),
                    None => (None, None, None, None, None, None),
                };

                found.push((Spot::Master(slot), Peek { scopes, history, meter, findings, spectra, tuning, knobs, held_back, host: rack.host_pid(slot) }));


            }
        }
        if let Some(rack) = self.monitor.as_mut() {
            for slot in 0..rack.len() {
                let knobs = rack.knobs(slot);
                let held_back = rack.slots().get(slot).is_some_and(|found| found.held_back);
                let (scopes, history, meter, findings, spectra, tuning) = match rack.built_at(slot) {
                    Some(made) => (made.scopes(), made.history(), made.meter(), made.findings(), made.spectra(), made.tuning()),
                    None => (None, None, None, None, None, None),
                };
                found.push((Spot::Monitor(slot), Peek { scopes, history, meter, findings, spectra, tuning, knobs, held_back, host: rack.host_pid(slot) }));
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

    fn called(&mut self, track: TrackId, slot: usize, label: &str) -> Result<(), String> {
        match self.chains.get_mut(&track) {
            Some(rack) => rack.called(slot, label),
            None => Err("that track has no plugins".into()),
        }
    }

    fn plugins_touched(&mut self) -> Vec<Touched> {
        let mut found = Vec::new();
        let tracks: Vec<TrackId> = self.chains.keys().copied().collect();
        for track in tracks {
            if let Some(rack) = self.chains.get_mut(&track) {
                for slot in 0..rack.len() {
                    if rack.was_touched(slot) {
                        found.push(Touched::Track(track, slot));
                    }
                }
            }
        }
        let clips: Vec<ClipId> = self.clips.keys().copied().collect();
        for clip in clips {
            if let Some(rack) = self.clips.get_mut(&clip) {
                for slot in 0..rack.len() {
                    if rack.was_touched(slot) {
                        found.push(Touched::Clip(clip, slot));
                    }
                }
            }
        }
        if let Some(rack) = self.master.as_mut() {
            for slot in 0..rack.len() {
                if rack.was_touched(slot) {
                    found.push(Touched::Master(slot));
                }
            }
        }
        if let Some(rack) = self.monitor.as_mut() {
            for slot in 0..rack.len() {
                if rack.was_touched(slot) {
                    found.push(Touched::Monitor(slot));
                }
            }
        }
        found
    }

    fn state_of(&mut self, which: Touched) -> Option<Vec<u8>> {
        match which {
            Touched::Track(track, slot) => self.chains.get_mut(&track)?.save(slot),
            Touched::Clip(clip, slot) => self.clips.get_mut(&clip)?.save(slot),
            Touched::Master(slot) => self.master.as_mut()?.save(slot),
            Touched::Monitor(slot) => self.monitor.as_mut()?.save(slot),
        }
    }

    fn on_track(&mut self, track: TrackId, slot: usize, name: &str, index: i64) -> Result<(), String> {
        match self.chains.get_mut(&track) {
            Some(rack) => rack.on_track(slot, name, index),
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

    fn load_monitor(&mut self, slot: usize) -> Result<(), String> {
        match self.monitor.as_mut() {
            Some(rack) => rack.load_held_back(slot),
            None => Err("the monitoring chain has no plugins".into()),
        }
    }

    fn show_monitor(&mut self, slot: usize) -> Result<(), String> {
        match self.monitor.as_mut() {
            Some(rack) => rack.show(slot),
            None => Err("the monitoring chain has no plugins".into()),
        }
    }

    fn tweak(&mut self, track: TrackId, slot: usize, knob: usize, value: f32) {
        if let Some(rack) = self.chains.get_mut(&track) {
            rack.tweak(slot, knob, value);
        }
    }

    fn knob_readings(&mut self, track: TrackId, slot: usize) -> Vec<(String, f32, String)> {
        let Some(rack) = self.chains.get_mut(&track) else { return Vec::new() };
        rack.readings(slot).into_iter().map(|found| (found.name, found.value, found.text)).collect()
    }

    fn knob_from_text(&mut self, track: TrackId, slot: usize, knob: usize, text: &str) -> Result<f32, String> {
        self.chains.get_mut(&track).ok_or("that track has no plugins running")?.from_text(slot, knob, text)
    }

    fn turn_and_keep(&mut self, track: TrackId, slot: usize, knob: usize, value: f32) -> Option<Vec<u8>> {
        self.chains.get_mut(&track)?.turn_and_save(slot, knob, value)
    }

    fn latency(&self, track: TrackId) -> usize {
        self.chains.get(&track).map(|rack| rack.latency()).unwrap_or(0)
    }

    fn takes_latency(&self, track: TrackId) -> usize {
        self.chains.get(&track).map(|rack| rack.takes_latency()).unwrap_or(0)
    }

    fn heard_latency(&self, track: TrackId) -> usize {
        self.chains.get(&track).map(Rack::heard_latency).unwrap_or(0)
    }

    fn heard_takes_latency(&self, track: TrackId) -> usize {
        self.chains.get(&track).map(Rack::heard_takes_latency).unwrap_or(0)
    }

    fn process_clip(&mut self, clip: ClipId, audio: &mut [[f32; 2]]) {
        let Self { clips, scratch, falls, spent_in_plugins, .. } = self;
        let Some(rack) = clips.get_mut(&clip) else { return };
        if rack.is_empty() {
            return;
        }
        let began = std::time::Instant::now();
        scratch.clear();
        scratch.extend_from_slice(audio);
        rack.process(scratch);
        note_plugin_time(spent_in_plugins, began);
        if rack.has_fallen() {
            tell_falls(falls, rack.take_fallen(), |slot| Spot::Clip(clip, slot));
        }
        let shared = scratch.len().min(audio.len());
        audio[..shared].copy_from_slice(&scratch[..shared]);
    }

    fn process_clip_at(&mut self, clip: ClipId, audio: &mut [[f32; 2]], at: loupe_engine::Frames) {
        let Self { clips, scratch, spent_in_plugins, .. } = self;
        let Some(rack) = clips.get_mut(&clip) else { return };
        if rack.is_empty() {
            return;
        }
        let began = std::time::Instant::now();
        scratch.clear();
        scratch.extend_from_slice(audio);
        rack.process_at(scratch, at as i64);
        note_plugin_time(spent_in_plugins, began);
        let shared = scratch.len().min(audio.len());
        audio[..shared].copy_from_slice(&scratch[..shared]);
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
        self.chains.values().chain(self.clips.values()).chain(self.master.iter()).chain(self.monitor.iter()).any(|rack| rack.still_opening())
    }

    fn nudge(&mut self) {
        self.publish();
        self.gather();
    }

    fn harvest_clips(&mut self) -> Vec<(ClipId, usize, Vec<u8>)> {
        all_at_once(self.clips.iter_mut().map(|(id, rack)| (*id, rack)).collect())
    }

    fn harvest_master(&mut self) -> Vec<(usize, Vec<u8>)> {
        match self.master.as_mut() {
            Some(rack) => rack.all_they_remember(),
            None => Vec::new(),
        }
    }

    fn harvest_monitor(&mut self) -> Vec<(usize, Vec<u8>)> {
        match self.monitor.as_mut() {
            Some(rack) => rack.all_they_remember(),
            None => Vec::new(),
        }
    }

    fn harvest(&mut self) -> Vec<(TrackId, usize, Vec<u8>)> {
        all_at_once(self.chains.iter_mut().map(|(id, rack)| (*id, rack)).collect())
    }

    fn everything_saved(&mut self) -> Saved {
        let mut everyone: Vec<(Which, &mut Rack)> = Vec::new();
        everyone.extend(self.chains.iter_mut().map(|(id, rack)| (Which::Track(*id), rack)));
        everyone.extend(self.clips.iter_mut().map(|(id, rack)| (Which::Clip(*id), rack)));
        everyone.extend(self.master.iter_mut().map(|rack| (Which::Master, rack)));
        everyone.extend(self.monitor.iter_mut().map(|rack| (Which::Monitor, rack)));
        let mut saved = Saved::default();
        for (which, slot, state) in all_at_once(everyone) {
            match which {
                Which::Track(id) => saved.tracks.push((id, slot, state)),
                Which::Clip(id) => saved.clips.push((id, slot, state)),
                Which::Master => saved.master.push((slot, state)),
                Which::Monitor => saved.monitor.push((slot, state)),
            }
        }
        saved
    }

    fn process_with(&mut self, track: TrackId, audio: &mut [[f32; 2]], side: &[[f32; 2]]) {
        self.through_track(track, audio, side, Rack::process_with);
    }

    fn hear_with(&mut self, track: TrackId, audio: &mut [[f32; 2]], side: &[[f32; 2]]) {
        self.through_track(track, audio, side, Rack::hear_with);
    }

    fn whole_waves_at_once(&self) -> bool {
        // On the audio thread too: the crew is already up, so a wave of independent tracks
        // runs its chains side by side instead of one after another.
        true
    }

    fn takes_its_time(&mut self, spare: bool) {
        self.off_the_audio_thread = spare;
        self.keep_time();
    }

    fn process_wave(&mut self, jobs: &mut [loupe_engine::Job<'_>]) {
        let began = std::time::Instant::now();
        self.run_a_whole_wave(jobs);
        self.note_plugin_time(began);
    }

    fn plugin_spending(&self) -> PluginSpending {
        PluginSpending {
            nanoseconds: self.spent_in_plugins.load(Ordering::Relaxed),
            running: self.running(),
            held_back: self.held_back(),
            waves: self.waves.load(Ordering::Relaxed),
            tracks_in_waves: self.tracks_in_waves.load(Ordering::Relaxed),
            at_once: tracks_at_once(),
            late_replies: self.every_rack().map(Rack::late_replies).sum(),
        }
    }

    fn process_takes(&mut self, track: TrackId, audio: &mut [[f32; 2]]) {
        let Self { chains, scratch, falls, spent_in_plugins, .. } = self;
        let Some(rack) = chains.get_mut(&track) else { return };
        if !rack.has_takes() {
            return;
        }
        let began = std::time::Instant::now();
        scratch.clear();
        scratch.extend_from_slice(audio);
        rack.hear_takes(scratch);
        note_plugin_time(spent_in_plugins, began);
        if rack.has_fallen() {
            tell_falls(falls, rack.take_fallen(), |slot| Spot::Track(track, slot));
        }
        let shared = scratch.len().min(audio.len());
        audio[..shared].copy_from_slice(&scratch[..shared]);
    }

    fn process_monitor(&mut self, audio: &mut [[f32; 2]]) {
        let Self { monitor, scratch, falls, spent_in_plugins, .. } = self;
        let Some(rack) = monitor.as_mut() else { return };
        if rack.is_empty() {
            return;
        }
        let began = std::time::Instant::now();
        scratch.clear();
        scratch.extend_from_slice(audio);
        rack.hear_with(scratch, &[]);
        note_plugin_time(spent_in_plugins, began);
        if rack.has_fallen() {
            tell_falls(falls, rack.take_fallen(), Spot::Monitor);
        }
        let shared = scratch.len().min(audio.len());
        audio[..shared].copy_from_slice(&scratch[..shared]);
    }

    fn song_goes(&mut self, song: loupe_engine::Song) {
        let told = loupe_plugins::wire::Song {
            at: song.at as i64,
            tempo: song.tempo,
            beats: song.beats,
            beat_unit: song.beat_unit,
            playing: song.playing,
            recording: song.recording,
        };
        for rack in self.chains.values_mut().chain(self.clips.values_mut()).chain(self.master.iter_mut()).chain(self.monitor.iter_mut()) {
            rack.told_the_time(told);
        }
    }

    fn monitor_chain_latency(&self) -> usize {
        self.monitor.as_ref().map(Rack::heard_latency).unwrap_or(0)
    }

    fn process_master(&mut self, audio: &mut [[f32; 2]]) {
        let Self { master, scratch, falls, spent_in_plugins, .. } = self;
        let Some(rack) = master.as_mut() else { return };
        if rack.is_empty() {
            return;
        }
        let began = std::time::Instant::now();
        scratch.clear();
        scratch.extend_from_slice(audio);
        rack.process(scratch);
        note_plugin_time(spent_in_plugins, began);
        if rack.has_fallen() {
            tell_falls(falls, rack.take_fallen(), Spot::Master);
        }
        let shared = scratch.len().min(audio.len());
        audio[..shared].copy_from_slice(&scratch[..shared]);
    }
}

const MOST_TRACKS_AT_ONCE: usize = 256;
const FEWEST_TRACKS_AT_ONCE: usize = 4;
const TRACKS_A_CORE_CARRIES: usize = 2;

/// How many crew threads stand by to run tracks side by side: half the cores, a few at least,
/// and never so many that they crowd out the plugin hosts.
fn crew_hands() -> usize {
    std::thread::available_parallelism().map_or(2, |cores| cores.get() / 2).clamp(2, 8)
}

fn tracks_at_once() -> usize {
    std::thread::available_parallelism()
        .map_or(FEWEST_TRACKS_AT_ONCE, |cores| cores.get() * TRACKS_A_CORE_CARRIES)
        .clamp(FEWEST_TRACKS_AT_ONCE, MOST_TRACKS_AT_ONCE)
}

fn note_plugin_time(spent: &AtomicU64, began: std::time::Instant) {
    spent.fetch_add(began.elapsed().as_nanos() as u64, Ordering::Relaxed);
}

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
            assert!(racks.whole_waves_at_once(), "waves run on the crew on and off the audio thread alike");
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

    const LIMITER: usize = 2;

    fn a_click_track(project: &mut Project, name: &str, start: u64) -> TrackId {
        let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: name.into() }) else { panic!("no track") };
        let mut frames = vec![[0.0f32; 2]; 4_800];
        frames[0] = [0.4, 0.4];
        let source = Arc::new(Source::from_frames("click", frames));
        project.apply(Command::AddClip { track, source, start }).unwrap();
        track
    }

    fn whole_mix(project: &Project, racks: &mut Racks, frames: usize) -> Vec<[f32; 2]> {
        racks.follow(project);
        let mut out = vec![[0.0f32; 2]; frames];
        let mut scratch = loupe_engine::Mixdown::default();
        loupe_engine::render_through(project, 0, &mut out, &mut scratch, Some(racks));
        out
    }

    fn loud_frames(audio: &[[f32; 2]]) -> Vec<usize> {
        audio.iter().enumerate().filter(|(_, frame)| frame[0].abs() > 0.01).map(|(at, _)| at).collect()
    }

    #[test]
    fn a_track_whose_chain_is_off_still_lands_with_the_rest_of_the_mix() {
        const CLICK: usize = 1_000;
        let mut project = Project::new(48_000);
        let latent = a_click_track(&mut project, "Vox", CLICK as u64);
        a_click_track(&mut project, "Gtr", CLICK as u64);
        project.apply(Command::AddFx { track: latent, fx: fx(loupe_plugins::BUILT_IN, LIMITER, &[]) }).unwrap();
        let mut on = racks(false);
        let heard = whole_mix(&project, &mut on, 4_096);
        let lag = on.latency(latent);
        assert!(lag > 0, "the limiter reported no latency, so this proves nothing");
        assert_eq!(loud_frames(&heard), vec![CLICK], "the two tracks did not land together with the chain on");
        project.apply(Command::BypassChain { track: latent, bypassed: true }).unwrap();
        let mut off = racks(false);
        let held = whole_mix(&project, &mut off, 4_096);
        assert_eq!(off.latency(latent), lag, "the chain stopped reporting its latency once it was off");
        assert!(off.chains[&latent].chain_bypassed(), "the rack never heard that the chain was off");
        assert_eq!(loud_frames(&held), vec![CLICK], "the track with the chain off moved in the mix");
        assert_eq!(held[CLICK], [0.8, 0.8], "the two clicks did not add up dry on the same frame");
    }

    #[test]
    fn listening_dry_takes_every_chain_out_of_circuit_without_moving_the_mix() {
        const CLICK: usize = 1_000;
        let mut project = Project::new(48_000);
        let latent = a_click_track(&mut project, "Vox", CLICK as u64);
        let plain = a_click_track(&mut project, "Gtr", CLICK as u64);
        let clip = project.tracks.iter().find(|found| found.id == plain).expect("the track").clips[0].id;
        project.apply(Command::AddFx { track: latent, fx: fx(loupe_plugins::BUILT_IN, LIMITER, &[]) }).unwrap();
        project.apply(Command::AddMasterFx(fx(loupe_plugins::BUILT_IN, LIMITER, &[]))).unwrap();
        project.apply(Command::AddClipFx { clip, fx: fx(loupe_plugins::BUILT_IN, 1, &[]) }).unwrap();
        let mut on = racks(false);
        let heard = whole_mix(&project, &mut on, 4_096);
        let lag = on.master.as_ref().map(Rack::latency).unwrap_or(0);
        assert!(lag > 0, "the master limiter reported no latency");
        assert_eq!(loud_frames(&heard), vec![CLICK + lag], "the mix was not in time to begin with");
        project.apply(Command::BypassAllFx { bypassed: true }).unwrap();
        let mut off = racks(false);
        let held = whole_mix(&project, &mut off, 4_096);
        assert!(off.chains[&latent].chain_bypassed(), "a track chain kept running");
        assert!(off.clips[&clip].chain_bypassed(), "a clip chain kept running");
        assert!(off.master.as_ref().is_some_and(Rack::chain_bypassed), "the master chain kept running");
        assert_eq!(loud_frames(&held), vec![CLICK + lag], "listening dry moved the mix");
        assert_eq!(held[CLICK + lag], [0.8, 0.8], "the mix did not come through dry");
    }


    fn a_singer_in_a_folder_with_every_kind_of_chain() -> (Project, TrackId, TrackId) {
        let mut project = Project::new(48_000);
        let Ok(Outcome::Track(bus)) = project.apply(Command::AddTrack { name: "Vox Bus".into() }) else { panic!() };
        let Ok(Outcome::Track(vox)) = project.apply(Command::AddTrack { name: "Vox".into() }) else { panic!() };
        project.apply(Command::SetTrackParent { track: vox, parent: Some(bus) }).unwrap();
        let rec = Fx { record: true, ..fx(loupe_plugins::BUILT_IN, LIMITER, &[]) };
        project.apply(Command::AddFx { track: vox, fx: rec }).unwrap();
        project.apply(Command::AddFx { track: bus, fx: fx(loupe_plugins::BUILT_IN, LIMITER, &[]) }).unwrap();
        project.apply(Command::AddMonitorFx(fx(loupe_plugins::BUILT_IN, LIMITER, &[]))).unwrap();
        (project, vox, bus)
    }

    fn sung_into(racks: &mut Racks, vox: TrackId, bus: TrackId) -> Vec<[f32; 2]> {
        let mut song = vec![[0.0f32; 2]; 512 * 16];
        song[512] = [0.5, 0.5];
        let mut heard = Vec::new();
        for block in song.chunks(512) {
            let mut block = block.to_vec();
            racks.process_takes(vox, &mut block);
            racks.hear_with(bus, &mut block, &[]);
            racks.process_monitor(&mut block);
            heard.extend_from_slice(&block);
        }
        heard
    }

    #[test]
    fn listening_dry_puts_no_delay_between_the_singer_and_their_headphones() {
        let (mut project, vox, bus) = a_singer_in_a_folder_with_every_kind_of_chain();
        let mut on = racks(false);
        on.follow(&project);
        let lag = on.latency(bus);
        assert!(lag > 0, "the limiter reported no latency, so this proves nothing");
        assert_eq!(loupe_engine::monitor_latency(&project, vox, &on), 3 * lag, "the Rec, folder and monitoring limiters are all heard");
        assert_eq!(loud_frames(&sung_into(&mut on, vox, bus)), vec![512 + 3 * lag]);
        project.apply(Command::BypassAllFx { bypassed: true }).unwrap();
        let mut off = racks(false);
        off.follow(&project);
        assert_eq!(off.latency(bus), lag, "the mix must still hold the place of the folder's chain");
        let mut held = racks(false);
        held.follow(&project);
        let mut song = vec![[0.0f32; 2]; 512 * 16];
        song[512] = [0.5, 0.5];
        for block in song.chunks_mut(512) {
            held.process_with(bus, block, &[]);
        }
        assert_eq!(loud_frames(&song), vec![512 + lag], "the mix stopped holding the place of a folder chain that is off");
        let heard = sung_into(&mut off, vox, bus);
        assert_eq!(loud_frames(&heard), vec![512], "with every chain off the singer still heard the chains' delay");
        assert_eq!(heard[512], [0.5, 0.5], "with every chain off the voice still went through a plugin");
        assert_eq!(loupe_engine::monitor_latency(&project, vox, &off), 0, "the lag shown to the singer counts chains that are off");
    }
    #[test]
    fn a_master_chain_that_is_off_leaves_the_mix_dry_and_in_time() {
        let mut project = Project::new(48_000);
        a_click_track(&mut project, "Vox", 0);
        project.apply(Command::AddMasterFx(fx(loupe_plugins::BUILT_IN, LIMITER, &[]))).unwrap();
        let mut on = racks(false);
        let heard = whole_mix(&project, &mut on, 4_096);
        let lag = on.master.as_ref().map(Rack::latency).unwrap_or(0);
        assert!(lag > 0, "the master limiter reported no latency");
        assert_eq!(loud_frames(&heard), vec![lag]);
        project.apply(Command::BypassMasterChain { bypassed: true }).unwrap();
        let mut off = racks(false);
        let held = whole_mix(&project, &mut off, 4_096);
        assert!(off.master.as_ref().is_some_and(Rack::chain_bypassed), "the master rack never heard that it was off");
        assert_eq!(loud_frames(&held), vec![lag], "the master chain going off moved the whole mix");
        assert_eq!(held[lag], [0.4, 0.4], "the click did not come through the master dry");
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
    fn a_master_plugins_settings_are_saved_and_come_back() {
        let mut project = Project::new(48_000);
        project.apply(Command::AddMasterFx(fx(loupe_plugins::BUILT_IN, 1, &[]))).unwrap();
        let mut racks = racks(false);
        racks.follow(&project);
        let found = racks.harvest_master();
        assert_eq!(found.len(), 1, "the master rack was not harvested");
        assert!(!found[0].1.is_empty(), "the master plugin saved nothing");
        for (slot, state) in found {
            project.apply(Command::SetMasterFxState { slot, state }).unwrap();
        }
        let kept = project.master_fx[0].state.clone();
        let text = loupe_engine::SavedProject::capture(&project, |_| None).to_text();
        let read = loupe_engine::SavedProject::parse(&text).expect("the song reads back");
        let (reopened, _) = read.build(&[], 48_000);
        assert_eq!(reopened.master_fx[0].state, kept, "the master plugin came back on defaults");
        let mut again = Racks::new(48_000, 512, Peeks::default(), Arc::new(AtomicBool::new(false)), Falls::default());
        again.follow(&reopened);
        assert_eq!(again.master.as_ref().map(Rack::len), Some(1));
        assert_eq!(again.harvest_master(), vec![(0, kept)]);
    }

    #[test]
    fn the_monitoring_chain_runs_on_its_own_rack_and_its_settings_are_saved() {
        let mut project = Project::new(48_000);
        let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: "Vox 1".into() }) else { panic!("no track") };
        let mut taking = fx(loupe_plugins::BUILT_IN, 1, &[]);
        taking.record = true;
        project.apply(Command::AddFx { track, fx: taking }).unwrap();
        project.apply(Command::AddMasterFx(fx(loupe_plugins::BUILT_IN, 2, &[]))).unwrap();
        project.apply(Command::AddMonitorFx(fx(loupe_plugins::BUILT_IN, 1, &[]))).unwrap();
        let mut racks = racks(false);
        racks.follow(&project);
        assert_eq!(racks.monitor.as_ref().map(Rack::len), Some(1), "the monitoring chain has no rack of its own");
        assert_eq!(racks.master.as_ref().map(Rack::len), Some(1));
        assert_eq!(racks.chains.get(&track).map(Rack::len), Some(1));
        assert_eq!(racks.chains[&track].takes_latency(), racks.takes_latency(track));

        let found = racks.harvest_monitor();
        assert_eq!(found.len(), 1, "the monitoring rack was not harvested");
        assert!(!found[0].1.is_empty(), "the monitoring plugin saved nothing");
        for (slot, state) in racks.everything_saved().monitor {
            project.apply(Command::SetMonitorFxState { slot, state }).unwrap();
        }
        let kept = project.monitor_fx[0].state.clone();
        assert!(!kept.is_empty());
        let text = loupe_engine::SavedProject::capture(&project, |_| None).to_text();
        let (reopened, _) = loupe_engine::SavedProject::parse(&text).expect("it reads back").build(&[], 48_000);
        assert_eq!(reopened.monitor_fx[0].state, kept, "the monitoring plugin came back on defaults");

        project.apply(Command::RemoveMonitorFx(0)).unwrap();
        racks.follow(&project);
        assert!(racks.monitor.is_none(), "the monitoring rack stayed behind with nothing on it");
        assert_eq!(racks.monitor_chain_latency(), 0);
    }

    #[test]
    fn the_monitoring_chain_is_heard_but_never_printed_or_mixed() {
        let mut project = Project::new(48_000);
        let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: "Vox 1".into() }) else { panic!("no track") };
        let source = Arc::new(Source::from_frames("take", vec![[0.4, -0.4]; 4_800]));
        project.apply(Command::AddClip { track, source, start: 0 }).unwrap();
        project.apply(Command::AddMonitorFx(fx(loupe_plugins::BUILT_IN, 1, &[]))).unwrap();
        let mut racks = racks(false);
        racks.follow(&project);
        let mut out = vec![[0.0f32; 2]; 512];
        let mut scratch = loupe_engine::Mixdown::default();
        loupe_engine::render_through(&project, 0, &mut out, &mut scratch, Some(&mut racks));
        let mut plain = vec![[0.0f32; 2]; 512];
        let mut other = loupe_engine::Mixdown::default();
        loupe_engine::render_through(&project, 0, &mut plain, &mut other, None);
        assert_eq!(out, plain, "the monitoring chain was heard in the mix");
        assert!(crate::printing::wanted(&project.tracks[0].fx).is_empty(), "a printed take would pick up the monitoring chain");

        let mut sound = vec![[0.5f32, -0.5]; 512];
        racks.process_monitor(&mut sound);
        assert!(sound.iter().any(|frame| (frame[0] - 0.5).abs() > 1e-6), "the monitoring chain did nothing to the input");
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

impl Drop for Racks {
    fn drop(&mut self) {
        let going = self.everything_leaving();
        let_them_go(going);
    }
}


#[cfg(all(test, target_os = "linux"))]
mod closing_tests {
    use super::*;
    use loupe_engine::{Command, Fx, Outcome, Source};
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::time::{Duration, Instant};

    const HOW_LONG_A_PLUGIN_TAKES_TO_SAVE: &str = "0.4";

    struct Fake(PathBuf);

    impl Drop for Fake {
        fn drop(&mut self) {
            if let Some(folder) = self.0.parent() {
                let _ = std::fs::remove_dir_all(folder);
            }
        }
    }

    fn slow_to_answer(name: &str) -> Fake {
        let folder = std::env::temp_dir().join(format!("loupe-closing-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("loupe-host");
        let script = format!(
            "#!/bin/sh\nwhile IFS= read -r ask ; do\n  case \"$ask\" in\n    load*) printf 'loaded\\t2\\t2\\t0\\t0\\n' ;;\n    save) sleep {HOW_LONG_A_PLUGIN_TAKES_TO_SAVE} ; printf 'state\\tdeadbeef\\n' ;;\n    knobs) printf 'knobs\\tGain\\n' ;;\n    quit) sleep 5 ; exit 0 ;;\n    *) printf 'fine\\n' ;;\n  esac\ndone\n"
        );
        std::fs::write(&file, script).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        Fake(file)
    }

    fn plugin(name: &str) -> Fx {
        Fx { path: PathBuf::from("/nowhere/Slow.vst3"), index: 0, name: name.into(), bypassed: false, state: Vec::new(), record: false, mix: 1.0 }
    }

    fn a_song_with_plugins_everywhere() -> Project {
        let mut project = Project::new(48_000);
        for at in 0..6 {
            let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: format!("T{at}") }) else { panic!("no track") };
            let source = Arc::new(Source::from_frames("take", vec![[0.1, 0.1]; 4_800]));
            let Ok(Outcome::Clip(clip)) = project.apply(Command::AddClip { track, source, start: 0 }) else { panic!("no clip") };
            project.apply(Command::AddFx { track, fx: plugin(&format!("A{at}")) }).unwrap();
            project.apply(Command::AddFx { track, fx: plugin(&format!("B{at}")) }).unwrap();
            if at < 2 {
                project.apply(Command::AddClipFx { clip, fx: plugin(&format!("C{at}")) }).unwrap();
            }
        }
        for at in 0..3 {
            project.apply(Command::AddMasterFx(plugin(&format!("M{at}")))).unwrap();
        }
        project
    }

    fn all_running(host: &Path, project: &Project) -> Racks {
        let mut racks = Racks::new(48_000, 512, Peeks::default(), Arc::new(AtomicBool::new(false)), Falls::default());
        racks.host = host.to_path_buf();
        racks.follow(project);
        let gave_up = Instant::now();
        while racks.still_opening() {
            assert!(gave_up.elapsed() < Duration::from_secs(120), "the fake hosts never started");
            racks.follow(project);
            std::thread::sleep(Duration::from_millis(5));
        }
        racks
    }

    fn hosts_in(racks: &Racks) -> Vec<u32> {
        let mut out = Vec::new();
        for rack in racks.chains.values().chain(racks.clips.values()).chain(racks.master.iter()) {
            out.extend((0..rack.len()).filter_map(|slot| rack.host_pid(slot)));
        }
        out
    }

    fn still_about(pid: u32) -> bool {
        Path::new(&format!("/proc/{pid}")).exists()
    }

    fn nobody_left(hosts: &[u32]) {
        let gave_up = Instant::now();
        while let Some(pid) = hosts.iter().copied().find(|pid| still_about(*pid)) {
            assert!(gave_up.elapsed() < Duration::from_secs(10), "host {pid} was left behind");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn every_plugin_in_the_song_saves_in_one_wave() {
        let fake = slow_to_answer("one-wave");
        let project = a_song_with_plugins_everywhere();
        let mut racks = all_running(&fake.0, &project);
        let hosts = hosts_in(&racks);
        assert_eq!(hosts.len(), 17, "the song should be running 17 plugins");
        let began = Instant::now();
        let saved = racks.everything_saved();
        let took = began.elapsed();
        assert_eq!(saved.tracks.len(), 12);
        assert_eq!(saved.clips.len(), 2);
        assert_eq!(saved.master.len(), 3);
        assert!(took < Duration::from_millis(1_600), "saving 17 plugins took {took:?}, as if a chain at a time");
        drop(racks);
        nobody_left(&hosts);
    }

    #[test]
    fn closing_a_song_lets_the_window_carry_on() {
        let fake = slow_to_answer("carry-on");
        let project = a_song_with_plugins_everywhere();
        let mut racks = all_running(&fake.0, &project);
        let hosts = hosts_in(&racks);
        assert_eq!(hosts.len(), 17);
        let began = Instant::now();
        racks.follow(&Project::new(48_000));
        let took = began.elapsed();
        assert!(took < Duration::from_millis(50), "closing the song held the window for {took:?}");
        assert!(racks.chains.is_empty() && racks.clips.is_empty() && racks.master.is_none());
        nobody_left(&hosts);
        drop(racks);
    }
}

#[cfg(all(test, target_os = "linux"))]
mod touch_tests {
    use super::*;
    use loupe_engine::{Command, Fx, Outcome, Source};
    use std::os::unix::fs::PermissionsExt;
    use std::time::Instant;

    struct Fake(PathBuf);

    impl Drop for Fake {
        fn drop(&mut self) {
            if let Some(folder) = self.0.parent() {
                let _ = std::fs::remove_dir_all(folder);
            }
        }
    }

    fn a_host_that_reports_moved_knobs(name: &str) -> Fake {
        let folder = std::env::temp_dir().join(format!("loupe-touched-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("loupe-host");
        let mark = folder.join("mark").display().to_string();
        let script = format!(
            "#!/bin/sh\nM={mark}\nwhile IFS= read -r ask ; do\n  case \"$ask\" in\n    load*) case \"$ask\" in *Moved*) : > \"$M.$$\" ; printf '11' > \"$M.$$.state\" ;; *) printf '22' > \"$M.$$.state\" ;; esac ; printf 'loaded\\t2\\t2\\t0\\t0\\n' ;;\n    touched) if [ -f \"$M.$$\" ] ; then rm -f \"$M.$$\" ; printf 'edited\\t1\\n' ; else printf 'edited\\t0\\n' ; fi ;;\n    save) printf 'state\\t%s\\n' \"$(cat $M.$$.state)\" ;;\n    knobs) printf 'knobs\\tGain\\n' ;;\n    quit) exit 0 ;;\n    *) printf 'fine\\n' ;;\n  esac\ndone\n"
        );
        std::fs::write(&file, script).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        Fake(file)
    }

    fn plugin(path: &str) -> Fx {
        Fx { path: PathBuf::from(path), index: 0, name: path.into(), bypassed: false, state: Vec::new(), record: false, mix: 1.0 }
    }

    fn a_song_with_knobs_everywhere() -> (Project, TrackId, ClipId) {
        let mut project = Project::new(48_000);
        let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: "Vox".into() }) else { panic!("no track") };
        let source = Arc::new(Source::from_frames("take", vec![[0.1, 0.1]; 480]));
        let Ok(Outcome::Clip(clip)) = project.apply(Command::AddClip { track, source, start: 0 }) else { panic!("no clip") };
        project.apply(Command::AddFx { track, fx: plugin("/nowhere/Moved.vst3") }).unwrap();
        project.apply(Command::AddFx { track, fx: plugin("/nowhere/Still.vst3") }).unwrap();
        project.apply(Command::AddClipFx { clip, fx: plugin("/nowhere/MovedClip.clap") }).unwrap();
        project.apply(Command::AddMasterFx(plugin("/nowhere/MovedGlue.vst3"))).unwrap();
        (project, track, clip)
    }

    fn all_running(host: &std::path::Path, project: &Project) -> Racks {
        let mut racks = Racks::new(48_000, 1, Peeks::default(), Arc::new(AtomicBool::new(false)), Falls::default());
        racks.host = host.to_path_buf();
        racks.follow(project);
        let gave_up = Instant::now();
        while racks.still_opening() {
            assert!(gave_up.elapsed() < Duration::from_secs(60), "the fake hosts never started");
            racks.follow(project);
            std::thread::sleep(Duration::from_millis(5));
        }
        racks
    }

    #[test]
    fn every_slot_that_moved_is_named_and_only_once() {
        let fake = a_host_that_reports_moved_knobs("named");
        let (project, track, clip) = a_song_with_knobs_everywhere();
        let mut racks = all_running(&fake.0, &project);
        let found = racks.plugins_touched();
        assert_eq!(found, vec![Touched::Track(track, 0), Touched::Clip(clip, 0), Touched::Master(0)]);
        assert!(racks.plugins_touched().is_empty(), "the racks kept reporting a knob that had already been read");
    }

    #[test]
    fn a_slot_gives_back_its_own_settings_and_nobody_elses() {
        let fake = a_host_that_reports_moved_knobs("settings");
        let (project, track, clip) = a_song_with_knobs_everywhere();
        let mut racks = all_running(&fake.0, &project);
        assert_eq!(racks.state_of(Touched::Track(track, 0)), Some(vec![0x11]));
        assert_eq!(racks.state_of(Touched::Track(track, 1)), Some(vec![0x22]));
        assert_eq!(racks.state_of(Touched::Clip(clip, 0)), Some(vec![0x11]));
        assert_eq!(racks.state_of(Touched::Master(0)), Some(vec![0x11]));
        assert_eq!(racks.state_of(Touched::Track(track, 9)), None);
        assert_eq!(racks.state_of(Touched::Clip(ClipId(9_999), 0)), None);
    }
}

#[cfg(test)]
mod lending_tests {
    use loupe_engine::Output;

    use super::*;

    fn holder() -> Lending<Box<dyn Chains>> {
        Lending::default()
    }

    #[test]
    fn a_set_the_app_holds_comes_straight_back() {
        let mut lending: Lending<u32> = Lending::default();
        lending.keep(7);
        assert!(lending.holding() && !lending.is_out());
        assert_eq!(lending.take(), Some(7));
        assert!(!lending.holding() && !lending.is_out());
    }

    #[test]
    fn a_set_that_went_out_is_not_held_here_as_well() {
        let mut lending: Lending<u32> = Lending::default();
        lending.keep(7);
        lending.gone_out();
        assert!(!lending.holding() && lending.is_out());
    }

    #[test]
    fn a_set_that_came_back_is_no_longer_out() {
        let mut lending: Lending<u32> = Lending::default();
        lending.gone_out();
        assert_eq!(lending.came_back(3), 3);
        assert!(!lending.is_out());
    }

    #[test]
    fn an_engine_that_went_away_takes_its_loan_with_it() {
        let mut lending: Lending<u32> = Lending::default();
        lending.gone_out();
        lending.forget();
        assert!(!lending.is_out() && !lending.holding());
    }

    #[test]
    fn with_no_sound_output_the_app_keeps_the_racks_itself() {
        let mut engine = Engine::start(Output::Silent);
        let mut held = holder();
        let made = borrow_or_make(&mut engine, &mut held, &Peeks::default(), &PluginsOff::default(), &Falls::default());
        assert!(made.is_some());
        lend(&mut engine, &mut held, made.unwrap());
        assert!(held.holding() && !held.is_out());
        assert!(borrow(&mut engine, &mut held).is_some());
    }

    #[test]
    fn a_timed_out_handover_can_be_asked_again_and_strands_nothing() {
        let mut engine = Engine::start(Output::Silent);
        let mut held = holder();
        held.gone_out();
        assert!(borrow(&mut engine, &mut held).is_none());
        assert!(held.is_out() && !held.holding());
        assert!(borrow_or_make(&mut engine, &mut held, &Peeks::default(), &PluginsOff::default(), &Falls::default()).is_none());
        assert!(held.is_out() && !held.holding());
    }
}

#[cfg(all(test, target_os = "linux"))]
mod timing_tests {
    use super::*;
    use loupe_engine::{Command, Fx, Outcome, Source};
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::time::{Duration, Instant};

    struct Fake(PathBuf);

    impl Drop for Fake {
        fn drop(&mut self) {
            if let Some(folder) = self.0.parent() {
                let _ = std::fs::remove_dir_all(folder);
            }
        }
    }

    impl Fake {
        fn told(&self) -> Vec<String> {
            let folder = self.0.parent().expect("the fake has a folder");
            std::fs::read_to_string(folder.join("timing")).unwrap_or_default().lines().map(str::to_string).collect()
        }
    }

    fn a_host_that_writes_down_the_time(name: &str) -> Fake {
        let folder = std::env::temp_dir().join(format!("loupe-racks-timing-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("loupe-host");
        let told = folder.join("timing").display().to_string();
        let script = format!(
            "#!/bin/sh\nT={told}\nwhile IFS= read -r ask ; do\n  case \"$ask\" in\n    load*) printf 'loaded\\t2\\t2\\t0\\t0\\n' ;;\n    timing*) printf '%s\\n' \"$ask\" >> $T ;;\n    process*) dd bs=1 count=13 of=/dev/null 2>/dev/null ; printf 'B\\001' ; dd if=/dev/zero bs=1 count=11 2>/dev/null ;;\n    quit) exit 0 ;;\n    *) printf 'fine\\n' ;;\n  esac\ndone\n"
        );
        std::fs::write(&file, script).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        Fake(file)
    }

    fn a_song_at(bpm: f64) -> Project {
        let mut project = Project::new(48_000);
        project.apply(Command::SetBpm(bpm)).unwrap();
        let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: "Beat".into() }) else { panic!("no track") };
        let source = Arc::new(Source::from_frames("beat", vec![[0.2, -0.2]; 4_800]));
        project.apply(Command::AddClip { track, source, start: 0 }).unwrap();
        let fx = Fx { path: PathBuf::from("/nowhere/AutoKey.vst3"), index: 0, name: "Auto-Key".into(), bypassed: false, state: Vec::new(), record: false, mix: 1.0 };
        project.apply(Command::AddFx { track, fx }).unwrap();
        project
    }

    fn all_running(host: &Path, project: &Project) -> Racks {
        let mut racks = Racks::new(48_000, 1, Peeks::default(), Arc::new(AtomicBool::new(false)), Falls::default());
        racks.host = host.to_path_buf();
        racks.follow(project);
        let gave_up = Instant::now();
        while racks.still_opening() {
            assert!(gave_up.elapsed() < Duration::from_secs(60), "the fake host never started");
            racks.follow(project);
            std::thread::sleep(Duration::from_millis(5));
        }
        racks
    }

    #[test]
    fn the_tempo_a_plugin_is_handed_is_the_tempo_of_the_song() {
        let fake = a_host_that_writes_down_the_time("tempo");
        let project = a_song_at(142.5);
        let mut racks = all_running(&fake.0, &project);
        let mut out = vec![[0.0f32; 2]; 1];
        let mut scratch = loupe_engine::Mixdown::default();
        loupe_engine::render_through(&project, 96_000, &mut out, &mut scratch, Some(&mut racks));
        let told = fake.told();
        assert_eq!(told.first().map(String::as_str), Some("timing\t96000\t142.5\t4\t4\t1\t0"), "the plugin was told {told:?}");
    }
}
