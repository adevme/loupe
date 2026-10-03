use std::fmt;
use std::fs::File;
use std::path::{Path, PathBuf};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as DecodeError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::resample::resample;

const PEAK_BASE: usize = 64;

pub struct Source {
    pub name: String,
    pub path: PathBuf,
    pub frames: Vec<[f32; 2]>,
    peaks: Vec<Vec<[f32; 2]>>,
}

impl fmt::Debug for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Source({:?}, {} frames)", self.name, self.frames.len())
    }
}

impl Source {
    pub fn from_frames(name: impl Into<String>, frames: Vec<[f32; 2]>) -> Self {
        let peaks = build_peaks(&frames);
        Self { name: name.into(), path: PathBuf::new(), frames, peaks }
    }

    pub fn missing(path: &Path) -> Self {
        Self { name: file_name(path), path: path.to_path_buf(), frames: Vec::new(), peaks: build_peaks(&[]) }
    }

    pub fn load(path: &Path, rate: u32) -> Result<Self, String> {
        let (frames, file_rate) = decode(path)?;
        if frames.is_empty() {
            return Err("the file has no audio in it".into());
        }
        let frames = resample(&frames, file_rate, rate);
        let name = file_name(path);
        let peaks = build_peaks(&frames);
        Ok(Self { name, path: path.to_path_buf(), frames, peaks })
    }

    pub fn stretched(&self, rate: u32, stretch: f64) -> Self {
        let frames = loupe_stretch::stretch(&self.frames, rate, stretch);
        let peaks = build_peaks(&frames);
        Self { name: self.name.clone(), path: self.path.clone(), frames, peaks }
    }

    pub fn peak(&self, from: usize, to: usize) -> (f32, f32) {
        let to = to.min(self.frames.len());
        if from >= to {
            return (0.0, 0.0);
        }
        let span = to - from;
        let level = (0..self.peaks.len()).rev().find(|k| PEAK_BASE << (2 * k) <= span);
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        match level {
            Some(k) => {
                let block = PEAK_BASE << (2 * k);
                let last = to.div_ceil(block).min(self.peaks[k].len());
                for p in &self.peaks[k][from / block..last] {
                    lo = lo.min(p[0]);
                    hi = hi.max(p[1]);
                }
            }
            None => {
                for f in &self.frames[from..to] {
                    lo = lo.min(f[0].min(f[1]));
                    hi = hi.max(f[0].max(f[1]));
                }
            }
        }
        (lo, hi)
    }
}

fn file_name(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Audio".into())
}

fn build_peaks(frames: &[[f32; 2]]) -> Vec<Vec<[f32; 2]>> {
    let mut levels = Vec::new();
    let mut level: Vec<[f32; 2]> = frames
        .chunks(PEAK_BASE)
        .map(|block| {
            block.iter().fold([f32::MAX, f32::MIN], |acc, f| {
                [acc[0].min(f[0].min(f[1])), acc[1].max(f[0].max(f[1]))]
            })
        })
        .collect();
    while level.len() > 1 {
        let next = level
            .chunks(4)
            .map(|group| {
                group
                    .iter()
                    .fold([f32::MAX, f32::MIN], |acc, p| [acc[0].min(p[0]), acc[1].max(p[1])])
            })
            .collect();
        levels.push(std::mem::replace(&mut level, next));
    }
    levels.push(level);
    levels
}

