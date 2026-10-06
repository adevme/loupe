mod blame;
mod channel;
mod window;

use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};

use loupe_plugins::blend::Dry;
use loupe_plugins::wire::{next_line, read_block, write_block, Ask, Link, Region, Reply};
use loupe_plugins::{clap, lv2, vst3};

#[cfg(windows)]
mod com {
    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(reserved: *mut core::ffi::c_void, model: u32) -> i32;
        fn CoUninitialize();
        fn OleInitialize(reserved: *mut core::ffi::c_void) -> i32;
    }

    pub fn start() {
        let model = match std::env::var("LOUPE_COM").as_deref() {
            Ok("mta") => 0x0,
            _ => 0x2,
        };
        unsafe {
            CoInitializeEx(std::ptr::null_mut(), model);
            OleInitialize(std::ptr::null_mut());
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

    fn settings(&mut self) -> Result<Vec<u8>, String> {
        match self {
            Open::Vst3(effect) => effect.settings(),
            other => other.save(),
        }
    }

    fn save(&mut self) -> Result<Vec<u8>, String> {
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

    fn readings(&mut self) -> Vec<loupe_plugins::wire::Reading> {
        match self {
            Open::Vst3(effect) => effect.readings(),
            Open::Clap(effect) => effect.readings(),
            Open::Lv2(_) => Vec::new(),
            #[cfg(target_os = "macos")]
            Open::Au(_) => Vec::new(),
        }
    }

    fn from_text(&mut self, knob: usize, text: &str) -> Option<f32> {
        match self {
            Open::Vst3(effect) => effect.from_text(knob, text),
            Open::Clap(effect) => effect.from_text(knob, text),
            Open::Lv2(_) => None,
            #[cfg(target_os = "macos")]
            Open::Au(_) => None,
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

struct Seated {
    open: Open,
    name: String,
    shown_as: String,
    latency: usize,
    dry: Dry,
}

struct Seats {
    taken: Vec<Option<Seated>>,
    at: usize,
}

impl Seats {
    fn empty() -> Self {
        Self { taken: Vec::new(), at: 0 }
    }

    fn here(&mut self) -> Option<&mut Seated> {
        self.taken.get_mut(self.at)?.as_mut()
    }

    fn at(&mut self, seat: usize) -> Option<&mut Seated> {
        self.taken.get_mut(seat)?.as_mut()
    }

    fn move_to(&mut self, seat: usize) {
        if self.taken.len() <= seat {
            self.taken.resize_with(seat + 1, || None);
        }
        self.at = seat;
    }

    fn put_here(&mut self, seated: Seated) {
        self.move_to(self.at);
        self.taken[self.at] = Some(seated);
    }

    fn clear(&mut self, seat: usize) {
        if let Some(room) = self.taken.get_mut(seat) {
            *room = None;
        }
    }

    fn anyone_is_ara(&self) -> bool {
        self.taken.iter().flatten().any(|seated| seated.open.is_ara())
    }

    fn all_idle(&self) {
        for seated in self.taken.iter().flatten() {
            seated.open.idle();
        }
    }

    fn run(&mut self, links: &[Link], audio: &mut [[f32; 2]], side: &[[f32; 2]]) {
        for link in links {
            let Some(seated) = self.at(link.seat) else { continue };
            let blending = link.mix < 1.0;
            if blending {
                seated.dry.room_for(seated.latency, audio.len());
                seated.dry.remember(audio);
            }
            blame::working_on(link.seat);
            seated.open.process_with(audio, side);
            blame::finished();
            if blending {
                seated.dry.blend(audio, link.mix);
            }
        }
    }
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
            let blocks = match &ask {
                Ask::Process => 1,
                Ask::ProcessWithSide => 2,
                Ask::ProcessAt(_) => 1,
                Ask::Chain { side, .. } => 1 + *side as usize,
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

    let (mut out, telling) = channel::take_stdout();
    blame::tell_on_a_crash(telling);
    let mut seats = Seats::empty();
    let mut editor: Option<(std::rc::Rc<loupe_plugins::editor::Editor>, window::Window, usize)> = None;
    let mut was_sized = (0, 0);
    let mut region: Option<Region> = None;
    let mut idled = std::time::Instant::now();
    'living: loop {
        if let Some((made, pane, seat)) = editor.as_ref() {
            let seat = *seat;
            for asked in pane.pump() {
                let name = seats.at(seat).map(|seated| seated.name.clone()).unwrap_or_default();
                use_preset(asked, seats.at(seat).map(|seated| &mut seated.open), pane, &name);
            }
            if let Some((width, height)) = made.wanted_size() {
                pane.fit_around(width, height);
                was_sized = (width, height);
            }
            let now = pane.inside();
            if now != was_sized && now.0 > 0 && now.1 > 0 {
                let settled = made.resized(now.0, now.1);
                was_sized = settled;
                if settled != now {
                    pane.fit_around(settled.0, settled.1);
                }
            }
        }
        if idled.elapsed() >= std::time::Duration::from_millis(30) {
            idled = std::time::Instant::now();
            seats.all_idle();
        }
        let ticking = editor.is_some() || seats.anyone_is_ara();
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
                let here = seats.at;
                if let Some(seated) = seats.here() {
                    blame::working_on(here);
                    seated.open.process_at(&mut audio, &side, at);
                    blame::finished();
                }
                if write_block(&mut out, &audio).is_err() {
                    break;
                }
                continue;
            }
            Ask::Chain { side: wants_side, links } => {
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
                seats.run(&links, &mut audio, &side);
                if write_block(&mut out, &audio).is_err() {
                    break;
                }
                continue;
            }
            Ask::Seat(seat) => {
                seats.move_to(seat);
                Reply::Fine
            }
            Ask::Unload(seat) => {
                if editor.as_ref().is_some_and(|(_, _, held)| *held == seat) {
                    editor = None;
                }
                seats.clear(seat);
                Reply::Fine
            }
            Ask::Quit => break,
            Ask::Region(wanted) => match seats.here().map(|seated| &mut seated.open).filter(|effect| effect.is_ara()) {
                Some(effect) => match effect.place(&wanted) {
                    Ok(()) => Reply::Fine,
                    Err(why) => Reply::Trouble(why),
                },
                None => {
                    region = Some(wanted);
                    Reply::Fine
                }
            },
            Ask::Show => {
                let here = seats.at;
                let name = seats.here().map(|seated| match seated.shown_as.is_empty() {
                    true => seated.name.clone(),
                    false => seated.shown_as.clone(),
                }).unwrap_or_default();
                match show(seats.here().map(|seated| &mut seated.open), &mut editor, &name, here) {
                    Ok(()) => Reply::Fine,
                    Err(why) => Reply::Trouble(why),
                }
            }
            Ask::Called(called) => {
                if let Some(seated) = seats.here() {
                    seated.shown_as = called;
                }
                Reply::Fine
            }
            Ask::Hide => {
                if let Some((_, pane, _)) = editor.as_ref() {
                    pane.hide();
                }
                Reply::Fine
            }
            Ask::Classes(path) => match names_in(&PathBuf::from(&path)) {
                Ok(names) => Reply::Classes(names),
                Err(why) => Reply::Trouble(why),
            },
            Ask::Load { path, index, rate, block } => {
                let here = seats.at;
                if editor.as_ref().is_some_and(|(_, _, held)| *held == here) {
                    editor = None;
                }
                let name = names_in(&PathBuf::from(&path)).ok().and_then(|names| names.get(index).cloned()).unwrap_or_default();
                blame::working_on(here);
                let made = open_one(&PathBuf::from(&path), index, rate as f64, block, region.as_ref());
                blame::finished();
                match made {
                    Ok(effect) => {
                        let latency = effect.latency();
                        let ara = effect.is_ara();
                        seats.put_here(Seated { open: effect, name, shown_as: String::new(), latency, dry: Dry::empty() });
                        Reply::Loaded { inputs: 2, outputs: 2, latency, ara }
                    }
                    Err(why) => Reply::Trouble(why),
                }
            }
            Ask::Knobs => match seats.here() {
                Some(seated) => Reply::Knobs(seated.open.knobs()),
                None => Reply::Trouble("no plugin is open".into()),
            },
            Ask::Readings => match seats.here() {
                Some(seated) => Reply::Readings(seated.open.readings()),
                None => Reply::Trouble("no plugin is open".into()),
            },
            Ask::FromText { knob, text } => match seats.here().map(|seated| seated.open.from_text(knob, &text)) {
                Some(Some(value)) => Reply::Value(value),
                Some(None) => Reply::Trouble(format!("the plugin did not understand \"{text}\"")),
                None => Reply::Trouble("no plugin is open".into()),
            },
            Ask::Turn { knob, value } => {
                if let Some(seated) = seats.here() {
                    seated.open.turn(knob, value);
                }
                Reply::Fine
            }
            Ask::Save => match seats.here() {
                Some(seated) => match seated.open.save() {
                    Ok(state) => Reply::State(state),
                    Err(why) => Reply::Trouble(why),
                },
                None => Reply::Trouble("no plugin is open".into()),
            },
            Ask::Restore(state) => {
                let here = seats.at;
                match seats.here() {
                    Some(seated) => {
                        blame::working_on(here);
                        let put = seated.open.restore(&state);
                        blame::finished();
                        match put {
                            Ok(()) => Reply::Fine,
                            Err(why) => Reply::Trouble(why),
                        }
                    }
                    None => Reply::Trouble("no plugin is open".into()),
                }
            }
        };
        let _ = reply.write(&mut out);
    }
    drop(editor);
    drop(seats);
    let _ = out.flush();
    com::stop();
}

fn show(
    open: Option<&mut Open>,
    editor: &mut Option<(std::rc::Rc<loupe_plugins::editor::Editor>, window::Window, usize)>,
    name: &str,
    seat: usize,
) -> Result<(), String> {
    if let Some((_, pane, held)) = editor.as_ref() {
        if *held == seat {
            pane.show();
            return Ok(());
        }
        return Err("this plugin host already has a window open".into());
    }
    let Some(Open::Vst3(effect)) = open else {
        return Err("only VST3 plugins have a window so far".into());
    };
    let made = std::rc::Rc::new(effect.editor()?);
    let kind = loupe_plugins::editor::platform_kind();
    if !made.fits(kind) {
        return Err("this plugin has no window for this computer".into());
    }
    let (width, height) = made.size();
    let title = if name.is_empty() { "Plugin" } else { name };

    let pane = window::Window::open(title, width, height, made.can_resize())?;
    if let Some(folder) = preset_folder(name) {
        pane.presets(&loupe_plugins::presets::list(&folder), None);
    }
    unsafe { made.attach(pane.inner(), kind) }?;
    let scale = pane.screen_scale();
    let asked = made.size();
    let grew_itself = asked.0 as f32 >= width as f32 * scale * ALREADY_SCALED;
    let settled = match scale <= 1.01 || grew_itself {
        true => asked,
        false => ((asked.0 as f32 * scale).round() as i32, (asked.1 as f32 * scale).round() as i32),
    };
    if settled != (width, height) && settled.0 > 0 && settled.1 > 0 {
        pane.fit_around(settled.0, settled.1);
    }
    let following = std::rc::Rc::clone(&made);
    pane.follow(Box::new(move |width, height| {
        following.resized(width, height);
    }));
    pane.show();
    *editor = Some((made, pane, seat));
    Ok(())
}


const ALREADY_SCALED: f32 = 0.9;

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
        window::Asked::Delete(name) => {
            if loupe_plugins::presets::remove(&folder, &name).is_ok() {
                pane.presets(&loupe_plugins::presets::list(&folder), None);
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
