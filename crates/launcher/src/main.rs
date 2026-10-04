#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::process::Command;

const CURRENT_FILE: &str = "current";
const VERSIONS_FOLDER: &str = "versions";

fn program_name() -> &'static str {
    if cfg!(windows) {
        "loupe.exe"
    } else {
        "loupe"
    }
}

pub fn version_parts(version: &str) -> Option<[u64; 3]> {
    let mut parts = version.trim().trim_start_matches('v').split('.').map(|part| part.parse::<u64>().ok());
    let parsed = [parts.next()??, parts.next().flatten().unwrap_or(0), parts.next().flatten().unwrap_or(0)];
    parts.next().is_none().then_some(parsed)
}

fn installed(home: &Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(home.join(VERSIONS_FOLDER))
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().join(program_name()).is_file())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| version_parts(name).is_some())
        .collect();
    found.sort_by_key(|name| version_parts(name));
    found
}

pub fn chosen(home: &Path) -> Option<PathBuf> {
    let wanted = std::fs::read_to_string(home.join(CURRENT_FILE)).ok().map(|text| text.trim().to_string());
    let versions = installed(home);
    let version = wanted.filter(|wanted| versions.contains(wanted)).or_else(|| versions.last().cloned())?;
    Some(home.join(VERSIONS_FOLDER).join(version).join(program_name()))
}

fn main() {
    let Some(home) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)) else {
        return;
    };
    let Some(program) = chosen(&home) else {
        return;
    };
    // Wait for Loupe rather than spawning and leaving. Whoever started the launcher
    // may hold it in a job that is killed when it returns, which would take Loupe with it.
    let _ = Command::new(program).args(std::env::args_os().skip(1)).status();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(name: &str, versions: &[&str], current: Option<&str>) -> PathBuf {
        let home = std::env::temp_dir().join(format!("loupe-launcher-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for version in versions {
            let folder = home.join(VERSIONS_FOLDER).join(version);
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join(program_name()), "").unwrap();
        }
        std::fs::create_dir_all(home.join(VERSIONS_FOLDER).join("1.9.0-broken")).unwrap();
        if let Some(current) = current {
            std::fs::write(home.join(CURRENT_FILE), format!("{current}\n")).unwrap();
        }
        home
    }

    #[test]
    fn versions_compare_by_number_not_by_text() {
        assert!(version_parts("1.10.0") > version_parts("1.9.3"));
        assert_eq!(version_parts("v2.1"), Some([2, 1, 0]));
        assert_eq!(version_parts("1.0.0.0"), None);
        assert_eq!(version_parts("nope"), None);
    }

    #[test]
    fn the_chosen_version_starts_and_a_missing_choice_falls_back_to_the_newest() {
        let place = home("chosen", &["1.0.0", "1.2.0", "1.10.0"], Some("1.2.0"));
        assert_eq!(chosen(&place), Some(place.join("versions").join("1.2.0").join(program_name())));
        std::fs::write(place.join(CURRENT_FILE), "3.0.0").unwrap();
        assert_eq!(chosen(&place), Some(place.join("versions").join("1.10.0").join(program_name())));
        std::fs::remove_file(place.join(CURRENT_FILE)).unwrap();
        assert_eq!(chosen(&place), Some(place.join("versions").join("1.10.0").join(program_name())));
        let empty = home("empty", &[], None);
        assert_eq!(chosen(&empty), None);
        std::fs::remove_dir_all(place).unwrap();
        std::fs::remove_dir_all(empty).unwrap();
    }
}
