use crate::biquad::{Biquad, Coefficients, Shape, BUTTERWORTH};
use crate::smooth::Smoothed;
use crate::{settled, Effect, Frame, Param, Unit, Values, DEFAULT_RATE};

const PARAMS: [Param; 9] = [
    Param::new("pre_delay", "Pre delay", 0.0, 200.0, 10.0, Unit::Milliseconds),
    Param::new("size", "Size", 0.0, 100.0, 50.0, Unit::Percent),
    Param::new("decay", "Decay", 0.2, 20.0, 2.0, Unit::Seconds),
    Param::new("damping", "Damping", 1000.0, 20_000.0, 7000.0, Unit::Hertz),
    Param::new("low_cut", "Low cut", 20.0, 1000.0, 150.0, Unit::Hertz),
    Param::new("width", "Width", 0.0, 100.0, 100.0, Unit::Width),
    Param::new("mix", "Mix", 0.0, 100.0, 25.0, Unit::Percent),
    Param::new("modulation", "Modulation", 0.0, 100.0, 25.0, Unit::Percent),
    Param::new("bass_decay", "Bass decay", 0.25, 4.0, 1.0, Unit::Ratio),
];
const LINES: usize = 8;
const LINE_MS: [f32; LINES] = [31.7, 37.9, 41.3, 47.1, 53.3, 59.9, 67.1, 73.7];
const SMALLEST_SIZE: f32 = 0.4;
const SIZE_SPAN: f32 = 1.6;
const DIFFUSERS: usize = 3;
const DIFFUSER_MS: [f32; DIFFUSERS] = [4.7, 3.6, 2.3];
const DIFFUSER_SPREAD: f32 = 1.13;
const DIFFUSION: f32 = 0.6;
const LONGEST_PRE_DELAY: f32 = 0.2;
const INTO_LINES: f32 = 0.35;
const OUT_OF_LINES: f32 = 0.3;
const WOBBLE_MS: f32 = 0.8;
const WOBBLE_HZ: [f32; LINES] = [0.13, 0.19, 0.23, 0.29, 0.31, 0.37, 0.41, 0.47];
const BASS_SPLIT_HZ: f32 = 250.0;
pub const DECAY: usize = 2;
pub const DAMPING: usize = 3;
pub const LOW_CUT: usize = 4;
pub const BASS_DECAY: usize = 8;

struct Ring {
    samples: Vec<f32>,
    write: usize,
}

impl Ring {
    fn new(frames: usize) -> Self {
        Self { samples: vec![0.0; frames.max(2)], write: 0 }
    }

    fn push(&mut self, value: f32) {
        self.write = (self.write + 1) % self.samples.len();
        self.samples[self.write] = value;
    }

    fn back(&self, frames: usize) -> f32 {
        let length = self.samples.len();
        self.samples[(self.write + length - frames.min(length - 1)) % length]
    }

    fn back_between(&self, frames: f32) -> f32 {
        let whole = frames.floor();
        let a = self.back(whole as usize);
        let b = self.back(whole as usize + 1);
        a + (b - a) * (frames - whole)
    }

    fn clear(&mut self) {
        self.samples.fill(0.0);
        self.write = 0;
    }
}

struct Diffuser {
    ring: Ring,
    delay: usize,
}

impl Diffuser {
    fn run(&mut self, x: f32) -> f32 {
        let held = self.ring.back(self.delay - 1);
        let into = x + DIFFUSION * held;
        self.ring.push(settled(into));
        held - DIFFUSION * into
    }
}

pub struct Reverb {
    values: Values<9>,
    rate: f32,
    pre_delay: Ring,
    pre_delay_frames: usize,
    diffusers: [Vec<Diffuser>; 2],
    lines: Vec<Ring>,
    lengths: [Smoothed; LINES],
    keep: [f32; LINES],
    keep_low: [f32; LINES],
    damp: f32,
    dampers: [f32; LINES],
    lows: [f32; LINES],
    split: f32,
    wobble: f32,
    turns: [f32; LINES],
    turn_steps: [f32; LINES],
    low_cut: Coefficients,
    low_cut_filter: Biquad,
    width: Smoothed,
    mix: Smoothed,
}

