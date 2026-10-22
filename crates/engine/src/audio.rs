use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::clock;
use crate::input::MOST_INPUTS;
use crate::devices::{self, Device, Failure, Running};
use crate::instrument::Note;
use crate::midi_in::{KeyEvent, KeySender};
use crate::model::{ClipId, Frames, MeterPoint, Project, TrackId, METER_POINTS};
use crate::render::{loudest, mix_tracks_metered, TrackPeaks, pan_gains, scale, shape_of, Chains, Mixdown, Round, Shape};

const MAX_BLOCK: usize = 4096;
const FADE_SECONDS: f32 = 0.005;
const QUEUE: usize = 256;
const SILENT_RATE: u32 = 48_000;
pub const METERS: usize = 64;
const TAPE: usize = 4096;
const LOAD_KEEP: f32 = 0.95;
const PEAK_KEEP: f32 = 0.995;
const ROLL_BLOCK: usize = 512;
const STILL_FOR: f32 = 0.1;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Load {
    pub average: f32,
    pub peak: f32,
    pub overloads: u64,
}
const MOST_HEARD: usize = 8;
const EAR_SPARE_SECONDS: f64 = 0.002;
const EAR_MOST_SPARE_SECONDS: f64 = 0.01;
const EAR_LOOKS_OVER_SECONDS: f64 = 0.25;
const EAR_FAR_BEHIND_SECONDS: f64 = 0.02;
const EAR_NEVER_BEHIND_SECONDS: f64 = 0.05;
const EAR_MOST_NUDGE: f64 = 0.005;
const EAR_DRY_AGAIN_SECONDS: f64 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TapedKey {
    pub track: TrackId,
    pub key: u8,
    pub velocity: f32,
    pub at: Frames,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    Device(Device),
    Silent,
    SilentAt(u32),
}

enum Msg {
    Project(Arc<Project>),
    Chains(Option<Box<dyn Chains>>),
    Tweak { track: TrackId, slot: usize, knob: usize, value: f32 },
    TweakClip { clip: ClipId, slot: usize, knob: usize, value: f32 },
    Play,
    Stop,
    Seek(Frames),
    Loop(Option<(Frames, Frames)>),
    Audition(Option<ClipId>),
    Endless(bool),
    NoteOn { track: TrackId, key: u8, velocity: f32 },
    NoteOff { track: TrackId, key: u8 },
    Silence,
    Tape(bool),
    Recording(bool),
    KeysGoTo(Option<TrackId>),
    Metronome(bool),
    CountIn(Frames),
    Hear(Option<(Consumer<f32>, u32, usize)>),
    HearOn([Option<TrackId>; MOST_HEARD]),
    HearThroughChain(bool),
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            levels: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU32::new(0))),
            master_level: AtomicU32::new(0),
            pos: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            heard_turn: AtomicU64::new(0),
            heard_pos: AtomicU64::new(0),
            heard_at: AtomicU64::new(0),
            load: AtomicU32::new(0),
            load_peak: AtomicU32::new(0),
            overloads: AtomicU64::new(0),
            monitor_lag: AtomicU64::new(0),
        }
    }
}

struct Shared {
    levels: [[AtomicU32; METER_POINTS]; METERS],
    master_level: AtomicU32,
    pos: AtomicU64,
    playing: AtomicBool,
    heard_turn: AtomicU64,
    heard_pos: AtomicU64,
    heard_at: AtomicU64,
    load: AtomicU32,
    load_peak: AtomicU32,
    overloads: AtomicU64,
    monitor_lag: AtomicU64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Roll {
    No,
    Due,
    Out,
}

struct Rolled {
    racks: Option<Box<dyn Chains>>,
    scratch: Mixdown,
    project: Arc<Project>,
    at: Frames,
    round: Round,
    only: Option<ClipId>,
    step: usize,
}

fn roll_ahead(mut asked: Consumer<Rolled>, mut done: Producer<Rolled>, wanted: Arc<AtomicBool>) {
    loop {
        match asked.pop() {
            Ok(mut parcel) => {
                let mut racks = parcel.racks.take();
                if let Some(racks) = racks.as_deref_mut() {
                    racks.takes_its_time(true);
                }
                parcel.scratch.get_ready(&parcel.project, parcel.at, parcel.round, parcel.only, parcel.step, racks.as_deref_mut(), Some(&wanted));
                if let Some(racks) = racks.as_deref_mut() {
                    racks.takes_its_time(false);
                }
                parcel.racks = racks;
                if done.push(parcel).is_err() {
                    return;
                }
            }
            Err(_) if asked.is_abandoned() => return,
            Err(_) => thread::sleep(Duration::from_millis(1)),
        }
    }
}

impl Shared {
    fn take_levels(&self) -> ([TrackPeaks; METERS], f32) {
        let take = |slot: &AtomicU32| f32::from_bits(slot.swap(0f32.to_bits(), Ordering::Relaxed));
        (std::array::from_fn(|i| std::array::from_fn(|point| take(&self.levels[i][point]))), take(&self.master_level))
    }

    fn note_track(&self, index: usize, point: MeterPoint, level: f32) {
        if let Some(points) = self.levels.get(index) {
            points[point.index()].fetch_max(level.to_bits(), Ordering::Relaxed);
        }
    }
}

struct Rt {
    scratch: Mixdown,
    load: f32,
    load_peak: f32,
    chains: Option<Box<dyn Chains>>,
    peaks: [TrackPeaks; METERS],
    project: Arc<Project>,
    inbox: Consumer<Msg>,
    retired: Producer<Arc<Project>>,
    hand_back: Producer<Box<dyn Chains>>,
    shared: Arc<Shared>,
    pos: Frames,
    playing: bool,
    seek: Option<Frames>,
    loop_range: Option<(Frames, Frames)>,
    audition: Option<ClipId>,
    endless: bool,
    master: f32,
    fade: u32,
    fade_len: u32,
    block: Vec<[f32; 2]>,
    live: Vec<Live>,
    keys: Consumer<KeyEvent>,
    keys_go_to: Option<TrackId>,
    taping: bool,
    recording: bool,
    tape: Producer<TapedKey>,
    metronome: bool,
    count_in: Frames,
    count_in_len: Frames,
    ear: Ear,
    hear_through_chain: bool,
    done_feeds: Producer<Consumer<f32>>,
    roll: Roll,
    asking: Producer<Rolled>,
    rolled: Consumer<Rolled>,
    racks_wanted: bool,
    asked_back: Arc<AtomicBool>,
    looked_at: Option<Frames>,
    still: Frames,
    step: usize,
}

const NOT_HEARD: usize = usize::MAX;

struct Ear {
    feed: Option<Consumer<f32>>,
    rate: u32,
    width: usize,
    on: [Option<TrackId>; MOST_HEARD],
    filling: bool,
    phase: f64,
    spare: usize,
    lowest: usize,
    looked: f64,
    nudge: f64,
    locked: bool,
    since_dry: f64,
    was: [f32; MOST_INPUTS],
    next: [f32; MOST_INPUTS],
    voice: Vec<f32>,
    track: Vec<[f32; 2]>,
    out: Vec<[f32; 2]>,
    mix: Vec<Vec<[f32; 2]>>,
    sides: Vec<Vec<[f32; 2]>>,
    pre: Vec<[f32; 2]>,
    depth: Vec<usize>,
    deepest: usize,
    shape: Vec<Shape>,
    armed: [Option<TrackId>; MOST_HEARD],
}

impl Default for Ear {
    fn default() -> Self {
        Self {
            feed: None,
            rate: 0,
            width: 1,
            on: [None; MOST_HEARD],
            filling: false,
            phase: 0.0,
            spare: 0,
            lowest: usize::MAX,
            looked: 0.0,
            nudge: 0.0,
            locked: false,
            since_dry: f64::INFINITY,
            was: [0.0; MOST_INPUTS],
            next: [0.0; MOST_INPUTS],
            voice: Vec::new(),
            track: Vec::new(),
            out: Vec::new(),
            mix: Vec::new(),
            sides: Vec::new(),
            pre: Vec::new(),
            depth: Vec::new(),
            deepest: 0,
            shape: Vec::new(),
            armed: [None; MOST_HEARD],
        }
    }
}

impl Ear {
    fn frames_in(&self, seconds: f64) -> usize {
        (self.rate as f64 * seconds).round() as usize
    }

    fn waiting(&self) -> usize {
        self.feed.as_ref().map_or(0, |feed| feed.slots() / self.width)
    }

    fn let_go(&mut self, frames: usize) {
        let width = self.width;
        let Some(feed) = self.feed.as_mut() else { return };
        if let Ok(old) = feed.read_chunk(frames.min(feed.slots() / width) * width) {
            old.commit_all();
        }
    }

    fn start_looking(&mut self) {
        self.lowest = usize::MAX;
        self.looked = 0.0;
    }

    fn ready_for(&mut self, frames: usize, step: f64) -> bool {
        let wanted = (frames as f64 * step).ceil() as usize;
        let waiting = self.waiting();
        let keep = wanted + self.spare;
        if self.filling {
            if waiting < wanted {
                return false;
            }
            self.filling = false;
            self.start_looking();
        }
        if waiting > keep + self.frames_in(EAR_NEVER_BEHIND_SECONDS) {
            self.let_go(waiting - keep);
            self.start_looking();
        }
        true
    }

    fn ran_dry(&mut self) {
        self.filling = true;
        if self.since_dry < self.rate as f64 * EAR_DRY_AGAIN_SECONDS {
            self.spare = (self.spare + self.frames_in(EAR_SPARE_SECONDS / 2.0)).min(self.frames_in(EAR_MOST_SPARE_SECONDS));
        }
        self.since_dry = 0.0;
    }

