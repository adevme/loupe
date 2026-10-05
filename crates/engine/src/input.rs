use std::fs::{self, File};
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::clock;
use crate::model::InputChannels;
use crate::wav;

const PRACTICE_SWELL_SECONDS: f32 = 3.0;
const PRACTICE_LOUDEST: f32 = 1.2;
const PRACTICE_RATE: u32 = 48_000;
const PRACTICE_TONE_HZ: f32 = 220.0;
pub const PRACTICE_INPUTS: u16 = 4;
pub const MOST_INPUTS: usize = 64;
const TAKE_QUEUE_FRAMES: usize = 1 << 17;
const TAKE_SETTLES_FOR: Duration = Duration::from_millis(30);
const KEEPER_RESTS_FOR: Duration = Duration::from_millis(15);
const EAR_QUEUE_FRAMES: usize = 1 << 14;
const SHAPE_STEP_MS: usize = 10;
const SHAPE_KEPT: usize = 60 * 60 * 1000 / SHAPE_STEP_MS;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputChoice {
    SystemDefault,
    Named(String),
    Practice,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Take {
    pub path: PathBuf,
    pub frames: u64,
    pub rate: u32,
    pub lost: u64,
}

struct Heard {
    peaks: [AtomicU32; MOST_INPUTS],
    taking: AtomicBool,
    began: AtomicU64,
    lost: AtomicU64,
}

impl Default for Heard {
    fn default() -> Self {
        Self {
            peaks: std::array::from_fn(|_| AtomicU32::new(0)),
            taking: AtomicBool::new(false),
            began: AtomicU64::new(0),
            lost: AtomicU64::new(0),
        }
    }
}

struct Tap {
    heard: Arc<Heard>,
    width: usize,
    queue: Producer<f32>,
    ear: Option<Producer<f32>>,
    was_taking: bool,
}

impl Tap {
    fn hear<Frame: Iterator<Item = f32>>(&mut self, frames: impl Iterator<Item = Frame>, first_at: Instant) {
        let taking = self.heard.taking.load(Ordering::Acquire);
        if taking && !self.was_taking {
            self.heard.began.store(clock::nanos(first_at).max(1), Ordering::Release);
        }
        self.was_taking = taking;
        let width = self.width;
        let mut loudest = [0.0f32; MOST_INPUTS];
        let mut lost = 0;
        for frame in frames {
            let keep = taking && self.queue.slots() >= width;
            if taking && !keep {
                lost += 1;
            }
            let mut ear = self.ear.as_mut().filter(|ear| ear.slots() >= width);
            for (channel, sample) in frame.chain(std::iter::repeat(0.0)).take(width).enumerate() {
                if sample.is_finite() {
                    loudest[channel] = loudest[channel].max(sample.abs());
                }
                if keep {
                    let _ = self.queue.push(sample);
                }
                if let Some(ear) = ear.as_mut() {
                    let _ = ear.push(sample);
                }
            }
        }
        for (peak, heard) in self.heard.peaks.iter().zip(&loudest[..width]) {
            note(peak, *heard);
        }
        if lost > 0 {
            self.heard.lost.fetch_add(lost, Ordering::Relaxed);
        }
    }
}

enum Order {
    Begin(Vec<(PathBuf, InputChannels)>, mpsc::Sender<Result<(), String>>),
    Finish(mpsc::Sender<Result<Vec<Take>, String>>),
}

struct Opened {
    rate: u32,
    width: usize,
    queue: Consumer<f32>,
    ear: Consumer<f32>,
}

#[derive(Default)]
struct Shaping {
    loudest: f32,
    counted: usize,
}

pub struct Input {
    heard: Arc<Heard>,
    shape: Arc<Mutex<Vec<f32>>>,
    rate: u32,
    width: usize,
    ear: Option<Consumer<f32>>,
    orders: Option<mpsc::Sender<Order>>,
    quit: Arc<AtomicBool>,
    host: Option<thread::JoinHandle<()>>,
    keeper: Option<thread::JoinHandle<()>>,
}

pub fn input_devices() -> Vec<String> {
    crate::devices::host()
        .input_devices()
        .map(|devices| devices.filter_map(|device| device.name().ok()).collect())
        .unwrap_or_default()
}

pub fn input_count(choice: &InputChoice) -> Option<u16> {
    if *choice == InputChoice::Practice {
        return Some(PRACTICE_INPUTS);
    }
    let first = devices_to_try(choice).ok()?.into_iter().next()?;
    let channels = settings_for(&first).ok()?.channels();
    Some(channels.clamp(1, MOST_INPUTS as u16))
}

impl Input {
    pub fn open(choice: InputChoice) -> Result<Self, String> {
        clock::start();
        let heard = Arc::new(Heard::default());
        let quit = Arc::new(AtomicBool::new(false));
        let (opened_tx, opened_rx) = mpsc::channel();
        let host = thread::Builder::new()
            .name("loupe-input".into())
            .spawn({
                let quit = quit.clone();
                let heard = heard.clone();
                move || listen(choice, heard, quit, opened_tx)
            })
            .map_err(|why| why.to_string())?;
        let Opened { rate, width, queue, ear } = opened_rx.recv().map_err(|_| "the input thread stopped".to_string())??;
        let (orders, orders_rx) = mpsc::channel();
        let shape = Arc::new(Mutex::new(Vec::new()));
        let keeper = thread::Builder::new()
            .name("loupe-takes".into())
            .spawn({
                let heard = heard.clone();
                let shape = shape.clone();
                move || keep_takes(queue, width, heard, shape, rate, orders_rx)
            })
            .map_err(|why| why.to_string())?;
        Ok(Self { heard, shape, rate, width, ear: Some(ear), orders: Some(orders), quit, host: Some(host), keeper: Some(keeper) })
    }

    pub fn shape(&self) -> Vec<f32> {
        self.shape.lock().map(|kept| kept.clone()).unwrap_or_default()
    }

    pub fn shape_step() -> Duration {
        Duration::from_millis(SHAPE_STEP_MS as u64)
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn inputs(&self) -> u16 {
        self.width as u16
    }

    pub(crate) fn take_ear(&mut self) -> Option<(Consumer<f32>, usize)> {
        self.ear.take().map(|ear| (ear, self.width))
    }

    pub fn take_peaks(&self) -> Vec<f32> {
        self.heard.peaks[..self.width].iter().map(|peak| f32::from_bits(peak.swap(0, Ordering::Relaxed))).collect()
    }

    pub fn begin_takes(&self, takes: &[(PathBuf, InputChannels)]) -> Result<(), String> {
        if let Some((_, missing)) = takes.iter().find(|(_, channels)| !channels.fits(self.inputs())) {
            return Err(format!("{} is not there: the input has {} channels", missing.name(), self.width));
        }
        let (reply, answer) = mpsc::channel();
        self.order(Order::Begin(takes.to_vec(), reply))?;
        answer.recv().map_err(|_| "the take keeper stopped".to_string())?
    }

    pub fn take_began(&self) -> Option<Instant> {
        match self.heard.began.load(Ordering::Acquire) {
            0 => None,
            nanos => Some(clock::instant(nanos)),
        }
    }

    pub fn finish_takes(&self) -> Result<Vec<Take>, String> {
        let (reply, answer) = mpsc::channel();
        self.order(Order::Finish(reply))?;
        answer.recv().map_err(|_| "the take keeper stopped".to_string())?
    }

    fn order(&self, order: Order) -> Result<(), String> {
        let orders = self.orders.as_ref().ok_or("the recording input is closing")?;
        orders.send(order).map_err(|_| "the take keeper stopped".to_string())
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        self.quit.store(true, Ordering::Relaxed);
        self.orders = None;
        for thread in [self.host.take(), self.keeper.take()].into_iter().flatten() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

fn note(peak: &AtomicU32, heard: f32) {
    if heard.is_finite() {
        peak.fetch_max(heard.abs().to_bits(), Ordering::Relaxed);
    }
}

struct TakeFile {
    path: PathBuf,
    channels: InputChannels,
    out: BufWriter<File>,
    frames: u64,
    rate: u32,
}

impl TakeFile {
    fn create(path: &Path, channels: InputChannels, rate: u32) -> io::Result<Self> {
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder)?;
        }
        let mut out = BufWriter::new(File::create(path)?);
        out.write_all(&wav::float_header(channels.width(), rate, 0))?;
        Ok(Self { path: path.to_path_buf(), channels, out, frames: 0, rate })
    }

    fn keep(&mut self, frame: &[f32]) -> io::Result<()> {
        if self.frames >= wav::most_frames(self.channels.width()) {
            return Ok(());
        }
        let first = self.channels.first() as usize;
        for sample in &frame[first..first + self.channels.width() as usize] {
            self.out.write_all(&sample.to_le_bytes())?;
        }
        self.frames += 1;
        Ok(())
    }

    fn close(mut self, lost: u64) -> io::Result<Take> {
        self.out.seek(SeekFrom::Start(0))?;
        self.out.write_all(&wav::float_header(self.channels.width(), self.rate, self.frames as u32))?;
        self.out.flush()?;
        Ok(Take { path: self.path, frames: self.frames, rate: self.rate, lost })
    }
}

fn named(path: &Path, why: io::Error) -> String {
    format!("{}: {why}", path.display())
}

fn drain(
    files: &mut [TakeFile],
    queue: &mut Consumer<f32>,
    width: usize,
    waiting: &mut Vec<f32>,
    shape: &Arc<Mutex<Vec<f32>>>,
    shaping: &mut Shaping,
    per_step: usize,
) -> Result<(), String> {
    let whole = queue.slots() / width * width;
    let Ok(chunk) = queue.read_chunk(whole) else {
        return Ok(());
    };
    let (front, back) = chunk.as_slices();
    waiting.clear();
    waiting.extend_from_slice(front);
    waiting.extend_from_slice(back);
    chunk.commit_all();
    let mut steps = Vec::new();
    for frame in waiting.chunks_exact(width) {
        for file in files.iter_mut() {
            file.keep(frame).map_err(|why| named(&file.path, why))?;
        }
        let loudest = frame.iter().fold(0.0f32, |most, sample| most.max(sample.abs()));
        shaping.loudest = shaping.loudest.max(loudest);
        shaping.counted += 1;
        if shaping.counted >= per_step {
            steps.push(shaping.loudest.min(1.0));
            *shaping = Shaping::default();
        }
    }
    if !steps.is_empty() {
        if let Ok(mut kept) = shape.lock() {
            kept.extend(steps);
            if kept.len() > SHAPE_KEPT {
                let over = kept.len() - SHAPE_KEPT;
                kept.drain(..over);
            }
        }
    }
    Ok(())
}

fn close_all(files: Vec<TakeFile>, lost: u64) -> Result<Vec<Take>, String> {
    files
        .into_iter()
        .map(|file| {
            let path = file.path.clone();
            file.close(lost).map_err(|why| named(&path, why))
        })
        .collect()
}

fn keep_takes(mut queue: Consumer<f32>, width: usize, heard: Arc<Heard>, shape: Arc<Mutex<Vec<f32>>>, rate: u32, orders: mpsc::Receiver<Order>) {
    let per_step = (rate as usize * SHAPE_STEP_MS / 1000).max(1);
    let mut shaping = Shaping::default();
    let mut open: Vec<TakeFile> = Vec::new();
    let mut trouble: Option<String> = None;
    let mut waiting = Vec::new();
    loop {
        match orders.recv_timeout(KEEPER_RESTS_FOR) {
            Ok(Order::Begin(takes, reply)) => {
                heard.taking.store(false, Ordering::Release);
                if let Ok(mut kept) = shape.lock() {
                    kept.clear();
                }
                shaping = Shaping::default();
                while queue.pop().is_ok() {}
                let made: Result<Vec<TakeFile>, String> =
                    takes.iter().map(|(path, channels)| TakeFile::create(path, *channels, rate).map_err(|why| named(path, why))).collect();
                let _ = reply.send(made.map(|files| {
                    open = files;
                    trouble = None;
                    heard.began.store(0, Ordering::Release);
                    heard.lost.store(0, Ordering::Relaxed);
                    heard.taking.store(true, Ordering::Release);
                }));
            }
            Ok(Order::Finish(reply)) => {
                heard.taking.store(false, Ordering::Release);
                thread::sleep(TAKE_SETTLES_FOR);
                let lost = heard.lost.load(Ordering::Relaxed);
                let mut files = std::mem::take(&mut open);
                let finished = match (files.is_empty(), trouble.take()) {
                    (true, _) => Err("nothing was being recorded".to_string()),
                    (false, Some(why)) => {
                        let _ = close_all(files, lost);
                        Err(why)
                    }
                    (false, None) => drain(&mut files, &mut queue, width, &mut waiting, &shape, &mut shaping, per_step).and_then(|_| close_all(files, lost)),
                };
                let _ = reply.send(finished);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                heard.taking.store(false, Ordering::Release);
                let mut files = std::mem::take(&mut open);
                let _ = drain(&mut files, &mut queue, width, &mut waiting, &shape, &mut shaping, per_step);
                let _ = close_all(files, 0);
                return;
            }
        }
        if !open.is_empty() && trouble.is_none() {
            if let Err(why) = drain(&mut open, &mut queue, width, &mut waiting, &shape, &mut shaping, per_step) {
                heard.taking.store(false, Ordering::Release);
                trouble = Some(why);
            }
        }
    }
}

fn tap_for(heard: Arc<Heard>, rate: u32, width: usize) -> (Tap, Opened) {
    let (queue_in, queue) = RingBuffer::new(TAKE_QUEUE_FRAMES * width);
    let (ear_in, ear) = RingBuffer::new(EAR_QUEUE_FRAMES * width);
    (Tap { heard, width, queue: queue_in, ear: Some(ear_in), was_taking: false }, Opened { rate, width, queue, ear })
}

fn listen(choice: InputChoice, heard: Arc<Heard>, quit: Arc<AtomicBool>, opened: mpsc::Sender<Result<Opened, String>>) {
    if choice == InputChoice::Practice {
        let (mut tap, ends) = tap_for(heard, PRACTICE_RATE, PRACTICE_INPUTS as usize);
        let _ = opened.send(Ok(ends));
        let began = Instant::now();
        let mut made: u64 = 0;
        while !quit.load(Ordering::Relaxed) {
            let due = (began.elapsed().as_secs_f64() * PRACTICE_RATE as f64) as u64;
            let first_at = began + Duration::from_secs_f64(made as f64 / PRACTICE_RATE as f64);
            tap.hear((made..due).map(|frame| (0..PRACTICE_INPUTS).map(move |channel| practice_sample(frame, channel))), first_at);
            made = due;
            thread::park_timeout(Duration::from_millis(10));
        }
        return;
    }
    match open_device(&choice, heard) {
        Ok((stream, ends)) => {
            let _ = opened.send(Ok(ends));
            while !quit.load(Ordering::Relaxed) {
                thread::park_timeout(Duration::from_millis(200));
            }
            drop(stream);
        }
        Err(why) => {
            let _ = opened.send(Err(why));
        }
    }
}

fn practice_sample(frame: u64, channel: u16) -> f32 {
    let seconds = frame as f32 / PRACTICE_RATE as f32;
    let phase = (seconds / PRACTICE_SWELL_SECONDS).fract();
    let swell = (1.0 - (phase * 2.0 - 1.0).abs()) * PRACTICE_LOUDEST;
    let turn = (frame % PRACTICE_RATE as u64) as f32 / PRACTICE_RATE as f32;
    swell * (turn * practice_tone(channel) * std::f32::consts::TAU).sin()
}

fn practice_tone(channel: u16) -> f32 {
    PRACTICE_TONE_HZ * (channel + 1) as f32
}

fn named_of(device: &cpal::Device) -> String {
    device.name().unwrap_or_else(|_| "an unnamed input".to_string())
}

fn devices_to_try(choice: &InputChoice) -> Result<Vec<cpal::Device>, String> {
    if crate::devices::one_device_both_ways() {
        return crate::devices::in_use()
            .map(|device| vec![device])
            .ok_or_else(|| "the ASIO driver is not open, so there is nothing to record from".to_string());
    }
    let host = crate::devices::host();
    let listed: Vec<cpal::Device> = host.input_devices().map_err(|why| why.to_string())?.collect();
    let mut order = Vec::new();
    if let InputChoice::Named(wanted) = choice {
        if let Some(found) = listed.iter().position(|device| device.name().is_ok_and(|name| &name == wanted)) {
            order.push(listed[found].clone());
        }
    }
    if let Some(usual) = host.default_input_device() {
        let name = named_of(&usual);
        if !order.iter().any(|device| named_of(device) == name) {
            order.push(usual);
        }
    }
    for device in listed {
        let name = named_of(&device);
        if !order.iter().any(|already| named_of(already) == name) {
            order.push(device);
        }
    }
    if order.is_empty() {
        return Err("no recording input found".to_string());
    }
    Ok(order)
}

fn settings_for(device: &cpal::Device) -> Result<cpal::SupportedStreamConfig, String> {
    if let Ok(usual) = device.default_input_config() {
        return Ok(usual);
    }
    if crate::devices::one_device_both_ways() {
        return Err("the ASIO driver would not say how it records".to_string());
    }
    let offered = device.supported_input_configs().map_err(|why| why.to_string())?;
    offered
        .max_by_key(|range| (range.channels().min(MOST_INPUTS as u16), range.max_sample_rate().0))
        .map(|range| range.with_max_sample_rate())
        .ok_or_else(|| "the input is listed but offers no way to record from it".to_string())
}

fn open_device(choice: &InputChoice, heard: Arc<Heard>) -> Result<(cpal::Stream, Opened), String> {
    let mut refused = Vec::new();
    for device in devices_to_try(choice)? {
        match open_one(&device, heard.clone()) {
            Ok(working) => return Ok(working),
            Err(why) => refused.push(format!("{}: {why}", named_of(&device))),
        }
    }
    Err(format!("no recording input would open. {}", refused.join("; ")))
}

fn smallest_block(device: &cpal::Device) -> Option<u32> {
    if crate::devices::one_device_both_ways() {
        return None;
    }
    device.supported_input_configs().ok()?.filter_map(|range| match range.buffer_size() {
        cpal::SupportedBufferSize::Range { min, .. } => Some(*min),
        cpal::SupportedBufferSize::Unknown => None,
    })
    .min()
}

fn open_one(device: &cpal::Device, heard: Arc<Heard>) -> Result<(cpal::Stream, Opened), String> {
    let supported = settings_for(device)?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let small = smallest_block(device);
    let attempt = |config: &cpal::StreamConfig| {
        let (tap, ends) = tap_for(heard.clone(), config.sample_rate.0, (config.channels as usize).clamp(1, MOST_INPUTS));
        let built = match format {
            cpal::SampleFormat::F32 => stream::<f32>(device, config, tap),
            cpal::SampleFormat::I16 => stream::<i16>(device, config, tap),
            cpal::SampleFormat::U16 => stream::<u16>(device, config, tap),
            cpal::SampleFormat::I32 => stream::<i32>(device, config, tap),
            other => return Err(format!("the input uses a format Loupe cannot read ({other})")),
        };
        built.map(|stream| (stream, ends)).map_err(|why| why.to_string())
    };
    let opened = match small {
        Some(block) => {
            let mut tight = config.clone();
            tight.buffer_size = cpal::BufferSize::Fixed(block);
            match attempt(&tight) {
                Ok(working) => Ok(working),
                Err(_) => attempt(&config),
            }
        }
        None => attempt(&config),
    };
    let (stream, ends) = opened?;
    stream.play().map_err(|why| why.to_string())?;
    Ok((stream, ends))
}

fn stream<T>(device: &cpal::Device, config: &cpal::StreamConfig, mut tap: Tap) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = config.channels.max(1) as usize;
    device.build_input_stream(
        config,
        move |heard: &[T], info: &cpal::InputCallbackInfo| {
            let stamp = info.timestamp();
            let delay = stamp.callback.duration_since(&stamp.capture).unwrap_or_default();
            let first_at = Instant::now().checked_sub(delay).unwrap_or_else(clock::start);
            tap.hear(heard.chunks(channels).map(|frame| frame.iter().map(|sample| f32::from_sample_(*sample))), first_at);
        },
        |why| eprintln!("loupe: recording input error: {why}"),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Source;

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("loupe-take-{}-{name}", std::process::id())).join("Audio").join("take.wav")
    }

    fn tone_of(frames: &[[f32; 2]], side: usize) -> f32 {
        let turns = frames.windows(2).filter(|pair| (pair[0][side] < 0.0) != (pair[1][side] < 0.0)).count();
        turns as f32 / 2.0 / (frames.len() as f32 / PRACTICE_RATE as f32)
    }

    fn near(heard: f32, tone: f32) -> bool {
        (heard - tone).abs() < tone * 0.1
    }

    #[test]
    fn the_practice_input_reports_a_level_that_moves() {
        let input = Input::open(InputChoice::Practice).unwrap();
        assert_eq!(input.inputs(), PRACTICE_INPUTS);
        thread::sleep(Duration::from_millis(120));
        let first = input.take_peaks();
        assert_eq!(first.len(), PRACTICE_INPUTS as usize);
        assert!(first.iter().all(|peak| *peak > 0.0 && *peak <= PRACTICE_LOUDEST));
        thread::sleep(Duration::from_millis(400));
        assert!(input.take_peaks()[1] > first[1], "the level keeps rising in the first half of the swell");
    }

    #[test]
    fn the_peak_is_the_loudest_since_the_last_look() {
        let peak = AtomicU32::new(0);
        for heard in [0.25, -0.75, 0.5, f32::NAN] {
            note(&peak, heard);
        }
        assert_eq!(f32::from_bits(peak.swap(0, Ordering::Relaxed)), 0.75);
        assert_eq!(f32::from_bits(peak.load(Ordering::Relaxed)), 0.0);
    }

    #[test]
    fn nothing_is_queued_until_a_take_begins_and_overflow_is_counted() {
        let heard = Arc::new(Heard::default());
        let (queue, mut kept) = RingBuffer::new(4);
        let mut tap = Tap { heard: heard.clone(), width: 1, queue, ear: None, was_taking: false };
        tap.hear([0.5, 0.5].into_iter().map(std::iter::once), Instant::now());
        assert!(kept.pop().is_err());
        assert_eq!(heard.began.load(Ordering::Relaxed), 0);
        heard.taking.store(true, Ordering::Relaxed);
        tap.hear([0.1, 0.2, 0.3, 0.4, 0.5, 0.6].into_iter().map(std::iter::once), Instant::now());
        assert_ne!(heard.began.load(Ordering::Relaxed), 0);
        assert_eq!(std::iter::from_fn(|| kept.pop().ok()).collect::<Vec<_>>(), [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(heard.lost.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn every_channel_is_queued_whole_frames_at_a_time_with_its_own_level() {
        let heard = Arc::new(Heard::default());
        let (queue, mut kept) = RingBuffer::new(5);
        let (ear, mut heard_back) = RingBuffer::new(8);
        let mut tap = Tap { heard: heard.clone(), width: 2, queue, ear: Some(ear), was_taking: false };
        heard.taking.store(true, Ordering::Relaxed);
        let frames = [vec![0.1, -0.9], vec![0.2, 0.3], vec![0.4]];
        tap.hear(frames.iter().map(|frame| frame.iter().copied()), Instant::now());
        assert_eq!(std::iter::from_fn(|| kept.pop().ok()).collect::<Vec<_>>(), [0.1, -0.9, 0.2, 0.3], "half a frame is never queued");
        assert_eq!(heard.lost.load(Ordering::Relaxed), 1, "lost counts frames");
        assert_eq!(std::iter::from_fn(|| heard_back.pop().ok()).collect::<Vec<_>>(), [0.1, -0.9, 0.2, 0.3, 0.4, 0.0]);
        let peaks: Vec<f32> = heard.peaks[..2].iter().map(|peak| f32::from_bits(peak.load(Ordering::Relaxed))).collect();
        assert_eq!(peaks, [0.4, 0.9]);
    }

    #[test]
    fn a_take_lands_on_disk_as_long_as_it_ran() {
        let file = scratch("length");
        let input = Input::open(InputChoice::Practice).unwrap();
        assert_eq!(input.take_began(), None);
        let asked = Instant::now();
        input.begin_takes(&[(file.clone(), InputChannels::Mono(0))]).unwrap();
        thread::sleep(Duration::from_millis(500));
        let takes = input.finish_takes().unwrap();
        let ran = asked.elapsed().as_secs_f64();
        let began = input.take_began().expect("the first sample has a time");
        assert!(began >= asked - Duration::from_millis(20) && began <= asked + Duration::from_millis(60));
        let take = &takes[0];
        assert_eq!((takes.len(), take.rate, take.lost), (1, PRACTICE_RATE, 0));
        let seconds = take.frames as f64 / take.rate as f64;
        assert!(seconds > 0.45 && seconds <= ran, "about half a second, got {seconds}");
        let read = Source::load(&take.path, PRACTICE_RATE).unwrap();
        assert_eq!(read.frames.len() as u64, take.frames);
        assert!(read.frames.iter().any(|frame| frame[0].abs() > 0.01), "the take is not silence");
        assert!(read.frames.iter().all(|frame| frame[0] == frame[1]), "one input lands in the middle");
        assert!(near(tone_of(&read.frames, 0), practice_tone(0)), "input 1 is recorded");
        assert!(input.finish_takes().is_err(), "there is no take left to finish");
        fs::remove_dir_all(file.parent().unwrap().parent().unwrap()).unwrap();
    }

    #[test]
    fn a_take_draws_its_shape_as_it_runs() {
        let file = scratch("shape");
        let input = Input::open(InputChoice::Practice).unwrap();
        assert!(input.shape().is_empty(), "nothing is recorded yet");
        input.begin_takes(&[(file.clone(), InputChannels::Mono(0))]).unwrap();
        thread::sleep(Duration::from_millis(300));
        let early = input.shape();
        thread::sleep(Duration::from_millis(300));
        let later = input.shape();
        let _ = input.finish_takes().unwrap();
        let steps = Input::shape_step().as_secs_f64();
        assert!(early.len() >= 20, "about {:.0} steps in 300 ms, got {}", 0.3 / steps, early.len());
        assert!(later.len() > early.len(), "the shape keeps growing");
        assert!(later.iter().all(|loudest| (0.0..=1.0).contains(loudest)), "every step is a level");
        assert!(later.iter().any(|loudest| *loudest > 0.01), "the shape is not silence");
        fs::remove_dir_all(file.parent().unwrap().parent().unwrap()).unwrap();
    }

    #[test]
    fn each_take_records_its_own_inputs_at_the_same_time() {
        let folder = scratch("several");
        let folder = folder.parent().unwrap();
        let second = folder.join("Vocal (take 1).wav");
        let pair = folder.join("Keys (take 1).wav");
        let first = folder.join("Guitar (take 1).wav");
        let input = Input::open(InputChoice::Practice).unwrap();
        let wanted = [(second.clone(), InputChannels::Mono(1)), (pair.clone(), InputChannels::Stereo(2)), (first.clone(), InputChannels::Mono(0))];
        input.begin_takes(&wanted).unwrap();
        thread::sleep(Duration::from_millis(400));
        let takes = input.finish_takes().unwrap();
        assert_eq!(takes.iter().map(|take| take.path.clone()).collect::<Vec<_>>(), [second.clone(), pair.clone(), first.clone()]);
        assert!(takes.iter().all(|take| take.frames == takes[0].frames && take.frames > 0), "every take is as long as the others");
        let vocal = Source::load(&second, PRACTICE_RATE).unwrap();
        assert!(vocal.frames.iter().all(|frame| frame[0] == frame[1]), "one input lands in the middle");
        assert!(near(tone_of(&vocal.frames, 0), practice_tone(1)), "input 2 landed on the vocal, heard {}", tone_of(&vocal.frames, 0));
        let keys = Source::load(&pair, PRACTICE_RATE).unwrap();
        assert!(keys.frames.iter().any(|frame| frame[0] != frame[1]), "the pair stays stereo");
        assert!(near(tone_of(&keys.frames, 0), practice_tone(2)), "input 3 is on the left");
        assert!(near(tone_of(&keys.frames, 1), practice_tone(3)), "input 4 is on the right");
        let guitar = Source::load(&first, PRACTICE_RATE).unwrap();
        assert!(near(tone_of(&guitar.frames, 0), practice_tone(0)), "input 1 landed on the guitar");
        fs::remove_dir_all(folder.parent().unwrap()).unwrap();
    }

    #[test]
    fn an_input_the_device_does_not_have_is_refused() {
        let file = scratch("missing");
        let input = Input::open(InputChoice::Practice).unwrap();
        let why = input.begin_takes(&[(file.clone(), InputChannels::Stereo(4))]).unwrap_err();
        assert!(why.contains("Inputs 5+6"), "{why}");
        assert!(!file.exists());
        assert_eq!(input_count(&InputChoice::Practice), Some(PRACTICE_INPUTS));
    }

    #[test]
    fn a_second_take_starts_clean() {
        let file = scratch("second");
        let input = Input::open(InputChoice::Practice).unwrap();
        input.begin_takes(&[(file.clone(), InputChannels::Mono(0))]).unwrap();
        thread::sleep(Duration::from_millis(300));
        let first = input.finish_takes().unwrap().remove(0);
        thread::sleep(Duration::from_millis(200));
        input.begin_takes(&[(file.clone(), InputChannels::Stereo(0))]).unwrap();
        thread::sleep(Duration::from_millis(100));
        let second = input.finish_takes().unwrap().remove(0);
        assert!(second.frames < first.frames, "nothing from between the takes is kept");
        fs::remove_dir_all(file.parent().unwrap().parent().unwrap()).unwrap();
    }

    #[test]
    fn closing_the_input_mid_take_still_leaves_a_readable_file() {
        let file = scratch("dropped");
        let input = Input::open(InputChoice::Practice).unwrap();
        input.begin_takes(&[(file.clone(), InputChannels::Stereo(0))]).unwrap();
        thread::sleep(Duration::from_millis(200));
        drop(input);
        let read = Source::load(&file, PRACTICE_RATE).unwrap();
        assert!(read.frames.len() > 4_000);
        assert!(read.frames.iter().any(|frame| frame[0] != frame[1]));
        fs::remove_dir_all(file.parent().unwrap().parent().unwrap()).unwrap();
    }
}
