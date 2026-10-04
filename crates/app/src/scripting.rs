use std::cell::RefCell;
use std::time::{Duration, Instant};

use std::path::PathBuf;

use loupe_engine::{ClipId, Command, CommandError, Frames, Fx, Instrument, Note, Outcome, Point, Project, Shape, Source, Target, TrackId};
use mlua::{Lua, LuaOptions, StdLib, Table, Value, Variadic};

const LONGEST_RUN: Duration = Duration::from_secs(5);
const CHECK_EVERY: u32 = 10_000;
const HIGHEST_KEY: i64 = 127;

pub const FUNCTIONS: [(&str, &str); 57] = [
    ("print(...)", "Show text in Loupe's status line"),
    ("bpm()", "The tempo in beats per minute"),
    ("set_bpm(bpm)", "Change the tempo"),
    ("rate()", "The sample rate"),
    ("length()", "The song length in seconds"),
    ("seconds_to_beats(seconds)", "Turn seconds into beats at the song tempo"),
    ("beats_to_seconds(beats)", "Turn beats into seconds at the song tempo"),
    ("playhead()", "Where the playhead is, in seconds"),
    ("set_playhead(seconds)", "Move the playhead"),
    ("is_playing()", "Whether the song is playing"),
    ("play()", "Start playing"),
    ("stop()", "Stop playing"),
    ("tracks()", "A list of every track id, top to bottom"),
    ("add_track(name)", "Add a track and return its id"),
    ("remove_track(track)", "Remove a track and its clips"),
    ("duplicate_track(track)", "Copy a track and return the new id"),
    ("track_name(track)", "A track's name"),
    ("set_track_name(track, name)", "Rename a track"),
    ("track_volume(track)", "A track's fader in dB"),
    ("set_track_volume(track, db)", "Set a track's fader in dB"),
    ("track_pan(track)", "A track's pan, -1 left to 1 right"),
    ("set_track_pan(track, pan)", "Set a track's pan"),
    ("track_muted(track)", "Whether a track is muted"),
    ("set_track_muted(track, muted)", "Mute or unmute a track"),
    ("track_solo(track)", "Whether a track is soloed"),
    ("set_track_solo(track, solo)", "Solo or unsolo a track"),
    ("track_color(track)", "A track's colour as #rrggbb, or nil"),
    ("set_track_color(track, color)", "Set a track's colour as #rrggbb, or nil for the theme's"),
    ("track_clips(track)", "A list of the clip ids on a track"),
    ("clip_track(clip)", "The track a clip is on"),
    ("clip_name(clip)", "A clip's name"),
    ("clip_start(clip)", "Where a clip starts, in seconds"),
    ("clip_length(clip)", "How long a clip is, in seconds"),
    ("move_clip(clip, track, seconds)", "Move a clip to a track and time"),
    ("clip_volume(clip)", "A clip's gain in dB"),
    ("set_clip_volume(clip, db)", "Set a clip's gain in dB"),
    ("clip_muted(clip)", "Whether a clip is muted"),
    ("set_clip_muted(clip, muted)", "Mute or unmute a clip"),
    ("split_clip(clip, seconds)", "Cut a clip at a time in the song and return the right part"),
    ("delete_clip(clip)", "Delete a clip"),
    ("is_note_clip(clip)", "Whether a clip holds notes"),
    ("add_note_clip(track, seconds, length)", "Add an empty note clip and return its id"),
    ("notes(clip)", "A note clip's notes: {key, start, length, velocity}, times in seconds from the clip start"),
    ("set_notes(clip, notes)", "Replace a note clip's notes with a list like the one notes() gives"),
    ("selected_clips()", "The selected clip ids"),
    ("select_clips(clips)", "Select these clips"),
    ("clear_selection()", "Select nothing"),
    ("track_plugins(track)", "The names of a track's plugins, slot 1 first"),
    ("add_plugin(track, name)", "Add a plugin by name, for example \"Loupe EQ\", and return its slot"),
    ("set_plugin_knob(track, slot, knob, value)", "Turn a knob on one of Loupe's own plugins; knob is a number from 1 or a name like \"threshold\""),
    ("add_send(from, to, db)", "Send a track to another, for example a reverb bus"),
    ("set_track_folder(track, folder)", "Put a track inside a folder track, or nil to take it out"),
    ("add_automation_point(track, what, seconds, value)", "Draw automation: what is \"volume\" in dB or \"pan\" from -1 to 1"),
    ("set_loop(start, finish)", "Loop between two times in seconds, or set_loop() to clear it"),
    ("import_audio(file, track, seconds)", "Place an audio file on a track and return the clip; paths can be relative to the song's folder"),
    ("set_instrument(track, name)", "Use \"synth\" or \"drums\" on a track"),
    ("export(stems)", "Export the song like File > Export once the script ends; pass true to also write stems"),
];

