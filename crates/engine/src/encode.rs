use std::fmt;
use std::fs::File;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

use flacenc::bitsink::ByteSink;
use flacenc::component::{BitRepr, Stream, StreamInfo};
use flacenc::error::{Verified, Verify};
use flacenc::source::{Context, Fill, FrameBuf};

use crate::wav;

const CHANNELS: u16 = 2;
const FLAC_BLOCK: usize = 4096;
const FLAC_HEADER_BYTES: usize = 42;
const MP3_KBPS: u32 = 320;
const MP3_HIGHEST_RATE: u32 = 48_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    WavFloat,
    Wav24,
    Wav16,
    Flac24,
    Flac16,
    Mp3Cbr320,
}

impl Format {
    pub const ALL: [Format; 6] = [Format::WavFloat, Format::Wav24, Format::Wav16, Format::Flac24, Format::Flac16, Format::Mp3Cbr320];

    pub fn key(self) -> &'static str {
        match self {
            Format::WavFloat => "wav_float",
            Format::Wav24 => "wav_24",
            Format::Wav16 => "wav_16",
            Format::Flac24 => "flac_24",
            Format::Flac16 => "flac_16",
            Format::Mp3Cbr320 => "mp3_320",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|format| format.key() == key.trim())
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::WavFloat | Format::Wav24 | Format::Wav16 => "wav",
            Format::Flac24 | Format::Flac16 => "flac",
            Format::Mp3Cbr320 => "mp3",
        }
    }

    pub fn bits(self) -> Option<u16> {
        match self {
            Format::Wav24 | Format::Flac24 => Some(24),
            Format::Wav16 | Format::Flac16 => Some(16),
            Format::WavFloat | Format::Mp3Cbr320 => None,
        }
    }

    pub fn mp3_rate(rate: u32) -> u32 {
        if rate <= MP3_HIGHEST_RATE {
            rate
        } else if rate % 44_100 == 0 {
            44_100
        } else {
            MP3_HIGHEST_RATE
        }
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Format::WavFloat => "WAV, 32 bit float",
            Format::Wav24 => "WAV, 24 bit",
            Format::Wav16 => "WAV, 16 bit",
            Format::Flac24 => "FLAC, 24 bit",
            Format::Flac16 => "FLAC, 16 bit",
            Format::Mp3Cbr320 => "MP3, 320 kbps",
        })
    }
}

pub struct Quantiser {
    full_scale: f64,
    dither: bool,
    state: u32,
}

impl Quantiser {
    pub fn new(bits: u16, dither: bool) -> Self {
        Self { full_scale: (1u64 << (bits - 1)) as f64, dither, state: 0x9E37_79B9 }
    }

    fn uniform(&mut self) -> f64 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 17;
        self.state ^= self.state << 5;
        (self.state >> 8) as f64 / (1u32 << 24) as f64
    }

    pub fn sample(&mut self, value: f32) -> i32 {
        let noise = if self.dither { self.uniform() - self.uniform() } else { 0.0 };
        (value as f64 * self.full_scale + noise).round().clamp(-self.full_scale, self.full_scale - 1.0) as i32
    }
}

pub struct Writer {
    kind: Kind,
}

enum Kind {
    Float(BufWriter<File>),
    Pcm { out: BufWriter<File>, quantiser: Quantiser, bytes: usize },
    Flac(Box<Flac>),
    Mp3(Box<Mp3>),
}

struct Flac {
    out: BufWriter<File>,
    config: Verified<flacenc::config::Encoder>,
    info: StreamInfo,
    held: (FrameBuf, Context),
    pending: Vec<i32>,
    quantiser: Quantiser,
}

struct Mp3 {
    out: BufWriter<File>,
    encoder: loupe_lame::Encoder,
    interleaved: Vec<f32>,
    encoded: Vec<u8>,
}

fn failed(why: impl fmt::Display) -> io::Error {
    io::Error::other(why.to_string())
}

fn too_long() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "the song is too long for one WAV file")
}

