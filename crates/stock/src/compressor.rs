use std::sync::Arc;

use crate::biquad::{Biquad, Coefficients, Shape, BUTTERWORTH};
use crate::history::{Gatherer, History};
use crate::smooth::Smoothed;
use crate::{db_of, decay_per_sample, gain_of, Effect, Frame, Param, Unit, Values, DEFAULT_RATE};

pub const STYLES: [&str; 3] = ["Clean", "Punch", "Vocal"];
const PARAMS: [Param; 12] = [
    Param::new("threshold", "Threshold", -60.0, 0.0, -18.0, Unit::Decibels),
    Param::new("ratio", "Ratio", 1.0, 20.0, 4.0, Unit::Ratio),
    Param::new("attack", "Attack", 0.05, 250.0, 10.0, Unit::Milliseconds),
    Param::new("release", "Release", 5.0, 2500.0, 120.0, Unit::Milliseconds),
    Param::new("knee", "Knee", 0.0, 24.0, 6.0, Unit::Decibels),
    Param::new("range", "Range", 0.0, 60.0, 60.0, Unit::Decibels),
    Param::new("makeup", "Makeup", -12.0, 24.0, 0.0, Unit::Decibels),
    Param::new("auto_gain", "Auto gain", 0.0, 1.0, 0.0, Unit::Switch),
    Param::new("mix", "Mix", 0.0, 100.0, 100.0, Unit::Percent),
    Param::choice("style", "Style", &STYLES, 0),
    Param::new("side_low_cut", "Side chain low cut", 20.0, 500.0, 20.0, Unit::Hertz),
    Param::new("output", "Output", -24.0, 24.0, 0.0, Unit::Decibels),
];
pub const THRESHOLD: usize = 0;
pub const RATIO: usize = 1;
pub const KNEE: usize = 4;
pub const RANGE: usize = 5;
const PUNCH_WINDOW_MS: f32 = 6.0;
const VOCAL_HOLD_MS: f32 = 400.0;
const VOCAL_FAST_SHARE: f32 = 0.25;
const SIDE_CHAIN_OFF_HZ: f32 = 21.0;

#[derive(Clone, Copy, PartialEq)]
enum Style {
    Clean,
    Punch,
    Vocal,
}

pub struct Curve {
    pub threshold: f32,
    pub ratio: f32,
    pub knee: f32,
    pub range: f32,
}

impl Curve {
    pub fn reduction(&self, level_db: f32) -> f32 {
        let slope = 1.0 / self.ratio - 1.0;
        let over = level_db - self.threshold;
        let wanted = if 2.0 * over <= -self.knee {
            0.0
        } else if 2.0 * over.abs() < self.knee {
            slope * (over + self.knee / 2.0).powi(2) / (2.0 * self.knee)
        } else {
            slope * over
        };
        wanted.max(-self.range)
    }
}

pub struct Compressor {
    values: Values<12>,
    rate: f32,
    curve: Curve,
    style: Style,
    attack: f32,
    release: f32,
    fast_release: f32,
    hold: f32,
    punch: f32,
    reduction: f32,
    slow_reduction: f32,
    power: f32,
    side_chain: Coefficients,
    side_filter: Biquad,
    side_chain_on: bool,
    makeup: Smoothed,
    mix: Smoothed,
    output: Smoothed,
    history: Arc<History>,
    gatherer: Gatherer,
}

impl Compressor {
    pub fn new() -> Self {
        let mut compressor = Self {
            values: Values::new(&PARAMS),
            rate: DEFAULT_RATE,
            curve: Curve { threshold: 0.0, ratio: 1.0, knee: 0.0, range: 0.0 },
            style: Style::Clean,
            attack: 0.0,
            release: 0.0,
            fast_release: 0.0,
            hold: 0.0,
            punch: 0.0,
            reduction: 0.0,
            slow_reduction: 0.0,
            power: 0.0,
            side_chain: Coefficients::PASS,
            side_filter: Biquad::default(),
            side_chain_on: false,
            makeup: Smoothed::new(1.0),
            mix: Smoothed::new(1.0),
            output: Smoothed::new(1.0),
            history: Arc::new(History::new()),
            gatherer: Gatherer::new(DEFAULT_RATE),
        };
        compressor.prepare(DEFAULT_RATE);
        compressor
    }

    pub fn history(&self) -> Arc<History> {
        self.history.clone()
    }