pub struct View {
    pub playhead: Frames,
    pub playing: bool,
    pub selected: Vec<ClipId>,
    pub plugins: Vec<(String, PathBuf, usize)>,
    pub folder: Option<PathBuf>,
}

#[derive(Debug, Default, PartialEq)]
pub struct Wishes {
    pub play: Option<bool>,
    pub seek: Option<Frames>,
    pub select: Option<Vec<ClipId>>,
    pub printed: Vec<String>,
    pub tweaks: Vec<(TrackId, usize, usize, f32)>,
    pub looped: Option<Option<(Frames, Frames)>>,
    pub export: Option<bool>,
}

#[derive(Debug, PartialEq)]
pub struct Ran {
    pub changed: bool,
    pub wishes: Wishes,
}

struct Host<'a> {
    project: &'a mut Project,
    view: &'a View,
    wishes: Wishes,
    changed: bool,
}

fn fail(words: impl Into<String>) -> mlua::Error {
    mlua::Error::runtime(words.into())
}

fn refused(error: CommandError) -> mlua::Error {
    fail(match error {
        CommandError::NoSuchTrack => "there is no such track",
        CommandError::NoSuchClip => "there is no such clip",
        CommandError::SplitOutsideClip => "that time is not inside the clip",
        CommandError::InvalidValue => "that value is out of range",
    })
}

impl Host<'_> {
    fn apply(&mut self, command: Command) -> mlua::Result<Outcome> {
        let outcome = self.project.apply(command).map_err(refused)?;
        self.changed = true;
        Ok(outcome)
    }

    fn frames(&self, seconds: f64) -> Frames {
        (seconds.max(0.0) * self.project.rate as f64).round() as Frames
    }

    fn seconds(&self, frames: Frames) -> f64 {
        frames as f64 / self.project.rate.max(1) as f64
    }

    fn track(&self, id: u64) -> mlua::Result<&loupe_engine::Track> {
        self.project.track(TrackId(id)).ok_or_else(|| fail(format!("there is no track {id}")))
    }

    fn clip(&self, id: u64) -> mlua::Result<&loupe_engine::Clip> {
        self.project.clip(ClipId(id)).ok_or_else(|| fail(format!("there is no clip {id}")))
    }
}

fn colour_text(colour: Option<[u8; 3]>) -> Option<String> {
    colour.map(|[r, g, b]| format!("#{r:02x}{g:02x}{b:02x}"))
}

fn colour_from(text: &str) -> Option<[u8; 3]> {
    let hex = text.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let part = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
    Some([part(0)?, part(2)?, part(4)?])
}

fn db_of(gain: f32) -> f64 {
    if gain <= 0.0 {
        f64::NEG_INFINITY
    } else {
        20.0 * (gain as f64).log10()
    }
}

fn gain_of(db: f64) -> f32 {
    10f64.powf(db / 20.0) as f32
}

pub fn shortcut_of(source: &str) -> Option<String> {
    source.lines().take(5).find_map(|line| {
        let rest = line.trim().strip_prefix("--")?.trim();
        let (key, value) = rest.split_once(':')?;
        key.trim().eq_ignore_ascii_case("shortcut").then(|| value.trim().to_string()).filter(|value| !value.is_empty())
    })
}

