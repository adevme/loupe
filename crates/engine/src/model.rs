use std::sync::Arc;

use crate::source::Source;

pub type Frames = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TrackId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClipId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fade {
    pub len: Frames,
    pub curve: f32,
}

impl Fade {
    pub const NONE: Fade = Fade { len: 0, curve: 0.0 };

    pub fn level(&self, progress: f32) -> f32 {
        progress.clamp(0.0, 1.0).powf(4f32.powf(-self.curve))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    In,
    Out,
}

#[derive(Clone, Debug)]
pub struct Clip {
    pub id: ClipId,
    pub source: Arc<Source>,
    pub start: Frames,
    pub offset: Frames,
    pub len: Frames,
    pub gain: f32,
    pub muted: bool,
    pub fade_in: Fade,
    pub fade_out: Fade,
}

impl Clip {
    pub fn end(&self) -> Frames {
        self.start + self.len
    }

    pub fn fade_level(&self, frames_into_clip: Frames) -> f32 {
        let mut level = 1.0;
        if frames_into_clip < self.fade_in.len {
            level *= self.fade_in.level(frames_into_clip as f32 / self.fade_in.len as f32);
        }
        let frames_left = self.len.saturating_sub(frames_into_clip + 1);
        if frames_left < self.fade_out.len {
            level *= self.fade_out.level(frames_left as f32 / self.fade_out.len as f32);
        }
        level
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Send {
    pub to: TrackId,
    pub gain: f32,
    pub pre_fader: bool,
    pub sidechain: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fx {
    pub path: std::path::PathBuf,
    pub index: usize,
    pub name: String,
    pub bypassed: bool,
    pub state: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct Track {
    pub id: TrackId,
    pub name: String,
    pub gain: f32,
    pub muted: bool,
    pub colour: Option<[u8; 3]>,
    pub clips: Vec<Clip>,
    pub parent: Option<TrackId>,
    pub collapsed: bool,
    pub sends: Vec<Send>,
    pub fx: Vec<Fx>,
}

#[derive(Clone, Debug)]
pub struct Project {
    pub rate: u32,
    pub bpm: f64,
    pub master: f32,
    pub master_muted: bool,
    pub tracks: Vec<Track>,
    pub sources: Vec<Arc<Source>>,
    next_id: u64,
}

#[derive(Clone, Debug)]
pub enum Command {
    AddTrack { name: String },
    RemoveTrack(TrackId),
    RenameTrack { track: TrackId, name: String },
    SetTrackColour { track: TrackId, colour: Option<[u8; 3]> },
    DuplicateTrack(TrackId),
    AddSource(Arc<Source>),
    SetTrackGain { track: TrackId, gain: f32 },
    SetTrackMuted { track: TrackId, muted: bool },
    AddClip { track: TrackId, source: Arc<Source>, start: Frames },
    MoveClip { clip: ClipId, track: TrackId, start: Frames },
    SplitClip { clip: ClipId, at: Frames },
    DeleteClip(ClipId),
    TrimClip { clip: ClipId, offset: Frames, len: Frames },
    SetClipGain { clip: ClipId, gain: f32 },
    SetClipMuted { clip: ClipId, muted: bool },
    SetClipFade { clip: ClipId, edge: Edge, fade: Fade },
    SetBpm(f64),
    SetMasterGain(f32),
    ToggleMasterMute,
    SetTrackParent { track: TrackId, parent: Option<TrackId> },
    ToggleCollapsed(TrackId),
    AddSend { from: TrackId, to: TrackId },
    RemoveSend { from: TrackId, to: TrackId },
    SetSendGain { from: TrackId, to: TrackId, gain: f32 },
    SetSendPreFader { from: TrackId, to: TrackId, pre_fader: bool },
    SetSendSidechain { from: TrackId, to: TrackId, sidechain: bool },
    AddFx { track: TrackId, fx: Fx },
    RemoveFx { track: TrackId, slot: usize },
    MoveFx { track: TrackId, slot: usize, to: usize },
    BypassFx { track: TrackId, slot: usize, bypassed: bool },
    SetFxState { track: TrackId, slot: usize, state: Vec<u8> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Track(TrackId),
    Clip(ClipId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandError {
    NoSuchTrack,
    NoSuchClip,
    SplitOutsideClip,
    InvalidValue,
}

pub const MIN_BPM: f64 = 20.0;
pub const MAX_BPM: f64 = 999.0;

impl Project {
    pub fn new(rate: u32) -> Self {
        Self { rate, bpm: 120.0, master: 1.0, master_muted: false, tracks: Vec::new(), sources: Vec::new(), next_id: 1 }
    }

    pub fn length(&self) -> Frames {
        self.clips().map(Clip::end).max().unwrap_or(0)
    }

    pub fn clips(&self) -> impl Iterator<Item = &Clip> {
        self.tracks.iter().flat_map(|t| t.clips.iter())
    }

    pub fn clip(&self, id: ClipId) -> Option<&Clip> {
        self.clips().find(|c| c.id == id)
    }

    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    pub fn track_of(&self, clip: ClipId) -> Option<&Track> {
        self.tracks.iter().find(|t| t.clips.iter().any(|c| c.id == clip))
    }

    pub fn apply(&mut self, command: Command) -> Result<Outcome, CommandError> {
        match command {
            Command::AddTrack { name } => {
                let id = TrackId(self.fresh());
                self.tracks.push(Track { id, name, gain: 1.0, muted: false, colour: None, clips: Vec::new(), parent: None, collapsed: false, sends: Vec::new(), fx: Vec::new() });
                Ok(Outcome::Track(id))
            }
            Command::RemoveTrack(track) => {
                let t = self.track_index(track)?;
                let parent = self.tracks[t].parent;
                self.tracks.remove(t);
                for other in &mut self.tracks {
                    if other.parent == Some(track) {
                        other.parent = parent;
                    }
                    other.sends.retain(|send| send.to != track);
                }
                Ok(Outcome::Done)
            }
            Command::RenameTrack { track, name } => {
                let t = self.track_index(track)?;
                let name = name.trim();
                if name.is_empty() {
                    return Err(CommandError::InvalidValue);
                }
                self.tracks[t].name = name.to_string();
                Ok(Outcome::Done)
            }
            Command::SetTrackColour { track, colour } => {
                let t = self.track_index(track)?;
                self.tracks[t].colour = colour;
                Ok(Outcome::Done)
            }
            Command::DuplicateTrack(track) => {
                let t = self.track_index(track)?;
                let mut copy = self.tracks[t].clone();
                copy.id = TrackId(self.fresh());
                for clip in &mut copy.clips {
                    clip.id = ClipId(self.fresh());
                }
                let id = copy.id;
                self.tracks.insert(t + 1, copy);
                Ok(Outcome::Track(id))
            }
            Command::AddSource(source) => {
                self.keep(&source);
                Ok(Outcome::Done)
            }
            Command::SetTrackGain { track, gain } => {
                let t = self.track_index(track)?;
                self.tracks[t].gain = valid_gain(gain)?;
                Ok(Outcome::Done)
            }
            Command::SetTrackMuted { track, muted } => {
                let t = self.track_index(track)?;
                self.tracks[t].muted = muted;
                Ok(Outcome::Done)
            }
            Command::AddClip { track, source, start } => {
                let t = self.track_index(track)?;
                let id = ClipId(self.fresh());
                let len = source.frames.len() as Frames;
                self.keep(&source);
                self.tracks[t].clips.push(Clip {
                    id,
                    source,
                    start,
                    offset: 0,
                    len,
                    gain: 1.0,
                    muted: false,
                    fade_in: Fade::NONE,
                    fade_out: Fade::NONE,
                });
                Ok(Outcome::Clip(id))
            }
            Command::MoveClip { clip, track, start } => {
                let to = self.track_index(track)?;
                let (from, i) = self.locate(clip)?;
                if from == to {
                    self.tracks[from].clips[i].start = start;
                } else {
                    let mut moved = self.tracks[from].clips.remove(i);
                    moved.start = start;
                    self.tracks[to].clips.push(moved);
                }
                Ok(Outcome::Done)
            }
            Command::SplitClip { clip, at } => {
                let (t, i) = self.locate(clip)?;
                let id = ClipId(self.fresh());
                let left = &mut self.tracks[t].clips[i];
                if at <= left.start || at >= left.end() {
                    return Err(CommandError::SplitOutsideClip);
                }
                let cut = at - left.start;
                let mut right = left.clone();
                right.id = id;
                right.start = at;
                right.offset += cut;
                right.len -= cut;
                right.fade_in = Fade::NONE;
                right.fade_out.len = right.fade_out.len.min(right.len);
                left.len = cut;
                left.fade_out = Fade::NONE;
                left.fade_in.len = left.fade_in.len.min(left.len);
                self.tracks[t].clips.insert(i + 1, right);
                Ok(Outcome::Clip(id))
            }
            Command::DeleteClip(clip) => {
                let (t, i) = self.locate(clip)?;
                self.tracks[t].clips.remove(i);
                Ok(Outcome::Done)
            }
            Command::TrimClip { clip, offset, len } => {
                let (t, i) = self.locate(clip)?;
                let target = &mut self.tracks[t].clips[i];
                let available = target.source.frames.len() as Frames;
                let len = if available == 0 { len } else { len.min(available.saturating_sub(offset)) };
                if len == 0 {
                    return Err(CommandError::InvalidValue);
                }
                target.offset = offset;
                target.len = len;
                target.fade_in.len = target.fade_in.len.min(len);
                target.fade_out.len = target.fade_out.len.min(len - target.fade_in.len);
                Ok(Outcome::Done)
            }
            Command::SetClipGain { clip, gain } => {
                let (t, i) = self.locate(clip)?;
                self.tracks[t].clips[i].gain = valid_gain(gain)?;
                Ok(Outcome::Done)
            }
            Command::SetClipMuted { clip, muted } => {
                let (t, i) = self.locate(clip)?;
                self.tracks[t].clips[i].muted = muted;
                Ok(Outcome::Done)
            }
            Command::SetClipFade { clip, edge, fade } => {
                let (t, i) = self.locate(clip)?;
                let target = &mut self.tracks[t].clips[i];
                let other = match edge {
                    Edge::In => target.fade_out.len,
                    Edge::Out => target.fade_in.len,
                };
                let curve_is_valid = fade.curve.is_finite() && (-1.0..=1.0).contains(&fade.curve);
                if !curve_is_valid || fade.len + other > target.len {
                    return Err(CommandError::InvalidValue);
                }
                match edge {
                    Edge::In => target.fade_in = fade,
                    Edge::Out => target.fade_out = fade,
                }
                Ok(Outcome::Done)
            }
            Command::SetMasterGain(gain) => {
                self.master = valid_gain(gain)?;
                Ok(Outcome::Done)
            }
            Command::ToggleMasterMute => {
                self.master_muted = !self.master_muted;
                Ok(Outcome::Done)
            }
            Command::SetTrackParent { track, parent } => {
                let t = self.track_index(track)?;
                if let Some(parent) = parent {
                    self.track_index(parent)?;
                    if parent == track || self.descends_from(parent, track) {
                        return Err(CommandError::InvalidValue);
                    }
                }
                self.tracks[t].parent = parent;
                Ok(Outcome::Done)
            }
            Command::ToggleCollapsed(track) => {
                let t = self.track_index(track)?;
                self.tracks[t].collapsed = !self.tracks[t].collapsed;
                Ok(Outcome::Done)
            }
            Command::AddSend { from, to } => {
                let f = self.track_index(from)?;
                self.track_index(to)?;
                if from == to {
                    return Err(CommandError::InvalidValue);
                }
                if self.tracks[f].sends.iter().any(|send| send.to == to) {
                    return Err(CommandError::InvalidValue);
                }
                self.tracks[f].sends.push(Send { to, gain: 1.0, pre_fader: false, sidechain: false });
                if self.feeds_back() {
                    self.tracks[f].sends.pop();
                    return Err(CommandError::InvalidValue);
                }
                Ok(Outcome::Done)
            }
            Command::RemoveSend { from, to } => {
                let f = self.track_index(from)?;
                self.tracks[f].sends.retain(|send| send.to != to);
                Ok(Outcome::Done)
            }
            Command::SetSendGain { from, to, gain } => {
                let gain = valid_gain(gain)?;
                let f = self.track_index(from)?;
                let send = self.tracks[f].sends.iter_mut().find(|send| send.to == to).ok_or(CommandError::NoSuchTrack)?;
                send.gain = gain;
                Ok(Outcome::Done)
            }
            Command::SetSendPreFader { from, to, pre_fader } => {
                let f = self.track_index(from)?;
                let send = self.tracks[f].sends.iter_mut().find(|send| send.to == to).ok_or(CommandError::NoSuchTrack)?;
                send.pre_fader = pre_fader;
                Ok(Outcome::Done)
            }
            Command::SetSendSidechain { from, to, sidechain } => {
                let f = self.track_index(from)?;
                let send = self.tracks[f].sends.iter_mut().find(|send| send.to == to).ok_or(CommandError::NoSuchTrack)?;
                send.sidechain = sidechain;
                Ok(Outcome::Done)
            }
            Command::AddFx { track, fx } => {
                let t = self.track_index(track)?;
                self.tracks[t].fx.push(fx);
                Ok(Outcome::Done)
            }
            Command::RemoveFx { track, slot } => {
                let t = self.track_index(track)?;
                if slot >= self.tracks[t].fx.len() {
                    return Err(CommandError::InvalidValue);
                }
                self.tracks[t].fx.remove(slot);
                Ok(Outcome::Done)
            }
            Command::MoveFx { track, slot, to } => {
                let t = self.track_index(track)?;
                let chain = &mut self.tracks[t].fx;
                if slot >= chain.len() || to >= chain.len() {
                    return Err(CommandError::InvalidValue);
                }
                let moved = chain.remove(slot);
                chain.insert(to, moved);
                Ok(Outcome::Done)
            }
            Command::BypassFx { track, slot, bypassed } => {
                let t = self.track_index(track)?;
                let fx = self.tracks[t].fx.get_mut(slot).ok_or(CommandError::InvalidValue)?;
                fx.bypassed = bypassed;
                Ok(Outcome::Done)
            }
            Command::SetFxState { track, slot, state } => {
                let t = self.track_index(track)?;
                let fx = self.tracks[t].fx.get_mut(slot).ok_or(CommandError::InvalidValue)?;
                fx.state = state;
                Ok(Outcome::Done)
            }
            Command::SetBpm(bpm) => {
                if !(MIN_BPM..=MAX_BPM).contains(&bpm) {
                    return Err(CommandError::InvalidValue);
                }
                self.bpm = bpm;
                Ok(Outcome::Done)
            }
        }
    }

    fn keep(&mut self, source: &Arc<Source>) {
        if !self.sources.iter().any(|kept| Arc::ptr_eq(kept, source)) {
            self.sources.push(source.clone());
        }
    }

    fn fresh(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn depth_of(&self, track: TrackId) -> usize {
        let mut depth = 0;
        let mut at = self.tracks.iter().find(|t| t.id == track).and_then(|t| t.parent);
        while let Some(id) = at {
            depth += 1;
            if depth > self.tracks.len() {
                break;
            }
            at = self.tracks.iter().find(|t| t.id == id).and_then(|t| t.parent);
        }
        depth
    }

    pub fn descends_from(&self, track: TrackId, ancestor: TrackId) -> bool {
        let mut at = self.tracks.iter().find(|t| t.id == track).and_then(|t| t.parent);
        let mut steps = 0;
        while let Some(id) = at {
            if id == ancestor {
                return true;
            }
            steps += 1;
            if steps > self.tracks.len() {
                return true;
            }
            at = self.tracks.iter().find(|t| t.id == id).and_then(|t| t.parent);
        }
        false
    }

    pub fn feeds_back(&self) -> bool {
        self.render_order().is_none()
    }

    pub fn render_order(&self) -> Option<Vec<TrackId>> {
        let mut waiting: Vec<TrackId> = self.tracks.iter().map(|t| t.id).collect();
        let mut done: Vec<TrackId> = Vec::with_capacity(waiting.len());
        while !waiting.is_empty() {
            let ready: Vec<TrackId> = waiting
                .iter()
                .copied()
                .filter(|id| {
                    let track = match self.tracks.iter().find(|t| t.id == *id) {
                        Some(track) => track,
                        None => return true,
                    };
                    let _ = track;
                    !self.tracks.iter().any(|other| {
                        other.id != *id
                            && waiting.contains(&other.id)
                            && (other.parent == Some(*id) || other.sends.iter().any(|s| s.to == *id))
                    })
                })
                .collect();
            if ready.is_empty() {
                return None;
            }
            waiting.retain(|id| !ready.contains(id));
            done.extend(ready);
        }
        Some(done)
    }

    fn track_index(&self, id: TrackId) -> Result<usize, CommandError> {
        self.tracks.iter().position(|t| t.id == id).ok_or(CommandError::NoSuchTrack)
    }

    fn locate(&self, id: ClipId) -> Result<(usize, usize), CommandError> {
        for (t, track) in self.tracks.iter().enumerate() {
            if let Some(i) = track.clips.iter().position(|c| c.id == id) {
                return Ok((t, i));
            }
        }
        Err(CommandError::NoSuchClip)
    }
}

fn valid_gain(gain: f32) -> Result<f32, CommandError> {
    if gain.is_finite() && gain >= 0.0 {
        Ok(gain)
    } else {
        Err(CommandError::InvalidValue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_with_clip(len: usize) -> (Project, TrackId, ClipId) {
        let mut p = Project::new(48_000);
        let Ok(Outcome::Track(track)) = p.apply(Command::AddTrack { name: "Vox".into() }) else {
            panic!("no track")
        };
        let source = Arc::new(Source::from_frames("ramp", vec![[0.5, 0.5]; len]));
        let Ok(Outcome::Clip(clip)) = p.apply(Command::AddClip { track, source, start: 100 }) else {
            panic!("no clip")
        };
        (p, track, clip)
    }

    #[test]
    fn split_keeps_every_frame_once() {
        let (mut p, _, clip) = project_with_clip(1000);
        let Ok(Outcome::Clip(right)) = p.apply(Command::SplitClip { clip, at: 400 }) else {
            panic!("no split")
        };
        let left = p.clip(clip).unwrap();
        let right = p.clip(right).unwrap();
        assert_eq!((left.start, left.offset, left.len), (100, 0, 300));
        assert_eq!((right.start, right.offset, right.len), (400, 300, 700));
    }

    #[test]
    fn split_outside_the_clip_is_refused() {
        let (mut p, _, clip) = project_with_clip(1000);
        for at in [0, 100, 1100, 5000] {
            assert_eq!(
                p.apply(Command::SplitClip { clip, at }).unwrap_err(),
                CommandError::SplitOutsideClip
            );
        }
        assert_eq!(p.clips().count(), 1);
    }

    #[test]
    fn move_between_tracks_keeps_the_clip() {
        let (mut p, first, clip) = project_with_clip(1000);
        let Ok(Outcome::Track(second)) = p.apply(Command::AddTrack { name: "Beat".into() }) else {
            panic!("no track")
        };
        p.apply(Command::MoveClip { clip, track: second, start: 7 }).unwrap();
        assert!(p.track(first).unwrap().clips.is_empty());
        assert_eq!(p.track_of(clip).unwrap().id, second);
        assert_eq!(p.clip(clip).unwrap().start, 7);
    }

    #[test]
    fn bad_values_are_refused() {
        let (mut p, track, clip) = project_with_clip(10);
        assert!(p.apply(Command::SetClipGain { clip, gain: f32::NAN }).is_err());
        assert!(p.apply(Command::SetTrackGain { track, gain: -1.0 }).is_err());
        assert!(p.apply(Command::SetBpm(0.0)).is_err());
        assert!(p.apply(Command::DeleteClip(ClipId(999))).is_err());
    }
}
