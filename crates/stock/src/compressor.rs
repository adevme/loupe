use crate::smooth::Smoothed;
use crate::{db_of, decay_per_sample, gain_of, Effect, Frame, Param, Unit, Values, DEFAULT_RATE};

const PARAMS: [Param; 7] = [
    Param::new("threshold", "Threshold", -60.0, 0.0, -18.0, Unit::Decibels),
    Param::new("ratio", "Ratio", 1.0, 20.0, 4.0, Unit::Ratio),
    Param::new("attack", "Attack", 0.1, 200.0, 10.0, Unit::Milliseconds),
    Param::new("release", "Release", 5.0, 2000.0, 120.0, Unit::Milliseconds),
    Param::new("knee", "Knee", 0.0, 24.0, 6.0, Unit::Decibels),
    Param::new("makeup", "Makeup", 0.0, 24.0, 0.0, Unit::Decibels),
    Param::new("mix", "Mix", 0.0, 100.0, 100.0, Unit::Percent),
];

pub struct Compressor {
    values: Values<7>,
    rate: f32,
    threshold: f32,
    slope: f32,
    knee: f32,
    attack: f32,
    release: f32,
    reduction: f32,
    deepest: f32,
    makeup: Smoothed,
    mix: Smoothed,
}

impl Compressor {
    pub fn new() -> Self {
        let mut compressor = Self {
            values: Values::new(&PARAMS),
            rate: DEFAULT_RATE,
            threshold: 0.0,
            slope: 0.0,
            knee: 0.0,
            attack: 0.0,
            release: 0.0,
            reduction: 0.0,
            deepest: 0.0,
            makeup: Smoothed::new(1.0),
            mix: Smoothed::new(1.0),
        };
        compressor.read_knobs();
        compressor.makeup.snap();
        compressor.mix.snap();
        compressor
    }

    fn read_knobs(&mut self) {
        self.threshold = self.values.get(0);
        self.slope = 1.0 / self.values.get(1) - 1.0;
        self.attack = decay_per_sample(self.values.get(2), self.rate);
        self.release = decay_per_sample(self.values.get(3), self.rate);
        self.knee = self.values.get(4);
        self.makeup.aim(gain_of(self.values.get(5)));
        self.mix.aim(self.values.get(6) / 100.0);
    }

    fn wanted_reduction(&self, level_db: f32) -> f32 {
        let over = level_db - self.threshold;
        if 2.0 * over <= -self.knee {
            0.0
        } else if 2.0 * over.abs() < self.knee {
            self.slope * (over + self.knee / 2.0).powi(2) / (2.0 * self.knee)
        } else {
            self.slope * over
        }
    }

    pub fn take_reduction_db(&mut self) -> f32 {
        std::mem::replace(&mut self.deepest, 0.0)
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
        self.makeup.prepare(rate);
        self.mix.prepare(rate);
        self.read_knobs();
        self.makeup.snap();
        self.mix.snap();
        self.reset();
    }

    fn reset(&mut self) {
        self.reduction = 0.0;
        self.deepest = 0.0;
    }

    fn process(&mut self, audio: &mut [Frame]) {
        if self.values.take_change() {
            self.read_knobs();
        }
        for frame in audio.iter_mut() {
            let level = db_of(frame[0].abs().max(frame[1].abs()));
            let wanted = self.wanted_reduction(level);
            let speed = if wanted < self.reduction { self.attack } else { self.release };
            self.reduction = wanted + (self.reduction - wanted) * speed;
            self.deepest = self.deepest.min(self.reduction);
            let mix = self.mix.next();
            let wet = gain_of(self.reduction) * self.makeup.next();
            let gain = 1.0 - mix + mix * wet;
            frame[0] *= gain;
            frame[1] *= gain;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::in_blocks;

    fn steady(level: f32, frames: usize) -> Vec<Frame> {
        vec![[level, -level]; frames]
    }

    #[test]
    fn a_loud_steady_sound_settles_at_the_ratio() {
        let mut compressor = Compressor::new();
        compressor.prepare(48_000.0);
        compressor.set_by_id("knee", 0.0);
        let mut audio = steady(0.5, 48_000);
        in_blocks(&mut compressor, &mut audio, 256);
        let out_db = db_of(audio[47_999][0]);
        let expected = -18.0 + (db_of(0.5) + 18.0) / 4.0;
        assert!((out_db - expected).abs() < 0.02, "{out_db} wanted {expected}");
        assert_eq!(audio[47_999][1], -audio[47_999][0], "both sides get the same gain");
        assert!(compressor.take_reduction_db() < -8.9);
    }

    #[test]
    fn quiet_sound_is_left_alone_and_makeup_adds_gain() {
        let mut compressor = Compressor::new();
        compressor.prepare(48_000.0);
        let mut quiet = steady(0.01, 4800);
        in_blocks(&mut compressor, &mut quiet, 64);
        assert_eq!(quiet[4799][0], 0.01);
        compressor.set_by_id("makeup", 6.0);
        let mut later = steady(0.01, 4800);
        in_blocks(&mut compressor, &mut later, 64);
        assert!((db_of(later[4799][0]) - db_of(0.01) - 6.0).abs() < 0.01);
    }

    #[test]
    fn attack_time_sets_how_fast_it_clamps_down() {
        let reached_after = |attack_ms: f32| {
            let mut compressor = Compressor::new();
            compressor.prepare(48_000.0);
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
    fn zero_mix_is_untouched_sound() {
        let mut compressor = Compressor::new();
        compressor.prepare(48_000.0);
        compressor.set_by_id("mix", 0.0);
        compressor.prepare(48_000.0);
        let input = crate::testing::noise(0.9, 5000, 11);
        let mut output = input.clone();
        compressor.process(&mut output);
        assert_eq!(output, input);
    }
}
