use crate::model::{Clip, ClipId, Frames, Project};

pub fn render(project: &Project, pos: Frames, out: &mut [[f32; 2]]) {
    mix_tracks(project, pos, out, None);
    scale(out, project.master, project.master);
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

pub fn mix_tracks(project: &Project, pos: Frames, out: &mut [[f32; 2]], only: Option<ClipId>) {
    out.fill([0.0; 2]);
    let end = pos + out.len() as Frames;
    for track in &project.tracks {
        if track.muted && only.is_none() {
            continue;
        }
        for clip in &track.clips {
            let silenced = match only {
                Some(chosen) => clip.id != chosen || clip.muted,
                None => clip.muted,
            };
            if silenced || clip.end() <= pos || clip.start >= end {
                continue;
            }
            let from = clip.start.max(pos);
            let to = clip.end().min(end);
            let source = &clip.source.frames;
            let source_from = ((clip.offset + (from - clip.start)) as usize).min(source.len());
            let count = ((to - from) as usize).min(source.len() - source_from);
            let gain = clip.gain * track.gain;
            let target = &mut out[(from - pos) as usize..][..count];
            let audio = &source[source_from..][..count];
            let first = from - clip.start;
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
    }
}

fn mix(target: &mut [[f32; 2]], audio: &[[f32; 2]], gain: f32) {
    for (o, s) in target.iter_mut().zip(audio) {
        o[0] += s[0] * gain;
        o[1] += s[1] * gain;
    }
}

fn mix_faded(target: &mut [[f32; 2]], audio: &[[f32; 2]], gain: f32, clip: &Clip, first: Frames) {
    for (i, (o, s)) in target.iter_mut().zip(audio).enumerate() {
        let level = gain * clip.fade_level(first + i as Frames);
        o[0] += s[0] * level;
        o[1] += s[1] * level;
    }
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
}
