use crate::instrument::{Instrument, Note};
use crate::model::{Clip, ClipId, Frames, Project, Track, TrackId};

pub trait Chains: Send {
    fn process(&mut self, track: TrackId, audio: &mut [[f32; 2]]) {
        self.process_with(track, audio, &[]);
    }

    fn process_with(&mut self, track: TrackId, audio: &mut [[f32; 2]], side: &[[f32; 2]]) {
        let _ = side;
        self.process(track, audio);
    }

    fn follow(&mut self, project: &Project) -> Vec<String> {
        let _ = project;
        Vec::new()
    }

    fn troubles(&self) -> Vec<(TrackId, String, String)> {
        Vec::new()
    }

    fn harvest(&mut self) -> Vec<(TrackId, usize, Vec<u8>)> {
        Vec::new()
    }

    fn show(&mut self, track: TrackId, slot: usize) -> Result<(), String> {
        let _ = (track, slot);
        Err("plugin windows are not wired up".into())
    }

    fn tweak(&mut self, track: TrackId, slot: usize, knob: usize, value: f32) {
        let _ = (track, slot, knob, value);
    }

    fn latency(&self, track: TrackId) -> usize {
        let _ = track;
        0
    }

    fn process_clip(&mut self, clip: ClipId, audio: &mut [[f32; 2]]) {
        let _ = (clip, audio);
    }

    fn clip_latency(&self, clip: ClipId) -> usize {
        let _ = clip;
        0
    }

    fn harvest_clips(&mut self) -> Vec<(ClipId, usize, Vec<u8>)> {
        Vec::new()
    }

    fn tweak_clip(&mut self, clip: ClipId, slot: usize, knob: usize, value: f32) {
        let _ = (clip, slot, knob, value);
    }

    fn automate(&mut self, track: TrackId, slot: usize, knob: usize, value: f32) {
        let _ = (track, slot, knob, value);
    }

    fn automate_clip(&mut self, clip: ClipId, slot: usize, knob: usize, value: f32) {
        let _ = (clip, slot, knob, value);
    }

    fn show_clip(&mut self, clip: ClipId, slot: usize) -> Result<(), String> {
        let _ = (clip, slot);
        Err("plugin windows are not wired up".into())
    }
}

pub fn render(project: &Project, pos: Frames, out: &mut [[f32; 2]]) {
    mix_tracks(project, pos, out, None);
    let target = crate::envelope::Target::MasterGain;
    let opens = automated(project, target, pos, project.master);
    let closes = automated(project, target, pos + out.len() as Frames, project.master);
    let (opens, closes) = if project.master_muted { (0.0, 0.0) } else { (opens, closes) };
    scale(out, opens, closes);
}

