use std::sync::Arc;

use crate::history::{Gatherer, History};
use crate::{decay_per_sample, gain_of, settled, Effect, Frame, Param, Unit, Values, DEFAULT_RATE};

const PARAMS: [Param; 6] = [
    Param::new("threshold", "Threshold", -80.0, 0.0, -40.0, Unit::Decibels),
    Param::new("range", "Range", 0.0, 80.0, 40.0, Unit::Decibels),
    Param::new("attack", "Attack", 0.1, 50.0, 1.0, Unit::Milliseconds),
    Param::new("hold", "Hold", 0.0, 500.0, 50.0, Unit::Milliseconds),
    Param::new("release", "Release", 5.0, 2000.0, 150.0, Unit::Milliseconds),
    Param::new("hysteresis", "Hysteresis", 0.0, 12.0, 4.0, Unit::Decibels),
];
pub const THRESHOLD: usize = 0;
pub const RANGE: usize = 1;
pub const ATTACK: usize = 2;
pub const HOLD: usize = 3;
pub const RELEASE: usize = 4;
pub const HYSTERESIS: usize = 5;
const DETECT_FALL_MS: f32 = 5.0;

pub struct Gate {
    values: Values<6>,
    rate: f32,
    open_at: f32,
    close_at: f32,
    range_db: f32,
    opening: f32,
    closing: f32,
    hold: usize,
    detect_fall: f32,
    level: f32,
    open: bool,
    held: usize,
    gain_db: f32,
    history: Arc<History>,
    gatherer: Gatherer,
}

impl Gate {
    pub fn new() -> Self {
        let mut gate = Self {
            values: Values::new(&PARAMS),
            rate: DEFAULT_RATE,
            open_at: 0.0,
            close_at: 0.0,
            range_db: 0.0,
            opening: 0.0,
            closing: 0.0,
            hold: 0,
            detect_fall: 0.0,
            level: 0.0,
            open: false,
            held: 0,
            gain_db: 0.0,
            history: Arc::new(History::new()),
            gatherer: Gatherer::new(DEFAULT_RATE),
        };
        gate.prepare(DEFAULT_RATE);
        gate
    }

    pub fn history(&self) -> Arc<History> {
        self.history.clone()
    }

    fn read_knobs(&mut self) {
        let threshold = self.values.get(THRESHOLD);
        self.open_at = gain_of(threshold);
        self.close_at = gain_of(threshold - self.values.get(HYSTERESIS));
        self.range_db = self.values.get(RANGE);
        let per_ms = self.rate * 0.001;
        self.opening = self.range_db / (self.values.get(ATTACK) * per_ms).max(1.0);
        self.closing = self.range_db / (self.values.get(RELEASE) * per_ms).max(1.0);
        self.hold = (self.values.get(HOLD) * 0.001 * self.rate) as usize;
    }
}

impl Default for Gate {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for Gate {
    fn history(&self) -> Option<Arc<History>> {
        Some(self.history())
    }

    fn name(&self) -> &'static str {
        "Loupe Gate"
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
        self.detect_fall = decay_per_sample(DETECT_FALL_MS, rate);
        self.gatherer = Gatherer::new(rate);
        self.read_knobs();
        self.values.take_change();
        self.reset();
    }

    fn reset(&mut self) {
        self.level = 0.0;
        self.open = false;
        self.held = 0;
        self.gain_db = -self.range_db;
    }

    fn process(&mut self, audio: &mut [Frame]) {
        if self.values.take_change() {
            self.read_knobs();
        }
        for frame in audio.iter_mut() {
            let heard = frame[0].abs().max(frame[1].abs());
            self.level = if heard > self.level { heard } else { heard + (self.level - heard) * self.detect_fall };
            if self.level >= self.open_at {
                self.open = true;
                self.held = self.hold;
            } else if self.open && self.level < self.close_at {
                if self.held > 0 {
                    self.held -= 1;
                } else {
                    self.open = false;
                }
            }
            self.gain_db = if self.open { (self.gain_db + self.opening).min(0.0) } else { (self.gain_db - self.closing).max(-self.range_db) };
            let gain = if self.gain_db >= 0.0 { 1.0 } else { gain_of(self.gain_db) };
            frame[0] *= gain;
            frame[1] *= gain;
            self.gatherer.hear(&self.history, heard, frame[0].abs().max(frame[1].abs()), gain);
        }
        self.level = settled(self.level);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db_of;
    use crate::testing::{in_blocks, rms, sine};

    fn bursts(rate: f32) -> Vec<Frame> {
        let tone = sine(rate, 200.0, 1.0, (rate * 2.0) as usize);
        tone.iter()
            .enumerate()
            .map(|(i, frame)| {
                let loud = (i / (rate as usize / 2)) % 2 == 0;
                let level = if loud { 0.5 } else { 0.003 };
                [frame[0] * level, frame[1] * level]
            })
            .collect()
    }

    #[test]
    fn quiet_parts_are_turned_down_by_the_range_and_loud_parts_pass() {
        let rate = 48_000.0;
        let mut gate = Gate::new();
        gate.prepare(rate);
        let input = bursts(rate);
        let mut output = input.clone();
        in_blocks(&mut gate, &mut output, 256);
        let loud = db_of(rms(&output[2_400..21_600]) / rms(&input[2_400..21_600]));
        let quiet = db_of(rms(&output[36_000..47_000]) / rms(&input[36_000..47_000]));
        assert!(loud.abs() < 0.5, "loud part moved {loud} dB");
        assert!(quiet < -35.0, "quiet part only {quiet} dB");
    }

    #[test]
    fn hold_keeps_it_open_through_short_dips() {
        let rate = 48_000.0;
        let mut gate = Gate::new();
        gate.set(HOLD, 200.0);
        gate.prepare(rate);
        let mut audio: Vec<Frame> = sine(rate, 200.0, 0.5, 9_600);
        for frame in &mut audio[4_800..7_200] {
            *frame = [frame[0] * 0.003, frame[1] * 0.003];
        }
        let input = audio.clone();
        in_blocks(&mut gate, &mut audio, 128);
        let dip = db_of(rms(&audio[5_200..7_000]) / rms(&input[5_200..7_000]));
        assert!(dip.abs() < 1.0, "the gate closed in a 50 ms dip: {dip} dB");
    }

    #[test]
    fn zero_range_never_changes_the_sound() {
        let rate = 48_000.0;
        let mut gate = Gate::new();
        gate.set(RANGE, 0.0);
        gate.prepare(rate);
        let input = bursts(rate);
        let mut output = input.clone();
        in_blocks(&mut gate, &mut output, 256);
        let biggest = input.iter().zip(&output).map(|(a, b)| (a[0] - b[0]).abs()).fold(0.0f32, f32::max);
        assert!(biggest < 1e-6, "{biggest}");
    }
}
