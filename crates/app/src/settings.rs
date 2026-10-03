use std::fs;
use std::path::PathBuf;

pub const MIN_SCALE: f64 = 0.5;
pub const MAX_SCALE: f64 = 3.0;
const SETTINGS_FILE: &str = "settings";

pub struct Settings {
    pub theme: Option<String>,
    pub scale: f64,
}

impl Settings {
    pub fn load() -> Self {
        let text = settings_path().and_then(|path| fs::read_to_string(path).ok()).unwrap_or_default();
        let value_of = |wanted: &str| entries(&text).find(|(_, key, _)| *key == wanted).map(|(_, _, value)| value);
        Self {
            theme: value_of("theme").map(str::to_string),
            scale: value_of("scale").and_then(|value| value.parse().ok()).map_or(1.0, clamp_scale),
        }
    }
}

pub fn clamp_scale(scale: f64) -> f64 {
    if scale.is_finite() {
        scale.clamp(MIN_SCALE, MAX_SCALE)
    } else {
        1.0
    }
}

pub fn save(key: &str, value: &str) -> Result<(), String> {
    let path = settings_path().ok_or("there is no settings folder on this system")?;
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let entry = format!("{key} = {value}");
    let mut replaced = false;
    let mut lines: Vec<String> = existing
        .lines()
        .map(|line| {
            let holds_key = line.split_once('=').is_some_and(|(found, _)| found.trim() == key);
            if holds_key && !replaced {
                replaced = true;
                entry.clone()
            } else {
                line.to_string()
            }
        })
        .collect();
    if !replaced {
        lines.push(entry);
    }
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    }
    fs::write(&path, lines.join("\n") + "\n").map_err(|e| e.to_string())
}

pub fn config_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        return std::env::var_os("APPDATA").map(|appdata| PathBuf::from(appdata).join("loupe"));
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .map(|config| config.join("loupe"))
}

fn settings_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join(SETTINGS_FILE))
}

pub fn entries(text: &str) -> impl Iterator<Item = (usize, &str, &str)> {
    text.lines().enumerate().filter_map(|(i, line)| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let (key, value) = line.split_once('=')?;
        Some((i + 1, key.trim(), value.trim()))
    })
}
