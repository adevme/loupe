use std::path::PathBuf;
use std::sync::Arc;

use loupe_engine::{ClipId, Command, Fade, Frames, Outcome, Source, TrackId};

use crate::{App, Run};

pub(crate) const TAKES_FOLDER: &str = "Audio";
const CONSOLIDATED_MARK: &str = " (consolidated";

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

    pub(crate) fn nudge_clips(&mut self, down: bool) {
        let Some(lead) = self.in_song_order().first().copied() else {
            return;
        };
        let (Some(start), Some(track)) = (self.project.clip(lead).map(|clip| clip.start), self.project.track_of(lead).map(|track| track.id)) else {
            return;
        };
        let Some(row) = self.project.tracks.iter().position(|found| found.id == track) else {
            return;
        };
        let shown = |at: &usize| self.project.tracks.get(*at).is_some_and(|found| !crate::timeline::out_of_sight(&self.project, found));
        let next = match down {
            true => (row + 1..self.project.tracks.len()).find(shown),
            false => (0..row).rev().find(shown),
        };
        let Some(to) = next else {
            self.notice = Some("No track that way.".into());
            return;
        };
        let Some(target) = self.project.tracks.get(to).map(|found| found.id) else {
            return;
        };
        let moved = self.selection.len();
        self.move_clips(lead, target, start);
        let now = self.project.track_of(lead).map(|track| track.id);
        self.notice = Some(match now == Some(target) {
            true => format!("Moved {moved} clip{} {}.", if moved == 1 { "" } else { "s" }, if down { "down" } else { "up" }),
            false => "Those clips cannot move that way.".into(),
        });
    }

    pub(crate) fn affected_by(&self, clip: ClipId) -> Vec<ClipId> {
        if self.selection.len() > 1 && self.selection.contains(&clip) {
            self.in_song_order()
        } else {
            vec![clip]
        }
    }

    pub(crate) fn trims_following(&self, lead: ClipId, start: Frames, offset: Frames, len: Frames) -> Vec<(ClipId, TrackId, Frames, Frames, Frames)> {
        let Some(was) = self.project.clip(lead).cloned() else {
            return Vec::new();
        };
        let shortest = crate::timeline::SHORTEST_CLIP;
        let from_left = start != was.start;
        let mut trims = Vec::new();
        for id in self.affected_by(lead) {
            let (Some(clip), Some(track)) = (self.project.clip(id), self.project.track_of(id).map(|track| track.id)) else { continue };
            if id == lead {
                trims.push((id, track, start, offset, len));
                continue;
            }
            if from_left {
                let earliest = clip.start.saturating_sub(clip.offset);
                let latest = clip.end().saturating_sub(shortest).max(earliest);
                let moved = (clip.start as i64 + start as i64 - was.start as i64).max(0) as Frames;
                let new_start = moved.clamp(earliest, latest);
                trims.push((id, track, new_start, (clip.offset as i64 + new_start as i64 - clip.start as i64).max(0) as Frames, clip.end() - new_start));
            } else {
                let longest = clip.audio_len().saturating_sub(clip.offset).max(clip.len);
                let grown = (clip.len as i64 + len as i64 - was.len as i64).max(0) as Frames;
                let new_len = grown.clamp(shortest.min(clip.len), longest);
                trims.push((id, track, clip.start, clip.offset, new_len));
            }
        }
        trims
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

    pub(crate) fn delete_parts(&mut self, clips: Vec<ClipId>, run: Option<Run>) {
        self.transact(run, |project| {
            for clip in clips {
                let folded = project.track_of(clip).filter(|track| !track.takes_open).map(|track| track.id);
                match (folded, project.clip(clip).filter(|found| found.lane.is_some()).map(|found| (found.start, found.end()))) {
                    (Some(track), Some((from, to))) => crate::takes::clear_between(project, track, from, to, Fade::NONE)?,
                    _ => {
                        project.apply(Command::DeleteClip(clip))?;
                    }
                }
            }
            Ok(Outcome::Done)
        });
        self.forget_gone_clips();
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

    pub(crate) fn join_selection(&mut self) {
        let clips = self.in_song_order();
        if clips.len() < 2 {
            return;
        }
        match self.consolidate(&clips) {
            Ok(whole) => self.choose([whole]),
            Err(why) => self.problem = Some(why),
        }
    }

    fn consolidate(&mut self, clips: &[ClipId]) -> Result<ClipId, String> {
        let track = self.project.track_of(clips[0]).ok_or("Those clips are gone.")?;
        if clips.iter().any(|clip| self.project.track_of(*clip).map(|t| t.id) != Some(track.id)) {
            return Err("Select clips on one track to consolidate them.".into());
        }
        if clips.iter().filter_map(|clip| self.project.clip(*clip)).any(|clip| clip.waiting_for_stretch()) {
            return Err("Still working out the time stretch. Try again in a moment.".into());
        }
        let folder = self
            .path
            .as_deref()
            .and_then(|file| file.parent())
            .map(|project| project.join(TAKES_FOLDER))
            .ok_or("Save the project first. Consolidating makes a new audio file.")?;
        let chosen: Vec<_> = track.clips.iter().filter(|clip| clips.contains(&clip.id)).collect();
        let from = chosen.iter().map(|clip| clip.start).min().unwrap_or(0);
        let to = chosen.iter().map(|clip| clip.end()).max().unwrap_or(0);
        let track_id = track.id;
        let first_name = chosen.iter().min_by_key(|clip| clip.start).map(|clip| clip.called().to_string()).unwrap_or_default();
        let name = crate::files::file_safe(first_name.split(CONSOLIDATED_MARK).next().unwrap_or(&first_name).trim_end());

        let mut alone = self.project.clone();
        alone.tracks.retain(|other| other.id == track_id);
        alone.master = 1.0;
        for only in &mut alone.tracks {
            only.gain = 1.0;
            only.muted = false;
            only.fx.clear();
            only.clips.retain(|clip| clips.contains(&clip.id));
        }

        std::fs::create_dir_all(&folder).map_err(|why| format!("{}: {why}", folder.display()))?;
        let file = unused_file(&folder, &name);
        let mut baked = self.offline_racks(&alone);
        loupe_engine::render_to_wav_through(&alone, &file, from, to, Some(baked.as_mut()))?;
        drop(baked);
        let source = Arc::new(Source::load(&file, self.project.rate)?);

        let joined = self.transact(None, |project| {
            for clip in clips {
                project.apply(Command::DeleteClip(*clip))?;
            }
            project.apply(Command::AddClip { track: track_id, source, start: from })
        });
        match joined {
            Some(Outcome::Clip(whole)) => Ok(whole),
            _ => Err("The clips could not be consolidated.".into()),
        }
    }
}

fn unused_file(folder: &std::path::Path, name: &str) -> PathBuf {
    let first = folder.join(format!("{name} (consolidated).wav"));
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|number| folder.join(format!("{name} (consolidated {number}).wav")))
        .find(|file| !file.exists())
        .unwrap_or(first)
}


