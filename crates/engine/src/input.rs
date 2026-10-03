use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};

const PRACTICE_SWELL_SECONDS: f32 = 3.0;
const PRACTICE_LOUDEST: f32 = 1.2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputChoice {
    SystemDefault,
    Named(String),
    Practice,
}

pub struct Input {
    peak: Arc<AtomicU32>,
    quit: Arc<AtomicBool>,
    host: Option<thread::JoinHandle<()>>,
}

pub fn input_devices() -> Vec<String> {
    cpal::default_host()
        .input_devices()
        .map(|devices| devices.filter_map(|device| device.name().ok()).collect())
        .unwrap_or_default()
}

impl Input {
    pub fn open(choice: InputChoice) -> Result<Self, String> {
        let peak = Arc::new(AtomicU32::new(0));
        let quit = Arc::new(AtomicBool::new(false));
        let (opened_tx, opened_rx) = mpsc::channel();
        let host = thread::Builder::new()
            .name("loupe-input".into())
            .spawn({
                let (peak, quit) = (peak.clone(), quit.clone());
                move || listen(choice, peak, quit, opened_tx)
            })
            .map_err(|why| why.to_string())?;
        opened_rx.recv().map_err(|_| "the input thread stopped".to_string())??;
        Ok(Self { peak, quit, host: Some(host) })
    }

    pub fn take_peak(&self) -> f32 {
        f32::from_bits(self.peak.swap(0, Ordering::Relaxed))
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        self.quit.store(true, Ordering::Relaxed);
        if let Some(host) = self.host.take() {
            host.thread().unpark();
            let _ = host.join();
        }
    }
}

fn note(peak: &AtomicU32, heard: f32) {
    if heard.is_finite() {
        peak.fetch_max(heard.abs().to_bits(), Ordering::Relaxed);
    }
}

fn listen(choice: InputChoice, peak: Arc<AtomicU32>, quit: Arc<AtomicBool>, opened: mpsc::Sender<Result<(), String>>) {
    if choice == InputChoice::Practice {
        let _ = opened.send(Ok(()));
        let began = Instant::now();
        while !quit.load(Ordering::Relaxed) {
            let phase = (began.elapsed().as_secs_f32() / PRACTICE_SWELL_SECONDS).fract();
            note(&peak, (1.0 - (phase * 2.0 - 1.0).abs()) * PRACTICE_LOUDEST);
            thread::park_timeout(Duration::from_millis(10));
        }
        return;
    }
    match open_device(&choice, peak) {
        Ok(stream) => {
            let _ = opened.send(Ok(()));
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

fn open_device(choice: &InputChoice, peak: Arc<AtomicU32>) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
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
        cpal::SampleFormat::F32 => stream::<f32>(&device, &config, peak),
        cpal::SampleFormat::I16 => stream::<i16>(&device, &config, peak),
        cpal::SampleFormat::U16 => stream::<u16>(&device, &config, peak),
        cpal::SampleFormat::I32 => stream::<i32>(&device, &config, peak),
        other => return Err(format!("the input uses a format Loupe cannot read ({other})")),
    }
    .map_err(|why| why.to_string())?;
    stream.play().map_err(|why| why.to_string())?;
    Ok(stream)
}

fn stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    peak: Arc<AtomicU32>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device.build_input_stream(
        config,
        move |heard: &[T], _: &cpal::InputCallbackInfo| {
            let loudest = heard.iter().map(|sample| f32::from_sample_(*sample).abs()).fold(0.0, f32::max);
            note(&peak, loudest);
        },
        |why| eprintln!("loupe: recording input error: {why}"),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
