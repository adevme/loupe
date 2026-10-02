#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod icons;
mod theme;
mod timeline;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use iced::futures::channel::oneshot;
use iced::widget::canvas::Cache;
use iced::widget::{
    button, canvas, column, container, horizontal_space, row, slider, text, text_input, Space,
};
use iced::{keyboard, window, Alignment, Element, Length, Size, Subscription, Task};
use loupe_engine::{
    ClipId, Command, CommandError, Engine, Frames, Outcome, Output, Project, Source, TrackId,
};

use timeline::{Timeline, View};

const AUDIO_TYPES: [&str; 8] = ["wav", "mp3", "flac", "m4a", "aac", "ogg", "aif", "aiff"];
const MIN_GAIN_DB: f32 = -24.0;
const MAX_GAIN_DB: f32 = 12.0;
const UNDO_STEPS: usize = 200;
const SETTLE_TICKS: u8 = 6;

fn main() -> iced::Result {
    iced::application("Loupe", App::update, App::view)
        .subscription(App::subscription)
        .theme(|_| theme::theme())
        .font(include_bytes!("../assets/Inter-Regular.ttf").as_slice())
        .font(include_bytes!("../assets/Inter-Medium.ttf").as_slice())
        .font(include_bytes!("../assets/Inter-SemiBold.ttf").as_slice())
        .font(include_bytes!("../assets/JetBrainsMono-Regular.ttf").as_slice())
        .font(include_bytes!("../assets/JetBrainsMono-Medium.ttf").as_slice())
        .font(include_bytes!("../assets/lucide.ttf").as_slice())
        .default_font(theme::INTER)
        .antialiasing(true)
        .window(window::Settings {
            size: START_SIZE,
            min_size: Some(Size::new(820.0, 420.0)),
            ..window::Settings::default()
        })
        .run_with(App::new)
}

const START_SIZE: Size = Size::new(1280.0, 760.0);

#[derive(Debug, Clone)]
pub enum Message {
    TogglePlay,
    ToStart,
    Seek(Frames),
    Tick,
    Import,
    Picked(Vec<PathBuf>),
    Dropped(PathBuf),
    Loaded(PathBuf, Result<Arc<Source>, String>),
    Select(Option<ClipId>),
    LaneClicked(Frames),
    MoveClip { clip: ClipId, track: TrackId, start: Frames },
    DragEnd,
    Split,
    Delete,
    Undo,
    Redo,
    ClipGain(f32),
    ClipGainDone,
    AddTrack,
    RemoveTrack(TrackId),
    ToggleMute(TrackId),
    BpmTyped(String),
    BpmEntered,
    SetView(View),
    Resized(Size),
}

#[derive(Clone, Copy, PartialEq)]
enum Run {
    Move(ClipId),
    Gain(ClipId),
}

