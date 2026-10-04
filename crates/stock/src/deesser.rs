use std::sync::Arc;

use crate::biquad::{Biquad, Coefficients, Shape, BUTTERWORTH};
use crate::history::{Gatherer, History};
use crate::{db_of, decay_per_sample, gain_of, settled, Effect, Frame, Param, Unit, Values, DEFAULT_RATE};

pub const BANDS: [&str; 2] = ["Split", "Wide"];
const PARAMS: [Param; 5] = [
    Param::new("threshold", "Threshold", -60.0, 0.0, -30.0, Unit::Decibels),
    Param::new("range", "Range", 0.0, 24.0, 10.0, Unit::Decibels),
    Param::new("frequency", "Frequency", 2_000.0, 16_000.0, 6_000.0, Unit::Hertz),
    Param::choice("band", "Band", &BANDS, 0),
    Param::new("listen", "Listen", 0.0, 1.0, 0.0, Unit::Switch),
];
pub const THRESHOLD: usize = 0;
pub const RANGE: usize = 1;
pub const FREQUENCY: usize = 2;
pub const BAND: usize = 3;
pub const LISTEN: usize = 4;
const RATIO: f32 = 6.0;
const KNEE_DB: f32 = 6.0;
const DETECT_ATTACK_MS: f32 = 0.3;
const DETECT_RELEASE_MS: f32 = 40.0;
const GAIN_ATTACK_MS: f32 = 0.8;
const GAIN_RELEASE_MS: f32 = 60.0;

pub fn reduction_db(level_db: f32, threshold: f32, range: f32) -> f32 {
    let slope = 1.0 / RATIO - 1.0;
    let over = level_db - threshold;
    let wanted = if 2.0 * over <= -KNEE_DB {
        0.0
    } else if 2.0 * over.abs() < KNEE_DB {
        slope * (over + KNEE_DB / 2.0).powi(2) / (2.0 * KNEE_DB)
    } else {
        slope * over
    };
    wanted.max(-range)
}

pub struct Deesser {
    values: Values<5>,
    rate: f32,
    threshold: f32,
    range: f32,
    wide: bool,
    listen: bool,
    detect: Coefficients,
    split: Coefficients,
    detect_first: Biquad,
    detect_second: Biquad,
    split_filter: Biquad,
    listen_first: Biquad,
    listen_second: Biquad,
    level: f32,
    gain: f32,
    detect_attack: f32,
    detect_release: f32,
    gain_attack: f32,
    gain_release: f32,
    history: Arc<History>,
    gatherer: Gatherer,
}

impl Deesser {
    pub fn new() -> Self {
        let mut deesser = Self {
            values: Values::new(&PARAMS),
            rate: DEFAULT_RATE,
            threshold: 0.0,
            range: 0.0,
            wide: false,
            listen: false,
            detect: Coefficients::PASS,
            split: Coefficients::PASS,
            detect_first: Biquad::default(),
            detect_second: Biquad::default(),
            split_filter: Biquad::default(),
            listen_first: Biquad::default(),
            listen_second: Biquad::default(),
            level: 0.0,
            gain: 1.0,
            detect_attack: 0.0,
            detect_release: 0.0,
            gain_attack: 0.0,
            gain_release: 0.0,
            history: Arc::new(History::new()),
            gatherer: Gatherer::new(DEFAULT_RATE),
        };
        deesser.prepare(DEFAULT_RATE);
        deesser
    }

    pub fn history(&self) -> Arc<History> {
        self.history.clone()
    }

    fn read_knobs(&mut self) {
        self.threshold = self.values.get(THRESHOLD);
        self.range = self.values.get(RANGE);
        let hz = self.values.get(FREQUENCY);
        self.detect = Coefficients::design(Shape::LowCut, self.rate, hz, BUTTERWORTH, 0.0);
        self.split = Coefficients::design(Shape::HighCut, self.rate, hz, BUTTERWORTH, 0.0);
        self.wide = self.values.get(BAND) > 0.5;
        self.listen = self.values.get(LISTEN) > 0.5;
    }
}

impl Default for Deesser {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for Deesser {
    fn history(&self) -> Option<Arc<History>> {
        Some(self.history())
    }

    fn name(&self) -> &'static str {
        "Loupe De-esser"
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
        self.detect_attack = decay_per_sample(DETECT_ATTACK_MS, rate);
        self.detect_release = decay_per_sample(DETECT_RELEASE_MS, rate);
        self.gain_attack = decay_per_sample(GAIN_ATTACK_MS, rate);
        self.gain_release = decay_per_sample(GAIN_RELEASE_MS, rate);
        self.gatherer = Gatherer::new(rate);
        self.read_knobs();
        self.values.take_change();
        self.reset();
    }

    fn reset(&mut self) {
        self.detect_first.reset();
        self.detect_second.reset();
        self.split_filter.reset();
        self.listen_first.reset();
        self.listen_second.reset();
        self.level = 0.0;
        self.gain = 1.0;
    }

