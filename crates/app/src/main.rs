#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod icons;
mod menus;
mod mixer;
mod pointer;
mod settings;
mod theme;
mod timeline;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use iced::futures::channel::oneshot;
use iced::widget::canvas::Cache;
use iced::advanced::widget::operation::Focusable;
use iced::advanced::widget::{Id, Operation};
use iced::widget::{
    button, canvas, column, container, horizontal_space, row, slider, stack, text, text_input, Space,
};
use iced::{keyboard, window, Alignment, Element, Length, Point, Size, Subscription, Task};
use loupe_engine::{
    ClipId, Command, CommandError, Edge, Engine, Fade, Frames, Outcome, Output, Project, Source, TrackId,
};

use settings::{Settings, MAX_SCALE, MIN_SCALE};
use theme::Palette;
use timeline::{LoopRange, Timeline, View, MAX_GAIN_DB, MIN_GAIN_DB};

const AUDIO_TYPES: [&str; 8] = ["wav", "mp3", "flac", "m4a", "aac", "ogg", "aif", "aiff"];
const UNDO_STEPS: usize = 200;
const SETTLE_TICKS: u8 = 6;

fn main() -> iced::Result {
    let settings = Settings::load();
    let loaded = Palette::load(settings.theme.as_deref());
    let scale = settings.scale;
    let ui_font = loaded.palette.ui;
    iced::application("Loupe", App::update, App::view)
        .subscription(App::subscription)
        .theme(|app: &App| app.palette.iced())
        .scale_factor(|app: &App| app.scale)
        .font(include_bytes!("../assets/Inter-Regular.ttf").as_slice())
        .font(include_bytes!("../assets/Inter-Medium.ttf").as_slice())
        .font(include_bytes!("../assets/Inter-SemiBold.ttf").as_slice())
        .font(include_bytes!("../assets/JetBrainsMono-Regular.ttf").as_slice())
        .font(include_bytes!("../assets/JetBrainsMono-Medium.ttf").as_slice())
        .font(include_bytes!("../assets/lucide.ttf").as_slice())
        .default_font(ui_font)
        .antialiasing(true)
        .window(window::Settings {
            size: START_SIZE,
            min_size: Some(Size::new(820.0, 420.0)),
            ..window::Settings::default()
        })
        .run_with(move || App::new(loaded, scale))
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
    RulerClicked(Frames),
    SetLoop(LoopRange),
    ResizeTrack { track: TrackId, height: f32 },
    MoveClip { clip: ClipId, track: TrackId, start: Frames },
    DragEnd,
    Split,
    Delete,
    Undo,
    Redo,
    ClipGain(f32),
    SetFade { clip: ClipId, edge: Edge, fade: Fade },
    AddTrack,
    RemoveTrack(TrackId),
    ToggleMute(TrackId),
    BpmTyped(String),
    BpmEntered,
    SetView(View),
    Resized(Size),
    OpenSettings,
    CloseOverlay,
    DeleteClip(ClipId),
    TrackMenu { track: TrackId, at: Point },
    StartRename(TrackId),
    StartColour(TrackId),
    EntryTyped(String),
    EntryEntered,
    ColourPicked(TrackId, Option<[u8; 3]>),
    DuplicateTrack(TrackId),
    ToggleMixer,
    TrackGain(TrackId, f32),
    ScaleDragged(f64),
    ScaleChosen,
    ScaleTyped(String),
    ScaleEntered,
    ScaleReset,
}

