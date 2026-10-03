use std::fs;
use std::path::{Path, PathBuf};

pub const MIN_SCALE: f64 = 0.5;
pub const MAX_SCALE: f64 = 3.0;
const SETTINGS_FILE: &str = "settings";

pub struct Settings {
    pub theme: Option<String>,
    pub scale: f64,
    pub mixer_height: Option<f32>,
    pub folder: Option<PathBuf>,
    pub mixer_open: bool,
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
        }
    }
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