pub fn run(source: &str, name: &str, project: &mut Project, view: &View) -> Result<Ran, String> {
    let lua = Lua::new_with(StdLib::TABLE | StdLib::STRING | StdLib::UTF8 | StdLib::MATH, LuaOptions::new()).map_err(|why| why.to_string())?;
    let started = Instant::now();
    lua.set_hook(mlua::HookTriggers::new().every_nth_instruction(CHECK_EVERY), move |_, _| {
        if started.elapsed() > LONGEST_RUN {
            Err(fail("the script ran for more than 5 seconds and was stopped"))
        } else {
            Ok(mlua::VmState::Continue)
        }
    })
    .map_err(|why| why.to_string())?;
    let host = RefCell::new(Host { project, view, wishes: Wishes::default(), changed: false });
    let result = lua.scope(|scope| {
        let globals = lua.globals();
        for unsafe_name in ["dofile", "loadfile", "load", "collectgarbage"] {
            globals.set(unsafe_name, Value::Nil)?;
        }
        let api = lua.create_table()?;
        macro_rules! def {
            ($name:literal, $body:expr) => {
                api.set($name, scope.create_function($body)?)?;
            };
        }
        def!("print", |_, words: Variadic<Value>| {
            host.borrow_mut().wishes.printed.push(say(words));
            Ok(())
        });
        globals.set("print", scope.create_function(|_, words: Variadic<Value>| {
            host.borrow_mut().wishes.printed.push(say(words));
            Ok(())
        })?)?;
        def!("bpm", |_, ()| Ok(host.borrow().project.bpm));
        def!("set_bpm", |_, bpm: f64| host.borrow_mut().apply(Command::SetBpm(bpm)).map(|_| ()));
        def!("rate", |_, ()| Ok(host.borrow().project.rate));
        def!("length", |_, ()| {
            let h = host.borrow();
            Ok(h.seconds(h.project.length()))
        });
        def!("seconds_to_beats", |_, seconds: f64| Ok(seconds * host.borrow().project.bpm / 60.0));
        def!("beats_to_seconds", |_, beats: f64| Ok(beats * 60.0 / host.borrow().project.bpm));
        def!("playhead", |_, ()| {
            let h = host.borrow();
            Ok(h.seconds(h.wishes.seek.unwrap_or(h.view.playhead)))
        });
        def!("set_playhead", |_, seconds: f64| {
            let mut h = host.borrow_mut();
            let at = h.frames(seconds);
            h.wishes.seek = Some(at);
            Ok(())
        });
        def!("is_playing", |_, ()| {
            let h = host.borrow();
            Ok(h.wishes.play.unwrap_or(h.view.playing))
        });
        def!("play", |_, ()| {
            host.borrow_mut().wishes.play = Some(true);
            Ok(())
        });
        def!("stop", |_, ()| {
            host.borrow_mut().wishes.play = Some(false);
            Ok(())
        });
        def!("tracks", |lua, ()| lua.create_sequence_from(host.borrow().project.tracks.iter().map(|t| t.id.0)));
        def!("add_track", |_, name: Option<String>| {
            let mut h = host.borrow_mut();
            let name = name.unwrap_or_else(|| format!("Track {}", h.project.tracks.len() + 1));
            match h.apply(Command::AddTrack { name })? {
                Outcome::Track(id) => Ok(id.0),
                _ => Err(fail("the track was not made")),
            }
        });
        def!("remove_track", |_, track: u64| {
            let mut h = host.borrow_mut();
            h.track(track)?;
            h.apply(Command::RemoveTrack(TrackId(track))).map(|_| ())
        });
        def!("duplicate_track", |_, track: u64| match host.borrow_mut().apply(Command::DuplicateTrack(TrackId(track)))? {
            Outcome::Track(id) => Ok(id.0),
            _ => Err(fail("the track was not copied")),
        });
        def!("track_name", |_, track: u64| Ok(host.borrow().track(track)?.name.clone()));
        def!("set_track_name", |_, (track, name): (u64, String)| {
            host.borrow_mut().apply(Command::RenameTrack { track: TrackId(track), name }).map(|_| ())
        });
        def!("track_volume", |_, track: u64| Ok(db_of(host.borrow().track(track)?.gain)));
        def!("set_track_volume", |_, (track, db): (u64, f64)| {
            host.borrow_mut().apply(Command::SetTrackGain { track: TrackId(track), gain: gain_of(db) }).map(|_| ())
        });
        def!("track_pan", |_, track: u64| Ok(host.borrow().track(track)?.pan));
        def!("set_track_pan", |_, (track, pan): (u64, f32)| {
            host.borrow_mut().apply(Command::SetTrackPan { track: TrackId(track), pan }).map(|_| ())
        });
        def!("track_muted", |_, track: u64| Ok(host.borrow().track(track)?.muted));
        def!("set_track_muted", |_, (track, muted): (u64, bool)| {
            host.borrow_mut().apply(Command::SetTrackMuted { track: TrackId(track), muted }).map(|_| ())
        });
        def!("track_solo", |_, track: u64| Ok(host.borrow().track(track)?.solo));
        def!("set_track_solo", |_, (track, solo): (u64, bool)| {
            host.borrow_mut().apply(Command::SetTrackSolo { track: TrackId(track), solo }).map(|_| ())
        });
        def!("track_color", |_, track: u64| Ok(colour_text(host.borrow().track(track)?.colour)));
        def!("set_track_color", |_, (track, colour): (u64, Option<String>)| {
            let colour = match colour {
                Some(text) => Some(colour_from(&text).ok_or_else(|| fail(format!("{text} is not a #rrggbb colour")))?),
                None => None,
            };
            host.borrow_mut().apply(Command::SetTrackColour { track: TrackId(track), colour }).map(|_| ())
        });
        def!("track_clips", |lua, track: u64| {
            let h = host.borrow();
            lua.create_sequence_from(h.track(track)?.clips.iter().map(|clip| clip.id.0))
        });
        def!("clip_track", |_, clip: u64| {
            let h = host.borrow();
            h.clip(clip)?;
            Ok(h.project.track_of(ClipId(clip)).map(|track| track.id.0))
        });
        def!("clip_name", |_, clip: u64| Ok(host.borrow().clip(clip)?.source.name.clone()));
        def!("clip_start", |_, clip: u64| {
            let h = host.borrow();
            Ok(h.seconds(h.clip(clip)?.start))
        });
        def!("clip_length", |_, clip: u64| {
            let h = host.borrow();
            Ok(h.seconds(h.clip(clip)?.len))
        });
        def!("move_clip", |_, (clip, track, seconds): (u64, u64, f64)| {
            let mut h = host.borrow_mut();
            let start = h.frames(seconds);
            h.apply(Command::MoveClip { clip: ClipId(clip), track: TrackId(track), start }).map(|_| ())
        });
        def!("clip_volume", |_, clip: u64| Ok(db_of(host.borrow().clip(clip)?.gain)));
        def!("set_clip_volume", |_, (clip, db): (u64, f64)| {
            host.borrow_mut().apply(Command::SetClipGain { clip: ClipId(clip), gain: gain_of(db) }).map(|_| ())
        });
        def!("clip_muted", |_, clip: u64| Ok(host.borrow().clip(clip)?.muted));
        def!("set_clip_muted", |_, (clip, muted): (u64, bool)| {
            host.borrow_mut().apply(Command::SetClipMuted { clip: ClipId(clip), muted }).map(|_| ())
        });
        def!("split_clip", |_, (clip, seconds): (u64, f64)| {
            let mut h = host.borrow_mut();
            let at = h.frames(seconds);
            match h.apply(Command::SplitClip { clip: ClipId(clip), at })? {
                Outcome::Clip(id) => Ok(id.0),
                _ => Err(fail("the clip was not split")),
            }
        });
        def!("delete_clip", |_, clip: u64| {
            let mut h = host.borrow_mut();
            h.clip(clip)?;
            h.apply(Command::DeleteClip(ClipId(clip))).map(|_| ())
        });
        def!("is_note_clip", |_, clip: u64| Ok(host.borrow().clip(clip)?.notes.is_some()));
        def!("add_note_clip", |_, (track, seconds, length): (u64, f64, f64)| {
            let mut h = host.borrow_mut();
            let name = h.track(track)?.name.clone();
            let (start, len) = (h.frames(seconds), h.frames(length).max(1));
            match h.apply(Command::AddNotesClip { track: TrackId(track), name, start, len, notes: Vec::new() })? {
                Outcome::Clip(id) => Ok(id.0),
                _ => Err(fail("the clip was not made")),
            }
        });
        def!("notes", |lua, clip: u64| {
            let h = host.borrow();
            let found = h.clip(clip)?;
            let notes = found.notes.as_deref().ok_or_else(|| fail(format!("clip {clip} holds audio, not notes")))?;
            let offset = found.offset;
            let list = lua.create_table()?;
            for note in notes {
                let row = lua.create_table()?;
                row.set("key", note.key)?;
                row.set("start", h.seconds(note.start.saturating_sub(offset)))?;
                row.set("length", h.seconds(note.len))?;
                row.set("velocity", note.velocity)?;
                list.push(row)?;
            }
            Ok(list)
        });
        def!("set_notes", |_, (clip, list): (u64, Table)| {
            let mut h = host.borrow_mut();
            let offset = h.clip(clip)?.offset;
            if h.clip(clip)?.notes.is_none() {
                return Err(fail(format!("clip {clip} holds audio, not notes")));
            }
            let mut notes = Vec::new();
            for row in list.sequence_values::<Table>() {
                let row = row?;
                let key: i64 = row.get("key")?;
                if !(0..=HIGHEST_KEY).contains(&key) {
                    return Err(fail(format!("key {key} is outside 0 to 127")));
                }
                let start: f64 = row.get("start")?;
                let length: f64 = row.get("length")?;
                let velocity: Option<f32> = row.get("velocity")?;
                notes.push(Note { key: key as u8, start: offset + h.frames(start), len: h.frames(length).max(1), velocity: velocity.unwrap_or(0.8) });
            }
            h.apply(Command::SetNotes { clip: ClipId(clip), notes }).map(|_| ())
        });
        def!("selected_clips", |lua, ()| {
            let h = host.borrow();
            let chosen: Vec<u64> = h.wishes.select.as_ref().unwrap_or(&h.view.selected).iter().map(|clip| clip.0).collect();
            lua.create_sequence_from(chosen)
        });
        def!("select_clips", |_, clips: Vec<u64>| {
            let mut h = host.borrow_mut();
            for clip in &clips {
                h.clip(*clip)?;
            }
            h.wishes.select = Some(clips.into_iter().map(ClipId).collect());
            Ok(())
        });
        def!("clear_selection", |_, ()| {
            host.borrow_mut().wishes.select = Some(Vec::new());
            Ok(())
        });
        def!("track_plugins", |lua, track: u64| {
            let h = host.borrow();
            lua.create_sequence_from(h.track(track)?.fx.iter().map(|fx| fx.name.clone()))
        });
        def!("add_plugin", |_, (track, name): (u64, String)| {
            let mut h = host.borrow_mut();
            h.track(track)?;
            let wanted = name.to_lowercase();
            let plugins = &h.view.plugins;
            let found = plugins
                .iter()
                .find(|(known, _, _)| known.to_lowercase() == wanted)
                .or_else(|| plugins.iter().find(|(known, _, _)| known.to_lowercase().contains(&wanted)))
                .cloned()
                .ok_or_else(|| fail(format!("no plugin called {name} was found")))?;
            let (name, path, index) = found;
            h.apply(Command::AddFx { track: TrackId(track), fx: Fx { path, index, name, bypassed: false, state: Vec::new() } })?;
            Ok(h.track(track)?.fx.len())
        });
        def!("set_plugin_knob", |_, (track, slot, knob, value): (u64, usize, Value, f32)| {
            let mut h = host.borrow_mut();
            let fx = h.track(track)?.fx.get(slot.wrapping_sub(1)).cloned().ok_or_else(|| fail(format!("track {track} has no plugin in slot {slot}")))?;
            if !loupe_plugins::rack::is_built_in(&fx.path) {
                return Err(fail(format!("{} is not one of Loupe's own plugins, so scripts cannot turn its knobs yet", fx.name)));
            }
            let mut effect = loupe_stock::make(&fx.name).ok_or_else(|| fail(format!("Loupe has no plugin called {}", fx.name)))?;
            let params = effect.params();
            let index = match &knob {
                Value::Integer(number) => (*number as usize).wrapping_sub(1),
                Value::Number(number) => (*number as usize).wrapping_sub(1),
                Value::String(text) => {
                    let text = text.to_str()?.to_lowercase();
                    params.iter().position(|param| param.id == text || param.name.to_lowercase() == text).unwrap_or(usize::MAX)
                }
                _ => usize::MAX,
            };
            let param = params.get(index).ok_or_else(|| fail(format!("{} has no knob {}", fx.name, knob.to_string().unwrap_or_default())))?;
            let mut values: Vec<f32> = if fx.state.len() == params.len() * 4 {
                fx.state.chunks_exact(4).map(|four| f32::from_le_bytes([four[0], four[1], four[2], four[3]])).collect()
            } else {
                (0..params.len()).map(|i| effect.value(i)).collect()
            };
            let value = param.clamp(value);
            values[index] = value;
            effect.set(index, value);
            let state = values.iter().flat_map(|v| v.to_le_bytes()).collect();
            h.apply(Command::SetFxState { track: TrackId(track), slot: slot - 1, state })?;
            h.wishes.tweaks.push((TrackId(track), slot - 1, index, value));
            Ok(value)
        });
        def!("add_send", |_, (from, to, db): (u64, u64, Option<f64>)| {
            let mut h = host.borrow_mut();
            h.apply(Command::AddSend { from: TrackId(from), to: TrackId(to) })?;
            h.apply(Command::SetSendGain { from: TrackId(from), to: TrackId(to), gain: gain_of(db.unwrap_or(0.0)) }).map(|_| ())
        });
        def!("set_track_folder", |_, (track, folder): (u64, Option<u64>)| {
            host.borrow_mut().apply(Command::SetTrackParent { track: TrackId(track), parent: folder.map(TrackId) }).map(|_| ())
        });
        def!("add_automation_point", |_, (track, what, seconds, value): (u64, String, f64, f32)| {
            let mut h = host.borrow_mut();
            h.track(track)?;
            let (target, value) = match what.to_lowercase().as_str() {
                "volume" => (Target::TrackGain(TrackId(track)), gain_of(value as f64).clamp(0.0, 2.0)),
                "pan" => (Target::TrackPan(TrackId(track)), value.clamp(-1.0, 1.0)),
                _ => return Err(fail(format!("{what} cannot be automated from scripts yet; use \"volume\" or \"pan\""))),
            };
            if h.project.envelope(target).is_none() {
                h.apply(Command::AddEnvelope { target })?;
                h.apply(Command::ShowEnvelopeLane { target, open: true })?;
            }
            let at = h.frames(seconds);
            h.apply(Command::PutPoint { target, point: Point { at, value, shape: Shape::Linear } }).map(|_| ())
        });
        def!("set_loop", |_, (start, finish): (Option<f64>, Option<f64>)| {
            let mut h = host.borrow_mut();
            h.wishes.looped = Some(match (start, finish) {
                (Some(start), Some(finish)) if finish > start => Some((h.frames(start), h.frames(finish))),
                (None, None) => None,
                _ => return Err(fail("set_loop needs a start before the finish, or nothing to clear the loop")),
            });
            Ok(())
        });
        def!("import_audio", |_, (file, track, seconds): (String, u64, Option<f64>)| {
            let mut h = host.borrow_mut();
            h.track(track)?;
            let mut path = PathBuf::from(&file);
            if path.is_relative() {
                if let Some(folder) = &h.view.folder {
                    path = folder.join(path);
                }
            }
            let source = std::sync::Arc::new(Source::load(&path, h.project.rate).map_err(|why| fail(format!("could not read {file}: {why}")))?);
            let start = h.frames(seconds.unwrap_or(0.0));
            match h.apply(Command::AddClip { track: TrackId(track), source, start })? {
                Outcome::Clip(id) => Ok(id.0),
                _ => Err(fail("the clip was not placed")),
            }
        });
        def!("set_instrument", |_, (track, name): (u64, String)| {
            let instrument = match name.to_lowercase().as_str() {
                "synth" => Instrument::default(),
                "drums" => Instrument::Drums,
                _ => return Err(fail(format!("{name} is not an instrument; use \"synth\" or \"drums\""))),
            };
            host.borrow_mut().apply(Command::SetInstrument { track: TrackId(track), instrument }).map(|_| ())
        });
        def!("export", |_, stems: Option<bool>| {
            host.borrow_mut().wishes.export = Some(stems.unwrap_or(false));
            Ok(())
        });
        globals.set("loupe", api)?;
        lua.load(source).set_name(format!("={name}")).exec()
    });
    let host = host.into_inner();
    match result {
        Ok(()) => Ok(Ran { changed: host.changed, wishes: host.wishes }),
        Err(why) => Err(tidy_error(&why)),
    }
}

