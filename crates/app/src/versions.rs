use std::path::{Path, PathBuf};

use iced::futures::channel::oneshot;
use iced::widget::{button, column, row, text, Space};
use iced::{Alignment, Element, Length, Task};

use crate::{App, Message};

pub const THIS: &str = env!("CARGO_PKG_VERSION");
const CURRENT_FILE: &str = "current";
const VERSIONS_FOLDER: &str = "versions";
const LAUNCHER: &str = if cfg!(windows) { "Loupe.exe" } else { "Loupe" };
const RELEASES: &str = "https://api.github.com/repos/adevme/loupe/releases/latest";
const SETUP_PREFIX: &str = "loupe-setup-";

pub fn parts(version: &str) -> Option<[u64; 3]> {
    let mut pieces = version.trim().trim_start_matches('v').split('.').map(|piece| piece.parse::<u64>().ok());
    let parsed = [pieces.next()??, pieces.next().flatten().unwrap_or(0), pieces.next().flatten().unwrap_or(0)];
    pieces.next().is_none().then_some(parsed)
}

pub fn newer_than_this(version: &str) -> bool {
    match (parts(version), parts(THIS)) {
        (Some(other), Some(mine)) => other > mine,
        (None, _) => false,
        (_, None) => false,
    }
}

pub fn install_home() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let version_folder = exe.parent()?;
    let versions = version_folder.parent()?;
    (versions.file_name()? == VERSIONS_FOLDER).then(|| versions.parent().map(Path::to_path_buf)).flatten()
}

pub fn installed(home: &Path) -> Vec<String> {
    let program = if cfg!(windows) { "loupe.exe" } else { "loupe" };
    let mut found: Vec<String> = std::fs::read_dir(home.join(VERSIONS_FOLDER))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().join(program).is_file())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| parts(name).is_some())
        .collect();
    found.sort_by_key(|name| std::cmp::Reverse(parts(name)));
    found
}

#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    pub version: String,
    pub notes: String,
    pub setup: Option<Setup>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Setup {
    pub name: String,
    pub url: String,
    pub sha256: Option<String>,
}

fn latest_release() -> Result<Option<Update>, String> {
    let reply = ureq::get(RELEASES)
        .set("Accept", "application/vnd.github+json")
        .set("User-Agent", "Loupe")
        .call()
        .map_err(|why| match why {
            ureq::Error::Status(404, _) => "no releases are published yet".to_string(),
            ureq::Error::Status(code, _) => format!("GitHub answered {code}"),
            other => other.to_string(),
        })?;
    let body = reply.into_string().map_err(|why| why.to_string())?;
    let field = |key: &str| json_text(&body, key);
    let Some(version) = field("tag_name").map(|tag| tag.trim_start_matches('v').to_string()) else {
        return Err("the release has no version".to_string());
    };
    if !newer_than_this(&version) {
        return Ok(None);
    }
    let setup = assets(&body).into_iter().find(|setup| setup.name.starts_with(SETUP_PREFIX) && setup.name.ends_with(".exe"));
    Ok(Some(Update { version, notes: field("body").unwrap_or_default(), setup }))
}

fn json_text(body: &str, key: &str) -> Option<String> {
    let start = body.find(&format!("\"{key}\""))?;
    let after = &body[start + key.len() + 2..];
    let quote = after.find('"')?;
    let mut out = String::new();
    let mut chars = after[quote + 1..].chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                'r' => {}
                't' => out.push('\t'),
                'u' => {
                    let code: String = chars.by_ref().take(4).collect();
                    out.push(u32::from_str_radix(&code, 16).ok().and_then(char::from_u32).unwrap_or('?'));
                }
                other => out.push(other),
            },
            other => out.push(other),
        }
    }
    None
}

fn assets(body: &str) -> Vec<Setup> {
    let Some(list) = body.find("\"assets\"").map(|at| &body[at..]) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut rest = list;
    while let Some(at) = rest.find("\"browser_download_url\"") {
        let piece = &rest[..at + 1];
        let name = piece.rfind("\"name\"").and_then(|n| json_text(&piece[n..], "name"));
        let sha256 = piece
            .rfind("\"digest\"")
            .and_then(|n| json_text(&piece[n..], "digest"))
            .and_then(|digest| digest.strip_prefix("sha256:").map(str::to_lowercase));
        let url = json_text(&rest[at..], "browser_download_url");
        if let (Some(name), Some(url)) = (name, url) {
            found.push(Setup { name, url, sha256 });
        }
        rest = &rest[at + 1..];
    }
    found
}

