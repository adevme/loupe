use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crate::history::{History, Moment, MOMENTS_PER_SECOND};
use crate::key::{detected, Detected};
use crate::{gain_of, Effect, Frame, Param, Unit, Values, DEFAULT_RATE};

pub const KEYS: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
pub const FOUND: &str = "Detected";
pub const KEY_CHOICES: [&str; 13] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B", FOUND];
pub const SCALES: [&str; 6] = ["Chromatic", "Major", "Minor", "Major pentatonic", "Minor pentatonic", FOUND];
pub const STEPS: [&[u8]; 5] = [&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11], &[0, 2, 4, 5, 7, 9, 11], &[0, 2, 3, 5, 7, 8, 10], &[0, 2, 4, 7, 9], &[0, 3, 5, 7, 10]];
pub const FOLLOWED_KEY: f32 = KEYS.len() as f32;
pub const FOLLOWED_SCALE: f32 = STEPS.len() as f32;

const PARAMS: [Param; 5] = [
    Param::choice("key", "Key", &KEY_CHOICES, 0),
    Param::choice("scale", "Scale", &SCALES, 0),
    Param::new("speed", "Speed", 0.0, 400.0, 40.0, Unit::Milliseconds),
    Param::new("amount", "Amount", 0.0, 100.0, 100.0, Unit::Percent),
    Param::new("formant", "Formant", -12.0, 12.0, 0.0, Unit::Semitones),
];
pub const KEY: usize = 0;
pub const SCALE: usize = 1;
pub const SPEED: usize = 2;
pub const AMOUNT: usize = 3;
pub const FORMANT: usize = 4;

pub const SILENT_NOTE: f32 = -1.0;
const LOWEST_HZ: f32 = 70.0;
const HIGHEST_HZ: f32 = 1000.0;
const LOOK_AHEAD_SECONDS: f32 = 0.06;
const DETECT_SECONDS: f32 = 0.043;
const DETECT_EVERY_SECONDS: f32 = 0.005;
const SQUEEZE: usize = 4;
const CLEAR_ENOUGH: f32 = 0.2;
const QUIETEST_DB: f32 = -50.0;
const STICKY: f32 = 0.25;
const UNVOICED_PERIOD_SECONDS: f32 = 0.005;
const KEPT: usize = 1 << 15;
const HOLD_SECONDS: f32 = 2.0;
const NOTHING: u8 = 255;

pub struct Tuning {
    word: AtomicU32,
}

impl Tuning {
    pub fn new() -> Self {
        Self { word: AtomicU32::new(u32::from(NOTHING) | u32::from(NOTHING) << 8) }
    }

    fn read(&self) -> (u8, u8) {
        let word = self.word.load(Ordering::Relaxed);
        (word as u8, (word >> 8) as u8)
    }

    pub fn following(&self) -> Option<u8> {
        let (following, _) = self.read();
        (following != NOTHING).then_some(following)
    }

    pub fn waiting(&self) -> Option<u8> {
        let (_, waiting) = self.read();
        (waiting != NOTHING).then_some(waiting)
    }

    fn tell(&self, following: Option<u8>, waiting: Option<u8>) {
        let packed = |which: Option<u8>| u32::from(which.unwrap_or(NOTHING));
        self.word.store(packed(following) | packed(waiting) << 8, Ordering::Relaxed);
    }
}

