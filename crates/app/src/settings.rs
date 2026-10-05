use std::fs;
use std::path::{Path, PathBuf};

pub const MIN_SCALE: f64 = 0.5;
pub const MAX_SCALE: f64 = 3.0;
const SETTINGS_FILE: &str = "settings";
pub const AUTOSAVE_MINUTES: std::ops::RangeInclusive<u32> = 0..=60;
pub const DEFAULT_AUTOSAVE_MINUTES: u32 = 2;
pub const BACKUPS_KEPT: std::ops::RangeInclusive<u32> = 1..=100;
pub const DEFAULT_BACKUPS_KEPT: u32 = 20;
pub const COUNT_IN_BARS: [u32; 4] = [0, 1, 2, 4];
pub const DEFAULT_PREROLL_BARS: u32 = 2;
const LONGEST_NUMBER: usize = 3;

pub struct Settings {
    pub theme: Option<String>,
    pub scale: f64,
    pub mixer_height: Option<f32>,
    pub folder: Option<PathBuf>,
    pub mixer_open: bool,
    pub mixer_alone: bool,
    pub input: Option<String>,
    pub autosave_minutes: u32,
    pub backups_kept: u32,
    pub check_updates: bool,
    pub usage: Option<bool>,
    pub install_id: Option<String>,
    pub last_version: Option<String>,
    pub metronome: bool,
    pub count_in_bars: u32,
    pub preroll_bars: u32,
    pub punch: bool,
    pub hear_input: Hearing,
    pub snap: bool,
    pub midi_inputs: Option<Vec<String>>,
    pub audio: loupe_engine::Device,
    pub export: ExportChoices,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExportChoices {
    pub format: loupe_engine::Format,
    pub dither_16: bool,
    pub dither_24: bool,
    pub normalise: loupe_engine::Normalise,
    pub split: bool,
}

impl ExportChoices {
    pub fn from_settings(value_of: impl Fn(&str) -> Option<String>) -> Self {
        Self {
            format: value_of("export_format").and_then(|key| loupe_engine::Format::from_key(&key)).unwrap_or(loupe_engine::Format::WavFloat),
            dither_16: value_of("export_dither_16").as_deref() != Some("off"),
            dither_24: value_of("export_dither_24").as_deref() == Some("on"),
            normalise: value_of("export_normalise").and_then(|key| loupe_engine::Normalise::from_key(&key)).unwrap_or(loupe_engine::Normalise::Off),
            split: value_of("export_split").as_deref() == Some("on"),
        }
    }