    pub fn curve(&self) -> Curve {
        Curve {
            threshold: self.values.get(THRESHOLD),
            ratio: self.values.get(RATIO),
            knee: self.values.get(KNEE),
            range: self.values.get(RANGE),
        }
    }

    pub fn curve_of(values: &[f32]) -> Curve {
        Curve { threshold: values[THRESHOLD], ratio: values[RATIO], knee: values[KNEE], range: values[RANGE] }
    }

    fn read_knobs(&mut self) {
        self.curve = self.curve();
        self.attack = decay_per_sample(self.values.get(2), self.rate);
        let release_ms = self.values.get(3);
        self.release = decay_per_sample(release_ms, self.rate);
        self.fast_release = decay_per_sample(release_ms * VOCAL_FAST_SHARE, self.rate);
        self.hold = decay_per_sample(VOCAL_HOLD_MS, self.rate);
        self.punch = decay_per_sample(PUNCH_WINDOW_MS, self.rate);
        self.style = match self.values.get(9) as usize {
            1 => Style::Punch,
            2 => Style::Vocal,
            _ => Style::Clean,
        };
        let side_hz = self.values.get(10);
        self.side_chain_on = side_hz > SIDE_CHAIN_OFF_HZ;
        self.side_chain = Coefficients::design(Shape::LowCut, self.rate, side_hz, BUTTERWORTH, 0.0);
        let automatic = if self.values.get(7) > 0.5 { -self.curve.reduction(0.0) * 0.5 } else { 0.0 };
        self.makeup.aim(gain_of(self.values.get(6) + automatic));
        self.mix.aim(self.values.get(8) / 100.0);
        self.output.aim(gain_of(self.values.get(11)));
    }

    fn detect(&mut self, frame: &Frame) -> f32 {
        let (mut left, mut right) = (frame[0], frame[1]);
        if self.side_chain_on {
            left = self.side_filter.run(&self.side_chain, 0, left);
            right = self.side_filter.run(&self.side_chain, 1, right);
        }
        let peak = left.abs().max(right.abs());
        match self.style {
            Style::Punch => {
                let square = (left * left + right * right) * 0.5;
                self.power = square + (self.power - square) * self.punch;
                db_of((2.0 * self.power).sqrt().max(peak * 0.5))
            }
            _ => db_of(peak),
        }
    }
}

impl Default for Compressor {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for Compressor {
    fn name(&self) -> &'static str {
        "Loupe Compressor"
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
        for smoothed in [&mut self.makeup, &mut self.mix, &mut self.output] {
            smoothed.prepare(rate);
        }
        self.read_knobs();
        self.values.take_change();
        for smoothed in [&mut self.makeup, &mut self.mix, &mut self.output] {
            smoothed.snap();
        }
        self.gatherer = Gatherer::new(rate);
        self.reset();
    }

    fn reset(&mut self) {
        self.reduction = 0.0;
        self.slow_reduction = 0.0;
        self.power = 0.0;
        self.side_filter.reset();
    }

