use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::clock;
use crate::instrument::Note;
use crate::midi_in::{KeyEvent, KeySender};
use crate::model::{ClipId, Frames, Project, TrackId};
use crate::render::{mix_tracks_metered, scale, Chains, Mixdown};

const MAX_BLOCK: usize = 4096;
const FADE_SECONDS: f32 = 0.005;
const QUEUE: usize = 256;
const SILENT_RATE: u32 = 48_000;
pub const METERS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Device,
    Silent,
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
    KeysGoTo(Option<TrackId>),
    Metronome(bool),
    CountIn(Frames),
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            levels: std::array::from_fn(|_| AtomicU32::new(0)),
            master_level: AtomicU32::new(0),
            pos: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            heard_turn: AtomicU64::new(0),
            heard_pos: AtomicU64::new(0),
            heard_at: AtomicU64::new(0),
        }
    }
}

struct Shared {
    levels: [AtomicU32; METERS],
    master_level: AtomicU32,
    pos: AtomicU64,
    playing: AtomicBool,
    heard_turn: AtomicU64,
    heard_pos: AtomicU64,
    heard_at: AtomicU64,
}

struct Rt {
    scratch: Mixdown,
    chains: Option<Box<dyn Chains>>,
    peaks: [f32; METERS],
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
    metronome: bool,
    count_in: Frames,
    count_in_len: Frames,
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
    shared: Arc<Shared>,
}