impl Reverb {
    pub fn new() -> Self {
        let mut reverb = Self {
            values: Values::new(&PARAMS),
            rate: DEFAULT_RATE,
            pre_delay: Ring::new(2),
            pre_delay_frames: 0,
            diffusers: [Vec::new(), Vec::new()],
            lines: Vec::new(),
            lengths: [Smoothed::new(1.0); LINES],
            keep: [0.0; LINES],
            keep_low: [0.0; LINES],
            damp: 0.0,
            dampers: [0.0; LINES],
            lows: [0.0; LINES],
            split: 0.0,
            wobble: 0.0,
            turns: [0.0; LINES],
            turn_steps: [0.0; LINES],
            low_cut: Coefficients::PASS,
            low_cut_filter: Biquad::default(),
            width: Smoothed::new(1.0),
            mix: Smoothed::new(0.0),
        };
        reverb.prepare(DEFAULT_RATE);
        reverb
    }

    fn read_knobs(&mut self) {
        let frames_per_ms = self.rate * 0.001;
        self.pre_delay_frames = (self.values.get(0) * frames_per_ms).round() as usize;
        let size = SMALLEST_SIZE + SIZE_SPAN * self.values.get(1) / 100.0;
        let decay = self.values.get(2);
        for (line, ms) in LINE_MS.iter().enumerate() {
            let length = ms * size * frames_per_ms;
            self.lengths[line].aim(length);
            self.keep[line] = 10f32.powf(-3.0 * length / (decay * self.rate));
            self.keep_low[line] = 10f32.powf(-3.0 * length / (decay * self.values.get(BASS_DECAY) * self.rate));
            self.turn_steps[line] = std::f32::consts::TAU * WOBBLE_HZ[line] / self.rate;
        }
        self.split = (-std::f32::consts::TAU * BASS_SPLIT_HZ / self.rate).exp();
        self.wobble = self.values.get(7) / 100.0 * WOBBLE_MS * frames_per_ms;
        self.damp = (-std::f32::consts::TAU * self.values.get(3) / self.rate).exp();
        self.low_cut = Coefficients::design(Shape::LowCut, self.rate, self.values.get(4), BUTTERWORTH, 0.0);
        self.width.aim(self.values.get(5) / 100.0);
        self.mix.aim(self.values.get(6) / 100.0);
    }
}

pub fn decay_seconds(values: &[f32], hz: f32) -> f32 {
    let rate = DEFAULT_RATE;
    let w = std::f32::consts::TAU * hz / rate;
    let one_pole = |pole: f32| (1.0 - pole) / (1.0 + pole * pole - 2.0 * pole * w.cos()).sqrt();
    let damping = one_pole((-std::f32::consts::TAU * values[DAMPING] / rate).exp());
    let low_share = one_pole((-std::f32::consts::TAU * BASS_SPLIT_HZ / rate).exp());
    let decay = values[DECAY];
    let length = 0.05 * rate;
    let keep = 10f32.powf(-3.0 * length / (decay * rate));
    let keep_low = 10f32.powf(-3.0 * length / (decay * values[BASS_DECAY] * rate));
    let per_pass = damping * (keep_low * low_share + keep * (1.0 - low_share));
    let seconds = -3.0 * length / (rate * per_pass.max(1e-9).log10());
    let cut = values[LOW_CUT];
    let below_cut = if hz < cut { (hz / cut).powi(2) } else { 1.0 };
    seconds * below_cut
}

