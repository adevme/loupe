use crate::{gain_of, settled, Effect, Frame, Param, Unit, Values, DEFAULT_RATE};

const PARAMS: [Param; 6] = [
    Param::new("reduction", "Reduction", 0.0, 40.0, 12.0, Unit::Decibels),
    Param::new("threshold", "Sensitivity", 0.0, 24.0, 6.0, Unit::Decibels),
    Param::new("smoothing", "Smoothing", 0.0, 100.0, 50.0, Unit::Percent),
    Param::new("learn", "Follow the room", 0.0, 1.0, 1.0, Unit::Switch),
    Param::new("keep_low", "Leave below", 20.0, 500.0, 60.0, Unit::Hertz),
    Param::new("mix", "Mix", 0.0, 100.0, 100.0, Unit::Percent),
];
pub const REDUCTION: usize = 0;
pub const SENSITIVITY: usize = 1;
pub const SMOOTHING: usize = 2;
pub const FOLLOW: usize = 3;
pub const KEEP_LOW: usize = 4;
pub const MIX: usize = 5;

const WINDOW: usize = 2048;
const HOP: usize = WINDOW / 4;
const BINS: usize = WINDOW / 2 + 1;
const QUIETEST_NOISE: f32 = 1e-12;
const NOISE_STARTS_AT: f32 = 1.0;
const NOISE_FALLS_PER_HOP: f32 = 0.35;
const NEIGHBOURS: usize = 3;
const NOISE_SETTLES_PER_HOP: f32 = 0.02;
const SIGNAL_IS_LOUDER_THAN_NOISE: f32 = 3.0;
const NOISE_IS_LOUDER_THAN_ITS_FLOOR: f32 = 1.4;

fn turn_pairs(real: &mut [f32], imaginary: &mut [f32]) {
    let n = real.len();
    let mut target = 0;
    for source in 1..n {
        let mut bit = n >> 1;
        while target & bit != 0 {
            target ^= bit;
            bit >>= 1;
        }
        target |= bit;
        if source < target {
            real.swap(source, target);
            imaginary.swap(source, target);
        }
    }
    let mut span = 2;
    while span <= n {
        let step = -std::f32::consts::TAU / span as f32;
        for start in (0..n).step_by(span) {
            for at in 0..span / 2 {
                let (sin, cos) = (step * at as f32).sin_cos();
                let here = start + at;
                let there = here + span / 2;
                let real_part = real[there] * cos - imaginary[there] * sin;
                let imaginary_part = real[there] * sin + imaginary[there] * cos;
                real[there] = real[here] - real_part;
                imaginary[there] = imaginary[here] - imaginary_part;
                real[here] += real_part;
                imaginary[here] += imaginary_part;
            }
        }
        span <<= 1;
    }
}

fn turn_back(real: &mut [f32], imaginary: &mut [f32]) {
    imaginary.iter_mut().for_each(|value| *value = -*value);
    turn_pairs(real, imaginary);
    let share = 1.0 / real.len() as f32;
    real.iter_mut().for_each(|value| *value *= share);
}

struct Settings {
    floor: f32,
    over: f32,
    towards: f32,
    following: bool,
    keep_below: usize,
}

struct Side {
    heard: Vec<f32>,
    heard_at: usize,
    coming: Vec<f32>,
    coming_at: usize,
    dry: Vec<f32>,
    dry_at: usize,
    real: Vec<f32>,
    imaginary: Vec<f32>,
    noise: Vec<f32>,
    gain: Vec<f32>,
    loud: Vec<f32>,
    soft: Vec<f32>,
}

impl Side {
    fn new() -> Self {
        Self {
            heard: vec![0.0; WINDOW],
            heard_at: 0,
            coming: vec![0.0; WINDOW],
            coming_at: 0,
            dry: vec![0.0; WINDOW + 1],
            dry_at: 0,
            real: vec![0.0; WINDOW],
            imaginary: vec![0.0; WINDOW],
            noise: vec![NOISE_STARTS_AT; BINS],
            gain: vec![1.0; BINS],
            loud: vec![0.0; BINS],
            soft: vec![0.0; BINS],
        }
    }

    fn start_again(&mut self) {
        self.heard.iter_mut().for_each(|sample| *sample = 0.0);
        self.coming.iter_mut().for_each(|sample| *sample = 0.0);
        self.dry.iter_mut().for_each(|sample| *sample = 0.0);
        self.noise.iter_mut().for_each(|level| *level = NOISE_STARTS_AT);
        self.gain.iter_mut().for_each(|keep| *keep = 1.0);
        self.heard_at = 0;
        self.coming_at = 0;
        self.dry_at = 0;
    }

