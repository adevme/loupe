use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use crate::tune::KEYS;
use crate::{Effect, Frame, Param, DEFAULT_RATE};

const LISTEN_RATE: f32 = 11_025.0;
const BLOCK: usize = 4_096;
const LOWEST_NOTE: i32 = 45;
const HIGHEST_NOTE: i32 = 88;
const FRAMES_PER_SECOND: f32 = 100.0;
const KEPT_SECONDS: f32 = 30.0;
const TEMPO_AFTER_SECONDS: f32 = 8.0;
const BAR_BEATS: f32 = 4.0;
const BEAT_SPLITS: [f32; 4] = [2.0, 4.0, 8.0, 16.0];
const FAVOUR_SPREAD: f32 = 0.5;
const NEAR_TIE: f32 = 0.9;
const SLOWEST_BPM: f32 = 60.0;
const FASTEST_BPM: f32 = 200.0;
const FAVOURED_BPM: f32 = 100.0;
const LOW_BAND_HZ: f32 = 150.0;
const HIGH_BAND_HZ: f32 = 2_000.0;
const SMOOTH_HZ: f32 = 20.0;
const BAND_WEIGHTS: [f32; 3] = [1.0, 1.0, 1.0];
const SILENT: f32 = 1e-4;
const NO_KEY: u32 = 255;
const MAJOR: [f32; 12] = [0.238, 0.006, 0.111, 0.006, 0.137, 0.094, 0.016, 0.214, 0.009, 0.080, 0.008, 0.081];
const MINOR: [f32; 12] = [0.220, 0.006, 0.104, 0.123, 0.019, 0.103, 0.012, 0.214, 0.062, 0.022, 0.061, 0.052];

pub struct Findings {
    bpm: AtomicU32,
    key: AtomicU32,
    sure: AtomicU32,
    heard: AtomicU32,
    forget: AtomicBool,
}

impl Findings {
    pub fn new() -> Self {
        Self {
            bpm: AtomicU32::new(0f32.to_bits()),
            key: AtomicU32::new(NO_KEY),
            sure: AtomicU32::new(0f32.to_bits()),
            heard: AtomicU32::new(0f32.to_bits()),
            forget: AtomicBool::new(false),
        }
    }

    pub fn bpm(&self) -> Option<f32> {
        Some(f32::from_bits(self.bpm.load(Ordering::Relaxed))).filter(|bpm| *bpm > 0.0)
    }

    pub fn key(&self) -> Option<u8> {
        let key = self.key.load(Ordering::Relaxed);
        (key != NO_KEY).then_some(key as u8)
    }

    pub fn sure(&self) -> f32 {
        f32::from_bits(self.sure.load(Ordering::Relaxed))
    }

    pub fn heard_seconds(&self) -> f32 {
        f32::from_bits(self.heard.load(Ordering::Relaxed))
    }

    pub fn start_again(&self) {
        self.forget.store(true, Ordering::Relaxed);
    }

    fn tell(&self, heard: &Heard) {
        self.bpm.store(heard.bpm.unwrap_or(0.0).to_bits(), Ordering::Relaxed);
        self.key.store(heard.key.map_or(NO_KEY, u32::from), Ordering::Relaxed);
        self.sure.store(heard.sure.to_bits(), Ordering::Relaxed);
        self.heard.store(heard.seconds.to_bits(), Ordering::Relaxed);
    }
}