    pub fn entries(&self) -> [(&'static str, String); 5] {
        let on_off = |on: bool| if on { "on" } else { "off" }.to_string();
        [
            ("export_format", self.format.key().to_string()),
            ("export_dither_16", on_off(self.dither_16)),
            ("export_dither_24", on_off(self.dither_24)),
            ("export_normalise", self.normalise.key()),
            ("export_split", on_off(self.split)),
        ]
    }

    pub fn dither(&self) -> bool {
        match self.format.bits() {
            Some(16) => self.dither_16,
            Some(_) => self.dither_24,
            None => false,
        }
    }
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
            mixer_alone: value_of("mixer_alone") == Some("yes"),
            input: value_of("input").map(str::to_string),
            autosave_minutes: value_of("autosave_minutes").and_then(|text| autosave_minutes_from(text).ok()).unwrap_or(DEFAULT_AUTOSAVE_MINUTES),
            backups_kept: value_of("backups_kept").and_then(|text| backups_kept_from(text).ok()).unwrap_or(DEFAULT_BACKUPS_KEPT),
            check_updates: value_of("check_updates") != Some("off"),
            usage: value_of("usage").map(|value| value != "off"),
            install_id: value_of("install_id").map(str::to_string),
            last_version: value_of("last_version").map(str::to_string),
            metronome: value_of("metronome") == Some("on"),
            count_in_bars: value_of("count_in").and_then(count_in_from).unwrap_or(0),
            preroll_bars: value_of("preroll").and_then(count_in_from).unwrap_or(DEFAULT_PREROLL_BARS),
            punch: value_of("punch") == Some("on"),
            hear_input: Hearing::from_saved(value_of("hear_input")),
            snap: value_of("snap") != Some("off"),
            midi_inputs: value_of("midi_inputs").map(|value| value.split('\t').filter(|name| !name.is_empty()).map(str::to_string).collect()),
            audio: loupe_engine::Device {
                driver: value_of("audio_driver").map(str::to_string),
                output: value_of("audio_output").map(str::to_string),
                rate: value_of("audio_rate").and_then(|text| text.parse().ok()).filter(|rate| loupe_engine::RATES.contains(rate)),
                buffer: value_of("audio_buffer").and_then(|text| text.parse().ok()).filter(|size| loupe_engine::BUFFERS.contains(size)),
            },
            export: ExportChoices::from_settings(|key| value_of(key).map(str::to_string)),
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

pub fn count_in_from(text: &str) -> Option<u32> {
    let text = text.trim();
    (text.len() == 1 && text.chars().all(|c| c.is_ascii_digit())).then(|| text.parse::<u32>().ok()).flatten().filter(|bars| COUNT_IN_BARS.contains(bars))
}

pub const HOME_FOLDER: &str = "Loupe";
const PROJECTS: &str = "Projects";
const TEMPLATES: &str = "Templates";
const CHAINS: &str = "Chains";
const PRESETS: &str = "Presets";

pub fn home_folder(chosen: Option<&Path>) -> PathBuf {
    chosen.map(Path::to_path_buf).unwrap_or_else(documents).join(HOME_FOLDER)
}

pub fn projects_folder(chosen: Option<&Path>) -> PathBuf {
    home_folder(chosen).join(PROJECTS)
}

pub fn templates_folder(chosen: Option<&Path>) -> PathBuf {
    home_folder(chosen).join(TEMPLATES)
}

pub fn chains_folder(chosen: Option<&Path>) -> PathBuf {
    home_folder(chosen).join(CHAINS)
}

pub fn presets_folder(chosen: Option<&Path>) -> PathBuf {
    home_folder(chosen).join(PRESETS)
}

pub fn make_folders(chosen: Option<&Path>) -> Result<(), String> {
    for folder in [projects_folder(chosen), templates_folder(chosen), chains_folder(chosen)] {
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
    fn hearing_says_when_the_input_comes_through() {
        assert!(!Hearing::Never.wants_input(true, true));
        assert!(Hearing::WhileArmed.wants_input(true, false), "armed is enough");
        assert!(Hearing::WhileArmed.wants_input(true, true));
        assert!(!Hearing::WhileRecording.wants_input(true, false), "armed alone is not enough");
        assert!(Hearing::WhileRecording.wants_input(true, true));
    }

    #[test]
    fn hearing_is_saved_and_read_back_and_an_old_on_still_means_recording() {
        for mode in [Hearing::Never, Hearing::WhileArmed, Hearing::WhileRecording] {
            assert_eq!(Hearing::from_saved(Some(mode.saved_as())), mode);
        }
        assert_eq!(Hearing::from_saved(Some("on")), Hearing::WhileRecording, "settings written before the modes existed");
        assert_eq!(Hearing::from_saved(None), Hearing::WhileRecording);
    }

    #[test]
    fn count_in_is_one_of_the_offered_lengths() {
        for (text, bars) in [("0", 0), ("1", 1), (" 2 ", 2), ("4", 4)] {
            assert_eq!(count_in_from(text), Some(bars));
        }
        for bad in ["3", "8", "+1", "-1", "", "1.0", "two", "04"] {
            assert_eq!(count_in_from(bad), None, "{bad} was accepted");
        }
    }

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
    #[test]
    fn export_choices_start_as_float_wav_and_come_back_as_saved() {
        let fresh = ExportChoices::from_settings(|_| None);
        assert_eq!(fresh.format, loupe_engine::Format::WavFloat);
        assert_eq!(fresh.normalise, loupe_engine::Normalise::Off);
        assert!(fresh.dither_16 && !fresh.dither_24 && !fresh.split && !fresh.dither());
        let chosen = ExportChoices {
            format: loupe_engine::Format::Flac24,
            dither_16: false,
            dither_24: true,
            normalise: loupe_engine::Normalise::Loudness(-14.0),
            split: true,
        };
        let saved = chosen.entries();
        let back = ExportChoices::from_settings(|key| saved.iter().find(|(name, _)| *name == key).map(|(_, value)| value.clone()));
        assert_eq!(back, chosen);
        assert!(back.dither());
        let mp3 = ExportChoices { format: loupe_engine::Format::Mp3Cbr320, ..back };
        assert!(!mp3.dither(), "MP3 and float are never dithered");
        let sixteen = ExportChoices { format: loupe_engine::Format::Wav16, ..fresh };
        assert!(sixteen.dither());
        let junk = ExportChoices::from_settings(|_| Some("nonsense".to_string()));
        assert_eq!(junk.format, loupe_engine::Format::WavFloat);
        assert_eq!(junk.normalise, loupe_engine::Normalise::Off);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Hearing {
    Never,
    WhileArmed,
    #[default]
    WhileRecording,
}

impl Hearing {
    pub fn saved_as(self) -> &'static str {
        match self {
            Hearing::Never => "off",
            Hearing::WhileArmed => "armed",
            Hearing::WhileRecording => "recording",
        }
    }

    pub fn from_saved(value: Option<&str>) -> Self {
        match value {
            Some("off") => Hearing::Never,
            Some("armed") => Hearing::WhileArmed,
            _ => Hearing::WhileRecording,
        }
    }

    pub fn wants_input(self, armed: bool, recording: bool) -> bool {
        match self {
            Hearing::Never => false,
            Hearing::WhileArmed => armed,
            Hearing::WhileRecording => recording,
        }
    }
}

impl std::fmt::Display for Hearing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Hearing::Never => write!(f, "Never"),
            Hearing::WhileArmed => write!(f, "Whenever a track is armed"),
            Hearing::WhileRecording => write!(f, "Only while recording"),
        }
    }
}