fn safe_name(given: &str) -> String {
    let kept: String = given
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .collect();
    let kept = kept.trim_start_matches('.');
    if kept.is_empty() {
        "loupe-setup.exe".to_string()
    } else {
        kept.to_string()
    }
}

fn fetch_setup(setup: &Setup) -> Result<PathBuf, String> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    let Some(expected) = setup.sha256.as_deref() else {
        return Err("GitHub gave no checksum for the installer, so it was not downloaded".to_string());
    };
    let reply = ureq::get(&setup.url).set("User-Agent", "Loupe").call().map_err(|why| why.to_string())?;
    let file = std::env::temp_dir().join(safe_name(&setup.name));
    let mut out = std::fs::File::create(&file).map_err(|why| why.to_string())?;
    let mut reader = reply.into_reader();
    let mut hasher = Sha256::new();
    let mut chunk = vec![0u8; 1 << 16];
    loop {
        let read = reader.read(&mut chunk).map_err(|why| why.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&chunk[..read]);
        out.write_all(&chunk[..read]).map_err(|why| why.to_string())?;
    }
    drop(out);
    let got: String = hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect();
    if got != expected {
        let _ = std::fs::remove_file(&file);
        return Err("the download did not match GitHub's checksum, so it was deleted and not run".to_string());
    }
    Ok(file)
}

impl App {
    pub(crate) fn check_for_updates(&mut self) -> Task<Message> {
        self.update_state = UpdateState::Checking;
        let (done, answer) = oneshot::channel();
        std::thread::spawn(move || {
            let _ = done.send(latest_release());
        });
        Task::perform(async move { answer.await.unwrap_or_else(|_| Err("the check stopped".to_string())) }, Message::UpdateChecked)
    }

    pub(crate) fn install_update(&mut self) -> Task<Message> {
        if !matches!(self.update_state, UpdateState::Found(_)) {
            return Task::none();
        }
        if self.dirty && !self.project.tracks.is_empty() {
            match self.path.clone() {
                Some(song) => self.write_to(song),
                None => {
                    let asked = self.save_as();
                    self.entry_problem = Some("Save your song first, then press Update again.".to_string());
                    return asked;
                }
            }
            if self.dirty {
                return Task::none();
            }
        }
        let UpdateState::Found(update) = &self.update_state else {
            return Task::none();
        };
        let Some(setup) = update.setup.clone() else {
            return Task::none();
        };
        self.update_state = UpdateState::Downloading;
        let (done, answer) = oneshot::channel();
        std::thread::spawn(move || {
            let _ = done.send(fetch_setup(&setup));
        });
        Task::perform(async move { answer.await.unwrap_or_else(|_| Err("the download stopped".to_string())) }, Message::UpdateDownloaded)
    }

    pub(crate) fn run_setup(&mut self, setup: PathBuf) -> Task<Message> {
        let mut setup_run = std::process::Command::new(&setup);
        setup_run.arg("/SILENT").arg("/RELAUNCH=1");
        if let Some(song) = &self.path {
            setup_run.arg(format!("/SONG={}", song.display()));
        }
        match setup_run.spawn() {
            Ok(_) => iced::exit(),
            Err(why) => {
                self.update_state = UpdateState::Failed(format!("Could not start {}: {why}", setup.display()));
                Task::none()
            }
        }
    }

    pub(crate) fn switch_version(&mut self, version: String) -> Task<Message> {
        let Some(home) = install_home() else {
            return Task::none();
        };
        if let Err(why) = std::fs::write(home.join(CURRENT_FILE), &version) {
            self.problem = Some(format!("Could not switch to Loupe {version}: {why}"));
            return Task::none();
        }
        match std::process::Command::new(home.join(LAUNCHER)).spawn() {
            Ok(_) => iced::exit(),
            Err(why) => {
                self.problem = Some(format!("Switched to Loupe {version}, but could not restart: {why}. Open Loupe again."));
                Task::none()
            }
        }
    }

