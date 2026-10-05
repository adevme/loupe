use std::f32::consts::{PI, TAU};

use crate::smooth::Smoothed;
use crate::{settled, Effect, Frame, Param, Unit, Values, DEFAULT_RATE};

pub const MODES: [&str; 2] = ["Chorus", "Flanger"];
const PARAMS: [Param; 6] = [
    Param::choice("mode", "Mode", &MODES, 0),
    Param::new("rate", "Rate", 0.05, 10.0, 0.8, Unit::Hertz),
    Param::new("depth", "Depth", 0.0, 100.0, 50.0, Unit::Percent),
    Param::new("feedback", "Feedback", 0.0, 95.0, 0.0, Unit::Percent),
    Param::new("width", "Width", 0.0, 200.0, 100.0, Unit::Percent),
    Param::new("mix", "Mix", 0.0, 100.0, 50.0, Unit::Percent),
];
pub const MODE: usize = 0;
pub const RATE: usize = 1;
pub const DEPTH: usize = 2;
pub const FEEDBACK: usize = 3;
pub const WIDTH: usize = 4;
pub const MIX: usize = 5;
const CHORUS_BASE_MS: f32 = 14.0;
const CHORUS_SWEEP_MS: f32 = 8.0;
const FLANGER_BASE_MS: f32 = 2.6;
const FLANGER_SWEEP_MS: f32 = 2.4;
const VOICES: usize = 2;
const LONGEST_MS: f32 = 30.0;

pub fn sweep(values: &[f32]) -> (f32, f32) {
    let flanger = values.get(MODE).copied().unwrap_or(0.0) > 0.5;
    let depth = values.get(DEPTH).copied().unwrap_or(0.0) / 100.0;
    if flanger {
        (FLANGER_BASE_MS, FLANGER_SWEEP_MS * depth)
    } else {
        (CHORUS_BASE_MS, CHORUS_SWEEP_MS * depth)
    }
}

pub fn spread(values: &[f32]) -> f32 {
    values.get(WIDTH).copied().unwrap_or(100.0) / 100.0 * PI / 4.0
}

pub struct Chorus {
    values: Values<6>,
    rate: f32,
    flanger: bool,
    step: f32,
    phase: f32,
    base: Smoothed,
    swing: Smoothed,
    feedback: Smoothed,
    width: Smoothed,
    mix: Smoothed,
    spread: f32,
    lines: [Vec<f32>; 2],
    written: usize,
    fed: Frame,
}

impl Chorus {
    pub fn new() -> Self {
        let mut chorus = Self {
            values: Values::new(&PARAMS),
            rate: DEFAULT_RATE,
            flanger: false,
            step: 0.0,
            phase: 0.0,
            base: Smoothed::new(0.0),
            swing: Smoothed::new(0.0),
            feedback: Smoothed::new(0.0),
            width: Smoothed::new(1.0),
            mix: Smoothed::new(0.5),
            spread: 0.0,
            lines: [Vec::new(), Vec::new()],
            written: 0,
            fed: [0.0; 2],
        };
        chorus.prepare(DEFAULT_RATE);
        chorus
    }

    fn read_knobs(&mut self) {
        let values: [f32; 6] = std::array::from_fn(|i| self.values.get(i));
        self.flanger = values[MODE] > 0.5;
        self.step = values[RATE] / self.rate;
        let (base, swing) = sweep(&values);
        let per_ms = self.rate / 1000.0;
        self.base.aim(base * per_ms);
        self.swing.aim(swing * per_ms);
        self.feedback.aim(values[FEEDBACK] / 100.0);
        self.width.aim(values[WIDTH] / 100.0);
        self.mix.aim(values[MIX] / 100.0);
        self.spread = spread(&values);
    }

    fn read(&self, channel: usize, behind: f32) -> f32 {
        let line = &self.lines[channel];
        let size = line.len();
        let whole = behind.floor();
        let part = behind - whole;
        let at = |back: usize| line[(self.written + size - back % size) % size];
        let older = whole as usize;
        let (a, b, c, d) = (at(older - 1), at(older), at(older + 1), at(older + 2));
        let (c0, c1, c2, c3) = (b, 0.5 * (c - a), a - 2.5 * b + 2.0 * c - 0.5 * d, 0.5 * (d - a) + 1.5 * (b - c));
        ((c3 * part + c2) * part + c1) * part + c0
    }
}

impl Default for Chorus {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for Chorus {
    fn name(&self) -> &'static str {
        "Loupe Chorus"
    }

    fn keeps_its_own_clock(&self) -> bool {
        true
    }

