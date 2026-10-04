use std::sync::Arc;

use crate::biquad::{Biquad, Coefficients, Shape, BUTTERWORTH};
use crate::history::{History, Moment, MOMENTS_PER_SECOND};
use crate::smooth::Smoothed;
use crate::{db_of, decay_per_sample, gain_of, Effect, Frame, Param, Unit, Values, DEFAULT_RATE};

pub const SOLOS: [&str; 4] = ["Off", "Low", "Mid", "High"];
pub const BANDS: usize = 3;
const PARAMS: [Param; 16] = [
    Param::new("low_cross", "Low cross", 40.0, 1_000.0, 150.0, Unit::Hertz),
    Param::new("high_cross", "High cross", 1_000.0, 12_000.0, 3_000.0, Unit::Hertz),
    Param::new("low_threshold", "Low threshold", -60.0, 0.0, -18.0, Unit::Decibels),
    Param::new("low_ratio", "Low ratio", 1.0, 20.0, 3.0, Unit::Ratio),
    Param::new("low_gain", "Low gain", -12.0, 12.0, 0.0, Unit::Decibels),
    Param::new("mid_threshold", "Mid threshold", -60.0, 0.0, -18.0, Unit::Decibels),
    Param::new("mid_ratio", "Mid ratio", 1.0, 20.0, 2.0, Unit::Ratio),
    Param::new("mid_gain", "Mid gain", -12.0, 12.0, 0.0, Unit::Decibels),
    Param::new("high_threshold", "High threshold", -60.0, 0.0, -18.0, Unit::Decibels),
    Param::new("high_ratio", "High ratio", 1.0, 20.0, 3.0, Unit::Ratio),
    Param::new("high_gain", "High gain", -12.0, 12.0, 0.0, Unit::Decibels),
    Param::new("attack", "Attack", 0.1, 200.0, 10.0, Unit::Milliseconds),
    Param::new("release", "Release", 5.0, 2_000.0, 150.0, Unit::Milliseconds),
    Param::new("mix", "Mix", 0.0, 100.0, 100.0, Unit::Percent),
    Param::new("output", "Output", -24.0, 12.0, 0.0, Unit::Decibels),
    Param::choice("solo", "Solo", &SOLOS, 0),
];
pub const LOW_CROSS: usize = 0;
pub const HIGH_CROSS: usize = 1;
pub const ATTACK: usize = 11;
pub const RELEASE: usize = 12;
pub const MIX: usize = 13;
pub const OUTPUT: usize = 14;
pub const SOLO: usize = 15;
const KNEE_DB: f32 = 6.0;

pub fn band_knob(band: usize, which: usize) -> usize {
    2 + band * 3 + which
}

pub fn reduction_db(level_db: f32, threshold: f32, ratio: f32) -> f32 {
    let slope = 1.0 / ratio - 1.0;
    let over = level_db - threshold;
    if 2.0 * over <= -KNEE_DB {
        0.0
    } else if 2.0 * over.abs() < KNEE_DB {
        slope * (over + KNEE_DB / 2.0).powi(2) / (2.0 * KNEE_DB)
    } else {
        slope * over
    }
}

#[derive(Default)]
struct Split {
    low: [Biquad; 2],
    high: [Biquad; 2],
}

impl Split {
    fn run(&mut self, lows: &Coefficients, highs: &Coefficients, channel: usize, x: f32) -> (f32, f32) {
        let half_low = self.low[0].run(lows, channel, x);
        let low = self.low[1].run(lows, channel, half_low);
        let half_high = self.high[0].run(highs, channel, x);
        let high = self.high[1].run(highs, channel, half_high);
        (low, high)
    }

    fn reset(&mut self) {
        for filter in self.low.iter_mut().chain(self.high.iter_mut()) {
            filter.reset();
        }
    }

