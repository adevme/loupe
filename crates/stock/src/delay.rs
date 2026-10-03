use crate::biquad::{Biquad, Coefficients, Shape, BUTTERWORTH};
use crate::smooth::Smoothed;
use crate::{decay_per_sample, Effect, Frame, Param, Unit, Values, DEFAULT_RATE};

pub const NOTES: [&str; 16] = [
    "1/64", "1/32T", "1/32", "1/16T", "1/16", "1/16D", "1/8T", "1/8", "1/8D", "1/4T", "1/4", "1/4D", "1/2T", "1/2", "1/2D", "1/1",
];
const NOTE_BEATS: [f32; 16] = [
    0.0625,
    0.125 * 2.0 / 3.0,
    0.125,
    0.25 * 2.0 / 3.0,
    0.25,
    0.375,
    0.5 * 2.0 / 3.0,
    0.5,
    0.75,
    2.0 / 3.0,
    1.0,
    1.5,
    2.0 * 2.0 / 3.0,
    2.0,
    3.0,
    4.0,
];
const PARAMS: [Param; 16] = [
    Param::new("sync", "Sync", 0.0, 1.0, 1.0, Unit::Switch),
    Param::choice("note_left", "Left note", &NOTES, 7),
    Param::choice("note_right", "Right note", &NOTES, 7),
    Param::new("time_left", "Left time", 1.0, 3000.0, 375.0, Unit::Milliseconds),
    Param::new("time_right", "Right time", 1.0, 3000.0, 375.0, Unit::Milliseconds),
    Param::new("link", "Link", 0.0, 1.0, 1.0, Unit::Switch),
    Param::new("feedback", "Feedback", 0.0, 98.0, 40.0, Unit::Percent),
    Param::new("cross", "Ping pong", 0.0, 100.0, 0.0, Unit::Percent),
    Param::new("low_cut", "Low cut", 20.0, 2000.0, 100.0, Unit::Hertz),
    Param::new("high_cut", "High cut", 500.0, 20_000.0, 8000.0, Unit::Hertz),
    Param::new("drive", "Drive", 0.0, 100.0, 0.0, Unit::Percent),
    Param::new("wobble_rate", "Mod rate", 0.05, 10.0, 0.6, Unit::Ratio),
    Param::new("wobble", "Mod depth", 0.0, 100.0, 0.0, Unit::Percent),
    Param::new("width", "Width", 0.0, 100.0, 100.0, Unit::Width),
    Param::new("duck", "Duck", 0.0, 100.0, 0.0, Unit::Percent),
    Param::new("mix", "Mix", 0.0, 100.0, 30.0, Unit::Percent),
];
pub const SYNC: usize = 0;
pub const NOTE_LEFT: usize = 1;
pub const NOTE_RIGHT: usize = 2;
pub const TIME_LEFT: usize = 3;
pub const TIME_RIGHT: usize = 4;
pub const LINK: usize = 5;
pub const FEEDBACK: usize = 6;
pub const CROSS: usize = 7;
pub const LOW_CUT: usize = 8;
pub const HIGH_CUT: usize = 9;
const LONGEST_SECONDS: f32 = 6.0;
const SPARE_FRAMES: usize = 8;
const WOBBLE_MS: f32 = 4.0;
const DRIVE_MOST: f32 = 6.0;
const DUCK_ATTACK_MS: f32 = 2.0;
const DUCK_RELEASE_MS: f32 = 250.0;
const DUCK_SENSE: f32 = 3.0;
const DEFAULT_BPM: f32 = 120.0;

pub fn echo_seconds(values: &[f32], bpm: f32) -> (f32, f32) {
    let beat = 60.0 / bpm.max(1.0);
    let note = |index: usize| NOTE_BEATS[(values[index].max(0.0) as usize).min(NOTE_BEATS.len() - 1)] * beat;
    let synced = values[SYNC] > 0.5;
    let linked = values[LINK] > 0.5;
    let left = if synced { note(NOTE_LEFT) } else { values[TIME_LEFT] * 0.001 };
    let right = match (linked, synced) {
        (true, _) => left,
        (false, true) => note(NOTE_RIGHT),
        (false, false) => values[TIME_RIGHT] * 0.001,
    };
    (left.min(LONGEST_SECONDS), right.min(LONGEST_SECONDS))
}

