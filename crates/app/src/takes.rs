use loupe_engine::{ClipId, Command, CommandError, Edge, Fade, Frames, Outcome, Project, TrackId};

use crate::App;

const COMP_FADE_SECONDS: f64 = 0.004;

impl App {
    pub(crate) fn comp(&mut self, track: TrackId, take: usize, from: Frames, to: Frames) {
        let fade = Fade { len: (COMP_FADE_SECONDS * self.project.rate as f64) as Frames, curve: 0.0 };
        let mut chosen = Vec::new();
        self.transact(None, |project| {
            chosen = comp(project, track, take, from, to, fade)?;
            Ok(Outcome::Done)
        });
        if chosen.is_empty() {
            self.notice = Some("Nothing to comp there.".into());
        }
    }
}

pub fn comp(project: &mut Project, track: TrackId, take: usize, from: Frames, to: Frames, fade: Fade) -> Result<Vec<ClipId>, CommandError> {
    let touched: Vec<ClipId> = project
        .track(track)
        .ok_or(CommandError::NoSuchTrack)?
        .clips
        .iter()
        .filter(|clip| clip.takes.len() > take && clip.start < to && clip.end() > from)
        .map(|clip| clip.id)
        .collect();
    let mut chosen = Vec::new();
    for clip in touched {
        let Some(found) = project.clip(clip) else { continue };
        let (start, end) = (found.start, found.end());
        let mut middle = clip;
        if start < from {
            if let Outcome::Clip(right) = project.apply(Command::SplitClip { clip, at: from })? {
                project.apply(Command::SetClipFade { clip, edge: Edge::Out, fade })?;
                project.apply(Command::SetClipFade { clip: right, edge: Edge::In, fade })?;
                middle = right;
            }
        }
        if end > to {
            if let Outcome::Clip(right) = project.apply(Command::SplitClip { clip: middle, at: to })? {
                project.apply(Command::SetClipFade { clip: middle, edge: Edge::Out, fade })?;
                project.apply(Command::SetClipFade { clip: right, edge: Edge::In, fade })?;
            }
        }
        if project.apply(Command::UseTake { clip: middle, take }).is_ok() {
            chosen.push(middle);
        }
    }
    Ok(chosen)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use loupe_engine::Source;

    use super::*;

    #[test]
    fn a_swipe_cuts_the_clip_and_plays_that_take_between_the_cuts() {
        let mut project = Project::new(48_000);
        let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: "Vocal".into() }) else { panic!() };
        let passes: Vec<[f32; 2]> = (0..3_000).map(|at| [(at / 1_000) as f32, 0.0]).collect();
        let source = Arc::new(Source::from_frames("Vocal (take 1)", passes));
        let Ok(Outcome::Clip(clip)) = project.apply(Command::AddClip { track, source, start: 0 }) else { panic!() };
        project.apply(Command::TrimClip { clip, offset: 2_000, len: 1_000 }).unwrap();
        project.apply(Command::SetTakes { clip, offsets: vec![0, 1_000, 2_000], active: 2 }).unwrap();
        let fade = Fade { len: 4, curve: 0.0 };
        let chosen = comp(&mut project, track, 0, 300, 600, fade).unwrap();
        assert_eq!(chosen.len(), 1);
        let pieces = &project.track(track).unwrap().clips;
        let mut spans: Vec<(Frames, Frames, usize)> = pieces.iter().map(|piece| (piece.start, piece.end(), piece.take)).collect();
        spans.sort();
        assert_eq!(spans, vec![(0, 300, 2), (300, 600, 0), (600, 1_000, 2)]);
        let middle = project.clip(chosen[0]).unwrap();
        assert_eq!(middle.audio()[middle.offset as usize][0], 0.0, "the first pass plays in the middle");
        assert_eq!((middle.fade_in.len, middle.fade_out.len), (4, 4));
        let again = comp(&mut project, track, 1, 0, 1_000, fade).unwrap();
        assert_eq!(again.len(), 3, "a swipe across the pieces switches them all");
        assert!(project.track(track).unwrap().clips.iter().all(|piece| piece.take == 1));
        assert!(comp(&mut project, track, 9, 0, 1_000, fade).unwrap().is_empty());
    }
}
