use std::sync::Arc;

use crate::history::{Gatherer, History};
use crate::{db_of, decay_per_sample, gain_of, settled, Effect, Frame, Param, Unit, Values, DEFAULT_RATE};

const PARAMS: [Param; 7] = [
    Param::new("target", "Target", -40.0, -6.0, -18.0, Unit::Decibels),
    Param::new("range_up", "Reach up", 0.0, 24.0, 12.0, Unit::Decibels),
    Param::new("range_down", "Reach down", 0.0, 24.0, 12.0, Unit::Decibels),
    Param::new("speed", "Speed", 20.0, 2000.0, 300.0, Unit::Milliseconds),
    Param::new("idle", "Idle below", -80.0, -20.0, -45.0, Unit::Decibels),
    Param::new("sensitivity", "Voice focus", 0.0, 100.0, 60.0, Unit::Percent),
    Param::new("output", "Output", -24.0, 24.0, 0.0, Unit::Decibels),
];
pub const TARGET: usize = 0;
pub const REACH_UP: usize = 1;
pub const REACH_DOWN: usize = 2;
pub const SPEED: usize = 3;
pub const IDLE: usize = 4;
pub const SENSITIVITY: usize = 5;
pub const OUTPUT: usize = 6;

const LOUDNESS_WINDOW_MS: f32 = 40.0;
const VOICE_LOW_HZ: f32 = 300.0;
const VOICE_HIGH_HZ: f32 = 3400.0;
const HOLD_MS: f32 = 120.0;

pub struct Rider {
    values: Values<7>,
    rate: f32,
    target_db: f32,
    up_db: f32,
    down_db: f32,
    idle_db: f32,
    voice_share: f32,
    output: f32,
    towards: f32,
    loudness_fall: f32,
    loudness: f32,
    band_loudness: f32,
    low_state: [f32; 2],
    high_state: [f32; 2],
    hold: usize,
    held: usize,
    ride_db: f32,
    history: Arc<History>,
    gatherer: Gatherer,
}

impl Rider {
    pub fn new() -> Self {
        let mut rider = Self {
            values: Values::new(&PARAMS),
            rate: DEFAULT_RATE,
            target_db: 0.0,
            up_db: 0.0,
            down_db: 0.0,
            idle_db: 0.0,
            voice_share: 0.0,
            output: 1.0,
            towards: 0.0,
            loudness_fall: 0.0,
            loudness: 0.0,
            band_loudness: 0.0,
            low_state: [0.0; 2],
            high_state: [0.0; 2],
            hold: 0,
            held: 0,
            ride_db: 0.0,
            history: Arc::new(History::new()),
            gatherer: Gatherer::new(DEFAULT_RATE),
        };
        rider.prepare(DEFAULT_RATE);
        rider
    }

    pub fn history(&self) -> Arc<History> {
        self.history.clone()
    }

    pub fn riding_db(&self) -> f32 {
        self.ride_db
    }

    fn read_knobs(&mut self) {
        self.target_db = self.values.get(TARGET);
        self.up_db = self.values.get(REACH_UP);
        self.down_db = self.values.get(REACH_DOWN);
        self.idle_db = self.values.get(IDLE);
        self.voice_share = self.values.get(SENSITIVITY) / 100.0;
        self.output = gain_of(self.values.get(OUTPUT));
        self.towards = 1.0 - decay_per_sample(self.values.get(SPEED), self.rate);
    }

    fn band_of(&mut self, heard: f32) -> f32 {
        let low = 1.0 - decay_per_sample(1000.0 / (VOICE_LOW_HZ * std::f32::consts::TAU), self.rate);
        let high = 1.0 - decay_per_sample(1000.0 / (VOICE_HIGH_HZ * std::f32::consts::TAU), self.rate);
        self.low_state[0] += (heard - self.low_state[0]) * high;
        self.high_state[0] += (heard - self.high_state[0]) * low;
        self.low_state[0] - self.high_state[0]
    }
}

impl Default for Rider {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for Rider {
    fn history(&self) -> Option<Arc<History>> {
        Some(self.history())
    }

