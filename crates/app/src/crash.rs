use std::fs::OpenOptions;
use std::io::Write;
use std::panic::PanicHookInfo;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::settings::config_dir;

const CRASH_FILE: &str = "crash.log";

pub fn keep_a_record() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        write_down(info);
        crate::backup::last_chance();
        default_hook(info);
    }));
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
    let said = info
        .payload()
        .downcast_ref::<&str>()
        .map(|text| text.to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no message".to_string());
    let _ = writeln!(
        file,
        "{seconds} | loupe {} | thread {} | {place} | {said}",
        env!("CARGO_PKG_VERSION"),
        thread.name().unwrap_or("unnamed")
    );
}
