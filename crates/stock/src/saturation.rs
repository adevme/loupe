use crate::biquad::{Biquad, Coefficients, Shape, BUTTERWORTH};
use crate::smooth::Smoothed;
use crate::{decay_per_sample, gain_of, settled, Effect, Frame, Param, Unit, Values, DEFAULT_RATE};

pub const STYLES: [&str; 4] = ["Tape", "Tube", "Soft clip", "Hard clip"];
const PARAMS: [Param; 6] = [
    Param::new("drive", "Drive", 0.0, 36.0, 6.0, Unit::Decibels),
    Param::choice("style", "Style", &STYLES, 0),
    Param::new("tone", "Tone", 1_000.0, 20_000.0, 20_000.0, Unit::Hertz),
    Param::new("mix", "Mix", 0.0, 100.0, 100.0, Unit::Percent),
    Param::new("output", "Output", -24.0, 12.0, 0.0, Unit::Decibels),
    Param::new("auto_gain", "Auto gain", 0.0, 1.0, 1.0, Unit::Switch),
];
pub const DRIVE: usize = 0;
pub const STYLE: usize = 1;
pub const TONE: usize = 2;
pub const MIX: usize = 3;
pub const OUTPUT: usize = 4;
pub const AUTO_GAIN: usize = 5;
const OVERSAMPLE: usize = 4;
const STAGES: usize = 3;
const BAND_EDGE: f32 = 0.45;
const TUBE_BIAS: f32 = 0.25;
const DC_HZ: f32 = 8.0;
const LEVEL_MS: f32 = 300.0;
const QUIETEST_LEVEL: f32 = 1e-6;
const MOST_MAKEUP: f32 = 4.0;
const LEAST_MAKEUP: f32 = 1.0 / 16.0;
const TONE_OFF_HZ: f32 = 19_500.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Tape,
    Tube,
    SoftClip,
    HardClip,
}

impl Style {
    fn of(value: f32) -> Self {
        match value.round() as i32 {
            1 => Self::Tube,
            2 => Self::SoftClip,
            3 => Self::HardClip,
            _ => Self::Tape,
        }
    }
}

pub fn shape(style: Style, x: f32) -> f32 {
    match style {
        Style::Tape => x.tanh(),
        Style::Tube => (x + TUBE_BIAS).tanh() - TUBE_BIAS.tanh(),
        Style::SoftClip => {
            let x = x.clamp(-1.5, 1.5);
            x - 4.0 * x * x * x / 27.0
        }
        Style::HardClip => x.clamp(-1.0, 1.0),
    }
}

pub fn curve_of(values: &[f32], x: f32) -> f32 {
    let drive = gain_of(values.get(DRIVE).copied().unwrap_or(0.0));
    shape(Style::of(values.get(STYLE).copied().unwrap_or(0.0)), x * drive)
}

struct Oversampler {
    up: Coefficients,
    down: Coefficients,
    up_filters: [Biquad; STAGES],
    down_filters: [Biquad; STAGES],
}

impl Oversampler {
    fn new(rate: f32) -> Self {
        let fast = rate * OVERSAMPLE as f32;
        let edge = rate * BAND_EDGE;
        let c = Coefficients::design(Shape::HighCut, fast, edge, BUTTERWORTH, 0.0);
        Self { up: c, down: c, up_filters: Default::default(), down_filters: Default::default() }
    }

    fn reset(&mut self) {
        for filter in self.up_filters.iter_mut().chain(self.down_filters.iter_mut()) {
            filter.reset();
        }
    }

    fn settle(&mut self) {
        for filter in self.up_filters.iter_mut().chain(self.down_filters.iter_mut()) {
            filter.settle();
        }
    }

    fn run(&mut self, channel: usize, x: f32, mut bend: impl FnMut(f32) -> f32) -> f32 {
        let mut out = 0.0;
        for step in 0..OVERSAMPLE {
            let mut v = if step == 0 { x * OVERSAMPLE as f32 } else { 0.0 };
            for filter in self.up_filters.iter_mut() {
                v = filter.run(&self.up, channel, v);
            }
            v = bend(v);
            for filter in self.down_filters.iter_mut() {
                v = filter.run(&self.down, channel, v);
            }
            if step == 0 {
                out = v;
            }
        }
        out
    }
}

