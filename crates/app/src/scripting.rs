use std::cell::RefCell;
use std::time::{Duration, Instant};

use loupe_engine::{ClipId, Command, CommandError, Frames, Note, Outcome, Project, TrackId};
use mlua::{Lua, LuaOptions, StdLib, Table, Value, Variadic};

const LONGEST_RUN: Duration = Duration::from_secs(5);
const CHECK_EVERY: u32 = 10_000;
const HIGHEST_KEY: i64 = 127;

pub const FUNCTIONS: [(&str, &str); 47] = [
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
];

pub struct View {
    pub playhead: Frames,
    pub playing: bool,
    pub selected: Vec<ClipId>,
}

#[derive(Debug, Default, PartialEq)]
pub struct Wishes {
    pub play: Option<bool>,
    pub seek: Option<Frames>,
    pub select: Option<Vec<ClipId>>,
    pub printed: Vec<String>,
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
        View { playhead: 48_000, playing: false, selected: Vec::new() }
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
