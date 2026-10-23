use std::sync::atomic::{AtomicBool, Ordering};

use crate::instrument::{Instrument, Note};
use crate::model::{Clip, ClipId, Frames, MeterPoint, Project, Track, TrackId, METER_POINTS};

pub type TrackPeaks = [f32; METER_POINTS];

pub struct Job<'a> {
    pub track: TrackId,
    pub audio: &'a mut [[f32; 2]],
    pub side: &'a [[f32; 2]],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PluginSpending {
    pub nanoseconds: u64,
    pub running: usize,
    pub held_back: usize,
    pub waves: u64,
    pub tracks_in_waves: u64,
    pub at_once: usize,
    pub late_replies: u64,
}

#[derive(Default)]
pub struct Saved {
    pub tracks: Vec<(TrackId, usize, Vec<u8>)>,
    pub clips: Vec<(ClipId, usize, Vec<u8>)>,
    pub master: Vec<(usize, Vec<u8>)>,
    pub monitor: Vec<(usize, Vec<u8>)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Song {
    pub at: Frames,
    pub tempo: f64,
    pub beats: u32,
    pub beat_unit: u32,
    pub playing: bool,
    pub recording: bool,
}

impl Song {
    pub fn rolling(project: &Project, at: Frames) -> Self {
        Self { at, tempo: project.bpm, beats: 4, beat_unit: 4, playing: true, recording: false }
    }

