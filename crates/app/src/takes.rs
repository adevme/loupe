use loupe_engine::{Clip, ClipId, Command, CommandError, Edge, Fade, Frames, Outcome, Project, Source, Track, TrackId};
use std::sync::Arc;

use crate::App;

const COMP_FADE_SECONDS: f64 = 0.004;

impl App {
    pub(crate) fn comp(&mut self, track: TrackId, take: usize, from: Frames, to: Frames) {
        let fade = short_fade(self.project.rate);
        let mut chosen = Vec::new();
        self.transact(None, |project| {
            chosen = comp(project, track, take, from, to, fade)?;
            Ok(Outcome::Done)
        });
        if chosen.is_empty() {
            self.notice = Some("Nothing to comp there.".into());
        }
    }

    pub(crate) fn use_clip_take(&mut self, clip: ClipId) {
        let fade = short_fade(self.project.rate);
        if self.project.clip(clip).is_some_and(|found| found.lane.is_some() && !found.lane_on) {
            self.transact(None, |project| {
                sound_only(project, clip, fade)?;
                Ok(Outcome::Done)
            });
        }
    }

    pub(crate) fn use_take_on_track(&mut self, track: TrackId, take: usize) {
        let fade = short_fade(self.project.rate);
        let waiting = self.project.track(track).is_some_and(|found| {
            found.clips.iter().any(|clip| clip.lane.is_some() && clip.lane_on != (clip.lane == Some(take)))
        });
        if waiting {
            self.transact(None, |project| {
                use_take_on_track(project, track, take, fade)?;
                Ok(Outcome::Done)
            });
        }
    }

    pub(crate) fn delete_take(&mut self, track: TrackId, take: usize) {
        let fade = short_fade(self.project.rate);
        let there = self.project.track(track).is_some_and(|found| found.clips.iter().any(|clip| clip.lane == Some(take)));
        if there {
            self.transact(None, |project| {
                delete_take(project, track, take, fade)?;
                Ok(Outcome::Done)
            });
            self.forget_gone_clips();
            self.notice = Some(format!("Deleted take {}. Ctrl+Z brings it back.", take + 1));
        }
    }

    pub(crate) fn toggle_takes(&mut self, track: TrackId) {
        let Some(open) = self.project.track(track).map(|found| !found.takes_open) else { return };
        self.edit(None, Command::SetTakesOpen { track, open });
    }

    pub(crate) fn delete_unused_takes(&mut self, track: TrackId) {
        let unused = self.project.track(track).map_or(0, unused_takes);
        if unused == 0 {
            self.notice = Some("Every take is used by the comp.".into());
            return;
        }
        let mut removed = 0;
        self.transact(None, |project| {
            removed = delete_unused_takes(project, track)?;
            Ok(Outcome::Done)
        });
        self.forget_gone_clips();
        let takes = if removed == 1 { "take" } else { "takes" };
        self.notice = Some(format!("Deleted {removed} unused {takes}. The recordings stay in the Audio folder."));
    }

    pub(crate) fn pull_out_take(&mut self, track: TrackId, take: usize) {
        self.overlay = crate::Overlay::None;
        self.gather_fx_state();
        let height = self.heights.get(&track).copied();
        let mut made = None;
        self.transact(None, |project| {
            made = Some(pull_out_take(project, track, take)?);
            Ok(Outcome::Done)
        });
        match made {
            Some(copy) => {
                if let Some(height) = height {
                    self.heights.insert(copy, height);
                }
                self.forget_gone_clips();
                self.notice = Some(format!("Take {} is on its own track. The song sounds the same, and Ctrl+Z puts it back.", take + 1));
            }
            None => self.notice = Some("That take could not be moved.".into()),
        }
    }

    pub(crate) fn move_all_takes(&mut self, track: TrackId) {
        self.overlay = crate::Overlay::None;
        self.gather_fx_state();
        let height = self.heights.get(&track).copied();
        let mut made = Vec::new();
        let mut moved = false;
        self.transact(None, |project| {
            made = move_all_takes(project, track)?;
            moved = true;
            Ok(Outcome::Done)
        });
        if !moved {
            self.notice = Some("Those takes could not be moved.".into());
            return;
        }
        if let Some(height) = height {
            for copy in &made {
                self.heights.insert(*copy, height);
            }
        }
        self.forget_gone_clips();
        self.notice = Some(format!("{} takes are on their own tracks. The song sounds the same, and Ctrl+Z puts them back.", made.len() + 1));
    }
}

