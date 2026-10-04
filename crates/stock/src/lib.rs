mod biquad;
mod chorus;
mod compressor;
mod deesser;
mod delay;
mod eq;
mod history;
mod limiter;
mod reverb;
mod saturation;
mod scope;
mod smooth;
mod transient;

pub use biquad::{Coefficients, Shape};
pub use chorus::{spread as chorus_spread, sweep as chorus_sweep, Chorus, MODES as CHORUS_MODES};
pub use compressor::{Compressor, Curve, STYLES};
pub use delay::{echo_seconds, Delay, NOTES};
pub use deesser::Deesser;
pub use eq::{design as band_design, knob, BandShape, Equalizer, Knob, Place, Scopes, BANDS, OUTPUT_KNOB, PLACES, SHAPES, SLOPES};
pub use history::{History, Moment, MOMENTS_PER_SECOND};
pub use scope::Scope;
pub use limiter::Limiter;
pub use transient::Transient;
pub use reverb::{decay_seconds, Reverb};
pub use saturation::{curve_of as saturation_curve, Saturation, STYLES as SATURATION_STYLES};

pub type Frame = [f32; 2];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Decibels,
    Hertz,
    Milliseconds,
    Seconds,
    Percent,
    Ratio,
    Width,
    Switch,
    Choice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skew {
    Linear,
    Logarithmic,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Param {
    pub id: &'static str,
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: Unit,
    pub skew: Skew,
    pub choices: &'static [&'static str],
}

impl Param {
    pub const fn new(id: &'static str, name: &'static str, min: f32, max: f32, default: f32, unit: Unit) -> Self {
        let skew = match unit {
            Unit::Hertz | Unit::Milliseconds | Unit::Seconds | Unit::Ratio if min > 0.0 => Skew::Logarithmic,
            _ => Skew::Linear,
        };
        Self { id, name, min, max, default, unit, skew, choices: &[] }
    }

    pub const fn choice(id: &'static str, name: &'static str, choices: &'static [&'static str], default: usize) -> Self {
        let max = if choices.is_empty() { 1.0 } else { (choices.len() - 1) as f32 };
        Self { id, name, min: 0.0, max, default: default as f32, unit: Unit::Choice, skew: Skew::Linear, choices }
    }

    pub fn chosen(&self, value: f32) -> Option<&'static str> {
        self.choices.get(self.clamp(value) as usize).copied()
    }

    pub fn clamp(&self, value: f32) -> f32 {
        if !value.is_finite() {
            return self.default;
        }
        let value = value.clamp(self.min, self.max);
        if matches!(self.unit, Unit::Switch | Unit::Choice) {
            value.round()
        } else {
            value
        }
    }

    pub fn from_position(&self, position: f32) -> f32 {
        let position = position.clamp(0.0, 1.0);
        let value = match self.skew {
            Skew::Linear => self.min + (self.max - self.min) * position,
            Skew::Logarithmic => self.min * (self.max / self.min).powf(position),
        };
        self.clamp(value)
    }

    pub fn to_position(&self, value: f32) -> f32 {
        let value = self.clamp(value);
        let position = match self.skew {
            Skew::Linear => (value - self.min) / (self.max - self.min),
            Skew::Logarithmic => (value / self.min).ln() / (self.max / self.min).ln(),
        };
        position.clamp(0.0, 1.0)
    }
}

pub trait Effect: Send {
    fn name(&self) -> &'static str;
    fn params(&self) -> &'static [Param];
    fn value(&self, index: usize) -> f32;
    fn set(&mut self, index: usize, value: f32);
    fn prepare(&mut self, rate: f32);
    fn reset(&mut self);
    fn process(&mut self, audio: &mut [Frame]);

    fn latency(&self) -> usize {
        0
    }

    fn scopes(&self) -> Option<std::sync::Arc<Scopes>> {
        None
    }

    fn history(&self) -> Option<std::sync::Arc<History>> {
        None
    }

    fn set_tempo(&mut self, _bpm: f32) {}

    fn set_by_id(&mut self, id: &str, value: f32) -> bool {
        match self.params().iter().position(|param| param.id == id) {
            Some(index) => {
                self.set(index, value);
                true
            }
            None => false,
        }
    }
}

pub const NAMES: [&str; 9] = ["Loupe EQ", "Loupe Compressor", "Loupe Limiter", "Loupe Delay", "Loupe Reverb", "Loupe De-esser", "Loupe Saturation", "Loupe Chorus", "Loupe Transient"];

pub fn make(name: &str) -> Option<Box<dyn Effect>> {
    Some(match name {
        "Loupe EQ" => Box::new(Equalizer::new()),
        "Loupe Compressor" => Box::new(Compressor::new()),
        "Loupe Limiter" => Box::new(Limiter::new()),
        "Loupe Delay" => Box::new(Delay::new()),
        "Loupe Reverb" => Box::new(Reverb::new()),
        "Loupe De-esser" => Box::new(Deesser::new()),
        "Loupe Saturation" => Box::new(Saturation::new()),
        "Loupe Chorus" => Box::new(Chorus::new()),
        "Loupe Transient" => Box::new(Transient::new()),
        _ => return None,
    })
}

