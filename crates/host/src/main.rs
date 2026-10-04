mod channel;
mod window;

use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};

use loupe_plugins::wire::{next_line, read_block, write_block, Ask, Region, Reply};
use loupe_plugins::{clap, lv2, vst3};

#[cfg(windows)]
mod com {
    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(reserved: *mut core::ffi::c_void, model: u32) -> i32;
        fn CoUninitialize();
    }

    pub fn start() {
        let model = match std::env::var("LOUPE_COM").as_deref() {
            Ok("mta") => 0x0,
            _ => 0x2,
        };
        unsafe {
            CoInitializeEx(std::ptr::null_mut(), model);
        }
    }

    pub fn stop() {
        unsafe {
            CoUninitialize();
        }
    }
}

#[cfg(not(windows))]
mod com {
    pub fn start() {}
    pub fn stop() {}
}

enum Open {
    Vst3(vst3::Effect),
    #[cfg(target_os = "macos")]
    Au(loupe_plugins::au::Effect),
    Clap(clap::Effect),
    Lv2(lv2::Effect),
}

impl Open {
    fn process_with(&mut self, audio: &mut [[f32; 2]], side: &[[f32; 2]]) {
        match self {
            Open::Vst3(effect) => effect.process_with(audio, side),
            Open::Clap(effect) => effect.process_with(audio, side),
            Open::Lv2(effect) => effect.process_with(audio, side),
            #[cfg(target_os = "macos")]
            Open::Au(effect) => effect.process(audio),
        }
    }

    fn process_at(&mut self, audio: &mut [[f32; 2]], side: &[[f32; 2]], at: Option<i64>) {
        match (self, at) {
            (Open::Vst3(effect), Some(at)) => effect.process_at(audio, side, at),
            (open, _) => open.process_with(audio, side),
        }
    }

    fn is_ara(&self) -> bool {
        matches!(self, Open::Vst3(effect) if effect.is_ara())
    }

    fn idle(&self) {
        if let Open::Vst3(effect) = self {
            effect.idle();
        }
    }

    fn place(&mut self, region: &Region) -> Result<(), String> {
        match self {
            Open::Vst3(effect) => effect.place(region),
            _ => Err("only VST3 plugins can follow a clip with ARA".into()),
        }
    }

    fn settings(&self) -> Result<Vec<u8>, String> {
        match self {
            Open::Vst3(effect) => effect.settings(),
            other => other.save(),
        }
    }

    fn save(&self) -> Result<Vec<u8>, String> {
        match self {
            Open::Vst3(effect) => effect.save(),
            Open::Clap(effect) => effect.save(),
            Open::Lv2(effect) => effect.save(),
            #[cfg(target_os = "macos")]
            Open::Au(effect) => effect.save(),
        }
    }

    fn knobs(&mut self) -> Vec<String> {
        match self {
            Open::Vst3(effect) => effect.knobs(),
            Open::Clap(effect) => effect.knobs(),
            Open::Lv2(_) => Vec::new(),
            #[cfg(target_os = "macos")]
            Open::Au(_) => Vec::new(),
        }
    }

    fn turn(&mut self, knob: usize, value: f32) {
        match self {
            Open::Clap(effect) => effect.turn(knob, value),
            Open::Vst3(effect) => effect.turn(knob, value),
            Open::Lv2(effect) => effect.turn(knob, value),
            #[cfg(target_os = "macos")]
            Open::Au(_) => {}
        }
    }

    fn latency(&self) -> usize {
        match self {
            Open::Vst3(effect) => effect.latency(),
            Open::Clap(effect) => effect.latency(),
            Open::Lv2(effect) => effect.latency(),
            #[cfg(target_os = "macos")]
            Open::Au(_) => 0,
        }
    }

    fn restore(&mut self, state: &[u8]) -> Result<(), String> {
        match self {
            Open::Vst3(effect) => effect.restore(state),
            Open::Clap(effect) => effect.restore(state),
            Open::Lv2(effect) => effect.restore(state),
            #[cfg(target_os = "macos")]
            Open::Au(effect) => effect.restore(state),
        }
    }
}

fn kind_of(path: &Path) -> &'static str {
    match path.extension().and_then(|end| end.to_str()).unwrap_or("").to_lowercase().as_str() {
        "clap" => "clap",
        "lv2" => "lv2",
        "component" => "au",
        _ => "vst3",
    }
}

fn names_in(path: &Path) -> Result<Vec<String>, String> {
    match kind_of(path) {
        "clap" => clap::Library::open(path).map(|library| library.classes().into_iter().map(|class| class.name).collect()),
        "lv2" => lv2::Library::open(path).map(|library| library.classes().into_iter().map(|class| class.name).collect()),
        "au" => au_names(),
        _ => vst3::Library::open(path).map(|library| library.classes().into_iter().map(|class| class.name).collect()),
    }
}

