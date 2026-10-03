use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use crate::settings::{config_dir, home_folder};

const BACKUP_FOLDER: &str = "Backup";
const UNSAVED_FOLDER: &str = "Autosave";
const UNSAVED_NAME: &str = "Untitled";
const RUNNING_PREFIX: &str = "running-";
const UNFINISHED: &str = "writing";
const EXTENSION: &str = "lp";

#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub folder: PathBuf,
    pub name: String,
    pub project: Option<PathBuf>,
}

impl Place {
    pub fn of(project: Option<&Path>, loupe_folder: Option<&Path>) -> Self {
        match project {
            Some(file) => Self {
                folder: file.parent().unwrap_or(Path::new(".")).join(BACKUP_FOLDER),
                name: file.file_stem().map_or(UNSAVED_NAME.to_string(), |stem| stem.to_string_lossy().into_owned()),
                project: Some(file.to_path_buf()),
            },
            None => Self { folder: home_folder(loupe_folder).join(UNSAVED_FOLDER), name: UNSAVED_NAME.to_string(), project: None },
        }
    }

    pub fn same_as(&self, project: Option<&Path>) -> bool {
        self.project.as_deref() == project
    }

    fn belongs(&self, file: &Path) -> bool {
        let Some(stem) = file.file_stem().map(|stem| stem.to_string_lossy().into_owned()) else {
            return false;
        };
        file.extension().is_some_and(|extension| extension == EXTENSION)
            && stem.strip_prefix(&self.name).and_then(|rest| rest.strip_prefix(' ')).is_some_and(stamped)
    }
}

fn stamped(rest: &str) -> bool {
    let shape = "0000-00-00 00-00-00";
    rest.len() == shape.len()
        && rest.chars().zip(shape.chars()).all(|(seen, want)| if want == '0' { seen.is_ascii_digit() } else { seen == want })
}

pub fn stamp(now: chrono::DateTime<chrono::Local>) -> String {
    now.format("%Y-%m-%d %H-%M-%S").to_string()
}

pub fn write(place: &Place, text: &str, keep: usize) -> io::Result<PathBuf> {
    fs::create_dir_all(&place.folder)?;
    let file = place.folder.join(format!("{} {}.{EXTENSION}", place.name, stamp(chrono::Local::now())));
    let unfinished = file.with_extension(UNFINISHED);
    fs::write(&unfinished, text)?;
    fs::rename(&unfinished, &file)?;
    prune(place, keep);
    Ok(file)
}

fn prune(place: &Place, keep: usize) {
    let mut kept = backups(place);
    while kept.len() > keep.max(1) {
        let oldest = kept.remove(0);
        let _ = fs::remove_file(oldest);
    }
}

pub fn backups(place: &Place) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(&place.folder)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|file| place.belongs(file))
        .collect();
    found.sort();
    found
}

pub fn newest(place: &Place) -> Option<PathBuf> {
    backups(place).pop()
}

static LATEST: Mutex<Option<(Place, String, usize)>> = Mutex::new(None);

pub fn remember(place: Place, text: String, keep: usize) {
    if let Ok(mut latest) = LATEST.lock() {
        *latest = Some((place, text, keep));
    }
}

pub fn last_chance() {
    let latest = LATEST.lock().unwrap_or_else(|held| held.into_inner());
    if let Some((place, text, keep)) = latest.as_ref() {
        let _ = write(place, text, *keep + 1);
    }
}

fn running_file(id: u32) -> Option<PathBuf> {
    config_dir().map(|dir| dir.join(format!("{RUNNING_PREFIX}{id}")))
}

pub fn mark_running(place: &Place) {
    let Some(file) = running_file(std::process::id()) else {
        return;
    };
    if let Some(folder) = file.parent() {
        let _ = fs::create_dir_all(folder);
    }
    let project = place.project.as_ref().map_or("-".to_string(), |path| path.display().to_string());
    let _ = fs::write(file, format!("folder {}\nname {}\nproject {project}\n", place.folder.display(), place.name));
}