impl Default for Findings {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Heard {
    pub bpm: Option<f32>,
    pub key: Option<u8>,
    pub sure: f32,
    pub seconds: f32,
}

pub fn key_name(key: u8) -> String {
    let root = KEYS[(key % 12) as usize];
    if key < 12 {
        format!("{root} major")
    } else {
        format!("{root} minor")
    }
}

pub fn relative_of(key: u8) -> u8 {
    if key < 12 {
        12 + (key + 9) % 12
    } else {
        (key - 12 + 3) % 12
    }
}

pub struct Listener {
    squeeze: usize,
    rate: f32,
    sum: f32,
    count: usize,
    block: Vec<f32>,
    filled: usize,
    window: Vec<f32>,
    notes: Vec<(usize, f32)>,
    chroma: [f64; 12],
    frame_len: usize,
    frame_energy: [f32; 3],
    low: f32,
    high: f32,
    low_pull: f32,
    high_pull: f32,
    smooth_pull: f32,
    frame_count: usize,
    last_level: [f32; 3],
    onsets: Vec<f32>,
    onset_at: usize,
    onset_total: usize,
    since_tempo: usize,
    heard_samples: u64,
    heard: Heard,
    ordered: Vec<f32>,
    scores: Vec<f32>,
}

impl Listener {
    pub fn new(rate: f32) -> Self {
        let squeeze = (rate / LISTEN_RATE).round().max(1.0) as usize;
        let listen = rate / squeeze as f32;
        let window = (0..BLOCK).map(|at| 0.5 - 0.5 * (std::f32::consts::TAU * at as f32 / (BLOCK - 1) as f32).cos()).collect();
        let notes = (LOWEST_NOTE..=HIGHEST_NOTE)
            .filter_map(|note| {
                let hz = 440.0 * 2f32.powf((note - 69) as f32 / 12.0);
                (hz < listen / 2.0).then_some((note.rem_euclid(12) as usize, std::f32::consts::TAU * hz / listen))
            })
            .collect();
        let frame_len = (listen / FRAMES_PER_SECOND).round().max(1.0) as usize;
        let kept = (KEPT_SECONDS * listen / frame_len as f32) as usize;
        Self {
            squeeze,
            rate: listen,
            sum: 0.0,
            count: 0,
            block: vec![0.0; BLOCK],
            filled: 0,
            window,
            notes,
            chroma: [0.0; 12],
            frame_len,
            frame_energy: [0.0; 3],
            low: 0.0,
            high: 0.0,
            low_pull: 1.0 - (-std::f32::consts::TAU * LOW_BAND_HZ / listen).exp(),
            high_pull: 1.0 - (-std::f32::consts::TAU * HIGH_BAND_HZ / listen).exp(),
            smooth_pull: 1.0 - (-std::f32::consts::TAU * SMOOTH_HZ / listen).exp(),
            frame_count: 0,
            last_level: [0.0; 3],
            onsets: vec![0.0; kept],
            onset_at: 0,
            onset_total: 0,
            since_tempo: 0,
            heard_samples: 0,
            heard: Heard::default(),
            ordered: Vec::with_capacity(kept),
            scores: Vec::with_capacity(kept),
        }
    }

    pub fn heard(&self) -> Heard {
        self.heard
    }

    fn frames_per_second(&self) -> f32 {
        self.rate / self.frame_len as f32
    }

    pub fn hear(&mut self, frame: Frame) -> bool {
        self.sum += (frame[0] + frame[1]) * 0.5;
        self.count += 1;
        if self.count < self.squeeze {
            return false;
        }
        let sample = self.sum / self.squeeze as f32;
        self.sum = 0.0;
        self.count = 0;
        let mut changed = false;
        self.block[self.filled] = sample;
        self.filled += 1;
        if self.filled == BLOCK {
            self.filled = 0;
            changed |= self.read_block();
        }
        self.low += (sample - self.low) * self.low_pull;
        self.high += (sample - self.high) * self.high_pull;
        let bands = [self.low, self.high - self.low, sample - self.high];
        for (energy, band) in self.frame_energy.iter_mut().zip(bands) {
            *energy += (band * band - *energy) * self.smooth_pull;
        }
        self.frame_count += 1;
        if self.frame_count == self.frame_len {
            let mut onset = 0.0;
            for band in 0..3 {
                let level = (1.0 + 1000.0 * self.frame_energy[band]).ln();
                onset += (level - self.last_level[band]).max(0.0) * BAND_WEIGHTS[band];
                self.last_level[band] = level;
            }
            self.frame_count = 0;
            let kept = self.onsets.len();
            self.onsets[self.onset_at] = onset;
            self.onset_at = (self.onset_at + 1) % kept;
            self.onset_total += 1;
            self.since_tempo += 1;
            if self.since_tempo as f32 >= self.frames_per_second() {
                self.since_tempo = 0;
                changed |= self.read_tempo();
            }
        }
        changed
    }

    pub fn finish(&mut self) -> Heard {
        self.read_tempo();
        self.heard
    }

    fn read_block(&mut self) -> bool {
        let loud = self.block.iter().map(|sample| sample * sample).sum::<f32>() / BLOCK as f32;
        if loud < SILENT * SILENT {
            return false;
        }
        self.heard_samples += BLOCK as u64;
        self.heard.seconds = self.heard_samples as f32 / self.rate;
        let mut block_chroma = [0.0f64; 12];
        for (class, step) in &self.notes {
            let pull = 2.0 * step.cos();
            let (mut before, mut older) = (0.0f32, 0.0f32);
            for (sample, shape) in self.block.iter().zip(&self.window) {
                let now = sample * shape + pull * before - older;
                older = before;
                before = now;
            }
            let power = before * before + older * older - pull * before * older;
            block_chroma[*class] += power.max(0.0).sqrt() as f64;
        }
        let total: f64 = block_chroma.iter().sum();
        if total > 0.0 {
            for (kept, value) in self.chroma.iter_mut().zip(block_chroma) {
                *kept += value / total;
            }
        }
        self.read_key();
        true
    }

