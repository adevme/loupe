use std::collections::HashMap;

use loupe_engine::{Frames, Project, Target, TrackId};

pub type KnobNames = HashMap<(u64, usize, usize, bool), String>;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Unit {
    Fader,
    Pan,
    Knob,
}

fn unit_of(target: Target) -> Unit {
    match target {
        Target::TrackGain(_) | Target::MasterGain | Target::SendGain { .. } | Target::ClipGain(_) => Unit::Fader,
        Target::TrackPan(_) => Unit::Pan,
        Target::TrackFx { .. } | Target::MasterFx { .. } | Target::ClipFx { .. } => Unit::Knob,
    }
}

fn signed(number: f32, places: usize) -> String {
    let shown = format!("{:.*}", places, number.abs());
    let zero = shown.chars().all(|c| c == '0' || c == '.');
    if zero {
        shown
    } else if number < 0.0 {
        format!("\u{2212}{shown}")
    } else {
        format!("+{shown}")
    }
}

fn decibels_text(gain: f32) -> String {
    if gain <= 0.0 {
        return "\u{2212}inf dB".into();
    }
    format!("{} dB", signed(20.0 * gain.log10(), 1))
}

fn fader_text(gain: f32) -> String {
    format!("{:.0} %  {}", gain.max(0.0) * 100.0, decibels_text(gain))
}

fn pan_text(pan: f32) -> String {
    let percent = (pan.clamp(-1.0, 1.0) * 100.0).round() as i32;
    match percent {
        0 => "C".into(),
        p if p < 0 => format!("L {} %", -p),
        p => format!("R {p} %"),
    }
}

fn knob_text(value: f32) -> String {
    format!("{:.0} %", value.clamp(0.0, 1.0) * 100.0)
}

fn value_text(target: Target, value: f32) -> String {
    match unit_of(target) {
        Unit::Fader => fader_text(value),
        Unit::Pan => pan_text(value),
        Unit::Knob => knob_text(value),
    }
}

pub fn bar_beat(at: Frames, rate: u32, bpm: f64) -> String {
    let seconds = at as f64 / rate.max(1) as f64;
    let beats = seconds * bpm / 60.0;
    format!("{}.{}", (beats / 4.0) as u64 + 1, beats as u64 % 4 + 1)
}

fn knob_name(knobs: &KnobNames, owner: u64, slot: usize, knob: usize, on_clip: bool) -> String {
    match knobs.get(&(owner, slot, knob, on_clip)) {
        Some(found) => found.clone(),
        None => format!("knob {knob}"),
    }
}

pub fn lane_label(project: &Project, knobs: &KnobNames, target: Target) -> String {
    let track_name = |want: TrackId| project.tracks.iter().find(|t| t.id == want).map(|t| t.name.clone()).unwrap_or_default();
    match target {
        Target::MasterGain => "Master volume".into(),
        Target::TrackGain(_) => "Volume".into(),
        Target::TrackPan(_) => "Pan".into(),
        Target::SendGain { to, .. } => format!("Send to {}", track_name(to)),
        Target::TrackFx { slot, knob, track } => {
            let name = project
                .tracks
                .iter()
                .find(|t| t.id == track)
                .and_then(|t| t.fx.get(slot))
                .map(|fx| fx.name.clone())
                .unwrap_or_default();
            format!("{name}: {}", knob_name(knobs, track.0, slot, knob, false))
        }
        Target::MasterFx { slot, knob } => {
            let name = project.master_fx.get(slot).map(|fx| fx.name.clone()).unwrap_or_default();
            format!("Master {name}: {}", knob_name(knobs, crate::MASTER_OWNER, slot, knob, false))
        }
        Target::ClipGain(_) => "Clip gain".into(),
        Target::ClipFx { clip, slot, knob } => {
            let name = project.clip(clip).and_then(|found| found.fx.get(slot)).map(|fx| fx.name.clone()).unwrap_or_default();
            format!("Clip {name}: {}", knob_name(knobs, clip.0, slot, knob, true))
        }
    }
}

pub fn point_reading(project: &Project, knobs: &KnobNames, target: Target, at: Frames, value: f32) -> String {
    let offset = target.on_clip().and_then(|clip| project.clip(clip)).map_or(0, |clip| clip.start);
    format!(
        "{}  {}  at {}",
        lane_label(project, knobs, target),
        value_text(target, value),
        bar_beat(at + offset, project.rate, project.bpm)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOOK: TrackId = TrackId(1);
    const THICKNER: TrackId = TrackId(2);

    #[test]
    fn faders_and_sends_read_as_percent_and_decibels() {
        let send = Target::SendGain { from: HOOK, to: THICKNER };
        assert_eq!(value_text(send, 0.12), "12 %  \u{2212}18.4 dB");
        assert_eq!(value_text(Target::TrackGain(HOOK), 1.0), "100 %  0.0 dB");
        assert_eq!(value_text(Target::TrackGain(HOOK), 2.0), "200 %  +6.0 dB");
        assert_eq!(value_text(Target::MasterGain, 0.5), "50 %  \u{2212}6.0 dB");
        assert_eq!(value_text(Target::ClipGain(loupe_engine::ClipId(3)), 0.25), "25 %  \u{2212}12.0 dB");
    }

    #[test]
    fn silence_reads_as_minus_infinity() {
        assert_eq!(value_text(Target::TrackGain(HOOK), 0.0), "0 %  \u{2212}inf dB");
        assert_eq!(value_text(Target::SendGain { from: HOOK, to: THICKNER }, 0.0), "0 %  \u{2212}inf dB");
    }

    #[test]
    fn pan_reads_as_left_or_right_percent() {
        assert_eq!(value_text(Target::TrackPan(HOOK), 0.0), "C");
        assert_eq!(value_text(Target::TrackPan(HOOK), -0.4), "L 40 %");
        assert_eq!(value_text(Target::TrackPan(HOOK), 1.0), "R 100 %");
        assert_eq!(value_text(Target::TrackPan(HOOK), 0.004), "C");
    }

    #[test]
    fn plugin_knobs_read_as_percent() {
        assert_eq!(value_text(Target::TrackFx { track: HOOK, slot: 0, knob: 2 }, 0.375), "38 %");
        assert_eq!(value_text(Target::MasterFx { slot: 1, knob: 0 }, 1.0), "100 %");
        assert_eq!(value_text(Target::ClipFx { clip: loupe_engine::ClipId(3), slot: 0, knob: 0 }, 0.0), "0 %");
    }

    #[test]
    fn positions_read_as_bar_and_beat() {
        assert_eq!(bar_beat(0, 48_000, 120.0), "1.1");
        assert_eq!(bar_beat(24_000, 48_000, 120.0), "1.2");
        assert_eq!(bar_beat(96_000 + 48_000, 48_000, 120.0), "2.3");
    }
}
