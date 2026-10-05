use std::fs::{self, OpenOptions};
use std::io::Write;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::settings::config_dir;
use crate::usage::system;
use crate::versions::THIS;

const CRASH_FILE: &str = "crash.log";
const REPORTS_FOLDER: &str = "crash-reports";
const REPORT_EXTENSION: &str = "report";
const SEND_TO: &str = "https://api.adev.me/loupe/crash";
const SEND_TO_VARIABLE: &str = "LOUPE_CRASH_TO";
const WAIT: Duration = Duration::from_secs(10);
const REPORTS_KEPT: usize = 5;
const FRAMES_KEPT: usize = 40;
const LONGEST_PLACE: usize = 300;
const LONGEST_MESSAGE: usize = 2000;
const LONGEST_TRACE: usize = 24 * 1024;
const LONGEST_NAME: usize = 128;
const LONGEST_NOTE: usize = 1000;
const MOST_PLUGINS: usize = 64;
const SHORTEST_NAME_HIDDEN: usize = 3;
const HIDDEN_PATH: &str = "<path>";
const HIDDEN_ADDRESS: &str = "<address>";
const HIDDEN_NAME: &str = "<name>";

static AROUND: Mutex<Around> = Mutex::new(Around { plugins: Vec::new(), driver: String::new(), output: String::new() });
static PANICKED: AtomicBool = AtomicBool::new(false);

struct Around {
    plugins: Vec<String>,
    driver: String,
    output: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Panic,
    Native,
    Plugin,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Kind::Panic => "panic",
            Kind::Native => "native",
            Kind::Plugin => "plugin",
        }
    }

    fn from_word(word: &str) -> Option<Self> {
        [Kind::Panic, Kind::Native, Kind::Plugin].into_iter().find(|kind| kind.word() == word)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub kind: Kind,
    pub version: String,
    pub os: String,
    pub place: String,
    pub message: String,
    pub backtrace: String,
    pub plugins: Vec<String>,
    pub plugin: String,
    pub driver: String,
    pub output: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Waiting {
    pub file: PathBuf,
    pub report: Report,
}

pub fn keep_a_record() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        PANICKED.store(true, Ordering::SeqCst);
        write_down(info);
        let place = info.location().map(|at| format!("{}:{}", at.file(), at.line())).unwrap_or_default();
        let trace = std::backtrace::Backtrace::force_capture().to_string();
        let _ = keep(&Report::panicked(&place, &said(info), &trace));
        crate::backup::last_chance();
        default_hook(info);
    }));
    #[cfg(windows)]
    native::watch();
}

fn said(info: &PanicHookInfo<'_>) -> String {
    info.payload()
        .downcast_ref::<&str>()
        .map(|text| text.to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no message".to_string())
}

fn write_down(info: &PanicHookInfo<'_>) {
    let Some(folder) = config_dir() else {
        return;
    };
    let _ = std::fs::create_dir_all(&folder);
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(folder.join(CRASH_FILE)) else {
        return;
    };
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH).map(|since| since.as_secs()).unwrap_or(0);
    let thread = std::thread::current();
    let place = info.location().map(|at| format!("{}:{}", at.file(), at.line())).unwrap_or_default();
    let _ = writeln!(
        file,
        "{seconds} | loupe {} | thread {} | {place} | {}",
        env!("CARGO_PKG_VERSION"),
        thread.name().unwrap_or("unnamed"),
        said(info)
    );
}

pub fn note_plugins(names: impl IntoIterator<Item = String>) {
    let mut names: Vec<String> =
        names.into_iter().map(|name| cut(&tidy_text(&name), LONGEST_NAME)).filter(|name| !name.trim().is_empty()).collect();
    names.sort();
    names.dedup();
    names.truncate(MOST_PLUGINS);
    if let Ok(mut around) = AROUND.lock() {
        around.plugins = names;
    }
}