#[cfg(test)]
mod nudge_tests {
    use super::*;

    #[test]
    fn alt_arrows_move_selected_clips_one_track_and_never_in_time() {
        let mut app = App::bare();
        let Ok(Outcome::Track(top)) = app.project.apply(Command::AddTrack { name: "Vox".into() }) else { panic!() };
        let Ok(Outcome::Track(below)) = app.project.apply(Command::AddTrack { name: "Hook".into() }) else { panic!() };
        let source = Arc::new(Source::from_frames("take", vec![[0.1, 0.1]; 4_000]));
        let Ok(Outcome::Clip(clip)) = app.project.apply(Command::AddClip { track: top, source, start: 12_345 }) else { panic!() };
        app.choose([clip]);
        app.nudge_clips(true);
        assert_eq!(app.project.track_of(clip).map(|track| track.id), Some(below));
        assert_eq!(app.project.clip(clip).map(|found| found.start), Some(12_345), "the clip slid in time");
        app.nudge_clips(true);
        assert_eq!(app.project.track_of(clip).map(|track| track.id), Some(below), "it left the last track");
        app.nudge_clips(false);
        assert_eq!(app.project.track_of(clip).map(|track| track.id), Some(top));
        assert_eq!(app.project.clip(clip).map(|found| found.start), Some(12_345));
    }


    #[test]
    fn trimming_one_of_several_selected_clips_trims_them_all() {
        let mut app = App::bare();
        let Ok(Outcome::Track(top)) = app.project.apply(Command::AddTrack { name: "Take 1".into() }) else { panic!() };
        let Ok(Outcome::Track(below)) = app.project.apply(Command::AddTrack { name: "Take 2".into() }) else { panic!() };
        let source = Arc::new(Source::from_frames("take", vec![[0.1, 0.1]; 4_000]));
        let Ok(Outcome::Clip(a)) = app.project.apply(Command::AddClip { track: top, source: source.clone(), start: 1_000 }) else { panic!() };
        let Ok(Outcome::Clip(b)) = app.project.apply(Command::AddClip { track: below, source, start: 1_000 }) else { panic!() };
        app.choose([a, b]);
        let _ = app.update(crate::Message::TrimClip { clip: a, start: 1_000, offset: 0, len: 3_000 });
        assert_eq!(app.project.clip(a).map(|clip| clip.len), Some(3_000));
        assert_eq!(app.project.clip(b).map(|clip| clip.len), Some(3_000), "the other selected clip kept its end");
        let _ = app.update(crate::Message::TrimClip { clip: b, start: 1_500, offset: 500, len: 2_500 });
        for clip in [a, b] {
            let found = app.project.clip(clip).unwrap();
            assert_eq!((found.start, found.offset, found.len), (1_500, 500, 2_500), "a selected clip kept its start");
        }
        app.choose([a]);
        let _ = app.update(crate::Message::TrimClip { clip: a, start: 1_500, offset: 500, len: 2_000 });
        assert_eq!(app.project.clip(b).map(|clip| clip.len), Some(2_500), "an unselected clip was trimmed");
    }