    /// The updates half of the About window: what is installed, what is available,
    /// and the button that goes looking. It has no window of its own because on its
    /// own it would say nothing About does not already say.
    pub(crate) fn updates_block(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let mut body = column![].spacing(12);
        // Only worth listing when there is more than one to pick between.
        let list = install_home().map(|home| installed(&home)).unwrap_or_default();
        if list.len() > 1 {
            body = body.push(text("Installed versions. Your songs and settings stay the same whichever you use.").size(12).color(palette.text_dim));
            for version in list {
                let using = version == THIS;
                let choose = button(text(if using { "In use" } else { "Use this version" }).size(12.5).font(palette.medium))
                    .padding([5, 12])
                    .style(move |_, status| palette.outlined(status))
                    .on_press_maybe((!using).then(|| Message::UseVersion(version.clone())));
                body = body.push(row![text(format!("Loupe {version}")).size(13).width(Length::Fill), choose].align_y(Alignment::Center));
            }
        }
        let status: Element<'_, Message> = match &self.update_state {
            UpdateState::Idle => text("").size(12).into(),
            UpdateState::Checking => text("Checking for updates...").size(12).color(palette.text_dim).into(),
            UpdateState::Current => text("You have the newest Loupe.").size(12).color(palette.text_dim).into(),
            UpdateState::Downloading => text("Downloading the update...").size(12).color(palette.text_dim).into(),
            UpdateState::Failed(why) => text(why.as_str()).size(12).color(palette.danger).into(),
            UpdateState::Found(update) => {
                let mut found = column![text(format!("Loupe {} is available.", update.version)).size(13).font(palette.medium)].spacing(6);
                if !update.notes.trim().is_empty() {
                    let notes: String = update.notes.chars().take(600).collect();
                    found = found.push(text(notes).size(12).color(palette.text_dim));
                }
                let install = button(text("Download and install").size(12.5).font(palette.medium))
                    .padding([6, 14])
                    .style(move |_, status| palette.solid(status))
                    .on_press_maybe(update.setup.is_some().then_some(Message::InstallUpdate));
                found.push(install).into()
            }
        };
        let check = button(text("Check for updates").size(12.5).font(palette.medium))
            .padding([6, 14])
            .style(move |_, status| palette.outlined(status))
            .on_press_maybe((!matches!(self.update_state, UpdateState::Checking | UpdateState::Downloading)).then_some(Message::CheckForUpdates));
        body.push(row![check, Space::with_width(Length::Fill)].align_y(Alignment::Center)).push(status).into()
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum UpdateState {
    #[default]
    Idle,
    Checking,
    Current,
    Found(Update),
    Downloading,
    Failed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_cannot_be_named_out_of_the_temp_folder() {
        assert_eq!(safe_name("loupe-setup-1.2.0.exe"), "loupe-setup-1.2.0.exe");
        assert_eq!(safe_name("../../evil.exe"), "evil.exe");
        assert_eq!(safe_name("C:\\Windows\\System32\\x.exe"), "CWindowsSystem32x.exe");
        assert_eq!(safe_name("..."), "loupe-setup.exe");
        assert_eq!(safe_name(""), "loupe-setup.exe");
    }

    #[test]
    fn versions_compare_as_numbers() {
        assert!(newer_than_this("999.0.0"));
        assert!(!newer_than_this("0.0.1"));
        assert!(!newer_than_this(THIS));
        assert!(!newer_than_this("not a version"));
        assert!(parts("1.10.0") > parts("1.9.9"));
    }

    #[test]
    fn a_release_reply_gives_its_version_notes_and_installer() {
        let body = r#"{"tag_name":"v1.2.0","name":"Loupe 1.2","body":"Faster export\r\n- New EQ é","assets":[{"name":"loupe-1.2.0.AppImage","browser_download_url":"https://x/a.AppImage"},{"name":"loupe-setup-1.2.0.exe","size":5,"digest":"sha256:ABCD12","browser_download_url":"https://x/loupe-setup-1.2.0.exe"}]}"#;
        assert_eq!(json_text(body, "tag_name").as_deref(), Some("v1.2.0"));
        assert_eq!(json_text(body, "body").as_deref(), Some("Faster export\n- New EQ é"));
        let found = assets(body);
        assert_eq!(found.len(), 2);
        let setup = Setup { name: "loupe-setup-1.2.0.exe".into(), url: "https://x/loupe-setup-1.2.0.exe".into(), sha256: Some("abcd12".into()) };
        assert_eq!(found[1], setup);
        assert_eq!(found[0].sha256, None);
    }

    #[test]
    fn only_a_copy_inside_a_versions_folder_counts_as_installed() {
        assert!(install_home().is_none());
    }
}

impl App {
    /// Looking for a new version is a trip to the Loupe server, so the choice sits
    /// beside the other settings about what Loupe sends and fetches.
    pub(crate) fn update_settings(&self) -> Element<'_, Message> {
        let palette = self.palette;
        column![
            text("Updates").size(13).font(palette.medium),
            text("Loupe asks the Loupe server whether a newer version is out. It never downloads one without you pressing Update.")
                .size(12)
                .color(palette.text_dim),
            iced::widget::checkbox("Check for updates when Loupe starts", self.check_updates)
                .on_toggle(Message::CheckUpdatesOnStart)
                .text_size(13),
        ]
        .spacing(8)
        .into()
    }
}
