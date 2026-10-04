use std::sync::Arc;

use crate::history::{Gatherer, History};
use crate::smooth::Smoothed;
use crate::{decay_per_sample, gain_of, Effect, Frame, Param, Unit, Values, DEFAULT_RATE};

const PARAMS: [Param; 4] = [
    Param::new("input", "Gain", 0.0, 30.0, 0.0, Unit::Decibels),
    Param::new("ceiling", "Ceiling", -12.0, 0.0, -0.3, Unit::Decibels),
    Param::new("release", "Release", 1.0, 1000.0, 60.0, Unit::Milliseconds),
    Param::new("true_peak", "True peak", 0.0, 1.0, 1.0, Unit::Switch),
];
const LOOKAHEAD_SECONDS: f32 = 0.0015;
pub(crate) const PHASES: usize = 4;
pub(crate) const HALF_TAPS: usize = 8;
pub(crate) const TAPS: usize = HALF_TAPS * 2;
const KAISER_BETA: f32 = 6.0;

fn bessel_i0(x: f32) -> f32 {
    let mut sum = 1.0f32;
    let mut term = 1.0f32;
    for k in 1..25 {
        term *= (x / (2.0 * k as f32)).powi(2);
        sum += term;
    }
    sum
}

pub(crate) fn interpolation_taps() -> [[f32; TAPS]; PHASES - 1] {
    let mut taps = [[0.0; TAPS]; PHASES - 1];
    for (phase, row) in taps.iter_mut().enumerate() {
        let offset = (phase + 1) as f32 / PHASES as f32;
        for (j, tap) in row.iter_mut().enumerate() {
            let distance = j as f32 - (HALF_TAPS as f32 - 1.0) - offset;
            let sinc = if distance.abs() < 1e-6 { 1.0 } else { (std::f32::consts::PI * distance).sin() / (std::f32::consts::PI * distance) };
            let along = distance / HALF_TAPS as f32;
            let window = if along.abs() >= 1.0 { 0.0 } else { bessel_i0(KAISER_BETA * (1.0 - along * along).sqrt()) / bessel_i0(KAISER_BETA) };
            *tap = sinc * window;
        }
        let sum: f32 = row.iter().sum();
        for tap in row.iter_mut() {
            *tap /= sum;
        }
    }
    taps
}

pub fn true_peak(recent: impl Fn(usize) -> f32, taps: &[[f32; TAPS]; PHASES - 1]) -> f32 {
    let mut loudest = recent(HALF_TAPS - 1).abs();
    for row in taps {
        let value: f32 = row.iter().enumerate().map(|(j, tap)| tap * recent(j)).sum();
        loudest = loudest.max(value.abs());
    }
    loudest
}

pub struct Limiter {
    values: Values<4>,
    rate: f32,
    input: Smoothed,
    ceiling: f32,
    release: f32,
    catch_between: bool,
    taps: [[f32; TAPS]; PHASES - 1],
    ahead: usize,
    clock: usize,
    heard: Vec<Frame>,
    late: Vec<Frame>,
    lows: Vec<(usize, f32)>,
    lows_front: usize,
    lows_len: usize,
    eased: f32,
    recent: Vec<f32>,
    recent_sum: f64,
    history: Arc<History>,
    gatherer: Gatherer,
}

impl Limiter {
    pub fn new() -> Self {
        let mut limiter = Self {
            values: Values::new(&PARAMS),
            rate: DEFAULT_RATE,
            input: Smoothed::new(1.0),
            ceiling: 1.0,
            release: 0.0,
            catch_between: true,
            taps: interpolation_taps(),
            ahead: 1,
            clock: 0,
            heard: Vec::new(),
            late: Vec::new(),
            lows: Vec::new(),
            lows_front: 0,
            lows_len: 0,
            eased: 1.0,
            recent: Vec::new(),
            recent_sum: 0.0,
            history: Arc::new(History::new()),
            gatherer: Gatherer::new(DEFAULT_RATE),
        };
        limiter.prepare(DEFAULT_RATE);
        limiter
    }

    pub fn history(&self) -> Arc<History> {
        self.history.clone()
    }

    fn read_knobs(&mut self) {
        self.input.aim(gain_of(self.values.get(0)));
        self.ceiling = gain_of(self.values.get(1));
        self.release = decay_per_sample(self.values.get(2), self.rate);
        self.catch_between = self.values.get(3) > 0.5;
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

    fn loudest_now(&self) -> f32 {
        let newest = self.clock % TAPS;
        let at = |side: usize| move |j: usize| self.heard[(newest + 1 + j) % TAPS][side];
        if self.catch_between {
            true_peak(at(0), &self.taps).max(true_peak(at(1), &self.taps))
        } else {
            let middle = self.heard[(newest + HALF_TAPS) % TAPS];
            middle[0].abs().max(middle[1].abs())
        }
    }
}

impl Default for Limiter {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for Limiter {
    fn history(&self) -> Option<std::sync::Arc<History>> {
        Some(self.history())
    }

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
        self.ahead + HALF_TAPS
    }

    fn prepare(&mut self, rate: f32) {
        self.rate = rate;
        self.ahead = ((LOOKAHEAD_SECONDS * rate).round() as usize).max(1);
        self.heard = vec![[0.0; 2]; TAPS];
        self.late = vec![[0.0; 2]; self.ahead + HALF_TAPS];
        self.lows = vec![(0, 1.0); self.ahead + 2];
        self.recent = vec![1.0; self.ahead];
        self.input.prepare(rate);
        self.read_knobs();
        self.values.take_change();
        self.input.snap();
        self.gatherer = Gatherer::new(rate);
        self.reset();
    }