    fn process(&mut self, audio: &mut [Frame]) {
        if self.values.take_change() {
            self.read_knobs();
        }
        for frame in audio.iter_mut() {
            let mono = (frame[0] + frame[1]) * 0.5;
            let side = self.detect_second.run(&self.detect, 0, self.detect_first.run(&self.detect, 0, mono));
            let heard = side.abs();
            let follow = if heard > self.level { self.detect_attack } else { self.detect_release };
            self.level = heard + (self.level - heard) * follow;
            let target = gain_of(reduction_db(db_of(self.level), self.threshold, self.range));
            let ease = if target < self.gain { self.gain_attack } else { self.gain_release };
            self.gain = target + (self.gain - target) * ease;
            let input_peak = frame[0].abs().max(frame[1].abs());
            if self.listen {
                for (channel, sample) in frame.iter_mut().enumerate() {
                    *sample = self.listen_second.run(&self.detect, channel, self.listen_first.run(&self.detect, channel, *sample));
                }
            } else if self.wide {
                frame[0] *= self.gain;
                frame[1] *= self.gain;
            } else {
                let cut = 1.0 - self.gain;
                for (channel, sample) in frame.iter_mut().enumerate() {
                    let low = self.split_filter.run(&self.split, channel, *sample);
                    *sample -= cut * (*sample - low);
                }
            }
            self.gatherer.hear(&self.history, self.level, input_peak, self.gain);
        }
        self.level = settled(self.level);
        self.detect_first.settle();
        self.detect_second.settle();
        self.split_filter.settle();
        self.listen_first.settle();
        self.listen_second.settle();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{in_blocks, rms, sine};

    fn through(deesser: &mut Deesser, audio: &[Frame]) -> Vec<Frame> {
        let mut out = audio.to_vec();
        in_blocks(deesser, &mut out, 256);
        out
    }

    fn mixed(rate: f32, low: f32, high: f32) -> Vec<Frame> {
        let a = sine(rate, 300.0, low, 48_000);
        let b = sine(rate, 8_000.0, high, 48_000);
        a.iter().zip(&b).map(|(x, y)| [x[0] + y[0], x[1] + y[1]]).collect()
    }

    fn band_rms(audio: &[Frame], rate: f32, hz: f32) -> f32 {
        let shape = if hz > 1000.0 { Shape::LowCut } else { Shape::HighCut };
        let c = Coefficients::design(shape, rate, if hz > 1000.0 { 4_000.0 } else { 1_000.0 }, BUTTERWORTH, 0.0);
        let mut first = Biquad::default();
        let mut second = Biquad::default();
        let kept: Vec<Frame> = audio.iter().map(|f| {
            let v = second.run(&c, 0, first.run(&c, 0, f[0]));
            [v, v]
        }).collect();
        rms(&kept[kept.len() / 2..])
    }

    #[test]
    fn quiet_highs_pass_untouched() {
        let rate = 48_000.0;
        let mut deesser = Deesser::new();
        deesser.prepare(rate);
        let input = mixed(rate, 0.3, 0.001);
        let output = through(&mut deesser, &input);
        let biggest = input.iter().zip(&output).map(|(a, b)| (a[0] - b[0]).abs()).fold(0.0f32, f32::max);
        assert!(biggest < 1e-3, "changed by {biggest}");
    }

    #[test]
    fn loud_highs_are_turned_down_and_lows_are_kept_in_split_mode() {
        let rate = 48_000.0;
        let mut deesser = Deesser::new();
        deesser.prepare(rate);
        deesser.set(THRESHOLD, -40.0);
        deesser.set(RANGE, 12.0);
        let input = mixed(rate, 0.3, 0.3);
        let output = through(&mut deesser, &input);
        let highs = db_of(band_rms(&output, rate, 8_000.0) / band_rms(&input, rate, 8_000.0));
        let lows = db_of(band_rms(&output, rate, 300.0) / band_rms(&input, rate, 300.0));
        assert!(highs < -6.0 && highs >= -12.5, "highs moved {highs} dB");
        assert!(lows.abs() < 1.0, "lows moved {lows} dB");
        let mut seen = [crate::Moment::default(); 20];
        deesser.history().latest(&mut seen);
        assert!(seen.iter().any(|moment| moment.reduction < -6.0));
    }

    #[test]
    fn wide_mode_turns_everything_down_and_listen_plays_only_the_highs() {
        let rate = 48_000.0;
        let mut wide = Deesser::new();
        wide.prepare(rate);
        wide.set(THRESHOLD, -40.0);
        wide.set(BAND, 1.0);
        let input = mixed(rate, 0.3, 0.3);
        let output = through(&mut wide, &input);
        let lows = db_of(band_rms(&output, rate, 300.0) / band_rms(&input, rate, 300.0));
        assert!(lows < -6.0, "wide mode kept the lows at {lows} dB");
        let mut listen = Deesser::new();
        listen.prepare(rate);
        listen.set(LISTEN, 1.0);
        let heard = through(&mut listen, &input);
        assert!(band_rms(&heard, rate, 300.0) < band_rms(&input, rate, 300.0) * 0.05);
        assert!(band_rms(&heard, rate, 8_000.0) > band_rms(&input, rate, 8_000.0) * 0.7);
    }

    #[test]
    fn the_curve_starts_soft_and_stops_at_the_range() {
        assert_eq!(reduction_db(-50.0, -30.0, 10.0), 0.0);
        assert!(reduction_db(-30.0, -30.0, 10.0) < 0.0);
        assert_eq!(reduction_db(0.0, -30.0, 10.0), -10.0);
        assert_eq!(reduction_db(0.0, -30.0, 0.0), 0.0);
    }
}