pub fn note_audio(driver: &str, output: &str) {
    if let Ok(mut around) = AROUND.lock() {
        around.driver = cut(&tidy_text(driver), LONGEST_NAME);
        around.output = cut(&tidy_text(output), LONGEST_NAME);
    }
}

impl Report {
    fn around(kind: Kind) -> Self {
        let (plugins, driver, output) = match AROUND.try_lock() {
            Ok(around) => (around.plugins.clone(), around.driver.clone(), around.output.clone()),
            Err(_) => (Vec::new(), String::new(), String::new()),
        };
        Self {
            kind,
            version: THIS.to_string(),
            os: system().to_string(),
            place: String::new(),
            message: String::new(),
            backtrace: String::new(),
            plugins,
            plugin: String::new(),
            driver,
            output,
        }
    }

    pub fn panicked(place: &str, message: &str, trace: &str) -> Self {
        let place = if place.is_empty() { "unknown".to_string() } else { cut(&tidy_place(place), LONGEST_PLACE) };
        Self { place, message: cut(&tidy_text(message), LONGEST_MESSAGE), backtrace: tidy_trace(trace), ..Self::around(Kind::Panic) }
    }

    #[cfg(any(windows, test))]
    pub fn native(place: &str, message: &str, trace: &str) -> Self {
        Self {
            place: cut(&tidy_text(place), LONGEST_PLACE),
            message: cut(&tidy_text(message), LONGEST_MESSAGE),
            backtrace: tidy_trace(trace),
            ..Self::around(Kind::Native)
        }
    }

    pub fn plugin_fell(name: &str, why: &str) -> Self {
        Self { plugin: cut(&tidy_text(name), LONGEST_NAME), message: cut(&tidy_text(why), LONGEST_MESSAGE), ..Self::around(Kind::Plugin) }
    }

    pub fn is_plugin(&self) -> bool {
        self.kind == Kind::Plugin
    }

    pub fn shown(&self, note: &str) -> String {
        let note = tidy_note(note);
        let mut lines = vec![format!("Loupe version: {}", self.version), format!("System: {}", self.os)];
        match self.kind {
            Kind::Plugin => lines.push(format!("Plugin that crashed: {}", self.plugin)),
            Kind::Native => lines.push(format!("Where it broke: {} (a crash outside Rust's own checks)", self.place)),
            Kind::Panic => lines.push(format!("Where it broke: {}", self.place)),
        }
        lines.push(format!("Message: {}", self.message));
        let plugins = if self.plugins.is_empty() { "none".to_string() } else { self.plugins.join(", ") };
        lines.push(format!("Plugins open: {plugins}"));
        lines.push(format!("Audio: {} / {}", or_unknown(&self.driver), or_unknown(&self.output)));
        if !note.is_empty() {
            lines.push(format!("What you were doing: {note}"));
        }
        if !self.backtrace.is_empty() {
            lines.push("Backtrace:".to_string());
            lines.push(self.backtrace.clone());
        }
        lines.join("\n")
    }

    pub fn body(&self, note: &str) -> String {
        let plugins: Vec<String> = self.plugins.iter().map(|name| quoted(name)).collect();
        format!(
            "{{\"kind\":{},\"version\":{},\"os\":{},\"where\":{},\"message\":{},\"backtrace\":{},\"plugins\":[{}],\"plugin\":{},\"driver\":{},\"output\":{},\"note\":{}}}",
            quoted(self.kind.word()),
            quoted(&self.version),
            quoted(&self.os),
            quoted(&self.place),
            quoted(&self.message),
            quoted(&self.backtrace),
            plugins.join(","),
            quoted(&self.plugin),
            quoted(&self.driver),
            quoted(&self.output),
            quoted(&tidy_note(note)),
        )
    }

