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
        // An empty take row can be removed too, not only one with audio in it.
        let there = self.project.track(track).is_some_and(|found| take < lane_rows(found));
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
    // Audio already on the track is not a take and is never touched: a new
    // recording over it goes on a track of its own (same sound and sends), and
    // later recordings over that spot stack as takes there.
    if found.clips.iter().any(|clip| !clip.is_notes() && clip.lane.is_none() && clip.start < end && clip.end() > start) {
        let takes = takes_track_for(project, track, start, end)?;
        return lay_take(project, takes, source, start, len, offsets, active);
    }
    let under: Vec<Clip> = found.clips.iter().filter(|clip| !clip.is_notes() && clip.start < end && clip.end() > start).cloned().collect();
    let next = lane_rows(found);
    // Every recording is a take, so the next one recorded over it stacks with it.
    let becomes_lanes = true;
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

// The track that holds recordings made over existing audio on `track`: the
// one made for that before if nothing already recorded sits in the way there,
// otherwise a fresh copy of `track` with the same sound and no clips.
fn takes_track_for(project: &mut Project, track: TrackId, start: Frames, end: Frames) -> Result<TrackId, CommandError> {
    let found = project.track(track).ok_or(CommandError::NoSuchTrack)?;
    let name = format!("{} takes", found.name);
    let free = project.tracks.iter().find(|other| {
        other.name == name && !other.clips.iter().any(|clip| !clip.is_notes() && clip.lane.is_none() && clip.start < end && clip.end() > start)
    });
    if let Some(free) = free {
        return Ok(free.id);
    }
    let Outcome::Track(copy) = project.apply(Command::DuplicateTrack(track))? else {
        return Err(CommandError::InvalidValue);
    };
    project.apply(Command::RenameTrack { track: copy, name })?;
    let copies: Vec<ClipId> = project.track(copy).ok_or(CommandError::NoSuchTrack)?.clips.iter().map(|clip| clip.id).collect();
    for clip in copies {
        project.apply(Command::DeleteClip(clip))?;
    }
    project.apply(Command::SetTakesOpen { track: copy, open: true })?;
    Ok(copy)
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
    // The take's track carries no plugins or sends of its own: it plays into
    // the track it came from, through that one chain, so a song with dozens of
    // takes never loads dozens of copies of the vocal chain.
    let made = project.track(copy).ok_or(CommandError::NoSuchTrack)?;
    let (slots, sends) = (made.fx.len(), made.sends.iter().map(|send| send.to).collect::<Vec<_>>());
    for slot in (0..slots).rev() {
        project.apply(Command::RemoveFx { track: copy, slot })?;
    }
    for to in sends {
        project.apply(Command::RemoveSend { from: copy, to })?;
    }
    project.apply(Command::SetTrackParent { track: copy, parent: Some(track) })?;
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

#[derive(Clone, Debug, PartialEq)]
pub struct Healed {
    pub source: String,
    pub tracks: usize,
    pub plugins: usize,
}

// Songs made before 0.27.13 have take tracks that "Move all takes to tracks"
// made with a full copy of the source track's chain and sends. A take track
// that is provably such a copy loses them and plays through the source track
// instead, as pull_out_take does now. Anything that differs is left alone.
pub fn heal_copied_take_chains(project: &mut Project) -> Vec<Healed> {
    let mut healed: Vec<Healed> = Vec::new();
    let mut at = 0;
    while at < project.tracks.len() {
        let source = project.tracks[at].id;
        let mut next = at + 1;
        while let Some(candidate) = project.tracks.get(next).map(|track| track.id) {
            if project.descends_from(candidate, source) {
                next += 1;
                continue;
            }
            let named_as_a_take = project.track(candidate).is_some_and(|track| take_number(&track.name, &project.tracks[at].name).is_some());
            if !named_as_a_take {
                break;
            }
            if !is_a_copied_take_of(project, candidate, source) {
                next += 1;
                continue;
            }
            let end_of_source = at + project.run_from(at);
            let Some(plugins) = heal_one(project, candidate, source, end_of_source) else {
                next += 1;
                continue;
            };
            let name = project.tracks[at].name.clone();
            match healed.iter_mut().find(|done| done.source == name) {
                Some(done) => {
                    done.tracks += 1;
                    done.plugins += plugins;
                }
                None => healed.push(Healed { source: name, tracks: 1, plugins }),
            }
            next += 1;
        }
        at += 1;
    }
    healed
}

pub fn word_for_healed(healed: &[Healed]) -> Option<String> {
    let parts: Vec<String> = healed
        .iter()
        .map(|done| {
            let tracks = if done.tracks == 1 { "take track" } else { "take tracks" };
            let play = if done.tracks == 1 { "it now plays" } else { "they now play" };
            format!("Removed the copied {} plugins from {} {tracks}; {play} through {}.", done.source, done.tracks, done.source)
        })
        .collect();
    if parts.is_empty() {
        return None;
    }
    Some(format!("{} Ctrl+Z puts them back.", parts.join(" ")))
}

fn take_number(name: &str, source: &str) -> Option<usize> {
    let number = name.strip_prefix(source)?.strip_prefix(" take ")?;
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    number.parse::<usize>().ok().filter(|number| *number >= 1)
}

fn same_fx(one: &loupe_engine::Fx, other: &loupe_engine::Fx) -> bool {
    one.path == other.path && one.index == other.index && one.state == other.state && one.bypassed == other.bypassed && one.mix == other.mix
}

fn is_a_copied_take_of(project: &Project, take: TrackId, source: TrackId) -> bool {
    use loupe_engine::Target;
    let (Some(copy), Some(original)) = (project.track(take), project.track(source)) else { return false };
    if take_number(&copy.name, &original.name).is_none() || original.name.contains(" take ") {
        return false;
    }
    if project.tracks.iter().filter(|track| track.name == original.name).count() != 1 {
        return false;
    }
    let chain_is_a_copy = !copy.fx.is_empty()
        && copy.fx.len() == original.fx.len()
        && copy.fx.iter().zip(&original.fx).all(|(one, other)| same_fx(one, other))
        && copy.fx_on == original.fx_on;
    let placed_as_a_copy = copy.parent.is_none() || copy.parent == original.parent;
    let plays_like_a_copy = copy.sends == original.sends && copy.pan == original.pan && !copy.solo && !original.solo && original.gain > 0.0;
    let nothing_leans_on_it = !project.tracks.iter().any(|track| track.parent == Some(take) || track.sends.iter().any(|send| send.to == take));
    let no_own_automation = !project.envelopes.iter().any(|shape| match shape.target {
        Target::TrackGain(id) | Target::TrackPan(id) => id == take,
        Target::SendGain { from, to } => from == take || to == take,
        Target::TrackFx { track, .. } => track == take,
        _ => false,
    });
    let sounds = copy.clips.iter().any(|clip| !clip.is_silent());
    let source_moves = project.envelopes.iter().any(|shape| matches!(shape.target, Target::TrackGain(id) | Target::TrackPan(id) if id == source));
    chain_is_a_copy && placed_as_a_copy && plays_like_a_copy && nothing_leans_on_it && no_own_automation && !(sounds && source_moves)
}

fn heal_one(project: &mut Project, take: TrackId, source: TrackId, to: usize) -> Option<usize> {
    let mut trial = project.clone();
    let plugins = strip_and_parent(&mut trial, take, source, to).ok()?;
    *project = trial;
    Some(plugins)
}

fn strip_and_parent(project: &mut Project, take: TrackId, source: TrackId, to: usize) -> Result<usize, CommandError> {
    let copy = project.track(take).ok_or(CommandError::NoSuchTrack)?;
    let original = project.track(source).ok_or(CommandError::NoSuchTrack)?;
    let (slots, sends) = (copy.fx.len(), copy.sends.iter().map(|send| send.to).collect::<Vec<_>>());
    // The take now also plays through the source's fader and pan, so its own
    // fader gives back what the source's adds and the level heard stays put.
    let gain = copy.gain / original.gain;
    for slot in (0..slots).rev() {
        project.apply(Command::RemoveFx { track: take, slot })?;
    }
    for to in sends {
        project.apply(Command::RemoveSend { from: take, to })?;
    }
    project.apply(Command::SetTrackGain { track: take, gain })?;
    project.apply(Command::SetTrackPan { track: take, pan: 0.0 })?;
    // A child sits in its parent's run of tracks, so a take that is not
    // already there moves up to the end of it.
    match project.tracks.iter().position(|track| track.id == take) == Some(to) {
        true => project.apply(Command::SetTrackParent { track: take, parent: Some(source) })?,
        false => project.apply(Command::MoveTrack { track: take, to, parent: Some(source) })?,
    };
    Ok(slots)
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
pub(crate) mod tests {
    use std::sync::Arc;

    use loupe_engine::Source;

    use super::*;
    use crate::Message;


    #[test]
    fn an_empty_take_row_can_be_deleted() {
        let mut project = Project::new(48_000);
        let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: "Vox".into() }) else { panic!() };
        let source = Arc::new(Source::from_frames("Take", vec![[0.5, 0.5]; 1_000]));
        let Ok(Outcome::Clip(clip)) = project.apply(Command::AddClip { track, source, start: 0 }) else { panic!() };
        project.apply(Command::SetClipLane { clip, lane: Some(2), on: true }).unwrap();
        assert_eq!(lane_rows(project.track(track).unwrap()), 3, "rows 1 and 2 should be empty takes");
        delete_take(&mut project, track, 0, Fade::NONE).unwrap();
        assert_eq!(lane_rows(project.track(track).unwrap()), 2, "the empty take was not removed");
    }

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

    fn chain_fx(path: &str, state: &[u8]) -> loupe_engine::Fx {
        loupe_engine::Fx { path: path.into(), index: 0, name: path.into(), bypassed: false, state: state.to_vec(), record: false, mix: 1.0 }
    }

    // A song as "Move all takes to tracks" left it before 0.27.13: every take
    // track a full copy of Vox, chain, sends, fader and pan included.
    pub(crate) fn an_old_song() -> (Project, TrackId, Vec<TrackId>) {
        let mut project = Project::new(48_000);
        let add = |project: &mut Project, name: &str| {
            let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: name.into() }) else { panic!() };
            track
        };
        let bus = add(&mut project, "Vocal Bus");
        let vox = add(&mut project, "Vox");
        let reverb = add(&mut project, "Vox Reverb");
        for track in [vox, reverb] {
            project.apply(Command::SetTrackParent { track, parent: Some(bus) }).unwrap();
        }
        project.apply(Command::AddFx { track: vox, fx: chain_fx("Pro-Q 4.vst3", b"eq") }).unwrap();
        project.apply(Command::AddFx { track: vox, fx: chain_fx("RX 11 Voice De-noise.vst3", b"denoise") }).unwrap();
        project.apply(Command::AddSend { from: vox, to: reverb }).unwrap();
        project.apply(Command::SetSendGain { from: vox, to: reverb, gain: 0.2 }).unwrap();
        project.apply(Command::SetTrackGain { track: vox, gain: 0.8 }).unwrap();
        project.apply(Command::SetTrackPan { track: vox, pan: 0.25 }).unwrap();
        let wave = |pitch: f32| Arc::new(Source::from_frames("Vox", (0..2_000).map(|at| [(at as f32 * pitch).sin() * 0.5, (at as f32 * pitch).cos() * 0.3]).collect()));
        project.apply(Command::AddClip { track: vox, source: wave(0.01), start: 0 }).unwrap();
        let mut takes = Vec::new();
        for (number, pitch, muted) in [(4, 0.05, false), (3, 0.03, true), (2, 0.02, false)] {
            let Ok(Outcome::Track(copy)) = project.apply(Command::DuplicateTrack(vox)) else { panic!() };
            project.apply(Command::RenameTrack { track: copy, name: format!("Vox take {number}") }).unwrap();
            let old: Vec<ClipId> = project.track(copy).unwrap().clips.iter().map(|clip| clip.id).collect();
            for clip in old {
                project.apply(Command::DeleteClip(clip)).unwrap();
            }
            let Ok(Outcome::Clip(clip)) = project.apply(Command::AddClip { track: copy, source: wave(pitch), start: 300 }) else { panic!() };
            project.apply(Command::SetClipMuted { clip, muted }).unwrap();
            takes.insert(0, copy);
        }
        (project, vox, takes)
    }

    fn heard_long(project: &Project) -> Vec<[f32; 2]> {
        let mut out = vec![[0.0; 2]; 3_000];
        loupe_engine::render(project, 0, &mut out);
        out
    }

    fn sounds_alike(one: &[[f32; 2]], other: &[[f32; 2]]) -> bool {
        one.len() == other.len() && one.iter().zip(other).all(|(a, b)| (a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5)
    }

    fn plugins_in(project: &Project) -> usize {
        project.tracks.iter().map(|track| track.fx.len()).sum()
    }

    fn children_sit_under_their_parent(project: &Project) -> bool {
        project.tracks.iter().enumerate().all(|(at, track)| match track.parent {
            None => true,
            Some(parent) => {
                let Some(head) = project.tracks.iter().position(|one| one.id == parent) else { return false };
                head < at && at < head + project.run_from(head)
            }
        })
    }

    #[test]
    fn copied_take_chains_are_removed_and_the_song_sounds_the_same() {
        let (mut project, vox, takes) = an_old_song();
        assert_eq!(plugins_in(&project), 8, "the old song carries four copies of the two plugin chain");
        let before = heard_long(&project);
        assert!(before.iter().any(|frame| frame[0] != 0.0));
        let healed = heal_copied_take_chains(&mut project);
        assert_eq!(healed, vec![Healed { source: "Vox".into(), tracks: 3, plugins: 6 }]);
        assert_eq!(plugins_in(&project), 2, "only Vox's own chain should be left");
        for take in &takes {
            let track = project.track(*take).unwrap();
            assert!(track.fx.is_empty() && track.sends.is_empty(), "{} kept its copies", track.name);
            assert_eq!(track.parent, Some(vox), "{} does not play through Vox", track.name);
        }
        assert!(children_sit_under_their_parent(&project));
        assert!(sounds_alike(&heard_long(&project), &before), "the song sounds different after the copies were removed");
        assert!(heal_copied_take_chains(&mut project).is_empty(), "a healed song is healed again");
        assert_eq!(
            word_for_healed(&healed).unwrap(),
            "Removed the copied Vox plugins from 3 take tracks; they now play through Vox. Ctrl+Z puts them back."
        );
    }

    #[test]
    fn a_take_that_is_not_an_exact_copy_is_left_alone() {
        let (mut project, vox, takes) = an_old_song();
        let changed = takes[1];
        project.apply(Command::SetFxState { track: changed, slot: 1, state: b"learned another noise".to_vec() }).unwrap();
        let before = heard_long(&project);
        let untouched = project.track(changed).unwrap().clone();
        let healed = heal_copied_take_chains(&mut project);
        assert_eq!(healed, vec![Healed { source: "Vox".into(), tracks: 2, plugins: 4 }]);
        let now = project.track(changed).unwrap();
        assert_eq!((&now.fx, &now.sends, now.parent, now.gain, now.pan), (&untouched.fx, &untouched.sends, untouched.parent, untouched.gain, untouched.pan));
        assert_eq!(project.track(takes[2]).unwrap().parent, Some(vox), "the exact copy after the odd one was not healed");
        assert!(children_sit_under_their_parent(&project));
        assert!(sounds_alike(&heard_long(&project), &before));
    }

    #[test]
    fn look_alikes_are_never_touched() {
        let checks: Vec<(&str, Box<dyn Fn(&mut Project, TrackId, TrackId)>)> = vec![
            ("renamed", Box::new(|project, _, take| { project.apply(Command::RenameTrack { track: take, name: "Vox take two".into() }).unwrap(); })),
            ("bypassed slot", Box::new(|project, _, take| { project.apply(Command::BypassFx { track: take, slot: 0, bypassed: true }).unwrap(); })),
            ("panned", Box::new(|project, _, take| { project.apply(Command::SetTrackPan { track: take, pan: -0.5 }).unwrap(); })),
            ("own send", Box::new(|project, _, take| { project.apply(Command::SetSendGain { from: take, to: project.tracks[2 + 3].id, gain: 0.9 }).unwrap(); })),
            ("fed by another", Box::new(|project, _, take| { let Ok(Outcome::Track(keys)) = project.apply(Command::AddTrack { name: "Keys".into() }) else { panic!() }; project.apply(Command::AddSend { from: keys, to: take }).unwrap(); })),
            ("automated", Box::new(|project, _, take| { project.apply(Command::AddEnvelope { target: loupe_engine::Target::TrackGain(take) }).unwrap(); })),
            ("soloed", Box::new(|project, _, take| { project.apply(Command::SetTrackSolo { track: take, solo: true }).unwrap(); })),
        ];
        for (what, change) in checks {
            let (mut project, vox, takes) = an_old_song();
            assert_eq!(project.tracks[5].name, "Vox Reverb");
            change(&mut project, vox, takes[0]);
            let untouched = project.track(takes[0]).unwrap().fx.clone();
            heal_copied_take_chains(&mut project);
            assert_eq!(project.track(takes[0]).unwrap().fx, untouched, "a {what} take lost its plugins");
            assert!(children_sit_under_their_parent(&project), "{what}");
        }
        let (mut project, _, _) = an_old_song();
        project.apply(Command::AddTrack { name: "Vox".into() }).unwrap();
        assert!(heal_copied_take_chains(&mut project).is_empty(), "two tracks called Vox: which one is the source is not certain");
    }
}