fn pair(rate: u32) -> (Rt, Remote) {
    let (keys_in, keys_out) = RingBuffer::new(QUEUE);
    let (outbox, inbox) = RingBuffer::new(QUEUE);
    let (retired_tx, retired_rx) = RingBuffer::new(QUEUE);
    let (back_tx, back_rx) = RingBuffer::new(QUEUE);
    let shared = Arc::new(Shared::default());
    let rt = Rt {
        live: Vec::with_capacity(MOST_LIVE_NOTES),
        keys: keys_out,
        keys_go_to: None,
        metronome: false,
        count_in: 0,
        count_in_len: 0,
        scratch: Mixdown::default(),
        chains: None,
        peaks: [0.0; METERS],
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
    (rt, Remote { keys: Arc::new(std::sync::Mutex::new(keys_in)), outbox, retired: retired_rx, handed_back: back_rx, shared })
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
                Msg::Seek(to) => self.seek = Some(to),
                Msg::Loop(range) => self.loop_range = range,
                Msg::Audition(clip) => self.audition = clip,
                Msg::Endless(endless) => self.endless = endless,
                Msg::NoteOn { track, key, velocity } => {
                    if self.live.len() == MOST_LIVE_NOTES {
                        self.live.remove(0);
                    }
                    self.live.push(Live { track, note: Note { key, start: 0, len: HELD, velocity }, played: 0 });
                }
                Msg::NoteOff { track, key } => {
                    for voice in self.live.iter_mut().filter(|v| v.track == track && v.note.key == key && v.note.len == HELD) {
                        voice.note.len = voice.played.max(1);
                    }
                }
                Msg::Silence => self.live.clear(),
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

        let rate = self.project.rate;
        let beat = beat_frames(self.project.bpm, rate);
        let out = &mut self.block[..frames];
        let mut done = 0;
        while done < frames {
            if self.fade == 0 {
                if let Some(to) = self.seek.take() {
                    self.pos = to;
                }
                if !self.playing {
                    out[done..].fill([0.0; 2]);
                    for slot in self.shared.levels.iter() {
                        slot.store(0f32.to_bits(), Ordering::Relaxed);
                    }
                    self.shared.master_level.store(0f32.to_bits(), Ordering::Relaxed);
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
            let rising = self.playing && self.seek.is_none();
            let mut part = if rising {
                frames - done
            } else {
                (self.fade as usize).clamp(1, frames - done)
            };
            let (stop_at, go_round_to) = match self.loop_range {
                _ if self.endless => (Frames::MAX, 0),
                Some((from, to)) if self.pos >= from && self.pos < to => (to, from),
                _ => (self.project.length(), 0),
            };
            if self.pos < stop_at {
                part = part.min((stop_at - self.pos) as usize);
            }
            let chunk = &mut out[done..done + part];
            self.peaks = [0.0; METERS];
            mix_tracks_metered(&self.project, self.pos, chunk, self.audition, Some(&mut self.peaks), &mut self.scratch, self.chains.as_deref_mut());
            let target = if self.project.master_muted { 0.0 } else { self.project.master };
            scale(chunk, self.master, target);
            self.master = target;
            let mut top = 0.0f32;
            for frame in chunk.iter() {
                top = top.max(frame[0].abs()).max(frame[1].abs());
            }
            for (slot, peak) in self.shared.levels.iter().zip(self.peaks) {
                slot.store(peak.to_bits(), Ordering::Relaxed);
            }
            self.shared.master_level.store(top.to_bits(), Ordering::Relaxed);
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
        self.play_live(frames);
        self.shared.pos.store(self.pos, Ordering::Relaxed);
        &self.block[..frames]
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
            let gain = if track.muted { 0.0 } else { track.gain * master };
            instrument.play(&voice.note, voice.played, out, |_| gain, rate);
            voice.played += frames as Frames;
            voice.note.len == HELD || voice.played < voice.note.len + instrument.tail(rate)
        });
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
    output_error: Option<String>,
    quit: Arc<AtomicBool>,
    host: Option<thread::JoinHandle<()>>,
}

struct Ready {
    remote: Remote,
    rate: u32,
    error: Option<String>,
}

impl Engine {
    pub fn start(output: Output) -> Self {
        clock::start();
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
            output_error: ready.error,
            quit,
            host: Some(host),
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn output_error(&self) -> Option<&str> {
        self.output_error.as_deref()
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

    pub fn levels(&self) -> ([f32; METERS], f32) {
        let shared = &self.remote.shared;
        let tracks = std::array::from_fn(|i| f32::from_bits(shared.levels[i].load(Ordering::Relaxed)));
        (tracks, f32::from_bits(shared.master_level.load(Ordering::Relaxed)))
    }

    pub fn set_metronome(&mut self, on: bool) {
        self.send(Msg::Metronome(on));
    }

    pub fn count_in(&mut self, len: Frames) {
        self.send(Msg::CountIn(len));
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

    pub fn use_chains(&mut self, chains: Box<dyn Chains>) {
        self.send(Msg::Chains(Some(chains)));
    }

    pub fn tweak(&mut self, track: TrackId, slot: usize, knob: usize, value: f32) {
        self.send(Msg::Tweak { track, slot, knob, value });
    }

    pub fn tweak_clip(&mut self, clip: ClipId, slot: usize, knob: usize, value: f32) {
        self.send(Msg::TweakClip { clip, slot, knob, value });
    }

    pub fn drop_chains(&mut self) {
        self.send(Msg::Chains(None));
    }

    pub fn chains_back(&mut self) -> Option<Box<dyn Chains>> {
        self.remote.handed_back.pop().ok()
    }

    pub fn collect(&mut self) {
        while self.remote.retired.pop().is_ok() {}
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
    if output == Output::Device {
        match open_device() {
            Ok((stream, remote, rate)) => {
                let _ = ready.send(Ready { remote, rate, error: None });
                while !quit.load(Ordering::Relaxed) {
                    thread::park_timeout(Duration::from_millis(200));
                }
                drop(stream);
                return;
            }
            Err(e) => error = Some(e),
        }
    }

    let (mut rt, remote) = pair(SILENT_RATE);
    let _ = ready.send(Ready { remote, rate: SILENT_RATE, error });
    let block = SILENT_RATE as usize / 100;
    let mut next = Instant::now();
    while !quit.load(Ordering::Relaxed) {
        rt.process(block);
        next += Duration::from_millis(10);
        rt.heard(next);
        match next.checked_duration_since(Instant::now()) {
            Some(wait) => thread::park_timeout(wait),
            None => next = Instant::now(),
        }
    }
}

fn open_device() -> Result<(cpal::Stream, Remote, u32), String> {
    let device = cpal::default_host().default_output_device().ok_or("no sound output found")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let rate = config.sample_rate.0;
    let (rt, remote) = pair(rate);
    let stream = match format {
        cpal::SampleFormat::F32 => stream::<f32>(&device, &config, rt),
        cpal::SampleFormat::I16 => stream::<i16>(&device, &config, rt),
        cpal::SampleFormat::U16 => stream::<u16>(&device, &config, rt),
        cpal::SampleFormat::I32 => stream::<i32>(&device, &config, rt),
        other => return Err(format!("the sound output uses a format Loupe cannot write ({other})")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, remote, rate))
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
            let mut heard = Instant::now() + delay;
            for part in data.chunks_mut(MAX_BLOCK * channels) {
                let frames = part.len() / channels;
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
    fn levels_fall_to_nothing_when_stopped() {
        let mut engine = Engine::start(Output::Silent);
        engine.set_project(&steady(SILENT_RATE as usize * 5));
        engine.play();
        thread::sleep(Duration::from_millis(150));
        engine.stop();
        thread::sleep(Duration::from_millis(150));
        let (tracks, master) = engine.levels();
        assert_eq!(tracks[0], 0.0, "track level stayed at {}", tracks[0]);
        assert_eq!(master, 0.0, "master level stayed at {master}");
    }

    #[test]
    fn the_engine_publishes_levels_while_playing() {
        let mut engine = Engine::start(Output::Silent);
        engine.set_project(&steady(SILENT_RATE as usize * 5));
        engine.play();
        thread::sleep(Duration::from_millis(200));
        let (tracks, master) = engine.levels();
        assert!(tracks[0] > 0.0, "track level stayed at {}", tracks[0]);
        assert!(master > 0.0, "master level stayed at {master}");
    }

    #[test]
    fn a_playing_track_reports_a_level() {
        let project = steady(256);
        let mut out = vec![[0.0f32; 2]; 64];
        let mut peaks = [0.0f32; METERS];
        crate::render::mix_tracks_metered(&project, 0, &mut out, None, Some(&mut peaks), &mut crate::render::Mixdown::default(), None);
        assert!(peaks[0] > 0.0, "track 0 should report a level, got {}", peaks[0]);
    }

    fn rig(len: usize) -> (Rt, Remote) {
        let (rt, mut remote) = pair(RATE);
        remote.outbox.push(Msg::Project(Arc::new(steady(len)))).ok().unwrap();
        (rt, remote)
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
}