    fn keep_close(&mut self, read: f64) {
        self.since_dry += read;
        if self.locked {
            self.nudge = 0.0;
            self.start_looking();
            return;
        }
        self.lowest = self.lowest.min(self.waiting());
        self.looked += read;
        if self.looked < self.rate as f64 * EAR_LOOKS_OVER_SECONDS {
            return;
        }
        let over = self.lowest as f64 - self.spare as f64;
        if over > self.frames_in(EAR_FAR_BEHIND_SECONDS) as f64 {
            self.let_go(over as usize);
        } else {
            self.nudge = (over / self.looked).clamp(-EAR_MOST_NUDGE, EAR_MOST_NUDGE);
        }
        self.start_looking();
    }
}

fn room_to_hear(ear: &mut Ear, tracks: usize, frames: usize) {
    if ear.mix.len() < tracks {
        ear.mix.resize_with(tracks, Vec::new);
        ear.sides.resize_with(tracks, Vec::new);
    }
    for buffer in ear.mix.iter_mut().take(tracks).chain(ear.sides.iter_mut().take(tracks)) {
        if buffer.len() < frames {
            buffer.resize(frames, [0.0; 2]);
        }
    }
    if ear.pre.len() < frames {
        ear.pre.resize(frames, [0.0; 2]);
    }
}

fn work_out_the_path(ear: &mut Ear, project: &Project) {
    let count = project.tracks.len();
    let same = ear.depth.len() == count
        && ear.armed == ear.on
        && ear.shape.len() == count
        && project.tracks.iter().zip(&ear.shape).all(|(track, known)| shape_of(track) == *known);
    if same {
        return;
    }
    ear.armed = ear.on;
    ear.shape.clear();
    ear.shape.extend(project.tracks.iter().map(shape_of));
    ear.depth.clear();
    ear.depth.resize(count, NOT_HEARD);
    for id in ear.on.iter().flatten() {
        if let Some(at) = project.tracks.iter().position(|track| track.id == *id) {
            ear.depth[at] = 0;
        }
    }
    for _ in 0..count {
        let mut moved = false;
        for at in 0..count {
            if ear.depth[at] == NOT_HEARD {
                continue;
            }
            let next = ear.depth[at] + 1;
            let track = &project.tracks[at];
            for fed in track.parent.into_iter().chain(track.sends.iter().map(|send| send.to)) {
                let Some(to) = project.tracks.iter().position(|t| t.id == fed) else { continue };
                if ear.depth[to] == NOT_HEARD || ear.depth[to] < next {
                    ear.depth[to] = next;
                    moved = true;
                }
            }
        }
        if !moved {
            break;
        }
    }
    ear.deepest = ear.depth.iter().filter(|deep| **deep != NOT_HEARD).copied().max().unwrap_or(0);
}

const MOST_LIVE_NOTES: usize = 32;
const HELD: Frames = Frames::MAX / 4;

#[derive(Clone, Copy)]
struct Live {
    track: TrackId,
    note: Note,
    played: Frames,
}

struct Remote {
    keys: KeySender,
    outbox: Producer<Msg>,
    retired: Consumer<Arc<Project>>,
    handed_back: Consumer<Box<dyn Chains>>,
    tape: Consumer<TapedKey>,
    shared: Arc<Shared>,
    done_feeds: Consumer<Consumer<f32>>,
}

fn pair(rate: u32) -> (Rt, Remote) {
    let (keys_in, keys_out) = RingBuffer::new(QUEUE);
    let (outbox, inbox) = RingBuffer::new(QUEUE);
    let (retired_tx, retired_rx) = RingBuffer::new(QUEUE);
    let (back_tx, back_rx) = RingBuffer::new(QUEUE);
    let (tape_in, tape_out) = RingBuffer::new(TAPE);
    let (done_in, done_out) = RingBuffer::new(4);
    let (asking, asked) = RingBuffer::new(1);
    let (rolled_in, rolled_out) = RingBuffer::new(1);
    let asked_back = Arc::new(AtomicBool::new(false));
    let theirs = asked_back.clone();
    thread::Builder::new().name("loupe-preroll".into()).spawn(move || roll_ahead(asked, rolled_in, theirs)).expect("start the pre-roll thread");
    let shared = Arc::new(Shared::default());
    let mut scratch = Mixdown::default();
    scratch.hurries();
    let rt = Rt {
        roll: Roll::No,
        asking,
        rolled: rolled_out,
        racks_wanted: false,
        asked_back,
        looked_at: None,
        still: 0,
        step: ROLL_BLOCK,
        live: Vec::with_capacity(MOST_LIVE_NOTES),
        keys: keys_out,
        keys_go_to: None,
        taping: false,
        recording: false,
        tape: tape_in,
        metronome: false,
        count_in: 0,
        count_in_len: 0,
        load: 0.0,
        load_peak: 0.0,
        ear: Ear { voice: vec![0.0; MAX_BLOCK * MOST_INPUTS], track: vec![[0.0; 2]; MAX_BLOCK], ..Ear::default() },
        hear_through_chain: true,
        done_feeds: done_in,
        scratch,
        chains: None,
        peaks: [[0.0; METER_POINTS]; METERS],
        project: Arc::new(Project::new(rate)),
        inbox,
        retired: retired_tx,
        hand_back: back_tx,
        shared: shared.clone(),
        pos: 0,
        playing: false,
        seek: None,
        loop_range: None,
        audition: None,
        endless: false,
        master: 1.0,
        fade: 0,
        fade_len: ((FADE_SECONDS * rate as f32).round() as u32).max(1),
        block: vec![[0.0; 2]; MAX_BLOCK],
    };
    (rt, Remote { keys: Arc::new(std::sync::Mutex::new(keys_in)), outbox, retired: retired_rx, handed_back: back_rx, tape: tape_out, shared, done_feeds: done_out })
}

impl Rt {
    fn process(&mut self, frames: usize) -> &[[f32; 2]] {
        while let Ok(msg) = self.inbox.pop() {
            match msg {
                Msg::Project(project) => {
                    let old = std::mem::replace(&mut self.project, project);
                    let _ = self.retired.push(old);
                }
                Msg::Chains(racks) => {
                    if self.roll == Roll::Out && racks.is_none() {
                        self.racks_wanted = true;
                        self.asked_back.store(true, Ordering::Relaxed);
                    }
                    if let Some(old) = std::mem::replace(&mut self.chains, racks) {
                        let _ = self.hand_back.push(old);
                    }
                }
                Msg::Tweak { track, slot, knob, value } => {
                    if let Some(racks) = self.chains.as_deref_mut() {
                        racks.tweak(track, slot, knob, value);
                    }
                }
                Msg::TweakClip { clip, slot, knob, value } => {
                    if let Some(racks) = self.chains.as_deref_mut() {
                        racks.tweak_clip(clip, slot, knob, value);
                    }
                }
                Msg::Play => self.playing = true,
                Msg::Stop => {
                    self.playing = false;
                    self.count_in = 0;
                }
                Msg::Metronome(on) => self.metronome = on,
                Msg::CountIn(len) => {
                    self.count_in = len;
                    self.count_in_len = len;
                }
                Msg::Hear(feed) => {
                    let (feed, rate, width) = match feed {
                        Some((feed, rate, width)) => (Some(feed), rate, width.clamp(1, MOST_INPUTS)),
                        None => (None, 0, 1),
                    };
                    if let Some(old) = std::mem::replace(&mut self.ear.feed, feed) {
                        let _ = self.done_feeds.push(old);
                    }
                    self.ear.rate = rate;
                    self.ear.width = width;
                    self.ear.filling = true;
                    self.ear.spare = self.ear.frames_in(EAR_SPARE_SECONDS);
                    self.ear.nudge = 0.0;
                    self.ear.locked = devices::one_device_both_ways();
                    self.ear.since_dry = f64::INFINITY;
                }
                Msg::HearOn(on) => self.ear.on = on,
                Msg::HearThroughChain(on) => self.hear_through_chain = on,
                Msg::Seek(to) => self.seek = Some(to),
                Msg::Loop(range) => self.loop_range = range,
                Msg::Audition(clip) => self.audition = clip,
                Msg::Endless(endless) => self.endless = endless,
                Msg::NoteOn { track, key, velocity } => {
                    self.taped(track, key, velocity);
                    if self.live.len() == MOST_LIVE_NOTES {
                        self.live.remove(0);
                    }
                    self.live.push(Live { track, note: Note { key, start: 0, len: HELD, velocity }, played: 0 });
                }
                Msg::NoteOff { track, key } => {
                    self.taped(track, key, 0.0);
                    for voice in self.live.iter_mut().filter(|v| v.track == track && v.note.key == key && v.note.len == HELD) {
                        voice.note.len = voice.played.max(1);
                    }
                }
                Msg::Silence => self.live.clear(),
                Msg::Tape(on) => self.taping = on,
                Msg::Recording(on) => self.recording = on,
                Msg::KeysGoTo(track) => {
                    if track != self.keys_go_to {
                        self.live.retain(|voice| Some(voice.track) != self.keys_go_to || voice.note.len != HELD);
                    }
                    self.keys_go_to = track;
                }
            }
        }

        while let Ok(event) = self.keys.pop() {
            let Some(track) = self.keys_go_to else { continue };
            self.taped(track, event.key, event.velocity);
            if event.velocity > 0.0 {
                if self.live.len() == MOST_LIVE_NOTES {
                    self.live.remove(0);
                }
                self.live.push(Live { track, note: Note { key: event.key, start: 0, len: HELD, velocity: event.velocity }, played: 0 });
            } else {
                for voice in self.live.iter_mut().filter(|v| v.track == track && v.note.key == event.key && v.note.len == HELD) {
                    voice.note.len = voice.played.max(1);
                }
            }
        }

        self.step = frames.clamp(1, MAX_BLOCK);
        self.collect_roll();
        let rate = self.project.rate;
        let beat = beat_frames(self.project.bpm, rate);
        let where_we_are = crate::render::Song::where_it_is(&self.project, self.pos, self.playing, self.recording);
        if let Some(racks) = self.chains.as_deref_mut() {
            racks.song_goes(where_we_are);
        }
        let mut block = std::mem::take(&mut self.block);
        let out = &mut block[..frames];
        let mut done = 0;
        while done < frames {
            if self.fade == 0 {
                if let Some(to) = self.seek.take() {
                    self.pos = to;
                }
                if !self.playing && self.roll != Roll::Out {
                    out[done..].fill([0.0; 2]);
                    break;
                }
            }
            if self.playing && self.count_in > 0 {
                let part = (self.count_in as usize).min(frames - done);
                let chunk = &mut out[done..done + part];
                chunk.fill([0.0; 2]);
                add_clicks(chunk, (self.count_in_len - self.count_in) as f64, beat, rate);
                self.count_in -= part as Frames;
                done += part;
                continue;
            }
            let round = self.round_at(self.pos);
            if self.roll == Roll::No && self.scratch.ready_for(&self.project, self.pos, round, self.chains.as_deref()) > frames {
                self.roll = Roll::Due;
            }
            if self.roll == Roll::Due && self.fade == 0 {
                self.send_a_roll(self.pos, round);
            }
            if self.roll == Roll::Out {
                out[done..].fill([0.0; 2]);
                break;
            }
            let rising = self.playing && self.seek.is_none() && self.roll == Roll::No;
            let mut part = if rising {
                frames - done
            } else {
                (self.fade as usize).clamp(1, frames - done)
            };
            let (stop_at, go_round_to) = match round {
                Some((from, to)) => (to, from),
                None => (Frames::MAX, 0),
            };
            if self.pos < stop_at {
                part = part.min((stop_at - self.pos) as usize);
            }
            let chunk = &mut out[done..done + part];
            self.peaks = [[0.0; METER_POINTS]; METERS];
            mix_tracks_metered(&self.project, self.pos, chunk, self.audition, Some(&mut self.peaks), &mut self.scratch, self.chains.as_deref_mut());
            let target = if self.project.master_muted { 0.0 } else { self.project.master };
            scale(chunk, self.master, target);
            self.master = target;
            let mut top = 0.0f32;
            for frame in chunk.iter() {
                top = top.max(frame[0].abs()).max(frame[1].abs());
            }
            for (index, peaks) in self.peaks.iter().enumerate() {
                for point in MeterPoint::ALL {
                    self.shared.note_track(index, point, peaks[point.index()]);
                }
            }
            self.shared.master_level.fetch_max(top.to_bits(), Ordering::Relaxed);
            if self.metronome && self.audition.is_none() {
                add_clicks(chunk, self.pos as f64, beat, rate);
            }
            if !rising || self.fade < self.fade_len {
                for frame in chunk.iter_mut() {
                    self.fade = if rising {
                        (self.fade + 1).min(self.fade_len)
                    } else {
                        self.fade.saturating_sub(1)
                    };
                    let level = self.fade as f32 / self.fade_len as f32;
                    frame[0] *= level;
                    frame[1] *= level;
                }
            }
            self.pos += part as Frames;
            done += part;
            if rising && stop_at > 0 && self.pos == stop_at {
                self.pos = go_round_to;
            }
        }
        self.block = block;
        if !self.playing || self.count_in > 0 {
            self.look_ahead();
        }
        self.play_live(frames);
        self.hear_input(frames);
        self.shared.pos.store(self.pos, Ordering::Relaxed);
        &self.block[..frames]
    }

    fn round_at(&self, at: Frames) -> Round {
        if self.endless {
            return None;
        }
        if let Some((from, to)) = self.loop_range {
            if at >= from && at < to {
                return Some((from, to));
            }
        }
        let end = self.project.length();
        match end > 0 && at < end {
            true => Some((0, end)),
            false => None,
        }
    }

    fn send_a_roll(&mut self, at: Frames, round: Round) {
        self.asked_back.store(false, Ordering::Relaxed);
        let parcel = Rolled {
            racks: self.chains.take(),
            scratch: std::mem::take(&mut self.scratch),
            project: self.project.clone(),
            at,
            round,
            only: self.audition,
            step: self.step,
        };
        match self.asking.push(parcel) {
            Ok(()) => self.roll = Roll::Out,
            Err(rtrb::PushError::Full(back)) => {
                self.scratch = back.scratch;
                self.chains = back.racks;
                self.roll = Roll::No;
            }
        }
    }

    fn collect_roll(&mut self) {
        let Ok(parcel) = self.rolled.pop() else { return };
        let Rolled { racks, scratch, project, .. } = parcel;
        self.scratch = scratch;
        match racks {
            Some(racks) if self.racks_wanted || self.chains.is_some() => {
                let _ = self.hand_back.push(racks);
                self.racks_wanted = false;
            }
            Some(racks) => self.chains = Some(racks),
            None => {}
        }
        let _ = self.retired.push(project);
        self.roll = Roll::No;
    }

    fn look_ahead(&mut self) {
        if self.roll != Roll::No || self.fade > 0 || self.ear.on.iter().any(Option::is_some) {
            return;
        }
        let at = match self.count_in > 0 {
            true => self.pos,
            false => self.loop_range.map(|(from, _)| from).unwrap_or(self.pos),
        };
        if self.looked_at != Some(at) {
            self.looked_at = Some(at);
            self.still = (STILL_FOR * self.project.rate as f32) as Frames;
            return;
        }
        self.still = self.still.saturating_sub(self.step as Frames);
        if self.still > 0 {
            return;
        }
        let round = self.round_at(at);
        if self.scratch.ready_for(&self.project, at, round, self.chains.as_deref()) == 0 {
            return;
        }
        self.send_a_roll(at, round);
    }

    fn note_monitor_lag(&mut self) {
        let mut most = 0;
        if let Some(racks) = self.chains.as_deref() {
            for id in self.ear.on.iter().flatten() {
                most = most.max(crate::render::monitor_latency(&self.project, *id, racks));
            }
        }
        self.shared.monitor_lag.store(most as u64, Ordering::Relaxed);
    }

    fn hear_input(&mut self, frames: usize) {
        self.note_monitor_lag();
        let ear = &mut self.ear;
        if ear.feed.is_none() {
            return;
        }
        let step = ear.rate as f64 / self.project.rate.max(1) as f64;
        if ear.on.iter().all(Option::is_none) {
            ear.let_go(ear.waiting());
            ear.filling = true;
            return;
        }
        if !ear.ready_for(frames, step) {
            return;
        }
        let width = ear.width;
        let pace = step * (1.0 + ear.nudge);
        let Some(feed) = ear.feed.as_mut() else { return };
        let mut ran_dry = false;
        for at in 0..frames {
            while ear.phase >= 1.0 {
                ear.was = ear.next;
                match feed.read_chunk(width) {
                    Ok(chunk) => {
                        let (front, back) = chunk.as_slices();
                        for (next, sample) in ear.next.iter_mut().zip(front.iter().chain(back)) {
                            *next = if sample.is_finite() { *sample } else { 0.0 };
                        }
                        chunk.commit_all();
                    }
                    Err(_) => {
                        ear.next[..width].fill(0.0);
                        ran_dry = true;
                    }
                }
                ear.phase -= 1.0;
            }
            let voice = &mut ear.voice[at * width..(at + 1) * width];
            for ((sample, was), next) in voice.iter_mut().zip(&ear.was).zip(&ear.next) {
                *sample = was + (next - was) * ear.phase as f32;
            }
            ear.phase += pace;
        }
        match ran_dry {
            true => ear.ran_dry(),
            false => ear.keep_close(frames as f64 * pace),
        }
        if self.ear.out.len() < frames {
            self.ear.out.resize(frames, [0.0; 2]);
        }
        self.ear.out[..frames].fill([0.0; 2]);
        match self.hear_through_chain {
            true => self.hear_through_the_chains(frames),
            false => self.hear_straight(frames),
        }
        self.monitor_chain(frames);
    }

    fn monitor_chain(&mut self, frames: usize) {
        let Self { ear, chains, block, shared, .. } = self;
        if let Some(racks) = chains.as_deref_mut() {
            racks.process_monitor(&mut ear.out[..frames]);
        }
        let mut top = 0.0f32;
        for (out, heard) in block[..frames].iter_mut().zip(ear.out[..frames].iter()) {
            top = top.max(heard[0].abs()).max(heard[1].abs());
            out[0] += heard[0];
            out[1] += heard[1];
        }
        shared.master_level.fetch_max(top.to_bits(), Ordering::Relaxed);
    }

    fn hear_straight(&mut self, frames: usize) {
        let ear = &mut self.ear;
        let width = ear.width;
        let master = if self.project.master_muted { 0.0 } else { self.project.master };
        for id in ear.on.iter().flatten() {
            let Some(index) = self.project.tracks.iter().position(|track| track.id == *id) else { continue };
            let track = &self.project.tracks[index];
            let sound = &mut ear.track[..frames];
            for (frame, heard) in sound.iter_mut().zip(ear.voice.chunks_exact(width)) {
                *frame = track.input.heard(heard);
            }
            self.shared.note_track(index, MeterPoint::PreFx, loudest(sound));
            if let Some(racks) = self.chains.as_deref_mut() {
                racks.process_takes(*id, sound);
            }
            self.shared.note_track(index, MeterPoint::PostFx, loudest(&ear.track[..frames]));
            let gain = if track.muted { 0.0 } else { track.gain * master };
            let (left, right) = pan_gains(track.pan);
            let mut top = 0.0f32;
            for (out, frame) in ear.out[..frames].iter_mut().zip(ear.track[..frames].iter()) {
                let heard = [frame[0] * gain * left, frame[1] * gain * right];
                top = top.max(heard[0].abs()).max(heard[1].abs());
                out[0] += heard[0];
                out[1] += heard[1];
            }
            self.shared.note_track(index, MeterPoint::PostFader, top);
        }
    }