pub struct Delay {
    values: Values<16>,
    rate: f32,
    bpm: f32,
    line: Vec<Frame>,
    write: usize,
    times: [Smoothed; 2],
    feedback: Smoothed,
    cross: Smoothed,
    mix: Smoothed,
    width: Smoothed,
    drive: f32,
    duck: f32,
    duck_level: f32,
    duck_attack: f32,
    duck_release: f32,
    wobble: f32,
    turn: f32,
    turn_step: f32,
    low_cut: Coefficients,
    high_cut: Coefficients,
    filters: [Biquad; 2],
}

impl Delay {
    pub fn new() -> Self {
        let mut delay = Self {
            values: Values::new(&PARAMS),
            rate: DEFAULT_RATE,
            bpm: DEFAULT_BPM,
            line: Vec::new(),
            write: 0,
            times: [Smoothed::new(1.0); 2],
            feedback: Smoothed::new(0.0),
            cross: Smoothed::new(0.0),
            mix: Smoothed::new(0.0),
            width: Smoothed::new(1.0),
            drive: 0.0,
            duck: 0.0,
            duck_level: 0.0,
            duck_attack: 0.0,
            duck_release: 0.0,
            wobble: 0.0,
            turn: 0.0,
            turn_step: 0.0,
            low_cut: Coefficients::PASS,
            high_cut: Coefficients::PASS,
            filters: [Biquad::default(); 2],
        };
        delay.prepare(DEFAULT_RATE);
        delay
    }

    pub fn bpm(&self) -> f32 {
        self.bpm
    }

    fn smoothers(&mut self) -> [&mut Smoothed; 6] {
        let [left, right] = &mut self.times;
        [left, right, &mut self.feedback, &mut self.cross, &mut self.mix, &mut self.width]
    }

    fn read_knobs(&mut self) {
        let values: Vec<f32> = (0..PARAMS.len()).map(|index| self.values.get(index)).collect();
        let (left, right) = echo_seconds(&values, self.bpm);
        let longest = (self.line.len() - SPARE_FRAMES) as f32 - self.wobble_frames(100.0);
        self.times[0].aim((left * self.rate).clamp(1.0, longest));
        self.times[1].aim((right * self.rate).clamp(1.0, longest));
        self.feedback.aim(values[FEEDBACK] / 100.0);
        self.cross.aim(values[CROSS] / 100.0);
        self.low_cut = Coefficients::design(Shape::LowCut, self.rate, values[LOW_CUT], BUTTERWORTH, 0.0);
        self.high_cut = Coefficients::design(Shape::HighCut, self.rate, values[HIGH_CUT], BUTTERWORTH, 0.0);
        self.drive = values[10] / 100.0 * DRIVE_MOST;
        self.turn_step = std::f32::consts::TAU * values[11] / self.rate;
        self.wobble = self.wobble_frames(values[12]);
        self.width.aim(values[13] / 100.0);
        self.duck = values[14] / 100.0;
        self.mix.aim(values[15] / 100.0);
    }

    fn wobble_frames(&self, percent: f32) -> f32 {
        percent / 100.0 * WOBBLE_MS * 0.001 * self.rate
    }

