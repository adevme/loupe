use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use iced::widget::{checkbox, column, text};
use iced::Element;

use crate::settings::{self, config_dir};
use crate::versions::{parts, THIS};
use crate::{App, Message};

const PING: &str = "https://api.adev.me/loupe/ping";
const SESSION_FILE: &str = "session";
const WAIT: Duration = Duration::from_secs(4);
static KEPT: Mutex<Option<Usage>> = Mutex::new(None);

pub fn finish() {
    let kept = KEPT.lock().ok().and_then(|kept| kept.clone());
    if let Some(usage) = kept {
        usage.finish();
    }
}
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Installed,
    Opened,
    Moved { from: String, to: String },
    Crashed { version: String },
    Closed { minutes: u64 },
}

pub fn system() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "mac"
    } else {
        "linux"
    }
}

pub fn new_install_id() -> String {
    let mut bytes = [0u8; 16];
    for (i, half) in bytes.chunks_mut(8).enumerate() {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_usize(i);
        hasher.write_u128(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        hasher.write_u32(std::process::id());
        half.copy_from_slice(&hasher.finish().to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
}

pub fn body(install: &str, action: &Action) -> String {
    let (name, extra) = match action {
        Action::Installed => ("installed", String::new()),
        Action::Opened => ("opened", String::new()),
        Action::Moved { from, to } => {
            let name = if parts(to) >= parts(from) { "upgraded" } else { "downgraded" };
            (name, format!(",\"from\":\"{from}\",\"to\":\"{to}\""))
        }
        Action::Crashed { version } => ("crashed", format!(",\"crashed_version\":\"{version}\"")),
        Action::Closed { minutes } => ("closed", format!(",\"minutes\":{minutes}")),
    };
    format!("{{\"install\":\"{install}\",\"action\":\"{name}\",\"os\":\"{}\",\"version\":\"{THIS}\"{extra}}}", system())
}

fn send(install: &str, actions: &[Action]) {
    if cfg!(debug_assertions) || std::env::var_os("LOUPE_NO_USAGE").is_some() {
        return;
    }
    let agent = ureq::AgentBuilder::new().timeout(WAIT).build();
    for action in actions {
        let _ = agent.post(PING).set("Content-Type", "application/json").send_string(&body(install, action));
    }
}

fn session_file() -> Option<std::path::PathBuf> {
    config_dir().map(|dir| dir.join(SESSION_FILE))
}

#[derive(Clone)]
pub struct Usage {
    pub on: bool,
    install: Option<String>,
    started: Instant,
}

impl Usage {
    pub fn begin(stored: &settings::Settings) -> (Self, bool) {
        // Off until the user turns it on, so Loupe never sends anything it has not
        // been told it may send, and nothing has to be announced on the first run.
        let first_run = stored.usage.is_none();
        let on = stored.usage.unwrap_or(false);
        if first_run {
            let _ = settings::save("usage", "off");
        }
        let first_time_asked = first_run;
        let left_open = session_file().and_then(|file| std::fs::read_to_string(file).ok()).map(|text| text.trim().to_string());
        if let Some(file) = session_file() {
            let _ = std::fs::write(file, THIS);
        }
        let mut usage = Self { on, install: stored.install_id.clone(), started: Instant::now() };
        if !on || first_time_asked {
            if first_time_asked && usage.install.is_none() {
                let install = new_install_id();
                let _ = settings::save("install_id", &install);
                usage.install = Some(install);
            }
            if first_time_asked {
                usage.on = false;
                let _ = settings::save("last_version", THIS);
            }
            usage.keep();
            return (usage, first_time_asked);
        }
        let mut actions = Vec::new();
        match (&usage.install, stored.last_version.as_deref()) {
            (None, _) => {
                let install = new_install_id();
                let _ = settings::save("install_id", &install);
                usage.install = Some(install);
                actions.push(Action::Installed);
            }
            (Some(_), Some(last)) if last != THIS && parts(last).is_some() => {
                actions.push(Action::Moved { from: last.to_string(), to: THIS.to_string() });
            }
            _ => {}
        }
        if let Some(version) = left_open.filter(|version| parts(version).is_some()) {
            actions.push(Action::Crashed { version });
        }
        actions.push(Action::Opened);
        let _ = settings::save("last_version", THIS);
        if let Some(install) = usage.install.clone() {
            std::thread::spawn(move || send(&install, &actions));
        }
        usage.keep();
        (usage, first_time_asked)
    }

    pub fn keep(&self) {
        if let Ok(mut kept) = KEPT.lock() {
            *kept = Some(self.clone());
        }
    }

    pub fn finish(&self) {
        if let Some(file) = session_file() {
            let _ = std::fs::remove_file(file);
        }
        if let (true, Some(install)) = (self.on, &self.install) {
            let minutes = self.started.elapsed().as_secs() / 60;
            send(install, &[Action::Closed { minutes }]);
        }
    }
}

impl App {
    pub(crate) fn set_usage(&mut self, on: bool) {
        self.usage.on = on;
        if on && self.usage.install.is_none() {
            let install = new_install_id();
            let _ = settings::save("install_id", &install);
            self.usage.install = Some(install);
        }
        self.usage.keep();
        if let Err(why) = settings::save("usage", if on { "on" } else { "off" }) {
            self.problem = Some(format!("Could not save settings: {why}"));
        }
    }

    pub(crate) fn privacy_settings(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let dim = |words: &'static str| text(words).size(12).color(palette.text_dim);
        column![
            text("Anonymous usage info").size(13).font(palette.medium),
            dim("When on, Loupe tells the Loupe server when it is installed, opened, updated, closed or after a crash, so we know how many people use it and on which systems."),
            dim("Each message holds only a random install ID made on this computer, the action, the Loupe version and whether this is Windows, Mac or Linux, plus the length of the session when Loupe closes and the old and new version after an update."),
            dim("Never your name, files, songs, plugins or settings. The server keeps the day, not the time, and no IP address."),
            checkbox("Send anonymous usage info", self.usage.on).on_toggle(Message::UsageToggled).text_size(13),
        ]
        .spacing(8)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_run_says_nothing_until_the_notice_has_been_seen() {
        let fresh = settings::Settings { midi_inputs: None,
            theme: None,
            scale: 1.0,
            mixer_height: None,
            folder: None,
            mixer_open: false,
            input: None,
            autosave_minutes: 5,
            backups_kept: 10,
            check_updates: false,
            usage: None,
            install_id: None,
            last_version: None,
            metronome: false,
            count_in_bars: 0,
            hear_input: true,
            snap: true,
            audio: loupe_engine::Device::default(),
        };
        let (usage, told) = Usage::begin(&fresh);
        assert!(told, "the notice should be shown on the first run");
        assert!(!usage.on, "nothing should be sent before the notice is read");
        assert!(usage.install.is_some(), "the install should still be given an id");
    }

    #[test]
    fn install_ids_are_random_version_four_ids() {
        let id = new_install_id();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
        assert_ne!(id, new_install_id());
    }

    #[test]
    fn each_message_holds_only_the_agreed_fields() {
        let id = "4f1c9a2e-7b3d-4e8a-9c61-2d5b8f0e7a13";
        let opened = body(id, &Action::Opened);
        assert_eq!(opened, format!("{{\"install\":\"{id}\",\"action\":\"opened\",\"os\":\"{}\",\"version\":\"{THIS}\"}}", system()));
        assert!(body(id, &Action::Moved { from: "0.0.1".into(), to: "9.0.0".into() }).contains("\"action\":\"upgraded\",\"os\""));
        assert!(body(id, &Action::Moved { from: "9.0.0".into(), to: "0.0.1".into() }).contains("\"action\":\"downgraded\""));
        assert!(body(id, &Action::Moved { from: "9.0.0".into(), to: "0.0.1".into() }).ends_with(",\"from\":\"9.0.0\",\"to\":\"0.0.1\"}"));
        assert!(body(id, &Action::Crashed { version: "1.0.0".into() }).ends_with(",\"crashed_version\":\"1.0.0\"}"));
        assert!(body(id, &Action::Closed { minutes: 42 }).ends_with(",\"minutes\":42}"));
    }
}
