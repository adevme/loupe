use std::fs;
use std::path::{Path, PathBuf};

use iced::{keyboard, Element, Task};
use loupe_engine::{Command, Outcome};

use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::scripting::{self, Fetch, Record, Response, View, FUNCTIONS};
use crate::{settings, App, Message};

const FOLDER: &str = "Scripts";
const EXTENSION: &str = "lua";
const REFERENCE: &str = "Loupe script functions.txt";
const MOST_PRINTED: usize = 3;
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
const MOST_RESPONSE_BYTES: u64 = 1024 * 1024;
const SCRIPTS_MENU_WIDTH: f32 = 320.0;
const EXAMPLES: [(&str, &str); 2] = [
    (
        "Number the tracks.lua",
        "-- shortcut: Ctrl+Alt+1\nfor i, track in ipairs(loupe.tracks()) do\n  local name = loupe.track_name(track):gsub(\"^%d+ \", \"\")\n  loupe.set_track_name(track, string.format(\"%02d %s\", i, name))\nend\n",
    ),
    (
        "Humanize selected notes.lua",
        "-- shortcut: Ctrl+Alt+2\nfor _, clip in ipairs(loupe.selected_clips()) do\n  if loupe.is_note_clip(clip) then\n    local notes = loupe.notes(clip)\n    for _, note in ipairs(notes) do\n      note.velocity = math.max(0.05, math.min(1, note.velocity + (math.random() - 0.5) * 0.2))\n      note.start = math.max(0, note.start + (math.random() - 0.5) * 0.01)\n    end\n    loupe.set_notes(clip, notes)\n  end\nend\n",
    ),
];

#[derive(Clone, Debug, PartialEq)]
pub struct Keys {
    ctrl: bool,
    alt: bool,
    shift: bool,
    key: String,
}

impl Keys {
    pub fn parse(text: &str) -> Option<Self> {
        let mut keys = Self { ctrl: false, alt: false, shift: false, key: String::new() };
        for part in text.split('+').map(|part| part.trim().to_lowercase()) {
            match part.as_str() {
                "ctrl" | "control" | "cmd" => keys.ctrl = true,
                "alt" | "option" => keys.alt = true,
                "shift" => keys.shift = true,
                key if key.chars().count() == 1 && keys.key.is_empty() => keys.key = key.to_string(),
                _ => return None,
            }
        }
        (!keys.key.is_empty() && (keys.ctrl || keys.alt)).then_some(keys)
    }

    pub fn matches(&self, key: &str, modifiers: keyboard::Modifiers) -> bool {
        self.key == key.to_lowercase() && self.ctrl == modifiers.command() && self.alt == modifiers.alt() && self.shift == modifiers.shift()
    }
}

#[derive(Clone, Debug)]
pub struct Script {
    pub name: String,
    pub path: PathBuf,
    pub shortcut: Option<String>,
    keys: Option<Keys>,
}

fn reference_text() -> String {
    let mut text = String::from("Loupe scripts are Lua 5.4 files in this folder. Every function lives in the loupe table, for example loupe.tracks().\nTimes are in seconds, volumes in dB, pan from -1 to 1. A script is one undo step, and if it fails nothing changes.\nPut a line like -- shortcut: Ctrl+Alt+3 at the top of a script to give it a key.\n\n");
    for (call, about) in FUNCTIONS {
        let call = if call.starts_with("print") { call.to_string() } else { format!("loupe.{call}") };
        text.push_str(&format!("{call}\n    {about}\n"));
    }
    text
}

fn prepare_folder(folder: &Path) {
    if !folder.exists() && fs::create_dir_all(folder).is_ok() {
        for (name, body) in EXAMPLES {
            let _ = fs::write(folder.join(name), body);
        }
    }
    let reference = folder.join(REFERENCE);
    let wanted = reference_text();
    if fs::read_to_string(&reference).ok().as_deref() != Some(wanted.as_str()) {
        let _ = fs::write(reference, wanted);
    }
}

pub fn find(folder: &Path) -> Vec<Script> {
    prepare_folder(folder);
    let mut found: Vec<Script> = fs::read_dir(folder)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case(EXTENSION)))
        .map(|path| {
            let source = fs::read_to_string(&path).unwrap_or_default();
            let shortcut = scripting::shortcut_of(&source);
            let keys = shortcut.as_deref().and_then(Keys::parse);
            let name = path.file_stem().map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
            Script { name, path, shortcut, keys }
        })
        .collect();
    found.sort_by_key(|script| script.name.to_lowercase());
    found
}

impl App {
    pub(crate) fn scripts_folder(&self) -> PathBuf {
        settings::home_folder(self.folder.as_deref()).join(FOLDER)
    }