pub fn mark_closed() {
    if let Some(file) = running_file(std::process::id()) {
        let _ = fs::remove_file(file);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Lost {
    pub marker: PathBuf,
    pub place: Place,
    pub backup: PathBuf,
    pub saved_at: SystemTime,
}

pub fn read_marker(text: &str) -> Option<Place> {
    let field = |key: &str| text.lines().find_map(|line| line.strip_prefix(key).and_then(|rest| rest.strip_prefix(' ')));
    let folder = PathBuf::from(field("folder")?);
    let name = field("name")?.to_string();
    let project = field("project").filter(|value| *value != "-").map(PathBuf::from);
    (!name.is_empty()).then_some(Place { folder, name, project })
}

pub fn find_lost() -> Vec<Lost> {
    let Some(dir) = config_dir() else {
        return Vec::new();
    };
    let mine = std::process::id();
    let mut lost = Vec::new();
    for entry in fs::read_dir(&dir).into_iter().flatten().filter_map(Result::ok) {
        let marker = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_prefix(RUNNING_PREFIX).and_then(|id| id.parse::<u32>().ok()) else {
            continue;
        };
        if id == mine || still_running(id) {
            continue;
        }
        let place = fs::read_to_string(&marker).ok().and_then(|text| read_marker(&text));
        match place.and_then(|place| newest(&place).map(|backup| (place, backup))) {
            Some((place, backup)) => {
                let saved_at = fs::metadata(&backup).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
                lost.push(Lost { marker, place, backup, saved_at });
            }
            None => {
                let _ = fs::remove_file(&marker);
            }
        }
    }
    lost.sort_by_key(|found| found.saved_at);
    lost
}

pub fn forget(lost: &Lost) {
    let _ = fs::remove_file(&lost.marker);
}

pub fn when(saved_at: SystemTime) -> String {
    let local: chrono::DateTime<chrono::Local> = saved_at.into();
    local.format("%H:%M on %-d %B").to_string()
}

#[cfg(target_os = "linux")]
fn still_running(id: u32) -> bool {
    Path::new(&format!("/proc/{id}")).exists()
}

#[cfg(windows)]
fn still_running(id: u32) -> bool {
    windows::still_running(id)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn still_running(id: u32) -> bool {
    std::process::Command::new("kill").args(["-0", &id.to_string()]).status().is_ok_and(|status| status.success())
}

#[cfg(windows)]
mod windows {
    const QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;

    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, id: u32) -> isize;
        fn GetExitCodeProcess(process: isize, code: *mut u32) -> i32;
        fn CloseHandle(handle: isize) -> i32;
    }

    pub fn still_running(id: u32) -> bool {
        unsafe {
            let process = OpenProcess(QUERY_LIMITED_INFORMATION, 0, id);
            if process == 0 {
                return false;
            }
            let mut code = 0u32;
            let known = GetExitCodeProcess(process, &mut code) != 0;
            CloseHandle(process);
            known && code == STILL_ACTIVE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("loupe-backup-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        folder
    }

    #[test]
    fn a_saved_song_backs_up_beside_itself_and_an_unsaved_one_in_the_loupe_folder() {
        let saved = Place::of(Some(Path::new("/music/Song/Song.lp")), None);
        assert_eq!(saved.folder, PathBuf::from("/music/Song/Backup"));
        assert_eq!(saved.name, "Song");
        let unsaved = Place::of(None, Some(Path::new("/d")));
        assert_eq!(unsaved.folder, PathBuf::from("/d/Loupe/Autosave"));
        assert_eq!(unsaved.name, "Untitled");
    }

    #[test]
    fn only_the_newest_backups_are_kept_and_other_files_are_left_alone() {
        let folder = scratch("keep");
        let place = Place { folder: folder.clone(), name: "Song".into(), project: None };
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("Song notes.txt"), "mine").unwrap();
        fs::write(folder.join("Song.lp"), "mine").unwrap();
        fs::write(folder.join("Other 2026-10-01 10-00-00.lp"), "mine").unwrap();
        for (n, stamp) in ["2026-10-01 10-00-00", "2026-10-01 11-00-00", "2026-10-02 09-30-00", "2026-10-02 09-30-01"].iter().enumerate() {
            fs::write(folder.join(format!("Song {stamp}.lp")), format!("{n}")).unwrap();
        }
        let written = write(&place, "newest", 3).unwrap();
        let left = backups(&place);
        assert_eq!(left.len(), 3);
        assert_eq!(left.last(), Some(&written));
        assert_eq!(fs::read_to_string(&written).unwrap(), "newest");
        assert!(left[0].ends_with("Song 2026-10-02 09-30-00.lp"));
        for untouched in ["Song notes.txt", "Song.lp", "Other 2026-10-01 10-00-00.lp"] {
            assert!(folder.join(untouched).exists(), "{untouched} was removed");
        }
        assert!(!fs::read_dir(&folder).unwrap().any(|e| e.unwrap().path().extension().is_some_and(|x| x == UNFINISHED)));
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_marker_reads_back_into_its_place() {
        let text = "folder /music/Song/Backup\nname Song Two\nproject /music/Song/Song Two.lp\n";
        let place = read_marker(text).unwrap();
        assert_eq!(place.folder, PathBuf::from("/music/Song/Backup"));
        assert_eq!(place.name, "Song Two");
        assert_eq!(place.project, Some(PathBuf::from("/music/Song/Song Two.lp")));
        assert_eq!(read_marker("folder /x\nname Untitled\nproject -\n").unwrap().project, None);
        assert!(read_marker("nonsense").is_none());
    }

    #[test]
    fn only_real_time_stamps_count_as_backups() {
        let place = Place { folder: PathBuf::from("/x"), name: "Song".into(), project: None };
        assert!(place.belongs(Path::new("/x/Song 2026-10-03 18-04-05.lp")));
        assert!(!place.belongs(Path::new("/x/Song a-sketch-i-made-here.lp")));
        assert!(!place.belongs(Path::new("/x/Song 2026-10-03 18-04-05.wav")));
        assert!(!place.belongs(Path::new("/x/Other 2026-10-03 18-04-05.lp")));
    }

    #[test]
    fn this_process_counts_as_running_and_a_made_up_one_does_not() {
        assert!(still_running(std::process::id()));
        assert!(!still_running(u32::MAX - 7));
    }
}
