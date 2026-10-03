use std::fs::{self, File};
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::clock;
use crate::wav;

const PRACTICE_SWELL_SECONDS: f32 = 3.0;
const PRACTICE_LOUDEST: f32 = 1.2;
const PRACTICE_RATE: u32 = 48_000;
const PRACTICE_TONE_HZ: f32 = 220.0;
const TAKE_QUEUE: usize = 1 << 19;
const TAKE_SETTLES_FOR: Duration = Duration::from_millis(30);
const KEEPER_RESTS_FOR: Duration = Duration::from_millis(15);
const TAKE_CHANNELS: u16 = 1;

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

#[derive(Default)]
struct Heard {
    peak: AtomicU32,
    taking: AtomicBool,
    began: AtomicU64,
    lost: AtomicU64,
}

struct Tap {
    heard: Arc<Heard>,
    queue: Producer<f32>,
    was_taking: bool,
}

impl Tap {
    fn hear(&mut self, samples: impl Iterator<Item = f32>, first_at: Instant) {
        let taking = self.heard.taking.load(Ordering::Acquire);
        if taking && !self.was_taking {
            self.heard.began.store(clock::nanos(first_at).max(1), Ordering::Release);
        }
        self.was_taking = taking;
        let mut loudest = 0.0f32;
        let mut lost = 0;
        for sample in samples {
            if sample.is_finite() {
                loudest = loudest.max(sample.abs());
            }
            if taking && self.queue.push(sample).is_err() {
                lost += 1;
            }
        }
        note(&self.heard.peak, loudest);
        if lost > 0 {
            self.heard.lost.fetch_add(lost, Ordering::Relaxed);
        }
    }
}

enum Order {
    Begin(PathBuf, mpsc::Sender<Result<(), String>>),
    Finish(mpsc::Sender<Result<Take, String>>),
}

pub struct Input {
    heard: Arc<Heard>,
    rate: u32,
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

impl Input {
    pub fn open(choice: InputChoice) -> Result<Self, String> {
        clock::start();
        let heard = Arc::new(Heard::default());
        let quit = Arc::new(AtomicBool::new(false));
        let (queue_in, queue_out) = RingBuffer::new(TAKE_QUEUE);
        let tap = Tap { heard: heard.clone(), queue: queue_in, was_taking: false };
        let (opened_tx, opened_rx) = mpsc::channel();
        let host = thread::Builder::new()
            .name("loupe-input".into())
            .spawn({
                let quit = quit.clone();
                move || listen(choice, tap, quit, opened_tx)
            })
            .map_err(|why| why.to_string())?;
        let rate = opened_rx.recv().map_err(|_| "the input thread stopped".to_string())??;
        let (orders, orders_rx) = mpsc::channel();
        let keeper = thread::Builder::new()
            .name("loupe-takes".into())
            .spawn({
                let heard = heard.clone();
                move || keep_takes(queue_out, heard, rate, orders_rx)
            })
            .map_err(|why| why.to_string())?;
        Ok(Self { heard, rate, orders: Some(orders), quit, host: Some(host), keeper: Some(keeper) })
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn take_peak(&self) -> f32 {
        f32::from_bits(self.heard.peak.swap(0, Ordering::Relaxed))
    }

    pub fn begin_take(&self, path: &Path) -> Result<(), String> {
        let (reply, answer) = mpsc::channel();
        self.order(Order::Begin(path.to_path_buf(), reply))?;
        answer.recv().map_err(|_| "the take keeper stopped".to_string())?
    }

    pub fn take_began(&self) -> Option<Instant> {
        match self.heard.began.load(Ordering::Acquire) {
            0 => None,
            nanos => Some(clock::instant(nanos)),
        }
    }

    pub fn finish_take(&self) -> Result<Take, String> {
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
    out: BufWriter<File>,
    frames: u64,
    rate: u32,
}

impl TakeFile {
    fn create(path: &Path, rate: u32) -> io::Result<Self> {
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder)?;
        }
        let mut out = BufWriter::new(File::create(path)?);
        out.write_all(&wav::float_header(TAKE_CHANNELS, rate, 0))?;
        Ok(Self { path: path.to_path_buf(), out, frames: 0, rate })
    }