    fn work_out_a_window(&mut self, shape: &[f32], how: &Settings) {
        for at in 0..WINDOW {
            self.real[at] = self.heard[(self.heard_at + at) % WINDOW] * shape[at];
            self.imaginary[at] = 0.0;
        }
        turn_pairs(&mut self.real, &mut self.imaginary);
        for bin in 0..BINS {
            self.loud[bin] = (self.real[bin] * self.real[bin] + self.imaginary[bin] * self.imaginary[bin]).sqrt();
        }
        for bin in 0..BINS {
            let from = bin.saturating_sub(NEIGHBOURS);
            let to = (bin + NEIGHBOURS).min(BINS - 1);
            let mut total = 0.0;
            for near in from..=to {
                total += self.loud[near];
            }
            self.soft[bin] = total / (to - from + 1) as f32;
        }
        for bin in 0..BINS {
            let steady = self.soft[bin];
            if how.following {
                let floor = &mut self.noise[bin];
                if steady < *floor {
                    *floor = settled(*floor + (steady - *floor) * NOISE_FALLS_PER_HOP).max(QUIETEST_NOISE);
                } else if steady < *floor * SIGNAL_IS_LOUDER_THAN_NOISE {
                    *floor = settled(*floor + (steady - *floor) * NOISE_SETTLES_PER_HOP).max(QUIETEST_NOISE);
                }
            }
            let wanted = match bin < how.keep_below || steady <= QUIETEST_NOISE {
                true => 1.0,
                false => {
                    let against = how.over * NOISE_IS_LOUDER_THAN_ITS_FLOOR * self.noise[bin] / steady;
                    (1.0 - against * against).max(0.0).sqrt().clamp(how.floor, 1.0)
                }
            };

            let was = self.gain[bin];
            let keep = settled(was + (wanted - was) * how.towards);
            self.gain[bin] = keep;
            self.real[bin] *= keep;
            self.imaginary[bin] *= keep;
            if bin > 0 && bin < BINS - 1 {
                let mirror = WINDOW - bin;
                self.real[mirror] = self.real[bin];
                self.imaginary[mirror] = -self.imaginary[bin];
            }
        }
        turn_back(&mut self.real, &mut self.imaginary);
        for at in 0..WINDOW {
            let into = (self.coming_at + at) % WINDOW;
            self.coming[into] += self.real[at] * shape[at];
        }
    }

    fn take(&mut self, heard: f32, shape: &[f32], how: &Settings, hop: &mut usize) -> f32 {
        self.heard[self.heard_at] = heard;
        self.heard_at = (self.heard_at + 1) % WINDOW;
        self.dry[self.dry_at] = heard;
        self.dry_at = (self.dry_at + 1) % (WINDOW + 1);
        if *hop == 0 {
            self.work_out_a_window(shape, how);
        }
        *hop = (*hop + 1) % HOP;
        let cleaned = self.coming[self.coming_at];
        self.coming[self.coming_at] = 0.0;
        self.coming_at = (self.coming_at + 1) % WINDOW;
        cleaned
    }

    fn held(&self) -> f32 {
        self.dry[self.dry_at]
    }
}

pub struct Denoise {
    values: Values<6>,
    rate: f32,
    how: Settings,
    mix: f32,
    shape: Vec<f32>,
    left: Side,
    right: Side,
    hop_left: usize,
    hop_right: usize,
}

impl Denoise {
    pub fn new() -> Self {
        let mut denoise = Self {
            values: Values::new(&PARAMS),
            rate: DEFAULT_RATE,
            how: Settings { floor: 0.0, over: 2.0, towards: 0.5, following: true, keep_below: 0 },
            mix: 1.0,
            shape: Vec::new(),
            left: Side::new(),
            right: Side::new(),
            hop_left: 0,
            hop_right: 0,
        };
        denoise.prepare(DEFAULT_RATE);
        denoise
    }

    fn read_knobs(&mut self) {
        self.how.floor = gain_of(-self.values.get(REDUCTION));
        self.how.over = gain_of(self.values.get(SENSITIVITY));
        self.how.towards = 1.0 - self.values.get(SMOOTHING) / 100.0 * 0.9;
        self.how.following = self.values.get(FOLLOW) >= 0.5;
        self.mix = self.values.get(MIX) / 100.0;
        let per_bin = self.rate / WINDOW as f32;
        self.how.keep_below = (self.values.get(KEEP_LOW) / per_bin).ceil() as usize;
    }
}

