use crate::biquad::{Biquad, Coefficients, Shape};
use crate::smooth::Smoothed;
use crate::{gain_of, Effect, Frame, Param, Unit, Values, DEFAULT_RATE};

const PARAMS: [Param; 18] = [
    Param::new("low_cut", "Low cut", 0.0, 1.0, 0.0, Unit::Switch),
    Param::new("low_cut_hz", "Low cut freq", 20.0, 1000.0, 80.0, Unit::Hertz),
    Param::new("low_shelf_hz", "Low shelf freq", 20.0, 1000.0, 100.0, Unit::Hertz),
    Param::new("low_shelf_db", "Low shelf gain", -18.0, 18.0, 0.0, Unit::Decibels),
    Param::new("bell_1_hz", "Bell 1 freq", 30.0, 18_000.0, 250.0, Unit::Hertz),
    Param::new("bell_1_db", "Bell 1 gain", -18.0, 18.0, 0.0, Unit::Decibels),
    Param::new("bell_1_q", "Bell 1 Q", 0.1, 18.0, 1.0, Unit::Ratio),
    Param::new("bell_2_hz", "Bell 2 freq", 30.0, 18_000.0, 1000.0, Unit::Hertz),
    Param::new("bell_2_db", "Bell 2 gain", -18.0, 18.0, 0.0, Unit::Decibels),
    Param::new("bell_2_q", "Bell 2 Q", 0.1, 18.0, 1.0, Unit::Ratio),
    Param::new("bell_3_hz", "Bell 3 freq", 30.0, 18_000.0, 4000.0, Unit::Hertz),
    Param::new("bell_3_db", "Bell 3 gain", -18.0, 18.0, 0.0, Unit::Decibels),
    Param::new("bell_3_q", "Bell 3 Q", 0.1, 18.0, 1.0, Unit::Ratio),
    Param::new("high_shelf_hz", "High shelf freq", 1000.0, 20_000.0, 8000.0, Unit::Hertz),
    Param::new("high_shelf_db", "High shelf gain", -18.0, 18.0, 0.0, Unit::Decibels),
    Param::new("high_cut", "High cut", 0.0, 1.0, 0.0, Unit::Switch),
    Param::new("high_cut_hz", "High cut freq", 1000.0, 20_000.0, 16_000.0, Unit::Hertz),
    Param::new("output", "Output", -24.0, 24.0, 0.0, Unit::Decibels),
];
const BANDS: usize = 7;
const OUTPUT: usize = 17;

#[derive(Clone, Copy)]
struct Band {
    shape: Shape,
    coefficients: Coefficients,
    active: bool,
    filter: Biquad,
}

pub struct Equalizer {
    values: Values<18>,
    rate: f32,
    bands: [Band; BANDS],
    output: Smoothed,
}

impl Equalizer {
    pub fn new() -> Self {
        let band = |shape| Band { shape, coefficients: Coefficients::PASS, active: false, filter: Biquad::default() };
        let mut eq = Self {
            values: Values::new(&PARAMS),
            rate: DEFAULT_RATE,
            bands: [
                band(Shape::LowCut),
                band(Shape::LowShelf),
                band(Shape::Bell),
                band(Shape::Bell),
                band(Shape::Bell),
                band(Shape::HighShelf),
                band(Shape::HighCut),
            ],
            output: Smoothed::new(1.0),
        };
        eq.redesign();
        eq
    }

    fn band_settings(&self, band: usize) -> (bool, f32, f32, f32) {
        let v = |index| self.values.get(index);
        match band {
            0 => (v(0) > 0.5, v(1), 0.0, 0.0),
            1 => (v(3) != 0.0, v(2), 0.707, v(3)),
            2 => (v(5) != 0.0, v(4), v(6), v(5)),
            3 => (v(8) != 0.0, v(7), v(9), v(8)),
            4 => (v(11) != 0.0, v(10), v(12), v(11)),
            5 => (v(14) != 0.0, v(13), 0.707, v(14)),
            _ => (v(15) > 0.5, v(16), 0.0, 0.0),
        }
    }

    fn redesign(&mut self) {
        for index in 0..BANDS {
            let (active, hz, q, gain_db) = self.band_settings(index);
            let band = &mut self.bands[index];
            if active && !band.active {
                band.filter.reset();
            }
            band.active = active;
            band.coefficients = Coefficients::design(band.shape, self.rate, hz, q, gain_db);
        }
        self.output.aim(gain_of(self.values.get(OUTPUT)));
    }

    pub fn response_db(&self, hz: f32) -> f32 {
        let bands: f32 = self.bands.iter().filter(|band| band.active).map(|band| band.coefficients.response_db(self.rate, hz)).sum();
        bands + self.values.get(OUTPUT)
    }
}

impl Default for Equalizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for Equalizer {
    fn name(&self) -> &'static str {
        "Loupe EQ"
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
        self.output.prepare(rate);
        self.values.mark_changed();
        self.redesign();
        self.output.snap();
        self.reset();
    }

    fn reset(&mut self) {
        for band in &mut self.bands {
            band.filter.reset();
        }
    }

    fn process(&mut self, audio: &mut [Frame]) {
        if self.values.take_change() {
            self.redesign();
        }
        for frame in audio.iter_mut() {
            let output = self.output.next();
            for band in self.bands.iter_mut().filter(|band| band.active) {
                frame[0] = band.filter.run(&band.coefficients, 0, frame[0]);
                frame[1] = band.filter.run(&band.coefficients, 1, frame[1]);
            }
            frame[0] *= output;
            frame[1] *= output;
        }
        for band in &mut self.bands {
            band.filter.settle();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{in_blocks, noise, rms, sine};

    #[test]
    fn untouched_it_passes_sound_through_exactly() {
        let mut eq = Equalizer::new();
        eq.prepare(48_000.0);
        let input = noise(0.7, 10_000, 3);
        let mut output = input.clone();
        in_blocks(&mut eq, &mut output, 256);
        assert_eq!(output, input);
    }

    #[test]
    fn a_boosted_bell_lifts_its_frequency_by_that_much() {
        let mut eq = Equalizer::new();
        eq.prepare(48_000.0);
        assert!(eq.set_by_id("bell_2_db", 6.0));
        assert!(eq.set_by_id("bell_2_hz", 1000.0));
        let input = sine(48_000.0, 1000.0, 0.25, 48_000);
        let mut output = input.clone();
        in_blocks(&mut eq, &mut output, 512);
        let lift = crate::db_of(rms(&output[24_000..]) / rms(&input[24_000..]));
        assert!((lift - 6.0).abs() < 0.05, "lift {lift}");
        assert!((eq.response_db(1000.0) - 6.0).abs() < 0.05);
    }

    #[test]
    fn the_low_cut_removes_rumble_only_when_switched_on() {
        let mut eq = Equalizer::new();
        eq.prepare(48_000.0);
        eq.set_by_id("low_cut_hz", 200.0);
        assert_eq!(eq.response_db(30.0), 0.0);
        eq.set_by_id("low_cut", 1.0);
        let mut rumble = sine(48_000.0, 30.0, 0.5, 48_000);
        in_blocks(&mut eq, &mut rumble, 128);
        assert!(crate::db_of(rms(&rumble[24_000..]) / (0.5 * std::f32::consts::FRAC_1_SQRT_2)) < -30.0);
    }
}
