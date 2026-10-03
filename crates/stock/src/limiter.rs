use crate::smooth::Smoothed;
use crate::{decay_per_sample, gain_of, Effect, Frame, Param, Unit, Values, DEFAULT_RATE};

const PARAMS: [Param; 3] = [
    Param::new("input", "Input", 0.0, 24.0, 0.0, Unit::Decibels),
    Param::new("ceiling", "Ceiling", -12.0, 0.0, -0.3, Unit::Decibels),
    Param::new("release", "Release", 1.0, 1000.0, 60.0, Unit::Milliseconds),
];
const LOOKAHEAD_SECONDS: f32 = 0.0015;

pub struct Limiter {
    values: Values<3>,
    input: Smoothed,
    ceiling: f32,
    release: f32,
    ahead: usize,
    clock: usize,
    late: Vec<Frame>,
    lows: Vec<(usize, f32)>,
    lows_front: usize,
    lows_len: usize,
    eased: f32,
    recent: Vec<f32>,
    recent_sum: f64,
    deepest: f32,
    rate_for_knobs: f32,
}

impl Limiter {
    pub fn new() -> Self {
        let mut limiter = Self {
            values: Values::new(&PARAMS),
            input: Smoothed::new(1.0),
            ceiling: 1.0,
            release: 0.0,
            ahead: 1,
            clock: 0,
            late: Vec::new(),
            lows: Vec::new(),
            lows_front: 0,
            lows_len: 0,
            eased: 1.0,
            recent: Vec::new(),
            recent_sum: 0.0,
            deepest: 1.0,
            rate_for_knobs: DEFAULT_RATE,
        };
        limiter.prepare(DEFAULT_RATE);
        limiter
    }

    fn read_knobs(&mut self, rate: f32) {
        self.input.aim(gain_of(self.values.get(0)));
        self.ceiling = gain_of(self.values.get(1));
        self.release = decay_per_sample(self.values.get(2), rate);
    }

    pub fn take_reduction_db(&mut self) -> f32 {
        crate::db_of(std::mem::replace(&mut self.deepest, 1.0))
    }

    fn lowest_lately(&mut self, needed: f32) -> f32 {
        let window = self.ahead + 1;
        let capacity = self.lows.len();
        while self.lows_len > 0 && self.lows[(self.lows_front + self.lows_len - 1) % capacity].1 >= needed {
            self.lows_len -= 1;
        }
        self.lows[(self.lows_front + self.lows_len) % capacity] = (self.clock, needed);
        self.lows_len += 1;
        while self.lows[self.lows_front].0 + window <= self.clock {
            self.lows_front = (self.lows_front + 1) % capacity;
            self.lows_len -= 1;
        }
        self.lows[self.lows_front].1
    }
}

impl Default for Limiter {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for Limiter {
    fn name(&self) -> &'static str {
        "Loupe Limiter"
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

    fn latency(&self) -> usize {
        self.ahead
    }

    fn prepare(&mut self, rate: f32) {
        self.ahead = ((LOOKAHEAD_SECONDS * rate).round() as usize).max(1);
        self.late = vec![[0.0; 2]; self.ahead];
        self.lows = vec![(0, 1.0); self.ahead + 2];
        self.recent = vec![1.0; self.ahead];
        self.input.prepare(rate);
        self.read_knobs(rate);
        self.input.snap();
        self.values.take_change();
        self.rate_for_knobs = rate;
        self.reset();
    }

    fn reset(&mut self) {
        self.clock = 0;
        self.late.fill([0.0; 2]);
        self.lows_front = 0;
        self.lows_len = 0;
        self.eased = 1.0;
        self.recent.fill(1.0);
        self.recent_sum = self.ahead as f64;
        self.deepest = 1.0;
    }

    fn process(&mut self, audio: &mut [Frame]) {
        if self.values.take_change() {
            self.read_knobs(self.rate_for_knobs);
        }
        let ceiling = self.ceiling;
        for frame in audio.iter_mut() {
            let input = self.input.next();
            let raised = [frame[0] * input, frame[1] * input];
            let loudest = raised[0].abs().max(raised[1].abs());
            let needed = if loudest > ceiling { ceiling / loudest } else { 1.0 };
            let lowest = self.lowest_lately(needed);
            self.eased = if lowest < self.eased { lowest } else { lowest + (self.eased - lowest) * self.release };
            let slot = self.clock % self.ahead;
            self.recent_sum += self.eased as f64 - self.recent[slot] as f64;
            self.recent[slot] = self.eased;
            let gain = (self.recent_sum / self.ahead as f64).min(1.0) as f32;
            self.deepest = self.deepest.min(gain);
            let out = std::mem::replace(&mut self.late[slot], raised);
            frame[0] = (out[0] * gain).clamp(-ceiling, ceiling);
            frame[1] = (out[1] * gain).clamp(-ceiling, ceiling);
            self.clock += 1;
        }
        if self.clock > usize::MAX / 2 {
            let shift = self.clock - self.clock % self.ahead;
            for low in &mut self.lows {
                low.0 = low.0.saturating_sub(shift);
            }
            self.clock -= shift;
        }
        self.recent_sum = self.recent.iter().map(|value| *value as f64).sum();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{in_blocks, noise, peak};

    #[test]
    fn nothing_gets_past_the_ceiling() {
        for rate in [44_100.0, 48_000.0, 96_000.0] {
            let mut limiter = Limiter::new();
            limiter.prepare(rate);
            limiter.set_by_id("input", 18.0);
            limiter.set_by_id("ceiling", -1.0);
            let mut audio = noise(0.8, 96_000, 5);
            in_blocks(&mut limiter, &mut audio, 333);
            assert!(peak(&audio) <= gain_of(-1.0) + 1e-6, "{rate}: {}", peak(&audio));
        }
    }

    #[test]
    fn quiet_sound_comes_out_unchanged_but_late_by_the_reported_latency() {
        let mut limiter = Limiter::new();
        limiter.prepare(48_000.0);
        let late = limiter.latency();
        assert_eq!(late, 72);
        let input = noise(0.3, 4000, 9);
        let mut output = input.clone();
        in_blocks(&mut limiter, &mut output, 100);
        assert!(output[..late].iter().all(|frame| *frame == [0.0, 0.0]));
        assert_eq!(&output[late..], &input[..input.len() - late]);
    }

    #[test]
    fn a_single_spike_is_caught_without_clipping_the_shape_around_it() {
        let mut limiter = Limiter::new();
        limiter.prepare(48_000.0);
        let mut audio = vec![[0.1f32, 0.1]; 2000];
        audio[1000] = [2.0, -2.0];
        limiter.process(&mut audio);
        let late = limiter.latency();
        let ceiling = gain_of(-0.3);
        assert!((audio[1000 + late][0] - ceiling).abs() < 1e-4, "{}", audio[1000 + late][0]);
        assert!(audio[1000 + late - 1][0] < 0.1 && audio[1000 + late - 1][0] > 0.0, "gain eases down before the spike");
        assert!(limiter.take_reduction_db() < -6.0);
    }
}
