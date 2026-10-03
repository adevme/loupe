use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::model::{ClipId, Frames, Project};
use crate::render::{mix_tracks, scale};

const MAX_BLOCK: usize = 4096;
const FADE_SECONDS: f32 = 0.005;
const QUEUE: usize = 256;
const SILENT_RATE: u32 = 48_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Device,
    Silent,
}

enum Msg {
    Project(Arc<Project>),
    Play,
    Stop,
    Seek(Frames),
    Loop(Option<(Frames, Frames)>),
    Audition(Option<ClipId>),
}

#[derive(Default)]
struct Shared {
    pos: AtomicU64,
    playing: AtomicBool,
}

struct Rt {
    project: Arc<Project>,
    inbox: Consumer<Msg>,
    retired: Producer<Arc<Project>>,
    shared: Arc<Shared>,
    pos: Frames,
    playing: bool,
    seek: Option<Frames>,
    loop_range: Option<(Frames, Frames)>,
    audition: Option<ClipId>,
    master: f32,
    fade: u32,
    fade_len: u32,
    block: Vec<[f32; 2]>,
}

struct Remote {
    outbox: Producer<Msg>,
    retired: Consumer<Arc<Project>>,
    shared: Arc<Shared>,
}

fn pair(rate: u32) -> (Rt, Remote) {
    let (outbox, inbox) = RingBuffer::new(QUEUE);
    let (retired_tx, retired_rx) = RingBuffer::new(QUEUE);
    let shared = Arc::new(Shared::default());
    let rt = Rt {
        project: Arc::new(Project::new(rate)),
        inbox,
        retired: retired_tx,
        shared: shared.clone(),
        pos: 0,
        playing: false,
        seek: None,
        loop_range: None,
        audition: None,
        master: 1.0,
        fade: 0,
        fade_len: ((FADE_SECONDS * rate as f32).round() as u32).max(1),
        block: vec![[0.0; 2]; MAX_BLOCK],
    };
    (rt, Remote { outbox, retired: retired_rx, shared })
}

impl Rt {
    fn process(&mut self, frames: usize) -> &[[f32; 2]] {
        while let Ok(msg) = self.inbox.pop() {
            match msg {
                Msg::Project(project) => {
                    let old = std::mem::replace(&mut self.project, project);
                    let _ = self.retired.push(old);
                }
                Msg::Play => self.playing = true,
                Msg::Stop => self.playing = false,
                Msg::Seek(to) => self.seek = Some(to),
                Msg::Loop(range) => self.loop_range = range,
                Msg::Audition(clip) => self.audition = clip,
            }
        }

        let out = &mut self.block[..frames];
        let mut done = 0;
        while done < frames {
            if self.fade == 0 {
                if let Some(to) = self.seek.take() {
                    self.pos = to;
                }
                if !self.playing {
                    out[done..].fill([0.0; 2]);
                    break;
                }
            }
            let rising = self.playing && self.seek.is_none();
            let mut part = if rising {
                frames - done
            } else {
                (self.fade as usize).clamp(1, frames - done)
            };
            let (stop_at, go_round_to) = match self.loop_range {
                Some((from, to)) if self.pos >= from && self.pos < to => (to, from),
                _ => (self.project.length(), 0),
            };
            if self.pos < stop_at {
                part = part.min((stop_at - self.pos) as usize);
            }
            let chunk = &mut out[done..done + part];
            mix_tracks(&self.project, self.pos, chunk, self.audition);
            scale(chunk, self.master, self.project.master);
            self.master = self.project.master;
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
        self.shared.pos.store(self.pos, Ordering::Relaxed);
        out
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

    pub fn position(&self) -> Frames {
        self.remote.shared.pos.load(Ordering::Relaxed)
    }

    pub fn is_playing(&self) -> bool {
        self.remote.shared.playing.load(Ordering::Relaxed)
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
    device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            for part in data.chunks_mut(MAX_BLOCK * channels) {
                let mixed = rt.process(part.len() / channels);
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