impl Default for Tuning {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Tune {
    values: Values<5>,
    rate: f32,
    ahead: usize,
    heard: Vec<Frame>,
    sums: Vec<Frame>,
    squeezed: Vec<f32>,
    squeeze_sum: f32,
    squeeze_count: usize,
    difference: Vec<f32>,
    now: u64,
    since_detect: usize,
    detect_every: usize,
    period: f32,
    voiced: bool,
    sung: f32,
    target: f32,
    applied: f32,
    follow: f32,
    next_grain: f64,
    mark: f64,
    since_moment: usize,
    moment_every: usize,
    history: Arc<History>,
    detected: Arc<Detected>,
    tuning: Arc<Tuning>,
    following: Option<u8>,
    waiting: Option<u8>,
    waited: usize,
    hold_for: usize,
}

impl Tune {
    pub fn new() -> Self {
        let mut tune = Self {
            values: Values::new(&PARAMS),
            rate: DEFAULT_RATE,
            ahead: 0,
            heard: vec![[0.0; 2]; KEPT],
            sums: vec![[0.0; 2]; KEPT],
            squeezed: Vec::new(),
            squeeze_sum: 0.0,
            squeeze_count: 0,
            difference: Vec::new(),
            now: 0,
            since_detect: 0,
            detect_every: 1,
            period: 1.0,
            voiced: false,
            sung: SILENT_NOTE,
            target: SILENT_NOTE,
            applied: 0.0,
            follow: 0.0,
            next_grain: 0.0,
            mark: 0.0,
            since_moment: 0,
            moment_every: 1,
            history: Arc::new(History::new()),
            detected: detected(),
            tuning: Arc::new(Tuning::new()),
            following: None,
            waiting: None,
            waited: 0,
            hold_for: 1,
        };
        tune.prepare(DEFAULT_RATE);
        tune
    }

    pub fn history(&self) -> Arc<History> {
        self.history.clone()
    }

    pub fn tuning(&self) -> Arc<Tuning> {
        self.tuning.clone()
    }

    pub fn listens_to(&mut self, detected: Arc<Detected>) {
        self.detected = detected;
        self.following = None;
        self.waiting = None;
        self.waited = 0;
    }

    fn follows(&self) -> bool {
        self.values.get(KEY) == FOLLOWED_KEY || self.values.get(SCALE) == FOLLOWED_SCALE
    }

    fn key_in_use(&self) -> i32 {
        if self.values.get(KEY) == FOLLOWED_KEY {
            return self.following.map_or(0, |key| i32::from(key % 12));
        }
        self.values.get(KEY) as i32
    }

    fn steps_in_use(&self) -> &'static [u8] {
        let scale = self.values.get(SCALE);
        if scale == FOLLOWED_SCALE {
            return match self.following {
                Some(key) if key < 12 => STEPS[1],
                Some(_) => STEPS[2],
                None => STEPS[0],
            };
        }
        STEPS[(scale as usize).min(STEPS.len() - 1)]
    }

    fn hear_detected(&mut self, frames: usize) {
        if !self.follows() {
            self.waiting = None;
            self.waited = 0;
            self.tuning.tell(None, None);
            return;
        }
        match self.detected.settled() {
            Some(key) if Some(key) != self.following => {
                if self.waiting == Some(key) {
                    self.waited += frames;
                } else {
                    self.waiting = Some(key);
                    self.waited = 0;
                }
                if self.waited >= self.hold_for && !self.voiced {
                    self.following = Some(key);
                    self.waiting = None;
                    self.waited = 0;
                    self.values.mark_changed();
                }
            }
            _ => {
                self.waiting = None;
                self.waited = 0;
            }
        }
        self.tuning.tell(self.following, self.waiting);
    }

    fn unvoiced_period(&self) -> f32 {
        UNVOICED_PERIOD_SECONDS * self.rate
    }

    fn longest_period(&self) -> f32 {
        (self.ahead as f32 / 4.0 - 2.0).max(8.0)
    }

    fn detect(&mut self) {
        let len = self.squeezed.len();
        let rate = self.rate / SQUEEZE as f32;
        let shortest = (rate / HIGHEST_HZ).floor().max(2.0) as usize;
        let longest = ((rate / LOWEST_HZ).ceil() as usize).min(len / 2);
        let window = len - longest;
        let start = len - window - longest;
        let loudest = self.squeezed[start..].iter().fold(0.0f32, |top, sample| top.max(sample.abs()));
        if loudest < gain_of(QUIETEST_DB) || longest <= shortest {
            self.voiced = false;
            self.target = SILENT_NOTE;
            return;
        }
        self.difference.clear();
        self.difference.push(1.0);
        let mut running = 0.0;
        for lag in 1..=longest {
            let mut total = 0.0;
            for at in start..start + window {
                let gap = self.squeezed[at] - self.squeezed[at + lag];
                total += gap * gap;
            }
            running += total;
            self.difference.push(if running > 0.0 { total * lag as f32 / running } else { 1.0 });
        }
        let mut found = None;
        let mut lag = shortest;
        while lag < longest {
            if self.difference[lag] < CLEAR_ENOUGH {
                while lag + 1 < longest && self.difference[lag + 1] < self.difference[lag] {
                    lag += 1;
                }
                found = Some(lag);
                break;
            }
            lag += 1;
        }
        let Some(lag) = found else {
            self.voiced = false;
            self.target = SILENT_NOTE;
            return;
        };
        let (before, here, after) = (self.difference[lag - 1], self.difference[lag], self.difference[lag + 1]);
        let bend = before - 2.0 * here + after;
        let exact = if bend.abs() > 1e-9 { lag as f32 + 0.5 * (before - after) / bend } else { lag as f32 };
        let period = (exact * SQUEEZE as f32).min(self.longest_period());
        self.voiced = true;
        self.period = period;
        self.sung = 69.0 + 12.0 * (self.rate / period / 440.0).log2();
        self.choose_target();
    }