impl Default for Reverb {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for Reverb {
    fn name(&self) -> &'static str {
        "Loupe Reverb"
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
        let frames_per_ms = rate * 0.001;
        self.pre_delay = Ring::new((LONGEST_PRE_DELAY * rate) as usize + 2);
        self.diffusers = [0, 1].map(|side| {
            DIFFUSER_MS
                .iter()
                .map(|ms| {
                    let spread = if side == 0 { 1.0 } else { DIFFUSER_SPREAD };
                    let delay = ((ms * spread * frames_per_ms) as usize).max(1);
                    Diffuser { ring: Ring::new(delay + 1), delay }
                })
                .collect()
        });
        let longest = SMALLEST_SIZE + SIZE_SPAN;
        let spare = (WOBBLE_MS * frames_per_ms) as usize + 4;
        self.lines = LINE_MS.iter().map(|ms| Ring::new((ms * longest * frames_per_ms) as usize + spare)).collect();
        for smoothed in self.lengths.iter_mut().chain([&mut self.width, &mut self.mix]) {
            smoothed.prepare(rate);
        }
        self.read_knobs();
        self.values.take_change();
        for smoothed in self.lengths.iter_mut().chain([&mut self.width, &mut self.mix]) {
            smoothed.snap();
        }
        self.reset();
    }

    fn reset(&mut self) {
        self.pre_delay.clear();
        for diffuser in self.diffusers.iter_mut().flatten() {
            diffuser.ring.clear();
        }
        for line in &mut self.lines {
            line.clear();
        }
        self.dampers = [0.0; LINES];
        self.lows = [0.0; LINES];
        self.turns = std::array::from_fn(|line| line as f32 * 0.7);
        self.low_cut_filter.reset();
    }