    fn to_text(&self) -> String {
        let one_line = |text: &str| text.replace('\\', "\\\\").replace('\n', "\\n");
        format!(
            "kind {}\nversion {}\nos {}\nwhere {}\nmessage {}\nplugins {}\nplugin {}\ndriver {}\noutput {}\nbacktrace\n{}",
            self.kind.word(),
            one_line(&self.version),
            one_line(&self.os),
            one_line(&self.place),
            one_line(&self.message),
            self.plugins.iter().map(|name| one_line(name)).collect::<Vec<_>>().join("\t"),
            one_line(&self.plugin),
            one_line(&self.driver),
            one_line(&self.output),
            self.backtrace
        )
    }

    fn from_text(text: &str) -> Option<Self> {
        let (head, backtrace) = text.split_once("\nbacktrace\n").unwrap_or((text, ""));
        let field =
            |key: &str| head.lines().find_map(|line| line.strip_prefix(key).and_then(|rest| rest.strip_prefix(' ')).map(unescape));
        let plugins = field("plugins").unwrap_or_default();
        Some(Self {
            kind: Kind::from_word(&field("kind")?)?,
            version: field("version")?,
            os: field("os")?,
            place: field("where").unwrap_or_default(),
            message: field("message").unwrap_or_default(),
            backtrace: backtrace.to_string(),
            plugins: plugins.split('\t').filter(|name| !name.is_empty()).map(str::to_string).collect(),
            plugin: field("plugin").unwrap_or_default(),
            driver: field("driver").unwrap_or_default(),
            output: field("output").unwrap_or_default(),
        })
    }
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

fn or_unknown(text: &str) -> &str {
    if text.is_empty() {
        "unknown"
    } else {
        text
    }
}

fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn cut(text: &str, longest: usize) -> String {
    if text.len() <= longest {
        return text.to_string();
    }
    let mut end = longest;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

fn tidy_note(note: &str) -> String {
    let plain: String = note.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    cut(tidy_text(plain.trim()).trim(), LONGEST_NOTE)
}

fn reports_folder() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join(REPORTS_FOLDER))
}

pub fn keep(report: &Report) -> Option<PathBuf> {
    keep_in(&reports_folder()?, report)
}

fn keep_in(folder: &Path, report: &Report) -> Option<PathBuf> {
    fs::create_dir_all(folder).ok()?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|since| since.as_nanos()).unwrap_or(0);
    let file = folder.join(format!("{stamp:024}-{}.{REPORT_EXTENSION}", std::process::id()));
    fs::write(&file, report.to_text()).ok()?;
    let mut kept = files_in(folder);
    while kept.len() > REPORTS_KEPT {
        let _ = fs::remove_file(kept.remove(0));
    }
    Some(file)
}

fn files_in(folder: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(folder)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|file| file.extension().is_some_and(|extension| extension == REPORT_EXTENSION))
        .collect();
    found.sort();
    found
}

pub fn waiting() -> Vec<Waiting> {
    reports_folder().map(|folder| waiting_in(&folder)).unwrap_or_default()
}

fn waiting_in(folder: &Path) -> Vec<Waiting> {
    let mut found = Vec::new();
    for file in files_in(folder) {
        match fs::read_to_string(&file).ok().as_deref().and_then(Report::from_text) {
            Some(report) => found.push(Waiting { file, report }),
            None => {
                let _ = fs::remove_file(&file);
            }
        }
    }
    found
}

pub fn forget(waiting: &Waiting) {
    let _ = fs::remove_file(&waiting.file);
}

fn address() -> Option<String> {
    match std::env::var(SEND_TO_VARIABLE) {
        Ok(to) if !to.is_empty() => Some(to),
        _ if cfg!(debug_assertions) => None,
        _ => Some(SEND_TO.to_string()),
    }
}

pub fn send(body: String) -> Result<(), String> {
    let to = address().ok_or("this is a test build, so it sends nowhere")?;
    let agent = ureq::AgentBuilder::new().timeout(WAIT).build();
    match agent.post(&to).set("Content-Type", "application/json").send_string(&body) {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(code, _)) => Err(format!("the server said {code}")),
        Err(_) => Err("the server could not be reached".to_string()),
    }
}