    fn hear_through_the_chains(&mut self, frames: usize) {
        let project: &Project = &self.project;
        let count = project.tracks.len();
        let ear = &mut self.ear;
        room_to_hear(ear, count, frames);
        work_out_the_path(ear, project);
        let width = ear.width;
        let master = if project.master_muted { 0.0 } else { project.master };
        for index in 0..count {
            if ear.depth[index] == NOT_HEARD {
                continue;
            }
            ear.mix[index][..frames].fill([0.0; 2]);
            ear.sides[index][..frames].fill([0.0; 2]);
        }
        for level in 0..=ear.deepest {
            for index in 0..count {
                if ear.depth[index] != level {
                    continue;
                }
                let track = &project.tracks[index];
                let armed = ear.on.iter().flatten().any(|id| *id == track.id);
                let mut arriving = loudest(&ear.mix[index][..frames]);
                if armed {
                    let sound = &mut ear.track[..frames];
                    for (frame, heard) in sound.iter_mut().zip(ear.voice.chunks_exact(width)) {
                        *frame = track.input.heard(heard);
                    }
                    arriving = arriving.max(loudest(&ear.track[..frames]));
                    if let Some(racks) = self.chains.as_deref_mut() {
                        racks.process_takes(track.id, &mut ear.track[..frames]);
                    }
                    for (dest, from) in ear.mix[index][..frames].iter_mut().zip(ear.track[..frames].iter()) {
                        dest[0] += from[0];
                        dest[1] += from[1];
                    }
                } else if !track.fx.is_empty() {
                    if let Some(racks) = self.chains.as_deref_mut() {
                        let Ear { mix, sides, .. } = ear;
                        racks.hear_with(track.id, &mut mix[index][..frames], &sides[index][..frames]);
                    }
                }
                self.shared.note_track(index, MeterPoint::PreFx, arriving);
                self.shared.note_track(index, MeterPoint::PostFx, loudest(&ear.mix[index][..frames]));
                if track.sends.iter().any(|send| send.pre_fader) {
                    ear.pre[..frames].copy_from_slice(&ear.mix[index][..frames]);
                }
                let gain = if track.muted { 0.0 } else { track.gain };
                let (left, right) = pan_gains(track.pan);
                let mut top = 0.0f32;
                for frame in ear.mix[index][..frames].iter_mut() {
                    frame[0] *= gain * left;
                    frame[1] *= gain * right;
                    top = top.max(frame[0].abs()).max(frame[1].abs());
                }
                self.shared.note_track(index, MeterPoint::PostFader, top);
                for send in &track.sends {
                    let Some(to) = project.tracks.iter().position(|t| t.id == send.to) else { continue };
                    if ear.depth[to] == NOT_HEARD {
                        continue;
                    }
                    for i in 0..frames {
                        let from = if send.pre_fader { ear.pre[i] } else { ear.mix[index][i] };
                        let dest = if send.sidechain { &mut ear.sides[to][i] } else { &mut ear.mix[to][i] };
                        dest[0] += from[0] * send.gain;
                        dest[1] += from[1] * send.gain;
                    }
                }
                let up = track.parent.and_then(|parent| project.tracks.iter().position(|t| t.id == parent)).filter(|to| ear.depth[*to] != NOT_HEARD);
                match up {
                    Some(to) => {
                        for i in 0..frames {
                            let from = ear.mix[index][i];
                            let dest = &mut ear.mix[to][i];
                            dest[0] += from[0];
                            dest[1] += from[1];
                        }
                    }
                    None => {
                        let Ear { mix, out, .. } = ear;
                        for (heard, frame) in out[..frames].iter_mut().zip(mix[index][..frames].iter()) {
                            heard[0] += frame[0] * master;
                            heard[1] += frame[1] * master;
                        }
                    }
                }
            }
        }
    }

    fn taped(&mut self, track: TrackId, key: u8, velocity: f32) {
        if self.taping && self.playing {
            let _ = self.tape.push(TapedKey { track, key, velocity, at: self.pos });
        }
    }

    fn play_live(&mut self, frames: usize) {
        if self.live.is_empty() {
            return;
        }
        let rate = self.project.rate;
        let master = if self.project.master_muted { 0.0 } else { self.project.master };
        let project = &self.project;
        let out = &mut self.block[..frames];
        self.live.retain_mut(|voice| {
            let Some(track) = project.track(voice.track) else {
                return false;
            };
            let instrument = track.instrument;
            let sample = track.sample.as_deref().map(|sample| &sample.frames[..]);
            let gain = if track.muted { 0.0 } else { track.gain * master };
            instrument.play_with(sample, &voice.note, voice.played, out, |_| gain, rate);
            voice.played += frames as Frames;
            voice.note.len == HELD || voice.played < voice.note.len + instrument.tail_for(&voice.note, rate, sample)
        });
    }