pub fn render_through(
    project: &Project,
    pos: Frames,
    out: &mut [[f32; 2]],
    scratch: &mut Mixdown,
    chains: Option<&mut (dyn Chains + '_)>,
) {
    mix_tracks_metered(project, pos, out, None, None, scratch, chains);
    let target = crate::envelope::Target::MasterGain;
    let opens = automated(project, target, pos, project.master);
    let closes = automated(project, target, pos + out.len() as Frames, project.master);
    let (opens, closes) = if project.master_muted { (0.0, 0.0) } else { (opens, closes) };
    scale(out, opens, closes);
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

#[derive(Default)]
pub struct Mixdown {
    buffers: Vec<Vec<[f32; 2]>>,
    sides: Vec<Vec<[f32; 2]>>,
    apart: Vec<[f32; 2]>,
    pre: Vec<[f32; 2]>,
    order: Vec<TrackId>,
    index: Vec<usize>,
}

impl Mixdown {
    pub fn room_for(&mut self, tracks: usize, frames: usize) {
        if self.buffers.len() < tracks {
            self.buffers.resize_with(tracks, Vec::new);
        }
        if self.sides.len() < tracks {
            self.sides.resize_with(tracks, Vec::new);
        }
        for buffer in self.buffers.iter_mut().take(tracks).chain(self.sides.iter_mut().take(tracks)) {
            if buffer.len() < frames {
                buffer.resize(frames, [0.0; 2]);
            }
        }
        if self.pre.len() < frames {
            self.pre.resize(frames, [0.0; 2]);
        }
    }
}

pub fn mix_tracks(project: &Project, pos: Frames, out: &mut [[f32; 2]], only: Option<ClipId>) {
    let mut scratch = Mixdown::default();
    mix_tracks_metered(project, pos, out, only, None, &mut scratch, None);
}

pub fn mix_tracks_metered(
    project: &Project,
    pos: Frames,
    out: &mut [[f32; 2]],
    only: Option<ClipId>,
    mut peaks: Option<&mut [f32]>,
    scratch: &mut Mixdown,
    mut chains: Option<&mut (dyn Chains + '_)>,
) {
    out.fill([0.0; 2]);
    let len = out.len();
    let count = project.tracks.len();
    if let Some(racks) = chains.as_deref_mut() {
        if !project.envelopes.is_empty() {
            turn_knobs(project, pos, racks);
        }
    }
    scratch.room_for(count, len);
    scratch.order.clear();
    match project.render_order() {
        Some(order) => scratch.order.extend(order),
        None => return,
    }
    scratch.index.clear();
    for id in &scratch.order {
        scratch.index.push(project.tracks.iter().position(|t| t.id == *id).unwrap_or(usize::MAX));
    }
    let ahead: Vec<Frames> = project
        .tracks
        .iter()
        .map(|track| match chains.as_deref() {
            Some(racks) => head_start(project, track.id, racks) as Frames,
            None => 0,
        })
        .collect();
    for (index, track) in project.tracks.iter().enumerate() {
        scratch.sides[index][..len].fill([0.0; 2]);
        let buffer = &mut scratch.buffers[index][..len];
        buffer.fill([0.0; 2]);
        lay_clips(project, track, pos + ahead[index], buffer, only, chains.as_deref_mut(), &mut scratch.apart);
    }
    for step in 0..scratch.order.len() {
        let index = scratch.index[step];
        if index == usize::MAX {
            continue;
        }
        let track = &project.tracks[index];
        if let Some(racks) = chains.as_deref_mut() {
            if !track.fx.is_empty() {
                let (mains, sides) = (&mut scratch.buffers, &scratch.sides);
                racks.process_with(track.id, &mut mains[index][..len], &sides[index][..len]);
            }
        }
        let silent = track.muted && only.is_none();
        let keep_pre = track.sends.iter().any(|send| send.pre_fader);
        if keep_pre {
            scratch.pre[..len].copy_from_slice(&scratch.buffers[index][..len]);
        }
        let target = crate::envelope::Target::TrackGain(track.id);
        let opens = automated(project, target, pos, track.gain);
        let closes = automated(project, target, pos + len as Frames, track.gain);
        let (opens, closes) = if silent { (0.0, 0.0) } else { (opens, closes) };
        let step = (closes - opens) / len.max(1) as f32;
        for (i, frame) in scratch.buffers[index][..len].iter_mut().enumerate() {
            let gain = opens + step * (i + 1) as f32;
            frame[0] *= gain;
            frame[1] *= gain;
        }
        if let Some(slot) = peaks.as_deref_mut().and_then(|p| p.get_mut(index)) {
            let mut top = 0.0f32;
            for frame in &scratch.buffers[index][..len] {
                top = top.max(frame[0].abs()).max(frame[1].abs());
            }
            *slot = top;
        }
        for send in &track.sends {
            let Some(target) = project.tracks.iter().position(|t| t.id == send.to) else {
                continue;
            };
            let gain = automated(
                project,
                crate::envelope::Target::SendGain { from: track.id, to: send.to },
                pos,
                send.gain,
            );
            for i in 0..len {
                let from = if send.pre_fader { scratch.pre[i] } else { scratch.buffers[index][i] };
                let dest = if send.sidechain { &mut scratch.sides[target][i] } else { &mut scratch.buffers[target][i] };
                dest[0] += from[0] * gain;
                dest[1] += from[1] * gain;
            }
        }
        match track.parent.and_then(|parent| project.tracks.iter().position(|t| t.id == parent)) {
            Some(target) => {
                for i in 0..len {
                    let from = scratch.buffers[index][i];
                    let dest = &mut scratch.buffers[target][i];
                    dest[0] += from[0];
                    dest[1] += from[1];
                }
            }
            None => {
                for (dest, from) in out.iter_mut().zip(&scratch.buffers[index][..len]) {
                    dest[0] += from[0];
                    dest[1] += from[1];
                }
            }
        }
    }
}

fn lay_clips(
    project: &Project,
    track: &Track,
    pos: Frames,
    out: &mut [[f32; 2]],
    only: Option<ClipId>,
    mut chains: Option<&mut (dyn Chains + '_)>,
    apart: &mut Vec<[f32; 2]>,
) {
    let rate = project.rate;
    for clip in &track.clips {
        let ahead = match chains.as_deref() {
            Some(racks) if !clip.fx.is_empty() => racks.clip_latency(clip.id) as Frames,
            _ => 0,
        };
        let pos = pos + ahead;
        let end = pos + out.len() as Frames;
        let silenced = match only {
            Some(chosen) => clip.id != chosen || clip.muted,
            None => clip.muted,
        };
        if silenced || clip.end() <= pos || clip.start >= end {
            continue;
        }
        let from = clip.start.max(pos);
        let to = clip.end().min(end);
        let source = clip.audio();
        let source_from = ((clip.offset + (from - clip.start)) as usize).min(source.len());
        let count = match clip.notes {
            Some(_) => (to - from) as usize,
            None => ((to - from) as usize).min(source.len() - source_from),
        };
        let gain = clip_automated(project, crate::envelope::Target::ClipGain(clip.id), pos, clip.start, clip.gain);
        let own = !clip.fx.is_empty() && chains.is_some();
        if own {
            apart.clear();
            apart.resize(out.len(), [0.0; 2]);
        }
        let target = match own {
            true => &mut apart[(from - pos) as usize..][..count],
            false => &mut out[(from - pos) as usize..][..count],
        };
        let first = from - clip.start;
        if let Some(notes) = &clip.notes {
            play_notes(track.instrument, notes, clip, first, target, rate);
        } else {
        let audio = &source[source_from..][..count];
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
        if own {
            if let Some(racks) = chains.as_deref_mut() {
                racks.process_clip(clip.id, apart);
            }
            for (into, from) in out.iter_mut().zip(apart.iter()) {
                into[0] += from[0];
                into[1] += from[1];
            }
        }
    }
}

fn play_notes(instrument: Instrument, notes: &[Note], clip: &Clip, first: Frames, target: &mut [[f32; 2]], rate: u32) {
    let tail = instrument.tail(rate);
    let window_from = clip.offset + first;
    let window_to = window_from + target.len() as Frames;
    let content_end = clip.offset + clip.len;
    for note in notes {
        if note.start >= window_to {
            break;
        }
        if note.start >= content_end || note.end() + tail <= window_from {
            continue;
        }
        let from = note.start.max(window_from);
        let to = (note.end() + tail).min(window_to);
        if to <= from {
            continue;
        }
        let into = (from - window_from) as usize;
        let span = &mut target[into..(to - window_from) as usize];
        let level = |i: usize| clip.gain * clip.fade_level(first + (into + i) as Frames);
        instrument.play(note, from - note.start, span, level, rate);
    }
}

fn mix(target: &mut [[f32; 2]], audio: &[[f32; 2]], gain: f32) -> f32 {
    let mut top = 0.0f32;
    for (o, s) in target.iter_mut().zip(audio) {
        let (l, r) = (s[0] * gain, s[1] * gain);
        o[0] += l;
        o[1] += r;
        top = top.max(l.abs()).max(r.abs());
    }
    top
}

fn mix_faded(target: &mut [[f32; 2]], audio: &[[f32; 2]], gain: f32, clip: &Clip, first: Frames) -> f32 {
    let mut top = 0.0f32;
    for (i, (o, s)) in target.iter_mut().zip(audio).enumerate() {
        let level = gain * clip.fade_level(first + i as Frames);
        let (l, r) = (s[0] * level, s[1] * level);
        o[0] += l;
        o[1] += r;
        top = top.max(l.abs()).max(r.abs());
    }
    top
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
    fn a_track_envelope_moves_the_level_while_it_plays() {
        use crate::envelope::{Point, Shape, Target};
        let mut p = Project::new(48_000);
        let one = track(&mut p);
        flat(&mut p, one, 1.0, 400);
        let target = Target::TrackGain(one);
        p.apply(Command::AddEnvelope { target }).unwrap();
        p.apply(Command::ClearPoints { target, from: 0, to: 1_000_000 }).unwrap();
        p.apply(Command::PutPoint { target, point: Point { at: 0, value: 0.0, shape: Shape::Linear } }).unwrap();
        p.apply(Command::PutPoint { target, point: Point { at: 100, value: 1.0, shape: Shape::Linear } }).unwrap();
        let mut out = vec![[0.0; 2]; 8];
        let mut scratch = Mixdown::default();
        mix_tracks_metered(&p, 0, &mut out, None, None, &mut scratch, None);
        let quiet = out[4][0];
        mix_tracks_metered(&p, 50, &mut out, None, None, &mut scratch, None);
        let middling = out[4][0];
        mix_tracks_metered(&p, 150, &mut out, None, None, &mut scratch, None);
        let loud = out[4][0];
        assert!(quiet < 0.05, "it started at {quiet}");
        assert!((middling - 0.5).abs() < 0.1, "halfway it read {middling}");
        assert!((loud - 1.0).abs() < 0.01, "at the end it read {loud}");
    }

    struct Knobs {
        seen: Vec<(usize, f32)>,
    }

    impl Chains for Knobs {
        fn process(&mut self, _track: TrackId, _audio: &mut [[f32; 2]]) {}

        fn automate(&mut self, _track: TrackId, _slot: usize, knob: usize, value: f32) {
            self.seen.push((knob, value));
        }
    }

    #[test]
    fn a_plugin_knob_envelope_reaches_the_plugin() {
        use crate::envelope::{Point, Shape, Target};
        let mut p = Project::new(48_000);
        let one = track(&mut p);
        let fx = crate::model::Fx {
            path: std::path::PathBuf::from("x.vst3"),
            index: 0,
            name: "x".into(),
            bypassed: false,
            state: Vec::new(),
        };
        p.apply(Command::AddFx { track: one, fx }).unwrap();
        let target = Target::TrackFx { track: one, slot: 0, knob: 2 };
        p.apply(Command::AddEnvelope { target }).unwrap();
        p.apply(Command::ClearPoints { target, from: 0, to: Frames::MAX }).unwrap();
        p.apply(Command::PutPoint { target, point: Point { at: 0, value: 0.0, shape: Shape::Linear } }).unwrap();
        p.apply(Command::PutPoint { target, point: Point { at: 1000, value: 1.0, shape: Shape::Linear } }).unwrap();
        let mut racks = Knobs { seen: Vec::new() };
        let mut out = vec![[0.0; 2]; 16];
        let mut scratch = Mixdown::default();
        mix_tracks_metered(&p, 500, &mut out, None, None, &mut scratch, Some(&mut racks));
        assert_eq!(racks.seen.len(), 1, "the knob was not turned");
        assert_eq!(racks.seen[0].0, 2, "the wrong knob moved");
        assert!((racks.seen[0].1 - 0.5).abs() < 1e-6, "it was turned to {}", racks.seen[0].1);
    }

    #[test]
    fn a_clip_envelope_travels_with_the_clip() {
        use crate::envelope::{Point, Shape, Target};
        let mut p = Project::new(48_000);
        let one = track(&mut p);
        let moved = clip(&mut p, one, counting(400), 200);
        let target = Target::ClipGain(moved);
        p.apply(Command::AddEnvelope { target }).unwrap();
        p.apply(Command::ClearPoints { target, from: 0, to: 1_000_000 }).unwrap();
        p.apply(Command::PutPoint { target, point: Point { at: 0, value: 0.0, shape: Shape::Hold } }).unwrap();
        p.apply(Command::PutPoint { target, point: Point { at: 100, value: 1.0, shape: Shape::Linear } }).unwrap();
        let mut out = vec![[0.0; 2]; 8];
        let mut scratch = Mixdown::default();
        mix_tracks_metered(&p, 250, &mut out, None, None, &mut scratch, None);
        assert_eq!(out[4][0], 0.0, "the hold did not follow the clip");
        mix_tracks_metered(&p, 320, &mut out, None, None, &mut scratch, None);
        assert!(out[4][0] > 0.0, "after the clip's own 100 frames it should be open");
    }

    struct ClipDoubler {
        on: ClipId,
        ran: usize,
    }

    impl Chains for ClipDoubler {
        fn process(&mut self, _track: TrackId, _audio: &mut [[f32; 2]]) {}

        fn process_clip(&mut self, clip: ClipId, audio: &mut [[f32; 2]]) {
            if clip != self.on {
                return;
            }
            self.ran += 1;
            for frame in audio.iter_mut() {
                frame[0] *= 2.0;
                frame[1] *= 2.0;
            }
        }
    }

    #[test]
    fn a_clip_can_carry_its_own_chain() {
        let mut p = Project::new(48_000);
        let one = track(&mut p);
        let loud = clip(&mut p, one, counting(200), 0);
        let plain = clip(&mut p, one, counting(200), 300);
        let fx = crate::model::Fx {
            path: std::path::PathBuf::from("x.vst3"),
            index: 0,
            name: "x".into(),
            bypassed: false,
            state: Vec::new(),
        };
        p.apply(Command::AddClipFx { clip: loud, fx }).unwrap();
        let mut racks = ClipDoubler { on: loud, ran: 0 };
        let mut out = vec![[0.0; 2]; 600];
        let mut scratch = Mixdown::default();
        mix_tracks_metered(&p, 0, &mut out, None, None, &mut scratch, Some(&mut racks));
        assert_eq!(racks.ran, 1, "the clip chain did not run once");
        assert_eq!(out[50], [100.0, -100.0], "the clip with a chain was not doubled");
        assert_eq!(out[350], [50.0, -50.0], "the clip without a chain changed");
        let _ = plain;
    }

    struct Ducker {
        on: TrackId,
        heard: f32,
    }

    impl Chains for Ducker {
        fn process_with(&mut self, track: TrackId, audio: &mut [[f32; 2]], side: &[[f32; 2]]) {
            if track != self.on {
                return;
            }
            self.heard = side.iter().fold(0.0f32, |most, frame| most.max(frame[0].abs()));
            let duck = 1.0 - self.heard.min(1.0);
            for frame in audio.iter_mut() {
                frame[0] *= duck;
                frame[1] *= duck;
            }
        }
    }

    #[test]
    fn a_sidechain_send_reaches_the_plugin_and_stays_out_of_the_mix() {
        let mut p = Project::new(48_000);
        let beat = track(&mut p);
        let vox = track(&mut p);
        flat(&mut p, beat, 0.8, 64);
        flat(&mut p, vox, 0.5, 64);
        p.apply(Command::AddSend { from: vox, to: beat }).unwrap();
        p.apply(Command::SetSendSidechain { from: vox, to: beat, sidechain: true }).unwrap();
        p.apply(Command::SetSendPreFader { from: vox, to: beat, pre_fader: true }).unwrap();
        p.apply(Command::SetTrackMuted { track: vox, muted: true }).unwrap();
        let fx = crate::model::Fx {
            path: std::path::PathBuf::from("duck.vst3"),
            index: 0,
            name: "duck".into(),
            bypassed: false,
            state: Vec::new(),
        };
        p.apply(Command::AddFx { track: beat, fx }).unwrap();
        let mut racks = Ducker { on: beat, heard: 0.0 };
        let mut out = vec![[0.0; 2]; 32];
        let mut scratch = Mixdown::default();
        mix_tracks_metered(&p, 1, &mut out, None, None, &mut scratch, Some(&mut racks));
        assert!((racks.heard - 0.5).abs() < 1e-6, "the plugin heard {}", racks.heard);
        assert!((out[8][0] - 0.4).abs() < 1e-6, "the mix came out at {}", out[8][0]);
    }

    struct Late {
        on: TrackId,
        by: usize,
        held: Vec<[f32; 2]>,
    }

    impl Chains for Late {
        fn process(&mut self, track: TrackId, audio: &mut [[f32; 2]]) {
            if track != self.on {
                return;
            }
            self.held.resize(self.by, [0.0; 2]);
            let mut out = Vec::with_capacity(audio.len());
            for frame in audio.iter() {
                self.held.push(*frame);
                out.push(self.held.remove(0));
            }
            audio.copy_from_slice(&out);
        }

        fn latency(&self, track: TrackId) -> usize {
            if track == self.on {
                self.by
            } else {
                0
            }
        }
    }

    #[test]
    fn a_slow_plugin_does_not_push_its_track_late() {
        let mut p = Project::new(48_000);
        let slow = track(&mut p);
        let plain = track(&mut p);
        clip(&mut p, slow, counting(400), 100);
        clip(&mut p, plain, counting(400), 100);
        let fx = crate::model::Fx {
            path: std::path::PathBuf::from("slow.vst3"),
            index: 0,
            name: "slow".into(),
            bypassed: false,
            state: Vec::new(),
        };
        p.apply(Command::AddFx { track: slow, fx }).unwrap();
        let mut racks = Late { on: slow, by: 32, held: Vec::new() };
        let mut out = vec![[0.0; 2]; 300];
        let mut scratch = Mixdown::default();
        mix_tracks_metered(&p, 0, &mut out, None, None, &mut scratch, Some(&mut racks));
        assert_eq!(out[101], [2.0, -2.0], "the two tracks did not land together");
        assert_eq!(out[150], [100.0, -100.0]);
    }

    #[test]
    fn with_no_plugins_nothing_moves() {
        let mut p = Project::new(48_000);
        let one = track(&mut p);
        clip(&mut p, one, counting(400), 100);
        let mut racks = Late { on: one, by: 32, held: Vec::new() };
        let mut out = vec![[0.0; 2]; 300];
        let mut scratch = Mixdown::default();
        mix_tracks_metered(&p, 0, &mut out, None, None, &mut scratch, Some(&mut racks));
        assert_eq!(out[101], [1.0, -1.0]);
    }

    struct Doubler {
        seen: Vec<TrackId>,
    }

    impl Chains for Doubler {
        fn process(&mut self, track: TrackId, audio: &mut [[f32; 2]]) {
            self.seen.push(track);
            for frame in audio.iter_mut() {
                frame[0] *= 2.0;
                frame[1] *= 2.0;
            }
        }
    }

    fn flat(p: &mut Project, track: TrackId, level: f32, frames: usize) {
        let source = Arc::new(Source::from_frames("flat", vec![[level, level]; frames + 2]));
        clip(p, track, source, 0);
    }

    #[test]
    fn a_chain_runs_before_the_fader_and_feeds_the_sends() {
        let mut p = Project::new(48_000);
        let vox = track(&mut p);
        let verb = track(&mut p);
        flat(&mut p, vox, 0.25, 64);
        p.apply(Command::SetTrackGain { track: vox, gain: 0.5 }).unwrap();
        p.apply(Command::AddSend { from: vox, to: verb }).unwrap();
        p.apply(Command::SetSendGain { from: vox, to: verb, gain: 1.0 }).unwrap();
        p.apply(Command::SetSendPreFader { from: vox, to: verb, pre_fader: true }).unwrap();
        let fx = crate::model::Fx {
            path: std::path::PathBuf::from("x.vst3"),
            index: 0,
            name: "x".into(),
            bypassed: false,
            state: Vec::new(),
        };
        p.apply(Command::AddFx { track: vox, fx }).unwrap();
        let mut racks = Doubler { seen: Vec::new() };
        let mut out = vec![[0.0; 2]; 32];
        let mut scratch = Mixdown::default();
        mix_tracks_metered(&p, 1, &mut out, None, None, &mut scratch, Some(&mut racks));
        assert_eq!(racks.seen, vec![vox]);
        assert!((out[8][0] - 0.75).abs() < 1e-6, "got {}", out[8][0]);
    }

    #[test]
    fn a_track_with_no_plugins_is_never_handed_to_the_chains() {
        let mut p = Project::new(48_000);
        let bare = track(&mut p);
        flat(&mut p, bare, 0.5, 64);
        let mut racks = Doubler { seen: Vec::new() };
        let mut out = vec![[0.0; 2]; 32];
        let mut scratch = Mixdown::default();
        mix_tracks_metered(&p, 1, &mut out, None, None, &mut scratch, Some(&mut racks));
        assert!(racks.seen.is_empty());
        assert!((out[8][0] - 0.5).abs() < 1e-6, "got {}", out[8][0]);
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

#[cfg(test)]
mod master_mute {
    use crate::model::{Command, Project};
    use crate::render::render;

    #[test]
    fn a_muted_master_renders_silence() {
        let mut project = Project::new(48_000);
        project.apply(Command::AddTrack { name: "t".into() }).unwrap();
        let mut out = vec![[0.5f32; 2]; 64];
        project.apply(Command::ToggleMasterMute).unwrap();
        render(&project, 0, &mut out);
        assert!(out.iter().all(|frame| frame[0] == 0.0 && frame[1] == 0.0));
    }
}

#[cfg(test)]
mod routing {
    use super::*;
    use crate::model::{Command, Outcome};
    use crate::source::Source;
    use std::sync::Arc;

    fn ones(len: usize) -> Arc<Source> {
        Arc::new(Source::from_frames("ones", vec![[1.0, 1.0]; len]))
    }

    fn track(p: &mut Project) -> TrackId {
        match p.apply(Command::AddTrack { name: "T".into() }) {
            Ok(Outcome::Track(id)) => id,
            other => panic!("{other:?}"),
        }
    }

    fn heard(project: &Project, len: usize) -> Vec<[f32; 2]> {
        let mut out = vec![[0.0; 2]; len];
        mix_tracks(project, 0, &mut out, None);
        out
    }

    #[test]
    fn a_folder_fader_scales_every_child() {
        let mut p = Project::new(48_000);
        let folder = track(&mut p);
        let child = track(&mut p);
        p.apply(Command::AddClip { track: child, source: ones(8), start: 0 }).unwrap();
        p.apply(Command::SetTrackParent { track: child, parent: Some(folder) }).unwrap();
        assert_eq!(heard(&p, 4)[0][0], 1.0);
        p.apply(Command::SetTrackGain { track: folder, gain: 0.5 }).unwrap();
        assert_eq!(heard(&p, 4)[0][0], 0.5);
    }

    #[test]
    fn muting_a_folder_silences_its_children() {
        let mut p = Project::new(48_000);
        let folder = track(&mut p);
        let child = track(&mut p);
        p.apply(Command::AddClip { track: child, source: ones(8), start: 0 }).unwrap();
        p.apply(Command::SetTrackParent { track: child, parent: Some(folder) }).unwrap();
        p.apply(Command::SetTrackMuted { track: folder, muted: true }).unwrap();
        assert_eq!(heard(&p, 4)[0][0], 0.0);
    }

    #[test]
    fn a_send_adds_the_signal_to_its_target() {
        let mut p = Project::new(48_000);
        let from = track(&mut p);
        let to = track(&mut p);
        p.apply(Command::AddClip { track: from, source: ones(8), start: 0 }).unwrap();
        p.apply(Command::AddSend { from, to }).unwrap();
        p.apply(Command::SetSendGain { from, to, gain: 0.5 }).unwrap();
        assert_eq!(heard(&p, 4)[0][0], 1.5);
    }

    #[test]
    fn a_pre_fader_send_ignores_the_track_fader() {
        let mut p = Project::new(48_000);
        let from = track(&mut p);
        let to = track(&mut p);
        p.apply(Command::AddClip { track: from, source: ones(8), start: 0 }).unwrap();
        p.apply(Command::AddSend { from, to }).unwrap();
        p.apply(Command::SetSendPreFader { from, to, pre_fader: true }).unwrap();
        p.apply(Command::SetTrackGain { track: from, gain: 0.0 }).unwrap();
        assert_eq!(heard(&p, 4)[0][0], 1.0);
    }

    #[test]
    fn a_send_that_would_feed_back_is_refused() {
        let mut p = Project::new(48_000);
        let a = track(&mut p);
        let b = track(&mut p);
        p.apply(Command::AddSend { from: a, to: b }).unwrap();
        assert!(p.apply(Command::AddSend { from: b, to: a }).is_err());
    }

    #[test]
    fn a_track_cannot_sit_inside_its_own_child() {
        let mut p = Project::new(48_000);
        let parent = track(&mut p);
        let child = track(&mut p);
        p.apply(Command::SetTrackParent { track: child, parent: Some(parent) }).unwrap();
        assert!(p.apply(Command::SetTrackParent { track: parent, parent: Some(child) }).is_err());
    }
}

fn head_start(project: &Project, track: TrackId, racks: &dyn Chains) -> usize {
    let running = |id: TrackId| match project.tracks.iter().find(|t| t.id == id) {
        Some(found) if !found.fx.is_empty() => racks.latency(id),
        _ => 0,
    };
    let mut total = running(track);
    let mut at = track;
    let mut guard = 0;
    while let Some(parent) = project.tracks.iter().find(|t| t.id == at).and_then(|t| t.parent) {
        total += running(parent);
        at = parent;
        guard += 1;
        if guard > project.tracks.len() {
            break;
        }
    }
    total
}

fn automated(project: &Project, target: crate::envelope::Target, at: Frames, fallback: f32) -> f32 {
    match project.envelope(target).and_then(|shape| shape.value_at(at)) {
        Some(value) => value,
        None => fallback,
    }
}

fn clip_automated(project: &Project, target: crate::envelope::Target, at: Frames, start: Frames, fallback: f32) -> f32 {
    match project.envelope(target) {
        Some(shape) => shape.value_at(at.saturating_sub(start)).unwrap_or(fallback),
        None => fallback,
    }
}

fn turn_knobs(project: &Project, at: Frames, racks: &mut (dyn Chains + '_)) {
    for shape in &project.envelopes {
        let Some(value) = shape.value_at(at) else { continue };
        match shape.target {
            crate::envelope::Target::TrackFx { track, slot, knob } => racks.automate(track, slot, knob, value),
            crate::envelope::Target::ClipFx { clip, slot, knob } => racks.automate_clip(clip, slot, knob, value),
            _ => {}
        }
    }
}

#[cfg(test)]
mod note_tests {
    use super::*;
    use crate::file::SavedProject;
    use crate::model::{Command, Outcome};

    const RATE: u32 = 48_000;

    fn song(instrument: Instrument, notes: Vec<Note>) -> (Project, ClipId) {
        let mut project = Project::new(RATE);
        let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: "Keys".into() }) else { panic!("no track") };
        project.apply(Command::SetInstrument { track, instrument }).unwrap();
        let made = Command::AddNotesClip { track, name: "Melody".into(), start: 4_800, len: 48_000, notes };
        let Ok(Outcome::Clip(clip)) = project.apply(made) else { panic!("no clip") };
        (project, clip)
    }

    fn heard(project: &Project, frames: usize) -> Vec<[f32; 2]> {
        let mut out = vec![[0.0; 2]; frames];
        render(project, 0, &mut out);
        out
    }

    fn loudness(audio: &[[f32; 2]]) -> f32 {
        audio.iter().map(|f| f[0].abs()).fold(0.0, f32::max)
    }

    #[test]
    fn a_note_sounds_where_it_sits_in_its_clip() {
        let (project, _) = song(Instrument::default(), vec![Note { key: 60, start: 9_600, len: 4_800, velocity: 1.0 }]);
        let out = heard(&project, 60_000);
        assert_eq!(loudness(&out[..4_800 + 9_600]), 0.0, "silent before the note");
        assert!(loudness(&out[14_400..19_200]) > 0.05);
        assert_eq!(project.length(), 4_800 + 48_000);
    }

    #[test]
    fn a_note_clip_plays_the_same_in_any_block_size_and_through_a_split() {
        let notes = vec![
            Note { key: 48, start: 0, len: 20_000, velocity: 0.9 },
            Note { key: 55, start: 10_000, len: 30_000, velocity: 0.7 },
        ];
        let (mut project, clip) = song(Instrument::default(), notes);
        let whole = heard(&project, 60_000);
        let mut pieces = vec![[0.0f32; 2]; 60_000];
        for (n, part) in pieces.chunks_mut(1000).enumerate() {
            render(&project, (n * 1000) as Frames, part);
        }
        assert_eq!(whole, pieces);
        project.apply(Command::SplitClip { clip, at: 25_000 }).unwrap();
        let split = heard(&project, 60_000);
        assert!(whole.iter().zip(&split).take(25_000).all(|(a, b)| a == b));
        let after: f32 = whole[25_000..40_000].iter().zip(&split[25_000..40_000]).map(|(a, b)| (a[0] - b[0]).abs()).fold(0.0, f32::max);
        assert!(after < 1e-6, "the right piece carries on the held notes, differs by {after}");
    }

    #[test]
    fn muting_and_clip_gain_apply_to_notes_too() {
        let (mut project, clip) = song(Instrument::Drums, vec![Note { key: 36, start: 0, len: 100, velocity: 1.0 }]);
        let full = loudness(&heard(&project, 20_000));
        project.apply(Command::SetClipGain { clip, gain: 0.5 }).unwrap();
        let half = loudness(&heard(&project, 20_000));
        assert!((half / full - 0.5).abs() < 1e-3);
        project.apply(Command::SetClipMuted { clip, muted: true }).unwrap();
        assert_eq!(loudness(&heard(&project, 20_000)), 0.0);
    }

    #[test]
    fn notes_and_instruments_survive_saving() {
        let notes = vec![Note { key: 62, start: 1_000, len: 2_000, velocity: 0.75 }, Note { key: 36, start: 0, len: 10, velocity: 1.0 }];
        let (mut project, clip) = song(Instrument::Drums, notes);
        project.apply(Command::TrimClip { clip, offset: 500, len: 9_000 }).unwrap();
        let text = SavedProject::capture(&project, |_| None).to_text();
        assert!(text.contains("instrument=drums") && text.contains("notes start=4800") && text.contains("note key=36"));
        let (back, _) = SavedProject::parse(&text).unwrap().build(&[], RATE);
        let clip = back.clips().next().unwrap();
        assert_eq!(clip.source.name, "Melody");
        assert_eq!((clip.start, clip.offset, clip.len), (4_800, 500, 9_000));
        assert_eq!(clip.notes.as_deref().unwrap(), &project.clips().next().unwrap().notes.as_deref().unwrap()[..]);
        assert_eq!(back.tracks[0].instrument, Instrument::Drums);
        assert_eq!(heard(&back, 20_000), heard(&project, 20_000));
        let synth = Instrument::Synth(crate::instrument::Synth { attack: 0.02, ..Default::default() });
        let (with_synth, _) = song(synth, Vec::new());
        let text = SavedProject::capture(&with_synth, |_| None).to_text();
        let (back, _) = SavedProject::parse(&text).unwrap().build(&[], RATE);
        assert_eq!(back.tracks[0].instrument, synth);
    }

    #[test]
    fn notes_are_kept_in_order_and_bad_ones_dropped() {
        let notes = vec![
            Note { key: 64, start: 900, len: 10, velocity: 2.0 },
            Note { key: 60, start: 100, len: 10, velocity: 0.5 },
            Note { key: 61, start: 50, len: 0, velocity: 0.5 },
        ];
        let (project, _) = song(Instrument::default(), notes);
        let kept = project.clips().next().unwrap().notes.clone().unwrap();
        assert_eq!(kept.iter().map(|n| n.key).collect::<Vec<_>>(), [60, 64]);
        assert_eq!(kept[1].velocity, 1.0);
    }
}