    pub(crate) fn find_scripts(&mut self) {
        self.scripts = find(&self.scripts_folder());
    }

    pub(crate) fn scripts_menu(&self) -> Element<'_, Message> {
        let mut items: Vec<Element<'_, Message>> = vec![self.item("Picking a script runs it now. Ctrl+Z undoes it.", "", None)];
        items.extend(self
            .scripts
            .iter()
            .map(|script| self.item(&script.name, script.shortcut.as_deref().unwrap_or(""), Some(Message::RunScript(script.path.clone()))))
            .collect::<Vec<_>>());
        if items.len() == 1 {
            items.push(self.item("No scripts yet", "", None));
        }
        items.push(crate::rule(self.palette));
        items.push(self.item("Open the scripts folder", "", Some(Message::OpenScriptsFolder)));
        self.menu_sized(items, SCRIPTS_MENU_WIDTH)
    }

    pub(crate) fn open_scripts_folder(&mut self) {
        self.overlay = crate::Overlay::None;
        let folder = self.scripts_folder();
        prepare_folder(&folder);
        if let Err(why) = crate::files::show_in_folder(&folder.join(REFERENCE)) {
            self.problem = Some(format!("Could not open {}: {why}", folder.display()));
        }
    }

    pub(crate) fn script_key(&mut self, key: &str, modifiers: keyboard::Modifiers) -> Task<Message> {
        let found = self.scripts.iter().find(|script| script.keys.as_ref().is_some_and(|keys| keys.matches(key, modifiers)));
        match found.map(|script| script.path.clone()) {
            Some(path) => self.run_script(&path),
            None => Task::none(),
        }
    }

    pub(crate) fn run_script(&mut self, path: &Path) -> Task<Message> {
        self.overlay = crate::Overlay::None;
        let name = path.file_stem().map_or_else(|| "script".to_string(), |stem| stem.to_string_lossy().into_owned());
        let source = match fs::read_to_string(path) {
            Ok(source) => source,
            Err(why) => {
                self.problem = Some(format!("Could not read the script {name}: {why}"));
                return Task::none();
            }
        };
        if let Some(every) = scripting::every_of(&source) {
            if let Some(at) = self.background.iter().position(|running| running.path == path) {
                self.background.remove(at);
                self.notice = Some(format!("Stopped the script \"{name}\"."));
                return Task::none();
            }
            self.background.push(Background::new(path.to_path_buf(), name.clone(), every));
            self.notice = Some(format!("The script \"{name}\" is running every {:.2} s. Pick it again to stop it.", every.as_secs_f64()));
            return self.run_background(self.background.len() - 1, source);
        }
        let mut plugins: Vec<(String, PathBuf, usize)> = self.found.iter().map(|found| (found.name.clone(), found.path.clone(), found.index)).collect();
        for (index, name) in loupe_stock::NAMES.iter().enumerate() {
            if !plugins.iter().any(|(known, _, _)| known == name) {
                plugins.push((name.to_string(), PathBuf::from(loupe_plugins::BUILT_IN), index));
            }
        }
        let view = View { plugins, ..self.script_view() };
        let hosted = self.project.tracks.iter().flat_map(|track| track.fx.iter()).any(|fx| !loupe_plugins::rack::is_built_in(&fx.path));
        let mut racks = if hosted { self.borrow_racks() } else { None };
        if let Some(running) = racks.as_deref_mut() {
            for (track, slot, state) in running.harvest() {
                let _ = self.project.apply(Command::SetFxState { track, slot, state });
            }
        }
        let before = self.project.clone();
        let ran = scripting::run(&source, &name, &mut self.project, &view, racks.as_deref_mut());
        let after = std::mem::replace(&mut self.project, before);
        if let Some(racks) = racks {
            self.racks.keep(racks);
        }
        if self.racks.holding() {
            self.follow_chains();
        }
        self.finish_script(&name, ran, after, false)
    }

    fn script_view(&self) -> View {
        let folder = self.path.as_deref().and_then(Path::parent).map(Path::to_path_buf);
        View {
            playhead: self.playhead,
            playing: self.playing,
            recording: self.recording.is_some(),
            armed: self.armed.iter().copied().collect(),
            selected: self.selection.iter().copied().collect(),
            folder,
            ..View::default()
        }
    }

    /// Runs the repeating scripts whose turn has come.
    pub(crate) fn run_due_scripts(&mut self) -> Task<Message> {
        let now = Instant::now();
        let mut tasks = Vec::new();
        let mut at = 0;
        while at < self.background.len() {
            if self.background[at].next > now {
                at += 1;
                continue;
            }
            let path = self.background[at].path.clone();
            match fs::read_to_string(&path) {
                Ok(source) => {
                    let before = self.background.len();
                    tasks.push(self.run_background(at, source));
                    if self.background.len() == before {
                        at += 1;
                    }
                }
                Err(why) => {
                    let gone = self.background.remove(at);
                    self.problem = Some(format!("Stopped the script {}: could not read it: {why}", gone.name));
                }
            }
        }
        Task::batch(tasks)
    }

    /// One turn of a repeating script. It never borrows the plugins, so the sound does not stutter while it runs.
    fn run_background(&mut self, at: usize, source: String) -> Task<Message> {
        let running = &mut self.background[at];
        running.next = Instant::now() + running.every;
        let responses = running.inbox.lock().map(|mut inbox| std::mem::take(&mut *inbox)).unwrap_or_default();
        let name = running.name.clone();
        let view = View { responses, kept: running.kept.clone(), next_fetch: running.next_fetch, ..self.script_view() };
        let before = self.project.clone();
        let ran = scripting::run(&source, &name, &mut self.project, &view, None);
        let after = std::mem::replace(&mut self.project, before);
        if let Ok(done) = &ran {
            let running = &mut self.background[at];
            running.next_fetch += done.wishes.fetches.len() as u64;
            for (key, value) in &done.wishes.keep {
                match value {
                    Some(value) => running.kept.insert(key.clone(), value.clone()),
                    None => running.kept.remove(key),
                };
            }
            for fetch in &done.wishes.fetches {
                ask(fetch.clone(), running.inbox.clone());
            }
        } else {
            self.background.remove(at);
        }
        self.finish_script(&name, ran, after, true)
    }

    fn finish_script(&mut self, name: &str, ran: Result<scripting::Ran, String>, after: loupe_engine::Project, quiet: bool) -> Task<Message> {
        if matches!(&ran, Ok(done) if done.changed) {
            self.transact(None, |project| {
                *project = after;
                Ok(Outcome::Done)
            });
        }
        self.forget_gone_clips();
        let ran = match ran {
            Ok(ran) => ran,
            Err(why) => {
                self.problem = Some(match quiet {
                    true => format!("Stopped the script {name}: {why}"),
                    false => format!("Script {name}: {why}"),
                });
                return Task::none();
            }
        };
        let wishes = ran.wishes;
        if let Some(chosen) = wishes.select {
            self.choose(chosen);
        }
        if let Some(at) = wishes.seek {
            self.seek(at);
        }
        for (track, slot, knob, value) in wishes.tweaks {
            self.engine.tweak(track, slot, knob, value);
        }
        if let Some(range) = wishes.looped {
            self.set_loop(range);
            self.cache.clear();
        }
        let printed = wishes.printed.iter().rev().take(MOST_PRINTED).rev().cloned().collect::<Vec<_>>().join("  ·  ");
        if !quiet || !printed.is_empty() {
            self.notice = Some(match (printed.is_empty(), ran.changed) {
                (true, true) => format!("Ran the script \"{name}\": it changed your song. Ctrl+Z undoes it."),
                (true, false) => format!("Ran the script \"{name}\": nothing in the song changed."),
                (false, _) => printed,
            });
        }
        let mut tasks = Vec::new();
        for (track, armed) in wishes.arm {
            if self.armed.contains(&track) != armed {
                tasks.push(self.handle(Message::ToggleArm(track)));
            }
        }
        if wishes.record.is_none() && wishes.play.is_some_and(|play| play != self.playing) {
            tasks.push(self.handle(Message::TogglePlay));
        }
        match wishes.record {
            Some(Record::Start) if self.recording.is_none() => tasks.push(self.toggle_recording()),
            Some(Record::Stop) if self.recording.is_some() => tasks.push(self.finish_recording()),
            Some(Record::Next) => {
                if self.recording.is_some() {
                    tasks.push(self.finish_recording());
                }
                tasks.push(self.toggle_recording());
            }
            _ => {}
        }
        if let Some(stems) = wishes.export {
            self.export.split = stems;
            tasks.push(self.start_export());
        }
        Task::batch(tasks)
    }

    /// Starts the repeating scripts marked to start by themselves.
    pub(crate) fn autostart_scripts(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        for script in self.scripts.clone() {
            let Ok(source) = fs::read_to_string(&script.path) else { continue };
            let (Some(every), true) = (scripting::every_of(&source), scripting::autostarts(&source)) else { continue };
            if self.background.iter().any(|running| running.path == script.path) {
                continue;
            }
            self.background.push(Background::new(script.path.clone(), script.name.clone(), every));
            tasks.push(self.run_background(self.background.len() - 1, source));
        }
        Task::batch(tasks)
    }
}

