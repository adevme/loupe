use std::sync::Arc;

use crate::history::{Gatherer, History};
use crate::smooth::Smoothed;
use crate::{db_of, decay_per_sample, gain_of, settled, Effect, Frame, Param, Unit, Values, DEFAULT_RATE};

const PARAMS: [Param; 4] = [
    Param::new("attack", "Attack", -100.0, 100.0, 0.0, Unit::Percent),
    Param::new("sustain", "Sustain", -100.0, 100.0, 0.0, Unit::Percent),
    Param::new("output", "Output", -24.0, 12.0, 0.0, Unit::Decibels),
    Param::new("mix", "Mix", 0.0, 100.0, 100.0, Unit::Percent),
];
pub const ATTACK: usize = 0;
pub const SUSTAIN: usize = 1;
pub const OUTPUT: usize = 2;
pub const MIX: usize = 3;
const MOST_DB: f32 = 15.0;
const QUICK_RISE_MS: f32 = 0.2;
const SLOW_RISE_MS: f32 = 25.0;
const SHORT_FALL_MS: f32 = 40.0;
const LONG_FALL_MS: f32 = 400.0;
const GAIN_GLIDE_MS: f32 = 1.0;
const QUIETEST: f32 = 1e-5;

pub struct Transient {
    values: Values<4>,
    attack: f32,
    sustain: f32,
    output: Smoothed,
    mix: Smoothed,
    quick: f32,
    slow: f32,
    short: f32,
    long: f32,
    gain_db: f32,
    quick_rise: f32,
    slow_rise: f32,
    short_fall: f32,
    long_fall: f32,
    glide: f32,
    history: Arc<History>,
    gatherer: Gatherer,
}

impl Transient {
    pub fn new() -> Self {
        let mut transient = Self {
            values: Values::new(&PARAMS),
            attack: 0.0,
            sustain: 0.0,
            output: Smoothed::new(1.0),
            mix: Smoothed::new(1.0),
            quick: 0.0,
            slow: 0.0,
            short: 0.0,
            long: 0.0,
            gain_db: 0.0,
            quick_rise: 0.0,
            slow_rise: 0.0,
            short_fall: 0.0,
            long_fall: 0.0,
            glide: 0.0,
            history: Arc::new(History::new()),
            gatherer: Gatherer::new(DEFAULT_RATE),
        };
        transient.prepare(DEFAULT_RATE);
        transient
    }

    pub fn history(&self) -> Arc<History> {
        self.history.clone()
    }

    fn read_knobs(&mut self) {
        self.attack = self.values.get(ATTACK) / 100.0;
        self.sustain = self.values.get(SUSTAIN) / 100.0;
        self.output.aim(gain_of(self.values.get(OUTPUT)));
        self.mix.aim(self.values.get(MIX) / 100.0);
    }
}

impl Default for Transient {
    fn default() -> Self {
        Self::new()
    }
}

fn follow(now: f32, heard: f32, rise: f32, fall: f32) -> f32 {
    let speed = if heard > now { rise } else { fall };
    heard + (now - heard) * speed
}

impl Effect for Transient {
    fn history(&self) -> Option<Arc<History>> {
        Some(self.history())
    }

