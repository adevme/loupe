use std::sync::Arc;

use loupe_engine::{ClipId, Command, CommandError, Edge, Fade, Frames, Outcome, Project, Track, TrackId};

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
        let takes = if removed == 1 { "take" } else { "takes" };
        self.notice = Some(format!("Deleted {removed} unused {takes}. The recordings stay in the Audio folder."));
    }
}

struct TakeFolder {
    clips: Vec<ClipId>,
    count: usize,
    used: Vec<usize>,
}

fn take_folders(track: &Track) -> Vec<TakeFolder> {
    let mut folders: Vec<(&Arc<loupe_engine::Source>, TakeFolder)> = Vec::new();
    for clip in track.clips.iter().filter(|clip| clip.has_takes() && !clip.is_notes()) {
        let count = clip.takes.len();
        let at = folders.iter().position(|(source, folder)| Arc::ptr_eq(source, &clip.source) && folder.count == count);
        let at = at.unwrap_or_else(|| {
            folders.push((&clip.source, TakeFolder { clips: Vec::new(), count, used: Vec::new() }));
            folders.len() - 1
        });
        let folder = &mut folders[at].1;
        folder.clips.push(clip.id);
        if !folder.used.contains(&clip.take) {
            folder.used.push(clip.take);
        }
    }
    folders
        .into_iter()
        .map(|(_, mut folder)| {
            folder.used.sort_unstable();
            folder
        })
        .collect()
}

pub fn unused_takes(track: &Track) -> usize {
    take_folders(track).iter().map(|folder| folder.count - folder.used.len()).sum()
}

pub fn delete_unused_takes(project: &mut Project, track: TrackId) -> Result<usize, CommandError> {
    let folders = take_folders(project.track(track).ok_or(CommandError::NoSuchTrack)?);
    let mut removed = 0;
    for folder in folders.iter().filter(|folder| folder.used.len() < folder.count) {
        for id in &folder.clips {
            let clip = project.clip(*id).ok_or(CommandError::NoSuchClip)?;
            let offsets: Vec<i64> = folder.used.iter().map(|take| clip.offset as i64 + clip.takes[*take]).collect();
            let active = folder.used.iter().position(|take| *take == clip.take).ok_or(CommandError::InvalidValue)?;
            project.apply(Command::SetTakes { clip: *id, offsets, active })?;
        }
        removed += folder.count - folder.used.len();
    }
    Ok(removed)
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

pub fn comp(project: &mut Project, track: TrackId, take: usize, from: Frames, to: Frames, fade: Fade) -> Result<Vec<ClipId>, CommandError> {
    let touched: Vec<ClipId> = project
        .track(track)
        .ok_or(CommandError::NoSuchTrack)?
        .clips
        .iter()
        .filter(|clip| clip.take_offset(take).is_some() && clip.start < to && clip.end() > from)
        .map(|clip| clip.id)
        .collect();
    let mut chosen = Vec::new();
    for clip in touched {
        let middle = middle_of(project, clip, from, to, fade, true)?;
        if project.apply(Command::UseTake { clip: middle, take }).is_ok() {
            chosen.push(middle);
        }
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
        let Ok(Outcome::Clip(clip)) = project.apply(Command::AddClip { track, source: passes, start: 0 }) else { panic!() };
        let last = (PASSES - 1) as Frames * PASS;
        project.apply(Command::TrimClip { clip, offset: last, len: PASS }).unwrap();
        let offsets = (0..PASSES).map(|pass| pass as i64 * PASS as i64).collect();
        project.apply(Command::SetTakes { clip, offsets, active: PASSES - 1 }).unwrap();
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
        let used: Vec<usize> = app.project.track(track).unwrap().clips.iter().map(|piece| piece.take).collect();
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
        let pieces_before = app.project.track(track).unwrap().clips.len();
        assert_eq!(unused_takes(app.project.track(track).unwrap()), 2, "takes 2 and 4 are not in the comp");
        let _ = app.update(Message::DeleteUnusedTakes(track));
        let pieces = &app.project.track(track).unwrap().clips;
        assert_eq!(pieces.len(), pieces_before, "deleting takes cut or removed comp pieces");
        assert!(pieces.iter().all(|piece| piece.takes.len() == 3), "every piece keeps the three takes the comp uses");
        assert_eq!(unused_takes(app.project.track(track).unwrap()), 0);
        assert_eq!(heard(&app.project), before, "the comp sounds different after deleting the unused takes");
        assert!(file.exists(), "the recording was deleted from disk");
        let _ = app.update(Message::Undo);
        assert!(app.project.track(track).unwrap().clips.iter().all(|piece| piece.takes.len() == PASSES), "undo did not bring the takes back");
        assert_eq!(heard(&app.project), before);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_comp_down_to_one_take_becomes_an_ordinary_clip_and_nothing_unused_leaves_no_undo_step() {
        let mut project = Project::new(48_000);
        let (track, clip) = a_looped_vocal(&mut project, Arc::new(Source::from_frames("Vox (take 1)", passes_of())));
        let before = heard(&project);
        assert_eq!(delete_unused_takes(&mut project, track).unwrap(), PASSES - 1);
        let kept = project.clip(clip).unwrap();
        assert!(!kept.has_takes() && kept.take == 0);
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

