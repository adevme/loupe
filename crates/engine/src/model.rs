use std::sync::Arc;

use crate::source::Source;

pub type Frames = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TrackId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClipId(pub u64);

#[derive(Clone, Debug)]
pub struct Clip {
    pub id: ClipId,
    pub source: Arc<Source>,
    pub start: Frames,
    pub offset: Frames,
    pub len: Frames,
    pub gain: f32,
}

impl Clip {
    pub fn end(&self) -> Frames {
        self.start + self.len
    }
}

#[derive(Clone, Debug)]
pub struct Track {
    pub id: TrackId,
    pub name: String,
    pub gain: f32,
    pub muted: bool,
    pub clips: Vec<Clip>,
}

#[derive(Clone, Debug)]
pub struct Project {
    pub rate: u32,
    pub bpm: f64,
    pub tracks: Vec<Track>,
    next_id: u64,
}

#[derive(Clone, Debug)]
pub enum Command {
    AddTrack { name: String },
    RemoveTrack(TrackId),
    SetTrackGain { track: TrackId, gain: f32 },
    SetTrackMuted { track: TrackId, muted: bool },
    AddClip { track: TrackId, source: Arc<Source>, start: Frames },
    MoveClip { clip: ClipId, track: TrackId, start: Frames },
    SplitClip { clip: ClipId, at: Frames },
    DeleteClip(ClipId),
    SetClipGain { clip: ClipId, gain: f32 },
    SetBpm(f64),
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
        Self { rate, bpm: 120.0, tracks: Vec::new(), next_id: 1 }
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
                self.tracks.push(Track { id, name, gain: 1.0, muted: false, clips: Vec::new() });
                Ok(Outcome::Track(id))
            }
            Command::RemoveTrack(track) => {
                let t = self.track_index(track)?;
                self.tracks.remove(t);
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
                self.tracks[t].clips.push(Clip { id, source, start, offset: 0, len, gain: 1.0 });
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
                left.len = cut;
                self.tracks[t].clips.insert(i + 1, right);
                Ok(Outcome::Clip(id))
            }
            Command::DeleteClip(clip) => {
                let (t, i) = self.locate(clip)?;
                self.tracks[t].clips.remove(i);
                Ok(Outcome::Done)
            }
            Command::SetClipGain { clip, gain } => {
                let (t, i) = self.locate(clip)?;
                self.tracks[t].clips[i].gain = valid_gain(gain)?;
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

    fn fresh(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
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
