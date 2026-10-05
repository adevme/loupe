use crate::{gain_of, settled, Effect, Frame, Param, Unit, Values, DEFAULT_RATE};
const PARAMS: [Param; 8] = [
    Param::new("voices", "Voices", 1.0, 4.0, 2.0, Unit::Choice),
    Param::new("delay", "Delay", 5.0, 80.0, 22.0, Unit::Milliseconds),
    Param::new("spread", "Spread", 0.0, 100.0, 60.0, Unit::Percent),
    Param::new("detune", "Detune", 0.0, 50.0, 12.0, Unit::Percent),
    Param::new("wander", "Wander", 0.0, 100.0, 35.0, Unit::Percent),
    Param::new("width", "Width", 0.0, 200.0, 100.0, Unit::Percent),
    Param::new("level", "Voice level", -40.0, 0.0, -14.0, Unit::Decibels),
    Param::new("mix", "Mix", 0.0, 100.0, 100.0, Unit::Percent),
];
pub const VOICES: usize = 0;
pub const DELAY: usize = 1;
pub const SPREAD: usize = 2;
pub const DETUNE: usize = 3;
pub const WANDER: usize = 4;
pub const WIDTH: usize = 5;
pub const LEVEL: usize = 6;
pub const MIX: usize = 7;
pub const MOST_VOICES: usize = 4;
const LONGEST_DELAY_MS: f32 = 120.0;
const WIDEST_DETUNE_CENTS: f32 = 25.0;
const WANDER_HZ: [f32; MOST_VOICES] = [0.17, 0.23, 0.31, 0.41];
const SIDES: [f32; MOST_VOICES] = [-1.0, 1.0, -0.6, 0.6];
struct Voice {
    at: f32,
    turn: f32,
    step: f32,
    centre: f32,
    reach: f32,
    side: f32,
}
pub struct Doubler {
    values: Values<8>,
    rate: f32,
    voices: usize,
    level: f32,
    mix: f32,
    width: f32,
    tape: [Vec<f32>; 2],
    writing: usize,
    voice: Vec<Voice>,
}
fn read_tape(tape: &[f32], writing: usize, back: f32) -> f32 {
    let len = tape.len();
    if len == 0 {
        return 0.0;
    }
    let want = writing as f32 + len as f32 - back;
    let whole = want.floor();
    let part = want - whole;
    let first = (whole as usize) % len;
    let second = (first + 1) % len;
    tape[first] * (1.0 - part) + tape[second] * part
}
impl Doubler {
    pub fn new() -> Self {
        let mut doubler = Self {
            values: Values::new(&PARAMS),
            rate: DEFAULT_RATE,
            voices: 2,
            level: 0.0,
            mix: 1.0,
            width: 1.0,
            tape: [Vec::new(), Vec::new()],
            writing: 0,
            voice: Vec::new(),
        };
        doubler.prepare(DEFAULT_RATE);
        doubler
    }
    fn read_knobs(&mut self) {
        self.voices = (self.values.get(VOICES).round() as usize).clamp(1, MOST_VOICES);
        self.level = gain_of(self.values.get(LEVEL));
        self.mix = self.values.get(MIX) / 100.0;
        self.width = self.values.get(WIDTH) / 100.0;
        let middle = self.values.get(DELAY) * 0.001 * self.rate;
        let spread = self.values.get(SPREAD) / 100.0;
        let detune = self.values.get(DETUNE) / 100.0 * WIDEST_DETUNE_CENTS;
        let wander = self.values.get(WANDER) / 100.0;
        self.voice.clear();
        for which in 0..self.voices {
            let lean = match self.voices {
                1 => 0.0,
                more => which as f32 / (more - 1) as f32 * 2.0 - 1.0,
            };
            let cents = detune * lean;
            self.voice.push(Voice {
                at: 0.0,
                turn: which as f32 * 0.37,
                step: WANDER_HZ[which] / self.rate,
                centre: middle * (1.0 + lean * spread * 0.6),
                reach: middle * spread * 0.35 * wander + cents.abs() * self.rate / 4000.0,
                side: SIDES[which],
            });
        }
    }
}
impl Default for Doubler {
    fn default() -> Self {
        Self::new()
    }
}
impl Effect for Doubler {
    fn name(&self) -> &'static str {
        "Loupe Doubler"
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
        let room = (LONGEST_DELAY_MS * 0.002 * rate) as usize + 4;
        self.tape = [vec![0.0; room], vec![0.0; room]];
        self.read_knobs();
        self.values.take_change();
        self.reset();
    }
    fn reset(&mut self) {
        for side in self.tape.iter_mut() {
            side.iter_mut().for_each(|sample| *sample = 0.0);
        }
        self.writing = 0;
        for voice in self.voice.iter_mut() {
            voice.at = voice.centre;
            voice.turn = 0.0;
        }
    }
    fn process(&mut self, audio: &mut [Frame]) {
        if self.values.take_change() {
            self.read_knobs();
        }
        if self.voice.is_empty() {
            return;
        }
        let room = self.tape[0].len();
        for frame in audio.iter_mut() {
            self.tape[0][self.writing] = frame[0];
            self.tape[1][self.writing] = frame[1];
            let (mut left, mut right) = (0.0, 0.0);
            for voice in self.voice.iter_mut() {
                voice.turn = (voice.turn + voice.step) % 1.0;
                let swing = (voice.turn * std::f32::consts::TAU).sin();
                let wanted = voice.centre + swing * voice.reach;
                voice.at = settled(voice.at + (wanted - voice.at) * 0.0004);
                let back = voice.at.clamp(1.0, room as f32 - 2.0);
                let heard = read_tape(&self.tape[0], self.writing, back) * 0.5 + read_tape(&self.tape[1], self.writing, back) * 0.5;
                let placed = voice.side * self.width;
                left += heard * (1.0 - placed).clamp(0.0, 2.0) * 0.5;
                right += heard * (1.0 + placed).clamp(0.0, 2.0) * 0.5;
            }
            self.writing = (self.writing + 1) % room;
            let share = self.level / (self.voices as f32).sqrt();
            frame[0] += left * share * self.mix;
            frame[1] += right * share * self.mix;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db_of;

    const RATE: f32 = 48_000.0;

    fn tone(seconds: f32) -> Vec<Frame> {
        (0..(RATE * seconds) as usize)
            .map(|at| {
                let turn = at as f32 / RATE * 440.0 * std::f32::consts::TAU;
                let sample = turn.sin() * 0.5;
                [sample, sample]
            })
            .collect()
    }

    fn sides(audio: &[Frame]) -> (f32, f32) {
        let mid = audio.iter().map(|frame| ((frame[0] + frame[1]) * 0.5).powi(2)).sum::<f32>();
        let side = audio.iter().map(|frame| ((frame[0] - frame[1]) * 0.5).powi(2)).sum::<f32>();
        (mid, side)
    }

    #[test]
    fn the_voices_come_out_to_the_sides() {
        let mut doubler = Doubler::new();
        doubler.prepare(RATE);
        let mut audio = tone(1.0);
        doubler.process(&mut audio);
        let (mid, side) = sides(&audio[RATE as usize / 2..]);
        assert!(side > 0.0, "two voices should put something off centre");
        assert!(mid > side, "the voice itself stays in the middle");
    }

    #[test]
    fn one_voice_stays_where_it_was_put() {
        let mut doubler = Doubler::new();
        doubler.set(VOICES, 1.0);
        doubler.set(WIDTH, 0.0);
        doubler.prepare(RATE);
        let mut audio = tone(1.0);
        doubler.process(&mut audio);
        let (_, side) = sides(&audio[RATE as usize / 2..]);
        assert!(side < 1e-6, "with no width there is nothing off centre, got {side}");
    }

    #[test]
    fn no_mix_leaves_the_voice_alone() {
        let mut doubler = Doubler::new();
        doubler.set(MIX, 0.0);
        doubler.prepare(RATE);
        let plain = tone(0.5);
        let mut audio = plain.clone();
        doubler.process(&mut audio);
        for (was, now) in plain.iter().zip(&audio) {
            assert!((was[0] - now[0]).abs() < 1e-6, "nothing is added at no mix");
        }
    }

    #[test]
    fn the_voices_are_quieter_than_the_one_that_was_sung() {
        let mut doubler = Doubler::new();
        doubler.prepare(RATE);
        let plain = tone(1.0);
        let mut audio = plain.clone();
        doubler.process(&mut audio);
        let was = db_of(plain.iter().fold(0.0f32, |most, frame| most.max(frame[0].abs())));
        let now = db_of(audio.iter().fold(0.0f32, |most, frame| most.max(frame[0].abs())));
        assert!(now > was, "doubling adds level, got {now} from {was}");
        assert!(now - was < 6.0, "it should not double the loudness, got {} dB more", now - was);
    }
}