    fn reset(&mut self) {
        self.clock = 0;
        self.heard.fill([0.0; 2]);
        self.late.fill([0.0; 2]);
        self.lows_front = 0;
        self.lows_len = 0;
        self.eased = 1.0;
        self.recent.fill(1.0);
        self.recent_sum = self.ahead as f64;
    }

    fn process(&mut self, audio: &mut [Frame]) {
        if self.values.take_change() {
            self.read_knobs();
        }
        let ceiling = self.ceiling;
        let delay = self.late.len();
        for frame in audio.iter_mut() {
            let input = self.input.next();
            let raised = [frame[0] * input, frame[1] * input];
            self.heard[self.clock % TAPS] = raised;
            let loudest = self.loudest_now();
            let needed = if loudest > ceiling { ceiling / loudest } else { 1.0 };
            let lowest = self.lowest_lately(needed);
            self.eased = if lowest < self.eased { lowest } else { lowest + (self.eased - lowest) * self.release };
            let slot = self.clock % self.ahead;
            self.recent_sum += self.eased as f64 - self.recent[slot] as f64;
            self.recent[slot] = self.eased;
            let gain = (self.recent_sum / self.ahead as f64).min(1.0) as f32;
            let out = std::mem::replace(&mut self.late[self.clock % delay], raised);
            frame[0] = (out[0] * gain).clamp(-ceiling, ceiling);
            frame[1] = (out[1] * gain).clamp(-ceiling, ceiling);
            let heard_level = out[0].abs().max(out[1].abs());
            self.gatherer.hear(&self.history, heard_level, frame[0].abs().max(frame[1].abs()), gain);
            self.clock += 1;
        }
        if self.clock > usize::MAX / 2 {
            let cycle = self.ahead * delay * TAPS;
            let shift = self.clock - self.clock % cycle;
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

    fn reconstructed_peak(audio: &[Frame]) -> f32 {
        let taps = interpolation_taps();
        let mut loudest = 0.0f32;
        for start in 0..audio.len().saturating_sub(TAPS) {
            for side in 0..2 {
                loudest = loudest.max(true_peak(|j| audio[start + j][side], &taps));
            }
        }
        loudest
    }

    #[test]
    fn nothing_gets_past_the_ceiling() {
        for rate in [44_100.0, 48_000.0, 96_000.0] {
            for true_peak in [0.0, 1.0] {
                let mut limiter = Limiter::new();
                limiter.prepare(rate);
                limiter.set_by_id("input", 18.0);
                limiter.set_by_id("ceiling", -1.0);
                limiter.set_by_id("true_peak", true_peak);
                let mut audio = noise(0.8, 96_000, 5);
                in_blocks(&mut limiter, &mut audio, 333);
                assert!(peak(&audio) <= gain_of(-1.0) + 1e-6, "{rate}: {}", peak(&audio));
            }
        }
    }

    #[test]
    fn true_peak_also_holds_the_peaks_between_samples() {
        let rate = 48_000.0;
        let tone: Vec<Frame> = (0..48_000)
            .map(|i| {
                let value = 0.95 * (std::f32::consts::TAU * 11_025.0 * i as f32 / 44_100.0 + 0.7).sin();
                [value, value]
            })
            .collect();
        let over = |true_peak: f32| {
            let mut limiter = Limiter::new();
            limiter.prepare(rate);
            limiter.set_by_id("ceiling", -1.0);
            limiter.set_by_id("input", 6.0);
            limiter.set_by_id("true_peak", true_peak);
            let mut audio = tone.clone();
            in_blocks(&mut limiter, &mut audio, 256);
            crate::db_of(reconstructed_peak(&audio[4800..]))
        };
        let caught = over(1.0);
        assert!(caught <= -0.85, "true peak let through {caught} dB");
        assert!(over(0.0) > caught + 0.5, "sample peak alone misses the peaks between samples");
    }

    #[test]
    fn quiet_sound_comes_out_unchanged_but_late_by_the_reported_latency() {
        for true_peak in [0.0, 1.0] {
            let mut limiter = Limiter::new();
            limiter.set_by_id("true_peak", true_peak);
            limiter.prepare(48_000.0);
            let late = limiter.latency();
            assert_eq!(late, 72 + HALF_TAPS);
            let input = noise(0.3, 4000, 9);
            let mut output = input.clone();
            in_blocks(&mut limiter, &mut output, 100);
            assert!(output[..late].iter().all(|frame| *frame == [0.0, 0.0]));
            assert_eq!(&output[late..], &input[..input.len() - late]);
        }
    }

    #[test]
    fn a_single_spike_is_caught_without_clipping_the_shape_around_it() {
        let mut limiter = Limiter::new();
        limiter.set_by_id("true_peak", 0.0);
        limiter.prepare(48_000.0);
        let mut audio = vec![[0.1f32, 0.1]; 2000];
        audio[1000] = [2.0, -2.0];
        limiter.process(&mut audio);
        let late = limiter.latency();
        let ceiling = gain_of(-0.3);
        assert!((audio[1000 + late][0] - ceiling).abs() < 1e-4, "{}", audio[1000 + late][0]);
        assert!(audio[1000 + late - 1][0] < 0.1 && audio[1000 + late - 1][0] > 0.0, "gain eases down before the spike");
        let mut seen = [crate::Moment::default(); 30];
        limiter.history().latest(&mut seen);
        assert!(seen.iter().any(|moment| moment.reduction < -6.0));
    }
}