    fn name(&self) -> &'static str {
        "Loupe Transient"
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
        self.quick_rise = decay_per_sample(QUICK_RISE_MS, rate);
        self.slow_rise = decay_per_sample(SLOW_RISE_MS, rate);
        self.short_fall = decay_per_sample(SHORT_FALL_MS, rate);
        self.long_fall = decay_per_sample(LONG_FALL_MS, rate);
        self.glide = decay_per_sample(GAIN_GLIDE_MS, rate);
        self.output.prepare(rate);
        self.mix.prepare(rate);
        self.gatherer = Gatherer::new(rate);
        self.read_knobs();
        self.values.take_change();
        self.output.snap();
        self.mix.snap();
        self.reset();
    }

    fn reset(&mut self) {
        self.quick = 0.0;
        self.slow = 0.0;
        self.short = 0.0;
        self.long = 0.0;
        self.gain_db = 0.0;
    }

    fn process(&mut self, audio: &mut [Frame]) {
        if self.values.take_change() {
            self.read_knobs();
        }
        for frame in audio.iter_mut() {
            let heard = frame[0].abs().max(frame[1].abs());
            self.quick = follow(self.quick, heard, self.quick_rise, self.short_fall);
            self.slow = follow(self.slow, heard, self.slow_rise, self.short_fall);
            self.short = follow(self.short, heard, self.quick_rise, self.short_fall);
            self.long = follow(self.long, heard, self.quick_rise, self.long_fall);
            let (punch, tail) = if self.quick > QUIETEST {
                ((db_of(self.quick) - db_of(self.slow)).clamp(0.0, MOST_DB), (db_of(self.long) - db_of(self.short)).clamp(0.0, MOST_DB))
            } else {
                (0.0, 0.0)
            };
            let wanted = (self.attack * punch + self.sustain * tail).clamp(-MOST_DB, MOST_DB);
            self.gain_db = wanted + (self.gain_db - wanted) * self.glide;
            let gain = gain_of(self.gain_db);
            let mix = self.mix.next();
            let output = self.output.next();
            let shaped = 1.0 + (gain - 1.0) * mix;
            frame[0] *= shaped * output;
            frame[1] *= shaped * output;
            self.gatherer.hear_change(&self.history, heard, frame[0].abs().max(frame[1].abs()), self.gain_db * mix);
        }
        self.quick = settled(self.quick);
        self.slow = settled(self.slow);
        self.short = settled(self.short);
        self.long = settled(self.long);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{in_blocks, peak, rms, sine};

    fn hits(rate: f32) -> Vec<Frame> {
        let tone = sine(rate, 120.0, 1.0, (rate * 2.0) as usize);
        tone.iter()
            .enumerate()
            .map(|(i, frame)| {
                let since = (i % (rate as usize / 2)) as f32 / rate;
                let level = 0.8 * (-since * 6.0).exp();
                [frame[0] * level, frame[1] * level]
            })
            .collect()
    }

    fn shaped(attack: f32, sustain: f32) -> Vec<Frame> {
        let rate = 48_000.0;
        let mut transient = Transient::new();
        transient.prepare(rate);
        transient.set(ATTACK, attack);
        transient.set(SUSTAIN, sustain);
        let mut audio = hits(rate);
        in_blocks(&mut transient, &mut audio, 256);
        audio
    }

    fn start_and_tail(audio: &[Frame]) -> (f32, f32) {
        let hit = 24_000;
        (peak(&audio[hit..hit + 960]), rms(&audio[hit + 9_600..hit + 19_200]))
    }

    #[test]
    fn centred_knobs_change_nothing() {
        let input = hits(48_000.0);
        let output = shaped(0.0, 0.0);
        let biggest = input.iter().zip(&output).map(|(a, b)| (a[0] - b[0]).abs()).fold(0.0f32, f32::max);
        assert!(biggest < 1e-6, "{biggest}");
    }

    #[test]
    fn attack_lifts_or_softens_the_start_of_each_hit() {
        let (plain_start, plain_tail) = start_and_tail(&hits(48_000.0));
        let (punchy, punchy_tail) = start_and_tail(&shaped(100.0, 0.0));
        let (soft, _) = start_and_tail(&shaped(-100.0, 0.0));
        assert!(db_of(punchy / plain_start) > 2.0, "punch only {} dB", db_of(punchy / plain_start));
        assert!(db_of(soft / plain_start) < -2.0, "soft only {} dB", db_of(soft / plain_start));
        assert!(db_of(punchy_tail / plain_tail).abs() < 1.5, "the tail moved {} dB", db_of(punchy_tail / plain_tail));
    }

    #[test]
    fn sustain_stretches_or_shortens_the_tail() {
        let (_, plain_tail) = start_and_tail(&hits(48_000.0));
        let (_, long) = start_and_tail(&shaped(0.0, 100.0));
        let (_, short) = start_and_tail(&shaped(0.0, -100.0));
        assert!(db_of(long / plain_tail) > 2.0, "{}", db_of(long / plain_tail));
        assert!(db_of(short / plain_tail) < -2.0, "{}", db_of(short / plain_tail));
    }
}