    fn settle(&mut self) {
        for filter in self.low.iter_mut().chain(self.high.iter_mut()) {
            filter.settle();
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Band {
    threshold: f32,
    ratio: f32,
    gain: f32,
    reduction: f32,
}

pub struct Multiband {
    values: Values<16>,
    rate: f32,
    low_cut: [Coefficients; 2],
    high_cut: [Coefficients; 2],
    first: Split,
    second: Split,
    line_up: Split,
    bands: [Band; BANDS],
    attack: f32,
    release: f32,
    mix: Smoothed,
    output: Smoothed,
    solo: usize,
    deepest: [f32; BANDS],
    counted: usize,
    every: usize,
    history: Arc<History>,
}

impl Multiband {
    pub fn new() -> Self {
        let mut multiband = Self {
            values: Values::new(&PARAMS),
            rate: DEFAULT_RATE,
            low_cut: [Coefficients::PASS; 2],
            high_cut: [Coefficients::PASS; 2],
            first: Split::default(),
            second: Split::default(),
            line_up: Split::default(),
            bands: [Band::default(); BANDS],
            attack: 0.0,
            release: 0.0,
            mix: Smoothed::new(1.0),
            output: Smoothed::new(1.0),
            solo: 0,
            deepest: [0.0; BANDS],
            counted: 0,
            every: 1,
            history: Arc::new(History::new()),
        };
        multiband.prepare(DEFAULT_RATE);
        multiband
    }

    pub fn history(&self) -> Arc<History> {
        self.history.clone()
    }

    fn read_knobs(&mut self) {
        let low_hz = self.values.get(LOW_CROSS);
        let high_hz = self.values.get(HIGH_CROSS).max(low_hz * 1.5);
        let pair = |hz: f32, rate: f32| {
            (
                Coefficients::design(Shape::HighCut, rate, hz, BUTTERWORTH, 0.0),
                Coefficients::design(Shape::LowCut, rate, hz, BUTTERWORTH, 0.0),
            )
        };
        let (low_pass, high_pass) = pair(low_hz, self.rate);
        self.low_cut = [low_pass, high_pass];
        let (low_pass, high_pass) = pair(high_hz, self.rate);
        self.high_cut = [low_pass, high_pass];
        for (index, band) in self.bands.iter_mut().enumerate() {
            band.threshold = self.values.get(band_knob(index, 0));
            band.ratio = self.values.get(band_knob(index, 1));
            band.gain = gain_of(self.values.get(band_knob(index, 2)));
        }
        self.attack = decay_per_sample(self.values.get(ATTACK), self.rate);
        self.release = decay_per_sample(self.values.get(RELEASE), self.rate);
        self.mix.aim(self.values.get(MIX) / 100.0);
        self.output.aim(gain_of(self.values.get(OUTPUT)));
        self.solo = self.values.get(SOLO).round() as usize;
    }
}

impl Default for Multiband {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for Multiband {
    fn history(&self) -> Option<Arc<History>> {
        Some(self.history())
    }

    fn name(&self) -> &'static str {
        "Loupe Multiband"
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
        self.every = ((rate / MOMENTS_PER_SECOND) as usize).max(1);
        self.mix.prepare(rate);
        self.output.prepare(rate);
        self.read_knobs();
        self.values.take_change();
        self.mix.snap();
        self.output.snap();
        self.reset();
    }

    fn reset(&mut self) {
        self.first.reset();
        self.second.reset();
        self.line_up.reset();
        for band in &mut self.bands {
            band.reduction = 0.0;
        }
        self.deepest = [0.0; BANDS];
        self.counted = 0;
    }

    fn process(&mut self, audio: &mut [Frame]) {
        if self.values.take_change() {
            self.read_knobs();
        }
        for frame in audio.iter_mut() {
            let mut parts = [[0.0f32; 2]; BANDS];
            for channel in 0..2 {
                let x = frame[channel];
                let (low, rest) = self.first.run(&self.low_cut[0], &self.low_cut[1], channel, x);
                let (mid, high) = self.second.run(&self.high_cut[0], &self.high_cut[1], channel, rest);
                let (low_a, low_b) = self.line_up.run(&self.high_cut[0], &self.high_cut[1], channel, low);
                parts[0][channel] = low_a + low_b;
                parts[1][channel] = mid;
                parts[2][channel] = high;
            }
            let mix = self.mix.next();
            let output = self.output.next();
            let mut wet = [0.0f32; 2];
            for (index, band) in self.bands.iter_mut().enumerate() {
                let part = parts[index];
                let level = db_of(part[0].abs().max(part[1].abs()));
                let wanted = reduction_db(level, band.threshold, band.ratio);
                let speed = if wanted < band.reduction { self.attack } else { self.release };
                band.reduction = wanted + (band.reduction - wanted) * speed;
                let gain = gain_of(band.reduction) * band.gain;
                self.deepest[index] = self.deepest[index].min(band.reduction);
                if self.solo == 0 || self.solo == index + 1 {
                    wet[0] += part[0] * gain;
                    wet[1] += part[1] * gain;
                }
            }
            let dry_part = if self.solo == 0 { *frame } else { parts[self.solo - 1] };
            for channel in 0..2 {
                frame[channel] = (dry_part[channel] + (wet[channel] - dry_part[channel]) * mix) * output;
            }
            self.counted += 1;
            if self.counted >= self.every {
                self.history.push(Moment { input: self.deepest[0], output: self.deepest[1], reduction: self.deepest[2] });
                self.deepest = [0.0; BANDS];
                self.counted = 0;
            }
        }
        self.first.settle();
        self.second.settle();
        self.line_up.settle();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{in_blocks, rms, sine};

    const RATE: f32 = 48_000.0;

    fn through(multiband: &mut Multiband, audio: &[Frame]) -> Vec<Frame> {
        let mut out = audio.to_vec();
        in_blocks(multiband, &mut out, 256);
        out
    }

    fn untouched() -> Multiband {
        let mut multiband = Multiband::new();
        for band in 0..BANDS {
            multiband.set(band_knob(band, 0), 0.0);
        }
        multiband.prepare(RATE);
        multiband
    }

    #[test]
    fn with_nothing_squeezed_every_frequency_comes_out_at_the_same_level() {
        for hz in [60.0, 150.0, 400.0, 1_000.0, 3_000.0, 8_000.0] {
            let mut multiband = untouched();
            let input = sine(RATE, hz, 0.1, 48_000);
            let output = through(&mut multiband, &input);
            let change = db_of(rms(&output[24_000..]) / rms(&input[24_000..]));
            assert!(change.abs() < 0.1, "{hz} Hz moved {change} dB");
        }
    }

    #[test]
    fn a_loud_low_end_is_held_down_and_the_highs_are_left_alone() {
        let mut multiband = untouched();
        multiband.set(band_knob(0, 0), -30.0);
        multiband.set(band_knob(0, 1), 8.0);
        let lows = sine(RATE, 60.0, 0.5, 48_000);
        let highs = sine(RATE, 8_000.0, 0.5, 48_000);
        let low_change = db_of(rms(&through(&mut multiband, &lows)[24_000..]) / rms(&lows[24_000..]));
        let mut fresh = untouched();
        fresh.set(band_knob(0, 0), -30.0);
        fresh.set(band_knob(0, 1), 8.0);
        let high_change = db_of(rms(&through(&mut fresh, &highs)[24_000..]) / rms(&highs[24_000..]));
        assert!(low_change < -10.0, "the lows only moved {low_change} dB");
        assert!(high_change.abs() < 0.5, "the highs moved {high_change} dB");
        let mut seen = [Moment::default(); 20];
        multiband.history().latest(&mut seen);
        assert!(seen.iter().any(|moment| moment.input < -10.0 && moment.reduction == 0.0));
    }

    #[test]
    fn solo_plays_one_band_only() {
        let mut multiband = untouched();
        multiband.set(SOLO, 3.0);
        let lows = sine(RATE, 60.0, 0.5, 48_000);
        let heard = through(&mut multiband, &lows);
        assert!(rms(&heard[24_000..]) < rms(&lows[24_000..]) * 0.01);
        let mut mid_solo = untouched();
        mid_solo.set(SOLO, 2.0);
        let middle = sine(RATE, 800.0, 0.5, 48_000);
        let heard = through(&mut mid_solo, &middle);
        assert!(db_of(rms(&heard[24_000..]) / rms(&middle[24_000..])).abs() < 1.0);
    }

    #[test]
    fn band_gain_lifts_its_band() {
        let mut multiband = untouched();
        multiband.set(band_knob(2, 2), 6.0);
        let highs = sine(RATE, 9_000.0, 0.1, 48_000);
        let change = db_of(rms(&through(&mut multiband, &highs)[24_000..]) / rms(&highs[24_000..]));
        assert!((change - 6.0).abs() < 0.3, "{change}");
    }
}
