use std::fs;
use std::path::{Path, PathBuf};

pub const MIN_SCALE: f64 = 0.5;
pub const MAX_SCALE: f64 = 3.0;
const SETTINGS_FILE: &str = "settings";
pub const AUTOSAVE_MINUTES: std::ops::RangeInclusive<u32> = 0..=60;
pub const DEFAULT_AUTOSAVE_MINUTES: u32 = 2;
pub const BACKUPS_KEPT: std::ops::RangeInclusive<u32> = 1..=100;
pub const DEFAULT_BACKUPS_KEPT: u32 = 20;
const LONGEST_NUMBER: usize = 3;

pub struct Settings {
    pub theme: Option<String>,
    pub scale: f64,
    pub mixer_height: Option<f32>,
    pub folder: Option<PathBuf>,
    pub mixer_open: bool,
    pub input: Option<String>,
    pub autosave_minutes: u32,
    pub backups_kept: u32,
    pub check_updates: bool,
    pub usage: Option<bool>,
    pub install_id: Option<String>,
    pub last_version: Option<String>,
}

impl Settings {
    pub fn load() -> Self {
        let text = settings_path().and_then(|path| fs::read_to_string(path).ok()).unwrap_or_default();
        let value_of = |wanted: &str| entries(&text).find(|(_, key, _)| *key == wanted).map(|(_, _, value)| value);
        Self {
            theme: value_of("theme").map(str::to_string),
            scale: value_of("scale").and_then(|value| value.parse().ok()).map_or(1.0, clamp_scale),
            mixer_height: value_of("mixer_height").and_then(|value| value.parse().ok()),
            folder: value_of("folder").map(PathBuf::from),
            mixer_open: value_of("mixer") == Some("open"),
            input: value_of("input").map(str::to_string),
            autosave_minutes: value_of("autosave_minutes").and_then(|text| autosave_minutes_from(text).ok()).unwrap_or(DEFAULT_AUTOSAVE_MINUTES),
            backups_kept: value_of("backups_kept").and_then(|text| backups_kept_from(text).ok()).unwrap_or(DEFAULT_BACKUPS_KEPT),
            check_updates: value_of("check_updates") != Some("off"),
            usage: value_of("usage").map(|value| value != "off"),
            install_id: value_of("install_id").map(str::to_string),
            last_version: value_of("last_version").map(str::to_string),
        }
    }
}

pub fn typed_number(text: &str) -> bool {
    text.len() <= LONGEST_NUMBER && text.chars().all(|c| c.is_ascii_digit())
}

fn whole_in(text: &str, range: &std::ops::RangeInclusive<u32>) -> Option<u32> {
    let text = text.trim();
    if text.is_empty() || !text.chars().all(|c| c.is_ascii_digit()) || text.len() > LONGEST_NUMBER {
        return None;
    }
    text.parse::<u32>().ok().filter(|number| range.contains(number))
}

pub fn autosave_minutes_from(text: &str) -> Result<u32, String> {
    whole_in(text, &AUTOSAVE_MINUTES).ok_or_else(|| {
        format!("Use a whole number of minutes from {} to {}. 0 turns autosave off.", AUTOSAVE_MINUTES.start(), AUTOSAVE_MINUTES.end())
    })
}

pub fn backups_kept_from(text: &str) -> Result<u32, String> {
    whole_in(text, &BACKUPS_KEPT).ok_or_else(|| format!("Keep from {} to {} backups.", BACKUPS_KEPT.start(), BACKUPS_KEPT.end()))
}

pub const HOME_FOLDER: &str = "Loupe";
const PROJECTS: &str = "Projects";
const TEMPLATES: &str = "Templates";

pub fn home_folder(chosen: Option<&Path>) -> PathBuf {
    chosen.map(Path::to_path_buf).unwrap_or_else(documents).join(HOME_FOLDER)
}

pub fn projects_folder(chosen: Option<&Path>) -> PathBuf {
    home_folder(chosen).join(PROJECTS)
}

pub fn templates_folder(chosen: Option<&Path>) -> PathBuf {
    home_folder(chosen).join(TEMPLATES)
}

pub fn make_folders(chosen: Option<&Path>) -> Result<(), String> {
    for folder in [projects_folder(chosen), templates_folder(chosen)] {
        fs::create_dir_all(&folder).map_err(|why| format!("{}: {why}", folder.display()))?;
    }
    Ok(())
}

fn documents() -> PathBuf {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    std::env::var_os("XDG_DOCUMENTS_DIR")
        .map(PathBuf::from)
        .or_else(|| home.map(|home| home.join("Documents")))
        .unwrap_or_else(|| PathBuf::from("."))
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

pub fn forget(key: &str) -> Result<(), String> {
    let Some(path) = settings_path() else {
        return Ok(());
    };
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let kept: Vec<&str> = existing
        .lines()
        .filter(|line| !line.split_once('=').is_some_and(|(found, _)| found.trim() == key))
        .collect();
    fs::write(&path, kept.join("\n") + "\n").map_err(|e| e.to_string())
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

const RECENT_FILE: &str = "recent";
const RECENT_KEPT: usize = 12;

pub fn recent() -> Vec<PathBuf> {
    let Some(file) = config_dir().map(|dir| dir.join(RECENT_FILE)) else {
        return Vec::new();
    };
    fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(PathBuf::from)
        .collect()
}

pub fn remember(project: &Path) {
    let Some(dir) = config_dir() else {
        return;
    };
    let mut list = recent();
    list.retain(|known| known != project);
    list.insert(0, project.to_path_buf());
    list.truncate(RECENT_KEPT);
    let lines: Vec<String> = list.iter().map(|path| path.display().to_string()).collect();
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(dir.join(RECENT_FILE), lines.join("\n") + "\n");
}

pub fn templates(chosen: Option<&Path>) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(templates_folder(chosen)) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension == "lp"))
        .collect();
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autosave_settings_only_take_sane_whole_numbers() {
        assert_eq!(autosave_minutes_from("2"), Ok(2));
        assert_eq!(autosave_minutes_from(" 0 "), Ok(0));
        assert_eq!(autosave_minutes_from("60"), Ok(60));
        for bad in ["61", "-1", "1.5", "", "two", "999", "0x2", "1e1", "+3"] {
            assert!(autosave_minutes_from(bad).is_err(), "{bad} was accepted");
        }
        assert_eq!(backups_kept_from("1"), Ok(1));
        assert_eq!(backups_kept_from("100"), Ok(100));
        for bad in ["0", "101", "", "-5", "ten", "20.0"] {
            assert!(backups_kept_from(bad).is_err(), "{bad} was accepted");
        }
        assert!(typed_number("12") && typed_number("") && !typed_number("1a") && !typed_number("1234"));
    }
}
