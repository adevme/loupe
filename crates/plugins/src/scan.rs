use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Format {
    Vst3,
    Clap,
    Lv2,
    Au,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::Vst3 => "VST3",
            Format::Clap => "CLAP",
            Format::Lv2 => "LV2",
            Format::Au => "AU",
        }
    }

    fn extension(self) -> &'static str {
        match self {
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

pub fn scan(places: &[(Format, PathBuf)]) -> Vec<Found> {
    let mut found = Vec::new();
    for (format, place) in places {
        if *format == Format::Au {
            continue;
        }
        look(*format, place, place, &mut found, 0);
    }
    found.extend(audio_units());
    found.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.format.cmp(&b.format)));
    found.dedup_by(|a, b| a.path == b.path && a.index == b.index);
    found
}

fn look(format: Format, root: &Path, at: &Path, found: &mut Vec<Found>, depth: usize) {
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
            if format == Format::Lv2 {
                for (index, one) in crate::lv2::plugins_in(&path).into_iter().enumerate() {
                    found.push(Found { name: one.name, path: path.clone(), format, vendor: vendor.clone(), index });
                }
                continue;
            }
            found.push(Found { name, path, format, vendor, index: 0 });
        } else if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            look(format, root, &path, found, depth + 1);
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