    fn process(&mut self, audio: &mut [Frame]) {
        if self.values.take_change() {
            self.read_knobs();
        }
        for frame in audio.iter_mut() {
            let input = frame[0].abs().max(frame[1].abs());
            let level = self.detect(frame);
            let wanted = self.curve.reduction(level);
            if wanted < self.reduction {
                self.reduction = wanted + (self.reduction - wanted) * self.attack;
            } else {
                let release = match self.style {
                    Style::Vocal if self.reduction < self.slow_reduction => self.fast_release,
                    _ => self.release,
                };
                self.reduction = wanted + (self.reduction - wanted) * release;
            }
            self.slow_reduction = self.reduction + (self.slow_reduction - self.reduction) * self.hold;
            let mix = self.mix.next();
            let squeeze = gain_of(self.reduction);
            let gain = (1.0 - mix + mix * squeeze * self.makeup.next()) * self.output.next();
            frame[0] *= gain;
            frame[1] *= gain;
            self.gatherer.hear(&self.history, input, frame[0].abs().max(frame[1].abs()), squeeze);
        }
        self.side_filter.settle();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{in_blocks, noise};
    use crate::Moment;

    fn steady(level: f32, frames: usize) -> Vec<Frame> {
        vec![[level, -level]; frames]
    }

    fn ready() -> Compressor {
        let mut compressor = Compressor::new();
        compressor.prepare(48_000.0);
        compressor
    }

    #[test]
    fn a_loud_steady_sound_settles_at_the_ratio() {
        let mut compressor = ready();
        compressor.set_by_id("knee", 0.0);
        let mut audio = steady(0.5, 48_000);
        in_blocks(&mut compressor, &mut audio, 256);
        let out_db = db_of(audio[47_999][0]);
        let expected = -18.0 + (db_of(0.5) + 18.0) / 4.0;
        assert!((out_db - expected).abs() < 0.02, "{out_db} wanted {expected}");
        assert_eq!(audio[47_999][1], -audio[47_999][0], "both sides get the same gain");
    }

    #[test]
    fn range_caps_how_far_it_turns_down() {
        let mut compressor = ready();
        for (id, value) in [("knee", 0.0), ("range", 3.0), ("ratio", 20.0), ("threshold", -40.0)] {
            compressor.set_by_id(id, value);
        }
        let mut audio = steady(0.5, 48_000);
        in_blocks(&mut compressor, &mut audio, 256);
        assert!((db_of(audio[47_999][0]) - db_of(0.5) + 3.0).abs() < 0.02);
    }

    #[test]
    fn quiet_sound_is_left_alone_and_makeup_adds_gain() {
        let mut compressor = ready();
        let mut quiet = steady(0.01, 4800);
        in_blocks(&mut compressor, &mut quiet, 64);
        assert_eq!(quiet[4799][0], 0.01);
        compressor.set_by_id("makeup", 6.0);
        let mut later = steady(0.01, 4800);
        in_blocks(&mut compressor, &mut later, 64);
        assert!((db_of(later[4799][0]) - db_of(0.01) - 6.0).abs() < 0.01);
    }

    #[test]
    fn auto_gain_brings_a_full_scale_sound_back_up_halfway() {
        let mut compressor = ready();
        for (id, value) in [("knee", 0.0), ("auto_gain", 1.0)] {
            compressor.set_by_id(id, value);
        }
        compressor.prepare(48_000.0);
        let mut audio = steady(1.0, 48_000);
        in_blocks(&mut compressor, &mut audio, 256);
        let squeezed = -18.0 * (1.0 / 4.0 - 1.0);
        assert!((db_of(audio[47_999][0]) - (-squeezed + squeezed / 2.0)).abs() < 0.05);
    }

    #[test]
    fn attack_time_sets_how_fast_it_clamps_down() {
        let reached_after = |attack_ms: f32| {
            let mut compressor = ready();
            compressor.set_by_id("attack", attack_ms);
            compressor.set_by_id("knee", 0.0);
            let mut audio = steady(1.0, 48_000);
            in_blocks(&mut compressor, &mut audio, 32);
            let target = audio[47_999][0];
            audio.iter().position(|frame| frame[0] <= target * 1.05).unwrap()
        };
        let fast = reached_after(1.0);
        let slow = reached_after(50.0);
        assert!(slow > fast * 20, "fast {fast} slow {slow}");
    }

    #[test]
    fn every_style_compresses_and_stays_finite() {
        for style in 0..STYLES.len() {
            let mut compressor = ready();
            compressor.set_by_id("style", style as f32);
            compressor.set_by_id("side_low_cut", 150.0);
            let mut audio = noise(0.9, 48_000, 21);
            in_blocks(&mut compressor, &mut audio, 128);
            assert!(audio.iter().flatten().all(|sample| sample.is_finite()));
            let mut seen = [Moment::default(); 4];
            compressor.history().latest(&mut seen);
            assert!(seen.iter().all(|moment| moment.reduction < -3.0), "{}: {seen:?}", STYLES[style]);
        }
    }

    #[test]
    fn the_side_chain_low_cut_ignores_rumble() {
        let level = |cut: f32| {
            let mut compressor = ready();
            compressor.set_by_id("side_low_cut", cut);
            compressor.set_by_id("threshold", -30.0);
            let mut audio = crate::testing::sine(48_000.0, 40.0, 0.5, 48_000);
            in_blocks(&mut compressor, &mut audio, 256);
            crate::testing::rms(&audio[24_000..])
        };
        assert!(level(400.0) > level(20.0) * 2.0);
    }

    #[test]
    fn zero_mix_is_untouched_sound() {
        let mut compressor = ready();
        compressor.set_by_id("mix", 0.0);
        compressor.prepare(48_000.0);
        let input = noise(0.9, 5000, 11);
        let mut output = input.clone();
        compressor.process(&mut output);
        assert_eq!(output, input);
    }
}
