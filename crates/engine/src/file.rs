use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::instrument::{Instrument, Note, Sampler, Synth, Wave};
use crate::model::{Clip, ClipId, Command, Edge, Fade, Frames, Outcome, Project, TrackId};
use crate::source::Source;

const HEADER: &str = "loupe project 1";

#[derive(Clone, Debug, PartialEq)]
pub struct SavedProject {
    pub saved_by: Option<String>,
    pub skipped: Vec<String>,
    pub rate: u32,
    pub bpm: f64,
    pub master: f32,
    pub master_muted: bool,
    pub master_fx: Vec<SavedFx>,
    pub sources: Vec<PathBuf>,
    pub tracks: Vec<SavedTrack>,
    pub envelopes: Vec<SavedEnvelope>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SavedEnvelope {
    pub target: SavedTarget,
    pub armed: Option<crate::envelope::Mode>,
    pub lane_open: bool,
    pub lowest: f32,
    pub highest: f32,
    pub points: Vec<crate::envelope::Point>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SavedTrack {
    pub name: String,
    pub gain: f32,
    pub muted: bool,
    pub pan: f32,
    pub solo: bool,
    pub records_notes: bool,
    pub colour: Option<[u8; 3]>,
    pub height: Option<f32>,
    pub clips: Vec<SavedClip>,
    pub parent: Option<usize>,
    pub collapsed: bool,
    pub sends: Vec<(usize, f32, bool, bool)>,
    pub fx: Vec<SavedFx>,
    pub instrument: Instrument,
    pub sample: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SavedFx {
    pub path: PathBuf,
    pub index: usize,
    pub name: String,
    pub bypassed: bool,
    pub state: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SavedClip {
    pub fx: Vec<SavedFx>,
    pub source: usize,
    pub start: Frames,
    pub offset: Frames,
    pub len: Frames,
    pub gain: f32,
    pub muted: bool,
    pub fade_in: Fade,
    pub fade_out: Fade,
    pub notes: Option<(String, Vec<Note>)>,
    pub stretch: f64,
}

impl SavedProject {
    pub fn capture(project: &Project, height_of: impl Fn(TrackId) -> Option<f32>) -> Self {
        let index_of = |clip: &Clip| {
            project.sources.iter().position(|kept| Arc::ptr_eq(kept, &clip.source)).unwrap_or(0)
        };
        Self {
            saved_by: Some(env!("CARGO_PKG_VERSION").to_string()),
            skipped: Vec::new(),
            rate: project.rate,
            bpm: project.bpm,
            master: project.master,
            master_muted: project.master_muted,
            master_fx: project
                .master_fx
                .iter()
                .map(|fx| SavedFx {
                    path: fx.path.clone(),
                    index: fx.index,
                    name: fx.name.clone(),
                    bypassed: fx.bypassed,
                    state: fx.state.clone(),
                })
                .collect(),
            sources: project.sources.iter().map(|source| source.path.clone()).collect(),
            envelopes: {
                let place = |want: TrackId| project.tracks.iter().position(|t| t.id == want).unwrap_or(usize::MAX);
                let spot = |want: ClipId| {
                    for (at, found) in project.tracks.iter().enumerate() {
                        if let Some(which) = found.clips.iter().position(|c| c.id == want) {
                            return (at, which);
                        }
                    }
                    (usize::MAX, usize::MAX)
                };
                project
                    .envelopes
                    .iter()
                    .filter_map(|shape| {
                        Some(SavedEnvelope {
                            target: target_from(&target_text(shape.target, &place, &spot))?,
                            armed: shape.armed,
                            lane_open: shape.lane_open,
                            lowest: shape.lowest,
                            highest: shape.highest,
                            points: shape.points.clone(),
                        })
                    })
                    .collect()
            },
            tracks: project
                .tracks
                .iter()
                .map(|track| SavedTrack {
                    name: track.name.clone(),
                    gain: track.gain,
                    muted: track.muted,
                    pan: track.pan,
                    solo: track.solo,
                    records_notes: track.records_notes,
                    colour: track.colour,
                    height: height_of(track.id),
                    parent: track.parent.and_then(|id| project.tracks.iter().position(|t| t.id == id)),
                    collapsed: track.collapsed,
                    instrument: track.instrument,
                    sample: track.sample.as_ref().and_then(|sample| project.sources.iter().position(|kept| Arc::ptr_eq(kept, sample))),
                    sends: track
                        .sends
                        .iter()
                        .filter_map(|send| {
                            project.tracks.iter().position(|t| t.id == send.to).map(|to| (to, send.gain, send.pre_fader, send.sidechain))
                        })
                        .collect(),
                    fx: track
                        .fx
                        .iter()
                        .map(|fx| SavedFx {
                            path: fx.path.clone(),
                            index: fx.index,
                            name: fx.name.clone(),
                            bypassed: fx.bypassed,
                            state: fx.state.clone(),
                        })
                        .collect(),
                    clips: track
                        .clips
                        .iter()
                        .map(|clip| SavedClip {
                            source: index_of(clip),
                            start: clip.start,
                            offset: clip.offset,
                            len: clip.len,
                            gain: clip.gain,
                            muted: clip.muted,
                            fade_in: clip.fade_in,
                            fade_out: clip.fade_out,
                            notes: clip.notes.as_ref().map(|notes| (clip.source.name.clone(), notes.to_vec())),
                            stretch: clip.stretch,
                            fx: clip
                                .fx
                                .iter()
                                .map(|fx| SavedFx {
                                    path: fx.path.clone(),
                                    index: fx.index,
                                    name: fx.name.clone(),
                                    bypassed: fx.bypassed,
                                    state: fx.state.clone(),
                                })
                                .collect(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    pub fn to_text(&self) -> String {
        let mut out = format!("{HEADER}\nsaved_by {}\nrate {}\nbpm {}\nmaster {}\nmaster_muted {}\n", env!("CARGO_PKG_VERSION"), self.rate, self.bpm, self.master, self.master_muted as u8);
        for path in &self.sources {
            out.push_str(&format!("source {}\n", path.display()));
        }
        // Before the tracks, so reading one does not land it on a track by mistake.
        for fx in &self.master_fx {
            let state = if fx.state.is_empty() { "-".to_string() } else { hex_of(&fx.state) };
            out.push_str(&format!("masterfxpath {}\n", fx.path.display()));
            out.push_str(&format!("masterfx index={} bypass={} state={state} name={}\n", fx.index, fx.bypassed as u8, fx.name));
        }
        for track in &self.tracks {
            let colour = track.colour.map_or("-".to_string(), |[r, g, b]| format!("#{r:02x}{g:02x}{b:02x}"));
            let height = track.height.map_or("-".to_string(), |h| h.to_string());
            let parent = track.parent.map_or("-".to_string(), |p| p.to_string());
            let sample = track.sample.map_or("-".to_string(), |s| s.to_string());
            out.push_str(&format!(
                "track gain={} muted={} pan={} solo={} keys={} colour={colour} height={height} parent={parent} collapsed={} instrument={} sample={sample} name={}\n",
                track.gain, track.muted as u8, track.pan, track.solo as u8, track.records_notes as u8, track.collapsed as u8, instrument_text(&track.instrument), track.name
            ));
            for (to, gain, pre, side) in &track.sends {
                out.push_str(&format!("send to={to} gain={gain} pre={} side={}\n", *pre as u8, *side as u8));
            }
            for fx in &track.fx {
                out.push_str(&format!("fxpath {}\n", fx.path.display()));
                let state = if fx.state.is_empty() { "-".to_string() } else { hex_of(&fx.state) };
                out.push_str(&format!(
                    "fx index={} bypass={} state={state} name={}\n",
                    fx.index, fx.bypassed as u8, fx.name
                ));
            }
            for clip in &track.clips {
                let opening = match &clip.notes {
                    Some(_) => "notes".to_string(),
                    None => format!("clip source={}", clip.source),
                };
                out.push_str(&format!(
                    "{opening} start={} offset={} len={} gain={} muted={} fade_in={}:{} fade_out={}:{}{}{}\n",
                    clip.start,
                    clip.offset,
                    clip.len,
                    clip.gain,
                    clip.muted as u8,
                    clip.fade_in.len,
                    clip.fade_in.curve,
                    clip.fade_out.len,
                    clip.fade_out.curve,
                    if clip.stretch == 1.0 { String::new() } else { format!(" stretch={}", clip.stretch) },
                    clip.notes.as_ref().map_or(String::new(), |(name, _)| format!(" name={name}"))
                ));
                for note in clip.notes.iter().flat_map(|(_, notes)| notes) {
                    out.push_str(&format!("note key={} start={} len={} velocity={}\n", note.key, note.start, note.len, note.velocity));
                }
                for fx in &clip.fx {
                    out.push_str(&format!("clipfxpath {}\n", fx.path.display()));
                    let state = if fx.state.is_empty() { "-".to_string() } else { hex_of(&fx.state) };
                    out.push_str(&format!("clipfx index={} bypass={} state={state} name={}\n", fx.index, fx.bypassed as u8, fx.name));
                }
            }
        }
        for shape in &self.envelopes {
            let armed = match shape.armed {
                Some(crate::envelope::Mode::Touch) => "touch",
                Some(crate::envelope::Mode::Latch) => "latch",
                None => "-",
            };
            out.push_str(&format!(
                "envelope on={} armed={armed} lane={} low={} high={}\n",
                where_text(&shape.target),
                shape.lane_open as u8,
                shape.lowest,
                shape.highest
            ));
            for point in &shape.points {
                out.push_str(&format!("point at={} value={} shape={}\n", point.at, point.value, shape_text(point.shape)));
            }
        }
        out
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let mut lines = text.lines().enumerate().map(|(i, line)| (i + 1, line.trim_end()));
        match lines.next() {
            Some((_, HEADER)) => {}
            _ => return Err("this is not a Loupe project file".into()),
        }
        let mut saved =
            Self { saved_by: None, skipped: Vec::new(), rate: 0, bpm: 120.0, master: 1.0, master_muted: false, master_fx: Vec::new(), sources: Vec::new(), tracks: Vec::new(), envelopes: Vec::new() };
        let mut held: Option<PathBuf> = None;
        for (number, line) in lines.filter(|(_, line)| !line.is_empty()) {
            let (kind, rest) = line.split_once(' ').unwrap_or((line, ""));
            let bad = |what: &str| format!("line {number}: {what}");
            match kind {
                "rate" => saved.rate = rest.parse().map_err(|_| bad("the sample rate is not a number"))?,
                "bpm" => saved.bpm = rest.parse().map_err(|_| bad("the tempo is not a number"))?,
                "master" => saved.master = rest.parse().map_err(|_| bad("the master level is not a number"))?,
                "master_muted" => saved.master_muted = rest.trim() != "0",
                "saved_by" => saved.saved_by = Some(rest.trim().to_string()),
                "source" => saved.sources.push(PathBuf::from(rest)),
                "track" => {
                    let (fields, name) = rest.split_once("name=").ok_or_else(|| bad("the track has no name"))?;
                    let fields = fields_of(fields);
                    saved.tracks.push(SavedTrack {
                        name: name.to_string(),
                        gain: number_in(&fields, "gain").ok_or_else(|| bad("the track has no gain"))?,
                        muted: fields.get("muted") == Some(&"1"),
                        pan: number_in(&fields, "pan").filter(|pan| (-1.0..=1.0).contains(pan)).unwrap_or(0.0),
                        solo: fields.get("solo") == Some(&"1"),
                        records_notes: fields.get("keys") == Some(&"1"),
                        colour: fields.get("colour").and_then(|value| colour_from(value)),
                        height: number_in(&fields, "height"),
                        clips: Vec::new(),
                        parent: fields.get("parent").and_then(|v| v.parse::<usize>().ok()),
                        collapsed: fields.get("collapsed") == Some(&"1"),
                        sends: Vec::new(),
                        fx: Vec::new(),
                        instrument: fields.get("instrument").and_then(|text| instrument_from(text)).unwrap_or_default(),
                        sample: fields.get("sample").and_then(|text| text.parse::<usize>().ok()),
                    });
                }
                "send" => {
                    let fields = fields_of(rest);
                    let track = saved.tracks.last_mut().ok_or_else(|| bad("a send before any track"))?;
                    let to = fields.get("to").and_then(|v| v.parse::<usize>().ok()).ok_or_else(|| bad("the send has no target"))?;
                    let gain = number_in(&fields, "gain").unwrap_or(1.0);
                    track.sends.push((to, gain, fields.get("pre") == Some(&"1"), fields.get("side") == Some(&"1")));
                }
                "fxpath" | "clipfxpath" => held = Some(PathBuf::from(rest)),
                "masterfxpath" => held = Some(PathBuf::from(rest)),
                "clipfx" | "fx" | "masterfx" => {
                    let (fields, name) = rest.split_once("name=").ok_or_else(|| bad("the plugin has no name"))?;
                    let fields = fields_of(fields);
                    let path = held.take().ok_or_else(|| bad("the plugin has no file"))?;
                    let state = match fields.get("state") {
                        Some(&"-") | None => Vec::new(),
                        Some(text) => bytes_of(text).ok_or_else(|| bad("the plugin settings are not readable"))?,
                    };
                    let fx = SavedFx {
                        path,
                        index: fields.get("index").and_then(|v| v.parse().ok()).unwrap_or(0),
                        name: name.to_string(),
                        bypassed: fields.get("bypass") == Some(&"1"),
                        state,
                    };
                    if kind == "masterfx" {
                        saved.master_fx.push(fx);
                        continue;
                    }
                    let track = saved.tracks.last_mut().ok_or_else(|| bad("a plugin before any track"))?;
                    if kind == "clipfx" {
                        track.clips.last_mut().ok_or_else(|| bad("a clip plugin before any clip"))?.fx.push(fx);
                    } else {
                        track.fx.push(fx);
                    }
                }
                "envelope" => {
                    let fields = fields_of(rest);
                    let on = fields.get("on").copied().ok_or_else(|| bad("the envelope has no target"))?;
                    let target = target_from(on).ok_or_else(|| bad("the envelope target is not readable"))?;
                    saved.envelopes.push(SavedEnvelope {
                        target,
                        armed: match fields.get("armed").copied() {
                            Some("touch") => Some(crate::envelope::Mode::Touch),
                            Some("latch") => Some(crate::envelope::Mode::Latch),
                            _ => None,
                        },
                        lane_open: fields.get("lane") != Some(&"0"),
                        lowest: number_in(&fields, "low").unwrap_or(0.0),
                        highest: number_in(&fields, "high").unwrap_or(1.0),
                        points: Vec::new(),
                    });
                }
                "point" => {
                    let fields = fields_of(rest);
                    let at = fields.get("at").and_then(|got| got.parse().ok()).ok_or_else(|| bad("the point has no time"))?;
                    let value = number_in(&fields, "value").ok_or_else(|| bad("the point has no value"))?;
                    let shape = shape_from(fields.get("shape").copied().unwrap_or("line"));
                    let holder = saved.envelopes.last_mut().ok_or_else(|| bad("a point before any envelope"))?;
                    holder.points.push(crate::envelope::Point { at, value, shape });
                }
                "note" => {
                    let fields = fields_of(rest);
                    let whole = |key: &str| fields.get(key).and_then(|v| v.parse::<u64>().ok()).ok_or_else(|| bad(&format!("the note has no {key}")));
                    let note = Note {
                        key: whole("key")?.min(127) as u8,
                        start: whole("start")?,
                        len: whole("len")?,
                        velocity: number_in(&fields, "velocity").unwrap_or(0.8),
                    };
                    let clip = saved.tracks.last_mut().and_then(|track| track.clips.last_mut()).ok_or_else(|| bad("a note before any clip"))?;
                    clip.notes.as_mut().ok_or_else(|| bad("a note in an audio clip"))?.1.push(note);
                }
                "clip" | "notes" => {
                    let (rest, name) = match kind {
                        "notes" => rest.split_once(" name=").map_or((rest, "Notes"), |(fields, name)| (fields, name)),
                        _ => (rest, ""),
                    };
                    let fields = fields_of(rest);
                    let need = |key: &str| fields.get(key).copied().ok_or_else(|| bad(&format!("the clip has no {key}")));
                    let whole = |key: &str| need(key)?.parse::<u64>().map_err(|_| bad(&format!("{key} is not a number")));
                    let clip = SavedClip {
                        source: if kind == "notes" { 0 } else { whole("source")? as usize },
                        start: whole("start")?,
                        offset: whole("offset")?,
                        len: whole("len")?,
                        gain: number_in(&fields, "gain").ok_or_else(|| bad("the clip has no gain"))?,
                        muted: fields.get("muted") == Some(&"1"),
                        fade_in: fade_from(need("fade_in")?).ok_or_else(|| bad("the fade in is not readable"))?,
                        fade_out: fade_from(need("fade_out")?).ok_or_else(|| bad("the fade out is not readable"))?,
                        fx: Vec::new(),
                        notes: (kind == "notes").then(|| (name.to_string(), Vec::new())),
                        stretch: match fields.get("stretch") {
                            Some(text) => text.parse::<f64>().ok().filter(|s| s.is_finite() && *s > 0.0).ok_or_else(|| bad("the stretch is not readable"))?,
                            None => 1.0,
                        },
                    };
                    if clip.notes.is_none() && clip.source >= saved.sources.len() {
                        return Err(bad("the clip points at audio the file does not list"));
                    }
                    saved.tracks.last_mut().ok_or_else(|| bad("a clip comes before any track"))?.clips.push(clip);
                }
                other => {
                    if !saved.skipped.iter().any(|known| known == other) {
                        saved.skipped.push(other.to_string());
                    }
                }
            }
        }
        if saved.rate == 0 {
            return Err("the file does not say its sample rate".into());
        }
        Ok(saved)
    }

    pub fn build(&self, sources: &[Arc<Source>], rate: u32) -> (Project, Vec<(TrackId, f32)>) {
        let rescale = |frames: Frames| (frames as u128 * rate as u128 / self.rate.max(1) as u128) as Frames;
        let mut project = Project::new(rate);
        let mut heights = Vec::new();
        let _ = project.apply(Command::SetBpm(self.bpm));
        let _ = project.apply(Command::SetMasterGain(self.master));
        if self.master_muted {
            let _ = project.apply(Command::ToggleMasterMute);
        }
        for source in sources {
            let _ = project.apply(Command::AddSource(source.clone()));
        }
        for fx in &self.master_fx {
            let _ = project.apply(Command::AddMasterFx(crate::model::Fx {
                path: fx.path.clone(),
                index: fx.index,
                name: fx.name.clone(),
                bypassed: fx.bypassed,
                state: fx.state.clone(),
            }));
        }
        for saved in &self.tracks {
            let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: saved.name.clone() }) else {
                continue;
            };
            let _ = project.apply(Command::SetTrackGain { track, gain: saved.gain });
            let _ = project.apply(Command::SetTrackMuted { track, muted: saved.muted });
            let _ = project.apply(Command::SetTrackPan { track, pan: saved.pan });
            let _ = project.apply(Command::SetTrackSolo { track, solo: saved.solo });
            let _ = project.apply(Command::SetRecordsNotes { track, on: saved.records_notes });
            let _ = project.apply(Command::SetTrackColour { track, colour: saved.colour });
            let _ = project.apply(Command::SetInstrument { track, instrument: saved.instrument });
            if let Some(sample) = saved.sample.and_then(|index| sources.get(index)) {
                let _ = project.apply(Command::SetSample { track, sample: Some(sample.clone()) });
            }
            if let Some(height) = saved.height {
                heights.push((track, height));
            }
            for clip in &saved.clips {
                let placed = match (&clip.notes, sources.get(clip.source)) {
                    (Some((name, notes)), _) => Command::AddNotesClip {
                        track,
                        name: name.clone(),
                        start: rescale(clip.start),
                        len: rescale(clip.len).max(1),
                        notes: notes.iter().map(|note| Note { start: rescale(note.start), len: rescale(note.len).max(1), ..*note }).collect(),
                    },
                    (None, Some(source)) => Command::AddClip { track, source: source.clone(), start: rescale(clip.start) },
                    (None, None) => continue,
                };
                let Ok(Outcome::Clip(id)) = project.apply(placed) else {
                    continue;
                };
                if clip.stretch != 1.0 {
                    let _ = project.apply(Command::SetStretch { clip: id, stretch: clip.stretch });
                }
                let trimmed = Command::TrimClip { clip: id, offset: rescale(clip.offset), len: rescale(clip.len) };
                let _ = project.apply(trimmed);
                let _ = project.apply(Command::SetClipGain { clip: id, gain: clip.gain });
                let _ = project.apply(Command::SetClipMuted { clip: id, muted: clip.muted });
                for (edge, fade) in [(Edge::In, clip.fade_in), (Edge::Out, clip.fade_out)] {
                    let fade = Fade { len: rescale(fade.len), curve: fade.curve };
                    let _ = project.apply(Command::SetClipFade { clip: id, edge, fade });
                }
                for fx in &clip.fx {
                    let added = crate::model::Fx {
                        path: fx.path.clone(),
                        index: fx.index,
                        name: fx.name.clone(),
                        bypassed: fx.bypassed,
                        state: fx.state.clone(),
                    };
                    let _ = project.apply(Command::AddClipFx { clip: id, fx: added });
                }
            }
        }
        let ids: Vec<_> = project.tracks.iter().map(|t| t.id).collect();
        let mut clip_ids: HashMap<(usize, usize), crate::model::ClipId> = HashMap::new();
        for (at, found) in project.tracks.iter().enumerate() {
            for (which, clip) in found.clips.iter().enumerate() {
                clip_ids.insert((at, which), clip.id);
            }
        }
        for (saved, track) in self.tracks.iter().zip(&ids) {
            if let Some(parent) = saved.parent.and_then(|p| ids.get(p)) {
                let _ = project.apply(Command::SetTrackParent { track: *track, parent: Some(*parent) });
            }
            if saved.collapsed {
                let _ = project.apply(Command::ToggleCollapsed(*track));
            }
            for (to, gain, pre, side) in &saved.sends {
                let Some(to) = ids.get(*to) else { continue };
                if project.apply(Command::AddSend { from: *track, to: *to }).is_ok() {
                    let _ = project.apply(Command::SetSendGain { from: *track, to: *to, gain: *gain });
                    let _ = project.apply(Command::SetSendPreFader { from: *track, to: *to, pre_fader: *pre });
                    let _ = project.apply(Command::SetSendSidechain { from: *track, to: *to, sidechain: *side });
                }
            }
            for fx in &saved.fx {
                let added = crate::model::Fx {
                    path: fx.path.clone(),
                    index: fx.index,
                    name: fx.name.clone(),
                    bypassed: fx.bypassed,
                    state: fx.state.clone(),
                };
                let _ = project.apply(Command::AddFx { track: *track, fx: added });
            }
        }
        for shape in &self.envelopes {
            let target = match shape.target {
                SavedTarget::Master => Some(crate::envelope::Target::MasterGain),
                SavedTarget::Track(track) => ids.get(track).map(|id| crate::envelope::Target::TrackGain(*id)),
                SavedTarget::Pan(track) => ids.get(track).map(|id| crate::envelope::Target::TrackPan(*id)),
                SavedTarget::Send(from, to) => match (ids.get(from), ids.get(to)) {
                    (Some(from), Some(to)) => Some(crate::envelope::Target::SendGain { from: *from, to: *to }),
                    _ => None,
                },
                SavedTarget::TrackFx(track, slot, knob) => {
                    ids.get(track).map(|id| crate::envelope::Target::TrackFx { track: *id, slot, knob })
                }
                SavedTarget::MasterFx(slot, knob) => Some(crate::envelope::Target::MasterFx { slot, knob }),
                SavedTarget::Clip(track, at) => clip_ids.get(&(track, at)).map(|id| crate::envelope::Target::ClipGain(*id)),
                SavedTarget::ClipFx(track, at, slot, knob) => {
                    clip_ids.get(&(track, at)).map(|id| crate::envelope::Target::ClipFx { clip: *id, slot, knob })
                }
            };
            let Some(target) = target else { continue };
            if project.apply(Command::AddEnvelope { target }).is_err() {
                continue;
            }
            let _ = project.apply(Command::ClearPoints { target, from: 0, to: Frames::MAX });
            for point in &shape.points {
                let point = crate::envelope::Point { at: rescale(point.at), ..*point };
                let _ = project.apply(Command::PutPoint { target, point });
            }
            let _ = project.apply(Command::ArmEnvelope { target, mode: shape.armed });
            let _ = project.apply(Command::ShowEnvelopeLane { target, open: shape.lane_open });
        }
        (project, heights)
    }
}

fn instrument_text(instrument: &Instrument) -> String {
    match instrument {
        Instrument::Drums => "drums".to_string(),
        Instrument::Sampler(sampler) => format!(
            "sampler:{}:{}:{}:{}:{}:{}",
            sampler.root, sampler.tune, sampler.attack, sampler.release, sampler.one_shot as u8, sampler.keytrack as u8
        ),
        Instrument::Synth(synth) => format!(
            "synth:{}:{}:{}:{}:{}:{}",
            synth.wave.name(),
            synth.attack,
            synth.decay,
            synth.sustain,
            synth.release,
            synth.detune
        ),
    }
}

fn instrument_from(text: &str) -> Option<Instrument> {
    if text == "drums" {
        return Some(Instrument::Drums);
    }
    if let Some(rest) = text.strip_prefix("sampler:") {
        let parts: Vec<&str> = rest.split(':').collect();
        let [root, tune, attack, release, one_shot, keytrack] = parts.as_slice() else {
            return None;
        };
        let positive = |text: &str| text.parse::<f32>().ok().filter(|n| n.is_finite() && *n >= 0.0);
        return Some(Instrument::Sampler(Sampler {
            root: root.parse::<u8>().ok().filter(|key| *key <= crate::instrument::HIGHEST_KEY)?,
            tune: tune.parse::<f32>().ok().filter(|n| n.is_finite())?.clamp(-48.0, 48.0),
            attack: positive(attack)?,
            release: positive(release)?,
            one_shot: *one_shot == "1",
            keytrack: *keytrack == "1",
        }));
    }
    let mut parts = text.strip_prefix("synth:")?.split(':');
    let wave = Wave::named(parts.next()?)?;
    let mut number = || parts.next()?.parse::<f32>().ok().filter(|n| n.is_finite() && *n >= 0.0);
    Some(Instrument::Synth(Synth {
        wave,
        attack: number()?,
        decay: number()?,
        sustain: number()?.min(1.0),
        release: number()?,
        detune: number()?,
    }))
}

fn hex_of(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn bytes_of(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    let raw = text.as_bytes();
    let mut out = Vec::with_capacity(text.len() / 2);
    for pair in raw.chunks(2) {
        let two = std::str::from_utf8(pair).ok()?;
        out.push(u8::from_str_radix(two, 16).ok()?);
    }
    Some(out)
}

fn fields_of(text: &str) -> HashMap<&str, &str> {
    text.split_whitespace().filter_map(|field| field.split_once('=')).collect()
}

fn number_in(fields: &HashMap<&str, &str>, key: &str) -> Option<f32> {
    fields.get(key)?.parse().ok()
}

fn colour_from(value: &str) -> Option<[u8; 3]> {
    let digits = value.strip_prefix('#')?;
    if digits.len() != 6 {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

fn fade_from(value: &str) -> Option<Fade> {
    let (len, curve) = value.split_once(':')?;
    Some(Fade { len: len.parse().ok()?, curve: curve.parse().ok()? })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::render;

    fn song() -> Project {
        let mut p = Project::new(48_000);
        let beat = Arc::new(Source::from_frames("beat", (0..2000).map(|i| [i as f32, -(i as f32)]).collect()));
        let spare = Arc::new(Source::from_frames("spare take", vec![[0.25, 0.25]; 500]));
        let Ok(Outcome::Track(track)) = p.apply(Command::AddTrack { name: "Lead vox, take 2".into() }) else {
            panic!("no track")
        };
        let Ok(Outcome::Clip(clip)) = p.apply(Command::AddClip { track, source: beat, start: 100 }) else {
            panic!("no clip")
        };
        let Ok(Outcome::Clip(right)) = p.apply(Command::SplitClip { clip, at: 900 }) else {
            panic!("no split")
        };
        p.apply(Command::SetClipGain { clip: right, gain: 0.5 }).unwrap();
        p.apply(Command::SetClipFade { clip: right, edge: Edge::Out, fade: Fade { len: 300, curve: 0.5 } }).unwrap();
        p.apply(Command::SetTrackGain { track, gain: 0.8 }).unwrap();
        p.apply(Command::SetTrackColour { track, colour: Some([12, 200, 255]) }).unwrap();
        p.apply(Command::AddSource(spare)).unwrap();
        p.apply(Command::SetBpm(93.5)).unwrap();
        p.apply(Command::SetMasterGain(0.5)).unwrap();
        p
    }

    fn sound(p: &Project) -> Vec<[f32; 2]> {
        let mut out = vec![[0.0; 2]; 2400];
        render(p, 0, &mut out);
        out
    }

    #[test]
    fn a_saved_song_opens_sounding_the_same() {
        let original = song();
        let saved = SavedProject::capture(&original, |_| Some(120.0));
        let reread = SavedProject::parse(&saved.to_text()).unwrap();
        assert_eq!(reread, saved);
        let (reopened, heights) = reread.build(&original.sources, 48_000);
        assert_eq!(sound(&reopened), sound(&original));
        assert_eq!(reopened.bpm, 93.5);
        assert_eq!(reopened.sources.len(), 2);
        assert_eq!(reopened.tracks[0].name, "Lead vox, take 2");
        assert_eq!(reopened.tracks[0].colour, Some([12, 200, 255]));
        assert_eq!(heights.len(), 1);
        assert_eq!(heights[0].1, 120.0);
    }

    #[test]
    fn a_song_saved_at_another_rate_keeps_its_timing() {
        let saved = SavedProject::capture(&song(), |_| None);
        let at_the_new_rate = [
            Arc::new(Source::from_frames("beat", vec![[0.5, 0.5]; 4000])),
            Arc::new(Source::from_frames("spare take", vec![[0.25, 0.25]; 1000])),
        ];
        let (reopened, _) = saved.build(&at_the_new_rate, 96_000);
        let clip = &reopened.tracks[0].clips[1];
        assert_eq!((clip.start, clip.offset, clip.len), (1800, 1600, 2400));
        assert_eq!(clip.fade_out.len, 600);
    }

    #[test]
    fn a_file_that_is_not_a_project_is_refused() {
        assert!(SavedProject::parse("hello").is_err());
        assert!(SavedProject::parse("loupe project 1\nbpm 120\n").is_err());
        assert!(SavedProject::parse("loupe project 1\nrate 48000\nclip source=0 start=0\n").is_err());
    }

    #[test]
    fn lines_from_a_newer_loupe_are_skipped_and_named() {
        let saved = SavedProject::parse("loupe project 1\nsaved_by 9.2.0\nrate 48000\nwidget 3\nwidget 4\nsparkle on\n").unwrap();
        assert_eq!(saved.saved_by.as_deref(), Some("9.2.0"));
        assert_eq!(saved.skipped, ["widget", "sparkle"]);
    }

    #[test]
    fn every_save_says_which_loupe_wrote_it() {
        let text = SavedProject::capture(&Project::new(48_000), |_| None).to_text();
        assert!(text.starts_with(&format!("loupe project 1\nsaved_by {}\n", env!("CARGO_PKG_VERSION"))));
        assert_eq!(SavedProject::parse(&text).unwrap().saved_by.as_deref(), Some(env!("CARGO_PKG_VERSION")));
    }
}

#[cfg(test)]
mod routing_round_trip {
    use super::*;
    use crate::model::{Command, Outcome};
    use crate::source::Source;

    #[test]
    fn master_plugins_survive_a_save_and_open_and_stay_off_the_tracks() {
        let mut p = Project::new(48_000);
        let Ok(Outcome::Track(track)) = p.apply(Command::AddTrack { name: "Vocal".into() }) else { panic!() };
        let on_track = crate::model::Fx {
            path: PathBuf::from("one.vst3"),
            index: 0,
            name: "On the track".into(),
            bypassed: false,
            state: Vec::new(),
        };
        p.apply(Command::AddFx { track, fx: on_track }).unwrap();
        let over_all = crate::model::Fx {
            path: PathBuf::from("glue.vst3"),
            index: 2,
            name: "Over the mix".into(),
            bypassed: true,
            state: vec![9, 8, 7],
        };
        p.apply(Command::AddMasterFx(over_all.clone())).unwrap();
        let text = SavedProject::capture(&p, |_| None).to_text();
        let (back, _) = SavedProject::parse(&text).unwrap().build(&[], 48_000);
        assert_eq!(back.master_fx.len(), 1);
        assert_eq!(back.master_fx[0], over_all);
        assert_eq!(back.tracks[0].fx.len(), 1, "the master plugin landed on a track");
        assert_eq!(back.tracks[0].fx[0].name, "On the track");
        assert!(p.apply(Command::RemoveMasterFx(7)).is_err());
        p.apply(Command::RemoveMasterFx(0)).unwrap();
        assert!(p.master_fx.is_empty());
    }

    #[test]
    fn a_master_plugin_knob_can_be_automated_and_comes_back() {
        let mut p = Project::new(48_000);
        let fx = crate::model::Fx {
            path: PathBuf::from("glue.vst3"),
            index: 0,
            name: "Glue".into(),
            bypassed: false,
            state: Vec::new(),
        };
        p.apply(Command::AddMasterFx(fx)).unwrap();
        let target = crate::envelope::Target::MasterFx { slot: 0, knob: 3 };
        assert_eq!(p.range_of(target), Some((0.0, 1.0, 0.5)));
        p.apply(Command::AddEnvelope { target }).unwrap();
        let point = |at, value| crate::envelope::Point { at, value, shape: crate::envelope::Shape::Linear };
        p.apply(Command::PutPoint { target, point: point(0, 0.2) }).unwrap();
        p.apply(Command::PutPoint { target, point: point(48_000, 0.9) }).unwrap();
        let text = SavedProject::capture(&p, |_| None).to_text();
        assert!(text.contains("masterfx:0:3"));
        let (back, _) = SavedProject::parse(&text).unwrap().build(&[], 48_000);
        let shape = back.envelope(target).expect("the master envelope came back");
        assert_eq!(shape.value_at(0), Some(0.2));
        assert_eq!(shape.value_at(48_000), Some(0.9));
        // A knob on a plugin that is not there has no range, so no envelope can point at it.
        assert_eq!(p.range_of(crate::envelope::Target::MasterFx { slot: 4, knob: 0 }), None);
    }

    #[test]
    fn a_stretched_clip_survives_a_save_and_open() {
        let mut p = Project::new(48_000);
        let Ok(Outcome::Track(track)) = p.apply(Command::AddTrack { name: "Loop".into() }) else { panic!() };
        let source = Arc::new(Source::from_frames("loop", vec![[0.1, 0.1]; 1_000]));
        let Ok(Outcome::Clip(clip)) = p.apply(Command::AddClip { track, source: source.clone(), start: 0 }) else { panic!() };
        p.apply(Command::SetStretch { clip, stretch: 1.5 }).unwrap();
        p.apply(Command::TrimClip { clip, offset: 300, len: 900 }).unwrap();
        let text = SavedProject::capture(&p, |_| None).to_text();
        assert!(text.contains(" stretch=1.5"));
        let (back, _) = SavedProject::parse(&text).unwrap().build(&[source], 48_000);
        let clip = &back.tracks[0].clips[0];
        assert_eq!((clip.stretch, clip.offset, clip.len), (1.5, 300, 900));
        assert!(clip.waiting_for_stretch());
        assert!(SavedProject::parse(&text.replace("stretch=1.5", "stretch=fast")).is_err());
    }

    #[test]
    fn a_sampler_and_its_sound_survive_a_save_and_open() {
        let mut p = Project::new(48_000);
        let Ok(Outcome::Track(track)) = p.apply(Command::AddTrack { name: "808".into() }) else { panic!() };
        let sample = Arc::new(Source::from_frames("808", vec![[0.2, 0.2]; 1_000]));
        let sampler = Sampler { root: 48, tune: -1.5, attack: 0.01, release: 0.2, one_shot: true, keytrack: false };
        p.apply(Command::SetInstrument { track, instrument: Instrument::Sampler(sampler) }).unwrap();
        p.apply(Command::SetSample { track, sample: Some(sample.clone()) }).unwrap();
        let saved = SavedProject::capture(&p, |_| None);
        assert_eq!(saved.tracks[0].sample, Some(0));
        let text = saved.to_text();
        let (back, _) = SavedProject::parse(&text).unwrap().build(&[sample.clone()], 48_000);
        assert_eq!(back.tracks[0].instrument, Instrument::Sampler(sampler));
        assert!(Arc::ptr_eq(back.tracks[0].sample.as_ref().unwrap(), &sample));
        assert!(SavedProject::parse(&text.replace("sampler:48", "sampler:400")).unwrap().tracks[0].instrument != Instrument::Sampler(sampler));
    }

    #[test]
    fn pan_and_solo_survive_a_save_and_open() {
        let mut p = Project::new(48_000);
        let Ok(Outcome::Track(a)) = p.apply(Command::AddTrack { name: "A".into() }) else { panic!() };
        p.apply(Command::AddTrack { name: "B".into() }).unwrap();
        p.apply(Command::SetTrackPan { track: a, pan: -0.4 }).unwrap();
        p.apply(Command::SetTrackSolo { track: a, solo: true }).unwrap();
        let text = SavedProject::capture(&p, |_| None).to_text();
        let (back, _) = SavedProject::parse(&text).unwrap().build(&[], 48_000);
        assert_eq!((back.tracks[0].pan, back.tracks[0].solo), (-0.4, true));
        assert_eq!((back.tracks[1].pan, back.tracks[1].solo), (0.0, false));
        let old = text.replace(" pan=-0.4 solo=1", "");
        let (back, _) = SavedProject::parse(&old).unwrap().build(&[], 48_000);
        assert_eq!((back.tracks[0].pan, back.tracks[0].solo), (0.0, false));
    }

    #[test]
    fn folders_and_sends_survive_a_save_and_open() {
        let mut p = Project::new(48_000);
        let Ok(Outcome::Track(folder)) = p.apply(Command::AddTrack { name: "Vox".into() }) else {
            panic!("no track")
        };
        let Ok(Outcome::Track(child)) = p.apply(Command::AddTrack { name: "Take".into() }) else {
            panic!("no track")
        };
        let Ok(Outcome::Track(verb)) = p.apply(Command::AddTrack { name: "Verb".into() }) else {
            panic!("no track")
        };
        p.apply(Command::SetTrackParent { track: child, parent: Some(folder) }).unwrap();
        p.apply(Command::ToggleCollapsed(folder)).unwrap();
        p.apply(Command::AddSend { from: child, to: verb }).unwrap();
        p.apply(Command::SetSendGain { from: child, to: verb, gain: 0.25 }).unwrap();
        p.apply(Command::SetSendPreFader { from: child, to: verb, pre_fader: true }).unwrap();
        let saved = SavedProject::capture(&p, |_| None);
        let text = saved.to_text();
        let read = SavedProject::parse(&text).unwrap();
        let sources: Vec<Arc<Source>> = Vec::new();
        let (back, _) = read.build(&sources, 48_000);
        assert_eq!(back.tracks[1].parent, Some(back.tracks[0].id));
        assert!(back.tracks[0].collapsed);
        assert_eq!(back.tracks[1].sends.len(), 1);
        assert_eq!(back.tracks[1].sends[0].gain, 0.25);
        assert!(back.tracks[1].sends[0].pre_fader);
    }

    #[test]
    fn plugin_chains_survive_a_save_and_open() {
        let mut p = Project::new(48_000);
        let Ok(Outcome::Track(track)) = p.apply(Command::AddTrack { name: "Vox".into() }) else {
            panic!("no track")
        };
        let eq = crate::model::Fx {
            path: PathBuf::from("/plugins/Pro Q 4.vst3"),
            index: 0,
            name: "FabFilter Pro-Q 4".into(),
            bypassed: false,
            state: vec![0, 1, 2, 250, 255],
        };
        let comp = crate::model::Fx {
            path: PathBuf::from("/plugins/Pro C 2.vst3"),
            index: 1,
            name: "FabFilter Pro-C 2".into(),
            bypassed: true,
            state: Vec::new(),
        };
        p.apply(Command::AddFx { track, fx: eq }).unwrap();
        p.apply(Command::AddFx { track, fx: comp }).unwrap();
        let text = SavedProject::capture(&p, |_| None).to_text();
        let read = SavedProject::parse(&text).unwrap();
        let sources: Vec<Arc<Source>> = Vec::new();
        let (back, _) = read.build(&sources, 48_000);
        let chain = &back.tracks[0].fx;
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].name, "FabFilter Pro-Q 4");
        assert_eq!(chain[0].path, PathBuf::from("/plugins/Pro Q 4.vst3"));
        assert_eq!(chain[0].state, vec![0, 1, 2, 250, 255]);
        assert!(!chain[0].bypassed);
        assert_eq!(chain[1].index, 1);
        assert!(chain[1].bypassed);
        assert!(chain[1].state.is_empty());
    }

    #[test]
    fn envelopes_survive_a_save_and_open() {
        use crate::envelope::{Mode, Point, Shape, Target};
        let mut p = Project::new(48_000);
        let Ok(Outcome::Track(one)) = p.apply(Command::AddTrack { name: "Vox".into() }) else {
            panic!("no track")
        };
        let target = Target::TrackGain(one);
        p.apply(Command::AddEnvelope { target }).unwrap();
        p.apply(Command::ClearPoints { target, from: 0, to: Frames::MAX }).unwrap();
        p.apply(Command::PutPoint { target, point: Point { at: 0, value: 0.25, shape: Shape::Hold } }).unwrap();
        p.apply(Command::PutPoint { target, point: Point { at: 4800, value: 1.5, shape: Shape::Curve(0.5) } }).unwrap();
        p.apply(Command::ArmEnvelope { target, mode: Some(Mode::Latch) }).unwrap();
        let text = SavedProject::capture(&p, |_| None).to_text();
        let read = SavedProject::parse(&text).unwrap();
        let sources: Vec<Arc<Source>> = Vec::new();
        let (back, _) = read.build(&sources, 48_000);
        assert_eq!(back.envelopes.len(), 1);
        let shape = &back.envelopes[0];
        assert_eq!(shape.armed, Some(Mode::Latch));
        assert_eq!(shape.points.len(), 2);
        assert_eq!(shape.points[0].value, 0.25);
        assert_eq!(shape.points[0].shape, Shape::Hold);
        assert_eq!(shape.points[1].at, 4800);
        assert_eq!(shape.points[1].shape, Shape::Curve(0.5));
        assert_eq!(shape.highest, 2.0);
    }
}

fn target_text(target: crate::envelope::Target, place: &dyn Fn(TrackId) -> usize, spot: &dyn Fn(ClipId) -> (usize, usize)) -> String {
    use crate::envelope::Target;
    match target {
        Target::MasterGain => "master".to_string(),
        Target::TrackGain(track) => format!("track:{}", place(track)),
        Target::TrackPan(track) => format!("pan:{}", place(track)),
        Target::SendGain { from, to } => format!("send:{}:{}", place(from), place(to)),
        Target::TrackFx { track, slot, knob } => format!("trackfx:{}:{slot}:{knob}", place(track)),
        Target::MasterFx { slot, knob } => format!("masterfx:{slot}:{knob}"),
        Target::ClipGain(clip) => {
            let (track, at) = spot(clip);
            format!("clip:{track}:{at}")
        }
        Target::ClipFx { clip, slot, knob } => {
            let (track, at) = spot(clip);
            format!("clipfx:{track}:{at}:{slot}:{knob}")
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SavedTarget {
    Master,
    Track(usize),
    Pan(usize),
    Send(usize, usize),
    TrackFx(usize, usize, usize),
    MasterFx(usize, usize),
    Clip(usize, usize),
    ClipFx(usize, usize, usize, usize),
}

fn target_from(text: &str) -> Option<SavedTarget> {
    let mut parts = text.split(':');
    let kind = parts.next()?;
    let mut number = || parts.next().and_then(|got| got.parse::<usize>().ok());
    match kind {
        "master" => Some(SavedTarget::Master),
        "track" => Some(SavedTarget::Track(number()?)),
        "pan" => Some(SavedTarget::Pan(number()?)),
        "send" => Some(SavedTarget::Send(number()?, number()?)),
        "trackfx" => Some(SavedTarget::TrackFx(number()?, number()?, number()?)),
        "masterfx" => Some(SavedTarget::MasterFx(number()?, number()?)),
        "clip" => Some(SavedTarget::Clip(number()?, number()?)),
        "clipfx" => Some(SavedTarget::ClipFx(number()?, number()?, number()?, number()?)),
        _ => None,
    }
}

fn shape_text(shape: crate::envelope::Shape) -> String {
    use crate::envelope::Shape;
    match shape {
        Shape::Linear => "line".to_string(),
        Shape::Hold => "hold".to_string(),
        Shape::Curve(bend) => format!("bend{bend}"),
    }
}

fn shape_from(text: &str) -> crate::envelope::Shape {
    use crate::envelope::Shape;
    match text {
        "hold" => Shape::Hold,
        other => match other.strip_prefix("bend").and_then(|rest| rest.parse().ok()) {
            Some(bend) => Shape::Curve(bend),
            None => Shape::Linear,
        },
    }
}

fn where_text(target: &SavedTarget) -> String {
    match target {
        SavedTarget::Master => "master".to_string(),
        SavedTarget::Track(track) => format!("track:{track}"),
        SavedTarget::Pan(track) => format!("pan:{track}"),
        SavedTarget::Send(from, to) => format!("send:{from}:{to}"),
        SavedTarget::TrackFx(track, slot, knob) => format!("trackfx:{track}:{slot}:{knob}"),
        SavedTarget::MasterFx(slot, knob) => format!("masterfx:{slot}:{knob}"),
        SavedTarget::Clip(track, at) => format!("clip:{track}:{at}"),
        SavedTarget::ClipFx(track, at, slot, knob) => format!("clipfx:{track}:{at}:{slot}:{knob}"),
    }
}
