use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ceiling::{Ceiling, Seat};
use crate::sandbox::Sandbox;
use crate::wire::{Ask, Region, Reply};

const SETTLING_BLOCKS: usize = 24;
const SETTLING_WAIT: std::time::Duration = std::time::Duration::from_millis(2);
const BLOCKS_BEFORE_WAITING: usize = 4;
pub const QUIET: f32 = 1.0e-7;
pub const QUIET_SECONDS_BEFORE_DOZING: f32 = 10.0;

fn loudest(audio: &[[f32; 2]]) -> f32 {
    audio.iter().fold(0.0f32, |top, frame| top.max(frame[0].abs()).max(frame[1].abs()))
}

fn must_stay_awake(made: &dyn loupe_stock::Effect) -> bool {
    made.keeps_its_own_clock() || made.scopes().is_some() || made.meter().is_some() || made.findings().is_some()
}

type Arrived = Result<Opened, (String, bool)>;

struct Dry {
    kept: Vec<[f32; 2]>,
    at: usize,
    held: usize,
    spare: Vec<[f32; 2]>,
}

impl Dry {
    fn empty() -> Self {
        Self { kept: Vec::new(), at: 0, held: 0, spare: Vec::new() }
    }

    fn room_for(&mut self, latency: usize, block: usize) {
        let want = latency + block.max(1);
        if self.held != latency || self.kept.len() < want {
            self.kept.clear();
            self.kept.resize(want.max(1), [0.0; 2]);
            self.spare.clear();
            self.spare.resize(block.max(1), [0.0; 2]);
            self.at = 0;
            self.held = latency;
        }
    }

    fn blend(&mut self, audio: &mut [[f32; 2]], mix: f32) {
        if self.kept.is_empty() {
            return;
        }
        let wet = mix.clamp(0.0, 1.0);
        let dry = 1.0 - wet;
        let room = self.kept.len();
        for (at, frame) in audio.iter_mut().enumerate() {
            let was = self.spare.get(at).copied().unwrap_or([0.0; 2]);
            self.kept[self.at] = was;
            let behind = (self.at + room - self.held.min(room)) % room;
            let older = self.kept[behind];
            self.at = (self.at + 1) % room;
            frame[0] = frame[0] * wet + older[0] * dry;
            frame[1] = frame[1] * wet + older[1] * dry;
        }
    }

    fn slide(&mut self, frames: usize) {
        if self.kept.is_empty() {
            return;
        }
        let room = self.kept.len();
        for at in 0..frames {
            self.kept[self.at] = self.spare.get(at).copied().unwrap_or([0.0; 2]);
            self.at = (self.at + 1) % room;
        }
    }

    fn remember(&mut self, audio: &[[f32; 2]]) {
        if self.spare.len() < audio.len() {
            return;
        }
        self.spare[..audio.len()].copy_from_slice(audio);
    }
}

pub struct Slot {
    pub path: PathBuf,
    pub index: usize,
    pub name: String,
    pub bypassed: bool,
    pub record: bool,
    pub trouble: Option<String>,
    pub held_back: bool,
    pub latency: usize,
    pub ara: bool,
    placed: Option<Region>,
    host: Option<Sandbox>,
    built: Option<Box<dyn loupe_stock::Effect>>,
    coming: Option<std::sync::mpsc::Receiver<Arrived>>,
    queued: Option<Beginning>,
    kept_state: Vec<u8>,
    known: Vec<u8>,
    wanted_open: bool,
    pub mix: f32,
    dry: Dry,
    always_awake: bool,
    quiet_for: usize,
}

impl Slot {
    fn may_doze(&self) -> bool {
        !self.always_awake && !self.ara && !self.record
    }

    fn quiet_long_enough(&self, rate: u32) -> usize {
        (rate as f32 * QUIET_SECONDS_BEFORE_DOZING) as usize + self.latency
    }

    fn dozes_through(&mut self, audio: &[[f32; 2]], rate: u32) -> bool {
        if !self.may_doze() {
            return false;
        }
        if loudest(audio) > QUIET {
            self.quiet_for = 0;
            return false;
        }
        self.quiet_for >= self.quiet_long_enough(rate)
    }

    fn note_what_came_out(&mut self, audio: &[[f32; 2]], rate: u32) {
        let enough = self.quiet_long_enough(rate);
        if loudest(audio) > QUIET {
            self.quiet_for = 0;
            return;
        }
        self.quiet_for = (self.quiet_for + audio.len()).min(enough);
    }

    pub fn dozing(&self, rate: u32) -> bool {
        self.may_doze() && self.quiet_for >= self.quiet_long_enough(rate)
    }

    pub fn working(&self) -> bool {
        (self.host.is_some() || self.built.is_some() || self.on_its_way()) && self.trouble.is_none()
    }

    pub fn on_its_way(&self) -> bool {
        self.coming.is_some() || self.queued.is_some()
    }

    pub fn built_in(&self) -> bool {
        self.built.is_some()
    }

    fn settle_in(&mut self, opened: Opened, region: Option<&Region>) {
        self.latency = opened.latency;
        self.ara = opened.ara;
        self.placed = if opened.ara { region.cloned() } else { None };
        self.host = Some(opened.host);
        self.quiet_for = 0;
    }
}