    fn process(&mut self, audio: &mut [Frame]) {
        if self.values.take_change() {
            self.read_knobs();
        }
        for frame in audio.iter_mut() {
            let mono = (frame[0] + frame[1]) * 0.5;
            let side_in = (frame[0] - frame[1]) * 0.5;
            let cleaned = self.low_cut_filter.run(&self.low_cut, 0, mono);
            self.pre_delay.push(cleaned);
            let delayed = self.pre_delay.back(self.pre_delay_frames);
            let spread = [delayed + side_in * 0.5, delayed - side_in * 0.5];
            let mut fed = [0.0; 2];
            for side in 0..2 {
                fed[side] = self.diffusers[side].iter_mut().fold(spread[side], |x, diffuser| diffuser.run(x));
            }

            let mut heard = [0.0f32; LINES];
            for line in 0..LINES {
                let length = self.lengths[line].next();
                self.turns[line] += self.turn_steps[line];
                if self.turns[line] > std::f32::consts::TAU {
                    self.turns[line] -= std::f32::consts::TAU;
                }
                let wobble = self.wobble * (1.0 + self.turns[line].sin()) * 0.5;
                heard[line] = self.lines[line].back_between(length - 1.0 + wobble);
            }
            let mut wet = [0.0f32; 2];
            let mut sum = 0.0f32;
            let mut kept = [0.0f32; LINES];
            for line in 0..LINES {
                wet[line % 2] += heard[line];
                self.dampers[line] = heard[line] + (self.dampers[line] - heard[line]) * self.damp;
                let damped = self.dampers[line];
                self.lows[line] = damped + (self.lows[line] - damped) * self.split;
                let low = self.lows[line];
                kept[line] = low * self.keep_low[line] + (damped - low) * self.keep[line];
                sum += kept[line];
            }
            let spread_back = sum * 2.0 / LINES as f32;
            for line in 0..LINES {
                let next = kept[line] - spread_back + fed[line % 2] * INTO_LINES;
                self.lines[line].push(settled(next));
            }

            let width = self.width.next();
            let middle = (wet[0] + wet[1]) * 0.5 * OUT_OF_LINES;
            let sides = (wet[0] - wet[1]) * 0.5 * OUT_OF_LINES * width;
            let mix = self.mix.next();
            frame[0] = frame[0] * (1.0 - mix) + (middle + sides) * mix;
            frame[1] = frame[1] * (1.0 - mix) + (middle - sides) * mix;
        }
        self.low_cut_filter.settle();
        for value in self.dampers.iter_mut().chain(self.lows.iter_mut()) {
            *value = settled(*value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{in_blocks, noise, peak, rms};

    fn tail(decay: f32) -> Vec<Frame> {
        let rate = 48_000.0;
        let mut reverb = Reverb::new();
        for (id, value) in [("decay", decay), ("mix", 100.0), ("pre_delay", 0.0), ("damping", 20_000.0)] {
            reverb.set_by_id(id, value);
        }
        reverb.prepare(rate);
        let mut audio = vec![[0.0f32; 2]; (rate * 3.0) as usize];
        audio[0] = [1.0, 1.0];
        in_blocks(&mut reverb, &mut audio, 256);
        audio
    }

    fn level_at(audio: &[Frame], seconds: f32) -> f32 {
        let start = (seconds * 48_000.0) as usize;
        crate::db_of(rms(&audio[start..start + 4800]))
    }

    #[test]
    fn the_tail_dies_away_at_about_the_decay_time() {
        let audio = tail(1.0);
        let early = level_at(&audio, 0.1);
        let at_decay = level_at(&audio, 1.1) - early;
        assert!(at_decay < -50.0 && at_decay > -75.0, "fell {at_decay} dB in one decay time");
        let longer = tail(4.0);
        assert!(level_at(&longer, 1.1) - level_at(&longer, 0.1) > -22.0, "a longer decay rings on");
    }

    #[test]
    fn zero_mix_is_untouched_sound() {
        let mut reverb = Reverb::new();
        reverb.set_by_id("mix", 0.0);
        reverb.prepare(48_000.0);
        let input = noise(0.8, 9000, 2);
        let mut output = input.clone();
        in_blocks(&mut reverb, &mut output, 300);
        assert_eq!(output, input);
    }

    #[test]
    fn the_longest_decay_with_loud_noise_stays_bounded() {
        let mut reverb = Reverb::new();
        for (id, value) in [("decay", 20.0), ("mix", 100.0), ("size", 100.0), ("damping", 20_000.0), ("low_cut", 20.0)] {
            reverb.set_by_id(id, value);
        }
        reverb.prepare(48_000.0);
        let mut audio = noise(1.0, 48_000 * 10, 8);
        in_blocks(&mut reverb, &mut audio, 512);
        assert!(audio.iter().flatten().all(|sample| sample.is_finite()));
        assert!(peak(&audio) < 20.0, "peak {}", peak(&audio));
    }

    #[test]
    fn bass_decay_lets_the_lows_ring_longer() {
        let measure = |bass: f32| {
            let rate = 48_000.0;
            let mut reverb = Reverb::new();
            for (id, value) in [("decay", 1.0), ("mix", 100.0), ("low_cut", 20.0), ("bass_decay", bass), ("modulation", 0.0)] {
                reverb.set_by_id(id, value);
            }
            reverb.prepare(rate);
            let mut audio = crate::testing::sine(rate, 80.0, 0.5, (rate * 0.3) as usize);
            audio.resize((rate * 2.0) as usize, [0.0; 2]);
            in_blocks(&mut reverb, &mut audio, 256);
            level_at(&audio, 1.2)
        };
        assert!(measure(3.0) > measure(1.0) + 15.0);
        let mut values: Vec<f32> = PARAMS.iter().map(|param| param.default).collect();
        values[BASS_DECAY] = 2.0;
        values[LOW_CUT] = 20.0;
        assert!((decay_seconds(&values, 1000.0) - 2.0).abs() < 0.3, "{}", decay_seconds(&values, 1000.0));
        assert!(decay_seconds(&values, 60.0) > decay_seconds(&values, 1000.0) * 1.5);
        assert!(decay_seconds(&values, 15_000.0) < decay_seconds(&values, 1000.0));
    }

    #[test]
    fn width_zero_is_mono_and_full_width_is_not() {
        let audio = tail(1.0);
        assert!(audio[2000..8000].iter().any(|frame| (frame[0] - frame[1]).abs() > 1e-4));
        let mut reverb = Reverb::new();
        for (id, value) in [("width", 0.0), ("mix", 100.0)] {
            reverb.set_by_id(id, value);
        }
        reverb.prepare(48_000.0);
        let mut mono = noise(0.5, 9000, 6);
        in_blocks(&mut reverb, &mut mono, 128);
        assert!(mono.iter().all(|frame| frame[0] == frame[1]));
    }
}