fn tidy_place(place: &str) -> String {
    match place.rsplit_once(':') {
        Some((file, line)) if !line.is_empty() && line.chars().all(|c| c.is_ascii_digit()) => format!("{}:{line}", tidy_source(file)),
        _ => tidy_source(place),
    }
}

pub fn tidy_source(path: &str) -> String {
    let path = format!("/{}", path.replace('\\', "/").trim_start_matches('/'));
    for (anchor, skipped) in [("/crates/", 0), ("/vendor/", 0), ("/library/", 0), ("/registry/src/", 1), ("/git/checkouts/", 2)] {
        if let Some(at) = path.rfind(anchor) {
            let rest = &path[at + anchor.len()..];
            let rest = rest.splitn(skipped + 1, '/').last().unwrap_or(rest);
            let kept = if skipped == 0 { format!("{}{rest}", &anchor[1..]) } else { rest.to_string() };
            return tidy_text(&kept);
        }
    }
    path.rsplit('/').next().map(tidy_text).unwrap_or_default()
}

fn frame_starts(line: &str) -> bool {
    line.trim_start().split_once(": ").is_some_and(|(number, _)| !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()))
}

pub fn tidy_trace(trace: &str) -> String {
    let lines: Vec<&str> = trace.lines().collect();
    let machinery = [
        "std::panicking",
        "core::panicking",
        "rust_begin_unwind",
        "begin_panic",
        "std::backtrace",
        "crash::keep_a_record",
        "crash::native",
        "panic_fmt",
        "__rust_end_short_backtrace",
    ];
    let ends = lines.iter().position(|line| frame_starts(line) && line.contains("__rust_begin_short_backtrace")).unwrap_or(lines.len());
    let skip_to = lines[..ends].iter().rposition(|line| frame_starts(line) && machinery.iter().any(|word| line.contains(word))).map_or(0, |at| at + 1);
    let mut out = Vec::new();
    let mut frames = 0;
    let mut kept_frame = false;
    for line in &lines[skip_to.min(lines.len())..] {
        if frame_starts(line) {
            kept_frame = !line.contains("__rust_begin_short_backtrace") && frames < FRAMES_KEPT;
            if !kept_frame {
                break;
            }
            frames += 1;
            out.push(tidy_text(line));
            continue;
        }
        if !kept_frame {
            continue;
        }
        match line.split_once("at ").filter(|(indent, _)| indent.trim().is_empty()) {
            Some((indent, at)) => out.push(format!("{indent}at {}", tidy_place_with_column(at))),
            None => out.push(tidy_text(line)),
        }
    }
    cut(&out.join("\n"), LONGEST_TRACE)
}

fn tidy_place_with_column(at: &str) -> String {
    let parts: Vec<&str> = at.rsplitn(3, ':').collect();
    let numbered = parts.len() == 3 && parts[..2].iter().all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()));
    if !numbered {
        return tidy_source(at);
    }
    format!("{}:{}:{}", tidy_source(parts[2]), parts[1], parts[0])
}

pub fn tidy_text(text: &str) -> String {
    let mut text = text.to_string();
    for home in [std::env::var("HOME"), std::env::var("USERPROFILE")].into_iter().flatten().filter(|home| home.len() > 1) {
        for separator in ['/', '\\'] {
            text = text.replace(&format!("{home}{separator}"), &format!("~{separator}"));
        }
    }
    let mut text = hide_addresses(&hide_paths(&text));
    let names = [std::env::var("USER"), std::env::var("USERNAME"), std::env::var("LOGNAME"), std::env::var("COMPUTERNAME"), std::env::var("HOSTNAME")];
    for name in names.into_iter().flatten() {
        text = hide_word(&text, &name);
    }
    text
}