impl Writer {
    pub fn create(path: &Path, format: Format, rate: u32, frames: u64, dither: bool) -> io::Result<Self> {
        let kind = match format {
            Format::WavFloat => {
                if frames > wav::most_frames(CHANNELS) {
                    return Err(too_long());
                }
                let mut out = BufWriter::new(File::create(path)?);
                out.write_all(&wav::float_header(CHANNELS, rate, frames as u32))?;
                Kind::Float(out)
            }
            Format::Wav24 | Format::Wav16 => {
                let bits = format.bits().unwrap_or(16);
                if frames > wav::most_pcm_frames(CHANNELS, bits) {
                    return Err(too_long());
                }
                let mut out = BufWriter::new(File::create(path)?);
                out.write_all(&wav::pcm_header(CHANNELS, rate, bits, frames as u32))?;
                Kind::Pcm { out, quantiser: Quantiser::new(bits, dither), bytes: bits as usize / 8 }
            }
            Format::Flac24 | Format::Flac16 => Kind::Flac(Box::new(Flac::create(path, rate, format.bits().unwrap_or(16), dither)?)),
            Format::Mp3Cbr320 => Kind::Mp3(Box::new(Mp3::create(path, rate)?)),
        };
        Ok(Self { kind })
    }

    pub fn push(&mut self, block: &[[f32; 2]]) -> io::Result<()> {
        match &mut self.kind {
            Kind::Float(out) => {
                for frame in block {
                    out.write_all(&frame[0].to_le_bytes())?;
                    out.write_all(&frame[1].to_le_bytes())?;
                }
                Ok(())
            }
            Kind::Pcm { out, quantiser, bytes } => {
                for frame in block {
                    for value in frame {
                        out.write_all(&quantiser.sample(*value).to_le_bytes()[..*bytes])?;
                    }
                }
                Ok(())
            }
            Kind::Flac(flac) => flac.push(block),
            Kind::Mp3(mp3) => mp3.push(block),
        }
    }

    pub fn finish(self) -> io::Result<()> {
        match self.kind {
            Kind::Float(mut out) | Kind::Pcm { mut out, .. } => out.flush(),
            Kind::Flac(flac) => flac.finish(),
            Kind::Mp3(mp3) => mp3.finish(),
        }
    }
}

impl Flac {
    fn create(path: &Path, rate: u32, bits: u16, dither: bool) -> io::Result<Self> {
        let config = flacenc::config::Encoder::default().into_verified().map_err(|(_, why)| failed(why))?;
        let mut info = StreamInfo::new(rate as usize, CHANNELS as usize, bits as usize).map_err(failed)?;
        info.set_block_sizes(FLAC_BLOCK, FLAC_BLOCK).map_err(failed)?;
        let held = (FrameBuf::with_size(CHANNELS as usize, FLAC_BLOCK).map_err(failed)?, Context::new(bits as usize, CHANNELS as usize));
        let mut out = BufWriter::new(File::create(path)?);
        out.write_all(&[0; FLAC_HEADER_BYTES])?;
        Ok(Self {
            out,
            config,
            info,
            held,
            pending: Vec::with_capacity(FLAC_BLOCK * CHANNELS as usize),
            quantiser: Quantiser::new(bits, dither),
        })
    }

    fn push(&mut self, block: &[[f32; 2]]) -> io::Result<()> {
        for frame in block {
            self.pending.push(self.quantiser.sample(frame[0]));
            self.pending.push(self.quantiser.sample(frame[1]));
            if self.pending.len() == FLAC_BLOCK * CHANNELS as usize {
                self.encode_pending()?;
            }
        }
        Ok(())
    }

    fn encode_pending(&mut self) -> io::Result<()> {
        self.held.fill_interleaved(&self.pending).map_err(failed)?;
        self.pending.clear();
        let number = self.held.1.current_frame_number().unwrap_or(0);
        let frame = flacenc::encode_fixed_size_frame(&self.config, &self.held.0, number, &self.info).map_err(failed)?;
        self.info.update_frame_info(&frame);
        let mut sink = ByteSink::new();
        frame.write(&mut sink).map_err(failed)?;
        self.out.write_all(sink.as_slice())
    }