    fn note_load(&mut self, spent: Duration, budget: Duration) {
        let load = spent.as_secs_f32() / budget.as_secs_f32().max(1e-6);
        self.load = self.load * LOAD_KEEP + load * (1.0 - LOAD_KEEP);
        self.load_peak = (self.load_peak * PEAK_KEEP).max(load);
        self.shared.load.store(self.load.to_bits(), Ordering::Relaxed);
        self.shared.load_peak.store(self.load_peak.to_bits(), Ordering::Relaxed);
        if load >= 1.0 {
            self.shared.overloads.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn heard(&self, at: Instant) {
        let shared = &self.shared;
        shared.heard_turn.fetch_add(1, Ordering::AcqRel);
        shared.heard_pos.store(self.pos.wrapping_sub(self.count_in), Ordering::Release);
        shared.heard_at.store(clock::nanos(at), Ordering::Release);
        shared.heard_turn.fetch_add(1, Ordering::AcqRel);
    }
}

pub struct Engine {
    remote: Remote,
    rate: u32,
    heard_tracks: bool,
    fed: bool,
    running: Option<Running>,
    output_error: Option<String>,
    output_failure: Option<Failure>,
    quit: Arc<AtomicBool>,
    host: Option<thread::JoinHandle<()>>,
}

struct Ready {
    remote: Remote,
    rate: u32,
    running: Option<Running>,
    error: Option<String>,
    failure: Option<Failure>,
}

impl Engine {
    pub fn start(output: Output) -> Self {
        clock::start();
        if let Output::Device(choice) = &output {
            devices::use_driver(choice.driver.as_deref());
        }
        let quit = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel();
        let host = thread::Builder::new()
            .name("loupe-audio".into())
            .spawn({
                let quit = quit.clone();
                move || host(output, ready_tx, quit)
            })
            .expect("start the audio thread");
        let ready: Ready = ready_rx.recv().expect("the audio thread reports in");
        Self {
            remote: ready.remote,
            rate: ready.rate,
            heard_tracks: false,
            fed: false,
            running: ready.running,
            output_error: ready.error,
            output_failure: ready.failure,
            quit,
            host: Some(host),
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn running(&self) -> Option<&Running> {
        self.running.as_ref()
    }

    pub fn output_error(&self) -> Option<&str> {
        self.output_error.as_deref()
    }

    pub fn output_failure(&self) -> Option<&Failure> {
        self.output_failure.as_ref()
    }

    pub fn set_project(&mut self, project: &Project) {
        self.send(Msg::Project(Arc::new(project.clone())));
    }

    pub fn play(&mut self) {
        self.remote.shared.playing.store(true, Ordering::Relaxed);
        self.send(Msg::Play);
    }

    pub fn stop(&mut self) {
        self.remote.shared.playing.store(false, Ordering::Relaxed);
        self.send(Msg::Stop);
    }

    pub fn seek(&mut self, to: Frames) {
        self.send(Msg::Seek(to));
    }

    pub fn set_loop(&mut self, range: Option<(Frames, Frames)>) {
        self.send(Msg::Loop(range));
    }

    pub fn audition(&mut self, clip: Option<ClipId>) {
        self.send(Msg::Audition(clip));
    }

    pub fn note_on(&mut self, track: TrackId, key: u8, velocity: f32) {
        self.send(Msg::NoteOn { track, key, velocity });
    }

    pub fn note_off(&mut self, track: TrackId, key: u8) {
        self.send(Msg::NoteOff { track, key });
    }

    pub fn silence_notes(&mut self) {
        self.send(Msg::Silence);
    }

    pub fn key_sender(&self) -> KeySender {
        self.remote.keys.clone()
    }

    pub fn keys_go_to(&mut self, track: Option<TrackId>) {
        self.send(Msg::KeysGoTo(track));
    }

    pub fn position(&self) -> Frames {
        self.remote.shared.pos.load(Ordering::Relaxed)
    }

    pub fn monitor_lag(&self) -> Frames {
        self.remote.shared.monitor_lag.load(Ordering::Relaxed)
    }

    pub fn load(&self) -> Load {
        let shared = &self.remote.shared;
        Load {
            average: f32::from_bits(shared.load.load(Ordering::Relaxed)),
            peak: f32::from_bits(shared.load_peak.load(Ordering::Relaxed)),
            overloads: shared.overloads.load(Ordering::Relaxed),
        }
    }

    pub fn take_levels(&self) -> ([TrackPeaks; METERS], f32) {
        self.remote.shared.take_levels()
    }

    pub fn set_metronome(&mut self, on: bool) {
        self.send(Msg::Metronome(on));
    }

    pub fn count_in(&mut self, len: Frames) {
        self.send(Msg::CountIn(len));
    }

    pub fn tape_keys(&mut self, on: bool) {
        self.send(Msg::Tape(on));
    }

    pub fn now_recording(&mut self, on: bool) {
        self.send(Msg::Recording(on));
    }

    pub fn taped_keys(&mut self) -> Vec<TapedKey> {
        std::iter::from_fn(|| self.remote.tape.pop().ok()).collect()
    }

    pub fn set_endless(&mut self, endless: bool) {
        self.send(Msg::Endless(endless));
    }

    pub fn position_at(&self, when: Instant) -> i64 {
        let shared = &self.remote.shared;
        let (pos, at) = loop {
            let turn = shared.heard_turn.load(Ordering::Acquire);
            let seen = (shared.heard_pos.load(Ordering::Acquire), shared.heard_at.load(Ordering::Acquire));
            if turn % 2 == 0 && shared.heard_turn.load(Ordering::Acquire) == turn {
                break seen;
            }
            std::hint::spin_loop();
        };
        let later = clock::nanos(when) as i128 - at as i128;
        pos as i64 + (later * self.rate as i128 / 1_000_000_000) as i64
    }

    pub fn is_playing(&self) -> bool {
        self.remote.shared.playing.load(Ordering::Relaxed)
    }

    pub fn is_hearing(&self) -> bool {
        self.fed && self.heard_tracks
    }

    pub fn use_chains(&mut self, chains: Box<dyn Chains>) {
        self.send(Msg::Chains(Some(chains)));
    }

    pub fn tweak(&mut self, track: TrackId, slot: usize, knob: usize, value: f32) {
        self.send(Msg::Tweak { track, slot, knob, value });
    }

    pub fn tweak_clip(&mut self, clip: ClipId, slot: usize, knob: usize, value: f32) {
        self.send(Msg::TweakClip { clip, slot, knob, value });
    }

    pub fn hear(&mut self, input: &mut crate::input::Input) {
        let rate = input.rate();
        if let Some((feed, width)) = input.take_ear() {
            self.fed = true;
            self.send(Msg::Hear(Some((feed, rate, width))));
        }
    }

    pub fn stop_hearing(&mut self) {
        self.fed = false;
        self.send(Msg::Hear(None));
    }

    pub fn hear_on(&mut self, tracks: &[TrackId]) {
        self.heard_tracks = !tracks.is_empty();
        let mut on = [None; MOST_HEARD];
        for (slot, track) in on.iter_mut().zip(tracks) {
            *slot = Some(*track);
        }
        self.send(Msg::HearOn(on));
    }

    pub fn hear_through_the_chains(&mut self, on: bool) {
        self.send(Msg::HearThroughChain(on));
    }

    pub fn drop_chains(&mut self) {
        self.send(Msg::Chains(None));
    }

    pub fn chains_back(&mut self) -> Option<Box<dyn Chains>> {
        self.remote.handed_back.pop().ok()
    }

    pub fn collect(&mut self) {
        while self.remote.retired.pop().is_ok() {}
        while self.remote.done_feeds.pop().is_ok() {}
    }

    fn send(&mut self, mut msg: Msg) {
        self.collect();
        for _ in 0..50 {
            match self.remote.outbox.push(msg) {
                Ok(()) => return,
                Err(rtrb::PushError::Full(back)) => msg = back,
            }
            thread::sleep(Duration::from_millis(1));
            self.collect();
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.quit.store(true, Ordering::Relaxed);
        if let Some(host) = self.host.take() {
            host.thread().unpark();
            let _ = host.join();
        }
    }
}

const BEATS_PER_BAR: f64 = 4.0;
const CLICK_SECONDS: f64 = 0.03;
const CLICK_HERTZ: f64 = 1000.0;
const ACCENT_HERTZ: f64 = 1600.0;
const CLICK_LEVEL: f32 = 0.3;
const ACCENT_LEVEL: f32 = 0.45;

fn beat_frames(bpm: f64, rate: u32) -> f64 {
    rate as f64 * 60.0 / bpm.max(1.0)
}

pub fn bar_frames(bpm: f64, rate: u32) -> Frames {
    (beat_frames(bpm, rate) * BEATS_PER_BAR).round() as Frames
}

fn add_clicks(out: &mut [[f32; 2]], from: f64, beat: f64, rate: u32) {
    let rate = rate as f64;
    let ringing = CLICK_SECONDS * rate;
    let first = (from / beat).floor();
    let last = ((from + out.len() as f64) / beat).floor();
    let mut which = first;
    while which <= last {
        let began = which * beat;
        let into = began - from;
        let start = into.max(0.0) as usize;
        let stop = ((into + ringing).ceil() as isize).clamp(0, out.len() as isize) as usize;
        if start >= stop {
            which += 1.0;
            continue;
        }
        let accent = (which as i64).rem_euclid(BEATS_PER_BAR as i64) == 0;
        let (hertz, level) = if accent { (ACCENT_HERTZ, ACCENT_LEVEL) } else { (CLICK_HERTZ, CLICK_LEVEL) };
        for (i, frame) in out[start..stop].iter_mut().enumerate() {
            let since = (from + (start + i) as f64 - began) / rate;
            if since < 0.0 || since >= CLICK_SECONDS {
                continue;
            }
            let tone = (std::f64::consts::TAU * hertz * since).sin() * (-since * 150.0).exp();
            let sample = tone as f32 * level;
            frame[0] += sample;
            frame[1] += sample;
        }
        which += 1.0;
    }
}

fn host(output: Output, ready: mpsc::Sender<Ready>, quit: Arc<AtomicBool>) {
    let mut error = None;
    let mut failure = None;
    if let Output::Device(choice) = &output {
        match open_device(choice) {
            Ok((stream, remote, running, warning)) => {
                let _ = ready.send(Ready { remote, rate: running.rate, running: Some(running), error: warning, failure: None });
                while !quit.load(Ordering::Relaxed) {
                    thread::park_timeout(Duration::from_millis(200));
                }
                drop(stream);
                return;
            }
            Err(e) => {
                failure = Some(Failure { driver: choice.driver.clone(), output: choice.output.clone(), reason: e.clone() });
                error = Some(e);
            }
        }
    }

    let rate = match output {
        Output::SilentAt(rate) => rate,
        _ => SILENT_RATE,
    };
    let (mut rt, remote) = pair(rate);
    let _ = ready.send(Ready { remote, rate, running: None, error, failure });
    let block = rate as usize / 100;
    let mut next = Instant::now();
    while !quit.load(Ordering::Relaxed) {
        let began = Instant::now();
        rt.process(block);
        rt.note_load(began.elapsed(), Duration::from_secs_f64(block as f64 / rate as f64));
        next += Duration::from_millis(10);
        rt.heard(next);
        match next.checked_duration_since(Instant::now()) {
            Some(wait) => thread::park_timeout(wait),
            None => next = Instant::now(),
        }
    }
}

fn open_device(choice: &Device) -> Result<(cpal::Stream, Remote, Running, Option<String>), String> {
    let host = devices::host_named(choice.driver.as_deref());
    let device = devices::output_named(host, choice.output.as_deref()).ok_or("no sound output found")?;
    let default = device.default_output_config().map_err(|e| e.to_string())?;
    let supported = match choice.rate {
        Some(rate) if rate != default.sample_rate().0 => device
            .supported_output_configs()
            .ok()
            .and_then(|configs| {
                let fits: Vec<_> = configs.filter(|c| (c.min_sample_rate().0..=c.max_sample_rate().0).contains(&rate)).collect();
                let best = fits.iter().find(|c| c.channels() == default.channels() && c.sample_format() == default.sample_format()).or_else(|| fits.iter().find(|c| c.channels() >= 2)).or(fits.first());
                best.cloned()
            })
            .map(|c| c.with_sample_rate(cpal::SampleRate(rate)))
            .unwrap_or(default),
        _ => default,
    };
    let format = supported.sample_format();
    let mut config: cpal::StreamConfig = supported.into();
    let mut warning = None;
    if let Some(size) = choice.buffer.filter(|_| !devices::one_device_both_ways()) {
        config.buffer_size = cpal::BufferSize::Fixed(size);
    }
    let stream = match start_stream(&device, &config, format) {
        Err(_) if choice.buffer.is_some() => {
            config.buffer_size = cpal::BufferSize::Default;
            warning = Some("the sound output refused that buffer size, so it uses its own".to_string());
            start_stream(&device, &config, format)
        }
        other => other,
    };
    let (stream, remote) = stream?;
    if choice.rate.is_some_and(|rate| rate != config.sample_rate.0) {
        warning = Some(format!("the sound output cannot run at that sample rate, so it runs at {} Hz", config.sample_rate.0));
    }
    let running = Running {
        driver: host.id().name().to_string(),
        output: device.name().unwrap_or_default(),
        rate: config.sample_rate.0,
        buffer: match config.buffer_size {
            cpal::BufferSize::Fixed(size) => Some(size),
            cpal::BufferSize::Default => None,
        },
    };
    devices::hold(&device);
    Ok((stream, remote, running, warning))
}

fn start_stream(device: &cpal::Device, config: &cpal::StreamConfig, format: cpal::SampleFormat) -> Result<(cpal::Stream, Remote), String> {
    let (rt, remote) = pair(config.sample_rate.0);
    let stream = match format {
        cpal::SampleFormat::F32 => stream::<f32>(device, config, rt),
        cpal::SampleFormat::I16 => stream::<i16>(device, config, rt),
        cpal::SampleFormat::U16 => stream::<u16>(device, config, rt),
        cpal::SampleFormat::I32 => stream::<i32>(device, config, rt),
        other => return Err(format!("the sound output uses a format Loupe cannot write ({other})")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, remote))
}

fn stream<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut rt: Rt,
) -> Result<cpal::Stream, cpal::BuildStreamError> {
    let channels = config.channels as usize;
    let rate = config.sample_rate.0 as f64;
    device.build_output_stream(
        config,
        move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
            let stamp = info.timestamp();
            let delay = stamp.playback.duration_since(&stamp.callback).unwrap_or_default();
            let began = Instant::now();
            let mut heard = began + delay;
            let mut made = 0;
            for part in data.chunks_mut(MAX_BLOCK * channels) {
                let frames = part.len() / channels;
                made += frames;
                let mixed = rt.process(frames);
                for (frame, m) in part.chunks_mut(channels).zip(mixed) {
                    match frame {
                        [mono] => *mono = T::from_sample((m[0] + m[1]) * 0.5),
                        [left, right, rest @ ..] => {
                            *left = T::from_sample(m[0]);
                            *right = T::from_sample(m[1]);
                            rest.fill(T::from_sample(0.0));
                        }
                        [] => {}
                    }
                }
                heard += Duration::from_secs_f64(frames as f64 / rate);
                rt.heard(heard);
            }
            rt.note_load(began.elapsed(), Duration::from_secs_f64(made as f64 / rate));
        },
        |e| eprintln!("loupe: sound output error: {e}"),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Command, Outcome};
    use crate::source::Source;

    const RATE: u32 = 48_000;

    fn steady(len: usize) -> Project {
        let mut p = Project::new(RATE);
        let Ok(Outcome::Track(track)) = p.apply(Command::AddTrack { name: "T".into() }) else {
            panic!("no track")
        };
        let source = Arc::new(Source::from_frames("one", vec![[1.0, 1.0]; len]));
        p.apply(Command::AddClip { track, source, start: 0 }).unwrap();
        p
    }

    #[test]
    fn the_engine_knows_while_it_is_hearing_the_input() {
        let mut engine = Engine::start(Output::Silent);
        let mut input = crate::input::Input::open(crate::input::InputChoice::Practice).expect("the practice input opens");
        assert!(!engine.is_hearing(), "nothing is heard before an input is given");
        engine.hear(&mut input);
        assert!(!engine.is_hearing(), "an input with no armed track is not heard");
        engine.hear_on(&[TrackId(1)]);
        assert!(engine.is_hearing(), "an armed track with an input is heard");
        engine.hear_on(&[]);
        assert!(!engine.is_hearing(), "disarming every track stops the hearing");
        engine.hear_on(&[TrackId(1)]);
        engine.stop_hearing();
        assert!(!engine.is_hearing(), "taking the input away stops the hearing");
    }

    #[test]
    fn levels_fall_to_nothing_when_stopped() {
        let mut engine = Engine::start(Output::Silent);
        engine.set_project(&steady(SILENT_RATE as usize * 5));
        engine.play();
        thread::sleep(Duration::from_millis(150));
        engine.stop();
        thread::sleep(Duration::from_millis(150));
        let _ = engine.take_levels();
        thread::sleep(Duration::from_millis(50));
        let (tracks, master) = engine.take_levels();
        assert_eq!(tracks[0][MeterPoint::PostFader.index()], 0.0, "track level stayed at {}", tracks[0][MeterPoint::PostFader.index()]);
        assert_eq!(master, 0.0, "master level stayed at {master}");
    }

    #[test]
    fn the_engine_publishes_levels_while_playing() {
        let mut engine = Engine::start(Output::Silent);
        engine.set_project(&steady(SILENT_RATE as usize * 5));
        engine.play();
        thread::sleep(Duration::from_millis(200));
        let (tracks, master) = engine.take_levels();
        assert!(tracks[0][MeterPoint::PostFader.index()] > 0.0, "track level stayed at {}", tracks[0][MeterPoint::PostFader.index()]);
        assert!(master > 0.0, "master level stayed at {master}");
    }

    #[test]
    fn a_playing_track_reports_a_level() {
        let project = steady(256);
        let mut out = vec![[0.0f32; 2]; 64];
        let mut peaks = [[0.0f32; METER_POINTS]; METERS];
        crate::render::mix_tracks_metered(&project, 0, &mut out, None, Some(&mut peaks), &mut crate::render::Mixdown::default(), None);
        assert!(peaks[0][MeterPoint::PostFader.index()] > 0.0, "track 0 should report a level, got {:?}", peaks[0]);
    }

    fn rig(len: usize) -> (Rt, Remote) {
        let (rt, mut remote) = pair(RATE);
        remote.outbox.push(Msg::Project(Arc::new(steady(len)))).ok().unwrap();
        (rt, remote)
    }

    #[test]
    fn a_loud_block_is_still_reported_after_quieter_blocks_before_the_read() {
        let (mut rt, mut remote) = pair(RATE);
        let mut project = Project::new(RATE);
        let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: "T".into() }) else { panic!() };
        let mut frames = vec![[0.9f32, 0.9]; 256];
        frames.extend(vec![[0.1f32, 0.1]; RATE as usize]);
        project.apply(Command::AddClip { track, source: Arc::new(Source::from_frames("burst", frames)), start: 0 }).unwrap();
        remote.outbox.push(Msg::Project(Arc::new(project))).ok().unwrap();
        remote.outbox.push(Msg::Play).ok().unwrap();
        for _ in 0..4 {
            rt.process(256);
        }
        assert!(remote.shared.pos.load(Ordering::Relaxed) >= 1024, "the song did not play past the loud block");
        let (tracks, master) = remote.shared.take_levels();
        assert!((tracks[0][MeterPoint::PostFader.index()] - 0.9).abs() < 1e-3, "the track read {} after a 0.9 block", tracks[0][MeterPoint::PostFader.index()]);
        assert!((master - 0.9).abs() < 1e-3, "the master read {master} after a 0.9 block");
        rt.process(256);
        let (tracks, master) = remote.shared.take_levels();
        assert!((tracks[0][MeterPoint::PostFader.index()] - 0.1).abs() < 1e-3, "the read did not clear, the track read {}", tracks[0][MeterPoint::PostFader.index()]);
        assert!((master - 0.1).abs() < 1e-3, "the read did not clear, the master read {master}");
    }

    #[test]
    fn engine_load_is_time_spent_over_time_allowed_and_overruns_are_counted() {
        let (mut rt, _remote) = rig(1_000);
        let budget = Duration::from_millis(10);
        rt.note_load(Duration::from_millis(5), budget);
        assert!((f32::from_bits(rt.shared.load_peak.load(Ordering::Relaxed)) - 0.5).abs() < 1e-3);
        assert_eq!(rt.shared.overloads.load(Ordering::Relaxed), 0);
        for _ in 0..200 {
            rt.note_load(Duration::from_millis(3), budget);
        }
        assert!((f32::from_bits(rt.shared.load.load(Ordering::Relaxed)) - 0.3).abs() < 0.01);
        rt.note_load(Duration::from_millis(12), budget);
        assert_eq!(rt.shared.overloads.load(Ordering::Relaxed), 1);
        assert!(f32::from_bits(rt.shared.load_peak.load(Ordering::Relaxed)) >= 1.2 - 1e-3);
    }

    #[test]
    fn stopped_is_silent_and_still() {
        let (mut rt, remote) = rig(RATE as usize);
        assert!(rt.process(480).iter().all(|f| *f == [0.0, 0.0]));
        assert_eq!(remote.shared.pos.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn play_fades_in_then_runs_at_full_level() {
        let (mut rt, mut remote) = rig(RATE as usize);
        remote.outbox.push(Msg::Play).ok().unwrap();
        let first = rt.process(480).to_vec();
        assert!(first[0][0] > 0.0 && first[0][0] < 0.01, "starts near silence");
        assert!(first.windows(2).all(|w| w[1][0] >= w[0][0]), "only rises");
        assert_eq!(first[479], [1.0, 1.0]);
        assert!(rt.process(480).iter().all(|f| *f == [1.0, 1.0]));
        assert_eq!(remote.shared.pos.load(Ordering::Relaxed), 960);
    }

    #[test]
    fn stop_fades_out_and_holds_its_place() {
        let (mut rt, mut remote) = rig(RATE as usize);
        remote.outbox.push(Msg::Play).ok().unwrap();
        rt.process(480);
        remote.outbox.push(Msg::Stop).ok().unwrap();
        let out = rt.process(480).to_vec();
        assert!(out[0][0] < 1.0 && out[0][0] > 0.9, "fades from full level");
        assert_eq!(out[479], [0.0, 0.0]);
        assert_eq!(remote.shared.pos.load(Ordering::Relaxed), 480 + 240);
        rt.process(480);
        assert_eq!(remote.shared.pos.load(Ordering::Relaxed), 480 + 240);
    }

    #[test]
    fn seek_while_stopped_moves_at_once() {
        let (mut rt, mut remote) = rig(RATE as usize);
        remote.outbox.push(Msg::Seek(12_345)).ok().unwrap();
        rt.process(480);
        assert_eq!(remote.shared.pos.load(Ordering::Relaxed), 12_345);
    }

    #[test]
    fn seek_while_playing_fades_jumps_and_carries_on() {
        let (mut rt, mut remote) = rig(RATE as usize);
        remote.outbox.push(Msg::Play).ok().unwrap();
        rt.process(480);
        remote.outbox.push(Msg::Seek(24_000)).ok().unwrap();
        let out = rt.process(960).to_vec();
        assert_eq!(out[239], [0.0, 0.0], "faded out before the jump");
        assert_eq!(out[959], [1.0, 1.0], "back at full level after it");
        assert_eq!(remote.shared.pos.load(Ordering::Relaxed), 24_000 + 720);
    }

    #[test]
    fn playback_goes_round_to_the_start_at_the_end_of_the_song() {
        let (mut rt, mut remote) = rig(1000);
        remote.outbox.push(Msg::Play).ok().unwrap();
        rt.process(480);
        rt.process(480);
        let out = rt.process(480).to_vec();
        assert!(out.iter().all(|f| *f == [1.0, 1.0]), "no gap at the join");
        assert_eq!(remote.shared.pos.load(Ordering::Relaxed), 440);
    }

    #[test]
    fn a_loop_range_plays_round_and_round() {
        let (mut rt, mut remote) = rig(1000);
        remote.outbox.push(Msg::Loop(Some((100, 300)))).ok().unwrap();
        remote.outbox.push(Msg::Seek(100)).ok().unwrap();
        remote.outbox.push(Msg::Play).ok().unwrap();
        rt.process(480);
        assert_eq!(remote.shared.pos.load(Ordering::Relaxed), 180);
        remote.outbox.push(Msg::Loop(None)).ok().unwrap();
        rt.process(480);
        assert_eq!(remote.shared.pos.load(Ordering::Relaxed), 180 + 480);
    }

    #[test]
    fn endless_playback_runs_past_the_loop_and_the_end_of_the_song() {
        let (mut rt, mut remote) = rig(1000);
        remote.outbox.push(Msg::Loop(Some((100, 300)))).ok().unwrap();
        remote.outbox.push(Msg::Seek(100)).ok().unwrap();
        remote.outbox.push(Msg::Endless(true)).ok().unwrap();
        remote.outbox.push(Msg::Play).ok().unwrap();
        for _ in 0..3 {
            rt.process(480);
        }
        assert_eq!(remote.shared.pos.load(Ordering::Relaxed), 100 + 1440);
    }

    #[test]
    fn the_engine_knows_where_the_song_was_at_an_earlier_moment() {
        let mut engine = Engine::start(Output::Silent);
        engine.set_endless(true);
        engine.seek(10_000);
        thread::sleep(Duration::from_millis(40));
        let pressed = Instant::now();
        engine.play();
        thread::sleep(Duration::from_millis(300));
        let was = engine.position_at(pressed);
        assert!((was - 10_000).abs() < 1_500, "play was pressed at about 10000, got {was}");
        let now = engine.position_at(Instant::now());
        assert!((now - was - 14_400).abs() < 1_500, "0.3 s later, got {} more", now - was);
    }

    #[test]
    fn a_sound_output_that_will_not_open_is_named_with_the_reason() {
        let choice = Device { driver: Some("No such driver".into()), output: Some("Not a real sound card".into()), rate: None, buffer: None };
        let engine = Engine::start(Output::Device(choice));
        match engine.output_failure() {
            Some(failure) => {
                assert_eq!(failure.output.as_deref(), Some("Not a real sound card"));
                assert!(!failure.reason.is_empty());
                assert!(engine.running().is_none());
            }
            None => assert!(engine.running().is_some(), "no failure and nothing running"),
        }
    }

    #[test]
    fn a_silent_engine_has_no_sound_output_to_fail() {
        let engine = Engine::start(Output::Silent);
        assert!(engine.output_failure().is_none() && engine.running().is_none());
    }

    #[test]
    fn an_empty_song_just_keeps_rolling() {
        let (mut rt, mut remote) = pair(RATE);
        remote.outbox.push(Msg::Play).ok().unwrap();
        for _ in 0..3 {
            rt.process(480);
        }
        assert_eq!(remote.shared.pos.load(Ordering::Relaxed), 1440);
    }

    #[test]
    fn a_replaced_project_is_handed_back_not_freed() {
        let (mut rt, mut remote) = rig(1000);
        rt.process(64);
        remote.outbox.push(Msg::Project(Arc::new(steady(2000)))).ok().unwrap();
        rt.process(64);
        assert_eq!(std::iter::from_fn(|| remote.retired.pop().ok()).count(), 2);
    }

    #[test]
    fn a_key_sounds_while_held_even_when_stopped_and_fades_after() {
        let (mut rt, mut remote) = pair(RATE);
        let mut project = Project::new(RATE);
        let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: "Keys".into() }) else { panic!() };
        remote.outbox.push(Msg::Project(Arc::new(project))).ok().unwrap();
        assert!(rt.process(480).iter().all(|f| *f == [0.0, 0.0]));
        remote.outbox.push(Msg::NoteOn { track, key: 60, velocity: 1.0 }).ok().unwrap();
        let loud = rt.process(4_000).iter().map(|f| f[0].abs()).fold(0.0, f32::max);
        assert!(loud > 0.05, "{loud}");
        remote.outbox.push(Msg::NoteOff { track, key: 60 }).ok().unwrap();
        for _ in 0..12 {
            rt.process(4_000);
        }
        assert!(rt.process(480).iter().all(|f| *f == [0.0, 0.0]), "silent after the release");
        assert!(rt.live.is_empty());
        for key in 0..40 {
            remote.outbox.push(Msg::NoteOn { track, key, velocity: 1.0 }).ok().unwrap();
        }
        rt.process(64);
        assert_eq!(rt.live.len(), MOST_LIVE_NOTES);
        remote.outbox.push(Msg::Silence).ok().unwrap();
        rt.process(64);
        assert!(rt.live.is_empty());
    }

    struct Halves;

    impl Chains for Halves {
        fn process_with(&mut self, _: TrackId, audio: &mut [[f32; 2]], _: &[[f32; 2]]) {
            audio.fill([9.0; 2]);
        }

        fn process_takes(&mut self, _: TrackId, audio: &mut [[f32; 2]]) {
            for frame in audio.iter_mut() {
                frame[0] *= 0.5;
                frame[1] *= 0.5;
            }
        }
    }

    #[test]
    fn the_input_is_heard_on_armed_tracks_through_their_recording_plugins_only() {
        let (mut rt, mut remote) = pair(RATE);
        let mut project = Project::new(RATE);
        let Ok(Outcome::Track(vocal)) = project.apply(Command::AddTrack { name: "Vocal".into() }) else { panic!() };
        project.apply(Command::SetTrackPan { track: vocal, pan: 0.0 }).unwrap();
        remote.outbox.push(Msg::Project(Arc::new(project))).ok().unwrap();
        remote.outbox.push(Msg::Chains(Some(Box::new(Halves)))).ok().unwrap();
        let (mut mic, feed) = RingBuffer::new(1 << 14);
        remote.outbox.push(Msg::Hear(Some((feed, RATE, 1)))).ok().unwrap();
        for _ in 0..4_000 {
            mic.push(0.8).unwrap();
        }
        assert!(rt.process(256).iter().all(|f| *f == [0.0, 0.0]), "nothing is armed yet");
        remote.outbox.push(Msg::HearOn([Some(vocal), None, None, None, None, None, None, None])).ok().unwrap();
        for _ in 0..4_000 {
            mic.push(0.8).unwrap();
        }
        rt.process(256);
        for _ in 0..256 {
            mic.push(0.8).unwrap();
        }
        let heard = rt.process(256).to_vec();
        let (left, _) = pan_gains(0.0);
        for frame in heard {
            assert!((frame[0] - 0.8 * 0.5 * left).abs() < 1e-4, "got {frame:?}");
        }
        assert!(!rt.playing, "hearing the input does not start the song");
        remote.outbox.push(Msg::HearOn([None; MOST_HEARD])).ok().unwrap();
        assert!(rt.process(256).iter().all(|f| *f == [0.0, 0.0]));
    }

    #[derive(Default)]
    struct Clock {
        songs: Arc<std::sync::Mutex<Vec<crate::render::Song>>>,
    }

    impl Chains for Clock {
        fn song_goes(&mut self, song: crate::render::Song) {
            if let Ok(mut kept) = self.songs.lock() {
                kept.push(song);
            }
        }
    }

    fn last_song(songs: &Arc<std::sync::Mutex<Vec<crate::render::Song>>>) -> crate::render::Song {
        *songs.lock().expect("the songs are there").last().expect("the engine told the plugins nothing")
    }

    #[test]
    fn the_plugins_are_told_the_songs_tempo_and_whether_the_transport_is_rolling() {
        let (mut rt, mut remote) = pair(RATE);
        let mut project = steady(4_096);
        project.apply(Command::SetBpm(142.5)).unwrap();
        let songs = Arc::new(std::sync::Mutex::new(Vec::new()));
        remote.outbox.push(Msg::Project(Arc::new(project))).ok().unwrap();
        remote.outbox.push(Msg::Chains(Some(Box::new(Clock { songs: songs.clone() })))).ok().unwrap();
        rt.process(256);
        let stopped = last_song(&songs);
        assert_eq!(stopped.tempo, 142.5, "the plugins were told the wrong tempo");
        assert!(!stopped.playing, "a stopped transport was passed on as rolling");
        assert!(!stopped.recording);
        assert_eq!((stopped.beats, stopped.beat_unit), (4, 4));

        remote.outbox.push(Msg::Play).ok().unwrap();
        rt.process(256);
        let rolling = last_song(&songs);
        assert!(rolling.playing, "the plugins were not told the transport is rolling");
        rt.process(256);
        let later = last_song(&songs);
        assert!(later.at > rolling.at, "the place in the song never moved on");

        remote.outbox.push(Msg::Recording(true)).ok().unwrap();
        rt.process(256);
        assert!(last_song(&songs).recording, "the plugins were not told the song is being recorded");

        remote.outbox.push(Msg::Stop).ok().unwrap();
        remote.outbox.push(Msg::Recording(false)).ok().unwrap();
        for _ in 0..4 {
            rt.process(256);
        }
        let after = last_song(&songs);
        assert!(!after.playing && !after.recording, "stopping never reached the plugins");
    }

    struct Monitoring {
        runs: Arc<AtomicU64>,
    }

    impl Chains for Monitoring {
        fn process_takes(&mut self, _: TrackId, audio: &mut [[f32; 2]]) {
            for frame in audio.iter_mut() {
                frame[0] *= 0.5;
                frame[1] *= 0.5;
            }
        }

        fn process_monitor(&mut self, audio: &mut [[f32; 2]]) {
            self.runs.fetch_add(1, Ordering::Relaxed);
            for frame in audio.iter_mut() {
                frame[0] *= 0.25;
                frame[1] *= 0.25;
            }
        }

        fn monitor_chain_latency(&self) -> usize {
            64
        }
    }

    fn armed_and_singing(rt: &mut Rt, remote: &mut Remote, runs: Arc<AtomicU64>) -> (TrackId, Producer<f32>) {
        let mut project = Project::new(RATE);
        let Ok(Outcome::Track(vocal)) = project.apply(Command::AddTrack { name: "Vox 1".into() }) else { panic!() };
        remote.outbox.push(Msg::Project(Arc::new(project))).ok().unwrap();
        remote.outbox.push(Msg::Chains(Some(Box::new(Monitoring { runs })))).ok().unwrap();
        let (mut mic, feed) = RingBuffer::new(1 << 14);
        remote.outbox.push(Msg::Hear(Some((feed, RATE, 1)))).ok().unwrap();
        for _ in 0..8_000 {
            mic.push(0.8).unwrap();
        }
        rt.process(256);
        (vocal, mic)
    }

    #[test]
    fn the_monitored_input_is_heard_through_the_projects_monitoring_chain() {
        let (mut rt, mut remote) = pair(RATE);
        let runs = Arc::new(AtomicU64::new(0));
        let (vocal, mut mic) = armed_and_singing(&mut rt, &mut remote, runs.clone());
        assert_eq!(runs.load(Ordering::Relaxed), 0, "the monitoring chain ran with nothing armed");
        remote.outbox.push(Msg::HearOn([Some(vocal), None, None, None, None, None, None, None])).ok().unwrap();
        for _ in 0..8_000 {
            mic.push(0.8).unwrap();
        }
        rt.process(256);
        for _ in 0..256 {
            mic.push(0.8).unwrap();
        }
        let heard = rt.process(256).to_vec();
        let (left, _) = pan_gains(0.0);
        for frame in heard {
            assert!((frame[0] - 0.8 * 0.5 * 0.25 * left).abs() < 1e-4, "got {frame:?}");
        }
        assert!(runs.load(Ordering::Relaxed) > 0, "the monitoring chain never ran");
    }

    #[test]
    fn playback_never_goes_through_the_monitoring_chain() {
        let (mut rt, mut remote) = pair(RATE);
        let runs = Arc::new(AtomicU64::new(0));
        let (_, _mic) = armed_and_singing(&mut rt, &mut remote, runs.clone());
        remote.outbox.push(Msg::Project(Arc::new(steady(2_048)))).ok().unwrap();
        remote.outbox.push(Msg::Play).ok().unwrap();
        for _ in 0..4 {
            rt.process(256);
        }
        assert_eq!(runs.load(Ordering::Relaxed), 0, "playback went through the monitoring chain");
        assert!(rt.process(256).iter().any(|frame| frame[0].abs() > 1e-6), "the song is silent, so it proves nothing");
    }

    #[test]
    fn the_monitoring_chain_is_heard_the_short_way_too() {
        let (mut rt, mut remote) = pair(RATE);
        let runs = Arc::new(AtomicU64::new(0));
        let (vocal, mut mic) = armed_and_singing(&mut rt, &mut remote, runs.clone());
        remote.outbox.push(Msg::HearThroughChain(false)).ok().unwrap();
        remote.outbox.push(Msg::HearOn([Some(vocal), None, None, None, None, None, None, None])).ok().unwrap();
        for _ in 0..8_000 {
            mic.push(0.8).unwrap();
        }
        rt.process(256);
        for _ in 0..256 {
            mic.push(0.8).unwrap();
        }
        let heard = rt.process(256).to_vec();
        let (left, _) = pan_gains(0.0);
        for frame in heard {
            assert!((frame[0] - 0.8 * 0.5 * 0.25 * left).abs() < 1e-4, "got {frame:?}");
        }
        assert!(runs.load(Ordering::Relaxed) > 0, "the short way skipped the monitoring chain");
    }

    #[test]
    fn what_the_monitoring_chain_adds_is_counted_in_the_lag_the_singer_hears() {
        let (mut rt, mut remote) = pair(RATE);
        let runs = Arc::new(AtomicU64::new(0));
        let (vocal, _mic) = armed_and_singing(&mut rt, &mut remote, runs);
        remote.outbox.push(Msg::HearOn([Some(vocal), None, None, None, None, None, None, None])).ok().unwrap();
        rt.process(256);
        assert_eq!(remote.shared.monitor_lag.load(Ordering::Relaxed), 64);
    }

    #[test]
    fn each_heard_track_hears_its_own_input() {
        let (mut rt, mut remote) = pair(RATE);
        let mut project = Project::new(RATE);
        let Ok(Outcome::Track(vocal)) = project.apply(Command::AddTrack { name: "Vocal".into() }) else { panic!() };
        let Ok(Outcome::Track(keys)) = project.apply(Command::AddTrack { name: "Keys".into() }) else { panic!() };
        project.apply(Command::SetTrackInput { track: vocal, input: crate::model::InputChannels::Mono(2) }).unwrap();
        project.apply(Command::SetTrackInput { track: keys, input: crate::model::InputChannels::Stereo(0) }).unwrap();
        project.apply(Command::SetTrackMuted { track: keys, muted: true }).unwrap();
        remote.outbox.push(Msg::Project(Arc::new(project.clone()))).ok().unwrap();
        let (mut mic, feed) = RingBuffer::new(1 << 15);
        remote.outbox.push(Msg::Hear(Some((feed, RATE, 3)))).ok().unwrap();
        remote.outbox.push(Msg::HearOn([Some(vocal), Some(keys), None, None, None, None, None, None])).ok().unwrap();
        let speak = |mic: &mut Producer<f32>| {
            for _ in 0..3_000 {
                for sample in [0.2, -0.4, 0.6] {
                    mic.push(sample).unwrap();
                }
            }
        };
        speak(&mut mic);
        rt.process(256);
        speak(&mut mic);
        let (left, right) = pan_gains(0.0);
        for frame in rt.process(256) {
            assert!((frame[0] - 0.6 * left).abs() < 1e-4 && (frame[1] - 0.6 * right).abs() < 1e-4, "the vocal hears input 3 on both sides, got {frame:?}");
        }
        project.apply(Command::SetTrackMuted { track: keys, muted: false }).unwrap();
        project.apply(Command::SetTrackMuted { track: vocal, muted: true }).unwrap();
        remote.outbox.push(Msg::Project(Arc::new(project))).ok().unwrap();
        speak(&mut mic);
        for frame in rt.process(256) {
            assert!((frame[0] - 0.2 * left).abs() < 1e-4 && (frame[1] + 0.4 * right).abs() < 1e-4, "the keys hear inputs 1+2 as a pair, got {frame:?}");
        }
    }

    #[derive(Default)]
    struct Trebles {
        ran: Arc<std::sync::Mutex<Vec<TrackId>>>,
    }

    impl Chains for Trebles {
        fn process_with(&mut self, track: TrackId, audio: &mut [[f32; 2]], _: &[[f32; 2]]) {
            if let Ok(mut ran) = self.ran.lock() {
                ran.push(track);
            }
            for frame in audio.iter_mut() {
                frame[0] *= 3.0;
                frame[1] *= 3.0;
            }
        }

        fn process_takes(&mut self, _: TrackId, audio: &mut [[f32; 2]]) {
            for frame in audio.iter_mut() {
                frame[0] *= 0.5;
                frame[1] *= 0.5;
            }
        }
    }

    fn some_fx(name: &str) -> crate::model::Fx {
        crate::model::Fx { path: std::path::PathBuf::from("/nowhere/Bus.vst3"), index: 0, name: name.into(), bypassed: false, state: Vec::new(), record: false, mix: 1.0 }
    }

    fn speak(mic: &mut Producer<f32>, samples: &[f32]) {
        for _ in 0..4_000 {
            for sample in samples {
                mic.push(*sample).unwrap();
            }
        }
    }

    #[test]
    fn a_heard_track_in_a_folder_is_heard_through_the_folders_chain() {
        let (mut rt, mut remote) = pair(RATE);
        let mut project = Project::new(RATE);
        let Ok(Outcome::Track(bus)) = project.apply(Command::AddTrack { name: "Vox Chain".into() }) else { panic!() };
        let Ok(Outcome::Track(vocal)) = project.apply(Command::AddTrack { name: "Vox 1".into() }) else { panic!() };
        project.apply(Command::SetTrackParent { track: vocal, parent: Some(bus) }).unwrap();
        project.apply(Command::AddFx { track: bus, fx: some_fx("Pro-Q") }).unwrap();
        project.apply(Command::SetTrackGain { track: bus, gain: 0.5 }).unwrap();
        let ran = Arc::new(std::sync::Mutex::new(Vec::new()));
        remote.outbox.push(Msg::Project(Arc::new(project))).ok().unwrap();
        remote.outbox.push(Msg::Chains(Some(Box::new(Trebles { ran: ran.clone() })))).ok().unwrap();
        let (mut mic, feed) = RingBuffer::new(1 << 15);
        remote.outbox.push(Msg::Hear(Some((feed, RATE, 1)))).ok().unwrap();
        remote.outbox.push(Msg::HearOn([Some(vocal), None, None, None, None, None, None, None])).ok().unwrap();
        speak(&mut mic, &[0.8]);
        rt.process(256);
        speak(&mut mic, &[0.8]);
        let heard = rt.process(256).to_vec();
        let (left, right) = pan_gains(0.0);
        let want = 0.8 * 0.5 * 3.0 * 0.5;
        for frame in heard {
            assert!((frame[0] - want * left).abs() < 1e-4 && (frame[1] - want * right).abs() < 1e-4, "got {frame:?}, wanted {want}");
        }
        assert_eq!(*ran.lock().unwrap(), vec![bus, bus], "the folder's chain runs once a block");
    }

    #[test]
    fn a_send_from_a_folder_is_fed_while_hearing_the_input() {
        let (mut rt, mut remote) = pair(RATE);
        let mut project = Project::new(RATE);
        let Ok(Outcome::Track(bus)) = project.apply(Command::AddTrack { name: "Vox Chain B".into() }) else { panic!() };
        let Ok(Outcome::Track(reverb)) = project.apply(Command::AddTrack { name: "Vox Reverb".into() }) else { panic!() };
        let Ok(Outcome::Track(vocal)) = project.apply(Command::AddTrack { name: "Vox 1".into() }) else { panic!() };
        project.apply(Command::SetTrackParent { track: vocal, parent: Some(bus) }).unwrap();
        project.apply(Command::AddSend { from: bus, to: reverb }).unwrap();
        project.apply(Command::SetSendGain { from: bus, to: reverb, gain: 0.5 }).unwrap();
        remote.outbox.push(Msg::Project(Arc::new(project.clone()))).ok().unwrap();
        remote.outbox.push(Msg::Chains(Some(Box::new(Trebles::default())))).ok().unwrap();
        let (mut mic, feed) = RingBuffer::new(1 << 15);
        remote.outbox.push(Msg::Hear(Some((feed, RATE, 1)))).ok().unwrap();
        remote.outbox.push(Msg::HearOn([Some(vocal), None, None, None, None, None, None, None])).ok().unwrap();
        speak(&mut mic, &[0.8]);
        rt.process(256);
        speak(&mut mic, &[0.8]);
        let (left, _) = pan_gains(0.0);
        let dry = 0.8 * 0.5;
        for frame in rt.process(256) {
            assert!((frame[0] - (dry + dry * 0.5) * left).abs() < 1e-4, "the send is not fed, got {frame:?}");
        }
        project.apply(Command::SetSendPreFader { from: bus, to: reverb, pre_fader: true }).unwrap();
        project.apply(Command::SetTrackMuted { track: bus, muted: true }).unwrap();
        remote.outbox.push(Msg::Project(Arc::new(project))).ok().unwrap();
        speak(&mut mic, &[0.8]);
        for frame in rt.process(256) {
            assert!((frame[0] - dry * 0.5 * left).abs() < 1e-4, "a pre-fader send is heard past the mute, got {frame:?}");
        }
    }

    #[test]
    fn two_heard_tracks_under_one_folder_run_its_chain_once() {
        let (mut rt, mut remote) = pair(RATE);
        let mut project = Project::new(RATE);
        let Ok(Outcome::Track(bus)) = project.apply(Command::AddTrack { name: "Vox Chain".into() }) else { panic!() };
        let Ok(Outcome::Track(one)) = project.apply(Command::AddTrack { name: "Vox 1".into() }) else { panic!() };
        let Ok(Outcome::Track(two)) = project.apply(Command::AddTrack { name: "Vox 2".into() }) else { panic!() };
        for track in [one, two] {
            project.apply(Command::SetTrackParent { track, parent: Some(bus) }).unwrap();
        }
        project.apply(Command::SetTrackInput { track: one, input: crate::model::InputChannels::Mono(0) }).unwrap();
        project.apply(Command::SetTrackInput { track: two, input: crate::model::InputChannels::Mono(1) }).unwrap();
        project.apply(Command::AddFx { track: bus, fx: some_fx("Pro-C 2") }).unwrap();
        let ran = Arc::new(std::sync::Mutex::new(Vec::new()));
        remote.outbox.push(Msg::Project(Arc::new(project))).ok().unwrap();
        remote.outbox.push(Msg::Chains(Some(Box::new(Trebles { ran: ran.clone() })))).ok().unwrap();
        let (mut mic, feed) = RingBuffer::new(1 << 15);
        remote.outbox.push(Msg::Hear(Some((feed, RATE, 2)))).ok().unwrap();
        remote.outbox.push(Msg::HearOn([Some(one), Some(two), None, None, None, None, None, None])).ok().unwrap();
        speak(&mut mic, &[0.2, 0.4]);
        rt.process(256);
        ran.lock().unwrap().clear();
        speak(&mut mic, &[0.2, 0.4]);
        let (left, _) = pan_gains(0.0);
        let want = (0.2 + 0.4) * 0.5 * 3.0;
        for frame in rt.process(256) {
            assert!((frame[0] - want * left).abs() < 1e-4, "got {frame:?}, wanted {want}");
        }
        assert_eq!(*ran.lock().unwrap(), vec![bus], "the folder's chain ran more than once");
    }

    #[test]
    fn hearing_the_short_way_skips_the_folders_chain() {
        let (mut rt, mut remote) = pair(RATE);
        let mut project = Project::new(RATE);
        let Ok(Outcome::Track(bus)) = project.apply(Command::AddTrack { name: "Vox Chain".into() }) else { panic!() };
        let Ok(Outcome::Track(vocal)) = project.apply(Command::AddTrack { name: "Vox 1".into() }) else { panic!() };
        project.apply(Command::SetTrackParent { track: vocal, parent: Some(bus) }).unwrap();
        project.apply(Command::AddFx { track: bus, fx: some_fx("Pro-Q") }).unwrap();
        project.apply(Command::SetTrackGain { track: bus, gain: 0.5 }).unwrap();
        let ran = Arc::new(std::sync::Mutex::new(Vec::new()));
        remote.outbox.push(Msg::Project(Arc::new(project))).ok().unwrap();
        remote.outbox.push(Msg::Chains(Some(Box::new(Trebles { ran: ran.clone() })))).ok().unwrap();
        let (mut mic, feed) = RingBuffer::new(1 << 15);
        remote.outbox.push(Msg::Hear(Some((feed, RATE, 1)))).ok().unwrap();
        remote.outbox.push(Msg::HearOn([Some(vocal), None, None, None, None, None, None, None])).ok().unwrap();
        remote.outbox.push(Msg::HearThroughChain(false)).ok().unwrap();
        speak(&mut mic, &[0.8]);
        rt.process(256);
        speak(&mut mic, &[0.8]);
        let (left, _) = pan_gains(0.0);
        let want = 0.8 * 0.5;
        for frame in rt.process(256) {
            assert!((frame[0] - want * left).abs() < 1e-4, "got {frame:?}, wanted {want}");
        }
        assert!(ran.lock().unwrap().is_empty(), "the folder's chain ran on the short path");
    }

    #[test]
    fn the_input_catches_up_rather_than_falling_further_behind() {
        let (mut rt, mut remote) = pair(RATE);
        let mut project = Project::new(RATE);
        let Ok(Outcome::Track(vocal)) = project.apply(Command::AddTrack { name: "Vocal".into() }) else { panic!() };
        remote.outbox.push(Msg::Project(Arc::new(project))).ok().unwrap();
        let (mut mic, feed) = RingBuffer::new(1 << 14);
        remote.outbox.push(Msg::Hear(Some((feed, RATE, 1)))).ok().unwrap();
        remote.outbox.push(Msg::HearOn([Some(vocal), None, None, None, None, None, None, None])).ok().unwrap();
        for _ in 0..10_000 {
            mic.push(0.5).unwrap();
        }
        rt.process(256);
        rt.process(256);
        let left = rt.ear.feed.as_ref().unwrap().slots();
        assert!(left <= 4 * 260, "{left} samples still waiting");
    }

    #[test]
    fn keyboard_notes_play_on_the_chosen_track_only() {
        let (mut rt, mut remote) = pair(RATE);
        let mut project = Project::new(RATE);
        let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: "Keys".into() }) else { panic!() };
        remote.outbox.push(Msg::Project(Arc::new(project))).ok().unwrap();
        remote.keys.lock().unwrap().push(KeyEvent { key: 60, velocity: 1.0 }).unwrap();
        assert!(rt.process(2_000).iter().all(|f| *f == [0.0, 0.0]), "no track chosen yet");
        remote.outbox.push(Msg::KeysGoTo(Some(track))).ok().unwrap();
        remote.keys.lock().unwrap().push(KeyEvent { key: 60, velocity: 1.0 }).unwrap();
        assert!(rt.process(2_000).iter().any(|f| f[0].abs() > 0.01));
        remote.keys.lock().unwrap().push(KeyEvent { key: 60, velocity: 0.0 }).unwrap();
        for _ in 0..10 {
            rt.process(4_000);
        }
        assert!(rt.live.is_empty());
    }