    fn name(&self) -> &'static str {
        "Loupe Rider"
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
        self.loudness_fall = decay_per_sample(LOUDNESS_WINDOW_MS, rate);
        self.hold = (HOLD_MS * 0.001 * rate) as usize;
        self.gatherer = Gatherer::new(rate);
        self.read_knobs();
        self.values.take_change();
        self.reset();
    }

    fn reset(&mut self) {
        self.loudness = 0.0;
        self.band_loudness = 0.0;
        self.low_state = [0.0; 2];
        self.high_state = [0.0; 2];
        self.held = 0;
        self.ride_db = 0.0;
    }

    fn process(&mut self, audio: &mut [Frame]) {
        if self.values.take_change() {
            self.read_knobs();
        }
        for frame in audio.iter_mut() {
            let heard = (frame[0] + frame[1]) * 0.5;
            let whole = heard.abs();
            let voice = self.band_of(heard).abs();
            self.loudness = whole.max(self.loudness + (whole - self.loudness) * (1.0 - self.loudness_fall));
            self.band_loudness = voice.max(self.band_loudness + (voice - self.band_loudness) * (1.0 - self.loudness_fall));
            let loud_db = db_of(self.loudness);
            let voiced = self.band_loudness >= self.loudness * (1.0 - self.voice_share);
            let speaking = loud_db > self.idle_db && voiced;
            if speaking {
                self.held = self.hold;
            } else if self.held > 0 {
                self.held -= 1;
            }
            let wanted = match self.held > 0 {
                true => (self.target_db - loud_db).clamp(-self.down_db, self.up_db),
                false => 0.0,
            };
            self.ride_db = settled(self.ride_db + (wanted - self.ride_db) * self.towards);
            let gain = gain_of(self.ride_db) * self.output;
            frame[0] *= gain;
            frame[1] *= gain;
            self.gatherer.hear_change(&self.history, whole, frame[0].abs().max(frame[1].abs()), self.ride_db);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;

    fn tone(seconds: f32, db: f32) -> Vec<Frame> {
        let gain = gain_of(db);
        (0..(RATE * seconds) as usize)
            .map(|at| {
                let turn = at as f32 / RATE * 1000.0 * std::f32::consts::TAU;
                let sample = turn.sin() * gain;
                [sample, sample]
            })
            .collect()
    }

    fn loudest(audio: &[Frame]) -> f32 {
        db_of(audio.iter().fold(0.0f32, |most, frame| most.max(frame[0].abs())))
    }

    fn ridden(db: f32) -> f32 {
        let mut rider = Rider::new();
        rider.prepare(RATE);
        let mut audio = tone(3.0, db);
        rider.process(&mut audio);
        loudest(&audio[audio.len() - RATE as usize..])
    }

    #[test]
    fn a_quiet_voice_is_brought_up_to_the_target() {
        let settled = ridden(-30.0);
        assert!((settled - -18.0).abs() < 1.5, "a -30 dB tone should end near the -18 target, got {settled}");
    }

    #[test]
    fn a_loud_voice_is_brought_down_to_the_target() {
        let settled = ridden(-6.0);
        assert!((settled - -18.0).abs() < 1.5, "a -6 dB tone should end near the -18 target, got {settled}");
    }

    #[test]
    fn it_will_not_reach_further_than_it_is_allowed() {
        let mut rider = Rider::new();
        rider.set(REACH_UP, 6.0);
        rider.prepare(RATE);
        let mut audio = tone(3.0, -40.0);
        rider.process(&mut audio);
        let settled = loudest(&audio[audio.len() - RATE as usize..]);
        assert!(settled < -32.0, "6 dB of reach cannot lift a -40 dB tone to -18, got {settled}");
        assert!(settled > -35.0, "it should still use the 6 dB it has, got {settled}");
    }

    #[test]
    fn silence_is_left_alone() {
        let mut rider = Rider::new();
        rider.prepare(RATE);
        let mut audio = tone(2.0, -70.0);
        rider.process(&mut audio);
        let settled = loudest(&audio[audio.len() - RATE as usize..]);
        assert!((settled - -70.0).abs() < 0.5, "a tone below the idle level is not lifted, got {settled}");
    }
}
