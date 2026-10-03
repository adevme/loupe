use crate::biquad::{Biquad, Coefficients, Shape, BUTTERWORTH};
use crate::smooth::Smoothed;
use crate::{Effect, Frame, Param, Unit, Values, DEFAULT_RATE};

const PARAMS: [Param; 6] = [
    Param::new("time", "Time", 1.0, 2000.0, 375.0, Unit::Milliseconds),
    Param::new("feedback", "Feedback", 0.0, 95.0, 35.0, Unit::Percent),
    Param::new("mix", "Mix", 0.0, 100.0, 25.0, Unit::Percent),
    Param::new("ping_pong", "Ping pong", 0.0, 1.0, 0.0, Unit::Switch),
    Param::new("low_cut", "Low cut", 20.0, 2000.0, 100.0, Unit::Hertz),
    Param::new("high_cut", "High cut", 1000.0, 20_000.0, 9000.0, Unit::Hertz),
];
const LONGEST_SECONDS: f32 = 2.0;
const SPARE_FRAMES: usize = 4;

pub struct Delay {
    values: Values<6>,
    rate: f32,
    line: Vec<Frame>,
    write: usize,
    time: Smoothed,
    feedback: Smoothed,
    mix: Smoothed,
    ping_pong: bool,
    low_cut: Coefficients,
    high_cut: Coefficients,
    filters: [Biquad; 2],
}

impl Delay {
    pub fn new() -> Self {
        let mut delay = Self {
            values: Values::new(&PARAMS),
            rate: DEFAULT_RATE,
            line: Vec::new(),
            write: 0,
            time: Smoothed::new(0.0),
            feedback: Smoothed::new(0.0),
            mix: Smoothed::new(0.0),
            ping_pong: false,
            low_cut: Coefficients::PASS,
            high_cut: Coefficients::PASS,
            filters: [Biquad::default(); 2],
        };
        delay.prepare(DEFAULT_RATE);
        delay
    }

    fn read_knobs(&mut self) {
        let longest = (self.line.len() - SPARE_FRAMES) as f32;
        self.time.aim((self.values.get(0) * 0.001 * self.rate).clamp(1.0, longest));
        self.feedback.aim(self.values.get(1) / 100.0);
        self.mix.aim(self.values.get(2) / 100.0);
        self.ping_pong = self.values.get(3) > 0.5;
        self.low_cut = Coefficients::design(Shape::LowCut, self.rate, self.values.get(4), BUTTERWORTH, 0.0);
        self.high_cut = Coefficients::design(Shape::HighCut, self.rate, self.values.get(5), BUTTERWORTH, 0.0);
    }

    fn read(&self, back: f32) -> Frame {
        let length = self.line.len();
        let whole = back.floor();
        let part = back - whole;
        let newer = (self.write + length - whole as usize) % length;
        let older = (newer + length - 1) % length;
        let (a, b) = (self.line[newer], self.line[older]);
        [a[0] + (b[0] - a[0]) * part, a[1] + (b[1] - a[1]) * part]
    }
}

impl Default for Delay {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for Delay {
    fn name(&self) -> &'static str {
        "Loupe Delay"
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
        self.line = vec![[0.0; 2]; (LONGEST_SECONDS * rate).ceil() as usize + SPARE_FRAMES];
        for smoothed in [&mut self.time, &mut self.feedback, &mut self.mix] {
            smoothed.prepare(rate);
        }
        self.read_knobs();
        self.values.take_change();
        for smoothed in [&mut self.time, &mut self.feedback, &mut self.mix] {
            smoothed.snap();
        }
        self.reset();
    }

    fn reset(&mut self) {
        self.line.fill([0.0; 2]);
        self.write = 0;
        for filter in &mut self.filters {
            filter.reset();
        }
    }

    fn process(&mut self, audio: &mut [Frame]) {
        if self.values.take_change() {
            self.read_knobs();
        }
        let length = self.line.len();
        for frame in audio.iter_mut() {
            let back_by = self.time.next();
            let heard = self.read(back_by);
            let mut echo = [0.0; 2];
            for side in 0..2 {
                let low_cut = self.filters[0].run(&self.low_cut, side, heard[side]);
                echo[side] = self.filters[1].run(&self.high_cut, side, low_cut);
            }
            let feedback = self.feedback.next();
            let back = [echo[0] * feedback, echo[1] * feedback];
            self.write = (self.write + 1) % length;
            self.line[self.write] = if self.ping_pong {
                [(frame[0] + frame[1]) * 0.5 + back[1], back[0]]
            } else {
                [frame[0] + back[0], frame[1] + back[1]]
            };
            let mix = self.mix.next();
            frame[0] = frame[0] * (1.0 - mix) + echo[0] * mix;
            frame[1] = frame[1] * (1.0 - mix) + echo[1] * mix;
        }
        for filter in &mut self.filters {
            filter.settle();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{in_blocks, rms, sine};

    fn burst_then_silence(rate: f32) -> Vec<Frame> {
        let mut audio = sine(rate, 1000.0, 0.5, (rate * 0.02) as usize);
        audio.resize((rate * 1.2) as usize, [0.0; 2]);
        audio
    }

    #[test]
    fn echoes_land_on_time_and_fade_by_the_feedback() {
        let rate = 48_000.0;
        let mut delay = Delay::new();
        delay.prepare(rate);
        for (id, value) in [("time", 250.0), ("feedback", 50.0), ("mix", 100.0), ("low_cut", 20.0), ("high_cut", 20_000.0)] {
            delay.set_by_id(id, value);
        }
        delay.prepare(rate);
        let input = burst_then_silence(rate);
        let mut output = input.clone();
        in_blocks(&mut delay, &mut output, 256);
        let burst = 960;
        let window = |start: usize| rms(&output[start..start + burst]);
        assert!(window(0) < 1e-6, "mix 100 means no dry sound");
        assert!(window(5000) < 1e-4, "nothing between the burst and its echo");
        let first = window(12_000) / rms(&input[..burst]);
        let second = window(24_000) / rms(&input[..burst]);
        assert!((crate::db_of(first)).abs() < 0.3, "first echo {}", crate::db_of(first));
        assert!((crate::db_of(second) + 6.02).abs() < 0.5, "second echo {}", crate::db_of(second));
    }

    #[test]
    fn ping_pong_puts_echoes_on_alternate_sides() {
        let rate = 48_000.0;
        let mut delay = Delay::new();
        for (id, value) in [("time", 250.0), ("feedback", 50.0), ("mix", 100.0), ("ping_pong", 1.0)] {
            delay.set_by_id(id, value);
        }
        delay.prepare(rate);
        let mut output = burst_then_silence(rate);
        in_blocks(&mut delay, &mut output, 256);
        let side = |start: usize, side: usize| -> f32 { output[start..start + 960].iter().map(|f| f[side].abs()).sum() };
        assert!(side(12_000, 0) > 100.0 * side(12_000, 1));
        assert!(side(24_000, 1) > 100.0 * side(24_000, 0));
    }

    #[test]
    fn full_feedback_never_runs_away() {
        let mut delay = Delay::new();
        for (id, value) in [("time", 10.0), ("feedback", 95.0), ("mix", 100.0)] {
            delay.set_by_id(id, value);
        }
        delay.prepare(48_000.0);
        let mut audio = crate::testing::noise(1.0, 48_000 * 5, 4);
        in_blocks(&mut delay, &mut audio, 512);
        assert!(crate::testing::peak(&audio) < 40.0);
        assert!(audio.iter().flatten().all(|sample| sample.is_finite()));
    }
}