    fn choose_target(&mut self) {
        let key = self.key_in_use();
        let steps = self.steps_in_use();
        let near = self.sung.round() as i32;
        let mut best = self.target;
        let mut best_gap = if self.target >= 0.0 { (self.sung - self.target).abs() - STICKY } else { f32::MAX };
        for note in near - 2..=near + 2 {
            let class = (note - key).rem_euclid(12) as u8;
            if !steps.contains(&class) {
                continue;
            }
            let gap = (self.sung - note as f32).abs();
            if gap < best_gap {
                best_gap = gap;
                best = note as f32;
            }
        }
        self.target = best;
    }

    fn place_grain(&mut self, centre: f64, shift: f32) {
        let period = if self.voiced { self.period } else { self.unvoiced_period() };
        let spacing = period as f64 / shift as f64;
        while self.mark + period as f64 / 2.0 < centre {
            self.mark += period as f64;
        }
        if self.mark > centre + period as f64 {
            self.mark = centre;
        }
        let half = period;
        let reach = half.ceil() as i64;
        let squash = self.squash();
        let scale = spacing as f32 / half;
        let from = self.mark;
        for step in -reach..=reach {
            if step.unsigned_abs() as f32 > half {
                continue;
            }
            let shape = 0.5 + 0.5 * (std::f32::consts::PI * step as f32 / half).cos();
            let read = from + step as f64 * squash as f64;
            let low = read.floor();
            let part = (read - low) as f32;
            let first = self.heard[(low as i64).rem_euclid(KEPT as i64) as usize];
            let second = self.heard[(low as i64 + 1).rem_euclid(KEPT as i64) as usize];
            let into = &mut self.sums[(centre.round() as i64 + step).rem_euclid(KEPT as i64) as usize];
            let weight = shape * scale;
            into[0] += (first[0] + (second[0] - first[0]) * part) * weight;
            into[1] += (first[1] + (second[1] - first[1]) * part) * weight;
        }
        self.next_grain = centre + spacing;
    }

    fn squash(&self) -> f32 {
        2f32.powf(self.values.get(FORMANT) / 12.0)
    }

    fn note_moment(&mut self) {
        self.since_moment += 1;
        if self.since_moment < self.moment_every {
            return;
        }
        self.since_moment = 0;
        let sung = if self.voiced { self.sung } else { SILENT_NOTE };
        let target = if self.voiced { self.target } else { SILENT_NOTE };
        self.history.push(Moment { input: sung, output: target, reduction: self.applied });
    }
}

impl Default for Tune {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for Tune {
    fn history(&self) -> Option<Arc<History>> {
        Some(self.history())
    }

    fn tuning(&self) -> Option<Arc<Tuning>> {
        Some(self.tuning())
    }