    fn read_key(&mut self) {
        let heard = self.chroma.map(|value| value as f32);
        let mut scores: [(u8, f32); 24] = std::array::from_fn(|key| {
            let profile = if key < 12 { &MAJOR } else { &MINOR };
            let root = key % 12;
            let shaped: [f32; 12] = std::array::from_fn(|class| profile[(class + 12 - root) % 12]);
            (key as u8, correlation(&heard, &shaped))
        });
        scores.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
        let (best, top) = scores[0];
        let runner = scores.iter().find(|(key, _)| *key != best && *key != relative_of(best)).map_or(0.0, |(_, score)| *score);
        if top.is_finite() && top > 0.0 {
            self.heard.key = Some(best);
            self.heard.sure = ((top - runner) * 4.0).clamp(0.0, 1.0) * (self.heard.seconds / 10.0).min(1.0);
        }
    }

    fn read_tempo(&mut self) -> bool {
        let fps = self.frames_per_second();
        let kept = self.onsets.len();
        let count = self.onset_total.min(kept);
        if (count as f32) < TEMPO_AFTER_SECONDS * fps {
            return false;
        }
        self.ordered.clear();
        self.ordered.extend((0..count).map(|back| self.onsets[(self.onset_at + kept - count + back) % kept]));
        let mean = self.ordered.iter().sum::<f32>() / count as f32;
        self.ordered.iter_mut().for_each(|value| *value -= mean);
        let centred = &self.ordered;
        let raw = |lag: usize| if lag == 0 || lag >= count { 0.0 } else { (0..count - lag).map(|at| centred[at] * centred[at + lag]).sum::<f32>() / (count - lag) as f32 };
        let shortest_bar = (BAR_BEATS * 60.0 * fps / FASTEST_BPM) as usize;
        let longest_bar = ((BAR_BEATS * 60.0 * fps / SLOWEST_BPM) as usize).min(count / 2);
        if longest_bar <= shortest_bar + 2 {
            return false;
        }
        self.scores.clear();
        self.scores.extend((0..=longest_bar + 1).map(|lag| if lag + 1 >= shortest_bar { raw(lag) + raw(lag * 2) } else { 0.0 }));
        let scores = &self.scores;
        let Some(strongest) = (shortest_bar..=longest_bar).map(|lag| scores[lag]).reduce(f32::max) else {
            return false;
        };
        if strongest <= 0.0 {
            return false;
        }
        let peak = |lag: usize| scores[lag] >= scores[lag - 1] && scores[lag] >= scores[lag + 1];
        let Some(bar) = (shortest_bar..=longest_bar).find(|lag| peak(*lag) && scores[*lag] >= NEAR_TIE * strongest) else {
            return false;
        };
        let (before, here, after) = (raw(bar - 1), raw(bar), raw(bar + 1));
        let bend = before - 2.0 * here + after;
        let exact = if bend < -1e-12 { bar as f32 + 0.5 * (before - after) / bend } else { bar as f32 };
        let favour = |bpm: f32| {
            let octaves = (bpm / FAVOURED_BPM).log2() / FAVOUR_SPREAD;
            (-0.5 * octaves * octaves).exp()
        };
        let Some(bpm) = BEAT_SPLITS
            .iter()
            .map(|split| 60.0 * fps * split / exact)
            .filter(|bpm| (SLOWEST_BPM..=FASTEST_BPM).contains(bpm))
            .max_by(|a, b| favour(*a).total_cmp(&favour(*b)))
        else {
            return false;
        };
        let bpm = if (bpm - bpm.round()).abs() < 0.3 { bpm.round() } else { (bpm * 10.0).round() / 10.0 };
        self.heard.bpm = Some(bpm);
        true
    }
}

fn correlation(a: &[f32; 12], b: &[f32; 12]) -> f32 {
    let mean_a = a.iter().sum::<f32>() / 12.0;
    let mean_b = b.iter().sum::<f32>() / 12.0;
    let (mut top, mut spread_a, mut spread_b) = (0.0, 0.0, 0.0);
    for at in 0..12 {
        let (x, y) = (a[at] - mean_a, b[at] - mean_b);
        top += x * y;
        spread_a += x * x;
        spread_b += y * y;
    }
    top / (spread_a * spread_b).sqrt().max(1e-12)
}

pub fn listen_to(audio: &[Frame], rate: f32) -> Heard {
    let mut listener = Listener::new(rate);
    for frame in audio {
        listener.hear(*frame);
    }
    listener.finish()
}

pub struct KeyListener {
    rate: f32,
    listener: Listener,
    findings: Arc<Findings>,
}

impl KeyListener {
    pub fn new() -> Self {
        Self { rate: DEFAULT_RATE, listener: Listener::new(DEFAULT_RATE), findings: Arc::new(Findings::new()) }
    }