/// A script that runs again every so often, with what it keeps between runs.
pub struct Background {
    pub path: PathBuf,
    pub name: String,
    every: Duration,
    next: Instant,
    kept: HashMap<String, String>,
    next_fetch: u64,
    inbox: Arc<Mutex<Vec<Response>>>,
}

impl Background {
    fn new(path: PathBuf, name: String, every: Duration) -> Self {
        Self { path, name, every, next: Instant::now(), kept: HashMap::new(), next_fetch: 1, inbox: Arc::default() }
    }
}

/// Asks a web address on its own thread; the answer waits in the inbox for the script's next run.
fn ask(fetch: Fetch, inbox: Arc<Mutex<Vec<Response>>>) {
    std::thread::spawn(move || {
        let agent = ureq::AgentBuilder::new().timeout(FETCH_TIMEOUT).build();
        let mut request = agent.request(&fetch.method, &fetch.url);
        for (name, value) in &fetch.headers {
            request = request.set(name, value);
        }
        let answer = match &fetch.body {
            Some(body) => request.send_string(body),
            None => request.call(),
        };
        let (status, body) = match answer {
            Ok(reply) | Err(ureq::Error::Status(_, reply)) => {
                let status = reply.status();
                let mut body = String::new();
                let _ = reply.into_reader().take(MOST_RESPONSE_BYTES).read_to_string(&mut body);
                (status, body)
            }
            Err(why) => (0, why.to_string()),
        };
        if let Ok(mut inbox) = inbox.lock() {
            inbox.push(Response { id: fetch.id, status, body });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_need_ctrl_or_alt_and_one_key() {
        let keys = Keys::parse("Ctrl+Alt+1").unwrap();
        let both = keyboard::Modifiers::CTRL | keyboard::Modifiers::ALT;
        assert!(keys.matches("1", both));
        assert!(!keys.matches("1", keyboard::Modifiers::CTRL));
        assert!(!keys.matches("2", both));
        assert!(Keys::parse("1").is_none());
        assert!(Keys::parse("Ctrl+").is_none());
        assert!(Keys::parse("Ctrl+ab").is_none());
        assert!(Keys::parse("Shift+k").is_none());
        assert!(Keys::parse("ctrl + shift + K").unwrap().matches("k", keyboard::Modifiers::CTRL | keyboard::Modifiers::SHIFT));
    }

    #[test]
    fn a_new_folder_gets_examples_and_the_function_list() {
        let folder = std::env::temp_dir().join(format!("loupe-scripts-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        let found = find(&folder);
        assert_eq!(found.len(), EXAMPLES.len());
        assert_eq!(found[1].name, "Number the tracks");
        assert_eq!(found[1].shortcut.as_deref(), Some("Ctrl+Alt+1"));
        let reference = fs::read_to_string(folder.join(REFERENCE)).unwrap();
        assert!(reference.contains("loupe.set_notes(clip, notes)"));
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_repeating_script_fetches_hears_the_answer_next_run_and_stops_when_picked_again() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut asked = [0u8; 1024];
                let _ = stream.read(&mut asked);
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\npong");
            }
        });
        let folder = std::env::temp_dir().join(format!("loupe-repeat-{}", std::process::id()));
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join("poll.lua");
        let body = format!(
            "-- every: 1\nfor _, r in ipairs(loupe.responses()) do loupe.keep(\"heard\", r.status .. \" \" .. r.body) end\nif not loupe.kept(\"asked\") then loupe.fetch(\"http://127.0.0.1:{port}/\") loupe.keep(\"asked\", \"yes\") end\n"
        );
        fs::write(&path, body).unwrap();
        let mut app = App::bare();
        let _ = app.run_script(&path);
        assert_eq!(app.background.len(), 1, "the script did not keep running");
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.background[0].inbox.lock().unwrap().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        app.background[0].next = Instant::now();
        let _ = app.run_due_scripts();
        assert_eq!(app.background[0].kept.get("heard").map(String::as_str), Some("200 pong"));
        let _ = app.run_script(&path);
        assert!(app.background.is_empty(), "picking the script again did not stop it");
        fs::remove_dir_all(folder).ok();
    }

    #[test]
    fn the_examples_run_cleanly() {
        let mut project = loupe_engine::Project::new(48_000);
        project.apply(loupe_engine::Command::AddTrack { name: "Drums".into() }).unwrap();
        let view = View::default();
        for (name, body) in EXAMPLES {
            scripting::run(body, name, &mut project, &view, None).unwrap_or_else(|why| panic!("{name}: {why}"));
        }
        assert_eq!(project.tracks[0].name, "01 Drums");
        scripting::run(EXAMPLES[0].1, "again", &mut project, &view, None).unwrap();
        assert_eq!(project.tracks[0].name, "01 Drums");
    }
}