struct Beginning {
    state: Vec<u8>,
    seat: Seat,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wanted {
    pub mix: f32,
    pub path: PathBuf,
    pub index: usize,
    pub name: String,
    pub bypassed: bool,
    pub state: Vec<u8>,
    pub record: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fallen {
    pub slot: usize,
    pub name: String,
    pub why: String,
}

pub struct Opened {
    host: Sandbox,
    latency: usize,
    ara: bool,
}

pub struct Rack {
    host: PathBuf,
    rate: u32,
    block: usize,
    slots: Vec<Slot>,
    fallen: Vec<Fallen>,
    region: Option<Region>,
    ceiling: Arc<Ceiling>,
}

impl Rack {
    pub fn new(host: PathBuf, rate: u32, block: usize) -> Self {
        Self::sharing(host, rate, block, Ceiling::for_this_computer())
    }

    pub fn with_room_for(host: PathBuf, rate: u32, block: usize, most: usize) -> Self {
        Self::sharing(host, rate, block, Ceiling::of(most))
    }

    pub fn sharing(host: PathBuf, rate: u32, block: usize, ceiling: Arc<Ceiling>) -> Self {
        Self { host, rate, block, slots: Vec::new(), fallen: Vec::new(), region: None, ceiling }
    }

    pub fn held_back(&self) -> usize {
        self.slots.iter().filter(|slot| slot.held_back).count()
    }

    pub fn has_fallen(&self) -> bool {
        !self.fallen.is_empty()
    }

    pub fn take_fallen(&mut self) -> Vec<Fallen> {
        std::mem::take(&mut self.fallen)
    }

    fn lost_host(&mut self, slot: usize, why: String, fell: bool) {
        let Some(found) = self.slots.get_mut(slot) else { return };
        found.host = None;
        found.trouble = Some(why.clone());
        if fell {
            self.fallen.push(Fallen { slot, name: found.name.clone(), why });
        }
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
            mix: 1.0,
            dry: Dry::empty(),
            path: path.to_path_buf(),
            index,
            name: name.to_string(),
            bypassed: false,
            record: false,
            trouble: None,
            held_back: false,
            latency: 0,
            ara: false,
            placed: None,
            host: None,
            built: None,
            coming: None,
            queued: None,
            kept_state: Vec::new(),
            known: Vec::new(),
            wanted_open: false,
            always_awake: false,
            quiet_for: 0,
        };
        if is_built_in(&slot.path) {
            match self.make_built(slot.index) {
                Ok(made) => {
                    slot.latency = made.latency();
                    slot.always_awake = must_stay_awake(made.as_ref());
                    slot.built = Some(made);
                }
                Err(why) => slot.trouble = Some(why),
            }
        } else {
            match self.open(&slot.path, slot.index) {
                Ok(opened) => slot.settle_in(opened, self.region.as_ref()),
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

    pub fn latency(&self) -> usize {
        self.slots.iter().filter(|slot| !slot.record && !slot.bypassed && slot.trouble.is_none()).map(|slot| slot.latency).sum()
    }

    pub fn tweak(&mut self, slot: usize, knob: usize, value: f32) {
        let Some(found) = self.slots.get_mut(slot) else { return };
        found.quiet_for = 0;
        if let Some(made) = found.built.as_mut() {
            made.set(knob, value);
            return;
        }
        let Some(host) = found.host.as_mut() else { return };
        let asked = host.ask(Ask::Turn { knob, value });
        let fell = host.gone();
        if let Err(why) = asked {
            self.lost_host(slot, why, fell);
        }
    }

    pub fn automate(&mut self, slot: usize, knob: usize, value: f32) {
        let Some(found) = self.slots.get_mut(slot) else { return };
        found.quiet_for = 0;
        if let Some(made) = found.built.as_mut() {
            let Some(param) = made.params().get(knob).copied() else { return };
            made.set(knob, param.from_position(value));
            return;
        }
        let Some(host) = found.host.as_mut() else { return };
        let asked = host.ask(Ask::Turn { knob, value });
        let fell = host.gone();
        if let Err(why) = asked {
            self.lost_host(slot, why, fell);
        }
    }

    pub fn knobs(&mut self, slot: usize) -> Vec<String> {
        let Some(found) = self.slots.get_mut(slot) else { return Vec::new() };
        if let Some(made) = found.built.as_ref() {
            return made.params().iter().map(|param| param.name.to_string()).collect();
        }
        let Some(host) = found.host.as_mut() else { return Vec::new() };
        match host.ask(Ask::Knobs) {
            Ok(Reply::Knobs(names)) => names,
            _ => Vec::new(),
        }
    }

    pub fn turn_and_save(&mut self, slot: usize, knob: usize, value: f32) -> Option<Vec<u8>> {
        let before = self.save(slot)?;
        self.tweak(slot, knob, value);
        let quiet = self.block.max(1);
        let mut saved = before.clone();
        for tries in 0..SETTLING_BLOCKS {
            let host = self.slots.get_mut(slot)?.host.as_mut()?;
            host.run(&mut vec![[0.0; 2]; quiet]).ok()?;
            saved = self.save(slot)?;
            if saved != before {
                break;
            }
            if tries >= BLOCKS_BEFORE_WAITING {
                std::thread::sleep(SETTLING_WAIT);
            }
        }
        Some(saved)
    }

    pub fn readings(&mut self, slot: usize) -> Vec<crate::wire::Reading> {
        let Some(host) = self.slots.get_mut(slot).and_then(|found| found.host.as_mut()) else { return Vec::new() };
        match host.ask(Ask::Readings) {
            Ok(Reply::Readings(found)) => found,
            _ => Vec::new(),
        }
    }

    pub fn from_text(&mut self, slot: usize, knob: usize, text: &str) -> Result<f32, String> {
        let host = self.slots.get_mut(slot).and_then(|found| found.host.as_mut()).ok_or("that plugin is not running")?;
        match host.ask(Ask::FromText { knob, text: text.to_string() }) {
            Ok(Reply::Value(value)) => Ok(value),
            Ok(Reply::Trouble(why)) => Err(why),
            Ok(_) => Err("the plugin host answered something else".into()),
            Err(why) => Err(why),
        }
    }

    pub fn host_pid(&self, slot: usize) -> Option<u32> {
        self.slots.get(slot)?.host.as_ref().map(Sandbox::pid)
    }

    pub fn built_at(&self, slot: usize) -> Option<&dyn loupe_stock::Effect> {
        self.slots.get(slot)?.built.as_deref()
    }

    pub fn bypass(&mut self, slot: usize, bypassed: bool) {
        if let Some(found) = self.slots.get_mut(slot) {
            found.bypassed = bypassed;
            found.quiet_for = 0;
        }
    }

    pub fn dozing(&self) -> usize {
        self.slots.iter().filter(|slot| slot.dozing(self.rate)).count()
    }

    pub fn revive(&mut self, slot: usize) -> Result<(), String> {
        let Some(found) = self.slots.get_mut(slot) else {
            return Err("there is no such slot".into());
        };
        found.host = None;
        found.trouble = None;
        found.held_back = false;
        let path = found.path.clone();
        let index = found.index;
        match self.open(&path, index) {
            Ok(opened) => {
                self.slots[slot].settle_in(opened, self.region.as_ref());
                Ok(())
            }
            Err(why) => {
                self.slots[slot].trouble = Some(why.clone());
                Err(why)
            }
        }
    }

    pub fn still_opening(&self) -> bool {
        self.slots.iter().any(|slot| slot.on_its_way())
    }

    pub fn open_waiting(&mut self) -> Vec<String> {
        self.take_arrivals();
        let waiting: Vec<usize> = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.wanted_open && slot.host.is_some())
            .map(|(at, _)| at)
            .collect();
        let mut troubles = Vec::new();
        for slot in waiting {
            self.slots[slot].wanted_open = false;
            if let Err(why) = self.show(slot) {
                troubles.push(why);
            }
        }
        troubles
    }

    pub fn show(&mut self, slot: usize) -> Result<(), String> {
        self.take_arrivals();
        if let Some(found) = self.slots.get_mut(slot) {
            if found.on_its_way() {
                found.wanted_open = true;
                return Ok(());
            }
        }
        if let Some(host) = self.slots.get(slot).and_then(|found| found.host.as_ref()) {
            host.may_come_forward();
        }
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
        let asked = host.ask(ask);
        let fell = host.gone();
        match asked {
            Ok(Reply::Fine) => Ok(()),
            Ok(Reply::Trouble(why)) => Err(why),
            Ok(other) => Err(format!("the plugin host answered out of turn: {other:?}")),
            Err(why) => {
                self.lost_host(slot, why.clone(), fell);
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
        let asked = host.ask(Ask::Save);
        let fell = host.gone();
        match asked {
            Ok(Reply::State(state)) => {
                found.known = state.clone();
                Some(state)
            }
            Ok(Reply::Trouble(why)) => {
                self.lost_host(slot, why, false);
                None
            }
            Ok(_) => None,
            Err(why) => {
                self.lost_host(slot, why, fell);
                None
            }
        }
    }

    pub fn process(&mut self, audio: &mut Vec<[f32; 2]>) {
        self.process_with(audio, &[]);
    }

    pub fn process_takes(&mut self, audio: &mut Vec<[f32; 2]>) {
        self.run(audio, &[], true, None);
    }

    pub fn takes_latency(&self) -> usize {
        self.slots.iter().filter(|slot| slot.record && !slot.bypassed && slot.trouble.is_none()).map(|slot| slot.latency).sum()
    }

    pub fn has_takes(&self) -> bool {
        self.slots.iter().any(|slot| slot.record)
    }

    pub fn ready(&mut self) -> bool {
        self.take_arrivals();
        !self.still_opening()
    }

    pub fn process_with(&mut self, audio: &mut Vec<[f32; 2]>, side: &[[f32; 2]]) {
        self.run(audio, side, false, None);
    }

    pub fn process_at(&mut self, audio: &mut Vec<[f32; 2]>, at: i64) {
        self.run(audio, &[], false, Some(at));
    }

    pub fn region(&self) -> Option<&Region> {
        self.region.as_ref()
    }

    pub fn follow_region(&mut self, region: Option<Region>) -> Vec<String> {
        self.take_arrivals();
        self.region = region;
        let mut troubles = Vec::new();
        let Some(wanted) = self.region.clone() else { return troubles };
        for slot in self.slots.iter_mut() {
            if !slot.ara || slot.placed.as_ref() == Some(&wanted) {
                continue;
            }
            let Some(host) = slot.host.as_mut() else { continue };
            match host.ask(Ask::Region(wanted.clone())) {
                Ok(Reply::Fine) => slot.placed = Some(wanted.clone()),
                Ok(Reply::Trouble(why)) => troubles.push(format!("{}: {why}", slot.name)),
                Ok(other) => troubles.push(format!("{}: the plugin host answered out of turn: {other:?}", slot.name)),
                Err(why) => {
                    troubles.push(format!("{}: {why}", slot.name));
                    slot.trouble = Some(why);
                    slot.host = None;
                }
            }
        }
        troubles
    }

    fn run(&mut self, audio: &mut Vec<[f32; 2]>, side: &[[f32; 2]], takes: bool, at: Option<i64>) {
        self.take_arrivals();
        let block = audio.len();
        let rate = self.rate;
        for (which, slot) in self.slots.iter_mut().enumerate() {
            if slot.record != takes || slot.bypassed || slot.trouble.is_some() {
                continue;
            }
            let blending = slot.mix < 1.0;
            if blending {
                slot.dry.room_for(slot.latency, block);
                slot.dry.remember(audio);
            }
            if slot.dozes_through(audio, rate) {
                if blending {
                    slot.dry.slide(block);
                }
                continue;
            }
            if let Some(made) = slot.built.as_mut() {
                made.process(audio);
                if blending {
                    slot.dry.blend(audio, slot.mix);
                }
                slot.note_what_came_out(audio, rate);
                continue;
            }
            let Some(host) = slot.host.as_mut() else { continue };
            let ran = match at {
                Some(at) if slot.ara => host.run_at(audio, at),
                _ => host.run_with(audio, side),
            };
            match ran {
                Ok(()) => {
                    if blending {
                        slot.dry.blend(audio, slot.mix);
                    }
                    slot.note_what_came_out(audio, rate);
                }
                Err(why) => {
                    let fell = host.gone();
                    slot.trouble = Some(why.clone());
                    slot.host = None;
                    if fell {
                        self.fallen.push(Fallen { slot: which, name: slot.name.clone(), why });
                    }
                }
            }
        }
    }

    pub fn reconcile(&mut self, want: &[Wanted]) -> Vec<String> {
        let mut pool: Vec<Slot> = self.slots.drain(..).collect();
        let mut troubles = Vec::new();
        for Wanted { path, index, name, bypassed, state, record, mix } in want {
            let found = pool.iter().position(|slot| &slot.path == path && slot.index == *index && slot.working());
            match found {
                Some(at) => {
                    let mut slot = pool.remove(at);
                    slot.name = name.clone();
                    slot.bypassed = *bypassed;
                    slot.mix = *mix;
                    slot.record = *record;
                    slot.quiet_for = 0;
                    if let Some(made) = slot.built.as_mut() {
                        put_knobs(made.as_mut(), state);
                    } else if !state.is_empty() && *state != slot.known {
                        if let Some(waiting) = slot.queued.as_mut() {
                            waiting.state = state.clone();
                            slot.known = state.clone();
                        } else if let Some(host) = slot.host.as_mut() {
                            match settle(host, state) {
                                Ok(()) => {
                                    let _ = host.run(&mut vec![[0.0; 2]; self.block.max(1)]);
                                    slot.known = state.clone();
                                }
                                Err(why) => troubles.push(format!("{} would not go back to its earlier settings: {why}", slot.name)),
                            }
                        }
                    }
                    self.slots.push(slot);
                }
                None => {
                    let mut slot = Slot {
                        mix: *mix,
                        dry: Dry::empty(),
                        path: path.clone(),
                        index: *index,
                        name: name.clone(),
                        bypassed: *bypassed,
                        record: *record,
                        trouble: None,
                        held_back: false,
                        latency: 0,
                        ara: false,
                        placed: None,
                        host: None,
                        built: None,
                        coming: None,
                        queued: None,
                        kept_state: Vec::new(),
                        known: state.clone(),
                        wanted_open: false,
                        always_awake: false,
                        quiet_for: 0,
                    };
                    if is_built_in(path) {
                        match self.make_built(*index) {
                            Ok(mut made) => {
                                put_knobs(made.as_mut(), state);
                                slot.latency = made.latency();
                                slot.always_awake = must_stay_awake(made.as_ref());
                                slot.built = Some(made);
                            }
                            Err(why) => {
                                troubles.push(format!("{name}: {why}"));
                                slot.trouble = Some(why);
                            }
                        }
                    } else {
                        slot.placed = self.region.clone();
                        match self.ceiling.take_a_seat() {
                            Some(seat) => slot.queued = Some(Beginning { state: state.clone(), seat }),
                            None => {
                                slot.held_back = true;
                                slot.kept_state = state.clone();
                                slot.trouble = Some(crate::ceiling::NO_ROOM.to_string());
                            }
                        }
                    }
                    self.slots.push(slot);
                }
            }
        }
        self.start_what_there_is_room_to_start();
        troubles
    }

    fn start_what_there_is_room_to_start(&mut self) {
        for at in 0..self.slots.len() {
            if self.slots[at].queued.is_none() {
                continue;
            }
            let Some(opening) = self.ceiling.may_start_opening() else { return };
            let Some(Beginning { state, seat }) = self.slots[at].queued.take() else { continue };
            let (done, waiting) = std::sync::mpsc::channel();
            let host = self.host.clone();
            let order = Order {
                path: self.slots[at].path.clone(),
                index: self.slots[at].index,
                rate: self.rate,
                block: self.block,
                region: self.slots[at].placed.clone(),
                state,
            };
            std::thread::spawn(move || {
                let _ = done.send(open_on_a_thread(&host, order, seat));
                drop(opening);
            });
            self.slots[at].coming = Some(waiting);
        }
    }

    fn take_arrivals(&mut self) {
        for (at, slot) in self.slots.iter_mut().enumerate() {
            let Some(waiting) = slot.coming.as_ref() else { continue };
            match waiting.try_recv() {
                Ok(Ok(opened)) => {
                    let placed = slot.placed.take();
                    slot.settle_in(opened, placed.as_ref());
                    slot.coming = None;
                }
                Ok(Err((why, fell))) => {
                    slot.trouble = Some(why.clone());
                    slot.coming = None;
                    if fell {
                        self.fallen.push(Fallen { slot: at, name: slot.name.clone(), why });
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    slot.trouble = Some("the plugin host stopped before it loaded".into());
                    slot.coming = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        self.start_what_there_is_room_to_start();
    }

    fn make_built(&self, index: usize) -> Result<Box<dyn loupe_stock::Effect>, String> {
        let name = loupe_stock::NAMES.get(index).ok_or("Loupe has no such built in plugin")?;
        let mut made = loupe_stock::make(name).ok_or("Loupe has no such built in plugin")?;
        made.prepare(self.rate as f32);
        Ok(made)
    }

    fn open(&self, path: &Path, index: usize) -> Result<Opened, String> {
        let seat = self.ceiling.squeeze_in();
        let order = Order {
            path: path.to_path_buf(),
            index,
            rate: self.rate,
            block: self.block,
            region: self.region.clone(),
            state: Vec::new(),
        };
        open_on_a_thread(&self.host, order, seat).map_err(|(why, _)| why)
    }

    pub fn load_held_back(&mut self, slot: usize) -> Result<(), String> {
        let seat = self.ceiling.squeeze_in();
        let found = self.slots.get_mut(slot).ok_or("there is no such slot")?;
        if !found.held_back {
            return Err("that plugin is not waiting for room".into());
        }
        let state = std::mem::take(&mut found.kept_state);
        found.held_back = false;
        found.trouble = None;
        found.queued = Some(Beginning { state, seat });
        self.take_arrivals();
        Ok(())
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
            Wanted { path: PathBuf::from("one.vst3"), index: 0, name: "One".into(), bypassed: false, state: Vec::new(), record: false, mix: 1.0 },
            Wanted { path: PathBuf::from("two.vst3"), index: 1, name: "Two".into(), bypassed: true, state: Vec::new(), record: false, mix: 1.0 },
        ];
        let troubles = rack.reconcile(&want);
        assert!(troubles.is_empty());
        assert!(rack.slots().iter().all(|slot| slot.on_its_way()));
        let names: Vec<_> = rack.slots().iter().map(|slot| slot.name.clone()).collect();
        assert_eq!(names, vec!["One", "Two"]);
        assert!(rack.slots()[1].bypassed);
        let mut audio = vec![[0.0; 2]; 64];
        let gave_up = std::time::Instant::now();
        while rack.slots().iter().any(|slot| slot.on_its_way()) {
            assert!(gave_up.elapsed() < std::time::Duration::from_secs(5), "the plugins never answered");
            rack.process(&mut audio);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(rack.slots().iter().all(|slot| slot.trouble.is_some()));
        assert!(!rack.has_fallen(), "a host that never started has not crashed");
        let shorter = vec![Wanted { path: PathBuf::from("two.vst3"), index: 1, name: "Two".into(), bypassed: false, state: Vec::new(), record: false, mix: 1.0 }];
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
mod dozing_tests {
    use super::*;

    const RATE: u32 = 48_000;
    const BLOCK: usize = 512;
    const REVERB: usize = 4;
    const COMPRESSOR: usize = 1;
    const SATURATION: usize = 6;
    const LIMITER: usize = 2;
    const DELAY: usize = 3;
    const GATE: usize = 9;
    const TRANSIENT: usize = 8;
    const CHORUS: usize = 7;
    const TUNE: usize = 12;
    const METER: usize = 10;

    fn rack_of(plugins: &[usize]) -> Rack {
        let mut rack = Rack::new(PathBuf::from("no-host-here"), RATE, BLOCK);
        for index in plugins {
            rack.add(Path::new(crate::BUILT_IN), *index, loupe_stock::NAMES[*index]).expect("it loads");
        }
        rack
    }

    fn kept_awake(plugins: &[usize]) -> Rack {
        let mut rack = rack_of(plugins);
        for slot in rack.slots.iter_mut() {
            slot.always_awake = true;
        }
        rack
    }

    fn let_it_doze(plugins: &[usize]) -> Rack {
        let mut rack = rack_of(plugins);
        for slot in rack.slots.iter_mut() {
            slot.always_awake = false;
        }
        rack
    }

    fn tone(frames: usize, from: usize) -> Vec<[f32; 2]> {
        (0..frames)
            .map(|at| {
                let turn = (at + from) as f32 * 440.0 * std::f32::consts::TAU / RATE as f32;
                let loud = turn.sin() * 0.5;
                [loud, loud]
            })
            .collect()
    }

    fn blocks_of(song: &[[f32; 2]]) -> Vec<Vec<[f32; 2]>> {
        song.chunks(BLOCK).map(|block| block.to_vec()).collect()
    }

    fn render(rack: &mut Rack, song: &[[f32; 2]]) -> Vec<[f32; 2]> {
        let mut out = Vec::with_capacity(song.len());
        for mut block in blocks_of(song) {
            rack.process(&mut block);
            out.extend_from_slice(&block);
        }
        out
    }

    fn burst_then_quiet(burst: usize, quiet: usize) -> Vec<[f32; 2]> {
        let mut song = tone(burst, 0);
        song.extend(std::iter::repeat_n([0.0f32; 2], quiet));
        song
    }

    fn song_with_gaps() -> Vec<[f32; 2]> {
        let gap = RATE as usize * (QUIET_SECONDS_BEFORE_DOZING as usize + 4);
        let sound = RATE as usize / 2;
        let mut song = Vec::new();
        for round in 0..3 {
            song.extend(tone(sound, round * sound));
            song.extend(std::iter::repeat_n([0.0f32; 2], gap));
        }
        song.extend(tone(sound, 0));
        song
    }

    fn worst_difference(one: &[[f32; 2]], two: &[[f32; 2]]) -> f32 {
        assert_eq!(one.len(), two.len());
        one.iter().zip(two).fold(0.0f32, |top, (here, there)| {
            top.max((here[0] - there[0]).abs()).max((here[1] - there[1]).abs())
        })
    }

    fn biggest_jump(audio: &[[f32; 2]]) -> f32 {
        audio.windows(2).fold(0.0f32, |top, pair| {
            top.max((pair[1][0] - pair[0][0]).abs()).max((pair[1][1] - pair[0][1]).abs())
        })
    }

    fn first_loud_frame(audio: &[[f32; 2]]) -> Option<usize> {
        audio.iter().position(|frame| frame[0].abs() > 0.01)
    }

    #[test]
    fn a_tail_rings_out_whether_the_chain_dozes_or_not() {
        let hold = RATE as usize * QUIET_SECONDS_BEFORE_DOZING as usize;
        let burst = RATE as usize / 2;
        let song = burst_then_quiet(burst, hold + RATE as usize * 8);
        let dozing = render(&mut let_it_doze(&[REVERB]), &song);
        let awake = render(&mut kept_awake(&[REVERB]), &song);
        let while_it_is_awake = burst + hold;
        assert_eq!(
            dozing[..while_it_is_awake],
            awake[..while_it_is_awake],
            "the tail must be the same samples for as long as the chain is awake"
        );
        let apart = worst_difference(&dozing, &awake);
        assert!(apart <= QUIET, "what is left of the tail after dozing differs by {apart}");
        let loudest_tail = awake[burst..while_it_is_awake].iter().fold(0.0f32, |top, frame| top.max(frame[0].abs()));
        assert!(loudest_tail > 0.01, "the reverb had no tail to compare, only {loudest_tail}");
    }

    #[test]
    fn a_chain_that_dozes_and_wakes_is_sample_for_sample_the_same() {
        let chain = [GATE, LIMITER, TRANSIENT];
        let song = song_with_gaps();
        let mut sleepy = rack_of(&chain);
        let dozing = render(&mut sleepy, &song);
        let awake = render(&mut kept_awake(&chain), &song);
        assert_eq!(dozing, awake, "a chain that dozed through the gaps changed the sound");
        assert!(first_loud_frame(&dozing).is_some(), "nothing came out at all");
    }

    #[test]
    fn no_stock_plugin_clicks_when_it_wakes() {
        let song = song_with_gaps();
        for index in 0..loupe_stock::NAMES.len() {
            let dozing = render(&mut let_it_doze(&[index]), &song);
            let awake = render(&mut kept_awake(&[index]), &song);
            let jumped = biggest_jump(&dozing);
            let allowed = biggest_jump(&awake) * 1.1 + QUIET;
            assert!(jumped <= allowed, "{} jumped {jumped} on waking, far more than the {allowed} it jumps anyway", loupe_stock::NAMES[index]);
        }
    }

    #[test]
    fn a_chain_dozes_through_a_long_gap_and_wakes_for_the_next_note() {
        let mut rack = rack_of(&[COMPRESSOR, SATURATION]);
        let quiet = RATE as usize * (QUIET_SECONDS_BEFORE_DOZING as usize + 1);
        let mut played = tone(BLOCK, 0);
        rack.process(&mut played);
        assert_eq!(rack.dozing(), 0, "a chain with sound in it must not doze");
        for _ in 0..quiet / BLOCK {
            let mut nothing = vec![[0.0f32; 2]; BLOCK];
            rack.process(&mut nothing);
        }
        assert_eq!(rack.dozing(), 2, "both plugins should be dozing by now");
        let mut again = tone(BLOCK, 0);
        rack.process(&mut again);
        assert_eq!(rack.dozing(), 0, "sound must wake the whole chain at once");
    }

    #[test]
    fn a_plugin_with_a_clock_of_its_own_is_left_awake() {
        for index in [CHORUS, TUNE] {
            let clocked = rack_of(&[index]);
            assert!(clocked.slots()[0].always_awake, "{} would come back out of step", loupe_stock::NAMES[index]);
        }
        for index in [COMPRESSOR, LIMITER, SATURATION, GATE, TRANSIENT, DELAY, REVERB] {
            let plain = rack_of(&[index]);
            assert!(!plain.slots()[0].always_awake, "{} has no reason to stay awake", loupe_stock::NAMES[index]);
        }
    }

    #[test]
    fn a_plugin_with_latency_still_lines_up_after_dozing() {
        let song = song_with_gaps();
        let mut sleepy = rack_of(&[LIMITER]);
        let latency = sleepy.latency();
        assert!(latency > 0, "the limiter reported no latency");
        let dozing = render(&mut sleepy, &song);
        let awake = render(&mut kept_awake(&[LIMITER]), &song);
        assert_eq!(dozing, awake, "dozing moved the sound of a plugin with latency");
        let came_in = first_loud_frame(&dozing).expect("something came out");
        assert_eq!(came_in, first_loud_frame(&awake).expect("something came out"));
        assert!(came_in >= latency, "the limiter's own delay went missing: {came_in} < {latency}");
    }

    #[test]
    fn a_plugin_that_shows_the_signal_is_left_awake() {
        let watching = rack_of(&[METER]);
        assert!(watching.slots()[0].always_awake, "the meter must keep reading");
        let working = rack_of(&[COMPRESSOR]);
        assert!(!working.slots()[0].always_awake);
    }

    #[test]
    fn a_recording_plugin_never_dozes() {
        let want = vec![Wanted {
            mix: 1.0,
            path: PathBuf::from(crate::BUILT_IN),
            index: COMPRESSOR,
            name: "Loupe Compressor".into(),
            bypassed: false,
            state: Vec::new(),
            record: true,
        }];
        let mut rack = Rack::new(PathBuf::from("no-host-here"), RATE, BLOCK);
        assert!(rack.reconcile(&want).is_empty());
        for _ in 0..RATE as usize * 4 / BLOCK {
            let mut nothing = vec![[0.0f32; 2]; BLOCK];
            rack.process_takes(&mut nothing);
        }
        assert_eq!(rack.dozing(), 0, "a recording chain must stay ready for the next take");
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
    fn recording_plugins_only_touch_what_is_being_recorded() {
        let squash = |record| Wanted {
            mix: 1.0,
            path: PathBuf::from(crate::BUILT_IN),
            index: 1,
            name: "Loupe Compressor".into(),
            bypassed: false,
            state: Vec::new(),
            record,
        };
        let loudest = |audio: &[[f32; 2]]| audio.iter().fold(0.0f32, |top, frame| top.max(frame[0].abs()));
        let mut rack = Rack::new(PathBuf::from("no-host-here"), 48_000, 512);
        assert!(rack.reconcile(&[squash(true)]).is_empty());
        assert!(rack.has_takes());
        let mut played = vec![[0.9, 0.9]; 512];
        rack.process(&mut played);
        assert_eq!(loudest(&played), 0.9, "the mix never hears a recording plugin");
        let mut taken = vec![[0.9, 0.9]; 512];
        rack.process_takes(&mut taken);
        assert!(loudest(&taken) < 0.9);
        assert!(rack.reconcile(&[squash(false)]).is_empty());
        assert!(!rack.has_takes());
        let mut taken = vec![[0.9, 0.9]; 512];
        rack.process_takes(&mut taken);
        assert_eq!(loudest(&taken), 0.9, "a mix plugin never hears the take");
    }

    #[test]
    fn a_built_in_remembers_its_knobs() {
        let mut rack = Rack::new(PathBuf::from("no-host-here"), 48_000, 512);
        rack.add(Path::new(crate::BUILT_IN), 0, "Loupe EQ").expect("it loads");
        let first = rack.save(0).expect("it saves");
        assert!(!first.is_empty());
        let want = vec![Wanted {
            mix: 1.0,
            path: PathBuf::from(crate::BUILT_IN),
            index: 0,
            name: "Loupe EQ".into(),
            bypassed: false,
            state: first.clone(),
            record: false,
        }];
        let mut fresh = Rack::new(PathBuf::from("no-host-here"), 48_000, 512);
        assert!(fresh.reconcile(&want).is_empty());
        assert_eq!(fresh.save(0), Some(first));
    }
}

struct Order {
    path: PathBuf,
    index: usize,
    rate: u32,
    block: usize,
    region: Option<Region>,
    state: Vec<u8>,
}

fn open_on_a_thread(host: &Path, order: Order, seat: Seat) -> Arrived {
    let Order { path, index, rate, block, region, state } = order;
    let mut sandbox = Sandbox::start_in_a_seat(host, seat).map_err(|why| (why, false))?;
    if let Some(region) = region {
        let placed = sandbox.ask(Ask::Region(region)).and_then(|reply| match reply {
            Reply::Fine => Ok(()),
            Reply::Trouble(why) => Err(why),
            other => Err(format!("the plugin host answered out of turn: {other:?}")),
        });
        placed.map_err(|why| (why, sandbox.gone()))?;
    }
    let ask = Ask::Load { path: path.to_string_lossy().to_string(), index, rate, block };
    let loaded = sandbox.ask(ask).and_then(|reply| match reply {
        Reply::Loaded { latency, ara, .. } => Ok((latency, ara)),
        Reply::Trouble(why) => Err(why),
        other => Err(format!("the plugin host answered out of turn: {other:?}")),
    });
    let (latency, ara) = loaded.map_err(|why| (why, sandbox.gone()))?;
    settle(&mut sandbox, &state).map_err(|why| (why, sandbox.gone()))?;
    Ok(Opened { host: sandbox, latency, ara })
}

#[cfg(all(test, unix))]
mod fallen_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    struct Host(PathBuf);

    impl Drop for Host {
        fn drop(&mut self) {
            if let Some(folder) = self.0.parent() {
                let _ = std::fs::remove_dir_all(folder);
            }
        }
    }

    fn host(name: &str, script: &str) -> Host {
        let folder = std::env::temp_dir().join(format!("loupe-fallen-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("loupe-host");
        std::fs::write(&file, format!("#!/bin/sh\n{script}")).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        Host(file)
    }

    fn wanted(name: &str) -> Wanted {
        Wanted { path: PathBuf::from("crashy.vst3"), index: 0, name: name.into(), bypassed: false, state: Vec::new(), record: false, mix: 1.0 }
    }

    fn play_until_settled(rack: &mut Rack) {
        let mut audio = vec![[0.25; 2]; 64];
        let gave_up = std::time::Instant::now();
        while rack.slots().iter().any(|slot| slot.on_its_way()) {
            assert!(gave_up.elapsed() < std::time::Duration::from_secs(10), "the plugin never answered");
            rack.process(&mut audio);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        rack.process(&mut audio);
    }

    #[test]
    fn a_host_that_dies_while_playing_is_named_once() {
        let script = host("playing", "read ask\nprintf 'loaded\\t2\\t2\\t0\\n'\nread ask\nexit 3\n");
        let mut rack = Rack::new(script.0.clone(), 48_000, 64);
        assert!(rack.reconcile(&[wanted("Crashy Synth")]).is_empty());
        play_until_settled(&mut rack);
        let mut audio = vec![[0.5; 2]; 64];
        rack.process(&mut audio);
        assert_eq!(audio, vec![[0.5; 2]; 64], "the song carries on without the plugin");
        assert!(rack.slots()[0].trouble.is_some());
        let fallen = rack.take_fallen();
        assert_eq!(fallen.len(), 1);
        assert_eq!(fallen[0].slot, 0);
        assert_eq!(fallen[0].name, "Crashy Synth");
        assert!(!rack.has_fallen());
    }

    #[test]
    fn a_host_that_dies_while_loading_is_named() {
        let script = host("loading", "read ask\nexit 3\n");
        let mut rack = Rack::new(script.0.clone(), 48_000, 64);
        rack.reconcile(&[wanted("Crashy Reverb")]);
        play_until_settled(&mut rack);
        let fallen = rack.take_fallen();
        assert_eq!(fallen.iter().map(|fell| fell.name.as_str()).collect::<Vec<_>>(), vec!["Crashy Reverb"]);
    }

    #[test]
    fn a_window_asked_for_while_the_plugin_loads_opens_without_the_song_playing() {
        let script = host("stopped", "read ask\nprintf 'loaded\\t2\\t2\\t0\\n'\nwhile read ask; do printf 'trouble\\tno window on this host\\n'; done\n");
        let mut rack = Rack::new(script.0.clone(), 48_000, 64);
        rack.reconcile(&[wanted("Tuner")]);
        assert!(rack.show(0).is_ok(), "the window is wanted as soon as the plugin is there");
        let gave_up = std::time::Instant::now();
        let mut troubles = Vec::new();
        while troubles.is_empty() {
            assert!(gave_up.elapsed() < std::time::Duration::from_secs(10), "the window was never asked for");
            troubles = rack.open_waiting();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(troubles, vec!["no window on this host".to_string()]);
    }

    #[test]
    fn a_plugin_that_refuses_to_load_has_not_crashed() {
        let script = host("refusing", "read ask\nprintf 'trouble\\tno such plugin\\n'\nread ask\n");
        let mut rack = Rack::new(script.0.clone(), 48_000, 64);
        rack.reconcile(&[wanted("Missing")]);
        play_until_settled(&mut rack);
        assert!(rack.slots()[0].trouble.is_some());
        assert!(!rack.has_fallen());
    }

    fn settle_within(rack: &mut Rack, how_long: std::time::Duration) {
        let gave_up = std::time::Instant::now();
        while rack.slots().iter().any(|slot| slot.on_its_way()) {
            assert!(gave_up.elapsed() < how_long, "the plugins never settled");
            rack.ready();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn only_as_many_plugins_as_there_is_room_for_are_loaded() {
        let script = host("ceiling", "read ask\nprintf 'loaded\\t2\\t2\\t0\\n'\nwhile : ; do sleep 1 ; done\n");
        let mut rack = Rack::with_room_for(script.0.clone(), 48_000, 64, 2);
        let want: Vec<Wanted> = (0..5).map(|which| wanted(&format!("Plugin {which}"))).collect();
        assert!(rack.reconcile(&want).is_empty());
        settle_within(&mut rack, std::time::Duration::from_secs(20));
        let loaded = rack.slots().iter().filter(|slot| slot.working()).count();
        assert_eq!(loaded, 2, "the ceiling holds at two plugins");
        assert_eq!(rack.held_back(), 3, "the other three say they are not loaded");
        assert!(rack.slots().iter().filter(|slot| slot.held_back).all(|slot| slot.trouble.as_deref() == Some(crate::ceiling::NO_ROOM)));
        let waiting = rack.slots().iter().position(|slot| slot.held_back).expect("one is waiting");
        assert!(rack.load_held_back(waiting).is_ok());
        settle_within(&mut rack, std::time::Duration::from_secs(20));
        assert_eq!(rack.slots().iter().filter(|slot| slot.working()).count(), 3, "the one the user asked for came on");
        assert_eq!(rack.held_back(), 2);
        assert!(rack.load_held_back(waiting).is_err(), "it is not waiting any more");
    }

    #[test]
    fn a_big_project_does_not_start_every_host_at_once() {
        let script = host("stagger", "read ask\nsleep 1\nprintf 'loaded\\t2\\t2\\t0\\n'\nwhile : ; do sleep 1 ; done\n");
        let mut rack = Rack::with_room_for(script.0.clone(), 48_000, 64, 16);
        let want: Vec<Wanted> = (0..16).map(|which| wanted(&format!("Plugin {which}"))).collect();
        assert!(rack.reconcile(&want).is_empty());
        let started = rack.slots().iter().filter(|slot| slot.coming.is_some()).count();
        assert!(started <= 8, "{started} hosts went at once");
        assert_eq!(rack.slots().iter().filter(|slot| slot.on_its_way()).count(), 16, "the rest are waiting their turn");
        settle_within(&mut rack, std::time::Duration::from_secs(60));
        assert_eq!(rack.slots().iter().filter(|slot| slot.working()).count(), 16, "all of them get there in the end");
    }

    #[test]
    fn a_plugin_that_is_late_with_one_block_keeps_playing() {
        let script = host(
            "slow",
            "read ask\nprintf 'loaded\\t2\\t2\\t0\\n'\nwhile read ask; do\n  head -c 512 > /dev/null\n  sleep 3\n  head -c 512 > /dev/null\n  printf 'block\\n'\ndone\n",
        );
        let mut rack = Rack::new(script.0.clone(), 48_000, 64);
        assert!(rack.reconcile(&[wanted("Dawdler")]).is_empty());
        play_until_settled(&mut rack);
        let mut audio = vec![[0.5; 2]; 64];
        rack.process(&mut audio);
        assert_eq!(audio, vec![[0.5; 2]; 64], "a late block passes the song through untouched");
        assert!(rack.slots()[0].trouble.is_none(), "being late is not being dead");
        assert!(!rack.has_fallen(), "a late plugin is not reported as a crash");
    }
}

#[cfg(test)]
mod blending {
    use super::*;

    fn line(latency: usize, block: usize) -> Dry {
        let mut dry = Dry::empty();
        dry.room_for(latency, block);
        dry
    }

    #[test]
    fn all_the_way_wet_leaves_the_plugin_alone() {
        let mut dry = line(0, 4);
        let mut audio = vec![[1.0, 1.0]; 4];
        dry.remember(&[[0.25, 0.25]; 4]);
        dry.blend(&mut audio, 1.0);
        assert_eq!(audio, vec![[1.0, 1.0]; 4]);
    }

    #[test]
    fn all_the_way_dry_gives_back_what_went_in() {
        let mut dry = line(0, 4);
        let mut audio = vec![[1.0, 1.0]; 4];
        dry.remember(&[[0.25, 0.25]; 4]);
        dry.blend(&mut audio, 0.0);
        assert_eq!(audio, vec![[0.25, 0.25]; 4]);
    }

    #[test]
    fn half_and_half_is_the_middle_of_the_two() {
        let mut dry = line(0, 4);
        let mut audio = vec![[1.0, -1.0]; 4];
        dry.remember(&[[0.0, 1.0]; 4]);
        dry.blend(&mut audio, 0.5);
        assert_eq!(audio, vec![[0.5, 0.0]; 4]);
    }

    #[test]
    fn a_plugin_with_latency_is_met_by_a_dry_that_waited_the_same() {
        let block = 4;
        let latency = 4;
        let mut dry = line(latency, block);
        let first = vec![[1.0, 1.0]; block];
        let second = vec![[2.0, 2.0]; block];
        let mut wet = vec![[0.0, 0.0]; block];
        dry.remember(&first);
        dry.blend(&mut wet, 0.0);
        assert_eq!(wet, vec![[0.0, 0.0]; block], "nothing has come through the plugin yet");
        let mut wet = vec![[0.0, 0.0]; block];
        dry.remember(&second);
        dry.blend(&mut wet, 0.0);
        assert_eq!(wet, first, "the dry arrives as late as the plugin does");
    }
}

#[cfg(test)]
mod real_plugin_tests {
    use super::*;

    fn real(state: Vec<u8>) -> Option<(Rack, Wanted)> {
        let plugin = PathBuf::from(std::env::var_os("LOUPE_TEST_VST3")?);
        let host = crate::sandbox::host_beside_us().parent()?.parent()?.join(format!("loupe-host{}", std::env::consts::EXE_SUFFIX));
        let wanted = Wanted { path: plugin, index: 0, name: "Test".into(), bypassed: false, state, record: false, mix: 1.0 };
        let mut rack = Rack::new(host, 48_000, 64);
        assert!(rack.reconcile(&[wanted.clone()]).is_empty());
        let mut audio = vec![[0.0; 2]; 64];
        let gave_up = std::time::Instant::now();
        while rack.slots()[0].on_its_way() {
            assert!(gave_up.elapsed() < std::time::Duration::from_secs(20), "the plugin never loaded");
            rack.process(&mut audio);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(rack.slots()[0].trouble.is_none(), "{:?}", rack.slots()[0].trouble);
        Some((rack, wanted))
    }

    #[test]
    fn a_turned_knob_reads_back_saves_and_undoes() {
        let Some((mut rack, mut wanted)) = real(Vec::new()) else { return };
        let first = rack.readings(0);
        assert!(!first.is_empty(), "the plugin shows no knobs");
        let named = std::env::var("LOUPE_TEST_KNOB").ok();
        let knob = named.as_deref().and_then(|name| first.iter().position(|reading| reading.name == name)).unwrap_or(first.len() - 1);
        let asked = std::env::var("LOUPE_TEST_TEXT").unwrap_or_else(|_| "0.5".into());
        let target = rack.from_text(0, knob, &asked).unwrap_or(0.5);
        let before = rack.save(0).unwrap();
        let after = rack.turn_and_save(0, knob, target).unwrap();
        assert_ne!(before, after, "the saved settings carry the turn");
        assert!((rack.readings(0)[knob].value - target).abs() < 0.01);
        if named.is_some() {
            let shown = rack.readings(0)[knob].text.clone();
            assert_eq!(crate::wording::number_in(&shown), crate::wording::number_in(&asked), "{asked} shows as {shown}");
        }
        wanted.state = before;
        assert!(rack.reconcile(&[wanted]).is_empty());
        assert!((rack.readings(0)[knob].value - first[knob].value).abs() < 0.01, "undo puts the knob back");
    }
}
