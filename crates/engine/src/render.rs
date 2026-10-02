use crate::model::{Frames, Project};

pub fn render(project: &Project, pos: Frames, out: &mut [[f32; 2]]) {
    out.fill([0.0; 2]);
    let end = pos + out.len() as Frames;
    for track in &project.tracks {
        if track.muted {
            continue;
        }
        for clip in &track.clips {
            if clip.end() <= pos || clip.start >= end {
                continue;
            }
            let from = clip.start.max(pos);
            let to = clip.end().min(end);
            let source = &clip.source.frames;
            let source_from = ((clip.offset + (from - clip.start)) as usize).min(source.len());
            let count = ((to - from) as usize).min(source.len() - source_from);
            let gain = clip.gain * track.gain;
            let target = &mut out[(from - pos) as usize..][..count];
            for (o, s) in target.iter_mut().zip(&source[source_from..][..count]) {
                o[0] += s[0] * gain;
                o[1] += s[1] * gain;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ClipId, Command, Outcome, TrackId};
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