    fn params(&self) -> &'static [Param] {
        &PARAMS
    }

    fn value(&self, index: usize) -> f32 {
        self.values.get(index)
    }

    fn set(&mut self, index: usize, value: f32) {
        self.values.set(index, value);
    }

    fn prepare(&mut self, rate: f32) {
        self.rate = rate;
        let size = (LONGEST_MS * rate / 1000.0) as usize + 8;
        self.lines = [vec![0.0; size], vec![0.0; size]];
        for smoothed in [&mut self.base, &mut self.swing, &mut self.feedback, &mut self.width, &mut self.mix] {
            smoothed.prepare(rate);
        }
        self.read_knobs();
        self.values.take_change();
        for smoothed in [&mut self.base, &mut self.swing, &mut self.feedback, &mut self.width, &mut self.mix] {
            smoothed.snap();
        }
        self.reset();
    }

    fn reset(&mut self) {
        for line in &mut self.lines {
            line.fill(0.0);
        }
        self.written = 0;
        self.phase = 0.0;
        self.fed = [0.0; 2];
    }

    fn process(&mut self, audio: &mut [Frame]) {
        if self.values.take_change() {
            self.read_knobs();
        }
        let size = self.lines[0].len();
        let voices = if self.flanger { 1 } else { VOICES };
        for frame in audio.iter_mut() {
            let base = self.base.next();
            let swing = self.swing.next();
            let feedback = self.feedback.next();
            let width = self.width.next();
            let mix = self.mix.next();
            let dry = *frame;
            for channel in 0..2 {
                self.lines[channel][self.written] = dry[channel] + self.fed[channel] * feedback;
            }
            let mut wet = [0.0f32; 2];
            for (channel, out) in wet.iter_mut().enumerate() {
                let offset = if channel == 0 { 0.0 } else { self.spread };
                let mut sum = 0.0;
                for voice in 0..voices {
                    let angle = TAU * self.phase + offset + voice as f32 * PI;
                    let behind = (base + swing * 0.5 * (1.0 + angle.sin())).clamp(1.0, size as f32 - 4.0);
                    sum += self.read(channel, behind);
                }
                *out = sum / voices as f32;
            }
            self.fed = [settled(wet[0]), settled(wet[1])];
            let middle = (wet[0] + wet[1]) * 0.5;
            let side = (wet[0] - wet[1]) * 0.5 * width;
            let wide = [middle + side, middle - side];
            for channel in 0..2 {
                frame[channel] = dry[channel] + (wide[channel] - dry[channel]) * mix;
            }
            self.written = (self.written + 1) % size;
            self.phase += self.step;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{in_blocks, noise, peak, rms, sine};

    fn through(chorus: &mut Chorus, audio: &[Frame]) -> Vec<Frame> {
        let mut out = audio.to_vec();
        in_blocks(chorus, &mut out, 256);
        out
    }

    #[test]
    fn no_mix_leaves_the_sound_alone() {
        let mut chorus = Chorus::new();
        chorus.set(MIX, 0.0);
        chorus.prepare(48_000.0);
        let input = noise(0.5, 9_600, 3);
        assert_eq!(through(&mut chorus, &input), input);
    }

    #[test]
    fn a_mono_sound_comes_out_wide_unless_width_is_zero() {
        let rate = 48_000.0;
        let input = sine(rate, 440.0, 0.5, 48_000);
        let difference = |width: f32| {
            let mut chorus = Chorus::new();
            chorus.set(WIDTH, width);
            chorus.set(MIX, 100.0);
            chorus.prepare(rate);
            let out = through(&mut chorus, &input);
            out[24_000..].iter().map(|f| (f[0] - f[1]).abs()).fold(0.0f32, f32::max)
        };
        assert!(difference(0.0) < 1e-5, "width 0 still differs by {}", difference(0.0));
        assert!(difference(100.0) > 0.05);
        assert!(difference(200.0) > difference(100.0));
    }

    #[test]
    fn the_wet_sound_moves_over_time() {
        let rate = 48_000.0;
        let mut chorus = Chorus::new();
        chorus.set(MIX, 100.0);
        chorus.set(DEPTH, 100.0);
        chorus.set(RATE, 2.0);
        chorus.prepare(rate);
        let input = sine(rate, 220.0, 0.5, 48_000);
        let out = through(&mut chorus, &input);
        let windows: Vec<f32> = out[4_800..].chunks(960).map(rms).collect();
        let (low, high) = windows.iter().fold((f32::MAX, 0.0f32), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
        assert!(high - low > 0.01 || windows.len() < 2, "no movement: {low}..{high}");
    }

    #[test]
    fn a_flanger_with_full_feedback_stays_bounded() {
        let rate = 44_100.0;
        let mut chorus = Chorus::new();
        chorus.set(MODE, 1.0);
        chorus.set(FEEDBACK, 95.0);
        chorus.set(MIX, 100.0);
        chorus.prepare(rate);
        let out = through(&mut chorus, &noise(0.5, 88_200, 11));
        assert!(out.iter().flatten().all(|v| v.is_finite()));
        assert!(peak(&out) < 20.0, "{}", peak(&out));
    }
}