    fn keeps_its_own_clock(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "Loupe Tune"
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

    fn latency(&self) -> usize {
        self.ahead
    }

    fn prepare(&mut self, rate: f32) {
        self.rate = rate;
        self.ahead = (LOOK_AHEAD_SECONDS * rate) as usize;
        self.detect_every = (DETECT_EVERY_SECONDS * rate).max(1.0) as usize;
        self.moment_every = (rate / MOMENTS_PER_SECOND).max(1.0) as usize;
        self.hold_for = (HOLD_SECONDS * rate).max(1.0) as usize;
        let detect_len = ((DETECT_SECONDS * rate) as usize / SQUEEZE).max(64) * 2;
        self.squeezed = vec![0.0; detect_len];
        self.difference = Vec::with_capacity(detect_len);
        self.values.take_change();
        self.reset();
    }

    fn reset(&mut self) {
        self.heard.iter_mut().for_each(|frame| *frame = [0.0; 2]);
        self.sums.iter_mut().for_each(|frame| *frame = [0.0; 2]);
        self.squeezed.iter_mut().for_each(|sample| *sample = 0.0);
        self.squeeze_sum = 0.0;
        self.squeeze_count = 0;
        self.now = self.ahead as u64 * 2;
        self.since_detect = 0;
        self.voiced = false;
        self.period = self.unvoiced_period();
        self.sung = SILENT_NOTE;
        self.target = SILENT_NOTE;
        self.applied = 0.0;
        self.next_grain = self.now as f64;
        self.mark = self.next_grain;
    }

    fn process(&mut self, audio: &mut [Frame]) {
        self.hear_detected(audio.len());
        if self.values.take_change() && self.voiced {
            self.target = SILENT_NOTE;
            self.choose_target();
        }
        let speed = self.values.get(SPEED);
        self.follow = if speed <= 0.0 { 0.0 } else { (-1.0 / (speed * 0.001 * self.rate)).exp() };
        let amount = self.values.get(AMOUNT) / 100.0;
        for frame in audio.iter_mut() {
            let at = (self.now % KEPT as u64) as usize;
            self.heard[at] = *frame;
            self.squeeze_sum += (frame[0] + frame[1]) * 0.5;
            self.squeeze_count += 1;
            if self.squeeze_count == SQUEEZE {
                self.squeezed.rotate_left(1);
                let last = self.squeezed.len() - 1;
                self.squeezed[last] = self.squeeze_sum / SQUEEZE as f32;
                self.squeeze_sum = 0.0;
                self.squeeze_count = 0;
            }
            self.since_detect += 1;
            if self.since_detect >= self.detect_every {
                self.since_detect = 0;
                self.detect();
            }
            let wanted = if self.voiced && self.target >= 0.0 { (self.target - self.sung) * amount } else { 0.0 };
            self.applied = wanted + (self.applied - wanted) * self.follow;
            let shift = 2f32.powf(self.applied / 12.0);
            let period = if self.voiced { self.period } else { self.unvoiced_period() };
            let half = period;
            let needs = half as f64 * self.squash().max(1.0) as f64;
            while self.next_grain <= self.now as f64 - needs {
                let centre = self.next_grain;
                self.place_grain(centre, shift);
            }
            let out_at = ((self.now - self.ahead as u64) % KEPT as u64) as usize;
            *frame = std::mem::replace(&mut self.sums[out_at], [0.0; 2]);
            self.note_moment();
            self.now += 1;
        }
    }
}

pub fn note_name(note: f32) -> String {
    if note < 0.0 {
        return "-".to_string();
    }
    let whole = note.round() as i32;
    format!("{}{}", KEYS[whole.rem_euclid(12) as usize], whole / 12 - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::sine;

    fn heard_hz(audio: &[Frame], rate: f32) -> f32 {
        let mut crossings = Vec::new();
        for at in 1..audio.len() {
            if audio[at - 1][0] < 0.0 && audio[at][0] >= 0.0 {
                let before = audio[at - 1][0];
                let after = audio[at][0];
                crossings.push(at as f32 - 1.0 + before / (before - after));
            }
        }
        let span = crossings.last().unwrap() - crossings.first().unwrap();
        (crossings.len() - 1) as f32 * rate / span
    }

    fn repeats_hz(audio: &[Frame], rate: f32) -> f32 {
        let len = 4_096.min(audio.len() / 2);
        let score = |lag: usize| (0..len).map(|at| audio[at][0] * audio[at + lag][0]).sum::<f32>();
        let shortest = (rate / 1_000.0) as usize;
        let longest = (rate / 70.0) as usize;
        let scores: Vec<f32> = (0..longest + 2).map(|lag| if lag >= shortest { score(lag) } else { 0.0 }).collect();
        let top = scores.iter().fold(0.0f32, |top, s| top.max(*s));
        let mut best = (shortest..longest).find(|lag| scores[*lag] >= 0.85 * top).unwrap();
        while scores[best + 1] > scores[best] {
            best += 1;
        }
        let (before, here, after) = (score(best - 1), score(best), score(best + 1));
        let exact = best as f32 + 0.5 * (before - after) / (before - 2.0 * here + after);
        rate / exact
    }

    fn run(tune: &mut Tune, audio: &mut [Frame]) {
        for block in audio.chunks_mut(512) {
            tune.process(block);
        }
    }

    #[test]
    fn a_flat_note_is_pulled_up_to_the_nearest_note_in_the_key() {
        let rate = 48_000.0;
        let mut tune = Tune::new();
        tune.prepare(rate);
        tune.set(SPEED, 0.0);
        let flat_a = 440.0 * 2f32.powf(-0.3 / 12.0);
        let mut audio = sine(rate, flat_a, 0.5, 48_000);
        run(&mut tune, &mut audio);
        let settled = &audio[24_000..];
        let hz = heard_hz(settled, rate);
        assert!((hz - 440.0).abs() < 3.0, "came out at {hz} Hz");
        assert_eq!(note_name(tune.target), "A4");
    }

    #[test]
    fn notes_outside_the_scale_go_to_a_note_inside_it() {
        let rate = 48_000.0;
        let mut tune = Tune::new();
        tune.prepare(rate);
        tune.set(SPEED, 0.0);
        tune.set(SCALE, 1.0);
        let c_sharp = 440.0 * 2f32.powf(-8.1 / 12.0);
        let mut audio = sine(rate, c_sharp, 0.5, 48_000);
        run(&mut tune, &mut audio);
        let name = note_name(tune.target);
        assert!(name == "C4" || name == "D4", "C# in C major went to {name}");
        let hz = heard_hz(&audio[24_000..], rate);
        let wanted = 440.0 * 2f32.powf((tune.target - 69.0) / 12.0);
        assert!((hz - wanted).abs() < 3.0, "came out at {hz} Hz, wanted {wanted}");
    }

    #[test]
    fn with_no_amount_the_voice_passes_through_late_by_the_latency() {
        let rate = 48_000.0;
        let mut tune = Tune::new();
        tune.prepare(rate);
        tune.set(AMOUNT, 0.0);
        let hz_in = 440.0 * 2f32.powf(-0.4 / 12.0);
        let mut audio = sine(rate, hz_in, 0.5, 48_000);
        run(&mut tune, &mut audio);
        let hz = heard_hz(&audio[24_000..], rate);
        assert!((hz - hz_in).abs() < 1.5, "came out at {hz} Hz");
        let loudest = audio[24_000..].iter().fold(0.0f32, |top, frame| top.max(frame[0].abs()));
        assert!((loudest - 0.5).abs() < 0.08, "level moved to {loudest}");
        assert_eq!(tune.latency(), (LOOK_AHEAD_SECONDS * rate) as usize);
    }

    #[test]
    fn a_sharp_note_comes_down_at_an_even_level_and_formant_moves_leave_the_pitch_alone() {
        let rate = 44_100.0;
        for formant in [0.0, 5.0, -5.0] {
            let mut tune = Tune::new();
            tune.prepare(rate);
            tune.set(SPEED, 0.0);
            tune.set(FORMANT, formant);
            let sharp_e = 329.63 * 2f32.powf(0.35 / 12.0);
            let mut audio = sine(rate, sharp_e, 0.5, 44_100);
            run(&mut tune, &mut audio);
            let hz = repeats_hz(&audio[22_000..], rate);
            assert!((hz - 329.63).abs() < 3.0, "formant {formant}: came out at {hz} Hz");
            if formant == 0.0 {
                let peaks: Vec<f32> = audio[22_000..].chunks(441).map(|part| part.iter().fold(0.0f32, |top, frame| top.max(frame[0].abs()))).collect();
                let (low, high) = peaks.iter().fold((f32::MAX, 0.0f32), |(low, high), peak| (low.min(*peak), high.max(*peak)));
                assert!(high - low < 0.12, "the level wobbles between {low} and {high}");
            }
        }
    }

    #[test]
    fn vibrato_across_the_half_way_point_keeps_one_note() {
        let rate = 48_000.0;
        let mut tune = Tune::new();
        tune.prepare(rate);
        let mut phase = 0.0f32;
        let mut audio: Vec<Frame> = (0..96_000)
            .map(|at| {
                let note = 62.4 + 0.2 * (std::f32::consts::TAU * 5.0 * at as f32 / rate).sin();
                phase = (phase + 440.0 * 2f32.powf((note - 69.0) / 12.0) / rate).fract();
                let value = 0.5 * (std::f32::consts::TAU * phase).sin();
                [value, value]
            })
            .collect();
        let mut targets = Vec::new();
        for block in audio.chunks_mut(480) {
            tune.process(block);
            targets.push(tune.target);
        }
        let settled: Vec<f32> = targets[20..].to_vec();
        assert!(settled.iter().all(|note| *note == settled[0]), "the note kept changing: {:?}", &settled[..20]);
    }

    #[test]
    fn speed_glides_onto_the_note_instead_of_jumping() {
        let rate = 48_000.0;
        let mut tune = Tune::new();
        tune.prepare(rate);
        tune.set(SPEED, 300.0);
        let mut audio = sine(rate, 440.0 * 2f32.powf(-0.4 / 12.0), 0.5, 48_000);
        let mut seen = Vec::new();
        for block in audio.chunks_mut(480) {
            tune.process(block);
            seen.push(tune.applied);
        }
        let early = seen[12];
        let late = *seen.last().unwrap();
        assert!(early > 0.02 && early < 0.3, "after a tenth of a second it had moved {early}");
        assert!((late - 0.4).abs() < 0.04, "it never arrived: {late}");
    }

    fn told(key: u8, sure: f32, seconds: f32) -> Arc<Detected> {
        let shared = Arc::new(Detected::new());
        shared.tell(1, &crate::Heard { bpm: None, key: Some(key), sure, seconds });
        shared
    }

    fn quiet(rate: f32, seconds: f32) -> Vec<Frame> {
        vec![[0.0; 2]; (rate * seconds) as usize]
    }

    const A_MINOR: u8 = 21;
    const D_MAJOR: u8 = 2;

    #[test]
    fn a_settled_reading_reaches_tune() {
        let rate = 48_000.0;
        let mut tune = Tune::new();
        tune.prepare(rate);
        tune.listens_to(told(A_MINOR, 0.8, 20.0));
        tune.set(KEY, FOLLOWED_KEY);
        tune.set(SCALE, FOLLOWED_SCALE);
        run(&mut tune, &mut quiet(rate, 3.0));
        assert_eq!(tune.following, Some(A_MINOR));
        assert_eq!(tune.key_in_use(), 9);
        assert_eq!(tune.steps_in_use(), STEPS[2]);
        assert_eq!(tune.tuning().following(), Some(A_MINOR));
        assert_eq!(tune.tuning().waiting(), None);
    }

    #[test]
    fn a_shaky_or_fresh_reading_is_not_followed() {
        let rate = 48_000.0;
        for (sure, seconds) in [(0.3, 20.0), (0.9, 4.0)] {
            let mut tune = Tune::new();
            tune.prepare(rate);
            tune.listens_to(told(A_MINOR, sure, seconds));
            tune.set(KEY, FOLLOWED_KEY);
            tune.set(SCALE, FOLLOWED_SCALE);
            run(&mut tune, &mut quiet(rate, 3.0));
            assert_eq!(tune.following, None, "{sure} sure after {seconds} s was followed");
            assert_eq!(tune.key_in_use(), 0);
            assert_eq!(tune.steps_in_use(), STEPS[0]);
            assert_eq!(tune.tuning().following(), None);
        }
    }

    #[test]
    fn a_key_set_by_hand_is_never_overridden() {
        let rate = 48_000.0;
        let mut tune = Tune::new();
        tune.prepare(rate);
        tune.listens_to(told(D_MAJOR, 0.95, 30.0));
        tune.set(KEY, 9.0);
        tune.set(SCALE, 2.0);
        run(&mut tune, &mut quiet(rate, 3.0));
        assert_eq!((tune.value(KEY), tune.value(SCALE)), (9.0, 2.0));
        assert_eq!(tune.key_in_use(), 9);
        assert_eq!(tune.steps_in_use(), STEPS[2]);
        assert_eq!(tune.following, None);
        assert_eq!(tune.tuning().following(), None);
    }

    #[test]
    fn a_new_key_waits_for_the_gap_between_phrases() {
        let rate = 48_000.0;
        let mut tune = Tune::new();
        tune.prepare(rate);
        let shared = told(A_MINOR, 0.8, 20.0);
        tune.listens_to(shared.clone());
        tune.set(KEY, FOLLOWED_KEY);
        tune.set(SCALE, FOLLOWED_SCALE);
        run(&mut tune, &mut quiet(rate, 3.0));
        assert_eq!(tune.following, Some(A_MINOR));
        shared.tell(1, &crate::Heard { bpm: None, key: Some(D_MAJOR), sure: 0.9, seconds: 40.0 });
        let mut singing = sine(rate, 440.0, 0.5, (rate * 2.5) as usize);
        run(&mut tune, &mut singing);
        assert_eq!(tune.following, Some(A_MINOR), "it swapped key while the voice was still going");
        assert_eq!(tune.waiting, Some(D_MAJOR));
        assert_eq!(tune.tuning().waiting(), Some(D_MAJOR));
        run(&mut tune, &mut quiet(rate, 0.5));
        assert_eq!(tune.following, Some(D_MAJOR));
        assert_eq!(tune.steps_in_use(), STEPS[1]);
    }

    #[test]
    fn with_no_loupe_key_tune_tunes_as_it_always_did() {
        let rate = 48_000.0;
        let mut by_hand = Tune::new();
        by_hand.prepare(rate);
        by_hand.set(SPEED, 0.0);
        let mut follower = Tune::new();
        follower.prepare(rate);
        follower.listens_to(Arc::new(Detected::new()));
        follower.set(SPEED, 0.0);
        follower.set(KEY, FOLLOWED_KEY);
        follower.set(SCALE, FOLLOWED_SCALE);
        let flat_a = 440.0 * 2f32.powf(-0.3 / 12.0);
        let mut one = sine(rate, flat_a, 0.5, 48_000);
        let mut two = one.clone();
        run(&mut by_hand, &mut one);
        run(&mut follower, &mut two);
        assert_eq!(one, two);
        assert_eq!(note_name(follower.target), note_name(by_hand.target));
        assert_eq!(follower.following, None);
    }

    #[test]
    fn following_comes_back_after_a_save_and_a_reload() {
        let rate = 48_000.0;
        let mut tune = Tune::new();
        tune.prepare(rate);
        tune.set(KEY, FOLLOWED_KEY);
        tune.set(SCALE, FOLLOWED_SCALE);
        let saved: Vec<u8> = (0..tune.params().len()).flat_map(|index| tune.value(index).to_le_bytes()).collect();
        let mut again = Tune::new();
        again.prepare(rate);
        for (index, four) in saved.chunks_exact(4).enumerate() {
            again.set(index, f32::from_le_bytes([four[0], four[1], four[2], four[3]]));
        }
        assert_eq!((again.value(KEY), again.value(SCALE)), (FOLLOWED_KEY, FOLLOWED_SCALE));
        assert_eq!(again.params()[KEY].chosen(FOLLOWED_KEY), Some(FOUND));
        assert_eq!(again.params()[SCALE].chosen(FOLLOWED_SCALE), Some(FOUND));
        again.listens_to(told(A_MINOR, 0.8, 20.0));
        run(&mut again, &mut quiet(rate, 3.0));
        assert_eq!(again.following, Some(A_MINOR));
    }

    #[test]
    fn silence_stays_silent_and_the_history_shows_no_note() {
        let mut tune = Tune::new();
        let mut audio = vec![[0.0; 2]; 9_600];
        run(&mut tune, &mut audio);
        assert!(audio.iter().all(|frame| frame[0] == 0.0));
        let mut seen = [Moment::default(); 4];
        tune.history().latest(&mut seen);
        assert!(seen.iter().all(|moment| moment.input == SILENT_NOTE));
    }
}
