use std::fs;
use std::path::{Path, PathBuf};

use iced::{keyboard, Element, Task};
use loupe_engine::{Command, Outcome};

use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use crate::scripting::{self, Event, Fetch, Machine, Record, Response, Turn, View, EVENTS, FUNCTIONS};
use crate::{settings, App, Message};

const FOLDER: &str = "Scripts";
const EXTENSION: &str = "lua";
const REFERENCE: &str = "Loupe script functions.txt";
const MOST_PRINTED: usize = 3;
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
const MOST_RESPONSE_BYTES: u64 = 1024 * 1024;
const SCRIPTS_MENU_WIDTH: f32 = 320.0;
const MOST_EVENT_ROUNDS: usize = 8;
const SHORTEST_TICK_MS: u64 = 10;
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
    let mut text = String::from("Loupe scripts are Lua 5.4 files in this folder. Every function lives in the loupe table, for example loupe.tracks().\nTimes are in seconds, volumes in dB, pan from -1 to 1. A script is one undo step, and if it fails nothing changes.\nPut a line like -- shortcut: Ctrl+Alt+3 at the top of a script to give it a key.\nA script that calls loupe.on, loupe.every, loupe.after or fetch with a handler keeps running after it is picked, until it is picked again. It reloads by itself when you save its file.\nPut -- autostart: yes at the top to start it whenever Loupe opens.\nA handler gets one table, e, that says what happened.\n\n");
    for (call, about) in FUNCTIONS {
        let call = if call.starts_with("print") { call.to_string() } else { format!("loupe.{call}") };
        text.push_str(&format!("{call}\n    {about}\n"));
    }
    text.push_str("\nEvents for loupe.on\n\n");
    for (event, about) in EVENTS {
        text.push_str(&format!("{event}\n    {about}\n"));
    }
    text.push_str("\nExample: hear Loupe quieter while you record\n\n-- autostart: yes\nloupe.on(\"record_start\", function(e) loupe.set_listen_volume(-6) end)\nloupe.on(\"record_stop\", function(e) loupe.set_listen_volume(0) end)\n");
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
            .map(|script| {
                let running = self.residents.iter().any(|resident| resident.path == script.path);
                let side = if running { "running" } else { script.shortcut.as_deref().unwrap_or("") };
                self.item(&script.name, side, Some(Message::RunScript(script.path.clone())))
            })
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
        let name = script_name(path);
        if let Some(at) = self.residents.iter().position(|resident| resident.path == path) {
            let stopped = self.stop_script(at);
            self.notice = Some(format!("Stopped the script \"{name}\"."));
            return stopped;
        }
        let source = match fs::read_to_string(path) {
            Ok(source) => source,
            Err(why) => {
                self.problem = Some(format!("Could not read the script {name}: {why}"));
                return Task::none();
            }
        };
        let machine = match Machine::new(&name) {
            Ok(machine) => machine,
            Err(why) => {
                self.problem = Some(format!("Script {name}: {why}"));
                return Task::none();
            }
        };
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
        let done = machine.begin(&source, Turn { project: self.project.clone(), view, plugins: racks });
        if let Some(racks) = done.plugins {
            self.racks.keep(racks);
        }
        if self.racks.holding() {
            self.follow_chains();
        }
        let stays = done.ran.is_ok() && machine.listens();
        let waits_for = machine.waits_for();
        if stays {
            self.residents.push(Resident::new(path.to_path_buf(), name.clone(), machine));
        }
        let finished = self.finish_script(&name, Some(path), done.ran, done.project, stays);
        if stays {
            self.notice = Some(format!("The script \"{name}\" is running and waits for {waits_for}. Pick it again to stop it."));
        }
        finished
    }

    fn stop_script(&mut self, at: usize) -> Task<Message> {
        let resident = self.residents.remove(at);
        let finished = match resident.machine.wants(&Event::Unload) {
            true => {
                let done = resident.machine.deliver(&Event::Unload, self.turn());
                self.finish_script(&resident.name, None, done.ran, done.project, true)
            }
            false => Task::none(),
        };
        if self.listen_owner.as_deref() == Some(resident.path.as_path()) {
            self.set_listen(1.0, None);
        }
        finished
    }

    fn turn(&self) -> Turn {
        Turn { project: self.project.clone(), view: self.script_view(), plugins: None }
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
            listen: self.listen_level,
            ..View::default()
        }
    }

    pub(crate) fn happened(&mut self, event: Event) {
        if self.residents.iter().any(|resident| resident.machine.wants(&event)) {
            self.script_events.push(event);
        }
    }

    pub(crate) fn note_retake(&mut self) {
        if let Some(Event::RecordStart { retake, .. }) = self.script_events.iter_mut().rev().find(|event| matches!(event, Event::RecordStart { .. })) {
            *retake = true;
        }
    }

    pub(crate) fn deliver_script_events(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        let mut rounds = 0;
        while !self.script_events.is_empty() {
            rounds += 1;
            if rounds > MOST_EVENT_ROUNDS {
                let names: Vec<&str> = self.script_events.iter().map(Event::name).collect();
                self.problem = Some(format!("Scripts kept setting each other off, so Loupe stopped passing on {} after {MOST_EVENT_ROUNDS} rounds.", names.join(", ")));
                self.script_events.clear();
                break;
            }
            for event in std::mem::take(&mut self.script_events) {
                let mut at = 0;
                while at < self.residents.len() {
                    if !self.residents[at].machine.wants(&event) {
                        at += 1;
                        continue;
                    }
                    let done = self.residents[at].machine.deliver(&event, self.turn());
                    let (name, path) = (self.residents[at].name.clone(), self.residents[at].path.clone());
                    if done.ran.is_err() {
                        self.residents.remove(at);
                        self.release_listen_of(&path);
                    } else {
                        at += 1;
                    }
                    tasks.push(self.finish_script(&name, Some(&path), done.ran, done.project, true));
                }
            }
        }
        Task::batch(tasks)
    }

    pub(crate) fn script_tick(&self) -> Option<Duration> {
        if self.residents.is_empty() {
            return None;
        }
        let shortest = self.residents.iter().filter_map(|resident| resident.machine.shortest_timer()).min();
        let tick = Duration::from_millis(crate::SCRIPT_CHECK_MS);
        Some(shortest.map_or(tick, |gap| gap.clamp(Duration::from_millis(SHORTEST_TICK_MS), tick)))
    }

    pub(crate) fn run_due_scripts(&mut self) -> Task<Message> {
        let mut tasks = vec![self.reload_changed_scripts()];
        let now = Instant::now();
        let mut at = 0;
        while at < self.residents.len() {
            let answers = self.residents[at].inbox.lock().map(|mut inbox| std::mem::take(&mut *inbox)).unwrap_or_default();
            if !self.residents[at].machine.is_due(now, !answers.is_empty()) {
                at += 1;
                continue;
            }
            let mut turn = self.turn();
            turn.view.responses = answers;
            let done = self.residents[at].machine.tick(now, turn);
            let (name, path) = (self.residents[at].name.clone(), self.residents[at].path.clone());
            let finished = done.ran.is_err() || !self.residents[at].machine.listens();
            if finished {
                self.residents.remove(at);
                if done.ran.is_err() {
                    self.release_listen_of(&path);
                }
            } else {
                at += 1;
            }
            tasks.push(self.finish_script(&name, Some(&path), done.ran, done.project, true));
        }
        Task::batch(tasks)
    }

    fn reload_changed_scripts(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        let changed: Vec<usize> = self.residents.iter().enumerate().filter(|(_, resident)| resident.written() != resident.written_at).map(|(at, _)| at).collect();
        for at in changed.into_iter().rev() {
            let path = self.residents[at].path.clone();
            let name = self.residents[at].name.clone();
            tasks.push(self.stop_script(at));
            if !path.exists() {
                self.notice = Some(format!("The script \"{name}\" was deleted, so it stopped."));
                continue;
            }
            tasks.push(self.run_script(&path));
            if self.residents.iter().any(|resident| resident.path == path) {
                self.notice = Some(format!("Reloaded the script \"{name}\" after its file changed."));
            }
        }
        Task::batch(tasks)
    }

    fn release_listen_of(&mut self, path: &Path) {
        if self.listen_owner.as_deref() == Some(path) {
            self.set_listen(1.0, None);
        }
    }

    fn set_listen(&mut self, level: f32, by: Option<&Path>) {
        self.listen_level = level;
        self.engine.set_listen_level(level);
        self.listen_owner = (level < 1.0).then(|| by.map(Path::to_path_buf)).flatten();
    }

    fn finish_script(&mut self, name: &str, from: Option<&Path>, ran: Result<scripting::Ran, String>, after: loupe_engine::Project, quiet: bool) -> Task<Message> {
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
        if let Some(level) = wishes.listen {
            self.set_listen(level, from);
        }
        let inbox = from.and_then(|path| self.residents.iter().find(|resident| resident.path == path)).map_or_else(Arc::default, |resident| resident.inbox.clone());
        for fetch in &wishes.fetches {
            ask(fetch.clone(), inbox.clone());
        }
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
            Some(Record::Next) => tasks.push(self.next_take()),
            _ => {}
        }
        if let Some(stems) = wishes.export {
            self.export.split = stems;
            tasks.push(self.start_export());
        }
        Task::batch(tasks)
    }

    pub(crate) fn autostart_scripts(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        for script in self.scripts.clone() {
            let Ok(source) = fs::read_to_string(&script.path) else { continue };
            if !scripting::autostarts(&source) || self.residents.iter().any(|resident| resident.path == script.path) {
                continue;
            }
            tasks.push(self.run_script(&script.path));
        }
        Task::batch(tasks)
    }
}

