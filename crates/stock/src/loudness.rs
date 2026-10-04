use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crate::history::{History, Moment};
use crate::limiter::{interpolation_taps, true_peak, PHASES, TAPS};
use crate::{Effect, Frame, Param, Unit, Values, DEFAULT_RATE};

const PARAMS: [Param; 2] = [
    Param::new("target", "Target", -30.0, 0.0, -14.0, Unit::Decibels),
    Param::new("reset", "Reset", 0.0, 1.0, 0.0, Unit::Switch),
];
pub const RESET: usize = 1;
pub const SILENT: f32 = -100.0;
const STEPS_PER_SECOND: usize = 10;
const MOMENTARY_STEPS: usize = 4;
const SHORT_STEPS: usize = 30;
const ABSOLUTE_GATE: f64 = -70.0;
const RELATIVE_GATE: f64 = -10.0;
const RANGE_GATE: f64 = -20.0;
const BIN_LU: f64 = 0.1;
const LOWEST_BIN: f64 = -70.0;
const BINS: usize = 800;
const RANGE_LOW: f64 = 0.10;
const RANGE_HIGH: f64 = 0.95;

pub struct Readings {
    momentary: AtomicU32,
    short_term: AtomicU32,
    integrated: AtomicU32,
    range: AtomicU32,
    peak: AtomicU32,
    pub history: History,
}

impl Readings {
    fn new() -> Self {
        let silent = SILENT.to_bits();
        Self {
            momentary: AtomicU32::new(silent),
            short_term: AtomicU32::new(silent),
            integrated: AtomicU32::new(silent),
            range: AtomicU32::new(0f32.to_bits()),
            peak: AtomicU32::new(silent),
            history: History::new(),
        }
    }

    fn read(slot: &AtomicU32) -> f32 {
        f32::from_bits(slot.load(Ordering::Relaxed))
    }

    pub fn momentary(&self) -> f32 {
        Self::read(&self.momentary)
    }

    pub fn short_term(&self) -> f32 {
        Self::read(&self.short_term)
    }

    pub fn integrated(&self) -> f32 {
        Self::read(&self.integrated)
    }

    pub fn range(&self) -> f32 {
        Self::read(&self.range)
    }

    pub fn peak(&self) -> f32 {
        Self::read(&self.peak)
    }
}

#[derive(Clone, Copy, Default)]
struct Stage {
    b: [f64; 3],
    a: [f64; 2],
    state: [[f64; 2]; 2],
}

