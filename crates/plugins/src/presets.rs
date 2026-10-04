use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

pub const FOLDER_VARIABLE: &str = "LOUPE_PRESETS";
const EXTENSION: &str = "lpreset";
const NOT_IN_FILE_NAMES: [char; 9] = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

static ROOT: RwLock<Option<PathBuf>> = RwLock::new(None);

pub fn keep_in(root: PathBuf) {
    if let Ok(mut held) = ROOT.write() {
        *held = Some(root);
    }
}

pub fn root() -> Option<PathBuf> {
    ROOT.read().ok().and_then(|held| held.clone())
}

pub fn safe(name: &str) -> String {
    name.trim().trim_end_matches('.').chars().filter(|c| !NOT_IN_FILE_NAMES.contains(c) && !c.is_control()).collect()
}

pub fn folder_for(root: &Path, plugin: &str) -> PathBuf {
    let named = safe(plugin);
    root.join(if named.is_empty() { "Plugin".to_string() } else { named })
}

pub fn list(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == EXTENSION))
        .filter_map(|path| Some(path.file_stem()?.to_string_lossy().to_string()))
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names
}

pub fn save(folder: &Path, name: &str, state: &[u8]) -> Result<String, String> {
    let name = safe(name);
    if name.is_empty() {
        return Err("give the preset a name first".into());
    }
    let file = folder.join(format!("{name}.{EXTENSION}"));
    fs::create_dir_all(folder).and_then(|_| fs::write(&file, state)).map_err(|why| format!("{}: {why}", file.display()))?;
    Ok(name)
}

pub fn load(folder: &Path, name: &str) -> Result<Vec<u8>, String> {
    let file = folder.join(format!("{}.{EXTENSION}", safe(name)));
    fs::read(&file).map_err(|why| format!("{}: {why}", file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_kept_per_plugin_and_come_back_byte_for_byte() {
        let root = std::env::temp_dir().join(format!("loupe-presets-{}", std::process::id()));
        let q = folder_for(&root, "FabFilter Pro-Q 4");
        let c = folder_for(&root, "FabFilter Pro-C 2");
        assert_eq!(save(&q, "Vocal: air", &[0, 1, 2, 255]).unwrap(), "Vocal air");
        save(&q, "bass cut", &[9]).unwrap();
        save(&c, "Glue", &[7]).unwrap();
        assert_eq!(list(&q), vec!["bass cut", "Vocal air"]);
        assert_eq!(list(&c), vec!["Glue"]);
        assert_eq!(load(&q, "Vocal air").unwrap(), vec![0, 1, 2, 255]);
        assert!(save(&q, "  ", &[1]).is_err());
        assert!(load(&q, "missing").is_err());
        fs::remove_dir_all(&root).unwrap();
    }
}