fn say(words: Variadic<Value>) -> String {
    words.iter().map(|value| value.to_string().unwrap_or_else(|_| "?".into())).collect::<Vec<_>>().join(" ")
}

fn tidy_error(error: &mlua::Error) -> String {
    let text = match error {
        mlua::Error::CallbackError { cause, .. } => return tidy_error(cause),
        mlua::Error::RuntimeError(text) | mlua::Error::SyntaxError { message: text, .. } => text.clone(),
        other => other.to_string(),
    };
    text.lines().next().unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song() -> Project {
        let mut project = Project::new(48_000);
        project.apply(Command::AddTrack { name: "Drums".into() }).unwrap();
        project
    }

    fn view() -> View {
        View {
            playhead: 48_000,
            playing: false,
            selected: Vec::new(),
            plugins: loupe_stock::NAMES.iter().enumerate().map(|(index, name)| (name.to_string(), PathBuf::from(loupe_plugins::BUILT_IN), index)).collect(),
            folder: None,
        }
    }

    fn ran(source: &str, project: &mut Project) -> Result<Ran, String> {
        run(source, "test", project, &view())
    }

    #[test]
    fn a_script_reads_and_changes_tracks() {
        let mut project = song();
        let done = ran(
            r##"
            local first = loupe.tracks()[1]
            loupe.set_track_name(first, loupe.track_name(first) .. " bus")
            local made = loupe.add_track("Keys")
            loupe.set_track_volume(made, -6)
            loupe.set_track_pan(made, -0.5)
            loupe.set_track_color(made, "#ff8800")
            print("tracks", #loupe.tracks(), loupe.track_color(made))
            "##,
            &mut project,
        )
        .unwrap();
        assert!(done.changed);
        assert_eq!(done.wishes.printed, vec!["tracks 2 #ff8800".to_string()]);
        assert_eq!(project.tracks[0].name, "Drums bus");
        assert!((db_of(project.tracks[1].gain) + 6.0).abs() < 1e-4);
        assert_eq!(project.tracks[1].pan, -0.5);
    }

    #[test]
    fn notes_go_in_and_come_back_in_seconds() {
        let mut project = song();
        let done = ran(
            r#"
            local track = loupe.tracks()[1]
            local clip = loupe.add_note_clip(track, 2, 4)
            local notes = {}
            for i = 0, 3 do
              notes[#notes + 1] = {key = 60 + i, start = i * 0.5, length = 0.25, velocity = 0.5}
            end
            loupe.set_notes(clip, notes)
            local back = loupe.notes(clip)
            print(#back, back[2].key, back[2].start, loupe.clip_start(clip), loupe.is_note_clip(clip))
            "#,
            &mut project,
        )
        .unwrap();
        assert_eq!(done.wishes.printed, vec!["4 61 0.5 2 true".to_string()]);
    }

    #[test]
    fn transport_and_selection_wishes_are_handed_back() {
        let mut project = song();
        let done = ran("loupe.set_playhead(loupe.beats_to_seconds(8)) loupe.play() print(loupe.playhead(), loupe.is_playing())", &mut project).unwrap();
        assert!(!done.changed);
        assert_eq!(done.wishes.seek, Some(192_000));
        assert_eq!(done.wishes.play, Some(true));
        assert_eq!(done.wishes.printed, vec!["4 true".to_string()]);
    }

    #[test]
    fn mistakes_come_back_as_plain_messages() {
        let mut project = song();
        assert_eq!(ran("loupe.track_name(99)", &mut project).unwrap_err(), "there is no track 99");
        assert_eq!(ran("loupe.set_bpm(5000)", &mut project).unwrap_err(), "that value is out of range");
        assert!(ran("this is not lua", &mut project).unwrap_err().contains("test"));
        assert!(ran("loupe.set_track_color(loupe.tracks()[1], 'red')", &mut project).unwrap_err().contains("not a #rrggbb"));
    }

    #[test]
    fn scripts_cannot_reach_files_or_run_forever() {
        let mut project = song();
        for blocked in ["io.open('x')", "os.execute('ls')", "dofile('x')", "load('return 1')()", "require('x')"] {
            assert!(ran(blocked, &mut project).is_err(), "{blocked} was allowed");
        }
        let started = Instant::now();
        let stuck = ran("while true do end", &mut project).unwrap_err();
        assert!(stuck.contains("more than 5 seconds"), "{stuck}");
        assert!(started.elapsed() < Duration::from_secs(8));
    }

    #[test]
    fn plugins_can_be_added_and_their_knobs_turned() {
        let mut project = song();
        let done = ran(
            r#"
            local track = loupe.tracks()[1]
            local slot = loupe.add_plugin(track, "loupe compressor")
            local set = loupe.set_plugin_knob(track, slot, "threshold", -30)
            local clamped = loupe.set_plugin_knob(track, slot, 2, 500)
            print(slot, loupe.track_plugins(track)[1], set, clamped)
            "#,
            &mut project,
        )
        .unwrap();
        assert_eq!(done.wishes.printed, vec!["1 Loupe Compressor -30 20".to_string()]);
        let fx = &project.tracks[0].fx[0];
        let values: Vec<f32> = fx.state.chunks_exact(4).map(|four| f32::from_le_bytes([four[0], four[1], four[2], four[3]])).collect();
        assert_eq!((values[0], values[1]), (-30.0, 20.0));
        assert_eq!(done.wishes.tweaks, vec![(project.tracks[0].id, 0, 0, -30.0), (project.tracks[0].id, 0, 1, 20.0)]);
        assert!(ran("loupe.add_plugin(loupe.tracks()[1], 'Fog machine')", &mut project).unwrap_err().contains("no plugin called"));
        assert!(ran("loupe.set_plugin_knob(loupe.tracks()[1], 1, 'colour', 1)", &mut project).unwrap_err().contains("has no knob"));
    }

    #[test]
    fn routing_automation_and_instruments_from_a_script() {
        let mut project = song();
        let done = ran(
            r#"
            local drums = loupe.tracks()[1]
            local verb = loupe.add_track("Verb")
            local bus = loupe.add_track("Bus")
            loupe.add_send(drums, verb, -12)
            loupe.set_track_folder(drums, bus)
            loupe.add_automation_point(drums, "volume", 2, -6)
            loupe.add_automation_point(drums, "pan", 4, -2)
            loupe.set_instrument(verb, "drums")
            loupe.set_loop(1, 3)
            loupe.export(true)
            "#,
            &mut project,
        )
        .unwrap();
        let drums = &project.tracks.iter().find(|t| t.name == "Drums").unwrap();
        assert_eq!(drums.sends.len(), 1);
        assert!((db_of(drums.sends[0].gain) + 12.0).abs() < 1e-3);
        assert!(drums.parent.is_some());
        let volume = project.envelope(Target::TrackGain(drums.id)).unwrap();
        assert!(volume.points.iter().any(|p| p.at == 96_000 && (db_of(p.value) + 6.0).abs() < 1e-3));
        let pan = project.envelope(Target::TrackPan(drums.id)).unwrap();
        assert!(pan.points.iter().any(|p| p.at == 192_000 && p.value == -1.0));
        assert!(project.tracks.iter().any(|t| t.instrument == Instrument::Drums));
        assert_eq!(done.wishes.looped, Some(Some((48_000, 144_000))));
        assert_eq!(done.wishes.export, Some(true));
        assert!(ran("loupe.set_loop(3, 1)", &mut project).is_err());
        assert_eq!(ran("loupe.set_loop()", &mut project).unwrap().wishes.looped, Some(None));
        assert!(ran("loupe.add_automation_point(loupe.tracks()[1], 'reverb', 1, 1)", &mut project).is_err());
    }

    #[test]
    fn audio_files_can_be_placed_from_a_script() {
        let folder = std::env::temp_dir().join(format!("loupe-script-audio-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let frames = 4_800u32;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + frames * 4).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&48_000u32.to_le_bytes());
        wav.extend_from_slice(&(48_000u32 * 4).to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(frames * 4).to_le_bytes());
        for i in 0..frames {
            let value = ((i as f32 * 0.05).sin() * 8_000.0) as i16;
            wav.extend_from_slice(&value.to_le_bytes());
            wav.extend_from_slice(&value.to_le_bytes());
        }
        std::fs::write(folder.join("hit.wav"), wav).unwrap();
        let mut project = song();
        let mut here = view();
        here.folder = Some(folder.clone());
        let done = run("local clip = loupe.import_audio('hit.wav', loupe.tracks()[1], 1.5) print(loupe.clip_start(clip), loupe.clip_length(clip))", "test", &mut project, &here).unwrap();
        assert_eq!(done.wishes.printed, vec!["1.5 0.1".to_string()]);
        assert!(run("loupe.import_audio('missing.wav', loupe.tracks()[1], 0)", "test", &mut project, &here).unwrap_err().contains("could not read"));
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_shortcut_line_is_found_near_the_top() {
        assert_eq!(shortcut_of("-- shortcut: Ctrl+Alt+1\nprint(1)"), Some("Ctrl+Alt+1".into()));
        assert_eq!(shortcut_of("-- Shortcut : ctrl+k\n"), Some("ctrl+k".into()));
        assert_eq!(shortcut_of("print(1)"), None);
    }

    #[test]
    fn every_listed_function_exists_and_none_are_missing_from_the_list() {
        let mut project = song();
        let listed: Vec<&str> = FUNCTIONS.iter().map(|(call, _)| call.split('(').next().unwrap()).filter(|name| *name != "print").collect();
        let check = format!(
            "local names = {{}} for name in pairs(loupe) do names[#names + 1] = name end table.sort(names) print(table.concat(names, ','))"
        );
        let done = ran(&check, &mut project).unwrap();
        let mut known: Vec<&str> = done.wishes.printed[0].split(',').filter(|name| *name != "print").collect();
        known.sort();
        let mut expected = listed.clone();
        expected.sort();
        assert_eq!(known, expected);
    }
}