pub fn take_count(track: &Track) -> usize {
    let mut lanes: Vec<usize> = track.clips.iter().filter_map(|clip| clip.lane).collect();
    lanes.sort_unstable();
    lanes.dedup();
    lanes.len()
}

pub fn lane_rows(track: &Track) -> usize {
    track.clips.iter().filter_map(|clip| clip.lane).max().map_or(0, |last| last + 1)
}

pub fn lay_take(project: &mut Project, track: TrackId, source: Arc<Source>, start: Frames, len: Frames, offsets: &[i64], active: usize) -> Result<ClipId, CommandError> {
    let end = start + len;
    let found = project.track(track).ok_or(CommandError::NoSuchTrack)?;
    let under: Vec<Clip> = found.clips.iter().filter(|clip| !clip.is_notes() && clip.start < end && clip.end() > start).cloned().collect();
    let mut next = lane_rows(found);
    let plain: Vec<ClipId> = under.iter().filter(|clip| clip.lane.is_none()).map(|clip| clip.id).collect();
    let becomes_lanes = offsets.len() > 1 || !under.is_empty();
    // Only the part of an old clip that the new take covers moves under it.
    // What sits before or after the recording keeps playing as it was.
    if !plain.is_empty() {
        for clip in plain {
            let middle = middle_of(project, clip, start, end, Fade::NONE, false)?;
            project.apply(Command::SetClipLane { clip: middle, lane: Some(next), on: false })?;
        }
        next += 1;
    }
    for clip in under.iter().filter(|clip| clip.lane.is_some() && clip.lane_on) {
        let middle = middle_of(project, clip.id, start, end, Fade::NONE, false)?;
        project.apply(Command::SetClipLane { clip: middle, lane: clip.lane, on: false })?;
    }
    let mut playing = None;
    for (pass, offset) in offsets.iter().enumerate() {
        let Outcome::Clip(clip) = project.apply(Command::AddClip { track, source: source.clone(), start })? else {
            return Err(CommandError::NoSuchClip);
        };
        project.apply(Command::TrimClip { clip, offset: (*offset).max(0) as Frames, len })?;
        if becomes_lanes {
            project.apply(Command::SetClipLane { clip, lane: Some(next + pass), on: pass == active })?;
        }
        if pass == active {
            playing = Some(clip);
        }
    }
    playing.ok_or(CommandError::InvalidValue)
}