    fn finish(mut self) -> io::Result<()> {
        if !self.pending.is_empty() {
            self.encode_pending()?;
        }
        self.info.set_md5_digest(&self.held.1.md5_digest());
        self.info.set_total_samples(self.held.1.total_samples());
        self.info.set_block_sizes(FLAC_BLOCK, FLAC_BLOCK).map_err(failed)?;
        let mut sink = ByteSink::new();
        Stream::with_stream_info(self.info).write(&mut sink).map_err(failed)?;
        if sink.as_slice().len() != FLAC_HEADER_BYTES {
            return Err(failed("the FLAC header came out the wrong size"));
        }
        self.out.flush()?;
        let file = self.out.get_mut();
        file.seek(SeekFrom::Start(0))?;
        file.write_all(sink.as_slice())?;
        file.flush()
    }
}

impl Mp3 {
    fn create(path: &Path, rate: u32) -> io::Result<Self> {
        let encoder = loupe_lame::Encoder::constant(rate, Format::mp3_rate(rate), MP3_KBPS).map_err(failed)?;
        Ok(Self { out: BufWriter::new(File::create(path)?), encoder, interleaved: Vec::new(), encoded: Vec::new() })
    }

    fn push(&mut self, block: &[[f32; 2]]) -> io::Result<()> {
        self.interleaved.clear();
        self.interleaved.extend(block.iter().flatten());
        self.encoded.clear();
        self.encoder.encode(&self.interleaved, &mut self.encoded).map_err(failed)?;
        self.out.write_all(&self.encoded)
    }