pub struct Saturation {
    values: Values<6>,
    rate: f32,
    style: Style,
    drive: Smoothed,
    mix: Smoothed,
    output: Smoothed,
    auto_gain: bool,
    tone_on: bool,
    tone: Coefficients,
    tone_filter: Biquad,
    dc: f32,
    dc_in: Frame,
    dc_out: Frame,
    oversampler: Oversampler,
    level_in: f32,
    level_out: f32,
    level_fall: f32,
    makeup: f32,
}

impl Saturation {
    pub fn new() -> Self {
        let mut saturation = Self {
            values: Values::new(&PARAMS),
            rate: DEFAULT_RATE,
            style: Style::Tape,
            drive: Smoothed::new(1.0),
            mix: Smoothed::new(1.0),
            output: Smoothed::new(1.0),
            auto_gain: true,
            tone_on: false,
            tone: Coefficients::PASS,
            tone_filter: Biquad::default(),
            dc: 0.0,
            dc_in: [0.0; 2],
            dc_out: [0.0; 2],
            oversampler: Oversampler::new(DEFAULT_RATE),
            level_in: 0.0,
            level_out: 0.0,
            level_fall: 0.0,
            makeup: 1.0,
        };
        saturation.prepare(DEFAULT_RATE);
        saturation
    }

    fn read_knobs(&mut self) {
        self.drive.aim(gain_of(self.values.get(DRIVE)));
        self.style = Style::of(self.values.get(STYLE));
        let tone = self.values.get(TONE);
        self.tone_on = tone < TONE_OFF_HZ;
        self.tone = Coefficients::design(Shape::HighCut, self.rate, tone, BUTTERWORTH, 0.0);
        self.mix.aim(self.values.get(MIX) / 100.0);
        self.output.aim(gain_of(self.values.get(OUTPUT)));
        self.auto_gain = self.values.get(AUTO_GAIN) > 0.5;
    }
}

impl Default for Saturation {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for Saturation {
    fn name(&self) -> &'static str {
        "Loupe Saturation"
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
        self.oversampler = Oversampler::new(rate);
        self.dc = (-std::f32::consts::TAU * DC_HZ / rate).exp();
        self.level_fall = decay_per_sample(LEVEL_MS, rate);
        for smoothed in [&mut self.drive, &mut self.mix, &mut self.output] {
            smoothed.prepare(rate);
        }
        self.read_knobs();
        self.values.take_change();
        for smoothed in [&mut self.drive, &mut self.mix, &mut self.output] {
            smoothed.snap();
        }
        self.reset();
    }

    fn reset(&mut self) {
        self.oversampler.reset();
        self.tone_filter.reset();
        self.dc_in = [0.0; 2];
        self.dc_out = [0.0; 2];
        self.level_in = 0.0;
        self.level_out = 0.0;
        self.makeup = 1.0;
    }

