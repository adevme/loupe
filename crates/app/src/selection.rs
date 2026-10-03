use loupe_engine::{ClipId, Command, Frames, Outcome, TrackId};

use crate::{App, Run};

impl App {
    pub(crate) fn choose(&mut self, clips: impl IntoIterator<Item = ClipId>) {
        self.selection = clips.into_iter().collect();
        self.selected = if self.selection.len() == 1 { self.selection.iter().next().copied() } else { None };
        self.cache.clear();
    }

    pub(crate) fn forget_gone_clips(&mut self) {
        let still_here: Vec<ClipId> =
            self.selection.iter().copied().filter(|clip| self.project.clip(*clip).is_some()).collect();
        if still_here.len() != self.selection.len() {
            self.choose(still_here);
        }
    }

    pub(crate) fn affected_by(&self, clip: ClipId) -> Vec<ClipId> {
        if self.selection.len() > 1 && self.selection.contains(&clip) {
            self.in_song_order()
        } else {
            vec![clip]
        }
    }

    fn in_song_order(&self) -> Vec<ClipId> {
        let mut clips: Vec<(Frames, ClipId)> = self
            .selection
            .iter()
            .filter_map(|id| self.project.clip(*id).map(|clip| (clip.start, clip.id)))
            .collect();
        clips.sort_by_key(|(start, _)| *start);
        clips.into_iter().map(|(_, id)| id).collect()
    }

    pub(crate) fn delete_clips(&mut self, clips: Vec<ClipId>, run: Option<Run>) {
        self.transact(run, |project| {
            for clip in clips {
                project.apply(Command::DeleteClip(clip))?;
            }
            Ok(Outcome::Done)
        });
        self.forget_gone_clips();
    }

    pub(crate) fn mute_clips(&mut self, clips: Vec<ClipId>, muted: bool) {
        self.transact(Some(Run::Paint), |project| {
            for clip in clips {
                project.apply(Command::SetClipMuted { clip, muted })?;
            }
            Ok(Outcome::Done)
        });
    }

    pub(crate) fn move_clips(&mut self, lead: ClipId, track: TrackId, start: Frames) {
        let group = self.affected_by(lead);
        let row_of = |wanted: TrackId| self.project.tracks.iter().position(|t| t.id == wanted);
        let placed: Vec<(ClipId, usize, Frames)> = group
            .iter()
            .filter_map(|id| {
                let clip = self.project.clip(*id)?;
                let row = row_of(self.project.track_of(*id)?.id)?;
                Some((*id, row, clip.start))
            })
            .collect();
        let (Some(lead_at), Some(to_row)) = (placed.iter().find(|(id, _, _)| *id == lead), row_of(track)) else {
            return;
        };
        let earliest = placed.iter().map(|(_, _, at)| *at).min().unwrap_or(0) as i64;
        let highest = placed.iter().map(|(_, row, _)| *row).min().unwrap_or(0) as i64;
        let lowest = placed.iter().map(|(_, row, _)| *row).max().unwrap_or(0) as i64;
        let last_row = self.project.tracks.len() as i64 - 1;
        let later = (start as i64 - lead_at.2 as i64).max(-earliest);
        let down = (to_row as i64 - lead_at.1 as i64).clamp(-highest, last_row - lowest);
        if later == 0 && down == 0 {
            return;
        }
        let moves: Vec<Command> = placed
            .iter()
            .map(|(clip, row, at)| Command::MoveClip {
                clip: *clip,
                track: self.project.tracks[(*row as i64 + down) as usize].id,
                start: (*at as i64 + later) as Frames,
            })
            .collect();
        self.transact(Some(Run::Move(lead)), |project| {
            for step in moves {
                project.apply(step)?;
            }
            Ok(Outcome::Done)
        });
    }

}
