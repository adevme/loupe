use std::fs;
use std::path::{Path, PathBuf};

fn cache_file() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(|appdata| PathBuf::from(appdata).join("loupe"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .map(|config| config.join("loupe"))
    };
    base.map(|dir| dir.join("plugin-classes"))
}

fn stamp(path: &Path) -> String {
    let Ok(facts) = fs::metadata(path) else {
        return "0-0".to_string();
    };
    let when = facts
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|gap| gap.as_secs())
        .unwrap_or(0);
    format!("{when}-{}", facts.len())
}

#[derive(Default)]
struct Remembered {
    known: std::collections::HashMap<String, (String, Vec<String>)>,
    fresh: std::collections::HashMap<String, (String, Vec<String>)>,
}

impl Remembered {
    fn read() -> Self {
        let mut known = std::collections::HashMap::new();
        if let Some(text) = cache_file().and_then(|file| fs::read_to_string(file).ok()) {
            for line in text.lines() {
                let mut parts = line.split('\t');
                let (Some(path), Some(stamped)) = (parts.next(), parts.next()) else {
                    continue;
                };
                let names: Vec<String> = parts.map(|one| one.replace('\x1f', "")).collect();
                known.insert(path.to_string(), (stamped.to_string(), names));
            }
        }
        Self { known, fresh: std::collections::HashMap::new() }
    }

    fn look(&self, path: &Path, now: &str) -> Option<Vec<String>> {
        let (stamped, names) = self.known.get(&path.display().to_string())?;
        (stamped == now).then(|| names.clone())
    }

    fn keep(&mut self, path: &Path, now: &str, names: &[String]) {
        self.fresh.insert(path.display().to_string(), (now.to_string(), names.to_vec()));
    }

    fn write(&self) {
        let Some(file) = cache_file() else { return };
        if let Some(folder) = file.parent() {
            let _ = fs::create_dir_all(folder);
        }
        let mut text = String::new();
        for (path, (stamped, names)) in &self.fresh {
            text.push_str(path);
            text.push('\t');
            text.push_str(stamped);
            for one in names {
                text.push('\t');
                text.push_str(one);
            }
            text.push('\n');
        }
        let _ = fs::write(file, text);
    }
}

struct Asker {
    host: PathBuf,
    open: Option<crate::sandbox::Sandbox>,
}

impl Asker {
    fn new() -> Self {
        Self { host: crate::sandbox::host_beside_us(), open: None }
    }