    fn process(&mut self, audio: &mut [Frame]) {
        if self.values.take_change() {
            self.read_knobs();
        }
        let style = self.style;
        for frame in audio.iter_mut() {
            let drive = self.drive.next();
            let mix = self.mix.next();
            let output = self.output.next();
            let dry = *frame;
            let mut wet = [0.0f32; 2];
            for channel in 0..2 {
                let bent = self.oversampler.run(channel, dry[channel], |x| shape(style, x * drive));
                let blocked = bent - self.dc_in[channel] + self.dc * self.dc_out[channel];
                self.dc_in[channel] = bent;
                self.dc_out[channel] = blocked;
                wet[channel] = if self.tone_on { self.tone_filter.run(&self.tone, channel, blocked) } else { blocked };
            }
            let heard_in = dry[0] * dry[0] + dry[1] * dry[1];
            let heard_out = wet[0] * wet[0] + wet[1] * wet[1];
            self.level_in = heard_in + (self.level_in - heard_in) * self.level_fall;
            self.level_out = heard_out + (self.level_out - heard_out) * self.level_fall;
            if self.auto_gain && self.level_out > QUIETEST_LEVEL {
                let wanted = (self.level_in / self.level_out).sqrt().clamp(LEAST_MAKEUP, MOST_MAKEUP);
                self.makeup = wanted + (self.makeup - wanted) * self.level_fall;
            } else if !self.auto_gain {
                self.makeup = 1.0;
            }
            for channel in 0..2 {
                frame[channel] = (dry[channel] + (wet[channel] * self.makeup - dry[channel]) * mix) * output;
            }
        }
        self.oversampler.settle();
        self.tone_filter.settle();
        for channel in 0..2 {
            self.dc_out[channel] = settled(self.dc_out[channel]);
        }
        self.level_in = settled(self.level_in);
        self.level_out = settled(self.level_out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{in_blocks, rms, sine};

    fn harmonic(audio: &[Frame], rate: f32, hz: f32) -> f32 {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, frame) in audio.iter().enumerate() {
            let phase = std::f64::consts::TAU * hz as f64 * i as f64 / rate as f64;
            re += frame[0] as f64 * phase.cos();
            im += frame[0] as f64 * phase.sin();
        }
        ((re * re + im * im).sqrt() * 2.0 / audio.len() as f64) as f32
    }

    fn driven(style: f32, drive: f32, auto_gain: f32) -> (Vec<Frame>, Vec<Frame>) {
        let rate = 48_000.0;
        let mut saturation = Saturation::new();
        saturation.prepare(rate);
        saturation.set(STYLE, style);
        saturation.set(DRIVE, drive);
        saturation.set(AUTO_GAIN, auto_gain);
        let input = sine(rate, 200.0, 0.5, 96_000);
        let mut output = input.clone();
        in_blocks(&mut saturation, &mut output, 256);
        (input[48_000..].to_vec(), output[48_000..].to_vec())
    }

    #[test]
    fn every_style_adds_harmonics_as_drive_rises() {
        for style in 0..STYLES.len() {
            let (_, gentle) = driven(style as f32, 0.0, 0.0);
            let (_, hard) = driven(style as f32, 24.0, 0.0);
            let third = |audio: &[Frame]| harmonic(audio, 48_000.0, 600.0) / harmonic(audio, 48_000.0, 200.0);
            assert!(third(&hard) > third(&gentle) * 3.0, "{} barely changed", STYLES[style]);
            assert!(hard.iter().flatten().all(|v| v.is_finite()));
        }
    }

    #[test]
    fn tube_adds_even_harmonics_and_tape_does_not() {
        let second = |audio: &[Frame]| harmonic(audio, 48_000.0, 400.0) / harmonic(audio, 48_000.0, 200.0);
        let (_, tube) = driven(1.0, 18.0, 0.0);
        let (_, tape) = driven(0.0, 18.0, 0.0);
        assert!(second(&tube) > 0.02, "tube second harmonic {}", second(&tube));
        assert!(second(&tape) < 0.005, "tape second harmonic {}", second(&tape));
    }

    #[test]
    fn auto_gain_keeps_the_level_close_to_the_input() {
        let (input, output) = driven(3.0, 30.0, 1.0);
        let change = crate::db_of(rms(&output) / rms(&input));
        assert!(change.abs() < 1.5, "level moved {change} dB");
    }

    #[test]
    fn no_mix_returns_the_dry_sound() {
        let rate = 48_000.0;
        let mut saturation = Saturation::new();
        saturation.prepare(rate);
        saturation.set(MIX, 0.0);
        saturation.set(DRIVE, 30.0);
        let input = sine(rate, 300.0, 0.4, 9_600);
        let mut output = input.clone();
        in_blocks(&mut saturation, &mut output, 128);
        let biggest = input[2_000..].iter().zip(&output[2_000..]).map(|(a, b)| (a[0] - b[0]).abs()).fold(0.0f32, f32::max);
        assert!(biggest < 1e-5, "{biggest}");
    }

    #[test]
    fn the_curve_matches_the_sound_shape() {
        let values = [0.0, 3.0, 20_000.0, 100.0, 0.0, 1.0];
        assert_eq!(curve_of(&values, 2.0), 1.0);
        assert_eq!(curve_of(&values, -0.5), -0.5);
        assert!((shape(Style::SoftClip, 1.5) - 1.0).abs() < 1e-6);
        assert_eq!(shape(Style::Tube, 0.0), 0.0);
    }
}