impl Default for Denoise {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for Denoise {
    fn name(&self) -> &'static str {
        "Loupe De-noise"
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
        WINDOW
    }

    fn prepare(&mut self, rate: f32) {
        self.rate = rate;
        let share = (2.0 / 3.0f32).sqrt();
        self.shape = (0..WINDOW)
            .map(|at| {
                let turn = std::f32::consts::TAU * at as f32 / WINDOW as f32;
                (0.5 - 0.5 * turn.cos()) * share
            })
            .collect();
        self.read_knobs();
        self.values.take_change();
        self.reset();
    }

    fn reset(&mut self) {
        self.left.start_again();
        self.right.start_again();
        self.hop_left = 0;
        self.hop_right = 0;
    }

    fn process(&mut self, audio: &mut [Frame]) {
        if self.values.take_change() {
            self.read_knobs();
        }
        for frame in audio.iter_mut() {
            let was_left = frame[0];
            let was_right = frame[1];
            let left = self.left.take(was_left, &self.shape, &self.how, &mut self.hop_left);
            let right = self.right.take(was_right, &self.shape, &self.how, &mut self.hop_right);
            frame[0] = self.left.held() * (1.0 - self.mix) + left * self.mix;
            frame[1] = self.right.held() * (1.0 - self.mix) + right * self.mix;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db_of;

    const RATE: f32 = 48_000.0;

    fn hiss_and_voice(seconds: f32, voice_db: f32, hiss_db: f32) -> Vec<Frame> {
        let voice = gain_of(voice_db);
        let hiss = gain_of(hiss_db);
        let mut seed: u32 = 12345;
        (0..(RATE * seconds) as usize)
            .map(|at| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let noise = (seed >> 9) as f32 / (1 << 22) as f32 - 1.0;
                let turn = at as f32 / RATE * 220.0 * std::f32::consts::TAU;
                let sample = turn.sin() * voice + noise * hiss;
                [sample, sample]
            })
            .collect()
    }

    fn strength(audio: &[Frame], hz: f32) -> f32 {
        let (mut sin, mut cos) = (0.0f64, 0.0f64);
        for (at, frame) in audio.iter().enumerate() {
            let turn = std::f64::consts::TAU * hz as f64 * at as f64 / RATE as f64;
            sin += frame[0] as f64 * turn.sin();
            cos += frame[0] as f64 * turn.cos();
        }
        ((sin * sin + cos * cos).sqrt() * 2.0 / audio.len() as f64) as f32
    }

    fn loudness(audio: &[Frame]) -> f32 {
        db_of((audio.iter().map(|frame| frame[0] * frame[0]).sum::<f32>() / audio.len() as f32).sqrt())
    }

    #[test]
    fn hiss_on_its_own_is_taken_away() {
        let mut denoise = Denoise::new();
        denoise.prepare(RATE);
        let mut audio = hiss_and_voice(3.0, -200.0, -40.0);
        denoise.process(&mut audio);
        let settled = loudness(&audio[audio.len() - RATE as usize..]);
        assert!(settled < -55.0, "hiss alone should come down a long way, got {settled}");
    }

    #[test]
    fn the_voice_is_left_standing() {
        let mut denoise = Denoise::new();
        denoise.prepare(RATE);
        let plain = hiss_and_voice(3.0, -20.0, -60.0);
        let mut audio = plain.clone();
        denoise.process(&mut audio);
        let from = audio.len() - RATE as usize;
        let was = strength(&plain[from..], 220.0);
        let now = strength(&audio[from..], 220.0);
        let lost = db_of(now) - db_of(was);
        assert!(lost > -1.5, "the voice should survive, lost {lost} dB");
    }

    #[test]
    fn no_mix_leaves_the_sound_alone() {
        let mut denoise = Denoise::new();
        denoise.set(MIX, 0.0);
        denoise.prepare(RATE);
        let plain = hiss_and_voice(0.5, -20.0, -50.0);
        let mut audio = plain.clone();
        denoise.process(&mut audio);
        let late = WINDOW + 10;
        for (at, now) in audio.iter().enumerate().skip(late) {
            let was = plain[at - WINDOW][0];
            assert!((was - now[0]).abs() < 1e-5, "at {at} the sound changed with no mix");
        }
    }

    #[test]
    fn it_says_how_far_behind_it_runs() {
        assert_eq!(Denoise::new().latency(), WINDOW);
    }
}
