use std::fs::File;
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as DecodeError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

#[derive(Debug, PartialEq)]
pub struct Samples {
    pub rate: f64,
    pub channels: Vec<Vec<f32>>,
}

impl Samples {
    pub fn frames(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }

    pub fn copy_into(&self, channel: usize, from: i64, out: &mut [f32]) {
        out.fill(0.0);
        let Some(source) = self.channels.get(channel) else { return };
        let (skip, start) = if from < 0 { ((-from) as usize, 0usize) } else { (0, from as usize) };
        if skip >= out.len() || start >= source.len() {
            return;
        }
        let count = (out.len() - skip).min(source.len() - start);
        out[skip..skip + count].copy_from_slice(&source[start..start + count]);
    }

    pub fn read(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|why| format!("{} could not be opened: {why}", path.display()))?;
        let stream = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(ending) = path.extension().and_then(|ending| ending.to_str()) {
            hint.with_extension(ending);
        }
        let options = FormatOptions { enable_gapless: true, ..Default::default() };
        let probed = symphonia::default::get_probe()
            .format(&hint, stream, &options, &MetadataOptions::default())
            .map_err(|_| "not an audio format Loupe can read".to_string())?;
        let mut format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or("the file has no audio track")?;
        let track_id = track.id;
        let mut decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(|_| "not an audio codec Loupe can read".to_string())?;
        let mut rate = track.codec_params.sample_rate.unwrap_or(0);
        let mut channels: Vec<Vec<f32>> = Vec::new();
        let mut buffer: Option<SampleBuffer<f32>> = None;
        loop {
            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(DecodeError::IoError(_)) | Err(DecodeError::ResetRequired) => break,
                Err(why) => return Err(why.to_string()),
            };
            if packet.track_id() != track_id {
                continue;
            }
            let decoded = match decoder.decode(&packet) {
                Ok(decoded) => decoded,
                Err(DecodeError::DecodeError(_)) => continue,
                Err(DecodeError::IoError(_)) => break,
                Err(why) => return Err(why.to_string()),
            };
            let spec = *decoded.spec();
            rate = spec.rate;
            let count = spec.channels.count().max(1);
            let kept = count.min(2);
            if channels.len() < kept {
                channels.resize(kept, Vec::new());
            }
            let needed = decoded.capacity() * count;
            if buffer.as_ref().is_none_or(|held| held.capacity() < needed) {
                buffer = Some(SampleBuffer::new(decoded.capacity() as u64, spec));
            }
            let Some(buffer) = buffer.as_mut() else { continue };
            buffer.copy_interleaved_ref(decoded);
            for frame in buffer.samples().chunks_exact(count) {
                for (channel, sample) in channels.iter_mut().zip(frame) {
                    channel.push(*sample);
                }
            }
        }
        if rate == 0 {
            return Err("the file does not say its sample rate".into());
        }
        if channels.first().is_none_or(Vec::is_empty) {
            return Err("the file has no audio in it".into());
        }
        Ok(Self { rate: rate as f64, channels })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn wav(path: &Path, rate: u32, channels: u16, samples: &[i16]) {
        let data = (samples.len() * 2) as u32;
        let mut file = File::create(path).unwrap();
        file.write_all(b"RIFF").unwrap();
        file.write_all(&(36 + data).to_le_bytes()).unwrap();
        file.write_all(b"WAVEfmt ").unwrap();
        file.write_all(&16u32.to_le_bytes()).unwrap();
        file.write_all(&1u16.to_le_bytes()).unwrap();
        file.write_all(&channels.to_le_bytes()).unwrap();
        file.write_all(&rate.to_le_bytes()).unwrap();
        file.write_all(&(rate * channels as u32 * 2).to_le_bytes()).unwrap();
        file.write_all(&(channels * 2).to_le_bytes()).unwrap();
        file.write_all(&16u16.to_le_bytes()).unwrap();
        file.write_all(b"data").unwrap();
        file.write_all(&data.to_le_bytes()).unwrap();
        for sample in samples {
            file.write_all(&sample.to_le_bytes()).unwrap();
        }
    }

    #[test]
    fn a_stereo_file_keeps_its_own_rate_and_both_sides() {
        let path = std::env::temp_dir().join(format!("loupe-ara-stereo-{}.wav", std::process::id()));
        let samples: Vec<i16> = (0..2000).map(|at| if at % 2 == 0 { 16_384 } else { -16_384 }).collect();
        wav(&path, 44_100, 2, &samples);
        let read = Samples::read(&path).expect("it reads");
        let _ = std::fs::remove_file(&path);
        assert_eq!(read.rate, 44_100.0);
        assert_eq!(read.channels.len(), 2);
        assert_eq!(read.frames(), 1000);
        assert!((read.channels[0][10] - 0.5).abs() < 1e-3);
        assert!((read.channels[1][10] + 0.5).abs() < 1e-3);
    }

    #[test]
    fn a_mono_file_stays_one_channel() {
        let path = std::env::temp_dir().join(format!("loupe-ara-mono-{}.wav", std::process::id()));
        wav(&path, 48_000, 1, &[8192; 480]);
        let read = Samples::read(&path).expect("it reads");
        let _ = std::fs::remove_file(&path);
        assert_eq!(read.rate, 48_000.0);
        assert_eq!(read.channels.len(), 1);
        assert_eq!(read.frames(), 480);
    }

    #[test]
    fn reading_past_either_end_gives_silence() {
        let samples = Samples { rate: 48_000.0, channels: vec![vec![1.0, 2.0, 3.0, 4.0]] };
        let mut out = [9.0; 6];
        samples.copy_into(0, -2, &mut out);
        assert_eq!(out, [0.0, 0.0, 1.0, 2.0, 3.0, 4.0]);
        samples.copy_into(0, 3, &mut out);
        assert_eq!(out, [4.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        samples.copy_into(0, 10, &mut out);
        assert_eq!(out, [0.0; 6]);
        samples.copy_into(0, -10, &mut out);
        assert_eq!(out, [0.0; 6]);
        samples.copy_into(1, 0, &mut out);
        assert_eq!(out, [0.0; 6]);
    }

    #[test]
    fn a_missing_file_says_so() {
        assert!(Samples::read(Path::new("there-is-no-such-file.wav")).is_err());
    }
}
