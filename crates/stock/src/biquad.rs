use std::f32::consts::TAU;

use crate::{settled, Frame};

pub const BUTTERWORTH: f32 = std::f32::consts::FRAC_1_SQRT_2;
const LOWEST_HZ: f32 = 10.0;
const HIGHEST_OF_NYQUIST: f32 = 0.98;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    LowCut,
    LowShelf,
    Bell,
    HighShelf,
    HighCut,
    Notch,
    BandPass,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coefficients {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Coefficients {
    pub const PASS: Self = Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 };

    pub fn design(shape: Shape, rate: f32, hz: f32, q: f32, gain_db: f32) -> Self {
        let hz = hz.clamp(LOWEST_HZ, rate * 0.5 * HIGHEST_OF_NYQUIST);
        let q = q.max(0.025);
        let (sin, cos) = (TAU * hz / rate).sin_cos();
        let alpha = sin / (2.0 * q);
        let a = 10f32.powf(gain_db / 40.0);
        let lift = 2.0 * a.sqrt() * alpha;
        let (b0, b1, b2, a0, a1, a2) = match shape {
            Shape::LowCut => ((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            Shape::HighCut => ((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            Shape::Bell => (1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a),
            Shape::Notch => (1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            Shape::BandPass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            Shape::LowShelf => (
                a * ((a + 1.0) - (a - 1.0) * cos + lift),
                2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                a * ((a + 1.0) - (a - 1.0) * cos - lift),
                (a + 1.0) + (a - 1.0) * cos + lift,
                -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                (a + 1.0) + (a - 1.0) * cos - lift,
            ),
            Shape::HighShelf => (
                a * ((a + 1.0) + (a - 1.0) * cos + lift),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                a * ((a + 1.0) + (a - 1.0) * cos - lift),
                (a + 1.0) - (a - 1.0) * cos + lift,
                2.0 * ((a - 1.0) - (a + 1.0) * cos),
                (a + 1.0) - (a - 1.0) * cos - lift,
            ),
        };
        Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
    }

    pub fn gentle(shape: Shape, rate: f32, hz: f32) -> Self {
        let hz = hz.clamp(LOWEST_HZ, rate * 0.5 * HIGHEST_OF_NYQUIST);
        let k = (std::f32::consts::PI * hz / rate).tan();
        let a1 = (k - 1.0) / (k + 1.0);
        match shape {
            Shape::LowCut => Self { b0: 1.0 / (1.0 + k), b1: -1.0 / (1.0 + k), b2: 0.0, a1, a2: 0.0 },
            Shape::HighCut => Self { b0: k / (1.0 + k), b1: k / (1.0 + k), b2: 0.0, a1, a2: 0.0 },
            _ => Self::PASS,
        }
    }

    pub fn wide(&self) -> [f64; 5] {
        [self.b0, self.b1, self.b2, self.a1, self.a2].map(f64::from)
    }

    pub fn response_db(&self, rate: f32, hz: f32) -> f32 {
        let w = (TAU * hz / rate) as f64;
        let (sin1, cos1) = w.sin_cos();
        let (sin2, cos2) = (2.0 * w).sin_cos();
        let (b0, b1, b2, a1, a2) = (self.b0 as f64, self.b1 as f64, self.b2 as f64, self.a1 as f64, self.a2 as f64);
        let top = (b0 + b1 * cos1 + b2 * cos2).powi(2) + (b1 * sin1 + b2 * sin2).powi(2);
        let bottom = (1.0 + a1 * cos1 + a2 * cos2).powi(2) + (a1 * sin1 + a2 * sin2).powi(2);
        (10.0 * (top / bottom).log10()) as f32
    }
}

#[derive(Clone, Copy, Default)]
pub struct Biquad {
    first: Frame,
    second: Frame,
}

impl Biquad {
    #[inline]
    pub fn run(&mut self, c: &Coefficients, channel: usize, x: f32) -> f32 {
        let y = c.b0 * x + self.first[channel];
        self.first[channel] = c.b1 * x - c.a1 * y + self.second[channel];
        self.second[channel] = c.b2 * x - c.a2 * y;
        y
    }

    pub fn settle(&mut self) {
        for value in self.first.iter_mut().chain(self.second.iter_mut()) {
            *value = settled(*value);
        }
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;

    #[test]
    fn each_shape_does_what_its_name_says() {
        let bell = Coefficients::design(Shape::Bell, RATE, 1000.0, 1.0, 6.0);
        assert!((bell.response_db(RATE, 1000.0) - 6.0).abs() < 0.01);
        assert!(bell.response_db(RATE, 50.0).abs() < 0.2);
        let low_cut = Coefficients::design(Shape::LowCut, RATE, 100.0, std::f32::consts::FRAC_1_SQRT_2, 0.0);
        assert!((low_cut.response_db(RATE, 100.0) + 3.01).abs() < 0.05);
        assert!(low_cut.response_db(RATE, 25.0) < -23.0);
        assert!(low_cut.response_db(RATE, 5000.0).abs() < 0.01);
        let high_cut = Coefficients::design(Shape::HighCut, RATE, 5000.0, std::f32::consts::FRAC_1_SQRT_2, 0.0);
        let notch = Coefficients::design(Shape::Notch, RATE, 3000.0, 4.0, 0.0);
        assert!(notch.response_db(RATE, 3000.0) < -60.0);
        assert!(notch.response_db(RATE, 1000.0).abs() < 0.3);
        let band = Coefficients::design(Shape::BandPass, RATE, 500.0, 2.0, 0.0);
        assert!(band.response_db(RATE, 500.0).abs() < 0.01);
        assert!(band.response_db(RATE, 5000.0) < -20.0);
        let gentle = Coefficients::gentle(Shape::LowCut, RATE, 100.0);
        assert!((gentle.response_db(RATE, 100.0) + 3.01).abs() < 0.05);
        assert!((gentle.response_db(RATE, 25.0) - gentle.response_db(RATE, 50.0) + 5.5).abs() < 0.6);
        assert!(high_cut.response_db(RATE, 20_000.0) < -20.0);
        let low_shelf = Coefficients::design(Shape::LowShelf, RATE, 200.0, 0.707, -9.0);
        assert!((low_shelf.response_db(RATE, 20.0) + 9.0).abs() < 0.3);
        assert!(low_shelf.response_db(RATE, 10_000.0).abs() < 0.1);
        let high_shelf = Coefficients::design(Shape::HighShelf, RATE, 4000.0, 0.707, 4.0);
        assert!((high_shelf.response_db(RATE, 20_000.0) - 4.0).abs() < 0.3);
    }

    #[test]
    fn the_filter_matches_its_drawn_response() {
        let bell = Coefficients::design(Shape::Bell, RATE, 2000.0, 2.0, -8.0);
        let mut filter = Biquad::default();
        let input = crate::testing::sine(RATE, 2000.0, 0.5, 48_000);
        let output: Vec<Frame> = input.iter().map(|frame| [filter.run(&bell, 0, frame[0]), 0.0]).collect();
        let measured = crate::db_of(crate::testing::rms(&output[24_000..]) / crate::testing::rms(&input[24_000..]));
        assert!((measured - bell.response_db(RATE, 2000.0)).abs() < 0.05, "measured {measured}");
    }
}