fn open_one(path: &Path, index: usize, rate: f64, block: usize, region: Option<&Region>) -> Result<Open, String> {
    match kind_of(path) {
        "clap" => clap::Library::open(path).and_then(|library| clap::Effect::start(library, index, rate, block)).map(Open::Clap),
        "lv2" => lv2::Library::open(path).and_then(|library| lv2::Effect::start(library, index, rate, block)).map(Open::Lv2),
        "au" => au_open(path, index, rate, block),
        _ => vst3::Library::open(path).and_then(|library| vst3::Effect::start_on(library, index, rate, block, region)).map(Open::Vst3),
    }
}

enum Came {
    Ask(Ask),
    Audio(Vec<[f32; 2]>),
    Mumble(String),
    Done,
}

fn main() {
    com::start();
    let (sends, came) = std::sync::mpsc::channel::<Came>();
    std::thread::spawn(move || {
        let mut input = BufReader::new(std::io::stdin());
        while let Some(line) = next_line(&mut input) {
            let Some(ask) = Ask::read(&line) else {
                if sends.send(Came::Mumble(line.trim().to_string())).is_err() {
                    return;
                }
                continue;
            };
            let blocks = match ask {
                Ask::Process => 1,
                Ask::ProcessWithSide => 2,
                Ask::ProcessAt(_) => 1,
                _ => 0,
            };
            if sends.send(Came::Ask(ask)).is_err() {
                return;
            }
            for _ in 0..blocks {
                let mut audio = Vec::new();
                if read_block(&mut input, &mut audio).is_err() {
                    return;
                }
                if sends.send(Came::Audio(audio)).is_err() {
                    return;
                }
            }
        }
        let _ = sends.send(Came::Done);
    });

    let mut out = channel::take_stdout();
    let mut open: Option<Open> = None;
    let mut editor: Option<(loupe_plugins::editor::Editor, window::Window)> = None;
    let mut loaded_name = String::new();
    let mut was_sized = (0, 0);
    let mut region: Option<Region> = None;
    let mut idled = std::time::Instant::now();
    'living: loop {
        if let Some((made, pane)) = editor.as_ref() {
            for asked in pane.pump() {
                use_preset(asked, open.as_mut(), pane, &loaded_name);
            }
            // The plugin is not told when the frame is dragged, so watch the size and
            // hand it on. Without this it keeps drawing at its old size in a bigger hole.
            let now = pane.inside();
            if now != was_sized && now.0 > 0 && now.1 > 0 {
                was_sized = now;
                made.resized(now.0, now.1);
            }
        }
        if idled.elapsed() >= std::time::Duration::from_millis(30) {
            idled = std::time::Instant::now();
            if let Some(effect) = open.as_ref() {
                effect.idle();
            }
        }
        let ticking = editor.is_some() || open.as_ref().is_some_and(Open::is_ara);
        let next = if ticking {
            match came.recv_timeout(std::time::Duration::from_millis(8)) {
                Ok(next) => next,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => break,
            }
        } else {
            match came.recv() {
                Ok(next) => next,
                Err(_) => break,
            }
        };
        let ask = match next {
            Came::Ask(ask) => ask,
            Came::Audio(_) => continue,
            Came::Mumble(line) => {
                let _ = Reply::Trouble(format!("I did not understand {line}")).write(&mut out);
                continue;
            }
            Came::Done => break,
        };
        let reply = match ask {
            Ask::Process | Ask::ProcessWithSide | Ask::ProcessAt(_) => {
                let wants_side = ask == Ask::ProcessWithSide;
                let at = match ask {
                    Ask::ProcessAt(at) => Some(at),
                    _ => None,
                };
                let mut audio = match came.recv() {
                    Ok(Came::Audio(audio)) => audio,
                    _ => break 'living,
                };
                let side = if wants_side {
                    match came.recv() {
                        Ok(Came::Audio(side)) => side,
                        _ => break 'living,
                    }
                } else {
                    Vec::new()
                };
                if let Some(effect) = open.as_mut() {
                    effect.process_at(&mut audio, &side, at);
                }
                if write_block(&mut out, &audio).is_err() {
                    break;
                }
                continue;
            }
            Ask::Quit => break,
            Ask::Region(wanted) => match open.as_mut().filter(|effect| effect.is_ara()) {
                Some(effect) => match effect.place(&wanted) {
                    Ok(()) => Reply::Fine,
                    Err(why) => Reply::Trouble(why),
                },
                None => {
                    region = Some(wanted);
                    Reply::Fine
                }
            },
            Ask::Show => match show(open.as_mut(), &mut editor, &loaded_name) {
                Ok(()) => Reply::Fine,
                Err(why) => Reply::Trouble(why),
            },
            Ask::Hide => {
                if let Some((_, pane)) = editor.as_ref() {
                    pane.hide();
                }
                Reply::Fine
            }
            Ask::Classes(path) => match names_in(&PathBuf::from(&path)) {
                Ok(names) => Reply::Classes(names),
                Err(why) => Reply::Trouble(why),
            },
            Ask::Load { path, index, rate, block } => {
                editor = None;
                // Kept for the window title, so a plugin window says which plugin it is.
                loaded_name = names_in(&PathBuf::from(&path)).ok().and_then(|names| names.get(index).cloned()).unwrap_or_default();
                match open_one(&PathBuf::from(&path), index, rate as f64, block, region.as_ref()) {
                    Ok(effect) => {
                        let latency = effect.latency();
                        let ara = effect.is_ara();
                        open = Some(effect);
                        Reply::Loaded { inputs: 2, outputs: 2, latency, ara }
                    }
                    Err(why) => Reply::Trouble(why),
                }
            }
            Ask::Knobs => match open.as_mut() {
                Some(effect) => Reply::Knobs(effect.knobs()),
                None => Reply::Trouble("no plugin is open".into()),
            },
            Ask::Turn { knob, value } => {
                if let Some(effect) = open.as_mut() {
                    effect.turn(knob, value);
                }
                Reply::Fine
            }
            Ask::Save => match open.as_ref() {
                Some(effect) => match effect.save() {
                    Ok(state) => Reply::State(state),
                    Err(why) => Reply::Trouble(why),
                },
                None => Reply::Trouble("no plugin is open".into()),
            },
            Ask::Restore(state) => match open.as_mut() {
                Some(effect) => match effect.restore(&state) {
                    Ok(()) => Reply::Fine,
                    Err(why) => Reply::Trouble(why),
                },
                None => Reply::Trouble("no plugin is open".into()),
            },
        };
        let _ = reply.write(&mut out);
    }
    drop(editor);
    drop(open);
    let _ = out.flush();
    com::stop();
}

