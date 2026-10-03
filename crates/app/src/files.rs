use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use iced::futures::channel::oneshot;
use iced::Task;
use loupe_engine::{Project, SavedProject, Source, TrackId};

use crate::{settings, App, Message, Overlay, Pending, Screen};

pub const EXTENSION: &str = "lp";

#[derive(Debug, Clone)]
pub struct Opened {
    saved: SavedProject,
    sources: Vec<Arc<Source>>,
    not_found: Vec<String>,
}

impl App {
    pub(crate) fn title(&self) -> String {
        if self.screen == Screen::Home {
            return "Loupe".to_string();
        }
        let unsaved = if self.dirty { " •" } else { "" };
        match self.path.as_deref().and_then(Path::file_stem) {
            Some(name) => format!("{}{unsaved} — Loupe", name.to_string_lossy()),
            None => format!("Loupe{unsaved}"),
        }
    }

    fn dialog_folder(&self) -> PathBuf {
        let beside_the_song = self.path.as_deref().and_then(Path::parent).map(Path::to_path_buf);
        beside_the_song.unwrap_or_else(|| crate::settings::projects_folder(self.folder.as_deref()))
    }

    pub(crate) fn save(&mut self) -> Task<Message> {
        match self.path.clone() {
            Some(path) => {
                self.write_to(path);
                Task::none()
            }
            None => self.save_as(),
        }
    }

    pub(crate) fn save_as(&mut self) -> Task<Message> {
        let suggested = self
            .path
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("Untitled.{EXTENSION}"));
        let start_in = self.dialog_folder();
        Task::perform(
            async move {
                rfd::AsyncFileDialog::new()
                    .add_filter("Loupe project", &[EXTENSION])
                    .set_title("Save project")
                    .set_directory(start_in)
                    .set_file_name(suggested)
                    .save_file()
                    .await
                    .map(|file| file.path().to_path_buf())
            },
            Message::SavePicked,
        )
    }

    pub(crate) fn write_to(&mut self, mut path: PathBuf) {
        if path.extension().is_none() {
            path.set_extension(EXTENSION);
        }
        let saved = SavedProject::capture(&self.project, |track| self.heights.get(&track).copied());
        match std::fs::write(&path, saved.to_text()) {
            Ok(()) => {
                settings::remember(&path);
                self.path = Some(path);
                self.dirty = false;
                self.problem = None;
            }
            Err(why) => self.problem = Some(format!("Could not save {}: {why}", path.display())),
        }
    }

    pub(crate) fn ask_to_open(&mut self) -> Task<Message> {
        if self.dirty && !self.project.tracks.is_empty() {
            self.overlay = Overlay::ConfirmDiscard(Pending::Open);
            return Task::none();
        }
        self.pick_project()
    }

    pub(crate) fn pick_project(&mut self) -> Task<Message> {
        self.overlay = Overlay::None;
        let start_in = self.dialog_folder();
        Task::perform(
            async move {
                rfd::AsyncFileDialog::new()
                    .add_filter("Loupe project", &[EXTENSION])
                    .set_title("Open project")
                    .set_directory(start_in)
                    .pick_file()
                    .await
                    .map(|file| file.path().to_path_buf())
            },
            Message::ProjectPicked,
        )
    }

    pub(crate) fn read_project(&mut self, path: PathBuf, as_template: bool) -> Task<Message> {
        let rate = self.project.rate;
        self.loading += 1;
        self.problem = None;
        let (done, opened) = oneshot::channel();
        let file = path.clone();
        std::thread::spawn(move || {
            let _ = done.send(read_file(&file, rate));
        });
        Task::perform(
            async move { opened.await.unwrap_or_else(|_| Err("opening stopped unexpectedly".into())) },
            move |result| Message::ProjectRead(path.clone(), as_template, result),
        )
    }

    pub(crate) fn refresh_home(&mut self) {
        self.templates = settings::templates(self.folder.as_deref());
        self.recent = settings::recent();
    }

    pub(crate) fn go_home(&mut self) {
        self.replace_project(Project::new(self.project.rate), HashMap::new());
        self.overlay = Overlay::None;
        self.screen = Screen::Home;
        self.refresh_home();
    }

    pub(crate) fn save_template(&mut self, typed: &str) {
        let name: String = typed.trim().chars().filter(|c| !matches!(c, '/' | '\\' | ':')).collect();
        if name.is_empty() {
            return;
        }
        self.overlay = Overlay::None;
        let folder = settings::templates_folder(self.folder.as_deref());
        let file = folder.join(format!("{name}.{EXTENSION}"));
        let saved = SavedProject::capture(&self.project, |track| self.heights.get(&track).copied());
        let written = std::fs::create_dir_all(&folder).and_then(|()| std::fs::write(&file, saved.to_text()));
        self.problem = written.err().map(|why| format!("Could not save the template {}: {why}", file.display()));
    }

    pub(crate) fn replace_project(&mut self, project: Project, heights: HashMap<TrackId, f32>) {
        self.project = project;
        self.heights = heights;
        self.undo.clear();
        self.redo.clear();
        self.run = None;
        self.selected = None;
        self.bpm = crate::format_bpm(self.project.bpm);
        self.set_loop(None);
        if self.playing {
            self.engine.stop();
            self.playing = false;
        }
        self.changed();
        self.seek(0);
        self.show_whole_song();
        self.path = None;
        self.dirty = false;
    }

    pub(crate) fn adopt(&mut self, path: PathBuf, opened: Opened, as_template: bool) {
        let (project, heights) = opened.saved.build(&opened.sources, self.project.rate);
        self.replace_project(project, heights.into_iter().collect());
        self.screen = Screen::Song;
        if !as_template {
            settings::remember(&path);
            self.path = Some(path);
        }
        self.problem = match opened.not_found.as_slice() {
            [] => None,
            [one] => Some(format!("Audio not found: {one}")),
            many => Some(format!("{} audio files not found, first: {}", many.len(), many[0])),
        };
    }
}

fn read_file(path: &Path, rate: u32) -> Result<Opened, String> {
    let text = std::fs::read_to_string(path).map_err(|why| why.to_string())?;
    let saved = SavedProject::parse(&text)?;
    let loaded: Vec<Result<Source, String>> = std::thread::scope(|scope| {
        let loading: Vec<_> =
            saved.sources.iter().map(|audio| scope.spawn(move || Source::load(audio, rate))).collect();
        loading
            .into_iter()
            .map(|thread| thread.join().unwrap_or_else(|_| Err("the audio could not be read".into())))
            .collect()
    });
    let mut not_found = Vec::new();
    let sources = loaded
        .into_iter()
        .zip(&saved.sources)
        .map(|(source, audio)| {
            Arc::new(source.unwrap_or_else(|_| {
                not_found.push(audio.display().to_string());
                Source::missing(audio)
            }))
        })
        .collect();
    Ok(Opened { saved, sources, not_found })
}
