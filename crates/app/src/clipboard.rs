use loupe_engine::{Clip, ClipId, Command, Frames, Outcome, Project, TrackId};

use crate::App;

#[derive(Clone, Debug)]
pub struct Copied {
    clips: Vec<(usize, Clip)>,
    top: TrackId,
    span: Frames,
}

impl Copied {
    pub fn take(project: &Project, chosen: impl IntoIterator<Item = ClipId>) -> Option<Self> {
        let row_of = |wanted: TrackId| project.tracks.iter().position(|track| track.id == wanted);
        let mut found: Vec<(usize, Clip)> = chosen
            .into_iter()
            .filter_map(|id| Some((row_of(project.track_of(id)?.id)?, project.clip(id)?.clone())))
            .collect();
        let earliest = found.iter().map(|(_, clip)| clip.start).min()?;
        let highest = found.iter().map(|(row, _)| *row).min()?;
        let span = found.iter().map(|(_, clip)| clip.end()).max()? - earliest;
        for (row, clip) in &mut found {
            *row -= highest;
            clip.start -= earliest;
        }
        found.sort_by_key(|(row, clip)| (*row, clip.start));
        Some(Self { clips: found, top: project.tracks[highest].id, span })
    }

    pub fn place(&self, project: &mut Project, top: TrackId, at: Frames) -> Vec<ClipId> {
        let Some(first_row) = project.tracks.iter().position(|track| track.id == top) else {
            return Vec::new();
        };
        let mut placed = Vec::new();
        for (row, clip) in &self.clips {
            let Some(track) = project.tracks.get(first_row + row).map(|track| track.id) else {
                continue;
            };
            let mut copy = clip.clone();
            copy.start += at;
            if let Ok(Outcome::Clip(id)) = project.apply(Command::PasteClip { track, clip: copy }) {
                placed.push(id);
            }
        }
        placed
    }
}

impl App {
    pub(crate) fn copy_clips(&mut self) -> bool {
        if self.selection.is_empty() {
            return false;
        }
        let with_effects = self.selection.iter().any(|id| self.project.clip(*id).is_some_and(|clip| !clip.fx.is_empty()));
        if with_effects {
            self.gather_fx_state();
        }
        self.copied_clips = Copied::take(&self.project, self.selection.iter().copied());
        if let Some(copied) = &self.copied_clips {
            let count = copied.clips.len();
            self.notice = Some(format!("Copied {count} clip{}.", if count == 1 { "" } else { "s" }));
        }
        self.copied_clips.is_some()
    }

    pub(crate) fn cut_clips(&mut self) {
        if self.copy_clips() {
            let chosen: Vec<ClipId> = self.selection.iter().copied().collect();
            self.delete_clips(chosen, None);
            self.notice = None;
        }
    }

    pub(crate) fn paste_clips(&mut self) {
        let Some(copied) = self.copied_clips.clone() else {
            self.notice = Some("Nothing copied yet. Select clips and press Ctrl+C.".into());
            return;
        };
        let top = self.paste_onto(&copied);
        let Some(top) = top else {
            self.notice = Some("Add a track to paste onto.".into());
            return;
        };
        let at = self.playhead;
        self.paste_at(&copied, top, at);
        if !self.playing && self.recording.is_none() {
            self.seek(at + copied.span);
        }
    }

    pub(crate) fn duplicate_clips(&mut self) {
        let Some(copied) = Copied::take(&self.project, self.selection.iter().copied()) else {
            return;
        };
        let after = self.selection.iter().filter_map(|id| self.project.clip(*id)).map(|clip| clip.start).min().unwrap_or(0) + copied.span;
        let top = copied.top;
        self.paste_at(&copied, top, after);
    }

    fn paste_onto(&self, copied: &Copied) -> Option<TrackId> {
        let row_of = |wanted: TrackId| self.project.tracks.iter().position(|track| track.id == wanted);
        let chosen_top = self.selection.iter().filter_map(|id| self.project.track_of(*id)).filter_map(|track| row_of(track.id)).min();
        chosen_top
            .map(|row| self.project.tracks[row].id)
            .or_else(|| row_of(copied.top).map(|_| copied.top))
            .or_else(|| self.project.tracks.first().map(|track| track.id))
    }

    fn paste_at(&mut self, copied: &Copied, top: TrackId, at: Frames) {
        let mut placed = Vec::new();
        self.transact(None, |project| {
            placed = copied.place(project, top, at);
            if placed.is_empty() {
                return Err(loupe_engine::CommandError::NoSuchTrack);
            }
            Ok(Outcome::Done)
        });
        if placed.is_empty() {
            return;
        }
        self.notice = None;
        self.choose(placed);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use loupe_engine::Source;

    use super::*;

    fn song() -> (Project, Vec<TrackId>, Vec<ClipId>) {
        let mut project = Project::new(1000);
        let mut tracks = Vec::new();
        for name in ["A", "B", "C"] {
            let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: name.into() }) else { panic!() };
            tracks.push(track);
        }
        let source = Arc::new(Source::from_frames("beat", vec![[0.1, 0.1]; 400]));
        let mut clips = Vec::new();
        for (track, start) in [(tracks[1], 1_000), (tracks[2], 1_200)] {
            let Ok(Outcome::Clip(clip)) = project.apply(Command::AddClip { track, source: source.clone(), start }) else { panic!() };
            clips.push(clip);
        }
        (project, tracks, clips)
    }

    #[test]
    fn copies_keep_their_spacing_and_rows_wherever_they_land() {
        let (mut project, tracks, clips) = song();
        let copied = Copied::take(&project, clips.clone()).unwrap();
        assert_eq!(copied.span, 600);
        assert_eq!(copied.top, tracks[1]);
        let placed = copied.place(&mut project, tracks[0], 5_000);
        assert_eq!(placed.len(), 2);
        let first = project.clip(placed[0]).unwrap();
        let second = project.clip(placed[1]).unwrap();
        assert_eq!((project.track_of(placed[0]).unwrap().id, first.start), (tracks[0], 5_000));
        assert_eq!((project.track_of(placed[1]).unwrap().id, second.start), (tracks[1], 5_200));
        assert!(clips.iter().all(|clip| project.clip(*clip).is_some()), "the originals stay");
    }

    #[test]
    fn rows_past_the_last_track_are_left_out_and_nothing_chosen_copies_nothing() {
        let (mut project, tracks, clips) = song();
        let copied = Copied::take(&project, clips).unwrap();
        assert_eq!(copied.place(&mut project, tracks[2], 0).len(), 1);
        assert!(Copied::take(&project, Vec::new()).is_none());
    }
}