    pub fn findings(&self) -> Arc<Findings> {
        self.findings.clone()
    }
}

impl Default for KeyListener {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for KeyListener {
    fn name(&self) -> &'static str {
        "Loupe Key"
    }

    fn params(&self) -> &'static [Param] {
        &[]
    }

    fn value(&self, _index: usize) -> f32 {
        0.0
    }

    fn set(&mut self, _index: usize, _value: f32) {}

    fn prepare(&mut self, rate: f32) {
        self.rate = rate;
        self.reset();
    }

    fn reset(&mut self) {
        self.listener = Listener::new(self.rate);
        self.findings.tell(&self.listener.heard());
    }

    fn findings(&self) -> Option<Arc<Findings>> {
        Some(self.findings())
    }

    fn process(&mut self, audio: &mut [Frame]) {
        if self.findings.forget.swap(false, Ordering::Relaxed) {
            self.reset();
        }
        let mut changed = false;
        for frame in audio.iter() {
            changed |= self.listener.hear(*frame);
        }
        if changed {
            self.findings.tell(&self.listener.heard());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 44_100.0;

    fn song(bpm: f32, chords: &[[i32; 3]], seconds: f32) -> Vec<Frame> {
        let beat = 60.0 / bpm;
        let total = (seconds * RATE) as usize;
        let mut noise = 1u32;
        (0..total)
            .map(|at| {
                let t = at as f32 / RATE;
                let in_beat = t % beat;
                let which = (t / beat) as usize % 4;
                let in_eighth = t % (beat / 2.0);
                let chord = chords[(t / (beat * 4.0)) as usize % chords.len()];
                let pad: f32 = chord.iter().map(|note| (std::f32::consts::TAU * 440.0 * 2f32.powf((*note - 69) as f32 / 12.0) * t).sin() * 0.12).sum();
                noise ^= noise << 13;
                noise ^= noise >> 17;
                noise ^= noise << 5;
                let white = noise as f32 / u32::MAX as f32 * 2.0 - 1.0;
                let kick = if which % 2 == 0 { 0.8 * (-in_beat * 25.0).exp() * (std::f32::consts::TAU * 55.0 * in_beat).sin() } else { 0.0 };
                let snare = if which % 2 == 1 { 0.5 * (-in_beat * 20.0).exp() * white } else { 0.0 };
                let hat = 0.12 * (-in_eighth * 80.0).exp() * white;
                let value = pad + kick + snare + hat;
                [value, value]
            })
            .collect()
    }

    #[test]
    fn a_minor_beat_at_92_is_heard_as_a_minor_at_92() {
        let chords = [[57, 60, 64], [50, 53, 57], [52, 56, 59], [57, 60, 64]];
        let heard = listen_to(&song(92.0, &chords, 24.0), RATE);
        assert_eq!(heard.key.map(key_name).as_deref(), Some("A minor"));
        let bpm = heard.bpm.unwrap();
        assert!((bpm - 92.0).abs() <= 1.0, "heard {bpm} BPM");
        assert!(heard.sure > 0.0);
    }

    #[test]
    fn a_major_beat_at_140_is_heard_in_its_key_and_tempo_or_half() {
        let chords = [[62, 66, 69], [67, 71, 74], [69, 73, 76], [62, 66, 69]];
        let heard = listen_to(&song(140.0, &chords, 24.0), RATE);
        assert_eq!(heard.key.map(key_name).as_deref(), Some("D major"));
        let bpm = heard.bpm.unwrap();
        assert!((bpm - 140.0).abs() <= 1.0 || (bpm - 70.0).abs() <= 0.5, "heard {bpm} BPM");
    }

    #[test]
    fn the_plugin_passes_sound_through_and_starts_again_when_asked() {
        let mut plugin = KeyListener::new();
        plugin.prepare(RATE);
        let chords = [[57, 60, 64], [53, 57, 60], [48, 52, 55], [55, 59, 62]];
        let mut audio = song(92.0, &chords, 12.0);
        let before = audio.clone();
        for block in audio.chunks_mut(512) {
            plugin.process(block);
        }
        assert_eq!(audio, before);
        let findings = plugin.findings();
        assert!(findings.key().is_some() && findings.bpm().is_some());
        assert!(findings.heard_seconds() > 10.0);
        findings.start_again();
        plugin.process(&mut [[0.0; 2]; 64]);
        assert_eq!((findings.key(), findings.bpm()), (None, None));
    }

    #[test]
    fn relatives_pair_up() {
        let a_minor = 12 + 9;
        assert_eq!(key_name(relative_of(a_minor)), "C major");
        assert_eq!(relative_of(relative_of(a_minor)), a_minor);
    }
}