    #[test]
    fn dragging_the_gain_of_one_selected_clip_moves_them_all_by_the_same_db() {
        let mut app = App::bare();
        let Ok(Outcome::Track(top)) = app.project.apply(Command::AddTrack { name: "Vox".into() }) else { panic!() };
        let source = Arc::new(Source::from_frames("take", vec![[0.1, 0.1]; 4_000]));
        let Ok(Outcome::Clip(a)) = app.project.apply(Command::AddClip { track: top, source: source.clone(), start: 0 }) else { panic!() };
        let Ok(Outcome::Clip(b)) = app.project.apply(Command::AddClip { track: top, source, start: 5_000 }) else { panic!() };
        app.project.apply(Command::SetClipGain { clip: b, gain: 0.5 }).unwrap();
        app.choose([a, b]);
        let _ = app.update(crate::Message::ClipGain { clip: a, db: -6.0 });
        let db = |clip| 20.0 * app.project.clip(clip).unwrap().gain.log10();
        assert!((db(a) + 6.0).abs() < 0.01, "the dragged clip is at {} dB", db(a));
        assert!((db(b) - (20.0 * 0.5f32.log10() - 6.0)).abs() < 0.01, "the other selected clip is at {} dB", db(b));
        assert_eq!(app.selection.len(), 2, "the drag dropped the selection");
    }
    #[test]
    fn the_vertical_drag_key_is_the_one_chosen() {
        use crate::settings::VerticalDrag;
        assert!(VerticalDrag::Shift.held(true, false));
        assert!(!VerticalDrag::Shift.held(false, true));
        assert!(VerticalDrag::Control.held(false, true));
        assert!(!VerticalDrag::Off.held(true, true));
        assert_eq!(VerticalDrag::from_saved(Some(VerticalDrag::Control.saved_as())), VerticalDrag::Control);
        assert_eq!(VerticalDrag::from_saved(None), VerticalDrag::Shift);
    }
}

#[cfg(test)]
mod nudge_take_tests {
    use super::*;

    #[test]
    fn alt_arrows_move_a_take_from_a_group() {
        let folder = std::env::temp_dir().join(format!("loupe-nudge-take-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&folder).unwrap();
        let mut app = App::bare();
        let Ok(Outcome::Track(top)) = app.project.apply(Command::AddTrack { name: "Vox".into() }) else { panic!() };
        let Ok(Outcome::Track(below)) = app.project.apply(Command::AddTrack { name: "Hook".into() }) else { panic!() };
        let take = |name: &str| {
            let file = folder.join(name);
            loupe_engine::write_frames(&file, &vec![[0.5, 0.5]; 48_000], 48_000).unwrap();
            Arc::new(Source::load(&file, 48_000).unwrap())
        };
        app.place_take(0, 0, false, None, None, Ok(vec![(top, take("one.wav"))]));
        app.place_take(0, 0, true, None, None, Ok(vec![(top, take("two.wav"))]));
        let playing = app.project.track(top).unwrap().clips.iter().find(|clip| clip.lane_on).unwrap().id;
        app.choose([playing]);
        app.nudge_clips(true);
        assert_eq!(app.project.track_of(playing).map(|track| track.id), Some(below), "the take did not move: {:?}", app.problem);
        std::fs::remove_dir_all(&folder).ok();
    }
}

#[cfg(test)]
mod nudge_hidden_tests {
    use super::*;

    #[test]
    fn alt_arrows_skip_hidden_tracks() {
        let mut app = App::bare();
        let Ok(Outcome::Track(top)) = app.project.apply(Command::AddTrack { name: "Vox".into() }) else { panic!() };
        let Ok(Outcome::Track(hidden)) = app.project.apply(Command::AddTrack { name: "Vox Reverb".into() }) else { panic!() };
        let Ok(Outcome::Track(below)) = app.project.apply(Command::AddTrack { name: "Hook".into() }) else { panic!() };
        app.project.apply(Command::SetTrackHidden { track: hidden, hidden: true }).unwrap();
        let source = Arc::new(Source::from_frames("take", vec![[0.1, 0.1]; 4_000]));
        let Ok(Outcome::Clip(clip)) = app.project.apply(Command::AddClip { track: top, source, start: 500 }) else { panic!() };
        app.choose([clip]);
        app.nudge_clips(true);
        assert_eq!(app.project.track_of(clip).map(|track| track.id), Some(below), "it landed on the hidden track");
        app.nudge_clips(false);
        assert_eq!(app.project.track_of(clip).map(|track| track.id), Some(top));
    }
}