    #[test]
    fn the_metronome_clicks_on_each_beat_and_louder_on_the_first() {
        let (mut rt, mut remote) = pair(RATE);
        remote.outbox.push(Msg::Play).ok().unwrap();
        assert!(rt.process(1_000).iter().all(|f| *f == [0.0, 0.0]), "quiet until it is on");
        remote.outbox.push(Msg::Stop).ok().unwrap();
        rt.process(1_000);
        remote.outbox.push(Msg::Seek(0)).ok().unwrap();
        remote.outbox.push(Msg::Metronome(true)).ok().unwrap();
        remote.outbox.push(Msg::Play).ok().unwrap();
        let beat = beat_frames(120.0, RATE) as usize;
        let mut heard = Vec::new();
        while heard.len() < beat * 5 {
            heard.extend_from_slice(rt.process(1_000));
        }
        let loudest = |from: usize| heard[from..from + beat / 4].iter().map(|f| f[0].abs()).fold(0.0, f32::max);
        let between = heard[beat / 2..beat - 10].iter().map(|f| f[0].abs()).fold(0.0, f32::max);
        assert!(loudest(beat) > 0.1 && between == 0.0);
        assert!(loudest(4 * beat) > loudest(beat) * 1.2, "the bar starts louder");
    }

    #[test]
    fn a_count_in_clicks_first_and_holds_the_song_back() {
        let (mut rt, mut remote) = pair(RATE);
        let bar = bar_frames(120.0, RATE);
        remote.outbox.push(Msg::Seek(5_000)).ok().unwrap();
        remote.outbox.push(Msg::CountIn(bar)).ok().unwrap();
        remote.outbox.push(Msg::Play).ok().unwrap();
        let first = rt.process(1_000).to_vec();
        assert!(first.iter().any(|f| f[0].abs() > 0.1), "the count in is heard without the metronome");
        rt.heard(Instant::now());
        assert_eq!(remote.shared.heard_pos.load(Ordering::Relaxed) as i64, 5_000 - (bar as i64 - 1_000));
        assert_eq!(remote.shared.pos.load(Ordering::Relaxed), 5_000);
        let mut left = bar - 1_000 + 2_000;
        while left > 0 {
            let part = left.min(1_000);
            rt.process(part as usize);
            left -= part;
        }
        assert_eq!(remote.shared.pos.load(Ordering::Relaxed), 7_000);
        remote.outbox.push(Msg::CountIn(bar)).ok().unwrap();
        remote.outbox.push(Msg::Stop).ok().unwrap();
        rt.process(100);
        assert_eq!(rt.count_in, 0);
    }