pub(crate) const DEFAULT_RATE: f32 = 48_000.0;
const SILENCE: f32 = 1e-15;

pub(crate) fn gain_of(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

pub(crate) fn db_of(gain: f32) -> f32 {
    20.0 * gain.max(1e-12).log10()
}

pub(crate) fn settled(value: f32) -> f32 {
    if value.abs() < SILENCE {
        0.0
    } else {
        value
    }
}

pub(crate) fn decay_per_sample(milliseconds: f32, rate: f32) -> f32 {
    (-1.0 / (milliseconds.max(0.01) * 0.001 * rate)).exp()
}

pub(crate) struct Values<const N: usize> {
    params: &'static [Param],
    now: [f32; N],
    changed: bool,
}

impl<const N: usize> Values<N> {
    pub(crate) fn new(params: &'static [Param]) -> Self {
        let mut now = [0.0; N];
        for (slot, param) in now.iter_mut().zip(params) {
            *slot = param.default;
        }
        Self { params, now, changed: true }
    }

    pub(crate) fn get(&self, index: usize) -> f32 {
        self.now.get(index).copied().unwrap_or(0.0)
    }

    pub(crate) fn set(&mut self, index: usize, value: f32) {
        if let (Some(slot), Some(param)) = (self.now.get_mut(index), self.params.get(index)) {
            let value = param.clamp(value);
            if *slot != value {
                *slot = value;
                self.changed = true;
            }
        }
    }

    pub(crate) fn take_change(&mut self) -> bool {
        std::mem::replace(&mut self.changed, false)
    }

    pub(crate) fn mark_changed(&mut self) {
        self.changed = true;
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::Frame;

    pub fn sine(rate: f32, hz: f32, level: f32, frames: usize) -> Vec<Frame> {
        (0..frames)
            .map(|i| {
                let value = level * (std::f32::consts::TAU * hz * i as f32 / rate).sin();
                [value, value]
            })
            .collect()
    }

    pub fn noise(level: f32, frames: usize, seed: u32) -> Vec<Frame> {
        let mut state = seed.max(1);
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state as f32 / u32::MAX as f32) * 2.0 - 1.0
        };
        (0..frames).map(|_| [level * next(), level * next()]).collect()
    }

    pub fn peak(audio: &[Frame]) -> f32 {
        audio.iter().flat_map(|frame| frame.iter()).fold(0.0f32, |most, sample| most.max(sample.abs()))
    }

    pub fn rms(audio: &[Frame]) -> f32 {
        let sum: f64 = audio.iter().map(|frame| (frame[0] as f64).powi(2)).sum();
        (sum / audio.len().max(1) as f64).sqrt() as f32
    }

    pub fn in_blocks(effect: &mut dyn super::Effect, audio: &mut [Frame], block: usize) {
        for part in audio.chunks_mut(block) {
            effect.process(part);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_stock_plugin_can_be_made_by_name_and_knows_its_knobs() {
        for name in NAMES {
            let mut effect = make(name).unwrap_or_else(|| panic!("{name} is missing"));
            assert_eq!(effect.name(), name);
            effect.prepare(44_100.0);
            for (index, param) in effect.params().iter().enumerate() {
                assert_eq!(effect.value(index), param.default, "{name} {}", param.id);
                assert!(param.min < param.max && (param.min..=param.max).contains(&param.default));
                effect.set(index, f32::MAX);
                assert_eq!(effect.value(index), param.max);
                effect.set(index, f32::NAN);
                assert_eq!(effect.value(index), param.default);
            }
            let mut audio = testing::noise(0.5, 4096, 7);
            effect.process(&mut audio);
            assert!(audio.iter().flatten().all(|sample| sample.is_finite()), "{name} made a bad sample");
        }
        assert!(make("Fog").is_none());
    }

    #[test]
    fn every_knob_maps_to_a_real_position() {
        for name in NAMES {
            let effect = make(name).unwrap();
            for param in effect.params() {
                for value in [param.min, param.default, param.max] {
                    let position = param.to_position(value);
                    assert!(position.is_finite(), "{name} {} at {value}", param.id);
                    assert!((param.from_position(position) - value).abs() <= (param.max - param.min) * 1e-4, "{name} {}", param.id);
                }
            }
        }
    }

    #[test]
    fn knob_positions_round_trip_on_both_scales() {
        let log = Param::new("f", "Freq", 20.0, 20_000.0, 1000.0, Unit::Hertz);
        assert!((log.from_position(0.5) - 632.4555).abs() < 0.01);
        assert!((log.to_position(log.from_position(0.3)) - 0.3).abs() < 1e-5);
        let switch = Param::new("on", "On", 0.0, 1.0, 1.0, Unit::Switch);
        assert_eq!(switch.clamp(0.4), 0.0);
        assert_eq!(switch.clamp(0.6), 1.0);
    }
}