    fn drain(&mut self, queue: &mut Consumer<f32>) -> io::Result<()> {
        let room = wav::most_frames(TAKE_CHANNELS);
        while let Ok(sample) = queue.pop() {
            if self.frames < room {
                self.out.write_all(&sample.to_le_bytes())?;
                self.frames += 1;
            }
        }
        Ok(())
    }

    fn close(mut self, lost: u64) -> io::Result<Take> {
        self.out.seek(SeekFrom::Start(0))?;
        self.out.write_all(&wav::float_header(TAKE_CHANNELS, self.rate, self.frames as u32))?;
        self.out.flush()?;
        Ok(Take { path: self.path, frames: self.frames, rate: self.rate, lost })
    }
}

fn keep_takes(mut queue: Consumer<f32>, heard: Arc<Heard>, rate: u32, orders: mpsc::Receiver<Order>) {
    let mut open: Option<TakeFile> = None;
    let mut trouble: Option<String> = None;
    let named = |path: &Path, why: io::Error| format!("{}: {why}", path.display());
    loop {
        match orders.recv_timeout(KEEPER_RESTS_FOR) {
            Ok(Order::Begin(path, reply)) => {
                heard.taking.store(false, Ordering::Release);
                while queue.pop().is_ok() {}
                let made = TakeFile::create(&path, rate).map_err(|why| named(&path, why));
                let _ = reply.send(made.map(|file| {
                    open = Some(file);
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
                let finished = match (open.take(), trouble.take()) {
                    (None, _) => Err("nothing was being recorded".to_string()),
                    (Some(file), Some(why)) => {
                        let _ = file.close(lost);
                        Err(why)
                    }
                    (Some(mut file), None) => {
                        let path = file.path.clone();
                        file.drain(&mut queue).and_then(|_| file.close(lost)).map_err(|why| named(&path, why))
                    }
                };
                let _ = reply.send(finished);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                heard.taking.store(false, Ordering::Release);
                if let Some(mut file) = open.take() {
                    let _ = file.drain(&mut queue).and_then(|_| file.close(0));
                }
                return;
            }
        }
        if let Some(file) = &mut open {
            if let Err(why) = file.drain(&mut queue) {
                heard.taking.store(false, Ordering::Release);
                trouble.get_or_insert(named(&file.path, why));
            }
        }
    }
}

fn listen(choice: InputChoice, mut tap: Tap, quit: Arc<AtomicBool>, opened: mpsc::Sender<Result<u32, String>>) {
    if choice == InputChoice::Practice {
        let _ = opened.send(Ok(PRACTICE_RATE));
        let began = Instant::now();
        let mut made: u64 = 0;
        while !quit.load(Ordering::Relaxed) {
            let due = (began.elapsed().as_secs_f64() * PRACTICE_RATE as f64) as u64;
            let first_at = began + Duration::from_secs_f64(made as f64 / PRACTICE_RATE as f64);
            tap.hear((made..due).map(practice_sample), first_at);
            made = due;
            thread::park_timeout(Duration::from_millis(10));
        }
        return;
    }
    match open_device(&choice, tap) {
        Ok((stream, rate)) => {
            let _ = opened.send(Ok(rate));
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

fn practice_sample(frame: u64) -> f32 {
    let seconds = frame as f32 / PRACTICE_RATE as f32;
    let phase = (seconds / PRACTICE_SWELL_SECONDS).fract();
    let swell = (1.0 - (phase * 2.0 - 1.0).abs()) * PRACTICE_LOUDEST;
    let turn = (frame % PRACTICE_RATE as u64) as f32 / PRACTICE_RATE as f32;
    swell * (turn * PRACTICE_TONE_HZ * std::f32::consts::TAU).sin()
}

fn open_device(choice: &InputChoice, tap: Tap) -> Result<(cpal::Stream, u32), String> {
    let host = crate::devices::host();
    let device = match choice {
        InputChoice::Named(wanted) => host
            .input_devices()
            .map_err(|why| why.to_string())?
            .find(|device| device.name().is_ok_and(|name| &name == wanted))
            .ok_or_else(|| format!("the input \"{wanted}\" is not connected"))?,
        _ => host.default_input_device().ok_or("no recording input found")?,
    };
    let supported = device.default_input_config().map_err(|why| why.to_string())?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let stream = match format {
        cpal::SampleFormat::F32 => stream::<f32>(&device, &config, tap),
        cpal::SampleFormat::I16 => stream::<i16>(&device, &config, tap),
        cpal::SampleFormat::U16 => stream::<u16>(&device, &config, tap),
        cpal::SampleFormat::I32 => stream::<i32>(&device, &config, tap),
        other => return Err(format!("the input uses a format Loupe cannot read ({other})")),
    }
    .map_err(|why| why.to_string())?;
    stream.play().map_err(|why| why.to_string())?;
    Ok((stream, config.sample_rate.0))
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
            tap.hear(heard.chunks(channels).map(|frame| f32::from_sample_(frame[0])), first_at);
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

    #[test]
    fn the_practice_input_reports_a_level_that_moves() {
        let input = Input::open(InputChoice::Practice).unwrap();
        thread::sleep(Duration::from_millis(120));
        let first = input.take_peak();
        assert!(first > 0.0 && first <= PRACTICE_LOUDEST);
        thread::sleep(Duration::from_millis(400));
        assert!(input.take_peak() > first, "the level keeps rising in the first half of the swell");
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
        let mut tap = Tap { heard: heard.clone(), queue, was_taking: false };
        tap.hear([0.5, 0.5].into_iter(), Instant::now());
        assert!(kept.pop().is_err());
        assert_eq!(heard.began.load(Ordering::Relaxed), 0);
        heard.taking.store(true, Ordering::Relaxed);
        tap.hear([0.1, 0.2, 0.3, 0.4, 0.5, 0.6].into_iter(), Instant::now());
        assert_ne!(heard.began.load(Ordering::Relaxed), 0);
        assert_eq!(std::iter::from_fn(|| kept.pop().ok()).collect::<Vec<_>>(), [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(heard.lost.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn a_take_lands_on_disk_as_long_as_it_ran() {
        let file = scratch("length");
        let input = Input::open(InputChoice::Practice).unwrap();
        assert_eq!(input.take_began(), None);
        let asked = Instant::now();
        input.begin_take(&file).unwrap();
        thread::sleep(Duration::from_millis(500));
        let take = input.finish_take().unwrap();
        let ran = asked.elapsed().as_secs_f64();
        let began = input.take_began().expect("the first sample has a time");
        assert!(began >= asked - Duration::from_millis(20) && began <= asked + Duration::from_millis(60));
        assert_eq!((take.rate, take.lost), (PRACTICE_RATE, 0));
        let seconds = take.frames as f64 / take.rate as f64;
        assert!(seconds > 0.45 && seconds <= ran, "about half a second, got {seconds}");
        let read = Source::load(&take.path, PRACTICE_RATE).unwrap();
        assert_eq!(read.frames.len() as u64, take.frames);
        assert!(read.frames.iter().any(|frame| frame[0].abs() > 0.01), "the take is not silence");
        assert!(read.frames.iter().all(|frame| frame[0] == frame[1]), "one input lands in the middle");
        assert!(input.finish_take().is_err(), "there is no take left to finish");
        fs::remove_dir_all(file.parent().unwrap().parent().unwrap()).unwrap();
    }

    #[test]
    fn a_second_take_starts_clean() {
        let file = scratch("second");
        let input = Input::open(InputChoice::Practice).unwrap();
        input.begin_take(&file).unwrap();
        thread::sleep(Duration::from_millis(300));
        let first = input.finish_take().unwrap();
        thread::sleep(Duration::from_millis(200));
        input.begin_take(&file).unwrap();
        thread::sleep(Duration::from_millis(100));
        let second = input.finish_take().unwrap();
        assert!(second.frames < first.frames, "nothing from between the takes is kept");
        fs::remove_dir_all(file.parent().unwrap().parent().unwrap()).unwrap();
    }

    #[test]
    fn closing_the_input_mid_take_still_leaves_a_readable_file() {
        let file = scratch("dropped");
        let input = Input::open(InputChoice::Practice).unwrap();
        input.begin_take(&file).unwrap();
        thread::sleep(Duration::from_millis(200));
        drop(input);
        let read = Source::load(&file, PRACTICE_RATE).unwrap();
        assert!(read.frames.len() > 4_000);
        fs::remove_dir_all(file.parent().unwrap().parent().unwrap()).unwrap();
    }
}