    fn classes(&mut self, path: &Path) -> Option<Vec<String>> {
        for _ in 0..2 {
            if self.open.is_none() {
                self.open = crate::sandbox::Sandbox::start(&self.host).ok();
            }
            let box_ = self.open.as_mut()?;
            match box_.ask(crate::wire::Ask::Classes(path.display().to_string())) {
                Ok(crate::wire::Reply::Classes(names)) => return Some(names),
                Ok(_) => return None,
                Err(_) => self.open = None,
            }
        }
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Format {
    Stock,
    Vst3,
    Clap,
    Lv2,
    Au,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::Stock => "Built in",
            Format::Vst3 => "VST3",
            Format::Clap => "CLAP",
            Format::Lv2 => "LV2",
            Format::Au => "AU",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Format::Stock => "loupe",
            Format::Vst3 => "vst3",
            Format::Clap => "clap",
            Format::Lv2 => "lv2",
            Format::Au => "component",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub name: String,
    pub path: PathBuf,
    pub format: Format,
    pub vendor: Option<String>,
    pub index: usize,
}

pub fn folders() -> Vec<(Format, PathBuf)> {
    let mut places = Vec::new();
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    if cfg!(windows) {
        let common = std::env::var_os("CommonProgramFiles").map(PathBuf::from);
        if let Some(common) = common {
            places.push((Format::Vst3, common.join("VST3")));
            places.push((Format::Clap, common.join("CLAP")));
        }
        if let Some(home) = &home {
            places.push((Format::Clap, home.join("AppData/Local/Programs/Common/CLAP")));
        }
    } else if cfg!(target_os = "macos") {
        places.push((Format::Vst3, PathBuf::from("/Library/Audio/Plug-Ins/VST3")));
        places.push((Format::Clap, PathBuf::from("/Library/Audio/Plug-Ins/CLAP")));
        places.push((Format::Au, PathBuf::from("/Library/Audio/Plug-Ins/Components")));
        if let Some(home) = &home {
            places.push((Format::Vst3, home.join("Library/Audio/Plug-Ins/VST3")));
            places.push((Format::Clap, home.join("Library/Audio/Plug-Ins/CLAP")));
            places.push((Format::Au, home.join("Library/Audio/Plug-Ins/Components")));
        }
    } else {
        places.push((Format::Vst3, PathBuf::from("/usr/lib/vst3")));
        places.push((Format::Vst3, PathBuf::from("/usr/local/lib/vst3")));
        places.push((Format::Clap, PathBuf::from("/usr/lib/clap")));
        places.push((Format::Clap, PathBuf::from("/usr/local/lib/clap")));
        places.push((Format::Lv2, PathBuf::from("/usr/lib/lv2")));
        places.push((Format::Lv2, PathBuf::from("/usr/local/lib/lv2")));
        if let Some(home) = &home {
            places.push((Format::Vst3, home.join(".vst3")));
            places.push((Format::Clap, home.join(".clap")));
            places.push((Format::Lv2, home.join(".lv2")));
        }
    }
    places
}

pub fn everything() -> Vec<Found> {
    let mut found = built_in();
    found.extend(scan(&folders()));
    found.extend(audio_units());
    found
}

pub fn scan(places: &[(Format, PathBuf)]) -> Vec<Found> {
    let mut seen = Remembered::read();
    let mut found = Vec::new();
    let mut wanted: Vec<(PathBuf, Option<String>, String)> = Vec::new();
    for (format, place) in places {
        if matches!(format, Format::Au | Format::Stock) {
            continue;
        }
        look(*format, place, place, &mut found, 0, &mut seen, &mut wanted);
    }

    if !wanted.is_empty() {
        let shared = std::sync::Arc::new(std::sync::Mutex::new(wanted.clone()));
        let hands = std::thread::available_parallelism().map(|many| many.get().min(8)).unwrap_or(4).max(2);
        let mut crews = Vec::new();
        for _ in 0..hands {
            let queue = std::sync::Arc::clone(&shared);
            crews.push(std::thread::spawn(move || {
                let mut asker = Asker::new();
                let mut done: Vec<(PathBuf, String, Vec<String>)> = Vec::new();
                loop {
                    let next = queue.lock().ok().and_then(|mut rest| rest.pop());
                    let Some((path, _, now)) = next else {
                        break;
                    };
                    let names = asker.classes(&path).unwrap_or_default();
                    done.push((path, now, names));
                }
                done
            }));
        }
        let mut answers = std::collections::HashMap::new();
        for crew in crews {
            for (path, now, names) in crew.join().unwrap_or_default() {
                seen.keep(&path, &now, &names);
                answers.insert(path, names);
            }
        }
        for (path, vendor, _) in wanted {
            let plain = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            match answers.get(&path).filter(|names| !names.is_empty()) {
                Some(names) => {
                    for (index, one) in names.iter().enumerate() {
                        if one.is_empty() {
                            continue;
                        }
                        found.push(Found { name: one.clone(), path: path.clone(), format: Format::Vst3, vendor: vendor.clone(), index });
                    }
                }
                None => found.push(Found { name: plain, path, format: Format::Vst3, vendor, index: 0 }),
            }
        }
    }

    seen.write();
    found.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.format.cmp(&b.format)));
    found.dedup_by(|a, b| a.path == b.path && a.index == b.index);
    found
}