#[derive(Clone, Copy, PartialEq)]
enum Run {
    Move(ClipId),
    Gain(ClipId),
    Fade(ClipId, Edge),
    TrackGain(TrackId),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Overlay {
    None,
    Settings,
    TrackMenu { track: TrackId, at: Point },
    Rename { track: TrackId, at: Point },
    Colour { track: TrackId, at: Point },
}

struct App {
    palette: Palette,
    heights: HashMap<TrackId, f32>,
    loop_range: LoopRange,
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
    startup_problem: Option<String>,
    scale: f64,
    pending_scale: f64,
    scale_text: String,
    overlay: Overlay,
    entry: String,
    mixer_open: bool,
    cache: Cache,
}

impl App {
    fn new(loaded: theme::Loaded, scale: f64) -> (Self, Task<Message>) {
        let silent = std::env::var("LOUPE_AUDIO").as_deref() == Ok("silent");
        let mut engine = Engine::start(if silent { Output::Silent } else { Output::Device });
        let project = Project::new(engine.rate());
        engine.set_project(&project);
        let no_sound = engine.output_error().map(|e| format!("No sound: {e}"));
        let mut app = Self {
            palette: loaded.palette,
            heights: HashMap::new(),
            loop_range: None,
            problem: None,
            startup_problem: no_sound.or(loaded.problem),
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
            window: Size::new(START_SIZE.width / scale as f32, START_SIZE.height / scale as f32),
            loading: 0,
            scale,
            pending_scale: scale,
            scale_text: format_scale(scale),
            overlay: Overlay::None,
            entry: String::new(),
            mixer_open: false,
            cache: Cache::new(),
        };
        let task = app.import(std::env::args_os().skip(1).map(PathBuf::from).collect());
        (app, task)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        let belongs_to_the_song = matches!(
            message,
            Message::TogglePlay
                | Message::ToStart
                | Message::Split
                | Message::Delete
                | Message::Undo
                | Message::Redo
                | Message::Import
        );
        if self.overlay != Overlay::None && belongs_to_the_song {
            return Task::none();
        }
        match message {
            Message::TogglePlay => {
                if self.playing {
                    self.engine.stop();
                    self.playing = false;
                    self.settle = SETTLE_TICKS;
                } else {
                    if let Some((from, _)) = self.loop_range {
                        self.seek(from);
                    }
                    self.engine.play();
                    self.playing = true;
                }
            }
            Message::ToStart => self.seek(0),
            Message::Seek(to) => self.seek(to),
            Message::Tick => {
                self.engine.collect();
                self.settle = self.settle.saturating_sub(1);
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
            Message::RulerClicked(at) => {
                self.set_loop(None);
                self.seek(at);
            }
            Message::SetLoop(range) => self.set_loop(range),
            Message::ResizeTrack { track, height } => {
                self.heights.insert(track, height);
                self.cache.clear();
            }
            Message::MoveClip { clip, track, start } => {
                self.edit(Some(Run::Move(clip)), Command::MoveClip { clip, track, start });
            }
            Message::DragEnd => self.run = None,
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
            Message::SetFade { clip, edge, fade } => {
                self.edit(Some(Run::Fade(clip, edge)), Command::SetClipFade { clip, edge, fade });
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
                self.heights.remove(&track);
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
                return unfocus();
            }
            Message::SetView(view) => {
                self.view = view;
                self.cache.clear();
            }
            Message::Resized(size) => self.window = size,
            Message::OpenSettings => {
                self.overlay = Overlay::Settings;
                self.pending_scale = self.scale;
                self.scale_text = format_scale(self.scale);
            }
            Message::CloseOverlay => self.overlay = Overlay::None,
            Message::DeleteClip(clip) => {
                if self.selected == Some(clip) {
                    self.selected = None;
                }
                self.edit(None, Command::DeleteClip(clip));
            }
            Message::TrackMenu { track, at } => self.overlay = Overlay::TrackMenu { track, at },
            Message::StartRename(track) => {
                if let (Overlay::TrackMenu { at, .. }, Some(found)) = (&self.overlay, self.project.track(track)) {
                    self.entry = found.name.clone();
                    self.overlay = Overlay::Rename { track, at: *at };
                    return Task::batch([text_input::focus(menus::ENTRY_ID), text_input::select_all(menus::ENTRY_ID)]);
                }
            }
            Message::StartColour(track) => {
                if let Overlay::TrackMenu { at, .. } = &self.overlay {
                    self.entry = String::new();
                    self.overlay = Overlay::Colour { track, at: *at };
                }
            }
            Message::EntryTyped(typed) => self.entry = typed,
            Message::EntryEntered => match self.overlay.clone() {
                Overlay::Rename { track, .. } => {
                    let name = self.entry.clone();
                    self.edit(None, Command::RenameTrack { track, name });
                    self.overlay = Overlay::None;
                }
                Overlay::Colour { track, .. } => {
                    if let Some(colour) = menus::colour_from_hex(&self.entry) {
                        self.edit(None, Command::SetTrackColour { track, colour: Some(colour) });
                        self.overlay = Overlay::None;
                    }
                }
                _ => {}
            },
            Message::ColourPicked(track, colour) => {
                self.edit(None, Command::SetTrackColour { track, colour });
                self.overlay = Overlay::None;
            }
            Message::DuplicateTrack(track) => {
                self.overlay = Overlay::None;
                let height = self.heights.get(&track).copied();
                if let (Some(Outcome::Track(copy)), Some(height)) = (self.edit(None, Command::DuplicateTrack(track)), height) {
                    self.heights.insert(copy, height);
                }
            }
            Message::ToggleMixer => self.mixer_open = !self.mixer_open,
            Message::TrackGain(track, db) => {
                let gain = mixer::gain_from_db(db);
                self.edit(Some(Run::TrackGain(track)), Command::SetTrackGain { track, gain });
            }
            Message::ScaleDragged(scale) => {
                self.pending_scale = scale;
                self.scale_text = format_scale(scale);
            }
            Message::ScaleChosen => self.apply_scale(self.pending_scale),
            Message::ScaleTyped(typed) => {
                if typed.len() <= 4 && typed.chars().all(|c| c.is_ascii_digit() || c == '.') {
                    self.scale_text = typed;
                }
            }
            Message::ScaleEntered => {
                let typed = self.scale_text.parse::<f64>().unwrap_or(self.scale);
                self.apply_scale(typed);
                return unfocus();
            }
            Message::ScaleReset => self.apply_scale(1.0),
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

    fn apply_scale(&mut self, wanted: f64) {
        let scale = settings::clamp_scale(wanted);
        let resize = (self.scale / scale) as f32;
        self.window = Size::new(self.window.width * resize, self.window.height * resize);
        self.scale = scale;
        self.pending_scale = scale;
        self.scale_text = format_scale(scale);
        self.cache.clear();
        if let Err(why) = settings::save("scale", &format_scale(scale)) {
            self.problem = Some(format!("Could not save settings: {why}"));
        }
    }

    fn set_loop(&mut self, range: LoopRange) {
        self.loop_range = range;
        self.engine.set_loop(range);
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
            palette: &self.palette,
            view: self.view,
            heights: &self.heights,
            selected: self.selected,
            playhead: self.playhead,
            loop_range: self.loop_range,
            width: self.window.width,
            cache: &self.cache,
        })
        .width(Length::Fill)
        .height(Length::Fill);

        let palette = self.palette;
        let mut song = column![self.transport(), rule(palette), timeline];
        if self.mixer_open {
            song = song.push(rule(palette)).push(self.mixer());
        }
        let song = song.push(rule(palette)).push(self.inspector());
        stack![song, self.overlay()].into()
    }

    fn transport(&self) -> Element<'_, Message> {
        let palette = self.palette;
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
        .style(move |_, status| palette.solid(status))
        .on_press(Message::TogglePlay);

        let tempo = text_input("", &self.bpm)
            .on_input(Message::BpmTyped)
            .on_submit(Message::BpmEntered)
            .font(palette.mono)
            .size(13)
            .padding([5, 8])
            .width(46)
            .style(move |_, status| palette.field(status));

        let mixer_open = self.mixer_open;
        let mixer = button(container(icon("sliders-vertical", 15.0)).center(30))
            .padding(0)
            .style(move |_, status| palette.toggled(mixer_open, status))
            .on_press(Message::ToggleMixer);

        let history = row![
            icon_button(palette, "undo-2", (!self.undo.is_empty()).then_some(Message::Undo)),
            icon_button(palette, "redo-2", (!self.redo.is_empty()).then_some(Message::Redo)),
        ]
        .spacing(2);

        let import = button(
            row![icon("folder-open", 14.0), text("Import").size(13).font(palette.medium)]
                .spacing(8)
                .align_y(Alignment::Center),
        )
        .padding([7, 14])
        .style(move |_, status| palette.outlined(status))
        .on_press(Message::Import);

        container(
            row![
                text("Loupe").size(16).font(palette.semibold),
                Space::with_width(18),
                icon_button(palette, "skip-back", Some(Message::ToStart)),
                play,
                Space::with_width(10),
                text(position).size(13).font(palette.mono).width(52),
                text(clock).size(13).font(palette.mono).color(palette.text_dim).width(84),
                text("BPM").size(11).font(palette.medium).color(palette.text_dim),
                tempo,
                horizontal_space(),
                history,
                mixer,
                icon_button(palette, "settings", Some(Message::OpenSettings)),
                import,
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding([0, 16])
        .height(52)
        .align_y(Alignment::Center)
        .style(move |_| palette.bar())
        .into()
    }

    fn settings_sheet(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let heading = row![
            text("Settings").size(16).font(palette.semibold),
            horizontal_space(),
            icon_button(palette, "x", Some(Message::CloseOverlay)),
        ]
        .align_y(Alignment::Center);

        let scale = column![
            text("Interface scale").size(13).font(palette.medium),
            text("Makes everything in the window larger or smaller. 1 is the normal size.")
                .size(12)
                .color(palette.text_dim),
            row![
                slider(MIN_SCALE..=MAX_SCALE, self.pending_scale, Message::ScaleDragged)
                    .step(0.05)
                    .on_release(Message::ScaleChosen)
                    .style(move |_, status| palette.slider(status)),
                text_input("", &self.scale_text)
                    .on_input(Message::ScaleTyped)
                    .on_submit(Message::ScaleEntered)
                    .font(palette.mono)
                    .size(13)
                    .padding([5, 8])
                    .width(60)
                    .style(move |_, status| palette.field(status)),
                button(text("Reset").size(12.5).font(palette.medium))
                    .padding([6, 12])
                    .style(move |_, status| palette.outlined(status))
                    .on_press_maybe((self.scale != 1.0).then_some(Message::ScaleReset)),
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        ]
        .spacing(8);

        container(column![heading, rule(palette), scale].spacing(16))
            .padding(20)
            .width(Length::Fill)
            .max_width(560)
            .style(move |_| palette.sheet())
            .into()
    }

    fn inspector(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let status: Element<'_, Message> = if let Some(problem) = self.problem.as_ref().or(self.startup_problem.as_ref()) {
            text(problem.as_str()).size(12).color(palette.danger).into()
        } else if self.loading > 0 {
            let what = if self.loading == 1 { "1 file".into() } else { format!("{} files", self.loading) };
            text(format!("Importing {what}…")).size(12).color(palette.text_dim).into()
        } else {
            Space::with_width(0).into()
        };

        let split = button(
            row![icon("scissors", 13.0), text("Split").size(12.5).font(palette.medium)]
                .spacing(7)
                .align_y(Alignment::Center),
        )
        .padding([6, 12])
            .style(move |_, status| palette.outlined(status))
            .on_press_maybe((!self.split_targets().is_empty()).then_some(Message::Split));

        let clip: Element<'_, Message> = match self.selected.and_then(|id| self.project.clip(id)) {
            Some(clip) => {
                let db = (20.0 * clip.gain.max(1e-6).log10()).clamp(MIN_GAIN_DB, MAX_GAIN_DB);
                let seconds = clip.len as f64 / self.project.rate as f64;
                row![
                    text(clip.source.name.as_str()).size(13).font(palette.medium),
                    text(format!("{seconds:.3} s")).size(12).font(palette.mono).color(palette.text_dim),
                    text(format!("{db:+.1} dB")).size(12).font(palette.mono).color(palette.text_dim).width(70),
                    split,
                    button(icon("trash-2", 14.0))
                        .padding([6, 10])
                        .style(move |_, status| palette.outlined(status))
                        .on_press(Message::Delete),
                ]
                .spacing(10)
                .align_y(Alignment::Center)
                .into()
            }
            None => row![
                text("Click a clip to work on it alone").size(12.5).color(palette.text_dim),
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
            .style(move |_| palette.bar())
            .into()
    }
}

fn shortcut(key: keyboard::Key, modifiers: keyboard::Modifiers) -> Option<Message> {
    use keyboard::key::Named;
    match key {
        keyboard::Key::Named(Named::Space) => Some(Message::TogglePlay),
        keyboard::Key::Named(Named::Home) => Some(Message::ToStart),
        keyboard::Key::Named(Named::Escape) => Some(Message::CloseOverlay),
        keyboard::Key::Named(Named::F9) => Some(Message::ToggleMixer),
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

fn format_scale(scale: f64) -> String {
    let text = format!("{scale:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

struct Unfocus;

impl Operation for Unfocus {
    fn container(
        &mut self,
        _id: Option<&Id>,
        _bounds: iced::Rectangle,
        operate_on_children: &mut dyn FnMut(&mut dyn Operation<()>),
    ) {
        operate_on_children(self);
    }

    fn focusable(&mut self, state: &mut dyn Focusable, _id: Option<&Id>) {
        state.unfocus();
    }
}

fn unfocus() -> Task<Message> {
    iced_runtime::task::widget(Unfocus).discard()
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

fn icon_button(palette: Palette, name: &str, on_press: Option<Message>) -> Element<'static, Message> {
    button(container(icon(name, 15.0)).center(30))
        .padding(0)
        .style(move |_, status| palette.ghost(status))
        .on_press_maybe(on_press)
        .into()
}

fn rule(palette: Palette) -> Element<'static, Message> {
    container(Space::new(Length::Fill, 1)).style(move |_| palette.rule()).into()
}