impl Stage {
    fn run(&mut self, channel: usize, x: f64) -> f64 {
        let s = &mut self.state[channel];
        let y = self.b[0] * x + s[0];
        s[0] = self.b[1] * x - self.a[0] * y + s[1];
        s[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

fn k_weighting(rate: f64) -> (Stage, Stage) {
    let (f0, gain, q) = (1681.974450955533, 3.999843853973347, 0.7071752369554196);
    let k = (std::f64::consts::PI * f0 / rate).tan();
    let vh = 10f64.powf(gain / 20.0);
    let vb = vh.powf(0.4996667741545416);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Stage {
        b: [(vh + vb * k / q + k * k) / a0, 2.0 * (k * k - vh) / a0, (vh - vb * k / q + k * k) / a0],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        state: [[0.0; 2]; 2],
    };
    let (f0, q) = (38.13547087602444, 0.5003270373238773);
    let k = (std::f64::consts::PI * f0 / rate).tan();
    let a0 = 1.0 + k / q + k * k;
    let low_cut = Stage { b: [1.0, -2.0, 1.0], a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0], state: [[0.0; 2]; 2] };
    (shelf, low_cut)
}

fn lufs(power: f64) -> f64 {
    if power <= 0.0 {
        f64::NEG_INFINITY
    } else {
        -0.691 + 10.0 * power.log10()
    }
}

fn power_of(lufs: f64) -> f64 {
    10f64.powf((lufs + 0.691) / 10.0)
}

fn bin_of(lufs: f64) -> Option<usize> {
    let bin = ((lufs - LOWEST_BIN) / BIN_LU).floor();
    (bin >= 0.0).then(|| (bin as usize).min(BINS - 1))
}

fn lufs_of_bin(bin: usize) -> f64 {
    LOWEST_BIN + (bin as f64 + 0.5) * BIN_LU
}

pub struct Meter {
    values: Values<2>,
    rate: f32,
    last_reset: f32,
    shelf: Stage,
    low_cut: Stage,
    step_len: usize,
    in_step: usize,
    step_sum: f64,
    steps: [f64; SHORT_STEPS],
    steps_seen: usize,
    blocks: [u32; BINS],
    block_power: [f64; BINS],
    shorts: [u32; BINS],
    short_power: [f64; BINS],
    taps: [[f32; TAPS]; PHASES - 1],
    recent: [[f32; TAPS]; 2],
    clock: usize,
    loudest: f32,
    readings: Arc<Readings>,
}

impl Meter {
    pub fn new() -> Self {
        let mut meter = Self {
            values: Values::new(&PARAMS),
            rate: DEFAULT_RATE,
            last_reset: 0.0,
            shelf: Stage::default(),
            low_cut: Stage::default(),
            step_len: 1,
            in_step: 0,
            step_sum: 0.0,
            steps: [0.0; SHORT_STEPS],
            steps_seen: 0,
            blocks: [0; BINS],
            block_power: [0.0; BINS],
            shorts: [0; BINS],
            short_power: [0.0; BINS],
            taps: interpolation_taps(),
            recent: [[0.0; TAPS]; 2],
            clock: 0,
            loudest: 0.0,
            readings: Arc::new(Readings::new()),
        };
        meter.prepare(DEFAULT_RATE);
        meter
    }

    pub fn readings(&self) -> Arc<Readings> {
        self.readings.clone()
    }

    fn clear(&mut self) {
        self.recent = [[0.0; TAPS]; 2];
        self.steps = [0.0; SHORT_STEPS];
        self.steps_seen = 0;
        self.step_sum = 0.0;
        self.in_step = 0;
        self.blocks = [0; BINS];
        self.block_power = [0.0; BINS];
        self.shorts = [0; BINS];
        self.short_power = [0.0; BINS];
        self.loudest = 0.0;
        for slot in [&self.readings.integrated, &self.readings.peak] {
            slot.store(SILENT.to_bits(), Ordering::Relaxed);
        }
        self.readings.range.store(0f32.to_bits(), Ordering::Relaxed);
    }

    fn integrated(&self) -> f64 {
        let (mut count, mut power) = (0u64, 0.0);
        for bin in 0..BINS {
            count += self.blocks[bin] as u64;
            power += self.block_power[bin];
        }
        if count == 0 {
            return f64::NEG_INFINITY;
        }
        let threshold = lufs(power / count as f64) + RELATIVE_GATE;
        let (mut kept, mut kept_power) = (0u64, 0.0);
        for bin in bin_of(threshold).unwrap_or(0)..BINS {
            kept += self.blocks[bin] as u64;
            kept_power += self.block_power[bin];
        }
        if kept == 0 {
            f64::NEG_INFINITY
        } else {
            lufs(kept_power / kept as f64)
        }
    }

    fn range(&self) -> f64 {
        let (mut count, mut power) = (0u64, 0.0);
        for bin in 0..BINS {
            count += self.shorts[bin] as u64;
            power += self.short_power[bin];
        }
        if count == 0 {
            return 0.0;
        }
        let first = bin_of(lufs(power / count as f64) + RANGE_GATE).unwrap_or(0);
        let kept: u64 = self.shorts[first..].iter().map(|n| *n as u64).sum();
        if kept == 0 {
            return 0.0;
        }
        let at = |share: f64| {
            let wanted = (share * kept as f64).ceil().max(1.0) as u64;
            let mut seen = 0u64;
            for bin in first..BINS {
                seen += self.shorts[bin] as u64;
                if seen >= wanted {
                    return lufs_of_bin(bin);
                }
            }
            lufs_of_bin(BINS - 1)
        };
        (at(RANGE_HIGH) - at(RANGE_LOW)).max(0.0)
    }

    fn finish_step(&mut self) {
        self.steps.rotate_left(1);
        self.steps[SHORT_STEPS - 1] = self.step_sum;
        self.step_sum = 0.0;
        self.steps_seen += 1;
        let mean = |count: usize| self.steps[SHORT_STEPS - count..].iter().sum::<f64>() / (count * self.step_len) as f64;
        let momentary = if self.steps_seen >= MOMENTARY_STEPS { lufs(mean(MOMENTARY_STEPS)) } else { f64::NEG_INFINITY };
        let short = if self.steps_seen >= SHORT_STEPS { lufs(mean(SHORT_STEPS)) } else { f64::NEG_INFINITY };
        if momentary > ABSOLUTE_GATE {
            if let Some(bin) = bin_of(momentary) {
                self.blocks[bin] += 1;
                self.block_power[bin] += power_of(momentary);
            }
        }
        if short > ABSOLUTE_GATE {
            if let Some(bin) = bin_of(short) {
                self.shorts[bin] += 1;
                self.short_power[bin] += power_of(short);
            }
        }
        let shown = |value: f64| if value.is_finite() { value.max(SILENT as f64) as f32 } else { SILENT };
        let readings = &self.readings;
        readings.momentary.store(shown(momentary).to_bits(), Ordering::Relaxed);
        readings.short_term.store(shown(short).to_bits(), Ordering::Relaxed);
        readings.integrated.store(shown(self.integrated()).to_bits(), Ordering::Relaxed);
        readings.range.store((self.range() as f32).to_bits(), Ordering::Relaxed);
        let peak = if self.loudest > 0.0 { crate::db_of(self.loudest) } else { SILENT };
        readings.peak.store(peak.to_bits(), Ordering::Relaxed);
        readings.history.push(Moment { input: shown(momentary), output: shown(short), reduction: 0.0 });
    }
}

impl Default for Meter {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for Meter {
    fn meter(&self) -> Option<Arc<Readings>> {
        Some(self.readings())
    }

    fn name(&self) -> &'static str {
        "Loupe Meter"
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
        let (shelf, low_cut) = k_weighting(rate as f64);
        self.shelf = shelf;
        self.low_cut = low_cut;
        self.step_len = (rate as usize / STEPS_PER_SECOND).max(1);
        self.last_reset = self.values.get(RESET);
        self.values.take_change();
        self.reset();
    }

    fn reset(&mut self) {
        self.shelf.state = [[0.0; 2]; 2];
        self.low_cut.state = [[0.0; 2]; 2];
        self.in_step = 0;
        self.step_sum = 0.0;
        self.steps = [0.0; SHORT_STEPS];
        self.steps_seen = 0;
        self.recent = [[0.0; TAPS]; 2];
        self.clock = 0;
        self.clear();
    }

    fn process(&mut self, audio: &mut [Frame]) {
        if self.values.take_change() {
            let pressed = self.values.get(RESET);
            if pressed != self.last_reset {
                self.last_reset = pressed;
                self.clear();
            }
        }
        for frame in audio.iter() {
            let slot = self.clock % TAPS;
            for channel in 0..2 {
                let weighted = self.low_cut.run(channel, self.shelf.run(channel, frame[channel] as f64));
                self.step_sum += weighted * weighted;
                self.recent[channel][slot] = frame[channel];
                let recent = &self.recent[channel];
                let peak = true_peak(|j| recent[(slot + 1 + j) % TAPS], &self.taps);
                self.loudest = self.loudest.max(peak);
            }
            self.clock = self.clock.wrapping_add(1);
            self.in_step += 1;
            if self.in_step == self.step_len {
                self.in_step = 0;
                self.finish_step();
            }
        }
        for stage in [&mut self.shelf, &mut self.low_cut] {
            for side in stage.state.iter_mut() {
                for value in side.iter_mut() {
                    if value.abs() < 1e-30 {
                        *value = 0.0;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::sine;

    const RATE: f32 = 48_000.0;

    fn tone(dbfs: f32, seconds: f32) -> Vec<Frame> {
        sine(RATE, 1_000.0, 10f32.powf(dbfs / 20.0), (RATE * seconds) as usize)
    }

    fn measured(parts: &[(f32, f32)]) -> Meter {
        let mut meter = Meter::new();
        meter.prepare(RATE);
        for (dbfs, seconds) in parts {
            let mut audio = tone(*dbfs, *seconds);
            for block in audio.chunks_mut(512) {
                meter.process(block);
            }
        }
        meter
    }

    #[test]
    fn a_1k_tone_at_minus_23_reads_minus_23_lufs() {
        let meter = measured(&[(-23.0, 5.0)]);
        let readings = meter.readings();
        for (name, value) in [("momentary", readings.momentary()), ("short", readings.short_term()), ("integrated", readings.integrated())] {
            assert!((value + 23.0).abs() < 0.1, "{name} read {value}");
        }
        assert!((readings.peak() + 23.0).abs() < 0.2, "peak {}", readings.peak());
    }

    #[test]
    fn quiet_parts_are_gated_out_of_the_integrated_reading() {
        let meter = measured(&[(-36.0, 5.0), (-23.0, 15.0), (-36.0, 5.0)]);
        let integrated = meter.readings().integrated();
        assert!((integrated + 23.0).abs() < 0.1, "integrated {integrated}");
    }

    #[test]
    fn two_levels_ten_lu_apart_give_a_range_of_ten() {
        let meter = measured(&[(-20.0, 20.0), (-30.0, 20.0)]);
        let range = meter.readings().range();
        assert!((range - 10.0).abs() < 1.0, "range {range}");
    }

    #[test]
    fn reset_clears_the_long_readings() {
        let mut meter = measured(&[(-12.0, 4.0)]);
        meter.set(RESET, 1.0);
        let mut quiet = tone(-30.0, 4.0);
        meter.process(&mut quiet);
        let integrated = meter.readings().integrated();
        assert!((integrated + 30.0).abs() < 0.3, "integrated {integrated}");
        assert!((meter.readings().peak() + 30.0).abs() < 0.3);
    }

    #[test]
    fn the_sound_passes_through_untouched() {
        let mut meter = Meter::new();
        let input = tone(-6.0, 0.5);
        let mut output = input.clone();
        meter.process(&mut output);
        assert_eq!(input, output);
    }
}
