use std::f32::consts::TAU;

use crate::model::Frames;

pub const LOWEST_KEY: u8 = 0;
pub const HIGHEST_KEY: u8 = 127;
const DRUM_TAIL_SECONDS: f32 = 0.9;
const VOICE_LEVEL: f32 = 0.22;
const DRUM_LEVEL: f32 = 0.8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Note {
    pub key: u8,
    pub start: Frames,
    pub len: Frames,
    pub velocity: f32,
}

impl Note {
    pub fn end(&self) -> Frames {
        self.start + self.len
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave {
    Saw,
    Square,
    Triangle,
    Sine,
}

impl Wave {
    pub const ALL: [Wave; 4] = [Wave::Saw, Wave::Square, Wave::Triangle, Wave::Sine];

    pub fn name(self) -> &'static str {
        match self {
            Wave::Saw => "saw",
            Wave::Square => "square",
            Wave::Triangle => "triangle",
            Wave::Sine => "sine",
        }
    }

    pub fn named(name: &str) -> Option<Wave> {
        Wave::ALL.into_iter().find(|wave| wave.name() == name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Synth {
    pub wave: Wave,
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
    pub detune: f32,
}

impl Default for Synth {
    fn default() -> Self {
        Self { wave: Wave::Saw, attack: 0.005, decay: 0.3, sustain: 0.6, release: 0.25, detune: 8.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Instrument {
    Synth(Synth),
    Drums,
}

impl Default for Instrument {
    fn default() -> Self {
        Instrument::Synth(Synth::default())
    }
}

pub fn hertz(key: u8) -> f32 {
    440.0 * 2f32.powf((key as f32 - 69.0) / 12.0)
}

pub fn key_name(key: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", NAMES[key as usize % 12], key as i32 / 12 - 1)
}

pub fn drum_name(key: u8) -> Option<&'static str> {
    Some(match key {
        35 | 36 => "Kick",
        37 => "Rim",
        38 | 40 => "Snare",
        39 => "Clap",
        41 | 43 => "Low tom",
        45 | 47 => "Mid tom",
        48 | 50 => "High tom",
        42 | 44 => "Closed hat",
        46 => "Open hat",
        49 | 57 => "Crash",
        _ => return None,
    })
}

fn noise(frame: u64, seed: u64) -> f32 {
    let mut x = frame.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ seed.wrapping_mul(0xD1B5_4A32_D192_ED03);
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 29;
    (x >> 40) as f32 / (1u64 << 23) as f32 - 1.0
}

fn blep(phase: f32, step: f32) -> f32 {
    if phase < step {
        let t = phase / step;
        t + t - t * t - 1.0
    } else if phase > 1.0 - step {
        let t = (phase - 1.0) / step;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

fn oscillator(wave: Wave, hz: f32, seconds: f32, rate: f32) -> f32 {
    let cycles = hz * seconds;
    let phase = cycles - cycles.floor();
    let step = (hz / rate).min(0.5);
    match wave {
        Wave::Sine => (TAU * phase).sin(),
        Wave::Saw => 2.0 * phase - 1.0 - blep(phase, step),
        Wave::Square => {
            let half = (phase + 0.5) - (phase + 0.5).floor();
            let raw = if phase < 0.5 { 1.0 } else { -1.0 };
            raw + blep(phase, step) - blep(half, step)
        }
        Wave::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
    }
}

impl Synth {
    fn envelope(&self, seconds: f32, held: f32) -> f32 {
        let while_held = |at: f32| {
            if at < self.attack {
                at / self.attack.max(1e-4)
            } else if at < self.attack + self.decay {
                1.0 - (1.0 - self.sustain) * (at - self.attack) / self.decay.max(1e-4)
            } else {
                self.sustain
            }
        };
        if seconds < held {
            while_held(seconds)
        } else {
            let released = (seconds - held) / self.release.max(1e-3);
            while_held(held) * (1.0 - released).max(0.0)
        }
    }

    fn sample(&self, key: u8, seconds: f32, held: f32, rate: f32) -> f32 {
        let level = self.envelope(seconds, held);
        if level <= 0.0 {
            return 0.0;
        }
        let hz = hertz(key);
        let spread = 2f32.powf(self.detune / 1200.0);
        let voice = oscillator(self.wave, hz * spread, seconds, rate) + oscillator(self.wave, hz / spread, seconds + 0.0013, rate);
        voice * 0.5 * level
    }
}

fn drum(key: u8, seconds: f32, frame: u64) -> f32 {
    let hiss = noise(frame, key as u64);
    let bright = hiss - noise(frame.wrapping_sub(1), key as u64);
    match key {
        35 | 36 => {
            let phase = 48.0 * seconds + (110.0 / 32.0) * (1.0 - (-32.0 * seconds).exp());
            (TAU * phase).sin() * (-7.0 * seconds).exp() + 0.15 * hiss * (-120.0 * seconds).exp()
        }
        37 => (TAU * 1700.0 * seconds).sin() * (-60.0 * seconds).exp() * 0.6 + bright * (-80.0 * seconds).exp() * 0.3,
        38 | 40 => 0.45 * (TAU * 190.0 * seconds).sin() * (-25.0 * seconds).exp() + 0.55 * hiss * (-16.0 * seconds).exp(),
        39 => {
            let bursts = [0.0f32, 0.011, 0.022].iter().map(|at| if seconds >= *at { (-(seconds - at) * 90.0).exp() } else { 0.0 }).sum::<f32>();
            0.7 * bright * (bursts + 0.6 * (-seconds * 14.0).exp())
        }
        41 | 43 => (TAU * (95.0 + 40.0 * (-20.0 * seconds).exp()) * seconds).sin() * (-8.0 * seconds).exp(),
        45 | 47 => (TAU * (140.0 + 50.0 * (-20.0 * seconds).exp()) * seconds).sin() * (-9.0 * seconds).exp(),
        48 | 50 => (TAU * (200.0 + 60.0 * (-20.0 * seconds).exp()) * seconds).sin() * (-10.0 * seconds).exp(),
        42 | 44 => 0.45 * bright * (-55.0 * seconds).exp(),
        46 => 0.4 * bright * (-6.0 * seconds).exp(),
        49 | 57 => 0.35 * bright * (-4.0 * seconds).exp(),
        _ => 0.0,
    }
}

impl Instrument {
    pub fn tail(&self, rate: u32) -> Frames {
        let seconds = match self {
            Instrument::Synth(synth) => synth.release,
            Instrument::Drums => DRUM_TAIL_SECONDS,
        };
        (seconds * rate as f32).ceil() as Frames
    }

    pub fn name(&self) -> &'static str {
        match self {
            Instrument::Synth(_) => "Loupe Synth",
            Instrument::Drums => "Loupe Drums",
        }
    }

    pub fn play(&self, note: &Note, since_start: Frames, out: &mut [[f32; 2]], gain: impl Fn(usize) -> f32, rate: u32) {
        let rate_f = rate as f32;
        let held = note.len as f32 / rate_f;
        let level = note.velocity.clamp(0.0, 1.0);
        for (i, frame) in out.iter_mut().enumerate() {
            let at = since_start + i as Frames;
            let seconds = at as f32 / rate_f;
            let value = match self {
                Instrument::Synth(synth) => synth.sample(note.key, seconds, held, rate_f) * VOICE_LEVEL,
                Instrument::Drums => drum(note.key, seconds, at) * DRUM_LEVEL,
            } * level
                * gain(i);
            frame[0] += value;
            frame[1] += value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn rendered(instrument: Instrument, note: Note, frames: usize) -> Vec<[f32; 2]> {
        let mut out = vec![[0.0; 2]; frames];
        instrument.play(&note, 0, &mut out, |_| 1.0, RATE);
        out
    }

    fn rms(audio: &[[f32; 2]]) -> f32 {
        (audio.iter().map(|f| f[0] * f[0]).sum::<f32>() / audio.len().max(1) as f32).sqrt()
    }

    #[test]
    fn keys_and_names_follow_the_midi_standard() {
        assert!((hertz(69) - 440.0).abs() < 1e-3);
        assert!((hertz(60) - 261.6256).abs() < 1e-2);
        assert_eq!(key_name(60), "C4");
        assert_eq!(key_name(61), "C#4");
        assert_eq!(key_name(21), "A0");
        assert_eq!(drum_name(36), Some("Kick"));
        assert_eq!(drum_name(70), None);
    }

    #[test]
    fn a_held_note_sounds_then_releases_to_silence() {
        let synth = Instrument::default();
        let note = Note { key: 57, start: 0, len: RATE as Frames / 2, velocity: 1.0 };
        let out = rendered(synth, note, RATE as usize);
        let held = rms(&out[4_800..20_000]);
        assert!(held > 0.02, "{held}");
        let tail = synth.tail(RATE) as usize;
        assert!(out[24_000 + tail + 10..].iter().all(|f| f[0] == 0.0), "silent once released");
        assert!(out.iter().flatten().all(|s| s.abs() <= 1.0));
    }

    #[test]
    fn rendering_in_pieces_matches_rendering_at_once() {
        let synth = Instrument::default();
        let note = Note { key: 64, start: 0, len: 9_000, velocity: 0.8 };
        let whole = rendered(synth, note, 30_000);
        let mut pieces = vec![[0.0f32; 2]; 30_000];
        for (n, part) in pieces.chunks_mut(777).enumerate() {
            synth.play(&note, (n * 777) as Frames, part, |_| 1.0, RATE);
        }
        assert_eq!(whole, pieces);
    }

    #[test]
    fn velocity_scales_the_level() {
        let synth = Instrument::default();
        let loud = rendered(synth, Note { key: 60, start: 0, len: 10_000, velocity: 1.0 }, 10_000);
        let soft = rendered(synth, Note { key: 60, start: 0, len: 10_000, velocity: 0.5 }, 10_000);
        assert!((rms(&soft) / rms(&loud) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn every_wave_has_its_pitch() {
        for wave in Wave::ALL {
            let synth = Instrument::Synth(Synth { wave, detune: 0.0, ..Synth::default() });
            let out = rendered(synth, Note { key: 69, start: 0, len: RATE as Frames, velocity: 1.0 }, RATE as usize);
            let part = &out[10_000..20_000];
            let likeness = |lag: usize| part.iter().zip(&part[lag..]).map(|(a, b)| a[0] * b[0]).sum::<f32>();
            let period = (80..140).max_by(|a, b| likeness(*a).total_cmp(&likeness(*b))).unwrap();
            assert!((period as i32 - 109).abs() <= 1, "{}: repeats every {period} frames", wave.name());
            assert_eq!(Wave::named(wave.name()), Some(wave));
        }
    }

    #[test]
    fn every_drum_hits_and_fades_within_its_tail() {
        let kit = Instrument::Drums;
        for key in [36, 37, 38, 39, 42, 45, 46, 49] {
            let out = rendered(kit, Note { key, start: 0, len: 100, velocity: 1.0 }, (RATE as f32 * 1.5) as usize);
            let hit = rms(&out[..2_400]);
            let after = rms(&out[kit.tail(RATE) as usize..]);
            assert!(hit > 0.02, "{key}: {hit}");
            assert!(after < hit * 0.05, "{key} still rings: {after} after {hit}");
        }
        let silent = rendered(kit, Note { key: 70, start: 0, len: 100, velocity: 1.0 }, 4_800);
        assert!(silent.iter().all(|f| f[0] == 0.0));
    }
}