    fn read(&self, side: usize, back: f32) -> f32 {
        let length = self.line.len();
        let whole = back.floor();
        let part = back - whole;
        let newer = (self.write + length - whole as usize) % length;
        let older = (newer + length - 1) % length;
        let (a, b) = (self.line[newer][side], self.line[older][side]);
        a + (b - a) * part
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

    fn set_tempo(&mut self, bpm: f32) {
        if bpm.is_finite() && bpm > 0.0 && bpm != self.bpm {
            self.bpm = bpm;
            self.values.mark_changed();
        }
    }

    fn prepare(&mut self, rate: f32) {
        self.rate = rate;
        self.line = vec![[0.0; 2]; (LONGEST_SECONDS * rate).ceil() as usize + (WOBBLE_MS * 0.001 * rate) as usize + SPARE_FRAMES];
        for smoothed in self.smoothers() {
            smoothed.prepare(rate);
        }
        self.duck_attack = decay_per_sample(DUCK_ATTACK_MS, rate);
        self.duck_release = decay_per_sample(DUCK_RELEASE_MS, rate);
        self.read_knobs();
        self.values.take_change();
        for smoothed in self.smoothers() {
            smoothed.snap();
        }
        self.reset();
    }

    fn reset(&mut self) {
        self.line.fill([0.0; 2]);
        self.write = 0;
        self.duck_level = 0.0;
        self.turn = 0.0;
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
            self.turn += self.turn_step;
            if self.turn > std::f32::consts::TAU {
                self.turn -= std::f32::consts::TAU;
            }
            let (sin, cos) = self.turn.sin_cos();
            let swing = [self.wobble * (1.0 + sin) * 0.5, self.wobble * (1.0 + cos) * 0.5];
            let mut echo = [0.0f32; 2];
            for side in 0..2 {
                let back = self.times[side].next() + swing[side];
                let heard = self.read(side, back);
                let low_cut = self.filters[0].run(&self.low_cut, side, heard);
                let shaped = self.filters[1].run(&self.high_cut, side, low_cut);
                echo[side] = if self.drive > 0.0 { (shaped * (1.0 + self.drive)).tanh() / (1.0 + self.drive) } else { shaped };
            }
            let feedback = self.feedback.next();
            let cross = self.cross.next();
            let back = [
                (echo[0] * (1.0 - cross) + echo[1] * cross) * feedback,
                (echo[1] * (1.0 - cross) + echo[0] * cross) * feedback,
            ];
            let mono = (frame[0] + frame[1]) * 0.5;
            self.write = (self.write + 1) % length;
            self.line[self.write] = [
                frame[0] * (1.0 - cross) + mono * cross + back[0],
                frame[1] * (1.0 - cross) + back[1],
            ];

            let loudness = mono.abs();
            let speed = if loudness > self.duck_level { self.duck_attack } else { self.duck_release };
            self.duck_level = loudness + (self.duck_level - loudness) * speed;
            let ducked = 1.0 - self.duck * (self.duck_level * DUCK_SENSE).min(1.0);
            let width = self.width.next();
            let middle = (echo[0] + echo[1]) * 0.5;
            let sides = (echo[0] - echo[1]) * 0.5 * width;
            let mix = self.mix.next();
            let wet = [(middle + sides) * ducked, (middle - sides) * ducked];
            frame[0] = frame[0] * (1.0 - mix) + wet[0] * mix;
            frame[1] = frame[1] * (1.0 - mix) + wet[1] * mix;
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

    fn burst_then_silence(rate: f32, seconds: f32) -> Vec<Frame> {
        let mut audio = sine(rate, 1000.0, 0.5, (rate * 0.02) as usize);
        audio.resize((rate * seconds) as usize, [0.0; 2]);
        audio
    }

    fn open(delay: &mut Delay) {
        for (id, value) in [("mix", 100.0), ("low_cut", 20.0), ("high_cut", 20_000.0)] {
            delay.set_by_id(id, value);
        }
    }

    #[test]
    fn synced_notes_follow_the_tempo() {
        let values: Vec<f32> = PARAMS.iter().map(|param| param.default).collect();
        let (left, right) = echo_seconds(&values, 120.0);
        assert!((left - 0.25).abs() < 1e-6 && left == right);
        let mut dotted = values.clone();
        dotted[LINK] = 0.0;
        dotted[NOTE_RIGHT] = NOTES.iter().position(|note| *note == "1/8D").unwrap() as f32;
        let (_, right) = echo_seconds(&dotted, 90.0);
        assert!((right - 0.5).abs() < 1e-6, "{right}");
        let triplet = NOTE_BEATS[NOTES.iter().position(|note| *note == "1/4T").unwrap()];
        assert!((triplet - 2.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn echoes_land_on_time_and_fade_by_the_feedback() {
        let rate = 48_000.0;
        let mut delay = Delay::new();
        open(&mut delay);
        delay.set_by_id("feedback", 50.0);
        delay.prepare(rate);
        delay.set_tempo(120.0);
        let input = burst_then_silence(rate, 1.2);
        let mut output = input.clone();
        in_blocks(&mut delay, &mut output, 256);
        let window = |start: usize| rms(&output[start..start + 960]);
        assert!(window(0) < 1e-6);
        assert!(window(5000) < 1e-4);
        let first = crate::db_of(window(12_000) / rms(&input[..960]));
        let second = crate::db_of(window(24_000) / rms(&input[..960]));
        assert!(first.abs() < 0.3, "first {first}");
        assert!((second + 6.02).abs() < 0.5, "second {second}");
    }

    #[test]
    fn a_tempo_change_moves_synced_echoes() {
        let rate = 48_000.0;
        let mut delay = Delay::new();
        open(&mut delay);
        delay.prepare(rate);
        delay.set_tempo(60.0);
        delay.prepare(rate);
        let mut output = burst_then_silence(rate, 1.2);
        in_blocks(&mut delay, &mut output, 256);
        assert!(rms(&output[12_000..12_960]) < 1e-4, "no echo at the old time");
        assert!(rms(&output[24_000..24_960]) > 0.1, "the echo moved to half a second");
    }

    #[test]
    fn full_ping_pong_puts_echoes_on_alternate_sides() {
        let rate = 48_000.0;
        let mut delay = Delay::new();
        open(&mut delay);
        delay.set_by_id("feedback", 50.0);
        delay.set_by_id("cross", 100.0);
        delay.prepare(rate);
        let mut output = burst_then_silence(rate, 1.2);
        in_blocks(&mut delay, &mut output, 256);
        let side = |start: usize, side: usize| -> f32 { output[start..start + 960].iter().map(|f| f[side].abs()).sum() };
        assert!(side(12_000, 0) > 100.0 * side(12_000, 1));
        assert!(side(24_000, 1) > 100.0 * side(24_000, 0));
    }

    #[test]
    fn ducking_holds_the_echoes_down_while_you_play() {
        let level = |duck: f32| {
            let rate = 48_000.0;
            let mut delay = Delay::new();
            delay.set_by_id("duck", duck);
            delay.set_by_id("mix", 50.0);
            delay.prepare(rate);
            let mut audio = sine(rate, 300.0, 0.5, 48_000);
            in_blocks(&mut delay, &mut audio, 256);
            rms(&audio[30_000..])
        };
        assert!(level(100.0) < level(0.0) * 0.8);
    }

    #[test]
    fn the_wildest_settings_never_run_away() {
        let mut delay = Delay::new();
        for (id, value) in [("sync", 0.0), ("time_left", 10.0), ("feedback", 98.0), ("mix", 100.0), ("drive", 100.0), ("wobble", 100.0), ("wobble_rate", 10.0), ("cross", 50.0)] {
            delay.set_by_id(id, value);
        }
        delay.prepare(48_000.0);
        let mut audio = crate::testing::noise(1.0, 48_000 * 5, 4);
        in_blocks(&mut delay, &mut audio, 512);
        assert!(audio.iter().flatten().all(|sample| sample.is_finite()));
        assert!(crate::testing::peak(&audio) < 40.0);
    }

    #[test]
    fn zero_mix_is_untouched_sound() {
        let mut delay = Delay::new();
        delay.set_by_id("mix", 0.0);
        delay.prepare(48_000.0);
        let input = crate::testing::noise(0.8, 9000, 2);
        let mut output = input.clone();
        in_blocks(&mut delay, &mut output, 300);
        assert_eq!(output, input);
    }
}