struct App {
    engine: Engine,
    project: Project,
    undo: Vec<Project>,
    redo: Vec<Project>,
    run: Option<Run>,
    selected: Option<ClipId>,
    playing: bool,
    settle: u8,
    playhead: Frames,
    view: View,
    window: Size,
    bpm: String,
    loading: usize,
    problem: Option<String>,
    cache: Cache,
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let silent = std::env::var("LOUPE_AUDIO").as_deref() == Ok("silent");
        let mut engine = Engine::start(if silent { Output::Silent } else { Output::Device });
        let project = Project::new(engine.rate());
        engine.set_project(&project);
        let mut app = Self {
            problem: engine.output_error().map(|e| format!("No sound: {e}")),
            bpm: format_bpm(project.bpm),
            engine,
            project,
            undo: Vec::new(),
            redo: Vec::new(),
            run: None,
            selected: None,
            playing: false,
            settle: 0,
            playhead: 0,
            view: View { zoom: 100.0, scroll: 0.0, scroll_y: 0.0 },
            window: START_SIZE,
            loading: 0,
            cache: Cache::new(),
        };
        let task = app.import(std::env::args_os().skip(1).map(PathBuf::from).collect());
        (app, task)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::TogglePlay => {
                if self.playing {
                    self.engine.stop();
                    self.playing = false;
                    self.settle = SETTLE_TICKS;
                } else {
                    self.engine.play();
                    self.playing = true;
                }
            }
            Message::ToStart => self.seek(0),
            Message::Seek(to) => self.seek(to),
            Message::Tick => {
                self.engine.collect();
                self.settle = self.settle.saturating_sub(1);
                if self.playing && !self.engine.is_playing() {
                    self.playing = false;
                    self.settle = SETTLE_TICKS;
                }
                let position = self.engine.position();
                if position != self.playhead {
                    self.playhead = position;
                    self.follow();
                }
            }
            Message::Import => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .add_filter("Audio", &AUDIO_TYPES)
                            .set_title("Import audio")
                            .pick_files()
                            .await
                            .map(|files| files.iter().map(|f| f.path().to_path_buf()).collect())
                            .unwrap_or_default()
                    },
                    Message::Picked,
                );
            }
            Message::Picked(paths) => return self.import(paths),
            Message::Dropped(path) => return self.import(vec![path]),
            Message::Loaded(path, result) => {
                self.loading = self.loading.saturating_sub(1);
                match result {
                    Ok(source) => self.place(source),
                    Err(why) => {
                        let name = path.file_name().unwrap_or_default().to_string_lossy();
                        self.problem = Some(format!("Could not import {name}: {why}"));
                    }
                }
            }
            Message::Select(clip) => {
                self.selected = clip;
                self.cache.clear();
            }
            Message::LaneClicked(at) => {
                self.selected = None;
                self.seek(at);
            }
            Message::MoveClip { clip, track, start } => {
                self.edit(Some(Run::Move(clip)), Command::MoveClip { clip, track, start });
            }
            Message::DragEnd | Message::ClipGainDone => self.run = None,
            Message::Split => self.split(),
            Message::Delete => {
                if let Some(clip) = self.selected.take() {
                    self.edit(None, Command::DeleteClip(clip));
                }
            }
            Message::Undo => {
                if let Some(earlier) = self.undo.pop() {
                    self.redo.push(std::mem::replace(&mut self.project, earlier));
                    self.restored();
                }
            }
            Message::Redo => {
                if let Some(later) = self.redo.pop() {
                    self.undo.push(std::mem::replace(&mut self.project, later));
                    self.restored();
                }
            }
            Message::ClipGain(db) => {
                if let Some(clip) = self.selected {
                    let gain = 10f32.powf(db / 20.0);
                    self.edit(Some(Run::Gain(clip)), Command::SetClipGain { clip, gain });
                }
            }
            Message::AddTrack => {
                let name = format!("Track {}", self.project.tracks.len() + 1);
                self.edit(None, Command::AddTrack { name });
            }
            Message::RemoveTrack(track) => {
                let holds_selection = self
                    .selected
                    .and_then(|clip| self.project.track_of(clip))
                    .is_some_and(|t| t.id == track);
                if holds_selection {
                    self.selected = None;
                }
                self.edit(None, Command::RemoveTrack(track));
            }
            Message::ToggleMute(track) => {
                if let Some(muted) = self.project.track(track).map(|t| !t.muted) {
                    self.edit(None, Command::SetTrackMuted { track, muted });
                }
            }
            Message::BpmTyped(typed) => {
                if typed.len() <= 6 && typed.chars().all(|c| c.is_ascii_digit() || c == '.') {
                    self.bpm = typed;
                }
            }
            Message::BpmEntered => {
                if let Ok(bpm) = self.bpm.parse::<f64>() {
                    self.edit(None, Command::SetBpm(bpm));
                }
                self.bpm = format_bpm(self.project.bpm);
                return iced::widget::focus_next();
            }
            Message::SetView(view) => {
                self.view = view;
                self.cache.clear();
            }
            Message::Resized(size) => self.window = size,
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        let shortcuts = keyboard::on_key_press(shortcut);
        let window = iced::event::listen_with(|event, _status, _window| match event {
            iced::Event::Window(window::Event::FileDropped(path)) => Some(Message::Dropped(path)),
            iced::Event::Window(window::Event::Resized(size)) => Some(Message::Resized(size)),
            _ => None,
        });
        let ticks = if self.playing || self.settle > 0 {
            iced::time::every(Duration::from_millis(16)).map(|_| Message::Tick)
        } else {
            Subscription::none()
        };
        Subscription::batch([shortcuts, window, ticks])
    }

    fn edit(&mut self, run: Option<Run>, command: Command) -> Option<Outcome> {
        self.transact(run, |project| project.apply(command))
    }

    fn transact(
        &mut self,
        run: Option<Run>,
        change: impl FnOnce(&mut Project) -> Result<Outcome, CommandError>,
    ) -> Option<Outcome> {
        let before = self.project.clone();
        match change(&mut self.project) {
            Ok(outcome) => {
                if run.is_none() || run != self.run {
                    self.undo.push(before);
                    if self.undo.len() > UNDO_STEPS {
                        self.undo.remove(0);
                    }
                }
                self.redo.clear();
                self.run = run;
                self.problem = None;
                self.changed();
                Some(outcome)
            }
            Err(_) => {
                self.project = before;
                None
            }
        }
    }

    fn changed(&mut self) {
        self.engine.set_project(&self.project);
        self.cache.clear();
    }

    fn restored(&mut self) {
        self.run = None;
        if self.selected.is_some_and(|clip| self.project.clip(clip).is_none()) {
            self.selected = None;
        }
        self.bpm = format_bpm(self.project.bpm);
        self.changed();
    }

    fn seek(&mut self, to: Frames) {
        self.engine.seek(to);
        self.playhead = to;
        self.cache.clear();
    }

    fn import(&mut self, paths: Vec<PathBuf>) -> Task<Message> {
        let rate = self.project.rate;
        self.loading += paths.len();
        if !paths.is_empty() {
            self.problem = None;
        }
        Task::batch(paths.into_iter().map(|path| {
            let (done, loaded) = oneshot::channel();
            let file = path.clone();
            std::thread::spawn(move || {
                let _ = done.send(Source::load(&file, rate).map(Arc::new));
            });
            Task::perform(
                async move {
                    loaded.await.unwrap_or_else(|_| Err("the import stopped unexpectedly".into()))
                },
                move |result| Message::Loaded(path.clone(), result),
            )
        }))
    }

    fn place(&mut self, source: Arc<Source>) {
        let first = self.project.tracks.is_empty();
        let start = self.playhead;
        let seconds = source.frames.len() as f64 / self.project.rate as f64;
        let name = source.name.clone();
        let placed = self.transact(None, |project| {
            let Outcome::Track(track) = project.apply(Command::AddTrack { name })? else {
                return Err(CommandError::NoSuchTrack);
            };
            project.apply(Command::AddClip { track, source, start })
        });
        if let Some(Outcome::Clip(clip)) = placed {
            self.selected = Some(clip);
            if first {
                let room = (self.window.width - timeline::HEADER_W - 48.0).max(100.0) as f64;
                self.view.zoom = (room / seconds.max(0.01))
                    .clamp(timeline::MIN_ZOOM, View::max_zoom(self.project.rate));
                self.view.scroll = 0.0;
            }
        }
    }

    fn split_targets(&self) -> Vec<ClipId> {
        let under = |clip: &&loupe_engine::Clip| self.playhead > clip.start && self.playhead < clip.end();
        match self.selected {
            Some(id) => self.project.clip(id).filter(under).map(|c| c.id).into_iter().collect(),
            None => self.project.clips().filter(under).map(|c| c.id).collect(),
        }
    }

    fn split(&mut self) {
        let targets = self.split_targets();
        if targets.is_empty() {
            return;
        }
        let at = self.playhead;
        let reselect = self.selected.is_some();
        let made = self.transact(None, |project| {
            let mut last = Outcome::Done;
            for clip in targets {
                last = project.apply(Command::SplitClip { clip, at })?;
            }
            Ok(last)
        });
        if let (true, Some(Outcome::Clip(right))) = (reselect, made) {
            self.selected = Some(right);
            self.cache.clear();
        }
    }

    fn follow(&mut self) {
        let width = (self.window.width - timeline::HEADER_W) as f64;
        let x = (self.playhead as f64 / self.project.rate as f64 - self.view.scroll) * self.view.zoom;
        if self.playing && (x > width - 24.0 || x < 0.0) {
            let lead = width * 0.1 / self.view.zoom;
            self.view.scroll = (self.playhead as f64 / self.project.rate as f64 - lead).max(0.0);
            self.cache.clear();
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let timeline = canvas(Timeline {
            project: &self.project,
            view: self.view,
            selected: self.selected,
            playhead: self.playhead,
            cache: &self.cache,
        })
        .width(Length::Fill)
        .height(Length::Fill);

        column![self.transport(), rule(), timeline, rule(), self.inspector()].into()
    }

    fn transport(&self) -> Element<'_, Message> {
        let seconds = self.playhead as f64 / self.project.rate as f64;
        let beats = seconds * self.project.bpm / 60.0;
        let position = format!("{}.{}", (beats / 4.0) as u64 + 1, beats as u64 % 4 + 1);
        let clock = format!(
            "{}:{:02}.{:03}",
            (seconds / 60.0) as u64,
            seconds as u64 % 60,
            (seconds.fract() * 1000.0) as u64
        );

        let play = button(
            container(icon(if self.playing { "pause" } else { "play" }, 14.0)).center(32),
        )
        .padding(0)
        .style(theme::solid)
        .on_press(Message::TogglePlay);

        let tempo = text_input("", &self.bpm)
            .on_input(Message::BpmTyped)
            .on_submit(Message::BpmEntered)
            .font(theme::MONO)
            .size(13)
            .padding([5, 8])
            .width(64)
            .style(theme::field);

        let history = row![
            icon_button("undo-2", (!self.undo.is_empty()).then_some(Message::Undo)),
            icon_button("redo-2", (!self.redo.is_empty()).then_some(Message::Redo)),
        ]
        .spacing(2);

        let import = button(
            row![icon("folder-open", 14.0), text("Import").size(13).font(theme::MEDIUM)]
                .spacing(8)
                .align_y(Alignment::Center),
        )
        .padding([7, 14])
        .style(theme::outlined)
        .on_press(Message::Import);

        container(
            row![
                text("Loupe").size(16).font(theme::SEMIBOLD),
                Space::with_width(18),
                icon_button("skip-back", Some(Message::ToStart)),
                play,
                Space::with_width(10),
                text(position).size(17).font(theme::MONO).width(64),
                text(clock).size(12).font(theme::MONO).color(theme::TEXT_DIM).width(84),
                text("BPM").size(11).font(theme::MEDIUM).color(theme::TEXT_DIM),
                tempo,
                horizontal_space(),
                history,
                import,
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding([0, 16])
        .height(52)
        .align_y(Alignment::Center)
        .style(theme::bar)
        .into()
    }

    fn inspector(&self) -> Element<'_, Message> {
        let status: Element<'_, Message> = if let Some(problem) = &self.problem {
            text(problem.as_str()).size(12).color(theme::ROSE).into()
        } else if self.loading > 0 {
            let what = if self.loading == 1 { "1 file".into() } else { format!("{} files", self.loading) };
            text(format!("Importing {what}…")).size(12).color(theme::TEXT_DIM).into()
        } else {
            Space::with_width(0).into()
        };

        let split = button(
            row![icon("scissors", 13.0), text("Split").size(12.5).font(theme::MEDIUM)]
                .spacing(7)
                .align_y(Alignment::Center),
        )
        .padding([6, 12])
            .style(theme::outlined)
            .on_press_maybe((!self.split_targets().is_empty()).then_some(Message::Split));

        let clip: Element<'_, Message> = match self.selected.and_then(|id| self.project.clip(id)) {
            Some(clip) => {
                let db = (20.0 * clip.gain.max(1e-6).log10()).clamp(MIN_GAIN_DB, MAX_GAIN_DB);
                let seconds = clip.len as f64 / self.project.rate as f64;
                row![
                    text(clip.source.name.as_str()).size(13).font(theme::MEDIUM),
                    text(format!("{seconds:.3} s")).size(12).font(theme::MONO).color(theme::TEXT_DIM),
                    Space::with_width(12),
                    text("Clip gain").size(12).color(theme::TEXT_DIM),
                    slider(MIN_GAIN_DB..=MAX_GAIN_DB, db, Message::ClipGain)
                        .step(0.1)
                        .default(0.0)
                        .on_release(Message::ClipGainDone)
                        .width(200)
                        .style(theme::gain),
                    text(format!("{db:+.1} dB")).size(12).font(theme::MONO).width(70),
                    split,
                    button(icon("trash-2", 14.0))
                        .padding([6, 10])
                        .style(theme::outlined)
                        .on_press(Message::Delete),
                ]
                .spacing(10)
                .align_y(Alignment::Center)
                .into()
            }
            None => row![
                text("Click a clip to work on it alone").size(12.5).color(theme::TEXT_DIM),
                Space::with_width(12),
                split,
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .into(),
        };

        container(row![clip, horizontal_space(), status].spacing(16).align_y(Alignment::Center))
            .padding([0, 16])
            .height(48)
            .align_y(Alignment::Center)
            .style(theme::bar)
            .into()
    }
}