fn script_name(path: &Path) -> String {
    path.file_stem().map_or_else(|| "script".to_string(), |stem| stem.to_string_lossy().into_owned())
}

pub struct Resident {
    pub path: PathBuf,
    pub name: String,
    machine: Machine,
    written_at: Option<SystemTime>,
    inbox: Arc<Mutex<Vec<Response>>>,
}

impl Resident {
    fn new(path: PathBuf, name: String, machine: Machine) -> Self {
        let written_at = fs::metadata(&path).and_then(|meta| meta.modified()).ok();
        Self { path, name, machine, written_at, inbox: Arc::default() }
    }

    fn written(&self) -> Option<SystemTime> {
        fs::metadata(&self.path).and_then(|meta| meta.modified()).ok()
    }
}

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

    fn answer_once(body: &'static str) -> u16 {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut asked = [0u8; 1024];
                let _ = stream.read(&mut asked);
                let reply = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        port
    }

    fn script_in(test: &str, file: &str, body: &str) -> (PathBuf, PathBuf) {
        let folder = std::env::temp_dir().join(format!("loupe-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join(file);
        fs::write(&path, body).unwrap();
        (folder, path)
    }

    fn wait_for_answer(app: &App) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.residents[0].inbox.lock().unwrap().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_repeating_script_fetches_hears_the_answer_next_run_and_stops_when_picked_again() {
        let port = answer_once("pong");
        let body = format!(
            "-- every: 0.25\nfor _, r in ipairs(loupe.responses()) do loupe.keep(\"heard\", r.status .. \" \" .. r.body) end\nif not loupe.kept(\"asked\") then loupe.fetch(\"http://127.0.0.1:{port}/\") loupe.keep(\"asked\", \"yes\") end\n"
        );
        let (folder, path) = script_in("repeat", "poll.lua", &body);
        let mut app = App::bare();
        let _ = app.run_script(&path);
        assert_eq!(app.residents.len(), 1, "the script did not keep running");
        wait_for_answer(&app);
        let deadline = Instant::now() + Duration::from_secs(2);
        while app.residents[0].machine.kept("heard").is_none() && Instant::now() < deadline {
            let _ = app.run_due_scripts();
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(app.residents[0].machine.kept("heard").as_deref(), Some("200 pong"));
        let _ = app.run_script(&path);
        assert!(app.residents.is_empty(), "picking the script again did not stop it");
        fs::remove_dir_all(folder).ok();
    }

    #[test]
    fn a_fetch_handler_is_called_with_the_answer_and_then_the_script_lets_go() {
        let port = answer_once("hello");
        let body = format!("loupe.fetch(\"http://127.0.0.1:{port}/\", function(r) print(r.status, r.body) end)\n");
        let (folder, path) = script_in("answer", "ask.lua", &body);
        let mut app = App::bare();
        let _ = app.run_script(&path);
        assert_eq!(app.residents.len(), 1, "the script let go before its answer came");
        wait_for_answer(&app);
        let _ = app.run_due_scripts();
        assert_eq!(app.notice.as_deref(), Some("200 hello"));
        assert!(app.residents.is_empty(), "a script with nothing left to wait for kept running");
        fs::remove_dir_all(folder).ok();
    }

    const QUIETER: &str = "-- autostart: yes\nlocal cap = 20 * math.log(0.8, 10)\nloupe.on(\"record_start\", function(e)\n  if loupe.master_volume() > cap then loupe.set_listen_volume(cap - loupe.master_volume()) end\nend)\nloupe.on(\"record_stop\", function(e) loupe.set_listen_volume(0) end)\n";

    #[test]
    fn recording_turns_the_listen_level_down_at_once_and_back_up_after() {
        let (folder, path) = script_in("quieter", "quieter.lua", QUIETER);
        let mut app = App::bare();
        let _ = app.run_script(&path);
        assert_eq!(app.residents.len(), 1);
        assert!(app.notice.as_deref().unwrap_or_default().contains("record_start, record_stop"), "{:?}", app.notice);
        app.happened(Event::RecordStart { tracks: Vec::new(), retake: false });
        let _ = app.deliver_script_events();
        assert!((app.listen_level - 0.8).abs() < 1e-4, "a full master was heard at {}", app.listen_level);
        assert_eq!(app.project.master, 1.0, "the song's master moved");
        assert!(!app.dirty, "turning the listen level down marked the song changed");
        app.happened(Event::RecordStop);
        let _ = app.deliver_script_events();
        assert_eq!(app.listen_level, 1.0);
        app.project.master = 0.5;
        app.happened(Event::RecordStart { tracks: Vec::new(), retake: false });
        let _ = app.deliver_script_events();
        assert_eq!(app.listen_level, 1.0, "a master already under the cap was turned down");
        fs::remove_dir_all(folder).ok();
    }

    const CAP_30: &str = "-- autostart: yes\nlocal cap = 20 * math.log(0.3, 10)\nlocal timer\n\nlocal function hold()\n  local over = loupe.master_volume() - cap\n  if over > 0 then\n    loupe.set_listen_volume(-over)\n  else\n    loupe.set_listen_volume(0)\n  end\nend\n\nloupe.on(\"record_start\", function(e)\n  hold()\n  if not timer then\n    timer = loupe.every(0.1, hold)\n  end\nend)\n\nloupe.on(\"record_stop\", function(e)\n  if timer then\n    loupe.off(timer)\n    timer = nil\n  end\n  loupe.set_listen_volume(0)\nend)\n";

    #[test]
    fn the_cap_script_holds_what_you_hear_at_30_percent_through_a_take() {
        let (folder, path) = script_in("cap-30", "cap.lua", CAP_30);
        let mut app = App::bare();
        let _ = app.run_script(&path);
        app.happened(Event::RecordStart { tracks: Vec::new(), retake: false });
        let _ = app.deliver_script_events();
        assert!((app.listen_level - 0.3).abs() < 1e-4, "a full master was heard at {}", app.listen_level);
        assert_eq!(app.project.master, 1.0, "the song's master moved");
        app.project.master = 0.6;
        std::thread::sleep(Duration::from_millis(150));
        let _ = app.run_due_scripts();
        assert!((app.project.master * app.listen_level - 0.3).abs() < 1e-4, "moving the master mid take was heard at {}", app.project.master * app.listen_level);
        app.project.master = 0.2;
        std::thread::sleep(Duration::from_millis(150));
        let _ = app.run_due_scripts();
        assert_eq!(app.listen_level, 1.0, "a master under the cap was turned down");
        app.project.master = 1.0;
        app.happened(Event::RecordStop);
        let _ = app.deliver_script_events();
        assert_eq!(app.listen_level, 1.0, "stopping left the listen level down");
        std::thread::sleep(Duration::from_millis(150));
        let _ = app.run_due_scripts();
        assert_eq!(app.listen_level, 1.0, "the timer kept running after the take");
        fs::remove_dir_all(folder).ok();
    }

    #[test]
    fn pressing_record_turns_it_down_and_a_next_take_says_so() {
        let (folder, path) = script_in("record-button", "quieter.lua", &format!("{QUIETER}loupe.on(\"record_start\", function(e) print(\"tracks\", #e.tracks, \"retake\", e.retake) end)\n"));
        let mut app = App::bare();
        let Ok(Outcome::Track(vox)) = app.project.apply(Command::AddTrack { name: "Vox".into() }) else { panic!() };
        app.path = Some(folder.join("Song.lp"));
        app.input = Some(loupe_engine::Input::open(loupe_engine::InputChoice::Practice).unwrap());
        app.armed.insert(vox);
        let _ = app.run_script(&path);
        let _ = app.update(Message::ToggleRecord);
        assert!(app.recording.is_some(), "recording did not start");
        assert!((app.listen_level - 0.8).abs() < 1e-4, "pressing record left the listen level at {}", app.listen_level);
        assert_eq!(app.notice.as_deref(), Some("tracks 1 retake false"));
        let _ = app.next_take();
        let _ = app.deliver_script_events();
        assert_eq!(app.notice.as_deref(), Some("tracks 1 retake true"));
        assert!((app.listen_level - 0.8).abs() < 1e-4, "a next take left the listen level at {}", app.listen_level);
        let _ = app.update(Message::ToggleRecord);
        assert!(app.recording.is_none());
        assert_eq!(app.listen_level, 1.0, "stopping left the listen level down");
        fs::remove_dir_all(folder).ok();
    }

    #[test]
    fn stopping_a_script_mid_take_gives_the_listen_level_back() {
        let (folder, path) = script_in("give-back", "quieter.lua", QUIETER);
        let mut app = App::bare();
        let _ = app.run_script(&path);
        app.happened(Event::RecordStart { tracks: Vec::new(), retake: false });
        let _ = app.deliver_script_events();
        assert!(app.listen_level < 1.0);
        let _ = app.run_script(&path);
        assert!(app.residents.is_empty());
        assert_eq!(app.listen_level, 1.0, "the listen level stayed down after its script stopped");
        fs::remove_dir_all(folder).ok();
    }

    #[test]
    fn play_and_arm_reach_handlers_through_the_real_controls() {
        let (folder, path) = script_in("controls", "watch.lua", "loupe.on(\"play\", function(e) print(\"heard\", e.name) end)\nloupe.on(\"arm\", function(e) print(\"armed\", e.track, e.armed) end)\n");
        let mut app = App::bare();
        app.project.apply(Command::AddTrack { name: "Vox".into() }).unwrap();
        let vox = app.project.tracks[0].id;
        let _ = app.run_script(&path);
        let _ = app.update(Message::TogglePlay);
        assert_eq!(app.notice.as_deref(), Some("heard play"));
        let _ = app.update(Message::ToggleArm(vox));
        assert_eq!(app.notice.as_deref(), Some(format!("armed {} true", vox.0).as_str()));
        fs::remove_dir_all(folder).ok();
    }

    #[test]
    fn a_handler_that_fails_stops_its_script_and_says_where() {
        let (folder, path) = script_in("fails", "broken.lua", "loupe.on(\"play\", function(e) error(\"no mic\") end)\n");
        let mut app = App::bare();
        let _ = app.run_script(&path);
        let _ = app.update(Message::TogglePlay);
        assert!(app.residents.is_empty());
        let problem = app.problem.clone().unwrap_or_default();
        assert!(problem.contains("broken") && problem.contains("on play") && problem.contains("no mic"), "{problem}");
        fs::remove_dir_all(folder).ok();
    }

    #[test]
    fn scripts_that_set_each_other_off_are_stopped_after_a_few_rounds() {
        let (folder, path) = script_in("ping-pong", "loop.lua", "loupe.on(\"arm\", function(e) loupe.set_track_armed(e.track, not e.armed) end)\n");
        let mut app = App::bare();
        app.project.apply(Command::AddTrack { name: "Vox".into() }).unwrap();
        let vox = app.project.tracks[0].id;
        let _ = app.run_script(&path);
        let _ = app.update(Message::ToggleArm(vox));
        assert!(app.problem.as_deref().unwrap_or_default().contains("kept setting each other off"), "{:?}", app.problem);
        fs::remove_dir_all(folder).ok();
    }

    #[test]
    fn saving_a_running_script_reloads_it() {
        let (folder, path) = script_in("reload", "live.lua", "loupe.on(\"play\", function(e) print(\"one\") end)\n");
        let mut app = App::bare();
        let _ = app.run_script(&path);
        fs::write(&path, "loupe.on(\"play\", function(e) print(\"two\") end)\n").unwrap();
        let later = SystemTime::now() + Duration::from_secs(5);
        fs::File::options().write(true).open(&path).unwrap().set_modified(later).unwrap();
        let _ = app.run_due_scripts();
        assert_eq!(app.residents.len(), 1);
        assert!(app.notice.as_deref().unwrap_or_default().contains("Reloaded"), "{:?}", app.notice);
        let _ = app.update(Message::TogglePlay);
        assert_eq!(app.notice.as_deref(), Some("two"));
        fs::remove_dir_all(folder).ok();
    }

    #[test]
    fn timers_fire_once_or_again_and_can_be_stopped() {
        let (folder, path) = script_in("timers", "timers.lua", "local n = 0\nlocal id\nid = loupe.every(0.01, function() n = n + 1 if n == 2 then loupe.off(id) print(\"twice\") end end)\nloupe.after(0.01, function() loupe.keep(\"after\", \"done\") end)\n");
        let mut app = App::bare();
        let _ = app.run_script(&path);
        assert_eq!(app.script_tick(), Some(Duration::from_millis(SHORTEST_TICK_MS)));
        for _ in 0..2 {
            std::thread::sleep(Duration::from_millis(20));
            let _ = app.run_due_scripts();
        }
        assert_eq!(app.notice.as_deref(), Some("twice"));
        assert!(app.residents.is_empty(), "a script whose timers are all done kept running");
        fs::remove_dir_all(folder).ok();
    }

    #[test]
    fn unknown_events_and_loud_listen_levels_are_refused() {
        let mut project = loupe_engine::Project::new(48_000);
        let refused = scripting::run("loupe.on(\"recordstart\", function() end)", "t", &mut project, View::default(), None).unwrap_err();
        assert!(refused.contains("record_start"), "{refused}");
        let louder = scripting::run("loupe.set_listen_volume(3)", "t", &mut project, View::default(), None).unwrap_err();
        assert!(louder.contains("only turns down"), "{louder}");
    }

    #[test]
    fn the_examples_run_cleanly() {
        let mut project = loupe_engine::Project::new(48_000);
        project.apply(loupe_engine::Command::AddTrack { name: "Drums".into() }).unwrap();
        for (name, body) in EXAMPLES {
            scripting::run(body, name, &mut project, View::default(), None).unwrap_or_else(|why| panic!("{name}: {why}"));
        }
        assert_eq!(project.tracks[0].name, "01 Drums");
        scripting::run(EXAMPLES[0].1, "again", &mut project, View::default(), None).unwrap();
        assert_eq!(project.tracks[0].name, "01 Drums");
    }
}