fn show(
    open: Option<&mut Open>,
    editor: &mut Option<(loupe_plugins::editor::Editor, window::Window)>,
    name: &str,
) -> Result<(), String> {
    if let Some((_, pane)) = editor.as_ref() {
        pane.show();
        return Ok(());
    }
    let Some(Open::Vst3(effect)) = open else {
        return Err("only VST3 plugins have a window so far".into());
    };
    let made = effect.editor()?;
    let kind = loupe_plugins::editor::platform_kind();
    if !made.fits(kind) {
        return Err("this plugin has no window for this computer".into());
    }
    let (width, height) = made.size();
    let title = if name.is_empty() { "Plugin" } else { name };
    // A fresh window starts at the plugin's own size, so forget the last one's.

    let pane = window::Window::open(title, width, height, made.can_resize())?;
    if let Some(folder) = preset_folder(name) {
        pane.presets(&loupe_plugins::presets::list(&folder), None);
    }
    // The window is this process's own and is kept alongside the editor below.
    unsafe { made.attach(pane.inner(), kind) }?;
    pane.show();
    *editor = Some((made, pane));
    Ok(())
}


fn preset_folder(plugin: &str) -> Option<PathBuf> {
    let root = std::env::var_os(loupe_plugins::presets::FOLDER_VARIABLE)?;
    Some(loupe_plugins::presets::folder_for(Path::new(&root), plugin))
}

fn use_preset(asked: window::Asked, open: Option<&mut Open>, pane: &window::Window, plugin: &str) {
    let (Some(effect), Some(folder)) = (open, preset_folder(plugin)) else { return };
    match asked {
        window::Asked::Load(name) => {
            if let Ok(state) = loupe_plugins::presets::load(&folder, &name) {
                let _ = effect.restore(&state);
            }
        }
        window::Asked::Save(name) => {
            let saved = effect.settings().and_then(|state| loupe_plugins::presets::save(&folder, &name, &state));
            if let Ok(name) = saved {
                pane.presets(&loupe_plugins::presets::list(&folder), Some(&name));
                pane.clear_name();
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn au_names() -> Result<Vec<String>, String> {
    loupe_plugins::au::Library::open(Path::new(""))
        .map(|library| library.classes().into_iter().map(|class| class.name).collect())
}

#[cfg(not(target_os = "macos"))]
fn au_names() -> Result<Vec<String>, String> {
    Err("Audio Units only run on a Mac".into())
}

#[cfg(target_os = "macos")]
fn au_open(_path: &Path, index: usize, rate: f64, block: usize) -> Result<Open, String> {
    loupe_plugins::au::Library::open(Path::new(""))
        .and_then(|library| loupe_plugins::au::Effect::start(library, index, rate, block))
        .map(Open::Au)
}

#[cfg(not(target_os = "macos"))]
fn au_open(_path: &Path, _index: usize, _rate: f64, _block: usize) -> Result<Open, String> {
    Err("Audio Units only run on a Mac".into())
}