    pub fn where_it_is(project: &Project, at: Frames, playing: bool, recording: bool) -> Self {
        Self { at, tempo: project.bpm, beats: 4, beat_unit: 4, playing, recording }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Touched {
    Track(TrackId, usize),
    Clip(ClipId, usize),
    Master(usize),
    Monitor(usize),
}

pub trait Chains: Send {
    fn process(&mut self, track: TrackId, audio: &mut [[f32; 2]]) {
        self.process_with(track, audio, &[]);
    }

    fn process_with(&mut self, track: TrackId, audio: &mut [[f32; 2]], side: &[[f32; 2]]) {
        let _ = side;
        self.process(track, audio);
    }

    fn whole_waves_at_once(&self) -> bool {
        false
    }

    fn takes_its_time(&mut self, spare: bool) {
        let _ = spare;
    }

    fn process_wave(&mut self, jobs: &mut [Job<'_>]) {
        for job in jobs.iter_mut() {
            self.process_with(job.track, job.audio, job.side);
        }
    }

    fn process_master(&mut self, audio: &mut [[f32; 2]]) {
        let _ = audio;
    }

    fn process_takes(&mut self, track: TrackId, audio: &mut [[f32; 2]]) {
        let _ = (track, audio);
    }

    fn hear_with(&mut self, track: TrackId, audio: &mut [[f32; 2]], side: &[[f32; 2]]) {
        self.process_with(track, audio, side);
    }

    fn process_monitor(&mut self, audio: &mut [[f32; 2]]) {
        let _ = audio;
    }

    fn song_goes(&mut self, song: Song) {
        let _ = song;
    }

    fn monitor_chain_latency(&self) -> usize {
        0
    }

    fn master_latency(&self) -> usize {
        0
    }

    fn follow(&mut self, project: &Project) -> Vec<String> {
        let _ = project;
        Vec::new()
    }

    fn troubles(&self) -> Vec<(TrackId, String, String)> {
        Vec::new()
    }

    fn harvest(&mut self) -> Vec<(TrackId, usize, Vec<u8>)> {
        Vec::new()
    }

    fn harvest_master(&mut self) -> Vec<(usize, Vec<u8>)> {
        Vec::new()
    }

    fn harvest_monitor(&mut self) -> Vec<(usize, Vec<u8>)> {
        Vec::new()
    }

    fn everything_saved(&mut self) -> Saved {
        Saved { tracks: self.harvest(), clips: self.harvest_clips(), master: self.harvest_master(), monitor: self.harvest_monitor() }
    }

    fn knob_readings(&mut self, track: TrackId, slot: usize) -> Vec<(String, f32, String)> {
        let _ = (track, slot);
        Vec::new()
    }

    fn knob_from_text(&mut self, track: TrackId, slot: usize, knob: usize, text: &str) -> Result<f32, String> {
        let _ = (track, slot, knob, text);
        Err("plugins are not wired up".into())
    }

    fn turn_and_keep(&mut self, track: TrackId, slot: usize, knob: usize, value: f32) -> Option<Vec<u8>> {
        let _ = (track, slot, knob, value);
        None
    }

    fn show(&mut self, track: TrackId, slot: usize) -> Result<(), String> {
        let _ = (track, slot);
        Err("plugin windows are not wired up".into())
    }

    fn called(&mut self, track: TrackId, slot: usize, label: &str) -> Result<(), String> {
        let _ = (track, slot, label);
        Ok(())
    }

    fn on_track(&mut self, track: TrackId, slot: usize, name: &str, index: i64) -> Result<(), String> {
        let _ = (track, slot, name, index);
        Ok(())
    }

    fn plugins_touched(&mut self) -> Vec<Touched> {
        Vec::new()
    }

    fn state_of(&mut self, which: Touched) -> Option<Vec<u8>> {
        let _ = which;
        None
    }

    fn load(&mut self, track: TrackId, slot: usize) -> Result<(), String> {
        let _ = (track, slot);
        Err("plugins are not wired up".into())
    }

    fn load_clip(&mut self, clip: ClipId, slot: usize) -> Result<(), String> {
        let _ = (clip, slot);
        Err("plugins are not wired up".into())
    }

    fn load_master(&mut self, slot: usize) -> Result<(), String> {
        let _ = slot;
        Err("plugins are not wired up".into())
    }

    fn load_monitor(&mut self, slot: usize) -> Result<(), String> {
        let _ = slot;
        Err("plugins are not wired up".into())
    }

    fn tweak(&mut self, track: TrackId, slot: usize, knob: usize, value: f32) {
        let _ = (track, slot, knob, value);
    }

    fn latency(&self, track: TrackId) -> usize {
        let _ = track;
        0
    }

    fn takes_latency(&self, track: TrackId) -> usize {
        let _ = track;
        0
    }

    fn heard_latency(&self, track: TrackId) -> usize {
        self.latency(track)
    }

    fn heard_takes_latency(&self, track: TrackId) -> usize {
        self.takes_latency(track)
    }

    fn process_clip(&mut self, clip: ClipId, audio: &mut [[f32; 2]]) {
        let _ = (clip, audio);
    }

    fn process_clip_at(&mut self, clip: ClipId, audio: &mut [[f32; 2]], at: Frames) {
        let _ = at;
        self.process_clip(clip, audio);
    }

    fn clip_latency(&self, clip: ClipId) -> usize {
        let _ = clip;
        0
    }

    fn harvest_clips(&mut self) -> Vec<(ClipId, usize, Vec<u8>)> {
        Vec::new()
    }

    fn tweak_clip(&mut self, clip: ClipId, slot: usize, knob: usize, value: f32) {
        let _ = (clip, slot, knob, value);
    }

    fn automate(&mut self, track: TrackId, slot: usize, knob: usize, value: f32) {
        let _ = (track, slot, knob, value);
    }

    fn automate_clip(&mut self, clip: ClipId, slot: usize, knob: usize, value: f32) {
        let _ = (clip, slot, knob, value);
    }

    fn automate_master(&mut self, slot: usize, knob: usize, value: f32) {
        let _ = (slot, knob, value);
    }

    fn show_clip(&mut self, clip: ClipId, slot: usize) -> Result<(), String> {
        let _ = (clip, slot);
        Err("plugin windows are not wired up".into())
    }

    fn show_master(&mut self, slot: usize) -> Result<(), String> {
        let _ = slot;
        Err("plugin windows are not wired up".into())
    }

    fn show_monitor(&mut self, slot: usize) -> Result<(), String> {
        let _ = slot;
        Err("plugin windows are not wired up".into())
    }

    fn still_opening(&self) -> bool {
        false
    }

    fn plugin_spending(&self) -> PluginSpending {
        PluginSpending::default()
    }

    fn nudge(&mut self) {}
}

pub fn render(project: &Project, pos: Frames, out: &mut [[f32; 2]]) {
    mix_tracks(project, pos, out, None);
    let target = crate::envelope::Target::MasterGain;
    let opens = automated(project, target, pos, project.master);
    let closes = automated(project, target, pos + out.len() as Frames, project.master);
    let (opens, closes) = if project.master_muted { (0.0, 0.0) } else { (opens, closes) };
    scale(out, opens, closes);
}

pub fn render_through(
    project: &Project,
    pos: Frames,
    out: &mut [[f32; 2]],
    scratch: &mut Mixdown,
    chains: Option<&mut (dyn Chains + '_)>,
) {
    let mut after = chains;
    if let Some(racks) = after.as_deref_mut() {
        racks.song_goes(Song::rolling(project, pos));
    }
    mix_tracks_metered(project, pos, out, None, None, scratch, after.as_deref_mut());
    if let Some(chains) = after.as_deref_mut() {
        chains.process_master(out);
    }
    let target = crate::envelope::Target::MasterGain;
    let opens = automated(project, target, pos, project.master);
    let closes = automated(project, target, pos + out.len() as Frames, project.master);
    let (opens, closes) = if project.master_muted { (0.0, 0.0) } else { (opens, closes) };
    scale(out, opens, closes);
}

pub fn scale(out: &mut [[f32; 2]], from: f32, to: f32) {
    if from == 1.0 && to == 1.0 {
        return;
    }
    let step = (to - from) / out.len().max(1) as f32;
    for (i, frame) in out.iter_mut().enumerate() {
        let level = from + step * (i + 1) as f32;
        frame[0] *= level;
        frame[1] *= level;
    }
}

pub(crate) type Shape = (TrackId, Option<TrackId>, u64);

pub type Round = Option<(Frames, Frames)>;

#[derive(Default)]
struct Line {
    delay: usize,
    held: Vec<[f32; 2]>,
    at: usize,
}

impl Line {
    fn settle(&mut self) {
        self.held.clear();
        self.held.resize(self.delay, [0.0; 2]);
        self.at = 0;
    }

    fn step(&mut self, frame: [f32; 2]) -> [f32; 2] {
        if self.held.is_empty() {
            return frame;
        }
        let out = self.held[self.at];
        self.held[self.at] = frame;
        self.at += 1;
        if self.at == self.held.len() {
            self.at = 0;
        }
        out
    }
}

#[derive(Default)]
pub struct Mixdown {
    buffers: Vec<Vec<[f32; 2]>>,
    sides: Vec<Vec<[f32; 2]>>,
    apart: Vec<[f32; 2]>,
    pre: Vec<[f32; 2]>,
    sink: Vec<[f32; 2]>,
    next: Option<Frames>,
    order: Vec<TrackId>,
    index: Vec<usize>,
    shape: Vec<Shape>,
    lags: Vec<usize>,
    ahead: Vec<usize>,
    ahead_was: Vec<usize>,
    lines: Vec<Line>,
    feed: Vec<usize>,
    waves: Vec<(usize, usize)>,
    depth: Vec<usize>,
    wanted: Vec<usize>,
    ordered: bool,
    round: Round,
    rounded: Round,
    hurry: bool,
}

pub(crate) fn shape_of(track: &Track) -> Shape {
    let sends = track.sends.iter().fold(0u64, |so_far, send| so_far.rotate_left(7) ^ send.to.0);
    (track.id, track.parent, sends)
}

fn own_lag(track: &Track, racks: Option<&dyn Chains>) -> usize {
    match racks {
        Some(racks) if !track.fx.is_empty() => racks.latency(track.id),
        _ => 0,
    }
}

impl Mixdown {
    fn same_shape(&self, project: &Project, racks: Option<&dyn Chains>) -> bool {
        self.shape.len() == project.tracks.len()
            && self.lags.len() == project.tracks.len()
            && project.tracks.iter().zip(&self.shape).all(|(track, known)| shape_of(track) == *known)
            && (racks.is_none() || project.tracks.iter().zip(&self.lags).all(|(track, known)| own_lag(track, racks) == *known))
    }

    fn carried_on(&self, pos: Frames) -> bool {
        match self.next {
            Some(next) if next == pos => true,
            Some(next) => self.round == self.rounded && self.round.is_some_and(|(from, to)| next == to && pos == from),
            None => false,
        }
    }

    fn rewired(&self, project: &Project) -> bool {
        self.shape.len() != project.tracks.len() || project.tracks.iter().zip(&self.shape).any(|(track, known)| shape_of(track) != *known)
    }

    fn work_out_order(&mut self, project: &Project, racks: Option<&dyn Chains>) {
        let rewired = self.rewired(project);
        self.shape.clear();
        self.shape.extend(project.tracks.iter().map(shape_of));
        if racks.is_some() || self.lags.len() != project.tracks.len() {
            self.lags.clear();
            self.lags.extend(project.tracks.iter().map(|track| own_lag(track, racks)));
        }
        self.order.clear();
        self.index.clear();
        self.ordered = match project.render_order() {
            Some(order) => {
                self.order.extend(order);
                for id in &self.order {
                    self.index.push(project.tracks.iter().position(|t| t.id == *id).unwrap_or(usize::MAX));
                }
                self.work_out_waves(project);
                if self.work_out_delays(project, rewired) {
                    self.next = None;
                }
                true
            }
            None => {
                self.next = None;
                false
            }
        };
    }

    fn work_out_delays(&mut self, project: &Project, rewired: bool) -> bool {
        let count = project.tracks.len();
        std::mem::swap(&mut self.ahead, &mut self.ahead_was);
        self.ahead.clear();
        self.ahead.resize(count, 0);
        self.feed.clear();
        self.feed.resize(count, 0);
        let mut next = 0;
        for (index, track) in project.tracks.iter().enumerate() {
            self.feed[index] = next;
            next += 1 + track.sends.len();
        }
        self.lines.resize_with(next, Line::default);
        if rewired {
            for line in self.lines.iter_mut() {
                line.delay = 0;
            }
        }
        for step in (0..self.order.len()).rev() {
            let index = self.index[step];
            if index == usize::MAX {
                continue;
            }
            let track = &project.tracks[index];
            let down = |id: TrackId| project.tracks.iter().position(|t| t.id == id);
            let up = track.parent.and_then(down).map(|parent| self.ahead[parent]).unwrap_or(0);
            let mut wants = up;
            for send in &track.sends {
                if let Some(target) = down(send.to) {
                    wants = wants.max(self.ahead[target]);
                }
            }
            self.ahead[index] = wants + self.lags[index];
            let first = self.feed[index];
            self.lines[first].delay = wants - up;
            for (which, send) in track.sends.iter().enumerate() {
                self.lines[first + 1 + which].delay = match down(send.to) {
                    Some(target) => wants - self.ahead[target],
                    None => 0,
                };
            }
        }
        let mut moved = rewired || self.ahead != self.ahead_was;
        for line in self.lines.iter_mut() {
            if rewired || line.held.len() != line.delay {
                line.settle();
                moved = true;
            }
        }
        moved
    }

    fn work_out_waves(&mut self, project: &Project) {
        self.depth.clear();
        self.depth.resize(self.order.len(), 0);
        self.waves.clear();
        for step in 0..self.order.len() {
            let Some(track) = project.tracks.iter().find(|t| t.id == self.order[step]) else { continue };
            let next = self.depth[step] + 1;
            for fed in track.parent.into_iter().chain(track.sends.iter().map(|send| send.to)) {
                if let Some(at) = self.order.iter().position(|id| *id == fed) {
                    self.depth[at] = self.depth[at].max(next);
                }
            }
        }
        let mut from = 0;
        for step in 1..=self.order.len() {
            if step == self.order.len() || self.depth[step] != self.depth[from] {
                self.waves.push((from, step));
                from = step;
            }
        }
    }
}

impl Mixdown {
    pub fn room_for(&mut self, tracks: usize, frames: usize) {
        if self.buffers.len() < tracks {
            self.buffers.resize_with(tracks, Vec::new);
        }
        if self.sides.len() < tracks {
            self.sides.resize_with(tracks, Vec::new);
        }
        for buffer in self.buffers.iter_mut().take(tracks).chain(self.sides.iter_mut().take(tracks)) {
            if buffer.len() < frames {
                buffer.resize(frames, [0.0; 2]);
            }
        }
        if self.pre.len() < frames {
            self.pre.resize(frames, [0.0; 2]);
        }
        if self.sink.len() < frames {
            self.sink.resize(frames, [0.0; 2]);
        }
    }

    pub fn hurries(&mut self) {
        self.hurry = true;
    }

    pub fn ready_for(&mut self, project: &Project, pos: Frames, round: Round, racks: Option<&dyn Chains>) -> usize {
        self.round = round;
        if !self.same_shape(project, racks) {
            self.work_out_order(project, racks);
        }
        if !self.ordered || self.carried_on(pos) {
            return 0;
        }
        self.ahead.iter().copied().max().unwrap_or(0)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn get_ready(
        &mut self,
        project: &Project,
        pos: Frames,
        round: Round,
        only: Option<ClipId>,
        block: usize,
        racks: Option<&mut (dyn Chains + '_)>,
        stop: Option<&AtomicBool>,
    ) {
        let len = block.max(1);
        self.round = round;
        self.room_for(project.tracks.len(), len);
        if !self.same_shape(project, racks.as_deref()) {
            self.work_out_order(project, racks.as_deref());
        }
        if !self.ordered {
            return;
        }
        let hurried = std::mem::replace(&mut self.hurry, false);
        let all_of_it = prime(project, pos, len, only, self, racks, stop);
        self.hurry = hurried;
        if !all_of_it {
            self.next = None;
            return;
        }
        self.next = Some(pos);
        self.rounded = round;
    }

    pub fn tap(&self, track: usize, frames: usize) -> Option<&[[f32; 2]]> {
        if !self.ordered {
            return None;
        }
        self.buffers.get(track).filter(|buffer| buffer.len() >= frames).map(|buffer| &buffer[..frames])
    }
}

pub fn mix_tracks(project: &Project, pos: Frames, out: &mut [[f32; 2]], only: Option<ClipId>) {
    let mut scratch = Mixdown::default();
    mix_tracks_metered(project, pos, out, only, None, &mut scratch, None);
}

pub fn mix_tracks_metered(
    project: &Project,
    pos: Frames,
    out: &mut [[f32; 2]],
    only: Option<ClipId>,
    peaks: Option<&mut [TrackPeaks]>,
    scratch: &mut Mixdown,
    mut chains: Option<&mut (dyn Chains + '_)>,
) {
    let len = out.len();
    let count = project.tracks.len();
    if let Some(racks) = chains.as_deref_mut() {
        if !project.envelopes.is_empty() {
            turn_knobs(project, pos, racks);
        }
    }
    scratch.room_for(count, len);
    if !scratch.same_shape(project, chains.as_deref()) {
        scratch.work_out_order(project, chains.as_deref());
    }
    if !scratch.ordered {
        out.fill([0.0; 2]);
        return;
    }
    if len > 0 && !scratch.carried_on(pos) {
        prime(project, pos, len, only, scratch, chains.as_deref_mut(), None);
    }
    run(project, pos as i64, out, only, peaks, scratch, chains);
    scratch.next = Some(pos + len as Frames);
    scratch.rounded = scratch.round;
}

fn prime(
    project: &Project,
    pos: Frames,
    len: usize,
    only: Option<ClipId>,
    scratch: &mut Mixdown,
    mut chains: Option<&mut (dyn Chains + '_)>,
    stop: Option<&AtomicBool>,
) -> bool {
    let wanted = scratch.ahead.iter().copied().max().unwrap_or(0);
    let mut left = match scratch.hurry {
        true => wanted.min(len),
        false => wanted,
    };
    while left > 0 {
        if stop.is_some_and(|asked| asked.load(Ordering::Relaxed)) {
            return false;
        }
        let part = len.min(left);
        let at = pos as i64 - left as i64;
        let mut sink = std::mem::take(&mut scratch.sink);
        run(project, at, &mut sink[..part], only, None, scratch, chains.as_deref_mut());
        scratch.sink = sink;
        left -= part;
    }
    true
}

fn run(
    project: &Project,
    at: i64,
    out: &mut [[f32; 2]],
    only: Option<ClipId>,
    mut peaks: Option<&mut [TrackPeaks]>,
    scratch: &mut Mixdown,
    mut chains: Option<&mut (dyn Chains + '_)>,
) {
    out.fill([0.0; 2]);
    let len = out.len();
    let pos = at.max(0) as Frames;
    let soloing = only.is_none() && project.any_solo();
    for (index, track) in project.tracks.iter().enumerate() {
        scratch.sides[index][..len].fill([0.0; 2]);
        let buffer = &mut scratch.buffers[index][..len];
        buffer.fill([0.0; 2]);
        if soloing && !project.heard_in_solo(track.id) {
            continue;
        }
        let read = at + scratch.ahead[index] as i64;
        lay_reading(project, track, read, buffer, scratch.round, only, chains.as_deref_mut(), &mut scratch.apart);
    }
    let in_waves = chains.as_deref().is_some_and(|racks| racks.whole_waves_at_once());
    for wave in 0..scratch.waves.len() {
        let (wave_from, wave_to) = scratch.waves[wave];
        if in_waves {
            note_peaks_at(MeterPoint::PreFx, peaks.as_deref_mut(), scratch, wave_from, wave_to, len);
            if let Some(racks) = chains.as_deref_mut() {
                run_a_wave(project, scratch, racks, wave_from, wave_to, len);
            }
        }
        for step in wave_from..wave_to {
            let index = scratch.index[step];
            if index == usize::MAX {
                continue;
            }
            let track = &project.tracks[index];
            if !in_waves {
                note_peaks_at(MeterPoint::PreFx, peaks.as_deref_mut(), scratch, step, step + 1, len);
                if let Some(racks) = chains.as_deref_mut() {
                    if !track.fx.is_empty() {
                        let (mains, sides) = (&mut scratch.buffers, &scratch.sides);
                        racks.process_with(track.id, &mut mains[index][..len], &sides[index][..len]);
                    }
                }
            }
            note_peaks_at(MeterPoint::PostFx, peaks.as_deref_mut(), scratch, step, step + 1, len);
            let silent = track.muted && only.is_none();
            let keep_pre = track.sends.iter().any(|send| send.pre_fader);
            if keep_pre {
                scratch.pre[..len].copy_from_slice(&scratch.buffers[index][..len]);
            }
            let target = crate::envelope::Target::TrackGain(track.id);
            let opens = automated(project, target, pos, track.gain);
            let closes = automated(project, target, pos + len as Frames, track.gain);
            let (opens, closes) = if silent { (0.0, 0.0) } else { (opens, closes) };
            let ramp = (closes - opens) / len.max(1) as f32;
            let pan_target = crate::envelope::Target::TrackPan(track.id);
            let pan_opens = automated(project, pan_target, pos, track.pan);
            let pan_closes = automated(project, pan_target, pos + len as Frames, track.pan);
            let pan_ramp = (pan_closes - pan_opens) / len.max(1) as f32;
            let (mut left, mut right) = pan_gains(pan_opens);
            for (i, frame) in scratch.buffers[index][..len].iter_mut().enumerate() {
                if pan_ramp != 0.0 {
                    (left, right) = pan_gains(pan_opens + pan_ramp * (i + 1) as f32);
                }
                let gain = opens + ramp * (i + 1) as f32;
                frame[0] *= gain * left;
                frame[1] *= gain * right;
            }
            note_peaks_at(MeterPoint::PostFader, peaks.as_deref_mut(), scratch, step, step + 1, len);
            let first = scratch.feed[index];
            for (which, send) in track.sends.iter().enumerate() {
                let Some(target) = project.tracks.iter().position(|t| t.id == send.to) else {
                    continue;
                };
                let send_target = crate::envelope::Target::SendGain { from: track.id, to: send.to };
                let send_opens = automated(project, send_target, pos, send.gain);
                let send_closes = automated(project, send_target, pos + len as Frames, send.gain);
                let send_ramp = (send_closes - send_opens) / len.max(1) as f32;
                let Mixdown { buffers, sides, pre, lines, .. } = &mut *scratch;
                let line = &mut lines[first + 1 + which];
                for i in 0..len {
                    let from = if send.pre_fader { pre[i] } else { buffers[index][i] };
                    let carried = line.step(from);
                    let gain = send_opens + send_ramp * (i + 1) as f32;
                    let dest = if send.sidechain { &mut sides[target][i] } else { &mut buffers[target][i] };
                    dest[0] += carried[0] * gain;
                    dest[1] += carried[1] * gain;
                }
            }
            match track.parent.and_then(|parent| project.tracks.iter().position(|t| t.id == parent)) {
                Some(target) => {
                    let Mixdown { buffers, lines, .. } = &mut *scratch;
                    let line = &mut lines[first];
                    for i in 0..len {
                        let carried = line.step(buffers[index][i]);
                        let dest = &mut buffers[target][i];
                        dest[0] += carried[0];
                        dest[1] += carried[1];
                    }
                }
                None => {
                    let Mixdown { buffers, lines, .. } = &mut *scratch;
                    let line = &mut lines[first];
                    for (i, dest) in out.iter_mut().enumerate().take(len) {
                        let carried = line.step(buffers[index][i]);
                        dest[0] += carried[0];
                        dest[1] += carried[1];
                    }
                }
            }
        }
    }
}

fn note_peaks_at(point: MeterPoint, peaks: Option<&mut [TrackPeaks]>, scratch: &Mixdown, from: usize, to: usize, len: usize) {
    let Some(peaks) = peaks else {
        return;
    };
    for step in from..to {
        let index = scratch.index[step];
        let Some(slot) = peaks.get_mut(index) else {
            continue;
        };
        slot[point.index()] = loudest(&scratch.buffers[index][..len]);
    }
}

pub(crate) fn loudest(frames: &[[f32; 2]]) -> f32 {
    frames.iter().fold(0.0f32, |top, frame| top.max(frame[0].abs()).max(frame[1].abs()))
}

fn run_a_wave(
    project: &Project,
    scratch: &mut Mixdown,
    racks: &mut (dyn Chains + '_),
    wave_from: usize,
    wave_to: usize,
    len: usize,
) {
    scratch.wanted.clear();
    for step in wave_from..wave_to {
        let index = scratch.index[step];
        if index != usize::MAX && !project.tracks[index].fx.is_empty() {
            scratch.wanted.push(index);
        }
    }
    if scratch.wanted.is_empty() {
        return;
    }
    let Mixdown { buffers, sides, wanted, .. } = scratch;
    let mut jobs: Vec<Job<'_>> = Vec::with_capacity(wanted.len());
    for (index, buffer) in buffers.iter_mut().enumerate() {
        if !wanted.contains(&index) {
            continue;
        }
        jobs.push(Job { track: project.tracks[index].id, audio: &mut buffer[..len], side: &sides[index][..len] });
    }
    racks.process_wave(&mut jobs);
}

pub fn pan_gains(pan: f32) -> (f32, f32) {
    let pan = pan.clamp(-1.0, 1.0);
    let left = (pan.max(0.0) * std::f32::consts::FRAC_PI_2).cos();
    let right = ((-pan).max(0.0) * std::f32::consts::FRAC_PI_2).cos();
    (left, right)
}

#[allow(clippy::too_many_arguments)]
fn lay_reading(
    project: &Project,
    track: &Track,
    read: i64,
    buffer: &mut [[f32; 2]],
    round: Round,
    only: Option<ClipId>,
    mut chains: Option<&mut (dyn Chains + '_)>,
    apart: &mut Vec<[f32; 2]>,
) {
    let len = buffer.len();
    if let Some((from, to)) = round.filter(|(from, to)| to > from) {
        let span = (to - from) as i64;
        let mut at = from as i64 + (read - from as i64).rem_euclid(span);
        let mut done = 0;
        while done < len {
            let part = (len - done).min((to as i64 - at) as usize);
            lay_clips(project, track, at as Frames, &mut buffer[done..done + part], only, chains.as_deref_mut(), apart);
            done += part;
            at = from as i64;
        }
        return;
    }
    if read + len as i64 <= 0 {
        return;
    }
    let (begin, into) = match read < 0 {
        true => (0, (-read) as usize),
        false => (read as Frames, 0),
    };
    lay_clips(project, track, begin, &mut buffer[into..], only, chains, apart);
}

const CLIP_CHAIN_TAIL_SECONDS: Frames = 30;

fn lay_clips(
    project: &Project,
    track: &Track,
    pos: Frames,
    out: &mut [[f32; 2]],
    only: Option<ClipId>,
    mut chains: Option<&mut (dyn Chains + '_)>,
    apart: &mut Vec<[f32; 2]>,
) {
    let rate = project.rate;
    for clip in &track.clips {
        let ahead = match chains.as_deref() {
            Some(racks) if !clip.fx.is_empty() => racks.clip_latency(clip.id) as Frames,
            _ => 0,
        };
        let pos = pos + ahead;
        let end = pos + out.len() as Frames;
        let silenced = match only {
            Some(chosen) => clip.id != chosen || clip.muted,
            None => clip.muted,
        };
        let own = !clip.fx.is_empty() && chains.is_some();
        let rings_until = match own {
            true => clip.end() + CLIP_CHAIN_TAIL_SECONDS * rate as Frames,
            false => clip.end(),
        };
        if silenced || rings_until <= pos || clip.start >= end {
            continue;
        }
        let from = clip.start.max(pos);
        let to = clip.end().min(end).max(from);
        let source = clip.audio();
        let source_from = ((clip.offset + (from - clip.start)) as usize).min(source.len());
        let count = match clip.notes {
            Some(_) => (to - from) as usize,
            None => ((to - from) as usize).min(source.len() - source_from),
        };
        let gain = clip_automated(project, crate::envelope::Target::ClipGain(clip.id), pos, clip.start, clip.gain);
        if own {
            apart.clear();
            apart.resize(out.len(), [0.0; 2]);
        }
        let target = match own {
            true => &mut apart[(from - pos) as usize..][..count],
            false => &mut out[(from - pos) as usize..][..count],
        };
        let first = from - clip.start;
        if let Some(notes) = &clip.notes {
            play_notes(track.instrument, track.sample.as_deref().map(|sample| &sample.frames[..]), notes, clip, first, target, rate);
        } else {
        let audio = &source[source_from..][..count];
        let fade_in_frames = (clip.fade_in.len.saturating_sub(first) as usize).min(count);
        let steady_end = clip.len - clip.fade_out.len;
        let fade_out_from = (steady_end.saturating_sub(first) as usize).clamp(fade_in_frames, count);
        mix_faded(&mut target[..fade_in_frames], &audio[..fade_in_frames], gain, clip, first);
        mix(&mut target[fade_in_frames..fade_out_from], &audio[fade_in_frames..fade_out_from], gain);
        mix_faded(
            &mut target[fade_out_from..],
            &audio[fade_out_from..],
            gain,
            clip,
            first + fade_out_from as Frames,
        );
        }
        if own {
            if let Some(racks) = chains.as_deref_mut() {
                racks.process_clip_at(clip.id, apart, pos);
            }
            for (into, from) in out.iter_mut().zip(apart.iter()) {
                into[0] += from[0];
                into[1] += from[1];
            }
        }
    }
}

fn play_notes(instrument: Instrument, sample: Option<&[[f32; 2]]>, notes: &[Note], clip: &Clip, first: Frames, target: &mut [[f32; 2]], rate: u32) {
    let window_from = clip.offset + first;
    let window_to = window_from + target.len() as Frames;
    let content_end = clip.offset + clip.len;
    for note in notes {
        if note.start >= window_to {
            break;
        }
        let tail = instrument.tail_for(note, rate, sample);
        if note.start >= content_end || note.end() + tail <= window_from {
            continue;
        }
        let from = note.start.max(window_from);
        let to = (note.end() + tail).min(window_to);
        if to <= from {
            continue;
        }
        let into = (from - window_from) as usize;
        let span = &mut target[into..(to - window_from) as usize];
        let level = |i: usize| clip.gain * clip.fade_level(first + (into + i) as Frames);
        instrument.play_with(sample, note, from - note.start, span, level, rate);
    }
}

fn mix(target: &mut [[f32; 2]], audio: &[[f32; 2]], gain: f32) -> f32 {
    let mut top = 0.0f32;
    for (o, s) in target.iter_mut().zip(audio) {
        let (l, r) = (s[0] * gain, s[1] * gain);
        o[0] += l;
        o[1] += r;
        top = top.max(l.abs()).max(r.abs());
    }
    top
}

fn mix_faded(target: &mut [[f32; 2]], audio: &[[f32; 2]], gain: f32, clip: &Clip, first: Frames) -> f32 {
    let mut top = 0.0f32;
    for (i, (o, s)) in target.iter_mut().zip(audio).enumerate() {
        let level = gain * clip.fade_level(first + i as Frames);
        let (l, r) = (s[0] * level, s[1] * level);
        o[0] += l;
        o[1] += r;
        top = top.max(l.abs()).max(r.abs());
    }
    top
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Command, Edge, Fade, Outcome, TrackId};
    use crate::source::Source;
    use std::sync::Arc;

    fn counting(len: usize) -> Arc<Source> {
        Arc::new(Source::from_frames("count", (0..len).map(|i| [i as f32, -(i as f32)]).collect()))
    }

    fn track(p: &mut Project) -> TrackId {
        match p.apply(Command::AddTrack { name: "T".into() }) {
            Ok(Outcome::Track(id)) => id,
            other => panic!("{other:?}"),
        }
    }

    struct Doubles {
        in_waves: bool,
    }

    impl Chains for Doubles {
        fn process(&mut self, _track: TrackId, audio: &mut [[f32; 2]]) {
            for frame in audio.iter_mut() {
                frame[0] *= 2.0;
                frame[1] *= 2.0;
            }
        }

        fn whole_waves_at_once(&self) -> bool {
            self.in_waves
        }
    }

    #[test]
    fn each_meter_point_reads_its_own_place_in_the_track() {
        for in_waves in [false, true] {
            let mut p = Project::new(48_000);
            let one = track(&mut p);
            clip(&mut p, one, Arc::new(Source::from_frames("steady", vec![[0.25, -0.25]; 4_000])), 0);
            let fx = crate::model::Fx { mix: 1.0, path: std::path::PathBuf::from("x.vst3"), index: 0, name: "x".into(), bypassed: false, state: Vec::new(), record: false };
            p.apply(Command::AddFx { track: one, fx }).unwrap();
            p.apply(Command::SetTrackGain { track: one, gain: 0.25 }).unwrap();
            let mut peaks = [[0.0f32; METER_POINTS]; 2];
            let mut out = vec![[0.0; 2]; 256];
            mix_tracks_metered(&p, 0, &mut out, None, Some(&mut peaks), &mut Mixdown::default(), Some(&mut Doubles { in_waves }));
            let read = |point: MeterPoint| peaks[0][point.index()];
            assert!((read(MeterPoint::PreFx) - 0.25).abs() < 1e-6, "pre-FX read {} (waves {in_waves})", read(MeterPoint::PreFx));
            assert!((read(MeterPoint::PostFx) - 0.5).abs() < 1e-6, "post-FX read {} (waves {in_waves})", read(MeterPoint::PostFx));
            assert!((read(MeterPoint::PostFader) - 0.125).abs() < 1e-6, "post-fader read {} (waves {in_waves})", read(MeterPoint::PostFader));
            assert!((out[100][0] - 0.125).abs() < 1e-6, "the sound changed: {:?}", out[100]);
        }
    }

    fn clip(p: &mut Project, track: TrackId, source: Arc<Source>, start: Frames) -> ClipId {
        match p.apply(Command::AddClip { track, source, start }) {
            Ok(Outcome::Clip(id)) => id,
            other => panic!("{other:?}"),
        }
    }

    fn whole(p: &Project, frames: usize) -> Vec<[f32; 2]> {
        let mut out = vec![[9.0; 2]; frames];
        render(p, 0, &mut out);
        out
    }

    #[test]
    fn a_clip_lands_on_its_exact_frame() {
        let mut p = Project::new(48_000);
        let t = track(&mut p);
        clip(&mut p, t, counting(100), 50);
        let out = whole(&p, 200);
        assert_eq!(out[49], [0.0, 0.0]);
        assert_eq!(out[50], [0.0, 0.0]);
        assert_eq!(out[51], [1.0, -1.0]);
        assert_eq!(out[149], [99.0, -99.0]);
        assert_eq!(out[150], [0.0, 0.0]);
    }

    #[test]
    fn a_track_envelope_moves_the_level_while_it_plays() {
        use crate::envelope::{Point, Shape, Target};
        let mut p = Project::new(48_000);
        let one = track(&mut p);
        flat(&mut p, one, 1.0, 400);
        let target = Target::TrackGain(one);
        p.apply(Command::AddEnvelope { target }).unwrap();
        p.apply(Command::ClearPoints { target, from: 0, to: 1_000_000 }).unwrap();
        p.apply(Command::PutPoint { target, point: Point { at: 0, value: 0.0, shape: Shape::Linear } }).unwrap();
        p.apply(Command::PutPoint { target, point: Point { at: 100, value: 1.0, shape: Shape::Linear } }).unwrap();
        let mut out = vec![[0.0; 2]; 8];
        let mut scratch = Mixdown::default();
        mix_tracks_metered(&p, 0, &mut out, None, None, &mut scratch, None);
        let quiet = out[4][0];
        mix_tracks_metered(&p, 50, &mut out, None, None, &mut scratch, None);
        let middling = out[4][0];
        mix_tracks_metered(&p, 150, &mut out, None, None, &mut scratch, None);
        let loud = out[4][0];
        assert!(quiet < 0.05, "it started at {quiet}");
        assert!((middling - 0.5).abs() < 0.1, "halfway it read {middling}");
        assert!((loud - 1.0).abs() < 0.01, "at the end it read {loud}");
    }

    #[test]
    fn a_pan_envelope_sweeps_the_track_across() {
        use crate::envelope::{Point, Shape, Target};
        let mut p = Project::new(48_000);
        let one = track(&mut p);
        flat(&mut p, one, 1.0, 400);
        let target = Target::TrackPan(one);
        p.apply(Command::AddEnvelope { target }).unwrap();
        p.apply(Command::ClearPoints { target, from: 0, to: Frames::MAX }).unwrap();
        p.apply(Command::PutPoint { target, point: Point { at: 0, value: -1.0, shape: Shape::Linear } }).unwrap();
        p.apply(Command::PutPoint { target, point: Point { at: 200, value: 1.0, shape: Shape::Linear } }).unwrap();
        let mut out = vec![[0.0; 2]; 8];
        let mut scratch = Mixdown::default();
        mix_tracks_metered(&p, 1, &mut out, None, None, &mut scratch, None);
        assert!(out[4][0] > 0.9 && out[4][1] < 0.15, "it should start nearly hard left, got {:?}", out[4]);
        mix_tracks_metered(&p, 199, &mut out, None, None, &mut scratch, None);
        assert!(out[4][1] > 0.9 && out[4][0] < 0.05, "it should end hard right, got {:?}", out[4]);
    }

    struct Knobs {
        seen: Vec<(usize, f32)>,
    }

    impl Chains for Knobs {
        fn process(&mut self, _track: TrackId, _audio: &mut [[f32; 2]]) {}

        fn automate(&mut self, _track: TrackId, _slot: usize, knob: usize, value: f32) {
            self.seen.push((knob, value));
        }
    }

    #[test]
    fn a_plugin_knob_envelope_reaches_the_plugin() {
        use crate::envelope::{Point, Shape, Target};
        let mut p = Project::new(48_000);
        let one = track(&mut p);
        let fx = crate::model::Fx {
            mix: 1.0,
            path: std::path::PathBuf::from("x.vst3"),
            index: 0,
            name: "x".into(),
            bypassed: false,
            state: Vec::new(),
            record: false,
        };
        p.apply(Command::AddFx { track: one, fx }).unwrap();
        let target = Target::TrackFx { track: one, slot: 0, knob: 2 };
        p.apply(Command::AddEnvelope { target }).unwrap();
        p.apply(Command::ClearPoints { target, from: 0, to: Frames::MAX }).unwrap();
        p.apply(Command::PutPoint { target, point: Point { at: 0, value: 0.0, shape: Shape::Linear } }).unwrap();
        p.apply(Command::PutPoint { target, point: Point { at: 1000, value: 1.0, shape: Shape::Linear } }).unwrap();
        let mut racks = Knobs { seen: Vec::new() };
        let mut out = vec![[0.0; 2]; 16];
        let mut scratch = Mixdown::default();
        mix_tracks_metered(&p, 500, &mut out, None, None, &mut scratch, Some(&mut racks));
        assert_eq!(racks.seen.len(), 1, "the knob was not turned");
        assert_eq!(racks.seen[0].0, 2, "the wrong knob moved");
        assert!((racks.seen[0].1 - 0.5).abs() < 1e-6, "it was turned to {}", racks.seen[0].1);
    }

    #[test]
    fn a_clip_envelope_travels_with_the_clip() {
        use crate::envelope::{Point, Shape, Target};
        let mut p = Project::new(48_000);
        let one = track(&mut p);
        let moved = clip(&mut p, one, counting(400), 200);
        let target = Target::ClipGain(moved);
        p.apply(Command::AddEnvelope { target }).unwrap();
        p.apply(Command::ClearPoints { target, from: 0, to: 1_000_000 }).unwrap();
        p.apply(Command::PutPoint { target, point: Point { at: 0, value: 0.0, shape: Shape::Hold } }).unwrap();
        p.apply(Command::PutPoint { target, point: Point { at: 100, value: 1.0, shape: Shape::Linear } }).unwrap();
        let mut out = vec![[0.0; 2]; 8];
        let mut scratch = Mixdown::default();
        mix_tracks_metered(&p, 250, &mut out, None, None, &mut scratch, None);
        assert_eq!(out[4][0], 0.0, "the hold did not follow the clip");
        mix_tracks_metered(&p, 320, &mut out, None, None, &mut scratch, None);
        assert!(out[4][0] > 0.0, "after the clip's own 100 frames it should be open");
    }

    struct ClipDoubler {
        on: ClipId,
        ran: usize,
    }

    impl Chains for ClipDoubler {
        fn process(&mut self, _track: TrackId, _audio: &mut [[f32; 2]]) {}

        fn process_clip(&mut self, clip: ClipId, audio: &mut [[f32; 2]]) {
            if clip != self.on {
                return;
            }
            self.ran += 1;
            for frame in audio.iter_mut() {
                frame[0] *= 2.0;
                frame[1] *= 2.0;
            }
        }
    }

    #[test]
    fn a_clip_can_carry_its_own_chain() {
        let mut p = Project::new(48_000);
        let one = track(&mut p);
        let loud = clip(&mut p, one, counting(200), 0);
        let plain = clip(&mut p, one, counting(200), 300);
        let fx = crate::model::Fx {
            mix: 1.0,
            path: std::path::PathBuf::from("x.vst3"),
            index: 0,
            name: "x".into(),
            bypassed: false,
            state: Vec::new(),
            record: false,
        };
        p.apply(Command::AddClipFx { clip: loud, fx }).unwrap();
        let mut racks = ClipDoubler { on: loud, ran: 0 };
        let mut out = vec![[0.0; 2]; 600];
        let mut scratch = Mixdown::default();
        mix_tracks_metered(&p, 0, &mut out, None, None, &mut scratch, Some(&mut racks));
        assert_eq!(racks.ran, 1, "the clip chain did not run once");
        assert_eq!(out[50], [100.0, -100.0], "the clip with a chain was not doubled");
        assert_eq!(out[350], [50.0, -50.0], "the clip without a chain changed");
        let _ = plain;
    }

    struct ClipEcho {
        held: Vec<[f32; 2]>,
        at: usize,
    }

    impl Chains for ClipEcho {
        fn process(&mut self, _track: TrackId, _audio: &mut [[f32; 2]]) {}

        fn process_clip(&mut self, _clip: ClipId, audio: &mut [[f32; 2]]) {
            for frame in audio.iter_mut() {
                let back = self.held[self.at];
                self.held[self.at] = *frame;
                self.at = (self.at + 1) % self.held.len();
                frame[0] += back[0] * 0.5;
                frame[1] += back[1] * 0.5;
            }
        }
    }

    #[test]
    fn a_clip_chain_rings_on_after_its_clip_ends() {
        let mut p = Project::new(48_000);
        let one = track(&mut p);
        let mut audio = vec![[0.0f32; 2]; 1_000];
        audio[900] = [0.8, 0.8];
        let shout = clip(&mut p, one, Arc::new(Source::from_frames("shout", audio)), 0);
        let fx = crate::model::Fx { mix: 1.0, path: std::path::PathBuf::from("delay.vst3"), index: 0, name: "Delay".into(), bypassed: false, state: Vec::new(), record: false };
        p.apply(Command::AddClipFx { clip: shout, fx }).unwrap();
        let mut racks = ClipEcho { held: vec![[0.0; 2]; 500], at: 0 };
        let mut scratch = Mixdown::default();
        let mut heard = Vec::new();
        for block in 0..8 {
            let mut out = vec![[0.0; 2]; 256];
            mix_tracks_metered(&p, block * 256, &mut out, None, None, &mut scratch, Some(&mut racks));
            heard.extend_from_slice(&out);
        }
        assert!((heard[900][0] - 0.8).abs() < 1e-6, "the shout read {:?}", heard[900]);
        assert!((heard[1_400][0] - 0.4).abs() < 1e-6, "the clip's echo was cut off with the clip: {:?}", heard[1_400]);
    }

    struct Placer {
        latency: usize,
        heard: Vec<Frames>,
    }

    impl Chains for Placer {
        fn process(&mut self, _track: TrackId, _audio: &mut [[f32; 2]]) {}

        fn process_clip_at(&mut self, _clip: ClipId, _audio: &mut [[f32; 2]], at: Frames) {
            self.heard.push(at);
        }

        fn clip_latency(&self, _clip: ClipId) -> usize {
            self.latency
        }
    }

    #[test]
    fn a_clip_chain_is_told_where_in_the_song_each_block_starts() {
        let mut p = Project::new(48_000);
        let one = track(&mut p);
        let placed = clip(&mut p, one, counting(1000), 100);
        let fx = crate::model::Fx {
            mix: 1.0,
            path: std::path::PathBuf::from("melodyne.vst3"),
            index: 0,
            name: "Melodyne".into(),
            bypassed: false,
            state: Vec::new(),
            record: false,
        };
        p.apply(Command::AddClipFx { clip: placed, fx }).unwrap();
        let mut racks = Placer { latency: 0, heard: Vec::new() };
        let mut out = vec![[0.0; 2]; 256];
        let mut scratch = Mixdown::default();
        mix_tracks_metered(&p, 0, &mut out, None, None, &mut scratch, Some(&mut racks));
        mix_tracks_metered(&p, 256, &mut out, None, None, &mut scratch, Some(&mut racks));
        assert_eq!(racks.heard, vec![0, 256]);
        let mut late = Placer { latency: 64, heard: Vec::new() };
        mix_tracks_metered(&p, 512, &mut out, None, None, &mut scratch, Some(&mut late));
        assert_eq!(late.heard, vec![576]);
    }

    struct Ducker {
        on: TrackId,
        heard: f32,
    }

    impl Chains for Ducker {
        fn process_with(&mut self, track: TrackId, audio: &mut [[f32; 2]], side: &[[f32; 2]]) {
            if track != self.on {
                return;
            }
            self.heard = side.iter().fold(0.0f32, |most, frame| most.max(frame[0].abs()));
            let duck = 1.0 - self.heard.min(1.0);
            for frame in audio.iter_mut() {
                frame[0] *= duck;
                frame[1] *= duck;
            }
        }
    }

    #[test]
    fn a_sidechain_send_reaches_the_plugin_and_stays_out_of_the_mix() {
        let mut p = Project::new(48_000);
        let beat = track(&mut p);
        let vox = track(&mut p);
        flat(&mut p, beat, 0.8, 64);
        flat(&mut p, vox, 0.5, 64);
        p.apply(Command::AddSend { from: vox, to: beat }).unwrap();
        p.apply(Command::SetSendSidechain { from: vox, to: beat, sidechain: true }).unwrap();
        p.apply(Command::SetSendPreFader { from: vox, to: beat, pre_fader: true }).unwrap();
        p.apply(Command::SetTrackMuted { track: vox, muted: true }).unwrap();
        let fx = crate::model::Fx {
            mix: 1.0,
            path: std::path::PathBuf::from("duck.vst3"),
            index: 0,
            name: "duck".into(),
            bypassed: false,
            state: Vec::new(),
            record: false,
        };
        p.apply(Command::AddFx { track: beat, fx }).unwrap();
        let mut racks = Ducker { on: beat, heard: 0.0 };
        let mut out = vec![[0.0; 2]; 32];
        let mut scratch = Mixdown::default();
        mix_tracks_metered(&p, 1, &mut out, None, None, &mut scratch, Some(&mut racks));
        assert!((racks.heard - 0.5).abs() < 1e-6, "the plugin heard {}", racks.heard);
        assert!((out[8][0] - 0.4).abs() < 1e-6, "the mix came out at {}", out[8][0]);
    }

    struct Late {
        on: TrackId,
        by: usize,
        held: Vec<[f32; 2]>,
    }

    impl Chains for Late {
        fn process(&mut self, track: TrackId, audio: &mut [[f32; 2]]) {
            if track != self.on {
                return;
            }
            self.held.resize(self.by, [0.0; 2]);
            let mut out = Vec::with_capacity(audio.len());
            for frame in audio.iter() {
                self.held.push(*frame);
                out.push(self.held.remove(0));
            }
            audio.copy_from_slice(&out);
        }

        fn latency(&self, track: TrackId) -> usize {
            if track == self.on {
                self.by
            } else {
                0
            }
        }
    }

    #[test]
    fn a_slow_plugin_does_not_push_its_track_late() {
        let mut p = Project::new(48_000);
        let slow = track(&mut p);
        let plain = track(&mut p);
        clip(&mut p, slow, counting(400), 100);
        clip(&mut p, plain, counting(400), 100);
        let fx = crate::model::Fx {
            mix: 1.0,
            path: std::path::PathBuf::from("slow.vst3"),
            index: 0,
            name: "slow".into(),
            bypassed: false,
            state: Vec::new(),
            record: false,
        };
        p.apply(Command::AddFx { track: slow, fx }).unwrap();
        let mut racks = Late { on: slow, by: 32, held: Vec::new() };
        let mut out = vec![[0.0; 2]; 300];
        let mut scratch = Mixdown::default();
        mix_tracks_metered(&p, 0, &mut out, None, None, &mut scratch, Some(&mut racks));
        assert_eq!(out[101], [2.0, -2.0], "the two tracks did not land together");
        assert_eq!(out[150], [100.0, -100.0]);
    }

    #[test]
    fn with_no_plugins_nothing_moves() {
        let mut p = Project::new(48_000);
        let one = track(&mut p);
        clip(&mut p, one, counting(400), 100);
        let mut racks = Late { on: one, by: 32, held: Vec::new() };
        let mut out = vec![[0.0; 2]; 300];
        let mut scratch = Mixdown::default();
        mix_tracks_metered(&p, 0, &mut out, None, None, &mut scratch, Some(&mut racks));
        assert_eq!(out[101], [1.0, -1.0]);
    }

    struct Doubler {
        seen: Vec<TrackId>,
    }

    impl Chains for Doubler {
        fn process(&mut self, track: TrackId, audio: &mut [[f32; 2]]) {
            self.seen.push(track);
            for frame in audio.iter_mut() {
                frame[0] *= 2.0;
                frame[1] *= 2.0;
            }
        }
    }

    fn flat(p: &mut Project, track: TrackId, level: f32, frames: usize) {
        let source = Arc::new(Source::from_frames("flat", vec![[level, level]; frames + 2]));
        clip(p, track, source, 0);
    }

    struct Colouring {
        in_waves: bool,
        seen: Vec<TrackId>,
    }

    impl Chains for Colouring {
        fn whole_waves_at_once(&self) -> bool {
            self.in_waves
        }

        fn process_with(&mut self, track: TrackId, audio: &mut [[f32; 2]], side: &[[f32; 2]]) {
            self.seen.push(track);
            let tint = 1.0 + track.0 as f32 / 8.0;
            for (at, frame) in audio.iter_mut().enumerate() {
                let from_the_side = side.get(at).map(|one| one[0]).unwrap_or(0.0);
                frame[0] = frame[0] * tint + from_the_side;
                frame[1] = frame[1] * tint - from_the_side;
            }
        }
    }

    fn a_song_of_groups_and_sends() -> Project {
        let mut p = Project::new(48_000);
        let bus = track(&mut p);
        let verb = track(&mut p);
        let vox = track(&mut p);
        let beat = track(&mut p);
        flat(&mut p, vox, 0.25, 64);
        flat(&mut p, beat, 0.5, 64);
        flat(&mut p, verb, 0.1, 64);
        p.apply(Command::SetTrackParent { track: vox, parent: Some(bus) }).unwrap();
        p.apply(Command::SetTrackParent { track: beat, parent: Some(bus) }).unwrap();
        p.apply(Command::AddSend { from: vox, to: verb }).unwrap();
        p.apply(Command::SetSendGain { from: vox, to: verb, gain: 0.7 }).unwrap();
        p.apply(Command::AddSend { from: beat, to: verb }).unwrap();
        p.apply(Command::SetSendSidechain { from: beat, to: verb, sidechain: true }).unwrap();
        for track in [bus, verb, vox, beat] {
            let fx = crate::model::Fx {
                mix: 1.0,
                path: std::path::PathBuf::from("x.vst3"),
                index: 0,
                name: "x".into(),
                bypassed: false,
                state: Vec::new(),
                record: false,
            };
            p.apply(Command::AddFx { track, fx }).unwrap();
        }
        p
    }

    #[test]
    fn handing_a_whole_wave_over_at_once_sounds_the_same_as_one_track_at_a_time() {
        let p = a_song_of_groups_and_sends();
        let mut both = Vec::new();
        for in_waves in [false, true] {
            let mut racks = Colouring { in_waves, seen: Vec::new() };
            let mut out = vec![[0.0; 2]; 32];
            let mut scratch = Mixdown::default();
            mix_tracks_metered(&p, 1, &mut out, None, None, &mut scratch, Some(&mut racks));
            assert_eq!(racks.seen.len(), 4, "every chain ran once");
            both.push(out);
        }
        assert_eq!(both[0], both[1], "waves and one at a time give the same mix");
        assert!(both[0][8][0].abs() > 1e-6, "the mix is not silent");
    }

    #[test]
    fn a_wave_never_holds_a_track_that_feeds_another_in_it() {
        let p = a_song_of_groups_and_sends();
        let mut scratch = Mixdown::default();
        scratch.work_out_order(&p, None);
        for (from, to) in scratch.waves.clone() {
            for step in from..to {
                let one = p.tracks.iter().find(|t| t.id == scratch.order[step]).expect("the track is there");
                for other in from..to {
                    let them = scratch.order[other];
                    assert_ne!(one.parent, Some(them), "a child sits in its parent's wave");
                    assert!(!one.sends.iter().any(|send| send.to == them), "a send lands inside its own wave");
                }
            }
        }
        assert!(scratch.waves.len() > 1, "groups and sends make more than one wave");
    }

    struct CountingWaves {
        handed: Vec<usize>,
    }

    impl Chains for CountingWaves {
        fn whole_waves_at_once(&self) -> bool {
            true
        }

        fn process_wave(&mut self, jobs: &mut [Job<'_>]) {
            self.handed.push(jobs.len());
        }
    }

    fn a_song_of_many_plain_tracks(how_many: usize) -> Project {
        let mut p = Project::new(48_000);
        for _ in 0..how_many {
            let one = track(&mut p);
            flat(&mut p, one, 0.1, 64);
            let fx = crate::model::Fx {
                mix: 1.0,
                path: std::path::PathBuf::from("x.vst3"),
                index: 0,
                name: "x".into(),
                bypassed: false,
                state: Vec::new(),
                record: false,
            };
            p.apply(Command::AddFx { track: one, fx }).unwrap();
        }
        p
    }

    #[test]
    fn a_hundred_plain_tracks_are_handed_over_as_one_wave_of_a_hundred() {
        let p = a_song_of_many_plain_tracks(100);
        let mut scratch = Mixdown::default();
        scratch.work_out_order(&p, None);
        assert_eq!(scratch.waves, vec![(0, 100)], "tracks that feed nothing belong in one wave");
        let mut racks = CountingWaves { handed: Vec::new() };
        let mut out = vec![[0.0; 2]; 32];
        mix_tracks_metered(&p, 0, &mut out, None, None, &mut scratch, Some(&mut racks));
        assert_eq!(racks.handed, vec![100], "the racks were handed one wave of every track");
    }

    #[test]
    fn groups_make_waves_that_are_still_wide() {
        let mut p = a_song_of_many_plain_tracks(100);
        let bus = track(&mut p);
        let under: Vec<TrackId> = p.tracks.iter().take(100).map(|one| one.id).collect();
        for one in under {
            p.apply(Command::SetTrackParent { track: one, parent: Some(bus) }).unwrap();
        }
        let mut scratch = Mixdown::default();
        scratch.work_out_order(&p, None);
        let sizes: Vec<usize> = scratch.waves.iter().map(|(from, to)| to - from).collect();
        assert_eq!(sizes, vec![100, 1], "the hundred children go over together, then the bus");
        let mut racks = CountingWaves { handed: Vec::new() };
        let mut out = vec![[0.0; 2]; 32];
        mix_tracks_metered(&p, 0, &mut out, None, None, &mut scratch, Some(&mut racks));
        assert_eq!(racks.handed, vec![100], "only the tracks with plugins are handed over");
    }

    #[test]
    fn a_chain_runs_before_the_fader_and_feeds_the_sends() {
        let mut p = Project::new(48_000);
        let vox = track(&mut p);
        let verb = track(&mut p);
        flat(&mut p, vox, 0.25, 64);
        p.apply(Command::SetTrackGain { track: vox, gain: 0.5 }).unwrap();
        p.apply(Command::AddSend { from: vox, to: verb }).unwrap();
        p.apply(Command::SetSendGain { from: vox, to: verb, gain: 1.0 }).unwrap();
        p.apply(Command::SetSendPreFader { from: vox, to: verb, pre_fader: true }).unwrap();
        let fx = crate::model::Fx {
            mix: 1.0,
            path: std::path::PathBuf::from("x.vst3"),
            index: 0,
            name: "x".into(),
            bypassed: false,
            state: Vec::new(),
            record: false,
        };
        p.apply(Command::AddFx { track: vox, fx }).unwrap();
        let mut racks = Doubler { seen: Vec::new() };
        let mut out = vec![[0.0; 2]; 32];
        let mut scratch = Mixdown::default();
        mix_tracks_metered(&p, 1, &mut out, None, None, &mut scratch, Some(&mut racks));
        assert_eq!(racks.seen, vec![vox]);
        assert!((out[8][0] - 0.75).abs() < 1e-6, "got {}", out[8][0]);
    }

    #[test]
    fn a_track_with_no_plugins_is_never_handed_to_the_chains() {
        let mut p = Project::new(48_000);
        let bare = track(&mut p);
        flat(&mut p, bare, 0.5, 64);
        let mut racks = Doubler { seen: Vec::new() };
        let mut out = vec![[0.0; 2]; 32];
        let mut scratch = Mixdown::default();
        mix_tracks_metered(&p, 1, &mut out, None, None, &mut scratch, Some(&mut racks));
        assert!(racks.seen.is_empty());
        assert!((out[8][0] - 0.5).abs() < 1e-6, "got {}", out[8][0]);
    }

    #[test]
    fn any_block_size_gives_the_same_sound() {
        let mut p = Project::new(48_000);
        let t = track(&mut p);
        clip(&mut p, t, counting(1000), 37);
        clip(&mut p, t, counting(300), 500);
        let reference = whole(&p, 1200);
        for block in [1, 7, 64, 480, 1200] {
            let mut pieces = Vec::new();
            let mut pos = 0;
            while pos < 1200 {
                let n = block.min(1200 - pos);
                let mut out = vec![[0.0; 2]; n];
                render(&p, pos as Frames, &mut out);
                pieces.extend(out);
                pos += n;
            }
            assert_eq!(pieces, reference, "block size {block}");
        }
    }

    #[test]
    fn a_split_cannot_be_heard() {
        let mut p = Project::new(48_000);
        let t = track(&mut p);
        let c = clip(&mut p, t, counting(1000), 10);
        let before = whole(&p, 1100);
        p.apply(Command::SplitClip { clip: c, at: 333 }).unwrap();
        assert_eq!(p.clips().count(), 2);
        assert_eq!(whole(&p, 1100), before);
    }

    #[test]
    fn gain_on_one_piece_leaves_the_other_alone() {
        let mut p = Project::new(48_000);
        let t = track(&mut p);
        let left = clip(&mut p, t, counting(1000), 0);
        let Ok(Outcome::Clip(right)) = p.apply(Command::SplitClip { clip: left, at: 500 }) else {
            panic!("no split")
        };
        p.apply(Command::SetClipGain { clip: right, gain: 0.5 }).unwrap();
        let out = whole(&p, 1000);
        assert_eq!(out[499], [499.0, -499.0]);
        assert_eq!(out[500], [250.0, -250.0]);
        assert_eq!(out[999], [499.5, -499.5]);
    }

    fn steady(len: usize) -> Arc<Source> {
        Arc::new(Source::from_frames("one", vec![[1.0, 1.0]; len]))
    }

    #[test]
    fn fades_shape_only_the_ends_of_their_own_clip() {
        let mut p = Project::new(48_000);
        let t = track(&mut p);
        let c = clip(&mut p, t, steady(1000), 0);
        p.apply(Command::SetClipFade { clip: c, edge: Edge::In, fade: Fade { len: 100, curve: 0.0 } }).unwrap();
        p.apply(Command::SetClipFade { clip: c, edge: Edge::Out, fade: Fade { len: 200, curve: 0.0 } }).unwrap();
        let out = whole(&p, 1000);
        assert_eq!(out[0], [0.0, 0.0]);
        assert_eq!(out[50], [0.5, 0.5]);
        assert_eq!(out[100], [1.0, 1.0]);
        assert_eq!(out[799], [1.0, 1.0]);
        assert_eq!(out[900], [0.495, 0.495]);
        assert_eq!(out[999], [0.0, 0.0]);
        for block in [1, 7, 64, 333] {
            let mut pieces = Vec::new();
            let mut pos = 0;
            while pos < 1000 {
                let n = block.min(1000 - pos);
                let mut part = vec![[0.0; 2]; n];
                render(&p, pos as Frames, &mut part);
                pieces.extend(part);
                pos += n;
            }
            assert_eq!(pieces, out, "block size {block}");
        }
    }

    #[test]
    fn a_curved_fade_bends_but_keeps_its_ends() {
        let mut p = Project::new(48_000);
        let t = track(&mut p);
        let c = clip(&mut p, t, steady(1000), 0);
        p.apply(Command::SetClipFade { clip: c, edge: Edge::In, fade: Fade { len: 100, curve: 1.0 } }).unwrap();
        let fast = whole(&p, 1000);
        p.apply(Command::SetClipFade { clip: c, edge: Edge::In, fade: Fade { len: 100, curve: -1.0 } }).unwrap();
        let slow = whole(&p, 1000);
        assert_eq!((fast[0], fast[100]), ([0.0, 0.0], [1.0, 1.0]));
        assert_eq!((slow[0], slow[100]), ([0.0, 0.0], [1.0, 1.0]));
        assert!(fast[50][0] > 0.8 && slow[50][0] < 0.1);
    }

    #[test]
    fn fades_that_do_not_fit_are_refused_and_a_split_drops_the_inner_ones() {
        let mut p = Project::new(48_000);
        let t = track(&mut p);
        let c = clip(&mut p, t, steady(1000), 0);
        p.apply(Command::SetClipFade { clip: c, edge: Edge::In, fade: Fade { len: 600, curve: 0.0 } }).unwrap();
        assert!(p.apply(Command::SetClipFade { clip: c, edge: Edge::Out, fade: Fade { len: 500, curve: 0.0 } }).is_err());
        assert!(p.apply(Command::SetClipFade { clip: c, edge: Edge::Out, fade: Fade { len: 10, curve: 2.0 } }).is_err());
        p.apply(Command::SetClipFade { clip: c, edge: Edge::Out, fade: Fade { len: 300, curve: 0.0 } }).unwrap();
        let Ok(Outcome::Clip(right)) = p.apply(Command::SplitClip { clip: c, at: 400 }) else {
            panic!("no split")
        };
        let (left, right) = (p.clip(c).unwrap(), p.clip(right).unwrap());
        assert_eq!((left.fade_in.len, left.fade_out.len), (400, 0));
        assert_eq!((right.fade_in.len, right.fade_out.len), (0, 300));
    }

    #[test]
    fn a_muted_clip_is_silent_and_its_neighbour_is_not() {
        let mut p = Project::new(48_000);
        let t = track(&mut p);
        let left = clip(&mut p, t, counting(1000), 0);
        let Ok(Outcome::Clip(right)) = p.apply(Command::SplitClip { clip: left, at: 500 }) else {
            panic!("no split")
        };
        p.apply(Command::SetClipMuted { clip: right, muted: true }).unwrap();
        let out = whole(&p, 1000);
        assert_eq!(out[499], [499.0, -499.0]);
        assert_eq!(out[500], [0.0, 0.0]);
        p.apply(Command::SetClipMuted { clip: right, muted: false }).unwrap();
        assert_eq!(whole(&p, 1000)[500], [500.0, -500.0]);
    }

    #[test]
    fn one_clip_can_be_heard_alone_unless_it_is_muted() {
        let mut p = Project::new(48_000);
        let a = track(&mut p);
        let b = track(&mut p);
        let wanted = clip(&mut p, a, counting(100), 0);
        clip(&mut p, b, counting(100), 0);
        p.apply(Command::SetTrackMuted { track: a, muted: true }).unwrap();
        let mut out = vec![[9.0; 2]; 100];
        mix_tracks(&p, 0, &mut out, Some(wanted));
        assert_eq!(out[10], [10.0, -10.0]);
        p.apply(Command::SetClipMuted { clip: wanted, muted: true }).unwrap();
        mix_tracks(&p, 0, &mut out, Some(wanted));
        assert_eq!(out[10], [0.0, 0.0]);
    }

    #[test]
    fn a_level_change_glides_instead_of_stepping() {
        let mut out = vec![[1.0, 1.0]; 4];
        scale(&mut out, 1.0, 0.0);
        assert_eq!(out, vec![[0.75, 0.75], [0.5, 0.5], [0.25, 0.25], [0.0, 0.0]]);
    }

    #[test]
    fn the_master_level_scales_everything() {
        let mut p = Project::new(48_000);
        let a = track(&mut p);
        let b = track(&mut p);
        clip(&mut p, a, counting(100), 0);
        clip(&mut p, b, counting(100), 0);
        p.apply(Command::SetMasterGain(0.25)).unwrap();
        assert_eq!(whole(&p, 100)[10], [5.0, -5.0]);
        assert!(p.apply(Command::SetMasterGain(-1.0)).is_err());
    }

    #[test]
    fn tracks_add_together_and_mute_removes_one() {
        let mut p = Project::new(48_000);
        let a = track(&mut p);
        let b = track(&mut p);
        clip(&mut p, a, counting(100), 0);
        clip(&mut p, b, counting(100), 0);
        assert_eq!(whole(&p, 100)[10], [20.0, -20.0]);
        p.apply(Command::SetTrackGain { track: b, gain: 0.5 }).unwrap();
        assert_eq!(whole(&p, 100)[10], [15.0, -15.0]);
        p.apply(Command::SetTrackMuted { track: a, muted: true }).unwrap();
        assert_eq!(whole(&p, 100)[10], [5.0, -5.0]);
    }

    #[test]
    fn moving_and_deleting_change_what_plays() {
        let mut p = Project::new(48_000);
        let t = track(&mut p);
        let c = clip(&mut p, t, counting(100), 0);
        p.apply(Command::MoveClip { clip: c, track: t, start: 60 }).unwrap();
        let out = whole(&p, 200);
        assert_eq!(out[10], [0.0, 0.0]);
        assert_eq!(out[70], [10.0, -10.0]);
        p.apply(Command::DeleteClip(c)).unwrap();
        assert!(whole(&p, 200).iter().all(|f| *f == [0.0, 0.0]));
    }

    struct Lookahead {
        monitor_lag: usize,
        lags: Vec<(TrackId, usize)>,
        held: Vec<(TrackId, Vec<[f32; 2]>)>,
        peek: Option<TrackId>,
        audio_at: Vec<usize>,
        side_at: Vec<usize>,
        seen: usize,
        frames: usize,
    }

    impl Lookahead {
        fn with(lags: &[(TrackId, usize)]) -> Self {
            Self { monitor_lag: 0, lags: lags.to_vec(), held: Vec::new(), peek: None, audio_at: Vec::new(), side_at: Vec::new(), seen: 0, frames: 0 }
        }

        fn line(&mut self, track: TrackId, by: usize) -> &mut Vec<[f32; 2]> {
            if !self.held.iter().any(|(id, _)| *id == track) {
                self.held.push((track, vec![[0.0; 2]; by]));
            }
            &mut self.held.iter_mut().find(|(id, _)| *id == track).expect("the line is there").1
        }
    }

    impl Chains for Lookahead {
        fn process_with(&mut self, track: TrackId, audio: &mut [[f32; 2]], side: &[[f32; 2]]) {
            self.frames += audio.len();
            if self.peek == Some(track) {
                for (at, frame) in audio.iter().enumerate() {
                    if frame[0].abs() > 0.5 {
                        self.audio_at.push(self.seen + at);
                    }
                }
                for (at, frame) in side.iter().enumerate() {
                    if frame[0].abs() > 0.5 {
                        self.side_at.push(self.seen + at);
                    }
                }
                self.seen += audio.len();
            }
            let by = self.latency(track);
            if by == 0 {
                return;
            }
            let held = self.line(track, by);
            for frame in audio.iter_mut() {
                held.push(*frame);
                *frame = held.remove(0);
            }
        }

        fn latency(&self, track: TrackId) -> usize {
            self.lags.iter().find(|(id, _)| *id == track).map(|(_, by)| *by).unwrap_or(0)
        }

        fn monitor_chain_latency(&self) -> usize {
            self.monitor_lag
        }
    }

    fn fx(p: &mut Project, track: TrackId) {
        let fx = crate::model::Fx {
            mix: 1.0,
            path: std::path::PathBuf::from("look.vst3"),
            index: 0,
            name: "look".into(),
            bypassed: false,
            state: Vec::new(),
            record: false,
        };
        p.apply(Command::AddFx { track, fx }).unwrap();
    }

    fn blip(at: usize, len: usize) -> Arc<Source> {
        let mut frames = vec![[0.0f32; 2]; len];
        frames[at] = [1.0, 1.0];
        Arc::new(Source::from_frames("blip", frames))
    }

    fn mixed(p: &Project, lags: &[(TrackId, usize)], frames: usize, block: usize) -> Vec<[f32; 2]> {
        let mut racks = Lookahead::with(lags);
        let mut scratch = Mixdown::default();
        let mut all = vec![[0.0; 2]; frames];
        let mut pos = 0;
        while pos < frames {
            let count = block.min(frames - pos);
            mix_tracks_metered(p, pos as Frames, &mut all[pos..pos + count], None, None, &mut scratch, Some(&mut racks));
            pos += count;
        }
        all
    }

    fn loud(audio: &[[f32; 2]]) -> f32 {
        audio.iter().fold(0.0f32, |most, frame| most.max(frame[0].abs()))
    }

    #[test]
    fn a_latent_track_lands_on_the_same_frame_as_a_plain_one() {
        let mut p = Project::new(48_000);
        let late = track(&mut p);
        let plain = track(&mut p);
        clip(&mut p, late, counting(600), 100);
        clip(&mut p, plain, counting(600), 100);
        fx(&mut p, late);
        let with = mixed(&p, &[(late, 547)], 1200, 64);
        let without = mixed(&p, &[], 1200, 64);
        assert_eq!(with, without, "the latent track did not null against the plain one");
        assert!(loud(&with) > 1.0, "the mix is silent");
    }

    #[test]
    fn a_parallel_path_rejoins_in_step_with_the_direct_one() {
        let mut p = Project::new(48_000);
        let busb = track(&mut p);
        let parallel = track(&mut p);
        let bus = track(&mut p);
        clip(&mut p, bus, counting(600), 100);
        p.apply(Command::SetTrackParent { track: parallel, parent: Some(busb) }).unwrap();
        p.apply(Command::SetTrackParent { track: bus, parent: Some(busb) }).unwrap();
        p.apply(Command::AddSend { from: bus, to: parallel }).unwrap();
        p.apply(Command::SetSendGain { from: bus, to: parallel, gain: 1.0 }).unwrap();
        for one in [busb, parallel, bus] {
            fx(&mut p, one);
        }
        let lags = [(busb, 64), (parallel, 547), (bus, 31)];
        let with = mixed(&p, &lags, 1500, 64);
        let without = mixed(&p, &[], 1500, 64);
        assert_eq!(with, without, "the parallel return smeared against the direct path");
        assert!(loud(&with) > 1.0, "the mix is silent");
    }

    #[test]
    fn a_send_arrives_in_step_with_the_bus_it_lands_in() {
        let mut p = Project::new(48_000);
        let verb = track(&mut p);
        let vox = track(&mut p);
        clip(&mut p, verb, counting(600), 100);
        clip(&mut p, vox, counting(600), 100);
        p.apply(Command::AddSend { from: vox, to: verb }).unwrap();
        p.apply(Command::SetSendGain { from: vox, to: verb, gain: 1.0 }).unwrap();
        for one in [verb, vox] {
            fx(&mut p, one);
        }
        let with = mixed(&p, &[(verb, 128), (vox, 547)], 1500, 64);
        let without = mixed(&p, &[], 1500, 64);
        assert_eq!(with, without, "the send did not land with the bus's own signal");
        assert!(loud(&with) > 1.0, "the mix is silent");
    }

    #[test]
    fn a_pre_fader_send_is_pulled_back_as_well() {
        let mut p = Project::new(48_000);
        let verb = track(&mut p);
        let vox = track(&mut p);
        clip(&mut p, verb, counting(600), 100);
        clip(&mut p, vox, counting(600), 100);
        p.apply(Command::AddSend { from: vox, to: verb }).unwrap();
        p.apply(Command::SetSendGain { from: vox, to: verb, gain: 1.0 }).unwrap();
        p.apply(Command::SetSendPreFader { from: vox, to: verb, pre_fader: true }).unwrap();
        p.apply(Command::SetTrackGain { track: vox, gain: 0.5 }).unwrap();
        for one in [verb, vox] {
            fx(&mut p, one);
        }
        let with = mixed(&p, &[(verb, 97), (vox, 400)], 1500, 64);
        let without = mixed(&p, &[], 1500, 64);
        assert_eq!(with, without, "the pre-fader send did not line up");
        assert!(loud(&with) > 1.0, "the mix is silent");
    }

    #[test]
    fn nested_folders_add_their_latency_up() {
        let mut p = Project::new(48_000);
        let outer = track(&mut p);
        let middle = track(&mut p);
        let inner = track(&mut p);
        let plain = track(&mut p);
        clip(&mut p, inner, counting(600), 100);
        clip(&mut p, plain, counting(600), 100);
        p.apply(Command::SetTrackParent { track: middle, parent: Some(outer) }).unwrap();
        p.apply(Command::SetTrackParent { track: inner, parent: Some(middle) }).unwrap();
        for one in [outer, middle, inner] {
            fx(&mut p, one);
        }
        let with = mixed(&p, &[(outer, 120), (middle, 240), (inner, 60)], 1500, 64);
        let without = mixed(&p, &[], 1500, 64);
        assert_eq!(with, without, "the nested folders did not stay in step");
        assert!(loud(&with) > 1.0, "the mix is silent");
    }

    #[test]
    fn a_plugin_that_moves_its_latency_has_the_graph_worked_out_again() {
        let mut p = Project::new(48_000);
        let one = track(&mut p);
        let plain = track(&mut p);
        clip(&mut p, one, counting(600), 100);
        clip(&mut p, plain, counting(600), 100);
        fx(&mut p, one);
        let mut racks = Lookahead::with(&[(one, 64)]);
        let mut scratch = Mixdown::default();
        let mut out = vec![[0.0; 2]; 64];
        mix_tracks_metered(&p, 0, &mut out, None, None, &mut scratch, Some(&mut racks));
        assert_eq!(scratch.lags, vec![64, 0]);
        assert!(scratch.same_shape(&p, Some(&racks)), "nothing moved, so nothing should be worked out again");
        racks.lags = vec![(one, 1_024)];
        assert!(!scratch.same_shape(&p, Some(&racks)), "the graph did not notice the plugin moved");
        mix_tracks_metered(&p, 64, &mut out, None, None, &mut scratch, Some(&mut racks));
        assert_eq!(scratch.lags, vec![1_024, 0], "the graph kept the old figure");
        assert_eq!(scratch.ahead, vec![1_024, 0], "the track with the plugin is not read ahead by its new figure");
    }

    #[test]
    fn a_sidechain_send_reaches_the_plugin_on_the_frame_it_is_ducking() {
        let mut p = Project::new(48_000);
        let beat = track(&mut p);
        let vox = track(&mut p);
        clip(&mut p, beat, blip(200, 600), 0);
        clip(&mut p, vox, blip(200, 600), 0);
        p.apply(Command::AddSend { from: vox, to: beat }).unwrap();
        p.apply(Command::SetSendGain { from: vox, to: beat, gain: 1.0 }).unwrap();
        p.apply(Command::SetSendSidechain { from: vox, to: beat, sidechain: true }).unwrap();
        p.apply(Command::SetSendPreFader { from: vox, to: beat, pre_fader: true }).unwrap();
        fx(&mut p, beat);
        fx(&mut p, vox);
        let mut racks = Lookahead::with(&[(beat, 547), (vox, 96)]);
        racks.peek = Some(beat);
        let mut scratch = Mixdown::default();
        let mut out = vec![[0.0; 2]; 64];
        for pos in 0..20 {
            mix_tracks_metered(&p, pos * 64, &mut out, None, None, &mut scratch, Some(&mut racks));
        }
        assert_eq!(racks.audio_at.len(), 1, "the beat blip was heard at {:?}", racks.audio_at);
        assert_eq!(racks.side_at, racks.audio_at, "the sidechain arrived on a different frame");
    }

    #[test]
    fn a_project_with_no_latency_mixes_exactly_as_it_did_before() {
        let p = a_song_of_groups_and_sends();
        let mut racks = Lookahead::with(&[]);
        let mut scratch = Mixdown::default();
        let mut through = vec![[0.0; 2]; 64];
        mix_tracks_metered(&p, 1, &mut through, None, None, &mut scratch, Some(&mut racks));
        let mut bare = vec![[0.0; 2]; 64];
        let mut spare = Mixdown::default();
        mix_tracks_metered(&p, 1, &mut bare, None, None, &mut spare, None);
        assert_eq!(through, bare, "a zero latency project moved");
        assert_eq!(scratch.ahead, vec![0; p.tracks.len()], "nothing should be read ahead");
        assert!(scratch.lines.iter().all(|line| line.delay == 0), "a delay line was handed a delay");
    }

    #[test]
    fn compensation_comes_out_the_same_in_any_block_size() {
        let mut p = Project::new(48_000);
        let busb = track(&mut p);
        let parallel = track(&mut p);
        let bus = track(&mut p);
        let verb = track(&mut p);
        clip(&mut p, bus, counting(600), 100);
        clip(&mut p, verb, counting(600), 100);
        p.apply(Command::SetTrackParent { track: parallel, parent: Some(busb) }).unwrap();
        p.apply(Command::SetTrackParent { track: bus, parent: Some(busb) }).unwrap();
        p.apply(Command::AddSend { from: bus, to: parallel }).unwrap();
        p.apply(Command::AddSend { from: bus, to: verb }).unwrap();
        for one in [busb, parallel, bus, verb] {
            fx(&mut p, one);
        }
        let lags = [(busb, 48), (parallel, 547), (bus, 31), (verb, 200)];
        let reference = mixed(&p, &lags, 2000, 2000);
        for block in [1, 7, 64, 480] {
            assert_eq!(mixed(&p, &lags, 2000, block), reference, "block size {block}");
        }
        assert_eq!(reference, mixed(&p, &[], 2000, 2000), "the compensated mix did not null");
    }

    fn a_song_of_three_heavy_buses() -> (Project, Vec<(TrackId, usize)>) {
        let mut p = Project::new(48_000);
        let mut lags = Vec::new();
        let verb = track(&mut p);
        fx(&mut p, verb);
        lags.push((verb, 4_096));
        for _ in 0..3 {
            let bus = track(&mut p);
            let vox = track(&mut p);
            clip(&mut p, vox, counting(300_000), 0);
            p.apply(Command::SetTrackParent { track: vox, parent: Some(bus) }).unwrap();
            p.apply(Command::AddSend { from: bus, to: verb }).unwrap();
            fx(&mut p, bus);
            lags.push((bus, 16_383 + 26_623 + 2_048));
        }
        (p, lags)
    }

    #[test]
    fn the_racks_leaving_and_coming_back_does_not_re_run_the_lookahead() {
        let (p, lags) = a_song_of_three_heavy_buses();
        let mut racks = Lookahead::with(&lags);
        let mut scratch = Mixdown::default();
        let mut out = vec![[0.0; 2]; 512];
        for block in 0..7 {
            mix_tracks_metered(&p, block * 512, &mut out, None, None, &mut scratch, Some(&mut racks));
        }
        let settled = racks.frames;
        mix_tracks_metered(&p, 7 * 512, &mut out, None, None, &mut scratch, Some(&mut racks));
        let a_block = racks.frames - settled;
        let ahead = scratch.ahead.clone();
        let delays: Vec<usize> = scratch.lines.iter().map(|line| line.delay).collect();
        mix_tracks_metered(&p, 8 * 512, &mut out, None, None, &mut scratch, None);
        assert_eq!(scratch.ahead, ahead, "the racks going out moved what the mix reads ahead");
        assert_eq!(scratch.lines.iter().map(|line| line.delay).collect::<Vec<usize>>(), delays, "the racks going out emptied the delay lines");
        let before = racks.frames;
        mix_tracks_metered(&p, 9 * 512, &mut out, None, None, &mut scratch, Some(&mut racks));
        let after = racks.frames - before;
        assert_eq!(after, a_block, "the racks coming back put {after} frames through the plugins in one block instead of {a_block}");
    }

    #[test]
    fn a_monitoring_chain_that_changes_leaves_the_mix_lookahead_alone() {
        let (p, lags) = a_song_of_three_heavy_buses();
        let mut racks = Lookahead::with(&lags);
        let mut scratch = Mixdown::default();
        let mut out = vec![[0.0; 2]; 512];
        for block in 0..8 {
            mix_tracks_metered(&p, block * 512, &mut out, None, None, &mut scratch, Some(&mut racks));
        }
        let settled = racks.frames;
        mix_tracks_metered(&p, 8 * 512, &mut out, None, None, &mut scratch, Some(&mut racks));
        let a_block = racks.frames - settled;
        let ahead = scratch.ahead.clone();
        let delays: Vec<usize> = scratch.lines.iter().map(|line| line.delay).collect();

        racks.monitor_lag = 4_096;
        let before = racks.frames;
        mix_tracks_metered(&p, 9 * 512, &mut out, None, None, &mut scratch, Some(&mut racks));
        let after = racks.frames - before;
        assert_eq!(scratch.ahead, ahead, "a monitoring chain moved what the mix reads ahead");
        assert_eq!(scratch.lines.iter().map(|line| line.delay).collect::<Vec<usize>>(), delays, "a monitoring chain emptied the delay lines");
        assert_eq!(after, a_block, "a monitoring chain put {after} frames through the plugins in one block instead of {a_block}");
    }

    fn looped(p: &Project, lags: &[(TrackId, usize)], from: Frames, to: Frames, rounds: usize, block: usize) -> Vec<[f32; 2]> {
        let mut racks = Lookahead::with(lags);
        let mut scratch = Mixdown::default();
        let round = Some((from, to));
        let mut all = vec![[0.0; 2]; (to - from) as usize * rounds];
        scratch.get_ready(p, from, round, None, block, Some(&mut racks), None);
        let mut pos = from;
        let mut done = 0;
        while done < all.len() {
            let count = block.min(all.len() - done).min((to - pos) as usize);
            mix_tracks_metered(p, pos, &mut all[done..done + count], None, None, &mut scratch, Some(&mut racks));
            done += count;
            pos += count as Frames;
            if pos == to {
                pos = from;
            }
        }
        all
    }

    #[test]
    fn a_loop_comes_round_with_the_compensation_still_nulled() {
        let (p, lags) = a_song_of_three_heavy_buses();
        let (from, to) = (100_000, 140_000);
        let with = looped(&p, &lags, from, to, 2, 512);
        let without = looped(&p, &[], from, to, 2, 512);
        assert_eq!(with, without, "the compensated loop did not null against the plain one");
        let span = (to - from) as usize;
        assert_eq!(with[..span], with[span..], "the second time round the loop came out different");
        assert!(loud(&with) > 1.0, "the loop is silent");
    }

    #[test]
    fn a_loop_wrap_asks_for_no_pre_roll() {
        let (p, lags) = a_song_of_three_heavy_buses();
        let (from, to) = (100_000, 140_000);
        let round = Some((from, to));
        let mut racks = Lookahead::with(&lags);
        let mut scratch = Mixdown::default();
        scratch.hurries();
        let mut out = vec![[0.0; 2]; 512];
        scratch.get_ready(&p, from, round, None, 512, Some(&mut racks), None);
        let mut pos = from;
        while pos + 512 < to {
            assert_eq!(scratch.ready_for(&p, pos, round, Some(&racks)), 0, "the mix wanted a pre-roll at {pos}");
            mix_tracks_metered(&p, pos, &mut out, None, None, &mut scratch, Some(&mut racks));
            pos += 512;
        }
        let last = (to - pos) as usize;
        mix_tracks_metered(&p, pos, &mut out[..last], None, None, &mut scratch, Some(&mut racks));
        assert_eq!(scratch.ready_for(&p, from, round, Some(&racks)), 0, "the wrap back to the loop start wanted a pre-roll");
    }

    #[test]
    fn a_seek_that_was_rolled_ahead_lands_on_the_same_audio() {
        let (p, lags) = a_song_of_three_heavy_buses();
        let target = 150_000;
        let mut fresh = Lookahead::with(&lags);
        let mut clean = Mixdown::default();
        let mut want = vec![[0.0; 2]; 1_024];
        clean.get_ready(&p, target, None, None, 512, Some(&mut fresh), None);
        mix_tracks_metered(&p, target, &mut want[..512], None, None, &mut clean, Some(&mut fresh));
        mix_tracks_metered(&p, target + 512, &mut want[512..], None, None, &mut clean, Some(&mut fresh));

        let mut racks = Lookahead::with(&lags);
        let mut scratch = Mixdown::default();
        scratch.hurries();
        let mut out = vec![[0.0; 2]; 1_024];
        for block in 0..8 {
            mix_tracks_metered(&p, block * 512, &mut out[..512], None, None, &mut scratch, Some(&mut racks));
        }
        scratch.get_ready(&p, target, None, None, 512, Some(&mut racks), None);
        mix_tracks_metered(&p, target, &mut out[..512], None, None, &mut scratch, Some(&mut racks));
        mix_tracks_metered(&p, target + 512, &mut out[512..], None, None, &mut scratch, Some(&mut racks));
        assert_eq!(out, want, "the mix after a rolled seek is not what an untouched render gives");
        assert!(loud(&want) > 1.0, "the mix is silent");
    }

    #[test]
    fn a_hurried_mix_never_primes_more_than_a_block() {
        let (p, lags) = a_song_of_three_heavy_buses();
        let mut racks = Lookahead::with(&lags);
        let mut scratch = Mixdown::default();
        scratch.hurries();
        let mut out = vec![[0.0; 2]; 512];
        for block in 0..8 {
            mix_tracks_metered(&p, block * 512, &mut out, None, None, &mut scratch, Some(&mut racks));
        }
        let settled = racks.frames;
        mix_tracks_metered(&p, 8 * 512, &mut out, None, None, &mut scratch, Some(&mut racks));
        let a_block = racks.frames - settled;
        let before = racks.frames;
        mix_tracks_metered(&p, 150_000, &mut out, None, None, &mut scratch, Some(&mut racks));
        let after = racks.frames - before;
        assert!(after <= 2 * a_block, "a jump put {after} frames through the plugins, a settled block puts {a_block}");
    }

    #[test]
    fn the_monitoring_path_reports_what_the_mix_pulls_back() {
        let mut p = Project::new(48_000);
        let bus = track(&mut p);
        let vox = track(&mut p);
        p.apply(Command::SetTrackParent { track: vox, parent: Some(bus) }).unwrap();
        for one in [bus, vox] {
            fx(&mut p, one);
        }
        let mut racks = Lookahead::with(&[(bus, 300), (vox, 547)]);
        assert_eq!(monitor_latency(&p, vox, &racks), 300, "a performer hears everything below their own chain");
        assert_eq!(monitor_latency(&p, bus, &racks), 0, "a top level track hears itself straight away");
        racks.monitor_lag = 64;
        assert_eq!(monitor_latency(&p, vox, &racks), 364, "the monitoring chain is not counted in what the singer hears");
        assert_eq!(monitor_latency(&p, bus, &racks), 64);
    }

    fn tapped(p: &Project, frames: usize) -> Vec<Vec<[f32; 2]>> {
        let mut out = vec![[0.0; 2]; frames];
        let mut scratch = Mixdown::default();
        mix_tracks_metered(p, 0, &mut out, None, None, &mut scratch, None);
        (0..p.tracks.len())
            .map(|index| scratch.tap(index, frames).map(<[[f32; 2]]>::to_vec).unwrap_or_default())
            .collect()
    }

    #[test]
    fn a_bus_with_no_clips_of_its_own_taps_what_its_child_feeds_it() {
        let mut p = Project::new(48_000);
        let bus = track(&mut p);
        let child = track(&mut p);
        flat(&mut p, child, 0.5, 64);
        p.apply(Command::SetTrackParent { track: child, parent: Some(bus) }).unwrap();
        let taps = tapped(&p, 32);
        assert!((taps[0][8][0] - 0.5).abs() < 1e-6, "the bus tapped {:?}", taps[0][8]);
        assert!((taps[1][8][0] - 0.5).abs() < 1e-6, "the child tapped {:?}", taps[1][8]);
    }

    #[test]
    fn a_tap_carries_the_track_gain_and_pan() {
        let mut p = Project::new(48_000);
        let one = track(&mut p);
        flat(&mut p, one, 0.8, 64);
        p.apply(Command::SetTrackGain { track: one, gain: 0.5 }).unwrap();
        p.apply(Command::SetTrackPan { track: one, pan: 1.0 }).unwrap();
        let taps = tapped(&p, 32);
        let (left, right) = pan_gains(1.0);
        assert!((taps[0][8][0] - 0.4 * left).abs() < 1e-6, "{:?}", taps[0][8]);
        assert!((taps[0][8][1] - 0.4 * right).abs() < 1e-6, "{:?}", taps[0][8]);
    }

    #[test]
    fn a_send_into_a_bus_shows_up_in_the_tap_of_that_bus() {
        let mut p = Project::new(48_000);
        let reverb = track(&mut p);
        let vox = track(&mut p);
        flat(&mut p, vox, 0.4, 64);
        p.apply(Command::AddSend { from: vox, to: reverb }).unwrap();
        p.apply(Command::SetSendGain { from: vox, to: reverb, gain: 0.25 }).unwrap();
        let taps = tapped(&p, 32);
        assert!((taps[0][8][0] - 0.1).abs() < 1e-6, "the reverb bus tapped {:?}", taps[0][8]);
        assert!((taps[1][8][0] - 0.4).abs() < 1e-6, "the send did not change the vox {:?}", taps[1][8]);
    }

    #[test]
    fn a_pre_fader_send_taps_the_level_before_the_fader() {
        let mut p = Project::new(48_000);
        let reverb = track(&mut p);
        let vox = track(&mut p);
        flat(&mut p, vox, 0.4, 64);
        p.apply(Command::AddSend { from: vox, to: reverb }).unwrap();
        p.apply(Command::SetSendPreFader { from: vox, to: reverb, pre_fader: true }).unwrap();
        p.apply(Command::SetTrackGain { track: vox, gain: 0.5 }).unwrap();
        let taps = tapped(&p, 32);
        assert!((taps[0][8][0] - 0.4).abs() < 1e-6, "the reverb bus tapped {:?}", taps[0][8]);
        assert!((taps[1][8][0] - 0.2).abs() < 1e-6, "{:?}", taps[1][8]);
    }

    #[test]
    fn the_taps_of_the_top_level_tracks_add_up_to_the_mix() {
        let mut p = Project::new(48_000);
        let bus = track(&mut p);
        let child = track(&mut p);
        let beat = track(&mut p);
        flat(&mut p, child, 0.3, 64);
        flat(&mut p, beat, 0.2, 64);
        p.apply(Command::SetTrackParent { track: child, parent: Some(bus) }).unwrap();
        let mut out = vec![[0.0; 2]; 32];
        let mut scratch = Mixdown::default();
        mix_tracks_metered(&p, 0, &mut out, None, None, &mut scratch, None);
        let bus_tap = scratch.tap(0, 32).unwrap()[8];
        let beat_tap = scratch.tap(2, 32).unwrap()[8];
        assert!((out[8][0] - bus_tap[0] - beat_tap[0]).abs() < 1e-6, "{:?}", out[8]);
    }
}

#[cfg(test)]
mod master_mute {
    use crate::model::{Command, Project};
    use crate::render::render;

    #[test]
    fn a_muted_master_renders_silence() {
        let mut project = Project::new(48_000);
        project.apply(Command::AddTrack { name: "t".into() }).unwrap();
        let mut out = vec![[0.5f32; 2]; 64];
        project.apply(Command::ToggleMasterMute).unwrap();
        render(&project, 0, &mut out);
        assert!(out.iter().all(|frame| frame[0] == 0.0 && frame[1] == 0.0));
    }
}

#[cfg(test)]
mod routing {
    use super::*;
    use crate::model::{Command, Outcome};
    use crate::source::Source;
    use std::sync::Arc;

    fn ones(len: usize) -> Arc<Source> {
        Arc::new(Source::from_frames("ones", vec![[1.0, 1.0]; len]))
    }

    fn track(p: &mut Project) -> TrackId {
        match p.apply(Command::AddTrack { name: "T".into() }) {
            Ok(Outcome::Track(id)) => id,
            other => panic!("{other:?}"),
        }
    }

    fn heard(project: &Project, len: usize) -> Vec<[f32; 2]> {
        let mut out = vec![[0.0; 2]; len];
        mix_tracks(project, 0, &mut out, None);
        out
    }

    #[test]
    fn pan_turns_down_the_far_side_only() {
        let mut p = Project::new(48_000);
        let t = track(&mut p);
        p.apply(Command::AddClip { track: t, source: ones(8), start: 0 }).unwrap();
        assert_eq!(heard(&p, 4)[0], [1.0, 1.0]);
        p.apply(Command::SetTrackPan { track: t, pan: -1.0 }).unwrap();
        let hard_left = heard(&p, 4)[0];
        assert!(hard_left[0] == 1.0 && hard_left[1].abs() < 1e-6);
        p.apply(Command::SetTrackPan { track: t, pan: 0.5 }).unwrap();
        let half_right = heard(&p, 4)[0];
        assert!((half_right[0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3 && half_right[1] == 1.0);
        assert!(p.apply(Command::SetTrackPan { track: t, pan: 1.5 }).is_err());
        assert!(p.apply(Command::SetTrackPan { track: t, pan: f32::NAN }).is_err());
    }

    #[test]
    fn solo_keeps_the_soloed_track_its_children_its_folder_and_its_sends() {
        let mut p = Project::new(48_000);
        let folder = track(&mut p);
        let vocal = track(&mut p);
        let drums = track(&mut p);
        let verb = track(&mut p);
        p.apply(Command::SetTrackParent { track: vocal, parent: Some(folder) }).unwrap();
        p.apply(Command::AddClip { track: folder, source: ones(8), start: 0 }).unwrap();
        p.apply(Command::AddClip { track: vocal, source: ones(8), start: 0 }).unwrap();
        p.apply(Command::AddClip { track: drums, source: ones(8), start: 0 }).unwrap();
        p.apply(Command::AddSend { from: vocal, to: verb }).unwrap();
        p.apply(Command::SetSendGain { from: vocal, to: verb, gain: 0.25 }).unwrap();
        assert_eq!(heard(&p, 4)[0][0], 3.25);
        p.apply(Command::SetTrackSolo { track: vocal, solo: true }).unwrap();
        assert_eq!(heard(&p, 4)[0][0], 1.25, "the vocal and its reverb only");
        p.apply(Command::SetTrackSolo { track: vocal, solo: false }).unwrap();
        p.apply(Command::SetTrackSolo { track: folder, solo: true }).unwrap();
        assert_eq!(heard(&p, 4)[0][0], 2.25, "the folder brings its children");
        p.apply(Command::SetTrackMuted { track: folder, muted: true }).unwrap();
        assert_eq!(heard(&p, 4)[0][0], 0.25, "mute beats solo");
    }

    #[test]
    fn a_folder_fader_scales_every_child() {
        let mut p = Project::new(48_000);
        let folder = track(&mut p);
        let child = track(&mut p);
        p.apply(Command::AddClip { track: child, source: ones(8), start: 0 }).unwrap();
        p.apply(Command::SetTrackParent { track: child, parent: Some(folder) }).unwrap();
        assert_eq!(heard(&p, 4)[0][0], 1.0);
        p.apply(Command::SetTrackGain { track: folder, gain: 0.5 }).unwrap();
        assert_eq!(heard(&p, 4)[0][0], 0.5);
    }

    #[test]
    fn muting_a_folder_silences_its_children() {
        let mut p = Project::new(48_000);
        let folder = track(&mut p);
        let child = track(&mut p);
        p.apply(Command::AddClip { track: child, source: ones(8), start: 0 }).unwrap();
        p.apply(Command::SetTrackParent { track: child, parent: Some(folder) }).unwrap();
        p.apply(Command::SetTrackMuted { track: folder, muted: true }).unwrap();
        assert_eq!(heard(&p, 4)[0][0], 0.0);
    }

    #[test]
    fn a_send_adds_the_signal_to_its_target() {
        let mut p = Project::new(48_000);
        let from = track(&mut p);
        let to = track(&mut p);
        p.apply(Command::AddClip { track: from, source: ones(8), start: 0 }).unwrap();
        p.apply(Command::AddSend { from, to }).unwrap();
        p.apply(Command::SetSendGain { from, to, gain: 0.5 }).unwrap();
        assert_eq!(heard(&p, 4)[0][0], 1.5);
    }

    #[test]
    fn a_pre_fader_send_ignores_the_track_fader() {
        let mut p = Project::new(48_000);
        let from = track(&mut p);
        let to = track(&mut p);
        p.apply(Command::AddClip { track: from, source: ones(8), start: 0 }).unwrap();
        p.apply(Command::AddSend { from, to }).unwrap();
        p.apply(Command::SetSendPreFader { from, to, pre_fader: true }).unwrap();
        p.apply(Command::SetTrackGain { track: from, gain: 0.0 }).unwrap();
        assert_eq!(heard(&p, 4)[0][0], 1.0);
    }

    #[test]
    fn a_send_that_would_feed_back_is_refused() {
        let mut p = Project::new(48_000);
        let a = track(&mut p);
        let b = track(&mut p);
        p.apply(Command::AddSend { from: a, to: b }).unwrap();
        assert!(p.apply(Command::AddSend { from: b, to: a }).is_err());
    }

    #[test]
    fn a_track_cannot_sit_inside_its_own_child() {
        let mut p = Project::new(48_000);
        let parent = track(&mut p);
        let child = track(&mut p);
        p.apply(Command::SetTrackParent { track: child, parent: Some(parent) }).unwrap();
        assert!(p.apply(Command::SetTrackParent { track: parent, parent: Some(child) }).is_err());
    }
}

fn heard_lag(track: &Track, racks: &dyn Chains) -> usize {
    match track.fx.is_empty() {
        true => 0,
        false => racks.heard_latency(track.id),
    }
}

fn lag_to_master(project: &Project, track: TrackId, racks: &dyn Chains, depth: usize) -> usize {
    if depth > project.tracks.len() {
        return 0;
    }
    let Some(found) = project.tracks.iter().find(|t| t.id == track) else { return 0 };
    heard_lag(found, racks) + below(project, found, racks, depth)
}

fn below(project: &Project, track: &Track, racks: &dyn Chains, depth: usize) -> usize {
    let mut most = match track.parent {
        Some(parent) => lag_to_master(project, parent, racks, depth + 1),
        None => 0,
    };
    for send in &track.sends {
        most = most.max(lag_to_master(project, send.to, racks, depth + 1));
    }
    most
}

pub fn monitor_latency(project: &Project, track: TrackId, racks: &dyn Chains) -> usize {
    let Some(found) = project.tracks.iter().find(|t| t.id == track) else { return 0 };
    racks.heard_takes_latency(track) + below(project, found, racks, 0) + racks.monitor_chain_latency()
}

fn automated(project: &Project, target: crate::envelope::Target, at: Frames, fallback: f32) -> f32 {
    match project.envelope(target).and_then(|shape| shape.value_at(at)) {
        Some(value) => value,
        None => fallback,
    }
}

fn clip_automated(project: &Project, target: crate::envelope::Target, at: Frames, start: Frames, fallback: f32) -> f32 {
    match project.envelope(target) {
        Some(shape) => shape.value_at(at.saturating_sub(start)).unwrap_or(fallback),
        None => fallback,
    }
}

fn turn_knobs(project: &Project, at: Frames, racks: &mut (dyn Chains + '_)) {
    for shape in &project.envelopes {
        let Some(value) = shape.value_at(at) else { continue };
        match shape.target {
            crate::envelope::Target::TrackFx { track, slot, knob } => racks.automate(track, slot, knob, value),
            crate::envelope::Target::ClipFx { clip, slot, knob } => racks.automate_clip(clip, slot, knob, value),
            crate::envelope::Target::MasterFx { slot, knob } => racks.automate_master(slot, knob, value),
            _ => {}
        }
    }
}

#[cfg(test)]
mod note_tests {
    use super::*;
    use crate::file::SavedProject;
    use crate::model::{Command, Outcome};

    const RATE: u32 = 48_000;

    fn song(instrument: Instrument, notes: Vec<Note>) -> (Project, ClipId) {
        let mut project = Project::new(RATE);
        let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: "Keys".into() }) else { panic!("no track") };
        project.apply(Command::SetInstrument { track, instrument }).unwrap();
        let made = Command::AddNotesClip { track, name: "Melody".into(), start: 4_800, len: 48_000, notes };
        let Ok(Outcome::Clip(clip)) = project.apply(made) else { panic!("no clip") };
        (project, clip)
    }

    fn heard(project: &Project, frames: usize) -> Vec<[f32; 2]> {
        let mut out = vec![[0.0; 2]; frames];
        render(project, 0, &mut out);
        out
    }

    fn loudness(audio: &[[f32; 2]]) -> f32 {
        audio.iter().map(|f| f[0].abs()).fold(0.0, f32::max)
    }

    #[test]
    fn a_note_sounds_where_it_sits_in_its_clip() {
        let (project, _) = song(Instrument::default(), vec![Note { key: 60, start: 9_600, len: 4_800, velocity: 1.0 }]);
        let out = heard(&project, 60_000);
        assert_eq!(loudness(&out[..4_800 + 9_600]), 0.0, "silent before the note");
        assert!(loudness(&out[14_400..19_200]) > 0.05);
        assert_eq!(project.length(), 4_800 + 48_000);
    }

    #[test]
    fn a_note_clip_plays_the_same_in_any_block_size_and_through_a_split() {
        let notes = vec![
            Note { key: 48, start: 0, len: 20_000, velocity: 0.9 },
            Note { key: 55, start: 10_000, len: 30_000, velocity: 0.7 },
        ];
        let (mut project, clip) = song(Instrument::default(), notes);
        let whole = heard(&project, 60_000);
        let mut pieces = vec![[0.0f32; 2]; 60_000];
        for (n, part) in pieces.chunks_mut(1000).enumerate() {
            render(&project, (n * 1000) as Frames, part);
        }
        assert_eq!(whole, pieces);
        project.apply(Command::SplitClip { clip, at: 25_000 }).unwrap();
        let split = heard(&project, 60_000);
        assert!(whole.iter().zip(&split).take(25_000).all(|(a, b)| a == b));
        let after: f32 = whole[25_000..40_000].iter().zip(&split[25_000..40_000]).map(|(a, b)| (a[0] - b[0]).abs()).fold(0.0, f32::max);
        assert!(after < 1e-6, "the right piece carries on the held notes, differs by {after}");
    }

    #[test]
    fn muting_and_clip_gain_apply_to_notes_too() {
        let (mut project, clip) = song(Instrument::Drums, vec![Note { key: 36, start: 0, len: 100, velocity: 1.0 }]);
        let full = loudness(&heard(&project, 20_000));
        project.apply(Command::SetClipGain { clip, gain: 0.5 }).unwrap();
        let half = loudness(&heard(&project, 20_000));
        assert!((half / full - 0.5).abs() < 1e-3);
        project.apply(Command::SetClipMuted { clip, muted: true }).unwrap();
        assert_eq!(loudness(&heard(&project, 20_000)), 0.0);
    }

    #[test]
    fn notes_and_instruments_survive_saving() {
        let notes = vec![Note { key: 62, start: 1_000, len: 2_000, velocity: 0.75 }, Note { key: 36, start: 0, len: 10, velocity: 1.0 }];
        let (mut project, clip) = song(Instrument::Drums, notes);
        project.apply(Command::TrimClip { clip, offset: 500, len: 9_000 }).unwrap();
        let text = SavedProject::capture(&project, |_| None).to_text();
        assert!(text.contains("instrument=drums") && text.contains("notes start=4800") && text.contains("note key=36"));
        let (back, _) = SavedProject::parse(&text).unwrap().build(&[], RATE);
        let clip = back.clips().next().unwrap();
        assert_eq!(clip.source.name, "Melody");
        assert_eq!((clip.start, clip.offset, clip.len), (4_800, 500, 9_000));
        assert_eq!(clip.notes.as_deref().unwrap(), &project.clips().next().unwrap().notes.as_deref().unwrap()[..]);
        assert_eq!(back.tracks[0].instrument, Instrument::Drums);
        assert_eq!(heard(&back, 20_000), heard(&project, 20_000));
        let synth = Instrument::Synth(crate::instrument::Synth { attack: 0.02, ..Default::default() });
        let (with_synth, _) = song(synth, Vec::new());
        let text = SavedProject::capture(&with_synth, |_| None).to_text();
        let (back, _) = SavedProject::parse(&text).unwrap().build(&[], RATE);
        assert_eq!(back.tracks[0].instrument, synth);
    }

    #[test]
    fn notes_are_kept_in_order_and_bad_ones_dropped() {
        let notes = vec![
            Note { key: 64, start: 900, len: 10, velocity: 2.0 },
            Note { key: 60, start: 100, len: 10, velocity: 0.5 },
            Note { key: 61, start: 50, len: 0, velocity: 0.5 },
        ];
        let (project, _) = song(Instrument::default(), notes);
        let kept = project.clips().next().unwrap().notes.clone().unwrap();
        assert_eq!(kept.iter().map(|n| n.key).collect::<Vec<_>>(), [60, 64]);
        assert_eq!(kept[1].velocity, 1.0);
    }
}
