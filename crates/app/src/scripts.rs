use std::fs;
use std::path::{Path, PathBuf};

use iced::{keyboard, Element, Task};
use loupe_engine::{CommandError, Outcome};

use crate::scripting::{self, View, FUNCTIONS};
use crate::{settings, App, Message};

const FOLDER: &str = "Scripts";
const EXTENSION: &str = "lua";
const REFERENCE: &str = "Loupe script functions.txt";
const MOST_PRINTED: usize = 3;
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
        let mut items: Vec<Element<'_, Message>> = self
            .scripts
            .iter()
            .map(|script| self.item(&script.name, script.shortcut.as_deref().unwrap_or(""), Some(Message::RunScript(script.path.clone()))))
            .collect();
        if items.is_empty() {
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
        let mut plugins: Vec<(String, PathBuf, usize)> = self.found.iter().map(|found| (found.name.clone(), found.path.clone(), found.index)).collect();
        for (index, name) in loupe_stock::NAMES.iter().enumerate() {
            if !plugins.iter().any(|(known, _, _)| known == name) {
                plugins.push((name.to_string(), PathBuf::from(loupe_plugins::BUILT_IN), index));
            }
        }
        let folder = self.path.as_deref().and_then(Path::parent).map(Path::to_path_buf);
        let view = View { playhead: self.playhead, playing: self.playing, selected: self.selection.iter().copied().collect(), plugins, folder };
        let mut finished = None;
        self.transact(None, |project| {
            let ran = scripting::run(&source, &name, project, &view);
            let keep = matches!(&ran, Ok(done) if done.changed);
            finished = Some(ran);
            if keep {
                Ok(Outcome::Done)
            } else {
                Err(CommandError::InvalidValue)
            }
        });
        self.forget_gone_clips();
        let ran = match finished {
            Some(Ok(ran)) => ran,
            Some(Err(why)) => {
                self.problem = Some(format!("Script {name}: {why}"));
                return Task::none();
            }
            None => return Task::none(),
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
        self.notice = Some(match printed.is_empty() {
            true => format!("{name} finished in {} ms.", ran.took.as_millis()),
            false => printed,
        });
        let playing = match wishes.play {
            Some(play) if play != self.playing => self.handle(Message::TogglePlay),
            _ => Task::none(),
        };
        let exporting = match wishes.export {
            Some(stems) => {
                self.export.split = stems;
                self.start_export()
            }
            None => Task::none(),
        };
        Task::batch([playing, exporting])
    }
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
    fn the_examples_run_cleanly() {
        let mut project = loupe_engine::Project::new(48_000);
        project.apply(loupe_engine::Command::AddTrack { name: "Drums".into() }).unwrap();
        let view = View { playhead: 0, playing: false, selected: Vec::new(), plugins: Vec::new(), folder: None };
        for (name, body) in EXAMPLES {
            scripting::run(body, name, &mut project, &view).unwrap_or_else(|why| panic!("{name}: {why}"));
        }
        assert_eq!(project.tracks[0].name, "01 Drums");
        scripting::run(EXAMPLES[0].1, "again", &mut project, &view).unwrap();
        assert_eq!(project.tracks[0].name, "01 Drums");
    }
}