    fn finish(mut self) -> io::Result<()> {
        self.encoded.clear();
        self.encoder.flush(&mut self.encoded).map_err(failed)?;
        self.out.write_all(&self.encoded)?;
        self.out.flush()?;
        let tag = self.encoder.tag();
        if !tag.is_empty() {
            let file = self.out.get_mut();
            file.seek(SeekFrom::Start(0))?;
            file.write_all(&tag)?;
            file.flush()?;
        }
        Ok(())
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Source;
    use std::f32::consts::TAU;
    use std::path::PathBuf;

    const RATE: u32 = 48_000;
    const LSB_16: f32 = 1.0 / 32_768.0;

    fn tone(frames: usize, level: f32) -> Vec<[f32; 2]> {
        (0..frames)
            .map(|i| {
                let s = level * (TAU * 997.0 * i as f32 / RATE as f32).sin();
                [s, -0.5 * s]
            })
            .collect()
    }

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("loupe-encode-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn write(path: &Path, format: Format, dither: bool, audio: &[[f32; 2]]) {
        let mut writer = Writer::create(path, format, RATE, audio.len() as u64, dither).unwrap();
        for block in audio.chunks(1000) {
            writer.push(block).unwrap();
        }
        writer.finish().unwrap();
    }

    fn worst_difference(a: &[[f32; 2]], b: &[[f32; 2]]) -> f32 {
        a.iter().zip(b).map(|(x, y)| (x[0] - y[0]).abs().max((x[1] - y[1]).abs())).fold(0.0, f32::max)
    }

    fn rms(audio: &[[f32; 2]]) -> f32 {
        (audio.iter().map(|f| f[0] * f[0]).sum::<f32>() / audio.len() as f32).sqrt()
    }

    #[test]
    fn without_dither_samples_round_to_the_nearest_step_and_never_wrap() {
        let mut quantiser = Quantiser::new(16, false);
        assert_eq!(quantiser.sample(0.0), 0);
        assert_eq!(quantiser.sample(0.4 * LSB_16), 0);
        assert_eq!(quantiser.sample(0.6 * LSB_16), 1);
        assert_eq!(quantiser.sample(-1.0), -32_768);
        assert_eq!(quantiser.sample(1.0), 32_767);
        assert_eq!(quantiser.sample(1.7), 32_767);
        assert_eq!(quantiser.sample(-3.0), -32_768);
        let mut deep = Quantiser::new(24, false);
        assert_eq!(deep.sample(1.0), 8_388_607);
        assert_eq!(deep.sample(-1.0), -8_388_608);
    }

    #[test]
    fn tpdf_dither_is_unbiased_with_a_quarter_step_squared_of_error() {
        let mut quantiser = Quantiser::new(16, true);
        let wanted = 0.3f64;
        let count = 400_000;
        let (mut sum, mut squares) = (0.0f64, 0.0f64);
        for _ in 0..count {
            let got = quantiser.sample((wanted / 32_768.0) as f32);
            assert!((-1..=2).contains(&got), "dither reached {got}");
            let error = got as f64 - wanted;
            sum += error;
            squares += error * error;
        }
        let mean = sum / count as f64;
        let variance = squares / count as f64 - mean * mean;
        assert!(mean.abs() < 0.01, "mean error {mean}");
        assert!((variance - 0.25).abs() < 0.02, "error variance {variance}");
    }

    #[test]
    fn dither_never_clips_or_wraps_at_full_scale() {
        for bits in [16, 24] {
            let mut quantiser = Quantiser::new(bits, true);
            let top = (1i32 << (bits - 1)) - 1;
            for _ in 0..100_000 {
                let high = quantiser.sample(1.0);
                let low = quantiser.sample(-1.0);
                assert!(high <= top && high >= top - 2, "{bits} bit high {high}");
                assert!(low >= -top - 1 && low <= -top + 1, "{bits} bit low {low}");
            }
        }
    }

    #[test]
    fn silence_stays_silent_without_dither_and_quiet_with_it() {
        let mut plain = Quantiser::new(16, false);
        assert!((0..1000).all(|_| plain.sample(0.0) == 0));
        let mut dithered = Quantiser::new(16, true);
        assert!((0..1000).all(|_| dithered.sample(0.0).abs() <= 1));
    }

    #[test]
    fn every_format_writes_a_file_that_decodes_back() {
        let folder = scratch("formats");
        let audio = tone(RATE as usize * 2, 0.5);
        for format in Format::ALL {
            let path = folder.join(format!("{}.{}", format.key(), format.extension()));
            write(&path, format, false, &audio);
            let read = Source::load(&path, RATE).unwrap();
            let allowed = match format {
                Format::WavFloat => 0.0,
                Format::Wav24 | Format::Flac24 => 1.0 / 8_388_608.0,
                Format::Wav16 | Format::Flac16 => LSB_16,
                Format::Mp3Cbr320 => f32::MAX,
            };
            if format.extension() == "mp3" {
                let frames = read.frames.len() as i64;
                assert!((frames - audio.len() as i64).abs() <= 1152, "{format}: {frames} frames");
                let level = 20.0 * (rms(&read.frames) / rms(&audio)).log10();
                assert!(level.abs() < 0.1, "{format}: level moved {level} dB");
            } else {
                assert_eq!(read.frames.len(), audio.len(), "{format}");
                let worst = worst_difference(&read.frames, &audio);
                assert!(worst <= allowed, "{format}: off by {worst}");
            }
        }
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn flac_holds_exactly_the_samples_a_wav_of_the_same_depth_holds() {
        let folder = scratch("lossless");
        let audio = tone(10_000, 0.9);
        for (wav, flac) in [(Format::Wav16, Format::Flac16), (Format::Wav24, Format::Flac24)] {
            write(&folder.join("a.wav"), wav, true, &audio);
            write(&folder.join("a.flac"), flac, true, &audio);
            let from_wav = Source::load(&folder.join("a.wav"), RATE).unwrap();
            let from_flac = Source::load(&folder.join("a.flac"), RATE).unwrap();
            assert_eq!(from_wav.frames, from_flac.frames, "{flac}");
        }
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn mp3_above_48k_is_written_at_half_the_rate() {
        assert_eq!(Format::mp3_rate(44_100), 44_100);
        assert_eq!(Format::mp3_rate(48_000), 48_000);
        assert_eq!(Format::mp3_rate(88_200), 44_100);
        assert_eq!(Format::mp3_rate(96_000), 48_000);
        let folder = scratch("mp3rate");
        let path = folder.join("high.mp3");
        let audio: Vec<[f32; 2]> = tone(96_000, 0.5);
        let mut writer = Writer::create(&path, Format::Mp3Cbr320, 96_000, audio.len() as u64, false).unwrap();
        writer.push(&audio).unwrap();
        writer.finish().unwrap();
        let read = Source::load(&path, 96_000).unwrap();
        assert!((read.frames.len() as i64 - 96_000).abs() <= 2400, "{} frames", read.frames.len());
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn format_keys_survive_a_round_trip() {
        for format in Format::ALL {
            assert_eq!(Format::from_key(format.key()), Some(format));
        }
        assert_eq!(Format::from_key("ogg"), None);
    }
}