fn path_starts(chars: &[char], at: usize) -> bool {
    let after_a_break = at == 0 || matches!(chars[at - 1], ' ' | '"' | '\'' | '(' | '[' | '=' | '`' | '<' | '\t' | '\n');
    if !after_a_break {
        return false;
    }
    let next = |n: usize| chars.get(at + n).copied();
    match chars[at] {
        '/' => next(1).is_some_and(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '~')),
        '~' => matches!(next(1), Some('/') | Some('\\')),
        '\\' => next(1) == Some('\\'),
        c if c.is_ascii_alphabetic() => next(1) == Some(':') && matches!(next(2), Some('/') | Some('\\')),
        _ => false,
    }
}

fn path_ends(chars: &[char], at: usize) -> bool {
    match chars[at] {
        '"' | '\'' | '`' | ')' | ']' | '>' | '\n' | ',' | ';' => true,
        ':' => chars.get(at + 1).is_none_or(|c| c.is_whitespace()),
        _ => false,
    }
}

fn hide_paths(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < chars.len() {
        if path_starts(&chars, at) {
            out.push_str(HIDDEN_PATH);
            while at < chars.len() && !path_ends(&chars, at) {
                at += 1;
            }
            continue;
        }
        out.push(chars[at]);
        at += 1;
    }
    out
}

fn four_numbers(word: &str) -> bool {
    let numbers: Vec<&str> = word.split('.').collect();
    numbers.len() == 4 && numbers.iter().all(|part| (1..=3).contains(&part.len()) && part.chars().all(|c| c.is_ascii_digit()))
}

fn an_address(word: &str) -> bool {
    let colons = word.matches(':').count();
    let four = four_numbers(word) || word.split_once(':').is_some_and(|(left, port)| four_numbers(left) && !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()));
    let six = word.len() >= 6
        && (word.contains("::") || colons >= 4)
        && word.chars().all(|c| c.is_ascii_hexdigit() || c == ':')
        && word.chars().any(|c| c.is_ascii_digit());
    four || six
}

fn hide_addresses(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let settle = |word: &mut String, out: &mut String| {
        let trailing = word.len() - word.trim_end_matches(['.', ':']).len();
        let rest = word.split_off(word.len() - trailing);
        out.push_str(if an_address(word) { HIDDEN_ADDRESS } else { word });
        out.push_str(&rest);
        word.clear();
    };
    for c in text.chars() {
        if c.is_ascii_hexdigit() || c == '.' || c == ':' {
            word.push(c);
            continue;
        }
        settle(&mut word, &mut out);
        out.push(c);
    }
    settle(&mut word, &mut out);
    out
}

fn hide_word(text: &str, word: &str) -> String {
    let word = word.trim();
    if word.chars().count() < SHORTEST_NAME_HIDDEN {
        return text.to_string();
    }
    let lower_text = text.to_lowercase();
    let lower_word = word.to_lowercase();
    if lower_text.len() != text.len() || lower_word.len() != word.len() {
        return text.replace(word, HIDDEN_NAME);
    }
    let part_of_a_word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    while let Some(found) = lower_text[from..].find(&lower_word) {
        let start = from + found;
        let end = start + lower_word.len();
        let alone = !part_of_a_word(text[..start].chars().next_back()) && !part_of_a_word(text[end..].chars().next());
        out.push_str(&text[from..start]);
        out.push_str(if alone { HIDDEN_NAME } else { &text[start..end] });
        from = end;
    }
    out.push_str(&text[from..]);
    out
}

#[cfg(windows)]
mod native {
    use std::ffi::c_void;
    use std::sync::atomic::Ordering;

    use super::{keep, Report, PANICKED};

    const STACK_OVERFLOW: u32 = 0xC000_00FD;
    const KEEP_LOOKING: i32 = 0;
    const FROM_ADDRESS: u32 = 0x4;
    const UNCHANGED_REFCOUNT: u32 = 0x2;
    const LONGEST_MODULE_NAME: usize = 520;

    #[repr(C)]
    struct ExceptionRecord {
        code: u32,
        flags: u32,
        record: *mut ExceptionRecord,
        address: *mut c_void,
        parameters: u32,
        information: [usize; 15],
    }