fn clear_lane(project: &mut Project, track: TrackId, take: usize) -> Result<(), CommandError> {
    let clips: Vec<Clip> = project.track(track).ok_or(CommandError::NoSuchTrack)?.clips.clone();
    for clip in clips {
        match clip.lane {
            Some(lane) if lane == take => {
                project.apply(Command::DeleteClip(clip.id))?;
            }
            Some(lane) if lane > take => {
                project.apply(Command::SetClipLane { clip: clip.id, lane: Some(lane - 1), on: clip.lane_on })?;
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn pull_out_take(project: &mut Project, track: TrackId, take: usize) -> Result<TrackId, CommandError> {
    let found = project.track(track).ok_or(CommandError::NoSuchTrack)?;
    let name = format!("{} take {}", found.name, take + 1);
    if !found.clips.iter().any(|clip| clip.lane == Some(take)) {
        return Err(CommandError::InvalidValue);
    }
    let Outcome::Track(copy) = project.apply(Command::DuplicateTrack(track))? else {
        return Err(CommandError::InvalidValue);
    };
    project.apply(Command::RenameTrack { track: copy, name })?;
    project.apply(Command::SetTakesOpen { track: copy, open: false })?;
    let copies: Vec<Clip> = project.track(copy).ok_or(CommandError::NoSuchTrack)?.clips.clone();
    for clip in copies {
        if clip.lane == Some(take) {
            project.apply(Command::SetClipMuted { clip: clip.id, muted: clip.is_silent() })?;
            project.apply(Command::SetClipLane { clip: clip.id, lane: None, on: false })?;
        } else {
            project.apply(Command::DeleteClip(clip.id))?;
        }
    }
    clear_lane(project, track, take)?;
    Ok(copy)
}

pub fn move_all_takes(project: &mut Project, track: TrackId) -> Result<Vec<TrackId>, CommandError> {
    let found = project.track(track).ok_or(CommandError::NoSuchTrack)?;
    if take_count(found) < 2 {
        return Err(CommandError::InvalidValue);
    }
    let rows = lane_rows(found);
    let first = found.clips.iter().filter_map(|clip| clip.lane).min().unwrap_or(0);
    let mut made = Vec::new();
    for take in (first + 1..rows).rev() {
        let there = project.track(track).ok_or(CommandError::NoSuchTrack)?.clips.iter().any(|clip| clip.lane == Some(take));
        if there {
            made.insert(0, pull_out_take(project, track, take)?);
        }
    }
    let left: Vec<Clip> = project.track(track).ok_or(CommandError::NoSuchTrack)?.clips.iter().filter(|clip| clip.lane.is_some()).cloned().collect();
    for clip in left {
        project.apply(Command::SetClipMuted { clip: clip.id, muted: clip.is_silent() })?;
        project.apply(Command::SetClipLane { clip: clip.id, lane: None, on: false })?;
    }
    project.apply(Command::SetTakesOpen { track, open: false })?;
    Ok(made)
}

fn used_lane(track: &Track, take: usize) -> bool {
    track.clips.iter().any(|clip| clip.lane == Some(take) && clip.lane_on)
}

pub fn unused_takes(track: &Track) -> usize {
    (0..lane_rows(track)).filter(|take| track.clips.iter().any(|clip| clip.lane == Some(*take)) && !used_lane(track, *take)).count()
}

pub fn delete_unused_takes(project: &mut Project, track: TrackId) -> Result<usize, CommandError> {
    let found = project.track(track).ok_or(CommandError::NoSuchTrack)?;
    let unused: Vec<usize> = (0..lane_rows(found)).filter(|take| found.clips.iter().any(|clip| clip.lane == Some(*take)) && !used_lane(found, *take)).collect();
    for take in unused.iter().rev() {
        clear_lane(project, track, *take)?;
    }
    Ok(unused.len())
}

pub fn delete_take(project: &mut Project, track: TrackId, take: usize, fade: Fade) -> Result<(), CommandError> {
    let spans: Vec<(Frames, Frames)> = project.track(track).ok_or(CommandError::NoSuchTrack)?.clips.iter().filter(|clip| clip.lane == Some(take) && clip.lane_on).map(|clip| (clip.start, clip.end())).collect();
    clear_lane(project, track, take)?;
    let fallback = take.saturating_sub(1);
    for (from, to) in spans {
        comp(project, track, fallback, from, to, fade)?;
    }
    Ok(())
}

pub fn use_take_on_track(project: &mut Project, track: TrackId, take: usize, fade: Fade) -> Result<(), CommandError> {
    // The picked take plays everywhere it has audio and every other take goes
    // quiet, even where the picked one is shorter, so one pick lights one take.
    let _ = fade;
    let lanes: Vec<(ClipId, Option<usize>, bool)> = project
        .track(track)
        .ok_or(CommandError::NoSuchTrack)?
        .clips
        .iter()
        .filter(|clip| clip.lane.is_some())
        .map(|clip| (clip.id, clip.lane, clip.lane == Some(take)))
        .collect();
    for (clip, lane, on) in lanes {
        project.apply(Command::SetClipLane { clip, lane, on })?;
    }
    rejoin_takes(project, track)
}

// Glues back pieces of one take that were cut apart but still sit end to end
// over the same recording, so a take that was comped and then picked whole is
// one clip again. The sound does not change.
fn rejoin_takes(project: &mut Project, track: TrackId) -> Result<(), CommandError> {
    loop {
        let found = project.track(track).ok_or(CommandError::NoSuchTrack)?;
        let mut pieces: Vec<&Clip> = found.clips.iter().filter(|clip| clip.lane.is_some()).collect();
        pieces.sort_by_key(|clip| (clip.lane, clip.start));
        let pair = pieces.windows(2).find_map(|two| {
            let (left, right) = (two[0], two[1]);
            let joins = left.lane == right.lane
                && left.lane_on == right.lane_on
                && Arc::ptr_eq(&left.source, &right.source)
                && right.start == left.end()
                && right.offset == left.offset + left.len
                && left.gain == right.gain
                && left.muted == right.muted
                && left.stretch == 1.0
                && right.stretch == 1.0
                && left.fx.is_empty()
                && right.fx.is_empty()
                && left.notes.is_none()
                && right.notes.is_none();
            joins.then(|| (left.id, left.offset, left.len + right.len, right.id, right.fade_out))
        });
        let Some((left, offset, len, right, fade_out)) = pair else { return Ok(()) };
        project.apply(Command::DeleteClip(right))?;
        project.apply(Command::TrimClip { clip: left, offset, len })?;
        project.apply(Command::SetClipFade { clip: left, edge: Edge::Out, fade: fade_out })?;
    }
}

fn middle_of(
    project: &mut Project,
    clip: ClipId,
    from: Frames,
    to: Frames,
    fade: Fade,
    fade_the_middle: bool,
) -> Result<ClipId, CommandError> {
    let Some(found) = project.clip(clip) else { return Ok(clip) };
    let (start, end) = (found.start, found.end());
    let mut middle = clip;
    if start < from {
        if let Outcome::Clip(right) = project.apply(Command::SplitClip { clip, at: from })? {
            project.apply(Command::SetClipFade { clip, edge: Edge::Out, fade })?;
            if fade_the_middle {
                project.apply(Command::SetClipFade { clip: right, edge: Edge::In, fade })?;
            }
            middle = right;
        }
    }
    if end > to {
        if let Outcome::Clip(right) = project.apply(Command::SplitClip { clip: middle, at: to })? {
            if fade_the_middle {
                project.apply(Command::SetClipFade { clip: middle, edge: Edge::Out, fade })?;
            }
            project.apply(Command::SetClipFade { clip: right, edge: Edge::In, fade })?;
        }
    }
    Ok(middle)
}

fn sound_only(project: &mut Project, clip: ClipId, fade: Fade) -> Result<(), CommandError> {
    let track = project.track_of(clip).ok_or(CommandError::NoSuchClip)?.id;
    let chosen = project.clip(clip).ok_or(CommandError::NoSuchClip)?;
    let (lane, from, to) = (chosen.lane, chosen.start, chosen.end());
    let others: Vec<ClipId> = project
        .track(track)
        .ok_or(CommandError::NoSuchTrack)?
        .clips
        .iter()
        .filter(|other| other.id != clip && other.lane.is_some() && other.lane != lane && other.lane_on && other.start < to && other.end() > from)
        .map(|other| other.id)
        .collect();
    for other in others {
        let middle = middle_of(project, other, from, to, fade, false)?;
        let lane = project.clip(middle).and_then(|found| found.lane);
        project.apply(Command::SetClipLane { clip: middle, lane, on: false })?;
    }
    project.apply(Command::SetClipLane { clip, lane, on: true })?;
    Ok(())
}

pub fn comp(project: &mut Project, track: TrackId, take: usize, from: Frames, to: Frames, fade: Fade) -> Result<Vec<ClipId>, CommandError> {
    let touched: Vec<ClipId> = project
        .track(track)
        .ok_or(CommandError::NoSuchTrack)?
        .clips
        .iter()
        .filter(|clip| clip.lane == Some(take) && clip.start < to && clip.end() > from)
        .map(|clip| clip.id)
        .collect();
    let mut chosen = Vec::new();
    for clip in touched {
        let middle = middle_of(project, clip, from, to, fade, true)?;
        sound_only(project, middle, fade)?;
        chosen.push(middle);
    }
    Ok(chosen)
}

pub fn clear_between(project: &mut Project, track: TrackId, from: Frames, to: Frames, fade: Fade) -> Result<(), CommandError> {
    let touched: Vec<ClipId> = project
        .track(track)
        .ok_or(CommandError::NoSuchTrack)?
        .clips
        .iter()
        .filter(|clip| !clip.is_notes() && clip.start < to && clip.end() > from)
        .map(|clip| clip.id)
        .collect();
    for clip in touched {
        let middle = middle_of(project, clip, from, to, fade, false)?;
        project.apply(Command::DeleteClip(middle))?;
    }
    Ok(())
}


pub fn short_fade(rate: u32) -> Fade {
    Fade { len: (COMP_FADE_SECONDS * rate as f64) as Frames, curve: 0.0 }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use loupe_engine::Source;

    use super::*;
    use crate::Message;

    #[test]
    fn picking_a_shorter_take_lights_only_that_take() {
        let mut project = Project::new(48_000);
        let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: "Vox".into() }) else { panic!() };
        let mut add = |project: &mut Project, frames: usize, lane: usize, on: bool| {
            let source = Arc::new(Source::from_frames("Take", vec![[0.5, 0.5]; frames]));
            let Ok(Outcome::Clip(clip)) = project.apply(Command::AddClip { track, source, start: 1_000 }) else { panic!() };
            project.apply(Command::SetClipLane { clip, lane: Some(lane), on }).unwrap();
        };
        add(&mut project, 20_000, 0, true);
        add(&mut project, 8_000, 1, false);
        add(&mut project, 30_000, 2, true);
        use_take_on_track(&mut project, track, 1, Fade::NONE).unwrap();
        let lit: Vec<usize> = project.track(track).unwrap().clips.iter().filter(|clip| clip.lane_on).filter_map(|clip| clip.lane).collect();
        assert_eq!(lit, vec![1], "other takes stayed lit after picking take 2");
    }

    #[test]
    fn picking_a_take_glues_its_cut_pieces_back_and_sounds_the_same() {
        let mut project = Project::new(48_000);
        let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: "Vox".into() }) else { panic!() };
        let frames: Vec<[f32; 2]> = (0..20_000).map(|i| [(i as f32 * 0.01).sin(), 0.0]).collect();
        let source = Arc::new(Source::from_frames("Take", frames));
        let Ok(Outcome::Clip(clip)) = project.apply(Command::AddClip { track, source, start: 1_000 }) else { panic!() };
        project.apply(Command::SetClipLane { clip, lane: Some(0), on: true }).unwrap();
        let Ok(Outcome::Clip(right)) = project.apply(Command::SplitClip { clip, at: 6_000 }) else { panic!() };
        project.apply(Command::SplitClip { clip: right, at: 9_000 }).unwrap();
        assert_eq!(project.track(track).unwrap().clips.len(), 3);
        use_take_on_track(&mut project, track, 0, Fade::NONE).unwrap();
        let clips = &project.track(track).unwrap().clips;
        assert_eq!(clips.len(), 1, "the cut pieces were not glued back");
        assert_eq!((clips[0].start, clips[0].offset, clips[0].len), (1_000, 0, 20_000));
    }

    #[test]
    fn punching_in_keeps_what_is_either_side_and_clears_the_middle() {
        let mut project = Project::new(48_000);
        let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: "Vocal".into() }) else { panic!() };
        let source = Arc::new(Source::from_frames("Verse", vec![[0.5, 0.5]; 10_000]));
        project.apply(Command::AddClip { track, source, start: 1_000 }).unwrap();
        clear_between(&mut project, track, 4_000, 6_000, Fade { len: 4, curve: 0.0 }).unwrap();
        let mut spans: Vec<(Frames, Frames, Frames)> = project.track(track).unwrap().clips.iter().map(|clip| (clip.start, clip.end(), clip.offset)).collect();
        spans.sort();
        assert_eq!(spans, vec![(1_000, 4_000, 0), (6_000, 11_000, 5_000)]);
        let clips = &project.track(track).unwrap().clips;
        assert!(clips.iter().all(|clip| clip.fade_in.len == 4 || clip.fade_out.len == 4));
        clear_between(&mut project, track, 0, 20_000, Fade::NONE).unwrap();
        assert!(project.track(track).unwrap().clips.is_empty(), "a range over everything clears everything");
    }


    #[test]
    fn pulling_out_a_take_gives_it_its_own_track_and_leaves_the_set_and_the_sound_alone() {
        let mut project = Project::new(48_000);
        let (track, _) = stepped_takes(&mut project, 2);
        let copy = pull_out_take(&mut project, track, 1).unwrap();
        let set = &project.track(track).unwrap().clips;
        assert_eq!(set.len(), 2, "the other takes left the set");
        assert_eq!(set[1].lane, Some(1), "the take above did not close the gap");
        assert!(!set[1].is_silent() && set[1].audio()[set[1].offset as usize][0] == 0.75, "the set stopped playing the same take");
        let made = project.track(copy).unwrap();
        assert_eq!(made.name, "Vox take 2");
        let pulled = &made.clips[0];
        assert!(pulled.lane.is_none(), "the moved take is still a take");
        assert_eq!((pulled.start, pulled.len), (500, 1_000), "the take moved in the song");
        assert_eq!(pulled.audio()[pulled.offset as usize][0], 0.5, "the wrong take came out");
        assert!(pulled.is_silent(), "a take that was not playing would double the vocal");
    }

    #[test]
    fn moving_all_takes_makes_one_track_per_take_and_the_song_sounds_the_same() {
        let mut project = Project::new(48_000);
        let (track, _) = stepped_takes(&mut project, 1);
        let made = move_all_takes(&mut project, track).unwrap();
        assert_eq!(made.len(), 2, "three takes should make two new tracks beside the first");
        let names: Vec<String> = made.iter().map(|id| project.track(*id).unwrap().name.clone()).collect();
        assert_eq!(names, ["Vox take 2", "Vox take 3"]);
        let first = &project.track(track).unwrap().clips[0];
        assert!(first.lane.is_none(), "the first track is still takes");
        assert_eq!(first.audio()[first.offset as usize][0], 0.25, "the first track holds the wrong take");
        let sounding: Vec<f32> = project.tracks.iter().flat_map(|t| t.clips.iter()).filter(|c| !c.is_silent()).map(|c| c.audio()[c.offset as usize][0]).collect();
        assert_eq!(sounding, [0.5], "the song should still play only the take it played");
        let order: Vec<TrackId> = project.tracks.iter().map(|t| t.id).collect();
        assert_eq!(order, [track, made[0], made[1]], "the takes are out of order");
    }

    #[test]
    fn moving_all_takes_after_deleting_some_makes_a_track_for_each_one_left() {
        let mut project = Project::new(48_000);
        let (track, _) = stepped_takes(&mut project, 2);
        delete_take(&mut project, track, 0, Fade::NONE).unwrap();
        let made = move_all_takes(&mut project, track).unwrap();
        assert_eq!(made.len(), 1, "two takes left should make one new track beside the first");
        assert_eq!(project.tracks.len(), 2);
        assert!(project.tracks.iter().all(|t| t.clips.len() == 1 && t.clips[0].lane.is_none()));
    }

    #[test]
    fn pulling_out_the_playing_take_moves_the_sound_with_it_and_does_not_double_it() {
        let mut project = Project::new(48_000);
        let (track, _) = stepped_takes(&mut project, 1);
        let copy = pull_out_take(&mut project, track, 1).unwrap();
        assert!(!project.track(copy).unwrap().clips[0].is_silent(), "the moved take stopped sounding");
        assert!(project.track(track).unwrap().clips.iter().all(|clip| clip.is_silent()), "the set doubles the moved take");
    }

    fn stepped_takes(project: &mut Project, active: usize) -> (TrackId, ClipId) {
        let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: "Vox".into() }) else { panic!() };
        let passes: Vec<[f32; 2]> = (0..3_000).map(|at| [(at / 1_000) as f32 / 4.0 + 0.25, 0.0]).collect();
        let source = Arc::new(Source::from_frames("Vox (take 1)", passes));
        let clip = lay_take(project, track, source, 500, 1_000, &[0, 1_000, 2_000], active).unwrap();
        (track, clip)
    }

    #[test]
    fn cutting_or_stretching_one_take_leaves_the_others_alone() {
        let mut project = Project::new(48_000);
        let (track, playing) = stepped_takes(&mut project, 2);
        let other = project.track(track).unwrap().clips[0].id;
        project.apply(Command::SplitClip { clip: other, at: 900 }).unwrap();
        project.apply(Command::SetStretch { clip: other, stretch: 1.5 }).unwrap();
        let clips = &project.track(track).unwrap().clips;
        assert_eq!(clips.iter().filter(|clip| clip.lane == Some(0)).count(), 2, "the take was not cut");
        assert!(clips.iter().filter(|clip| clip.lane != Some(0)).all(|clip| clip.len == 1_000 && clip.stretch == 1.0), "another take was changed");
        assert!(!project.clip(playing).unwrap().is_silent());
    }
    #[test]
    fn a_swipe_cuts_the_clip_and_plays_that_take_between_the_cuts() {
        let mut project = Project::new(48_000);
        let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: "Vocal".into() }) else { panic!() };
        let passes: Vec<[f32; 2]> = (0..3_000).map(|at| [(at / 1_000) as f32, 0.0]).collect();
        let source = Arc::new(Source::from_frames("Vocal (take 1)", passes));
        lay_take(&mut project, track, source, 0, 1_000, &[0, 1_000, 2_000], 2).unwrap();
        let fade = Fade { len: 4, curve: 0.0 };
        let chosen = comp(&mut project, track, 0, 300, 600, fade).unwrap();
        assert_eq!(chosen.len(), 1);
        let pieces = &project.track(track).unwrap().clips;
        let mut spans: Vec<(Frames, Frames, Option<usize>)> = pieces.iter().filter(|piece| !piece.is_silent()).map(|piece| (piece.start, piece.end(), piece.lane)).collect();
        spans.sort();
        assert_eq!(spans, vec![(0, 300, Some(2)), (300, 600, Some(0)), (600, 1_000, Some(2))]);
        let middle = project.clip(chosen[0]).unwrap();
        assert_eq!(middle.audio()[middle.offset as usize + 299][0], 0.0, "the first pass plays in the middle");
        assert_eq!((middle.fade_in.len, middle.fade_out.len), (4, 4));
        let again = comp(&mut project, track, 1, 0, 1_000, fade).unwrap();
        assert_eq!(again.len(), 1, "a swipe over a whole take picks that whole take");
        assert!(project.track(track).unwrap().clips.iter().filter(|piece| !piece.is_silent()).all(|piece| piece.lane == Some(1)));
        assert!(comp(&mut project, track, 9, 0, 1_000, fade).unwrap().is_empty());
    }

    const PASS: Frames = 4_000;
    const PASSES: usize = 5;

    fn passes_of() -> Vec<[f32; 2]> {
        (0..PASS as usize * PASSES).map(|at| {
            let pass = (at / PASS as usize) as f32 + 1.0;
            let ripple = ((at % 11) as f32 - 5.0) / 5.0;
            [pass * 0.03 * ripple, pass * 0.02]
        }).collect()
    }

    fn a_looped_vocal(project: &mut Project, passes: Arc<Source>) -> (TrackId, ClipId) {
        let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: "Vox".into() }) else { panic!() };
        let offsets: Vec<i64> = (0..PASSES).map(|pass| pass as i64 * PASS as i64).collect();
        let clip = lay_take(project, track, passes, 0, PASS, &offsets, PASSES - 1).unwrap();
        (track, clip)
    }

    fn heard(project: &Project) -> Vec<[f32; 2]> {
        let mut out = vec![[0.0; 2]; PASS as usize + 200];
        loupe_engine::render(project, 0, &mut out);
        out
    }

    fn comped(app: &mut App, track: TrackId) {
        app.comp(track, 0, 500, 1_500);
        app.comp(track, 2, 2_500, 3_500);
        let used: Vec<usize> = app.project.track(track).unwrap().clips.iter().filter(|piece| !piece.is_silent()).filter_map(|piece| piece.lane).collect();
        assert!(used.contains(&0) && used.contains(&2) && used.contains(&(PASSES - 1)), "the comp did not use the takes it was asked for: {used:?}");
    }

    #[test]
    fn folding_the_takes_away_keeps_the_comp_sounding_the_same_and_undo_brings_the_fold_back() {
        let mut app = App::bare();
        let (track, _) = a_looped_vocal(&mut app.project, Arc::new(Source::from_frames("Vox (take 1)", passes_of())));
        assert!(!app.project.track(track).unwrap().takes_open, "a track starts with its takes folded");
        comped(&mut app, track);
        let folded = heard(&app.project);
        assert!(folded.iter().any(|frame| frame[0] != 0.0));
        let _ = app.update(Message::ToggleTakes(track));
        assert!(app.project.track(track).unwrap().takes_open);
        assert_eq!(heard(&app.project), folded, "opening the takes changed what plays");
        let _ = app.update(Message::ToggleTakes(track));
        assert!(!app.project.track(track).unwrap().takes_open);
        assert_eq!(heard(&app.project), folded, "folding the takes changed what plays");
        let _ = app.update(Message::Undo);
        assert!(app.project.track(track).unwrap().takes_open, "undo did not reopen the takes");
        let _ = app.update(Message::Undo);
        assert!(!app.project.track(track).unwrap().takes_open, "undo did not fold the takes again");
        assert_eq!(heard(&app.project), folded);
    }

    #[test]
    fn deleting_unused_takes_keeps_the_comp_sounding_the_same_and_leaves_the_recording_on_disk() {
        let folder = std::env::temp_dir().join(format!("loupe-unused-takes-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("Vox (take 1).wav");
        let mut app = App::bare();
        let rate = app.project.rate;
        loupe_engine::write_frames(&file, &passes_of(), rate).unwrap();
        let recorded = Arc::new(Source::load(&file, rate).unwrap());
        let (track, _) = a_looped_vocal(&mut app.project, recorded);
        comped(&mut app, track);
        let before = heard(&app.project);
        assert_eq!(unused_takes(app.project.track(track).unwrap()), 2, "takes 2 and 4 are not in the comp");
        let _ = app.update(Message::DeleteUnusedTakes(track));
        assert_eq!(take_count(app.project.track(track).unwrap()), 3, "the three takes the comp uses should be left");
        assert_eq!(unused_takes(app.project.track(track).unwrap()), 0);
        assert_eq!(heard(&app.project), before, "the comp sounds different after deleting the unused takes");
        assert!(file.exists(), "the recording was deleted from disk");
        let _ = app.update(Message::Undo);
        assert_eq!(take_count(app.project.track(track).unwrap()), PASSES, "undo did not bring the takes back");
        assert_eq!(heard(&app.project), before);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_comp_down_to_one_take_becomes_an_ordinary_clip_and_nothing_unused_leaves_no_undo_step() {
        let mut project = Project::new(48_000);
        let (track, clip) = a_looped_vocal(&mut project, Arc::new(Source::from_frames("Vox (take 1)", passes_of())));
        let before = heard(&project);
        assert_eq!(delete_unused_takes(&mut project, track).unwrap(), PASSES - 1);
        assert!(project.clip(clip).is_some());
        assert_eq!(heard(&project), before);
        let mut app = App::bare();
        let (track, _) = a_looped_vocal(&mut app.project, Arc::new(Source::from_frames("Vox (take 1)", passes_of())));
        let _ = app.update(Message::DeleteUnusedTakes(track));
        let _ = app.update(Message::DeleteUnusedTakes(track));
        assert_eq!(app.undo.len(), 1, "a second delete with nothing unused made an undo step");
    }

    #[test]
    fn comping_a_take_that_is_not_there_leaves_the_song_whole() {
        let mut project = Project::new(48_000);
        let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: "Vocal".into() }) else { panic!() };
        let source = Arc::new(Source::from_frames("take", vec![[0.1, 0.1]; 10_000]));
        let Ok(Outcome::Clip(clip)) = project.apply(Command::AddClip { track, source, start: 0 }) else { panic!() };
        let fade = Fade { len: 100, curve: 0.0 };
        let chosen = comp(&mut project, track, 1, 2_000, 4_000, fade).unwrap();
        assert!(chosen.is_empty(), "it comped a take that is not there");
        let clips = &project.track(track).unwrap().clips;
        assert_eq!(clips.len(), 1, "the song was chopped up for nothing");
        assert_eq!(clips[0].id, clip);
    }
}