fn look(format: Format, root: &Path, at: &Path, found: &mut Vec<Found>, depth: usize, seen: &mut Remembered, wanted: &mut Vec<(PathBuf, Option<String>, String)>) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = fs::read_dir(at) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_plugin = path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case(format.extension()));
        if is_plugin {
            let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let vendor = path
                .parent()
                .filter(|parent| *parent != root)
                .and_then(|parent| parent.file_name())
                .map(|name| name.to_string_lossy().into_owned());
            if format == Format::Vst3 {
                let now = stamp(&path);
                match seen.look(&path, &now) {
                    Some(names) => {
                        seen.keep(&path, &now, &names);
                        match names.iter().any(|one| !one.is_empty()) {
                            true => {
                                for (index, one) in names.into_iter().enumerate() {
                                    if one.is_empty() {
                                        continue;
                                    }
                                    found.push(Found { name: one, path: path.clone(), format, vendor: vendor.clone(), index });
                                }
                            }
                            false => found.push(Found { name: name.clone(), path: path.clone(), format, vendor: vendor.clone(), index: 0 }),
                        }
                    }
                    None => wanted.push((path.clone(), vendor.clone(), now)),
                }
                continue;
            }
            if format == Format::Lv2 {
                for (index, one) in crate::lv2::plugins_in(&path).into_iter().enumerate() {
                    found.push(Found { name: one.name, path: path.clone(), format, vendor: vendor.clone(), index });
                }
                continue;
            }
            found.push(Found { name, path, format, vendor, index: 0 });
        } else if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            look(format, root, &path, found, depth + 1, seen, wanted);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let at = std::env::temp_dir().join(format!("loupe-scan-{name}"));
        let _ = fs::remove_dir_all(&at);
        fs::create_dir_all(&at).unwrap();
        at
    }

    #[test]
    fn it_finds_plugins_and_names_their_vendor() {
        let root = scratch("vendor");
        fs::create_dir_all(root.join("FabFilter")).unwrap();
        fs::write(root.join("FabFilter/FabFilter Pro-Q 4.vst3"), b"x").unwrap();
        fs::write(root.join("Loose.vst3"), b"x").unwrap();
        let found = scan(&[(Format::Vst3, root.clone())]);
        assert_eq!(found.len(), 2);
        let pro_q = found.iter().find(|f| f.name.contains("Pro-Q")).unwrap();
        assert_eq!(pro_q.vendor.as_deref(), Some("FabFilter"));
        assert_eq!(pro_q.format, Format::Vst3);
        let loose = found.iter().find(|f| f.name == "Loose").unwrap();
        assert_eq!(loose.vendor, None);
    }

    #[test]
    fn a_vst3_can_be_a_folder() {
        let root = scratch("bundle");
        fs::create_dir_all(root.join("Thing.vst3/Contents/x86_64-win")).unwrap();
        let found = scan(&[(Format::Vst3, root.clone())]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Thing");
    }

    #[test]
    fn other_formats_are_left_alone() {
        let root = scratch("mixed");
        fs::write(root.join("A.vst3"), b"x").unwrap();
        fs::write(root.join("B.clap"), b"x").unwrap();
        let found = scan(&[(Format::Vst3, root.clone())]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "A");
    }
}

#[cfg(target_os = "macos")]
fn audio_units() -> Vec<Found> {
    crate::au::effects()
        .into_iter()
        .enumerate()
        .map(|(index, (name, _))| Found {
            name,
            path: PathBuf::from("audio-units.component"),
            format: Format::Au,
            vendor: None,
            index,
        })
        .collect()
}

#[cfg(not(target_os = "macos"))]
fn audio_units() -> Vec<Found> {
    Vec::new()
}

pub const BUILT_IN: &str = "loupe.loupe";

fn built_in() -> Vec<Found> {
    loupe_stock::NAMES
        .iter()
        .enumerate()
        .map(|(index, name)| Found {
            name: name.to_string(),
            path: PathBuf::from(BUILT_IN),
            format: Format::Stock,
            vendor: None,
            index,
        })
        .collect()
}
