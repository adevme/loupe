use std::sync::{Arc, OnceLock};

use crate::biquad::{Coefficients, Shape, BUTTERWORTH};
use crate::scope::Scope;
use crate::smooth::Smoothed;
use crate::{gain_of, Effect, Frame, Param, Unit, Values, DEFAULT_RATE};

pub const BANDS: usize = 24;
const PER_BAND: usize = 7;
pub const OUTPUT_KNOB: usize = BANDS * PER_BAND;
const COUNT: usize = OUTPUT_KNOB + 1;
const MOST_SECTIONS: usize = 8;
const SCOPE_LENGTH: usize = 1 << 14;
const TILT_Q: f32 = 0.5;
const SILENT_STATE: f64 = 1e-30;

pub const SHAPES: [&str; 8] = ["Bell", "Low shelf", "Low cut", "High shelf", "High cut", "Notch", "Band pass", "Tilt shelf"];
pub const SLOPES: [&str; 8] = ["6 dB/oct", "12 dB/oct", "18 dB/oct", "24 dB/oct", "36 dB/oct", "48 dB/oct", "72 dB/oct", "96 dB/oct"];
const SLOPE_ORDERS: [usize; 8] = [1, 2, 3, 4, 6, 8, 12, 16];
pub const PLACES: [&str; 5] = ["Stereo", "Left", "Right", "Mid", "Side"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BandShape {
    Bell,
    LowShelf,
    LowCut,
    HighShelf,
    HighCut,
    Notch,
    BandPass,
    TiltShelf,
}

impl BandShape {
    const ALL: [BandShape; 8] = [
        BandShape::Bell,
        BandShape::LowShelf,
        BandShape::LowCut,
        BandShape::HighShelf,
        BandShape::HighCut,
        BandShape::Notch,
        BandShape::BandPass,
        BandShape::TiltShelf,
    ];

    pub fn from_index(index: f32) -> Self {
        Self::ALL[(index.max(0.0) as usize).min(Self::ALL.len() - 1)]
    }

    pub fn index(self) -> f32 {
        Self::ALL.iter().position(|shape| *shape == self).unwrap_or(0) as f32
    }

    pub fn has_gain(self) -> bool {
        matches!(self, BandShape::Bell | BandShape::LowShelf | BandShape::HighShelf | BandShape::TiltShelf)
    }

    pub fn has_slope(self) -> bool {
        matches!(self, BandShape::LowCut | BandShape::HighCut)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Stereo,
    Left,
    Right,
    Mid,
    Side,
}

impl Place {
    fn from_index(index: f32) -> Self {
        [Place::Stereo, Place::Left, Place::Right, Place::Mid, Place::Side][(index.max(0.0) as usize).min(4)]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Knob {
    On,
    Shape,
    Freq,
    Gain,
    Q,
    Slope,
    Place,
}

pub fn knob(band: usize, which: Knob) -> usize {
    band * PER_BAND + which as usize
}

fn params() -> &'static [Param] {
    static PARAMS: OnceLock<Vec<Param>> = OnceLock::new();
    PARAMS.get_or_init(|| {
        let text = |value: String| -> &'static str { Box::leak(value.into_boxed_str()) };
        let mut all = Vec::with_capacity(COUNT);
        for band in 1..=BANDS {
            let id = |what: &str| text(format!("band_{band}_{what}"));
            let name = |what: &str| text(format!("Band {band} {what}"));
            all.push(Param::new(id("on"), name("on"), 0.0, 1.0, 0.0, Unit::Switch));
            all.push(Param::choice(id("shape"), name("shape"), &SHAPES, 0));
            all.push(Param::new(id("freq"), name("freq"), 10.0, 30_000.0, 1000.0, Unit::Hertz));
            all.push(Param::new(id("gain"), name("gain"), -30.0, 30.0, 0.0, Unit::Decibels));
            all.push(Param::new(id("q"), name("Q"), 0.025, 40.0, 1.0, Unit::Ratio));
            all.push(Param::choice(id("slope"), name("slope"), &SLOPES, 1));
            all.push(Param::choice(id("place"), name("placement"), &PLACES, 0));
        }
        all.push(Param::new("output", "Output", -36.0, 36.0, 0.0, Unit::Decibels));
        all
    })
}

#[derive(Clone, Copy, Default)]
struct Section {
    c: [f64; 5],
    first: [f64; 2],
    second: [f64; 2],
}

impl Section {
    #[inline]
    fn run(&mut self, channel: usize, x: f64) -> f64 {
        let [b0, b1, b2, a1, a2] = self.c;
        let y = b0 * x + self.first[channel];
        self.first[channel] = b1 * x - a1 * y + self.second[channel];
        self.second[channel] = b2 * x - a2 * y;
        y
    }

    fn settle(&mut self) {
        for value in self.first.iter_mut().chain(self.second.iter_mut()) {
            if value.abs() < SILENT_STATE {
                *value = 0.0;
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Band {
    active: bool,
    shape: BandShape,
    place: Place,
    designs: [Coefficients; MOST_SECTIONS],
    count: usize,
    sections: [Section; MOST_SECTIONS],
}

impl Band {
    const IDLE: Band = Band {
        active: false,
        shape: BandShape::Bell,
        place: Place::Stereo,
        designs: [Coefficients::PASS; MOST_SECTIONS],
        count: 0,
        sections: [Section { c: [1.0, 0.0, 0.0, 0.0, 0.0], first: [0.0; 2], second: [0.0; 2] }; MOST_SECTIONS],
    };

    #[inline]
    fn run(&mut self, channel: usize, x: f64) -> f64 {
        self.sections[..self.count].iter_mut().fold(x, |x, section| section.run(channel, x))
    }

    fn process(&mut self, frame: &mut Frame) {
        let (left, right) = (frame[0] as f64, frame[1] as f64);
        let (left, right) = match self.place {
            Place::Stereo => (self.run(0, left), self.run(1, right)),
            Place::Left => (self.run(0, left), right),
            Place::Right => (left, self.run(1, right)),
            Place::Mid => {
                let (mid, side) = ((left + right) * 0.5, (left - right) * 0.5);
                let mid = self.run(0, mid);
                (mid + side, mid - side)
            }
            Place::Side => {
                let (mid, side) = ((left + right) * 0.5, (left - right) * 0.5);
                let side = self.run(0, side);
                (mid + side, mid - side)
            }
        };
        *frame = [left as f32, right as f32];
    }
}

pub fn design(shape: BandShape, rate: f32, hz: f32, gain_db: f32, q: f32, slope: usize) -> ([Coefficients; MOST_SECTIONS], usize) {
    let mut designs = [Coefficients::PASS; MOST_SECTIONS];
    let count = match shape {
        BandShape::Bell => {
            designs[0] = Coefficients::design(Shape::Bell, rate, hz, q, gain_db);
            1
        }
        BandShape::LowShelf => {
            designs[0] = Coefficients::design(Shape::LowShelf, rate, hz, q.min(4.0), gain_db);
            1
        }
        BandShape::HighShelf => {
            designs[0] = Coefficients::design(Shape::HighShelf, rate, hz, q.min(4.0), gain_db);
            1
        }
        BandShape::TiltShelf => {
            designs[0] = Coefficients::design(Shape::LowShelf, rate, hz, TILT_Q, -gain_db / 2.0);
            designs[1] = Coefficients::design(Shape::HighShelf, rate, hz, TILT_Q, gain_db / 2.0);
            2
        }
        BandShape::Notch => {
            designs[0] = Coefficients::design(Shape::Notch, rate, hz, q, 0.0);
            1
        }
        BandShape::BandPass => {
            designs[0] = Coefficients::design(Shape::BandPass, rate, hz, q, 0.0);
            1
        }
        BandShape::LowCut | BandShape::HighCut => {
            let kind = if shape == BandShape::LowCut { Shape::LowCut } else { Shape::HighCut };
            let order = SLOPE_ORDERS[slope.min(SLOPE_ORDERS.len() - 1)];
            let pairs = order / 2;
            let mut count = 0;
            for k in 1..=pairs {
                let step = if order % 2 == 0 { (2 * k - 1) as f32 / (2 * order) as f32 } else { k as f32 / order as f32 };
                let angle = step * std::f32::consts::PI;
                let mut section_q = 1.0 / (2.0 * angle.cos());
                if k == pairs {
                    section_q *= q / BUTTERWORTH;
                }
                designs[count] = Coefficients::design(kind, rate, hz, section_q, 0.0);
                count += 1;
            }
            if order % 2 == 1 {
                designs[count] = Coefficients::gentle(kind, rate, hz);
                count += 1;
            }
            count
        }
    };
    (designs, count)
}

pub struct Scopes {
    pub before: Scope,
    pub after: Scope,
}

pub struct Equalizer {
    values: Values<COUNT>,
    rate: f32,
    bands: [Band; BANDS],
    output: Smoothed,
    scopes: Arc<Scopes>,
}

impl Equalizer {
    pub fn new() -> Self {
        let mut eq = Self {
            values: Values::new(params()),
            rate: DEFAULT_RATE,
            bands: [Band::IDLE; BANDS],
            output: Smoothed::new(1.0),
            scopes: Arc::new(Scopes { before: Scope::new(SCOPE_LENGTH), after: Scope::new(SCOPE_LENGTH) }),
        };
        eq.redesign();
        eq.output.snap();
        eq
    }

    pub fn rate(&self) -> f32 {
        self.rate
    }

    pub fn scopes(&self) -> Arc<Scopes> {
        self.scopes.clone()
    }

    fn settings(&self, band: usize) -> (bool, BandShape, f32, f32, f32, usize, Place) {
        let v = |which| self.values.get(knob(band, which));
        let shape = BandShape::from_index(v(Knob::Shape));
        let flat = shape.has_gain() && v(Knob::Gain) == 0.0;
        (v(Knob::On) > 0.5 && !flat, shape, v(Knob::Freq), v(Knob::Gain), v(Knob::Q), v(Knob::Slope) as usize, Place::from_index(v(Knob::Place)))
    }

    fn redesign(&mut self) {
        for index in 0..BANDS {
            let (active, shape, hz, gain_db, q, slope, place) = self.settings(index);
            let (designs, count) = design(shape, self.rate, hz, gain_db, q, slope);
            let band = &mut self.bands[index];
            let restart = !band.active || band.shape != shape || band.count != count || band.place != place;
            band.active = active;
            band.shape = shape;
            band.place = place;
            band.designs = designs;
            band.count = count;
            for (section, design) in band.sections.iter_mut().zip(designs) {
                section.c = design.wide();
                if restart {
                    section.first = [0.0; 2];
                    section.second = [0.0; 2];
                }
            }
        }
        self.output.aim(gain_of(self.values.get(OUTPUT_KNOB)));
    }

    pub fn band_response_db(&self, band: usize, hz: f32) -> f32 {
        let (_, shape, freq, gain_db, q, slope, _) = self.settings(band);
        let (designs, count) = design(shape, self.rate, freq, gain_db, q, slope);
        designs[..count].iter().map(|c| c.response_db(self.rate, hz)).sum()
    }

    pub fn response_db(&self, hz: f32) -> f32 {
        let bands: f32 = (0..BANDS).filter(|band| self.settings(*band).0).map(|band| self.band_response_db(band, hz)).sum();
        bands + self.values.get(OUTPUT_KNOB)
    }

    pub fn band_in_use(&self, band: usize) -> bool {
        self.values.get(knob(band, Knob::On)) > 0.5
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
        params()
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
        self.values.take_change();
        self.output.snap();
        self.reset();
    }

    fn reset(&mut self) {
        for band in &mut self.bands {
            for section in &mut band.sections {
                section.first = [0.0; 2];
                section.second = [0.0; 2];
            }
        }
    }

    fn process(&mut self, audio: &mut [Frame]) {
        if self.values.take_change() {
            self.redesign();
        }
        let scopes = self.scopes.clone();
        let mut at = scopes.after.writer();
        for frame in audio.iter_mut() {
            scopes.before.put(at, (frame[0] + frame[1]) * 0.5);
            let output = self.output.next();
            for band in self.bands.iter_mut().filter(|band| band.active) {
                band.process(frame);
            }
            frame[0] *= output;
            frame[1] *= output;
            scopes.after.put(at, (frame[0] + frame[1]) * 0.5);
            at = at.wrapping_add(1);
        }
        scopes.before.publish(at);
        scopes.after.publish(at);
        for band in self.bands.iter_mut().filter(|band| band.active) {
            for section in &mut band.sections[..band.count] {
                section.settle();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{in_blocks, noise, rms, sine};

    const RATE: f32 = 48_000.0;

    fn band(eq: &mut Equalizer, index: usize, shape: BandShape, hz: f32, gain: f32) {
        eq.set(knob(index, Knob::On), 1.0);
        eq.set(knob(index, Knob::Shape), shape.index());
        eq.set(knob(index, Knob::Freq), hz);
        eq.set(knob(index, Knob::Gain), gain);
    }

    fn ready() -> Equalizer {
        let mut eq = Equalizer::new();
        eq.prepare(RATE);
        eq
    }

    #[test]
    fn untouched_it_passes_sound_through_exactly() {
        let mut eq = ready();
        band(&mut eq, 3, BandShape::Bell, 1000.0, 0.0);
        let input = noise(0.7, 10_000, 3);
        let mut output = input.clone();
        in_blocks(&mut eq, &mut output, 256);
        assert_eq!(output, input);
    }

    #[test]
    fn a_boosted_bell_lifts_its_frequency_by_that_much() {
        let mut eq = ready();
        band(&mut eq, 0, BandShape::Bell, 1000.0, 6.0);
        let input = sine(RATE, 1000.0, 0.25, 48_000);
        let mut output = input.clone();
        in_blocks(&mut eq, &mut output, 512);
        let lift = crate::db_of(rms(&output[24_000..]) / rms(&input[24_000..]));
        assert!((lift - 6.0).abs() < 0.05, "lift {lift}");
        assert!((eq.response_db(1000.0) - 6.0).abs() < 0.05);
    }

    #[test]
    fn cut_slopes_fall_as_steeply_as_they_say() {
        for (slope, order) in SLOPE_ORDERS.iter().enumerate() {
            let mut eq = ready();
            band(&mut eq, 0, BandShape::LowCut, 1000.0, 0.0);
            eq.set(knob(0, Knob::Slope), slope as f32);
            eq.set(knob(0, Knob::Q), BUTTERWORTH);
            let at_cutoff = eq.response_db(1000.0);
            assert!((at_cutoff + 3.01).abs() < 0.1, "{order}: {at_cutoff} at the cutoff");
            let octave_down = eq.response_db(62.5) - eq.response_db(125.0);
            let wanted = -6.02 * *order as f32;
            assert!((octave_down - wanted).abs() < 0.6, "{order}: fell {octave_down} per octave, wanted {wanted}");
            assert!(eq.response_db(16_000.0).abs() < 0.05);
        }
    }

    #[test]
    fn the_steepest_cut_low_down_stays_stable_and_clean() {
        let mut eq = ready();
        band(&mut eq, 0, BandShape::LowCut, 20.0, 0.0);
        eq.set(knob(0, Knob::Slope), 7.0);
        let input = sine(RATE, 1000.0, 0.5, 96_000);
        let mut output = input.clone();
        in_blocks(&mut eq, &mut output, 128);
        assert!(output.iter().flatten().all(|sample| sample.is_finite()));
        let change = crate::db_of(rms(&output[48_000..]) / rms(&input[48_000..]));
        assert!(change.abs() < 0.01, "{change}");
    }

    #[test]
    fn placement_keeps_the_other_parts_untouched() {
        let mut eq = ready();
        band(&mut eq, 0, BandShape::Bell, 1000.0, 12.0);
        eq.set(knob(0, Knob::Place), 1.0);
        let input = noise(0.5, 6000, 12);
        let mut output = input.clone();
        in_blocks(&mut eq, &mut output, 200);
        assert!(output.iter().zip(&input).all(|(out, before)| out[1] == before[1]), "right untouched by a left band");
        assert!(output.iter().zip(&input).any(|(out, before)| out[0] != before[0]));

        let mut eq = ready();
        band(&mut eq, 0, BandShape::Bell, 1000.0, 12.0);
        eq.set(knob(0, Knob::Place), 4.0);
        let mono: Vec<Frame> = input.iter().map(|frame| [frame[0], frame[0]]).collect();
        let mut output = mono.clone();
        in_blocks(&mut eq, &mut output, 200);
        assert_eq!(output, mono, "a side band leaves a mono sound alone");
    }

    #[test]
    fn notch_tilt_and_band_pass_shape_the_curve() {
        let mut eq = ready();
        band(&mut eq, 0, BandShape::Notch, 3000.0, 0.0);
        eq.set(knob(0, Knob::Q), 8.0);
        assert!(eq.response_db(3000.0) < -40.0);
        let mut eq = ready();
        band(&mut eq, 0, BandShape::TiltShelf, 1000.0, 6.0);
        assert!((eq.response_db(20.0) + 3.0).abs() < 0.3 && (eq.response_db(20_000.0) - 3.0).abs() < 0.3);
        let mut eq = ready();
        band(&mut eq, 0, BandShape::BandPass, 1000.0, 0.0);
        assert!(eq.response_db(1000.0).abs() < 0.01 && eq.response_db(100.0) < -15.0);
    }

    #[test]
    fn the_scopes_hear_before_and_after() {
        let mut eq = ready();
        band(&mut eq, 0, BandShape::Bell, 1000.0, -12.0);
        let mut audio = sine(RATE, 1000.0, 0.5, 8192);
        in_blocks(&mut eq, &mut audio, 512);
        let scopes = eq.scopes();
        let (mut before, mut after) = (vec![0.0; 2048], vec![0.0; 2048]);
        scopes.before.latest(&mut before);
        scopes.after.latest(&mut after);
        let level = |samples: &[f32]| (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
        assert!((crate::db_of(level(&after) / level(&before)) + 12.0).abs() < 0.2);
    }
}