fn shortcut(key: keyboard::Key, modifiers: keyboard::Modifiers) -> Option<Message> {
    use keyboard::key::Named;
    match key {
        keyboard::Key::Named(Named::Space) => Some(Message::TogglePlay),
        keyboard::Key::Named(Named::Home) => Some(Message::ToStart),
        keyboard::Key::Named(Named::Delete | Named::Backspace) => Some(Message::Delete),
        keyboard::Key::Character(c) => {
            match (c.to_lowercase().as_str(), modifiers.command(), modifiers.shift()) {
                ("s", false, _) => Some(Message::Split),
                ("z", true, false) => Some(Message::Undo),
                ("z", true, true) | ("y", true, _) => Some(Message::Redo),
                ("i", true, _) => Some(Message::Import),
                _ => None,
            }
        }
        _ => None,
    }
}

fn format_bpm(bpm: f64) -> String {
    let text = format!("{bpm:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn icon(name: &str, size: f32) -> iced::widget::Text<'static> {
    text(icons::glyph(name).to_string())
        .font(theme::ICONS)
        .shaping(text::Shaping::Advanced)
        .size(size)
}

fn icon_button(name: &str, on_press: Option<Message>) -> Element<'static, Message> {
    button(container(icon(name, 15.0)).center(30))
        .padding(0)
        .style(theme::ghost)
        .on_press_maybe(on_press)
        .into()
}

fn rule() -> Element<'static, Message> {
    container(Space::new(Length::Fill, 1)).style(theme::line).into()
}