    #[test]
    fn keys_are_taped_with_the_song_position_only_while_taping_and_playing() {
        let (mut rt, mut remote) = pair(RATE);
        let mut project = Project::new(RATE);
        let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: "Keys".into() }) else { panic!() };
        remote.outbox.push(Msg::Project(Arc::new(project))).ok().unwrap();
        remote.outbox.push(Msg::KeysGoTo(Some(track))).ok().unwrap();
        remote.outbox.push(Msg::Tape(true)).ok().unwrap();
        remote.keys.lock().unwrap().push(KeyEvent { key: 60, velocity: 1.0 }).unwrap();
        rt.process(480);
        assert!(remote.tape.pop().is_err(), "nothing is taped while stopped");
        remote.outbox.push(Msg::Play).ok().unwrap();
        rt.process(1_000);
        remote.keys.lock().unwrap().push(KeyEvent { key: 62, velocity: 0.5 }).unwrap();
        rt.process(480);
        remote.outbox.push(Msg::NoteOff { track, key: 62 }).ok().unwrap();
        rt.process(480);
        let first = remote.tape.pop().unwrap();
        assert_eq!((first.track, first.key, first.velocity, first.at), (track, 62, 0.5, 1_000));
        let second = remote.tape.pop().unwrap();
        assert_eq!((second.key, second.velocity, second.at), (62, 0.0, 1_480));
        remote.outbox.push(Msg::Tape(false)).ok().unwrap();
        remote.keys.lock().unwrap().push(KeyEvent { key: 64, velocity: 1.0 }).unwrap();
        rt.process(480);
        assert!(remote.tape.pop().is_err());
    }

    #[test]
    fn the_silent_engine_keeps_time() {
        let mut engine = Engine::start(Output::Silent);
        assert_eq!(engine.rate(), SILENT_RATE);
        engine.set_project(&steady(SILENT_RATE as usize * 10));
        engine.play();
        thread::sleep(Duration::from_millis(300));
        let pos = engine.position();
        assert!(engine.is_playing());
        assert!((9_600..=19_200).contains(&pos), "about 0.3 s in, got {pos}");
        engine.stop();
        thread::sleep(Duration::from_millis(60));
        let rest = engine.position();
        thread::sleep(Duration::from_millis(60));
        assert_eq!(engine.position(), rest);
    }

    struct Heavy {
        lags: Vec<(TrackId, usize)>,
        held: Vec<(TrackId, Vec<[f32; 2]>, usize)>,
        counted: Arc<AtomicU64>,
        slow: bool,
    }

    impl Heavy {
        fn with(lags: &[(TrackId, usize)], counted: Arc<AtomicU64>) -> Self {
            Self { lags: lags.to_vec(), held: Vec::new(), counted, slow: false }
        }

        fn slowly(mut self) -> Self {
            self.slow = true;
            self
        }
    }

    impl Chains for Heavy {
        fn process_with(&mut self, track: TrackId, audio: &mut [[f32; 2]], _: &[[f32; 2]]) {
            self.counted.fetch_add(audio.len() as u64, Ordering::Relaxed);
            if self.slow {
                thread::sleep(Duration::from_millis(1));
            }
            let by = self.latency(track);
            if by == 0 {
                return;
            }
            if !self.held.iter().any(|(id, _, _)| *id == track) {
                self.held.push((track, vec![[0.0; 2]; by], 0));
            }
            let (_, line, at) = self.held.iter_mut().find(|(id, _, _)| *id == track).expect("the line is there");
            for frame in audio.iter_mut() {
                std::mem::swap(&mut line[*at], frame);
                *at = (*at + 1) % line.len();
            }
        }

        fn latency(&self, track: TrackId) -> usize {
            self.lags.iter().find(|(id, _)| *id == track).map(|(_, by)| *by).unwrap_or(0)
        }
    }

    fn a_song_of_three_heavy_buses() -> (Project, Vec<(TrackId, usize)>) {
        let mut p = Project::new(RATE);
        let mut lags = Vec::new();
        let source = Arc::new(Source::from_frames("one", vec![[1.0, 1.0]; 300_000]));
        let Ok(Outcome::Track(verb)) = p.apply(Command::AddTrack { name: "Verb".into() }) else { panic!("no track") };
        p.apply(Command::AddFx { track: verb, fx: some_fx("verb") }).unwrap();
        lags.push((verb, 4_096));
        for which in 0..3 {
            let Ok(Outcome::Track(bus)) = p.apply(Command::AddTrack { name: format!("Bus {which}") }) else { panic!("no track") };
            let Ok(Outcome::Track(vox)) = p.apply(Command::AddTrack { name: format!("Vox {which}") }) else { panic!("no track") };
            p.apply(Command::AddClip { track: vox, source: source.clone(), start: 0 }).unwrap();
            p.apply(Command::SetTrackParent { track: vox, parent: Some(bus) }).unwrap();
            p.apply(Command::AddSend { from: bus, to: verb }).unwrap();
            p.apply(Command::AddFx { track: bus, fx: some_fx("bus") }).unwrap();
            lags.push((bus, 16_383 + 26_623 + 2_048));
        }
        (p, lags)
    }

    fn rolling(rt: &mut Rt, remote: &Remote, frames: usize, want: Frames) {
        for _ in 0..2_000 {
            rt.process(frames);
            if remote.shared.pos.load(Ordering::Relaxed) > want {
                return;
            }
            thread::sleep(Duration::from_millis(1));
        }
        panic!("the transport never got past {want}");
    }

    #[test]
    fn a_seek_puts_no_more_than_a_block_through_the_plugins() {
        let counted = Arc::new(AtomicU64::new(0));
        let (p, lags) = a_song_of_three_heavy_buses();
        let (mut rt, mut remote) = pair(RATE);
        remote.outbox.push(Msg::Project(Arc::new(p))).ok().unwrap();
        remote.outbox.push(Msg::Chains(Some(Box::new(Heavy::with(&lags, counted.clone()))))).ok().unwrap();
        remote.outbox.push(Msg::Endless(true)).ok().unwrap();
        remote.outbox.push(Msg::Play).ok().unwrap();
        rolling(&mut rt, &remote, 512, 4 * 512);
        let settled = counted.load(Ordering::Relaxed);
        rt.process(512);
        let a_block = counted.load(Ordering::Relaxed) - settled;
        remote.outbox.push(Msg::Seek(100_000)).ok().unwrap();
        let mut most = 0;
        for _ in 0..2_000 {
            let before = counted.load(Ordering::Relaxed);
            rt.process(512);
            most = most.max(counted.load(Ordering::Relaxed) - before);
            if remote.shared.pos.load(Ordering::Relaxed) > 100_000 + 512 {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert!(
            most <= a_block,
            "a seek put {most} frames through the plugins in one callback, a settled block puts {a_block}"
        );
    }

    struct Counted;

    thread_local! {
        static WATCHING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

    unsafe impl std::alloc::GlobalAlloc for Counted {
        unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
            if WATCHING.try_with(|watching| watching.get()).unwrap_or(false) {
                ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            }
            unsafe { std::alloc::GlobalAlloc::alloc(&std::alloc::System, layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
            unsafe { std::alloc::GlobalAlloc::dealloc(&std::alloc::System, ptr, layout) }
        }
    }

    #[global_allocator]
    static COUNTED: Counted = Counted;

    fn watched(run: impl FnOnce()) -> u64 {
        let before = ALLOCATIONS.load(Ordering::Relaxed);
        WATCHING.with(|watching| watching.set(true));
        run();
        WATCHING.with(|watching| watching.set(false));
        ALLOCATIONS.load(Ordering::Relaxed) - before
    }

    fn heavy_rig(counted: &Arc<AtomicU64>) -> (Arc<Project>, Vec<(TrackId, usize)>, Rt, Remote) {
        let (p, lags) = a_song_of_three_heavy_buses();
        let p = Arc::new(p);
        let (rt, mut remote) = pair(RATE);
        remote.outbox.push(Msg::Project(p.clone())).ok().unwrap();
        remote.outbox.push(Msg::Chains(Some(Box::new(Heavy::with(&lags, counted.clone()))))).ok().unwrap();
        (p, lags, rt, remote)
    }

    fn rolled_ahead(rt: &mut Rt, frames: usize) {
        let mut went_out = false;
        for _ in 0..4_000 {
            rt.process(frames);
            went_out |= rt.roll == Roll::Out;
            if went_out && rt.roll == Roll::No {
                return;
            }
            thread::sleep(Duration::from_millis(1));
        }
        panic!("the pre-roll never came back");
    }

    fn untouched(p: &Project, lags: &[(TrackId, usize)], at: Frames, round: Round, blocks: usize) -> Vec<[f32; 2]> {
        let mut racks = Heavy::with(lags, Arc::new(AtomicU64::new(0)));
        let mut scratch = crate::render::Mixdown::default();
        let mut all = vec![[0.0; 2]; blocks * 512];
        scratch.get_ready(p, at, round, None, 512, Some(&mut racks), None);
        for block in 0..blocks {
            let from = block * 512;
            mix_tracks_metered(p, at + from as Frames, &mut all[from..from + 512], None, None, &mut scratch, Some(&mut racks));
        }
        all
    }

    #[test]
    fn play_from_stopped_is_rolled_ahead_while_the_transport_waits() {
        let counted = Arc::new(AtomicU64::new(0));
        let (p, lags, mut rt, mut remote) = heavy_rig(&counted);
        remote.outbox.push(Msg::Endless(true)).ok().unwrap();
        remote.outbox.push(Msg::Seek(50_000)).ok().unwrap();
        rolled_ahead(&mut rt, 512);
        assert_eq!(remote.shared.pos.load(Ordering::Relaxed), 50_000, "the playhead moved while it was parked");
        remote.outbox.push(Msg::Play).ok().unwrap();
        let settled = counted.load(Ordering::Relaxed);
        let first = rt.process(512).to_vec();
        assert!(counted.load(Ordering::Relaxed) - settled <= 2_048, "pressing play put the pre-roll through the plugins again");
        assert_eq!(remote.shared.pos.load(Ordering::Relaxed), 50_512, "play did not start at once");
        assert!(first[..240].windows(2).all(|w| w[1][0].abs() >= w[0][0].abs()), "the fade in is not clean");
        let second = rt.process(512).to_vec();
        let want = untouched(&p, &lags, 50_000, None, 2);
        assert_eq!(second, want[512..], "the first full block of play is not what an untouched render gives");
        assert!(want.iter().any(|frame| frame[0].abs() > 1.0), "the song is silent");
    }

    #[test]
    fn a_seek_while_playing_lands_on_the_same_audio_an_untouched_render_gives() {
        let counted = Arc::new(AtomicU64::new(0));
        let (p, lags, mut rt, mut remote) = heavy_rig(&counted);
        remote.outbox.push(Msg::Endless(true)).ok().unwrap();
        remote.outbox.push(Msg::Play).ok().unwrap();
        rolling(&mut rt, &remote, 512, 4 * 512);
        remote.outbox.push(Msg::Seek(150_000)).ok().unwrap();
        for _ in 0..4_000 {
            rt.process(512);
            if remote.shared.pos.load(Ordering::Relaxed) >= 150_512 {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(remote.shared.pos.load(Ordering::Relaxed), 150_512, "the seek never took");
        let second = rt.process(512).to_vec();
        let want = untouched(&p, &lags, 150_000, None, 2);
        assert_eq!(second, want[512..], "the mix after a seek is not what an untouched render gives");
    }

    #[test]
    fn a_loop_wrap_never_pauses_or_asks_for_a_pre_roll() {
        let counted = Arc::new(AtomicU64::new(0));
        let (p, lags, mut rt, mut remote) = heavy_rig(&counted);
        remote.outbox.push(Msg::Loop(Some((100_000, 140_000)))).ok().unwrap();
        remote.outbox.push(Msg::Seek(100_000)).ok().unwrap();
        rolled_ahead(&mut rt, 512);
        remote.outbox.push(Msg::Play).ok().unwrap();
        let round = Some((100_000, 140_000));
        let want = untouched(&p, &lags, 100_000, round, 2);
        rt.process(512);
        assert_eq!(rt.process(512).to_vec(), want[512..], "the first pass of the loop is not what an untouched render gives");
        let mut wrapped = false;
        let mut most = 0;
        for _ in 0..200 {
            let was = remote.shared.pos.load(Ordering::Relaxed);
            let settled = counted.load(Ordering::Relaxed);
            let heard = rt.process(512).to_vec();
            most = most.max(counted.load(Ordering::Relaxed) - settled);
            let now = remote.shared.pos.load(Ordering::Relaxed);
            assert_ne!(now, was, "the transport stopped at {was}");
            assert_eq!(rt.roll, Roll::No, "a loop wrap asked for a pre-roll at {was}");
            assert!(heard.iter().any(|frame| frame[0].abs() > 1.0), "the loop went quiet at {was}");
            if now < was {
                wrapped = true;
                break;
            }
        }
        assert!(wrapped, "the loop never came round");
        assert!(most <= 2_048, "a loop wrap put {most} frames through the plugins in one callback");
        let round = rt.process(512).to_vec();
        assert_eq!(round, want[..512], "the second time round the loop did not come out the same");
    }

    #[test]
    fn nothing_more_is_allocated_in_the_callback_when_the_transport_jumps() {
        let counted = Arc::new(AtomicU64::new(0));
        let (_, _, mut rt, mut remote) = heavy_rig(&counted);
        remote.outbox.push(Msg::Endless(true)).ok().unwrap();
        remote.outbox.push(Msg::Play).ok().unwrap();
        rolling(&mut rt, &remote, 512, 8 * 512);
        let steady = watched(|| {
            for _ in 0..8 {
                rt.process(512);
            }
        });
        remote.outbox.push(Msg::Seek(150_000)).ok().unwrap();
        let jumping = watched(|| {
            for _ in 0..4_000 {
                rt.process(512);
                if rt.roll == Roll::No && remote.shared.pos.load(Ordering::Relaxed) > 150_512 {
                    return;
                }
                thread::sleep(Duration::from_millis(1));
            }
        });
        assert_eq!((steady, jumping), (0, 0), "the callback allocated {steady} times steady and {jumping} times across a jump");
    }

    #[test]
    fn a_pre_roll_gives_the_racks_back_the_moment_the_app_asks() {
        let counted = Arc::new(AtomicU64::new(0));
        let (p, lags) = a_song_of_three_heavy_buses();
        let (mut rt, mut remote) = pair(RATE);
        remote.outbox.push(Msg::Project(Arc::new(p))).ok().unwrap();
        remote.outbox.push(Msg::Chains(Some(Box::new(Heavy::with(&lags, counted.clone()).slowly())))).ok().unwrap();
        remote.outbox.push(Msg::Seek(50_000)).ok().unwrap();
        let mut went_out = false;
        for _ in 0..4_000 {
            rt.process(512);
            went_out |= rt.roll == Roll::Out;
            if went_out {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert!(went_out, "the pre-roll never went out");
        remote.outbox.push(Msg::Chains(None)).ok().unwrap();
        let mut back = None;
        for _ in 0..4_000 {
            rt.process(512);
            back = remote.handed_back.pop().ok();
            if back.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert!(back.is_some(), "the racks never came back");
        assert!(counted.load(Ordering::Relaxed) < 198_648 / 2, "the pre-roll ran on after the app asked for the racks");
    }
}

#[cfg(test)]
mod monitor_delay_tests {
    use super::*;
    use crate::model::{Command, Outcome};

    const RATE: u32 = 48_000;
    const RING: usize = 1 << 14;

    struct Wire {
        packet: usize,
        block: usize,
        input_runs_fast_by: f64,
        late_every: usize,
        backlog: usize,
    }

    struct Heard {
        delays: Vec<(f64, f64)>,
        skips: usize,
        repeats: usize,
    }

    impl Heard {
        fn worst_after(&self, settled: f64) -> f64 {
            self.delays.iter().filter(|(at, _)| *at >= settled).map(|(_, delay)| *delay).fold(0.0, f64::max)
        }

        fn last(&self) -> f64 {
            self.delays.last().map(|(_, delay)| *delay).unwrap_or(f64::INFINITY)
        }
    }

    fn singing(rt: &mut Rt, remote: &mut Remote, folder: Option<Folder>) -> (TrackId, Producer<f32>) {
        let mut project = Project::new(RATE);
        let Ok(Outcome::Track(vocal)) = project.apply(Command::AddTrack { name: "Vox".into() }) else { panic!() };
        project.apply(Command::SetTrackPan { track: vocal, pan: 0.0 }).unwrap();
        let chains: Option<Box<dyn Chains>> = match folder {
            Some(folder) => {
                let Ok(Outcome::Track(bus)) = project.apply(Command::AddTrack { name: "Vox Bus".into() }) else { panic!() };
                project.apply(Command::SetTrackPan { track: bus, pan: 0.0 }).unwrap();
                project.apply(Command::SetTrackParent { track: vocal, parent: Some(bus) }).unwrap();
                let fx = crate::model::Fx { mix: 1.0, path: "late.vst3".into(), index: 0, name: "late".into(), bypassed: false, state: Vec::new(), record: false };
                project.apply(Command::AddFx { track: bus, fx }).unwrap();
                Some(Box::new(folder))
            }
            None => None,
        };
        remote.outbox.push(Msg::Project(Arc::new(project))).ok().unwrap();
        remote.outbox.push(Msg::Chains(chains)).ok().unwrap();
        let (mic, feed) = RingBuffer::new(RING);
        remote.outbox.push(Msg::Hear(Some((feed, RATE, 1)))).ok().unwrap();
        remote.outbox.push(Msg::HearOn([Some(vocal), None, None, None, None, None, None, None])).ok().unwrap();
        rt.process(16);
        (vocal, mic)
    }

    fn sing_over(wire: &Wire, seconds: f64) -> Heard {
        sing_through(wire, seconds, None).0
    }

    fn sing_through(wire: &Wire, seconds: f64, folder: Option<Folder>) -> (Heard, Frames) {
        let (mut rt, mut remote) = pair(RATE);
        let (_, mut mic) = singing(&mut rt, &mut remote, folder);
        let input_rate = RATE as f64 * (1.0 + wire.input_runs_fast_by);
        let mut sung: u64 = 0;
        let sing = |mic: &mut Producer<f32>, frames: usize, sung: &mut u64| {
            for _ in 0..frames {
                if mic.slots() > 0 {
                    let _ = mic.push((*sung + 1) as f32);
                }
                *sung += 1;
            }
        };
        sing(&mut mic, wire.backlog, &mut sung);
        let began = wire.backlog as f64 / input_rate;
        let mut next_in = began + wire.packet as f64 / input_rate;
        let mut next_out = began + wire.block as f64 / RATE as f64;
        let mut calls = 0usize;
        let mut owed = 0usize;
        let mut last_heard: Option<u64> = None;
        let mut was_heard = false;
        let mut heard = Heard { delays: Vec::new(), skips: 0, repeats: 0 };
        while next_out < seconds {
            if next_in <= next_out {
                sing(&mut mic, wire.packet, &mut sung);
                next_in += wire.packet as f64 / input_rate;
                continue;
            }
            calls += 1;
            owed += wire.block;
            let now = next_out;
            next_out += wire.block as f64 / RATE as f64;
            if wire.late_every > 0 && calls.is_multiple_of(wire.late_every) {
                continue;
            }
            let frames = std::mem::take(&mut owed);
            let out = rt.process(frames).to_vec();
            let played_from = now - (frames - wire.block) as f64 / RATE as f64;
            for (at, frame) in out.iter().enumerate() {
                let index = frame[0].round() as u64;
                let clean = std::mem::replace(&mut was_heard, index > 0);
                if index == 0 || !clean {
                    continue;
                }
                let index = index - 1;
                if at == 0 || heard.delays.last().is_none_or(|(when, _)| *when < played_from) {
                    let captured = index as f64 / input_rate;
                    let plays = played_from + at as f64 / RATE as f64;
                    heard.delays.push((plays, (plays - captured) * 1000.0));
                }
                match last_heard {
                    Some(was) if index > was + 2 => heard.skips += 1,
                    Some(was) if index < was => heard.repeats += 1,
                    _ => {}
                }
                last_heard = Some(index);
            }
        }
        let lag = remote.shared.monitor_lag.load(Ordering::Relaxed);
        (heard, lag)
    }

    fn report(name: &str, wire: &Wire, heard: &Heard) {
        eprintln!(
            "{name}: packet {} block {} drift {:+.0} ppm late every {} backlog {}: worst after 1 s {:.1} ms, last {:.1} ms, skips {}, repeats {}",
            wire.packet,
            wire.block,
            wire.input_runs_fast_by * 1e6,
            wire.late_every,
            wire.backlog,
            heard.worst_after(1.0),
            heard.last(),
            heard.skips,
            heard.repeats
        );
    }

    fn ms(frames: usize) -> f64 {
        frames as f64 * 1000.0 / RATE as f64
    }

    #[test]
    fn an_input_that_runs_fast_never_piles_up_behind_the_output() {
        let wire = Wire { packet: 480, block: 480, input_runs_fast_by: 300e-6, late_every: 0, backlog: 0 };
        let heard = sing_over(&wire, 180.0);
        report("fast input", &wire, &heard);
        assert_eq!(heard.skips + heard.repeats, 0, "drift was corrected by jumping instead of by pace");
        let bound = ms(wire.packet + wire.block) + 5.0;
        assert!(heard.worst_after(1.0) <= bound, "the monitor fell {:.1} ms behind, more than {bound:.1} ms", heard.worst_after(1.0));
        assert!(heard.last() <= bound, "after three minutes the monitor was {:.1} ms behind", heard.last());
    }

    #[test]
    fn an_input_that_runs_slow_stays_close_behind_the_output() {
        let wire = Wire { packet: 480, block: 480, input_runs_fast_by: -300e-6, late_every: 0, backlog: 0 };
        let heard = sing_over(&wire, 180.0);
        report("slow input", &wire, &heard);
        let wraps = (180.0 * 300e-6 * RATE as f64 / wire.packet as f64).ceil() as usize;
        assert!(heard.skips <= wraps, "{} gaps in three minutes, the clocks only slipped a packet {wraps} times", heard.skips);
        let bound = ms(wire.packet + wire.block) + 5.0;
        assert!(heard.worst_after(1.0) <= bound, "the monitor fell {:.1} ms behind, more than {bound:.1} ms", heard.worst_after(1.0));
    }

    #[test]
    fn late_output_callbacks_do_not_leave_the_monitor_behind() {
        let wire = Wire { packet: 480, block: 480, input_runs_fast_by: 100e-6, late_every: 37, backlog: 0 };
        let heard = sing_over(&wire, 60.0);
        report("late callbacks", &wire, &heard);
        let bound = ms(wire.packet + 2 * wire.block) + 5.0;
        assert!(heard.worst_after(1.0) <= bound, "the monitor fell {:.1} ms behind, more than {bound:.1} ms", heard.worst_after(1.0));
        assert!(heard.last() <= ms(wire.packet + wire.block) + 5.0, "the monitor never caught up: {:.1} ms", heard.last());
    }

    #[test]
    fn input_that_waited_before_monitoring_began_is_never_heard() {
        let wire = Wire { packet: 480, block: 480, input_runs_fast_by: 0.0, late_every: 0, backlog: RING - 1 };
        let heard = sing_over(&wire, 5.0);
        report("backlog", &wire, &heard);
        let first = heard.delays.first().map(|(_, delay)| *delay).unwrap_or(f64::INFINITY);
        assert!(first <= ms(wire.packet + wire.block) + 5.0, "the first thing heard was {first:.1} ms old");
    }

    #[test]
    fn small_output_blocks_under_big_input_packets_play_every_sample() {
        let wire = Wire { packet: 480, block: 128, input_runs_fast_by: 50e-6, late_every: 0, backlog: 0 };
        let heard = sing_over(&wire, 30.0);
        report("small blocks", &wire, &heard);
        assert!(heard.worst_after(1.0) <= ms(wire.packet + wire.block) + 5.0, "the monitor fell {:.1} ms behind", heard.worst_after(1.0));
        assert_eq!(heard.skips + heard.repeats, 0, "{} jumps in thirty seconds of singing", heard.skips + heard.repeats);
    }

    struct Folder {
        off: bool,
        latency: usize,
        line: std::collections::VecDeque<[f32; 2]>,
    }

    impl Folder {
        fn of(latency: usize, off: bool) -> Self {
            Self { off, latency, line: std::iter::repeat_n([0.0; 2], latency).collect() }
        }

        fn delay(&mut self, audio: &mut [[f32; 2]]) {
            for frame in audio.iter_mut() {
                self.line.push_back(*frame);
                *frame = self.line.pop_front().unwrap_or([0.0; 2]);
            }
        }
    }

    impl Chains for Folder {
        fn process_with(&mut self, _: TrackId, audio: &mut [[f32; 2]], _: &[[f32; 2]]) {
            self.delay(audio);
        }

        fn hear_with(&mut self, _: TrackId, audio: &mut [[f32; 2]], _: &[[f32; 2]]) {
            if !self.off {
                self.delay(audio);
            }
        }

        fn latency(&self, _: TrackId) -> usize {
            self.latency
        }

        fn heard_latency(&self, _: TrackId) -> usize {
            match self.off {
                true => 0,
                false => self.latency,
            }
        }
    }

    #[test]
    fn a_folder_chain_that_is_off_adds_nothing_to_the_monitor() {
        const LATE: usize = 4_800;
        let wire = Wire { packet: 480, block: 480, input_runs_fast_by: 0.0, late_every: 0, backlog: 0 };
        let straight = sing_over(&wire, 10.0).last();
        let (off, lag_off) = sing_through(&wire, 10.0, Some(Folder::of(LATE, true)));
        report("folder chain off", &wire, &off);
        assert!((off.last() - straight).abs() < 0.1, "a folder chain that is off held the singer back {:.1} ms against {straight:.1} ms", off.last());
        assert!(off.worst_after(1.0) <= ms(wire.packet + wire.block) + 5.0);
        assert_eq!(lag_off, 0, "the lag shown counts a chain that is off");
        let (on, lag_on) = sing_through(&wire, 10.0, Some(Folder::of(LATE, false)));
        report("folder chain on", &wire, &on);
        assert!((on.last() - straight - ms(LATE)).abs() < 0.1, "a working folder chain should add exactly its own {:.1} ms, got {:.1}", ms(LATE), on.last() - straight);
        assert_eq!(lag_on, LATE as Frames);
    }
}