    #[repr(C)]
    struct ExceptionPointers {
        record: *mut ExceptionRecord,
        context: *mut c_void,
    }

    type Filter = Option<unsafe extern "system" fn(*mut ExceptionPointers) -> i32>;

    #[link(name = "kernel32")]
    extern "system" {
        fn SetUnhandledExceptionFilter(filter: Filter) -> Filter;
        fn GetModuleHandleExW(flags: u32, name: *const u16, module: *mut isize) -> i32;
        fn GetModuleFileNameW(module: isize, name: *mut u16, size: u32) -> u32;
    }

    pub fn watch() {
        unsafe {
            SetUnhandledExceptionFilter(Some(caught));
        }
    }

    fn named(code: u32) -> &'static str {
        match code {
            0xC000_0005 => "access violation",
            0xC000_00FD => "stack overflow",
            0xC000_0094 => "integer divide by zero",
            0xC000_001D => "illegal instruction",
            0xC000_0409 => "stack buffer overrun",
            0xC000_0374 => "heap corruption",
            0x8000_0003 => "breakpoint",
            _ => "unhandled exception",
        }
    }

    unsafe fn module_of(address: *mut c_void) -> (String, usize) {
        let mut module = 0isize;
        if GetModuleHandleExW(FROM_ADDRESS | UNCHANGED_REFCOUNT, address as *const u16, &mut module) == 0 || module == 0 {
            return ("unknown module".to_string(), address as usize);
        }
        let mut name = vec![0u16; LONGEST_MODULE_NAME];
        let length = GetModuleFileNameW(module, name.as_mut_ptr(), name.len() as u32) as usize;
        let full = String::from_utf16_lossy(&name[..length.min(name.len())]);
        let file = full.rsplit(['\\', '/']).next().unwrap_or("unknown module").to_string();
        (file, (address as usize).wrapping_sub(module as usize))
    }

    unsafe extern "system" fn caught(pointers: *mut ExceptionPointers) -> i32 {
        if PANICKED.load(Ordering::SeqCst) || pointers.is_null() || (*pointers).record.is_null() {
            return KEEP_LOOKING;
        }
        let record = &*(*pointers).record;
        let (module, offset) = module_of(record.address);
        let place = format!("{module}+0x{offset:x}");
        let message = format!("exception 0x{:08x} ({})", record.code, named(record.code));
        let kept = keep(&Report::native(&place, &message, ""));
        if let (Some(file), false) = (kept, record.code == STACK_OVERFLOW) {
            let trace = std::backtrace::Backtrace::force_capture().to_string();
            let _ = std::fs::write(&file, Report::native(&place, &message, &trace).to_text());
        }
        crate::backup::last_chance();
        KEEP_LOOKING
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Report {
        Report {
            kind: Kind::Panic,
            version: "0.5.2".into(),
            os: "windows".into(),
            place: "crates/app/src/main.rs:812".into(),
            message: "index out of bounds: the len is 3 but the index is 7".into(),
            backtrace: "   4: loupe::App::handle\n             at crates/app/src/main.rs:812:21".into(),
            plugins: vec!["Pro-Q 3".into(), "Serum \"x64\"".into()],
            plugin: String::new(),
            driver: "ASIO".into(),
            output: "Focusrite USB ASIO".into(),
        }
    }

    #[test]
    fn a_report_holds_every_agreed_field_and_nothing_else() {
        let body = sample().body("Dragging a clip");
        assert_eq!(
            body,
            "{\"kind\":\"panic\",\"version\":\"0.5.2\",\"os\":\"windows\",\"where\":\"crates/app/src/main.rs:812\",\"message\":\"index out of bounds: the len is 3 but the index is 7\",\"backtrace\":\"   4: loupe::App::handle\\n             at crates/app/src/main.rs:812:21\",\"plugins\":[\"Pro-Q 3\",\"Serum \\\"x64\\\"\"],\"plugin\":\"\",\"driver\":\"ASIO\",\"output\":\"Focusrite USB ASIO\",\"note\":\"Dragging a clip\"}"
        );
        let shown = sample().shown("Dragging a clip");
        for part in ["0.5.2", "windows", "crates/app/src/main.rs:812", "index out of bounds", "Pro-Q 3, Serum \"x64\"", "ASIO / Focusrite USB ASIO", "Dragging a clip", "loupe::App::handle"] {
            assert!(shown.contains(part), "{part} is not shown");
        }
        assert!(!sample().shown("").contains("What you were doing"));
    }

    #[test]
    fn what_is_shown_is_what_is_sent() {
        let note = "Bounced C:\\Users\\Someone\\Music\\My Song.wav to 10.0.0.12";
        let shown = sample().shown(note);
        let body = sample().body(note);
        let sent_note = tidy_note(note);
        assert!(shown.contains(&format!("What you were doing: {sent_note}")));
        assert!(body.contains(&quoted(&sent_note)));
        assert!(!body.contains("My Song") && !body.contains("10.0.0.12") && !body.contains("Someone"));
    }

    #[test]
    fn backtrace_paths_lose_the_home_folder_and_keep_the_source_file() {
        let trace = "stack backtrace:\n   0: std::backtrace::Backtrace::force_capture\n             at /rustc/90b35a6239c3d8bdabc530a6a0816f7ff89a0aaf/library/std/src/backtrace.rs:312:9\n   1: loupe::crash::keep_a_record::{{closure}}\n             at /home/someone/dev/loupe/crates/app/src/crash.rs:20:9\n   2: std::panicking::rust_panic_with_hook\n   3: loupe::App::handle\n             at /home/someone/dev/loupe/crates/app/src/main.rs:812:21\n   4: iced_winit::program::run\n             at /home/someone/.cargo/registry/src/index.crates.io-6f17d22bba15001f/iced_winit-0.13.0/src/program.rs:44:5\n   5: C:\\Users\\Someone\\loupe.exe\n   6: std::sys::backtrace::__rust_begin_short_backtrace\n             at /rustc/90b35a/library/std/src/sys/backtrace.rs:154:18\n   7: std::panicking::try::do_call\n   8: main\n";
        let tidy = tidy_trace(trace);
        assert!(tidy.starts_with("   3: loupe::App::handle"), "the panic machinery stays out: {tidy}");
        assert!(tidy.contains("at crates/app/src/main.rs:812:21"), "{tidy}");
        assert!(tidy.contains("at iced_winit-0.13.0/src/program.rs:44:5"), "{tidy}");
        assert!(!tidy.contains("someone") && !tidy.contains("Someone") && !tidy.contains("/home"), "{tidy}");
        assert!(!tidy.contains("8: main") && !tidy.contains("do_call") && !tidy.contains("__rust_begin_short_backtrace"), "{tidy}");
    }

    #[test]
    fn source_paths_are_cut_to_what_names_the_code() {
        assert_eq!(tidy_source("crates/app/src/main.rs"), "crates/app/src/main.rs");
        assert_eq!(tidy_source("/home/someone/loupe/crates/engine/src/audio.rs"), "crates/engine/src/audio.rs");
        assert_eq!(tidy_source("C:\\Users\\Someone\\loupe\\crates\\app\\src\\main.rs"), "crates/app/src/main.rs");
        assert_eq!(tidy_source("/rustc/abc/library/core/src/panicking.rs"), "library/core/src/panicking.rs");
        assert_eq!(tidy_source("/home/someone/.cargo/git/checkouts/iced-1a2b/3c4d/winit/src/lib.rs"), "winit/src/lib.rs");
        assert_eq!(tidy_source("/home/someone/Music/Secret Song/notes.rs"), "notes.rs");
        assert_eq!(tidy_source("./dev/someone/loupe/crates/app/src/main.rs"), "crates/app/src/main.rs");
        assert_eq!(tidy_source("./.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/iced-0.13.1/src/application.rs"), "iced-0.13.1/src/application.rs");
        assert_eq!(tidy_source("../Secret Song/notes.rs"), "notes.rs");
        assert_eq!(tidy_place("/home/someone/loupe/crates/app/src/main.rs:12"), "crates/app/src/main.rs:12");
    }

    #[test]
    fn messages_lose_paths_addresses_and_names() {
        assert_eq!(tidy_text("could not open /srv/someone/Music/My Song.wav: no such file"), "could not open <path>: no such file");
        assert_eq!(tidy_text("could not open \"D:\\Beats\\Song 3.lp\" now"), "could not open \"<path>\" now");
        assert_eq!(tidy_text("could not reach 192.168.0.158:8080 or fe80::1c2b:3d4e"), "could not reach <address> or <address>");
        assert_eq!(tidy_text("the len is 3.5 but std::io::Error says 1.2.3"), "the len is 3.5 but std::io::Error says 1.2.3");
        assert_eq!(tidy_text("and/or a ratio of 2/3"), "and/or a ratio of 2/3");
        assert_eq!(tidy_text("started at 12:34:56"), "started at 12:34:56");
        assert_eq!(hide_word("Ashley's AirPods by ash", "Ash"), "Ashley's AirPods by <name>");
        assert_eq!(hide_word("Ash's AirPods", "ash"), "<name>'s AirPods");
        assert_eq!(hide_word("a cat", "a"), "a cat");
    }

    #[test]
    fn a_kept_report_reads_back_the_same() {
        let folder = std::env::temp_dir().join(format!("loupe-crash-{}-keep", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        let mut report = sample();
        report.message = "two\nlines with a \\ in".into();
        let file = keep_in(&folder, &report).expect("kept");
        let found = waiting_in(&folder);
        assert_eq!(found, vec![Waiting { file: file.clone(), report: report.clone() }]);
        fs::write(folder.join("broken.report"), "nonsense").unwrap();
        assert_eq!(waiting_in(&folder).len(), 1, "a broken report is dropped");
        for _ in 0..REPORTS_KEPT + 3 {
            keep_in(&folder, &report);
        }
        assert_eq!(files_in(&folder).len(), REPORTS_KEPT);
        forget(&waiting_in(&folder)[0]);
        assert_eq!(files_in(&folder).len(), REPORTS_KEPT - 1);
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_plugin_crash_names_the_plugin_and_has_no_place() {
        let report = Report::plugin_fell("Serum", "the plugin crashed");
        assert!(report.is_plugin());
        assert!(report.body("").contains("\"kind\":\"plugin\""));
        assert!(report.body("").contains("\"plugin\":\"Serum\""));
        assert!(report.shown("").contains("Plugin that crashed: Serum"));
        assert!(report.place.is_empty() && report.backtrace.is_empty());
    }

    #[test]
    fn a_native_crash_names_the_module_and_the_exception() {
        let report = Report::native("loupe.exe+0x1a2b3c", "exception 0xc0000005 (access violation)", "");
        assert!(report.body("").contains("\"kind\":\"native\""));
        assert!(report.body("").contains("\"where\":\"loupe.exe+0x1a2b3c\""));
        assert!(report.shown("").contains("Where it broke: loupe.exe+0x1a2b3c (a crash outside Rust's own checks)"));
    }

    #[test]
    fn a_test_build_sends_nothing_unless_told_where() {
        if std::env::var_os(SEND_TO_VARIABLE).is_none() && cfg!(debug_assertions) {
            assert_eq!(address(), None);
            assert!(send(sample().body("")).is_err());
        }
    }

    #[test]
    fn long_text_is_cut_on_a_character() {
        assert_eq!(cut("ééé", 3), "é");
        assert_eq!(tidy_note(&"x".repeat(LONGEST_NOTE * 2)).len(), LONGEST_NOTE);
        assert_eq!(tidy_note("one\ntwo\u{7}"), "one two");
    }
}