fn decode(path: &Path) -> Result<(Vec<[f32; 2]>, u32), String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let format_options = FormatOptions { enable_gapless: true, ..Default::default() };
    let probed = symphonia::default::get_probe()
        .format(&hint, stream, &format_options, &MetadataOptions::default())
        .map_err(|_| "not an audio format Loupe can read".to_string())?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("the file has no audio track")?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|_| "not an audio codec Loupe can read".to_string())?;

    let mut rate = track.codec_params.sample_rate.unwrap_or(0);
    let mut frames = Vec::new();
    let mut buffer: Option<SampleBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(DecodeError::IoError(_)) | Err(DecodeError::ResetRequired) => break,
            Err(e) => return Err(e.to_string()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(DecodeError::DecodeError(_)) => continue,
            Err(DecodeError::IoError(_)) => break,
            Err(e) => return Err(e.to_string()),
        };
        let spec = *decoded.spec();
        rate = spec.rate;
        let channels = spec.channels.count();
        let needed = decoded.capacity() * channels;
        if buffer.as_ref().map_or(true, |b| b.capacity() < needed) {
            buffer = Some(SampleBuffer::new(decoded.capacity() as u64, spec));
        }
        let buffer = buffer.as_mut().unwrap();
        buffer.copy_interleaved_ref(decoded);
        frames.extend(buffer.samples().chunks_exact(channels).map(|f| match f {
            [mono] => [*mono, *mono],
            [left, right, ..] => [*left, *right],
            [] => [0.0, 0.0],
        }));
    }
    if rate == 0 {
        return Err("the file does not say its sample rate".into());
    }
    Ok((frames, rate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn peaks_match_the_samples() {
        let frames: Vec<[f32; 2]> =
            (0..10_000).map(|i| [(i as f32 / 10_000.0), -(i as f32 / 20_000.0)]).collect();
        let source = Source::from_frames("ramp", frames.clone());
        assert_eq!(source.peak(10, 20), (-19.0 / 20_000.0, 19.0 / 10_000.0));
        let (lo, hi) = source.peak(1000, 9000);
        assert!(lo <= -8999.0 / 20_000.0 && hi >= 8999.0 / 10_000.0);
        assert!(lo >= -0.5 && hi <= 1.0);
        assert_eq!(source.peak(20_000, 30_000), (0.0, 0.0));
    }

    #[test]
    fn a_wav_file_loads_at_the_project_rate() {
        const PCM: u16 = 1;
        const MONO: u16 = 1;
        let rate = 44_100u32;
        let samples: Vec<i16> = (0..rate)
            .map(|i| ((i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 16_000.0) as i16)
            .collect();
        let path = std::env::temp_dir().join(format!("loupe-test-{}.wav", std::process::id()));
        let mut file = File::create(&path).unwrap();
        let data_len = (samples.len() * 2) as u32;
        file.write_all(b"RIFF").unwrap();
        file.write_all(&(36 + data_len).to_le_bytes()).unwrap();
        file.write_all(b"WAVEfmt ").unwrap();
        file.write_all(&16u32.to_le_bytes()).unwrap();
        file.write_all(&PCM.to_le_bytes()).unwrap();
        file.write_all(&MONO.to_le_bytes()).unwrap();
        file.write_all(&rate.to_le_bytes()).unwrap();
        file.write_all(&(rate * 2).to_le_bytes()).unwrap();
        file.write_all(&2u16.to_le_bytes()).unwrap();
        file.write_all(&16u16.to_le_bytes()).unwrap();
        file.write_all(b"data").unwrap();
        file.write_all(&data_len.to_le_bytes()).unwrap();
        for s in &samples {
            file.write_all(&s.to_le_bytes()).unwrap();
        }
        drop(file);

        let source = Source::load(&path, 48_000);
        std::fs::remove_file(&path).unwrap();
        let source = source.unwrap();
        assert_eq!(source.frames.len(), 48_000);
        let (lo, hi) = source.peak(0, 48_000);
        assert!((hi - 16_000.0 / 32_768.0).abs() < 0.01, "peak {hi}");
        assert!((lo + 16_000.0 / 32_768.0).abs() < 0.01, "trough {lo}");
        assert_eq!(source.frames[1000][0], source.frames[1000][1]);
    }

    #[test]
    fn a_file_that_is_not_audio_is_refused() {
        let path = std::env::temp_dir().join(format!("loupe-test-{}.txt", std::process::id()));
        std::fs::write(&path, "not audio").unwrap();
        let result = Source::load(&path, 48_000);
        std::fs::remove_file(&path).unwrap();
        assert!(result.is_err());
    }
}
