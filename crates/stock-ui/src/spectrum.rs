use std::f32::consts::{PI, TAU};

pub const POINTS: usize = 4096;
const FLOOR_DB: f32 = -120.0;
const FALL_DB_PER_FRAME: f32 = 1.2;
const TILT_DB_PER_OCTAVE: f32 = 4.5;
const TILT_CENTRE_HZ: f32 = 1000.0;
const SMOOTHING_OCTAVES: f32 = 1.0 / 12.0;

pub struct Spectrum {
    window: Vec<f32>,
    real: Vec<f32>,
    imaginary: Vec<f32>,
    raw: Vec<f32>,
    power: Vec<f64>,
    running: Vec<f64>,
    pub shown: Vec<f32>,
    rate: f32,
}

impl Spectrum {
    pub fn new(rate: f32) -> Self {
        let window = (0..POINTS).map(|i| 0.5 - 0.5 * (TAU * i as f32 / POINTS as f32).cos()).collect();
        Self {
            window,
            real: vec![0.0; POINTS],
            imaginary: vec![0.0; POINTS],
            raw: vec![0.0; POINTS],
            power: vec![0.0; POINTS / 2],
            running: vec![0.0; POINTS / 2 + 1],
            shown: vec![FLOOR_DB; POINTS / 2],
            rate,
        }
    }

    pub fn samples(&mut self) -> &mut [f32] {
        &mut self.raw
    }

    pub fn analyse(&mut self) {
        let gain_of_window = 2.0 / (POINTS as f32 * 0.5);
        for i in 0..POINTS {
            self.real[i] = self.raw[i] * self.window[i];
            self.imaginary[i] = 0.0;
        }
        transform(&mut self.real, &mut self.imaginary);
        let bin_hz = self.rate / POINTS as f32;
        for bin in 0..POINTS / 2 {
            self.power[bin] = (self.real[bin].powi(2) + self.imaginary[bin].powi(2)) as f64;
            self.running[bin + 1] = self.running[bin] + self.power[bin];
        }
        let reach = 2f32.powf(SMOOTHING_OCTAVES / 2.0);
        for bin in 1..POINTS / 2 {
            let low = ((bin as f32 / reach).floor() as usize).max(1);
            let high = ((bin as f32 * reach).ceil() as usize).clamp(bin + 1, POINTS / 2);
            let power = ((self.running[high] - self.running[low]) / (high - low) as f64) as f32;
            let hz = bin as f32 * bin_hz;
            let tilt = TILT_DB_PER_OCTAVE * (hz / TILT_CENTRE_HZ).log2();
            let db = (10.0 * (power.max(1e-30)).log10() + 20.0 * gain_of_window.log10() + tilt).max(FLOOR_DB);
            let held = self.shown[bin] - FALL_DB_PER_FRAME;
            self.shown[bin] = db.max(held);
        }
    }

    pub fn level_at(&self, from_hz: f32, to_hz: f32) -> f32 {
        let bin_hz = self.rate / POINTS as f32;
        let wanted = (from_hz * to_hz).sqrt() / bin_hz;
        if !wanted.is_finite() {
            return FLOOR_DB;
        }
        let exact = wanted.clamp(1.0, (POINTS / 2 - 2) as f32);
        let below = exact.floor() as usize;
        let part = exact - below as f32;
        self.shown[below] + (self.shown[below + 1] - self.shown[below]) * part
    }
}

fn transform(real: &mut [f32], imaginary: &mut [f32]) {
    let n = real.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            real.swap(i, j);
            imaginary.swap(i, j);
        }
    }
    let mut length = 2;
    while length <= n {
        let angle = -2.0 * PI / length as f32;
        let (step_sin, step_cos) = angle.sin_cos();
        for start in (0..n).step_by(length) {
            let (mut turn_cos, mut turn_sin) = (1.0f32, 0.0f32);
            for k in 0..length / 2 {
                let (a, b) = (start + k, start + k + length / 2);
                let (br, bi) = (real[b] * turn_cos - imaginary[b] * turn_sin, real[b] * turn_sin + imaginary[b] * turn_cos);
                real[b] = real[a] - br;
                imaginary[b] = imaginary[a] - bi;
                real[a] += br;
                imaginary[a] += bi;
                let next_cos = turn_cos * step_cos - turn_sin * step_sin;
                turn_sin = turn_cos * step_sin + turn_sin * step_cos;
                turn_cos = next_cos;
            }
        }
        length <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tone_shows_up_at_its_frequency_and_level() {
        let rate = 48_000.0;
        let mut spectrum = Spectrum::new(rate);
        let hz = 1000.0;
        for (i, sample) in spectrum.samples().iter_mut().enumerate() {
            *sample = 0.5 * (TAU * hz * i as f32 / rate).sin();
        }
        spectrum.analyse();
        let at_tone = spectrum.level_at(990.0, 1010.0);
        assert!(at_tone < -6.0 && at_tone > -14.0, "{at_tone}");
        assert!(spectrum.level_at(4000.0, 4100.0) < at_tone - 60.0);
    }
}
