#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod clip_window;
mod crash;
mod exporting;
mod files;
mod home;
mod icons;
mod knob;
mod menus;
mod mixer;
mod pointer;
mod pool;
mod recording;
mod selection;
mod settings;
mod spinner;
mod theme;
mod timeline;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::futures::channel::oneshot;
use iced::widget::canvas::Cache;
use iced::advanced::widget::operation::Focusable;
use iced::advanced::widget::{Id, Operation};
use iced::widget::{
    button, canvas, column, container, horizontal_space, pick_list, progress_bar, row, slider, stack, text,
    text_input, Space,
};
use iced::{keyboard, window, Alignment, Element, Length, Point, Size, Subscription, Task};
use loupe_engine::{
    ClipId, Command, CommandError, Edge, Engine, Fade, Frames, Input, InputChoice, Outcome, Output, Project, Source,
    TrackId,
};

use settings::{Settings, MAX_SCALE, MIN_SCALE};
use theme::Palette;
use timeline::{LoopRange, Timeline, Tool, View};

const AUDIO_TYPES: [&str; 8] = ["wav", "mp3", "flac", "m4a", "aac", "ogg", "aif", "aiff"];
const UNDO_STEPS: usize = 200;
const SETTLE_TICKS: u8 = 6;
const STATUS_HEIGHT: f32 = 30.0;
const METER_FALL_PER_TICK: f32 = 0.86;
const SYSTEM_INPUT: &str = "System default";
const COPIED_SHOWN_FOR: Duration = Duration::from_millis(1500);
const EXPORT_PROGRESS_STEPS: u32 = 1000;
const MASTER_PERCENT_PER_PX: f32 = 0.5;
const EMPTY_SONG_ZOOM: f64 = 100.0;
const TOP_BAR_HEIGHT: f32 = 53.0;
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const SETTINGS_PAGE_HEIGHT: f32 = 96.0;

fn main() -> iced::Result {
    crash::keep_a_record();
    let settings = Settings::load();
    let loaded = Palette::load(settings.theme.as_deref());
    let ui_font = loaded.palette.ui;
    let opens_a_song = std::env::args_os().len() > 1;
    let first_size = if opens_a_song { START_SIZE } else { scaled(HOME_SIZE, settings.scale) };
    iced::application(App::title, App::update, App::view)
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
            size: first_size,
            icon: window::icon::from_file_data(include_bytes!("../assets/icon.png"), None).ok(),
            min_size: Some(Size::new(820.0, 420.0)),
            ..window::Settings::default()
        })
        .run_with(move || App::new(loaded, settings))
}

const START_SIZE: Size = Size::new(1280.0, 760.0);
const HOME_SIZE: Size = Size::new(940.0, 600.0);

#[derive(Debug, Clone)]
pub enum Message {
    TogglePlay,
    ToggleRecord,
    TakeReady { tracks: Vec<TrackId>, start: i64, warning: Option<String>, result: Result<Arc<Source>, String> },
    ToStart,
    Seek(Frames),
    Tick,
    Import,
    Picked(Vec<PathBuf>),
    Dropped(PathBuf),
    Loaded(PathBuf, Result<Arc<Source>, String>),
    Select(Option<ClipId>),
    ToggleSelect(ClipId),
    SelectMany(Vec<ClipId>),
    SelectAll,
    Join,
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
    ToggleArm(TrackId),
    InputChosen(String),
    BpmTyped(String),
    BpmEntered,
    SetView(View),
    Resized(Size),
    OpenSettings,
    CloseOverlay,
    DeleteClip(ClipId),
    TrackMenu { track: TrackId, at: Point },
    OpenFileMenu,
    OpenHelpMenu,
    OpenAbout,
    StartRename(TrackId),
    StartColour(TrackId),
    EntryTyped(String),
    EntryEntered,
    ColourPicked(TrackId, Option<[u8; 3]>),
    DuplicateTrack(TrackId),
    ToggleMixer,
    MixerGrabbed,
    MixerDragged(f32),
    MixerReleased,
    MasterPercent(f32),
    LevelPressed(mixer::Level),
    OpenClip(ClipId),
    ClipToTrack(ClipId, TrackId),
    ToggleClipMute(ClipId),
    TogglePreview(ClipId),
    CopyText(String),
    ShowInFolder(PathBuf),
    LevelEntered,
    SetTool(Tool),
    Refresh,
    PaintMute { clip: ClipId, muted: bool },
    PaintDelete(ClipId),
    TrimClip { clip: ClipId, start: Frames, offset: Frames, len: Frames },
    Slice { cuts: Vec<(TrackId, Frames)> },
    TrackGain(TrackId, f32),
    TogglePool,
    PlaceSource(usize),
    OpenProject,
    Discard,
    GoHome,
    NewBlank,
    NewFromTemplate(PathBuf),
    OpenRecent(PathBuf),
    SaveAsTemplate,
    ProjectPicked(Option<PathBuf>),
    ProjectRead(PathBuf, bool, Result<files::Opened, String>),
    Save,
    SaveAs,
    SavePicked(Option<PathBuf>),
    SaveElsewhere,
    OpenExport,
    ExportSplit(bool),
    ExportRangeOnly(bool),
    ExportElsewhere,
    ExportFolderPicked(Option<PathBuf>),
    StartExport,
    Exported(Result<PathBuf, String>),
    ScaleDragged(f64),
    ScaleChosen,
    ScaleTyped(String),
    ScaleEntered,
    ScaleReset,
    LeftSong { maximized: bool },
    PickFolder,
    FolderPicked(Option<PathBuf>),
    FolderReset,
    SettingsTab(SettingsTab),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    Display,
    File,
    Recording,
}

impl SettingsTab {
    const ALL: [Self; 3] = [Self::Display, Self::File, Self::Recording];

    fn label(self) -> &'static str {
        match self {
            Self::Display => "Display",
            Self::File => "File",
            Self::Recording => "Recording",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Run {
    Move(ClipId),
    Gain(ClipId),
    Fade(ClipId, Edge),
    TrackGain(TrackId),
    Master,
    Paint,
    Trim(ClipId),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Overlay {
    None,
    Settings,
    FileMenu,
    HelpMenu,
    About,
    TrackMenu { track: TrackId, at: Point },
    Rename { track: TrackId, at: Point },
    Colour { track: TrackId, at: Point },
    ConfirmDiscard(Pending),
    TemplateName,
    SaveName,
    Export,
    Clip(ClipId),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pending {
    Open,
    Home,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Screen {
    Home,
    Song,
}

struct App {
    palette: Palette,
    heights: HashMap<TrackId, f32>,
    loop_range: LoopRange,
    tool: Tool,
    level_press: Option<(mixer::Level, Instant)>,
    editing_level: Option<mixer::Level>,
    preview: Option<clip_window::Preview>,
    copied: Option<Instant>,
    armed: HashSet<TrackId>,
    recording: Option<recording::Recording>,
    input: Option<Input>,
    input_level: f32,
    input_name: Option<String>,
    input_names: Vec<String>,
    practice_input: bool,
    engine: Engine,
    project: Project,
    undo: Vec<Project>,
    redo: Vec<Project>,
    run: Option<Run>,
    selected: Option<ClipId>,
    selection: HashSet<ClipId>,
    playing: bool,
    settle: u8,
    playhead: Frames,
    view: View,
    window: Size,
    bpm: String,
    loading: usize,
    problem: Option<String>,
    notice: Option<String>,
    export_split: bool,
    export_range_only: bool,
    export_elsewhere: Option<PathBuf>,
    exporting: bool,
    export_progress: Arc<AtomicU32>,
    startup_problem: Option<String>,
    scale: f64,
    pending_scale: f64,
    scale_text: String,
    overlay: Overlay,
    settings_tab: SettingsTab,
    entry: String,
    entry_problem: Option<String>,
    mixer_open: bool,
    mixer_height: f32,
    folder: Option<PathBuf>,
    resizing_mixer: bool,
    pool_open: bool,
    path: Option<PathBuf>,
    dirty: bool,
    screen: Screen,
    opening: Option<(String, Instant)>,
    song_size: Size,
    song_maximized: bool,
    templates: Vec<PathBuf>,
    recent: Vec<PathBuf>,
    cache: Cache,
}

impl App {
    fn new(loaded: theme::Loaded, settings: Settings) -> (Self, Task<Message>) {
        let scale = settings.scale;
        let silent = std::env::var("LOUPE_AUDIO").as_deref() == Ok("silent");
        let mut engine = Engine::start(if silent { Output::Silent } else { Output::Device });
        let project = Project::new(engine.rate());
        engine.set_project(&project);
        let no_sound = engine.output_error().map(|e| format!("No sound: {e}"));
        let no_folder = settings::make_folders(settings.folder.as_deref()).err().map(|why| format!("Could not make the Loupe folder: {why}"));
        let mut app = Self {
            palette: loaded.palette,
            heights: HashMap::new(),
            loop_range: None,
            tool: Tool::default(),
            level_press: None,
            editing_level: None,
            preview: None,
            copied: None,
            armed: HashSet::new(),
            recording: None,
            input: None,
            input_level: 0.0,
            input_name: settings.input.clone(),
            input_names: Vec::new(),
            practice_input: silent,
            problem: None,
            notice: None,
            export_split: false,
            export_range_only: false,
            export_elsewhere: None,
            exporting: false,
            export_progress: Arc::new(AtomicU32::new(0)),
            startup_problem: no_sound.or(loaded.problem).or(no_folder),
            bpm: format_bpm(project.bpm),
            engine,
            project,
            undo: Vec::new(),
            redo: Vec::new(),
            run: None,
            selected: None,
            selection: HashSet::new(),
            playing: false,
            settle: 0,
            playhead: 0,
            view: View { zoom: EMPTY_SONG_ZOOM, scroll: 0.0, scroll_y: 0.0 },
            window: Size::new(START_SIZE.width / scale as f32, START_SIZE.height / scale as f32),
            loading: 0,
            scale,
            pending_scale: scale,
            scale_text: format_scale(scale),
            overlay: Overlay::None,
            settings_tab: SettingsTab::default(),
            entry: String::new(),
            entry_problem: None,
            mixer_open: settings.mixer_open,
            mixer_height: settings.mixer_height.unwrap_or(mixer::MIXER_HEIGHT).max(mixer::SHORTEST_MIXER),
            resizing_mixer: false,
            folder: settings.folder.clone(),
            pool_open: false,
            path: None,
            dirty: false,
            screen: Screen::Song,
            opening: None,
            song_size: START_SIZE,
            song_maximized: false,
            templates: Vec::new(),
            recent: Vec::new(),
            cache: Cache::new(),
        };
        let (projects, audio): (Vec<PathBuf>, Vec<PathBuf>) = std::env::args_os()
            .skip(1)
            .map(PathBuf::from)
            .partition(|path| path.extension().is_some_and(|extension| extension == files::EXTENSION));
        let task = match projects.into_iter().next() {
            Some(project) => app.read_project(project, false),
            None if audio.is_empty() => {
                app.screen = Screen::Home;
                app.window = Size::new(HOME_SIZE.width, HOME_SIZE.height);
                app.refresh_home();
                Task::none()
            }
            None => app.import(audio),
        };
        (app, task)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        let before = self.screen;
        let task = self.handle(message);
        match (before, self.screen) {
            (Screen::Song, Screen::Home) => Task::batch([
                task,
                window::get_latest()
                    .and_then(window::get_maximized)
                    .map(|maximized| Message::LeftSong { maximized }),
            ]),
            (Screen::Home, Screen::Song) => {
                let (size, maximized) = (self.song_size, self.song_maximized);
                let fit = window::get_latest().and_then(move |id| {
                    if maximized {
                        window::maximize(id, true)
                    } else {
                        window::resize(id, size)
                    }
                });
                Task::batch([task, fit])
            }
            _ => task,
        }
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        let belongs_to_the_song = matches!(
            message,
            Message::TogglePlay
                | Message::ToggleRecord
                | Message::ToStart
                | Message::Split
                | Message::Delete
                | Message::Undo
                | Message::Redo
                | Message::Import
        );
        if (self.overlay != Overlay::None || self.screen == Screen::Home) && belongs_to_the_song {
            return Task::none();
        }
        let would_break_the_take = matches!(
            message,
            Message::ToggleArm(_)
                | Message::RemoveTrack(_)
                | Message::InputChosen(_)
                | Message::GoHome
                | Message::OpenProject
                | Message::OpenRecent(_)
                | Message::NewBlank
                | Message::NewFromTemplate(_)
                | Message::ProjectPicked(_)
        );
        if self.recording.is_some() && would_break_the_take {
            self.notice = Some("Stop recording first.".into());
            return Task::none();
        }
        match message {
            Message::ToggleRecord => return self.toggle_recording(),
            Message::TakeReady { tracks, start, warning, result } => {
                self.place_take(tracks, start, result);
                if warning.is_some() {
                    self.problem = warning;
                }
            }
            Message::TogglePlay => {
                if self.recording.is_some() {
                    return self.finish_recording();
                }
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
                if self.copied.is_some_and(|at| at.elapsed() > COPIED_SHOWN_FOR) {
                    self.copied = None;
                }
                if let Some(input) = &self.input {
                    self.input_level = input.take_peak().max(self.input_level * METER_FALL_PER_TICK);
                }
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
                    Ok(source) => {
                        let already_here = self.project.sources.iter().find(|kept| kept.path == source.path && !kept.frames.is_empty());
                        let source = already_here.cloned().unwrap_or(source);
                        self.place(source);
                    }
                    Err(why) => {
                        let name = path.file_name().unwrap_or_default().to_string_lossy();
                        self.problem = Some(format!("Could not import {name}: {why}"));
                    }
                }
            }
            Message::Select(clip) => self.choose(clip),
            Message::ToggleSelect(clip) => {
                let mut chosen = self.selection.clone();
                if !chosen.remove(&clip) {
                    chosen.insert(clip);
                }
                self.choose(chosen);
            }
            Message::SelectMany(clips) => self.choose(clips),
            Message::SelectAll => {
                let every: Vec<ClipId> = self.project.clips().map(|clip| clip.id).collect();
                self.choose(every);
            }
            Message::Join => self.join_selection(),
            Message::LaneClicked(at) => {
                self.choose(None);
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
            Message::MoveClip { clip, track, start } => self.move_clips(clip, track, start),
            Message::DragEnd => self.run = None,
            Message::Split => self.split(),
            Message::Delete => {
                let chosen: Vec<ClipId> = self.selection.iter().copied().collect();
                if !chosen.is_empty() {
                    self.delete_clips(chosen, None);
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
                self.heights.remove(&track);
                self.armed.remove(&track);
                self.listen_if_armed();
                self.edit(None, Command::RemoveTrack(track));
                self.forget_gone_clips();
            }
            Message::ToggleArm(track) => {
                if !self.armed.remove(&track) {
                    self.armed.insert(track);
                }
                self.listen_if_armed();
                self.cache.clear();
            }
            Message::InputChosen(name) => {
                let chosen = (name != SYSTEM_INPUT).then_some(name);
                let saved = match &chosen {
                    Some(name) => settings::save("input", name),
                    None => settings::forget("input"),
                };
                if let Err(why) = saved {
                    self.problem = Some(format!("Could not save settings: {why}"));
                }
                self.input_name = chosen;
                self.input = None;
                self.listen_if_armed();
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
                self.input_names = loupe_engine::input_devices();
                self.overlay = Overlay::Settings;
                self.pending_scale = self.scale;
                self.scale_text = format_scale(self.scale);
            }
            Message::CloseOverlay => {
                if self.overlay == Overlay::None {
                    self.choose(None);
                }
                self.overlay = Overlay::None;
                self.editing_level = None;
                self.stop_preview();
            }
            Message::OpenClip(clip) => {
                self.choose([clip]);
                self.overlay = Overlay::Clip(clip);
            }
            Message::ClipToTrack(clip, track) => {
                if let Some(start) = self.project.clip(clip).map(|clip| clip.start) {
                    self.edit(None, Command::MoveClip { clip, track, start });
                }
            }
            Message::ToggleClipMute(clip) => {
                if let Some(muted) = self.project.clip(clip).map(|clip| !clip.muted) {
                    self.edit(None, Command::SetClipMuted { clip, muted });
                }
            }
            Message::TogglePreview(clip) => self.toggle_preview(clip),
            Message::CopyText(copied) => {
                self.copied = Some(Instant::now());
                return iced::clipboard::write(copied);
            }
            Message::ShowInFolder(file) => {
                if let Err(why) = files::show_in_folder(&file) {
                    self.problem = Some(format!("Could not open the folder: {why}"));
                }
            }
            Message::DeleteClip(clip) => self.delete_clips(self.affected_by(clip), None),
            Message::TrackMenu { track, at } => self.overlay = Overlay::TrackMenu { track, at },
            Message::OpenFileMenu => self.overlay = Overlay::FileMenu,
            Message::OpenHelpMenu => self.overlay = Overlay::HelpMenu,
            Message::OpenAbout => self.overlay = Overlay::About,
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
            Message::EntryTyped(typed) => {
                self.entry = typed;
                self.entry_problem = None;
            }
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
                Overlay::TemplateName => {
                    let name = self.entry.clone();
                    self.save_template(&name);
                }
                Overlay::SaveName => {
                    let name = self.entry.clone();
                    self.save_named(&name);
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
            Message::ToggleMixer => {
                self.mixer_open = !self.mixer_open;
                let _ = settings::save("mixer", if self.mixer_open { "open" } else { "closed" });
            }
            Message::MixerGrabbed => self.resizing_mixer = true,
            Message::MixerDragged(pointer_y) => {
                let below = if self.status().is_some() { STATUS_HEIGHT + 1.0 } else { 0.0 };
                let tallest = (self.window.height - TOP_BAR_HEIGHT - below).max(mixer::SHORTEST_MIXER);
                self.mixer_height = (self.window.height - below - pointer_y).clamp(mixer::SHORTEST_MIXER, tallest);
            }
            Message::MixerReleased => {
                self.resizing_mixer = false;
                if let Err(why) = settings::save("mixer_height", &self.mixer_height.round().to_string()) {
                    self.problem = Some(format!("Could not save settings: {why}"));
                }
            }
            Message::TrackGain(track, db) => {
                let gain = mixer::gain_from_db(db);
                self.edit(Some(Run::TrackGain(track)), Command::SetTrackGain { track, gain });
            }
            Message::SetTool(tool) => {
                self.tool = tool;
                self.cache.clear();
            }
            Message::Refresh => {}
            Message::PaintMute { clip, muted } => self.mute_clips(self.affected_by(clip), muted),
            Message::PaintDelete(clip) => self.delete_clips(self.affected_by(clip), Some(Run::Paint)),
            Message::TrimClip { clip, start, offset, len } => {
                if let Some(track) = self.project.track_of(clip).map(|track| track.id) {
                    self.transact(Some(Run::Trim(clip)), |project| {
                        project.apply(Command::TrimClip { clip, offset, len })?;
                        project.apply(Command::MoveClip { clip, track, start })
                    });
                }
            }
            Message::Slice { cuts } => {
                let splits: Vec<(ClipId, Frames)> = cuts
                    .iter()
                    .filter_map(|(track, at)| self.project.track(*track).map(|track| (track, *at)))
                    .flat_map(|(track, at)| track.clips.iter().map(move |clip| (clip, at)))
                    .filter(|(clip, at)| *at > clip.start && *at < clip.end())
                    .map(|(clip, at)| (clip.id, at))
                    .collect();
                if !splits.is_empty() {
                    self.transact(None, |project| {
                        for (clip, at) in splits {
                            project.apply(Command::SplitClip { clip, at })?;
                        }
                        Ok(Outcome::Done)
                    });
                }
                self.cache.clear();
            }
            Message::LevelPressed(level) => {
                let pressed_twice = self
                    .level_press
                    .is_some_and(|(last, at)| last == level && at.elapsed() < DOUBLE_CLICK);
                self.level_press = Some((level, Instant::now()));
                if pressed_twice {
                    let gain = match level {
                        mixer::Level::Master => Some(self.project.master),
                        mixer::Level::Track(track) => self.project.track(track).map(|track| track.gain),
                        mixer::Level::Clip(clip) => self.project.clip(clip).map(|clip| clip.gain),
                    };
                    if let Some(gain) = gain {
                        self.entry = if level == mixer::Level::Master {
                            mixer::percent_text(gain)
                        } else {
                            mixer::level_text(gain)
                        };
                        self.editing_level = Some(level);
                        return Task::batch([
                            text_input::focus(mixer::LEVEL_ENTRY_ID),
                            text_input::select_all(mixer::LEVEL_ENTRY_ID),
                        ]);
                    }
                }
            }
            Message::LevelEntered => {
                if self.editing_level == Some(mixer::Level::Master) {
                    self.editing_level = None;
                    if let Some(percent) = mixer::percent_from_typed(&self.entry) {
                        self.edit(None, Command::SetMasterGain(percent / 100.0));
                    }
                    return unfocus();
                }
                if let (Some(level), Some(db)) = (self.editing_level.take(), mixer::db_from_typed(&self.entry)) {
                    let fader = mixer::gain_from_db(db.clamp(mixer::SILENT_DB, mixer::LOUDEST_DB));
                    let command = match level {
                        mixer::Level::Master => return unfocus(),
                        mixer::Level::Track(track) => Command::SetTrackGain { track, gain: fader },
                        mixer::Level::Clip(clip) => {
                            let gain = 10f32.powf(db.clamp(timeline::MIN_GAIN_DB, timeline::MAX_GAIN_DB) / 20.0);
                            Command::SetClipGain { clip, gain }
                        }
                    };
                    self.edit(None, command);
                }
                return unfocus();
            }
            Message::MasterPercent(percent) => {
                self.edit(Some(Run::Master), Command::SetMasterGain(percent / 100.0));
            }
            Message::TogglePool => {
                self.pool_open = !self.pool_open;
                self.overlay = Overlay::None;
                self.cache.clear();
            }
            Message::PlaceSource(index) => {
                if let Some(source) = self.project.sources.get(index).cloned() {
                    self.place(source);
                }
            }
            Message::OpenProject => return self.ask_to_open(),
            Message::Discard => match self.overlay {
                Overlay::ConfirmDiscard(Pending::Open) => return self.pick_project(),
                Overlay::ConfirmDiscard(Pending::Home) => self.go_home(),
                _ => {}
            },
            Message::GoHome => {
                if self.dirty && !self.project.tracks.is_empty() {
                    self.overlay = Overlay::ConfirmDiscard(Pending::Home);
                } else {
                    self.go_home();
                }
            }
            Message::NewBlank => {
                self.replace_project(Project::new(self.project.rate), HashMap::new());
                self.screen = Screen::Song;
            }
            Message::NewFromTemplate(template) => return self.read_project(template, true),
            Message::OpenRecent(project) => return self.read_project(project, false),
            Message::SaveAsTemplate => {
                self.overlay = Overlay::TemplateName;
                self.entry = self.path.as_deref().map(home::stem).unwrap_or_default();
                return Task::batch([text_input::focus(menus::ENTRY_ID), text_input::select_all(menus::ENTRY_ID)]);
            }
            Message::ProjectPicked(path) => {
                if let Some(path) = path {
                    return self.read_project(path, false);
                }
            }
            Message::ProjectRead(path, as_template, result) => {
                self.loading = self.loading.saturating_sub(1);
                self.opening = None;
                match result {
                    Ok(opened) => self.adopt(path, opened, as_template),
                    Err(why) => {
                        let name = path.file_name().unwrap_or_default().to_string_lossy();
                        self.problem = Some(format!("Could not open {name}: {why}"));
                    }
                }
            }
            Message::Save => {
                self.overlay = Overlay::None;
                if !self.project.tracks.is_empty() {
                    return self.save();
                }
            }
            Message::SaveAs => {
                self.overlay = Overlay::None;
                if !self.project.tracks.is_empty() {
                    return self.save_as();
                }
            }
            Message::SaveElsewhere => return self.save_elsewhere(),
            Message::OpenExport => {
                if !self.project.tracks.is_empty() {
                    self.overlay = Overlay::Export;
                }
            }
            Message::ExportSplit(split) => self.export_split = split,
            Message::ExportRangeOnly(range_only) => self.export_range_only = range_only,
            Message::ExportElsewhere => return self.pick_export_folder(),
            Message::ExportFolderPicked(folder) => {
                if folder.is_some() {
                    self.export_elsewhere = folder;
                }
            }
            Message::StartExport => return self.start_export(),
            Message::Exported(result) => {
                self.exporting = false;
                match result {
                    Ok(folder) => self.notice = Some(format!("Exported to {}", folder.display())),
                    Err(why) => self.problem = Some(format!("Could not export: {why}")),
                }
            }
            Message::SavePicked(path) => {
                if let Some(path) = path {
                    self.write_to(path);
                }
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
            Message::LeftSong { maximized } => {
                self.song_maximized = maximized;
                if !maximized {
                    self.song_size = scaled(self.window, self.scale);
                }
                let home = scaled(HOME_SIZE, self.scale);
                return window::get_latest()
                    .and_then(move |id| Task::batch([window::maximize(id, false), window::resize(id, home)]));
            }
            Message::PickFolder => {
                let start_in = settings::home_folder(self.folder.as_deref());
                return Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_title("Choose where the Loupe folder lives")
                            .set_directory(start_in)
                            .pick_folder()
                            .await
                            .map(|folder| folder.path().to_path_buf())
                    },
                    Message::FolderPicked,
                );
            }
            Message::FolderPicked(folder) => {
                if let Some(folder) = folder {
                    self.use_folder(Some(folder));
                }
            }
            Message::FolderReset => self.use_folder(None),
            Message::SettingsTab(tab) => self.settings_tab = tab,
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
        let watching = self.exporting || self.copied.is_some() || self.input.is_some() || self.opening.is_some();
        let ticks = if self.playing || self.settle > 0 || watching {
            iced::time::every(Duration::from_millis(16)).map(|_| Message::Tick)
        } else {
            Subscription::none()
        };
        let mixer_drag = if self.resizing_mixer {
            iced::event::listen_with(|event, _status, _window| match event {
                iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => Some(Message::MixerDragged(position.y)),
                iced::Event::Mouse(iced::mouse::Event::ButtonReleased(_)) => Some(Message::MixerReleased),
                _ => None,
            })
        } else {
            Subscription::none()
        };
        Subscription::batch([shortcuts, window, ticks, mixer_drag])
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
                self.notice = None;
                self.dirty = true;
                self.engine.set_project(&self.project);
                let timeline_looks_the_same = matches!(run, Some(Run::Master | Run::TrackGain(_)));
                if !timeline_looks_the_same {
                    self.cache.clear();
                }
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
        self.dirty = true;
        self.forget_gone_clips();
        self.bpm = format_bpm(self.project.bpm);
        self.changed();
    }

    pub(crate) fn listen_if_armed(&mut self) {
        if self.armed.is_empty() {
            self.input = None;
            self.input_level = 0.0;
            return;
        }
        if self.input.is_some() {
            return;
        }
        let choice = match (self.practice_input, &self.input_name) {
            (true, _) => InputChoice::Practice,
            (false, Some(name)) => InputChoice::Named(name.clone()),
            (false, None) => InputChoice::SystemDefault,
        };
        match Input::open(choice) {
            Ok(input) => self.input = Some(input),
            Err(why) => {
                self.armed.clear();
                self.problem = Some(format!("Could not open the recording input: {why}"));
            }
        }
    }

    fn use_folder(&mut self, chosen: Option<PathBuf>) {
        let saved = match &chosen {
            Some(folder) => settings::save("folder", &folder.display().to_string()),
            None => settings::forget("folder"),
        };
        self.folder = chosen;
        let made = settings::make_folders(self.folder.as_deref());
        self.problem = saved.and(made).err().map(|why| format!("Could not set the Loupe folder: {why}"));
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

    fn canvas_width(&self) -> f32 {
        let beside = if self.pool_open { pool::POOL_WIDTH + 1.0 } else { 0.0 };
        (self.window.width - beside).max(timeline::HEADER_W + 1.0)
    }

    fn show_whole_song(&mut self) {
        let seconds = self.project.length() as f64 / self.project.rate.max(1) as f64;
        let room = (self.canvas_width() - timeline::HEADER_W - 48.0).max(100.0) as f64;
        self.view.zoom = if seconds > 0.0 {
            (room / seconds).clamp(timeline::MIN_ZOOM, View::max_zoom(self.project.rate))
        } else {
            EMPTY_SONG_ZOOM
        };
        self.view.scroll = 0.0;
        self.cache.clear();
    }

    fn set_loop(&mut self, range: LoopRange) {
        self.loop_range = range;
        self.engine.set_loop(range);
    }

    fn seek(&mut self, to: Frames) {
        if self.recording.is_some() {
            return;
        }
        self.engine.seek(to);
        self.playhead = to;
        self.cache.clear();
    }

    fn import(&mut self, paths: Vec<PathBuf>) -> Task<Message> {
        let rate = self.project.rate;
        self.loading += paths.len();
        if !paths.is_empty() {
            self.problem = None;
            self.screen = Screen::Song;
        }
        Task::batch(paths.into_iter().map(|path| {
            let already_here = self.project.sources.iter().find(|kept| kept.path == path && !kept.frames.is_empty());
            if let Some(source) = already_here {
                return Task::done(Message::Loaded(path, Ok(source.clone())));
            }
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
        let name = source.name.clone();
        let placed = self.transact(None, |project| {
            let Outcome::Track(track) = project.apply(Command::AddTrack { name })? else {
                return Err(CommandError::NoSuchTrack);
            };
            project.apply(Command::AddClip { track, source, start })
        });
        if let Some(Outcome::Clip(clip)) = placed {
            self.choose([clip]);
            if first {
                self.show_whole_song();
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
            self.choose([right]);
        }
    }

    fn follow(&mut self) {
        let width = (self.canvas_width() - timeline::HEADER_W) as f64;
        let x = (self.playhead as f64 / self.project.rate as f64 - self.view.scroll) * self.view.zoom;
        if self.playing && (x > width - 24.0 || x < 0.0) {
            let lead = width * 0.1 / self.view.zoom;
            self.view.scroll = (self.playhead as f64 / self.project.rate as f64 - lead).max(0.0);
            self.cache.clear();
        }
    }

    fn view(&self) -> Element<'_, Message> {
        if self.screen == Screen::Home {
            return stack![self.home(), self.overlay(), self.opening_layer()].into();
        }
        let timeline = canvas(Timeline {
            project: &self.project,
            palette: &self.palette,
            view: self.view,
            heights: &self.heights,
            selected: self.selected,
            selection: &self.selection,
            playhead: self.playhead,
            loop_range: self.loop_range,
            tool: self.tool,
            armed: &self.armed,
            recording_from: self.recording.as_ref().map(|recording| recording.from),
            input_level: self.input_level,
            opening: self.opening.is_some(),
            width: self.canvas_width(),
            cache: &self.cache,
        })
        .width(Length::Fill)
        .height(Length::Fill);

        let palette = self.palette;
        let mut middle = row![timeline];
        if self.pool_open {
            middle = middle.push(upright_rule(palette)).push(self.pool());
        }
        let mut song = column![self.transport(), rule(palette), middle];
        if self.mixer_open {
            song = song.push(rule(palette)).push(self.mixer());
        }
        if let Some(status) = self.status() {
            song = song.push(rule(palette)).push(status);
        }
        stack![song, self.overlay(), self.opening_layer()].into()
    }

    fn opening_layer(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let Some((name, since)) = &self.opening else {
            return Space::new(0, 0).into();
        };
        let notice = column![
            canvas(spinner::Spinner { palette: &self.palette, since: *since }).width(44).height(44),
            text(format!("Opening {name}")).size(14).font(palette.medium),
        ]
        .spacing(16)
        .align_x(Alignment::Center);
        let card = container(notice).padding([26, 40]).style(move |_| palette.sheet());
        iced::widget::opaque(iced::widget::center(card).style(move |_| palette.backdrop()))
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

        let recording = self.recording.is_some();
        let record = button(container(container(Space::new(11, 11)).style(move |_| palette.record_mark(recording))).center(30))
            .padding(0)
            .style(move |_, status| palette.record(recording, status))
            .on_press(Message::ToggleRecord);

        let tempo = text_input("", &self.bpm)
            .on_input(Message::BpmTyped)
            .on_submit(Message::BpmEntered)
            .font(palette.mono)
            .size(13)
            .padding([5, 8])
            .width(46)
            .style(move |_, status| palette.field(status));

        let file_menu_open = self.overlay == Overlay::FileMenu;
        let mixer_open = self.mixer_open;
        let file = button(
            row![text("File").size(13).font(palette.medium), icon("chevron-down", 12.0)]
                .spacing(5)
                .align_y(Alignment::Center),
        )
        .padding([6, 10])
        .style(move |_, status| palette.toggled(file_menu_open, status))
        .on_press(Message::OpenFileMenu);

        let help_menu_open = self.overlay == Overlay::HelpMenu;
        let help = button(text("Help").size(13).font(palette.medium))
            .padding([6, 10])
            .style(move |_, status| palette.toggled(help_menu_open, status))
            .on_press(Message::OpenHelpMenu);

        let mixer = button(container(icon("sliders-vertical", 15.0)).center(30))
            .padding(0)
            .style(move |_, status| palette.toggled(mixer_open, status))
            .on_press(Message::ToggleMixer);

        let master = row![
            text("Master").size(11).font(palette.medium).color(palette.text_dim),
            canvas(knob::Knob {
                palette: &self.palette,
                value: self.project.master * 100.0,
                lowest: 0.0,
                highest: mixer::LOUDEST_MASTER_PERCENT,
                resting: 100.0,
                per_px: MASTER_PERCENT_PER_PX,
                on_turn: Message::MasterPercent,
            })
            .width(30)
            .height(30),
            container(self.level_readout(mixer::Level::Master, self.project.master)).width(52),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

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
                file,
                help,
                Space::with_width(6),
                icon_button(palette, "skip-back", Some(Message::ToStart)),
                play,
                record,
                Space::with_width(10),
                text(position).size(13).font(palette.mono).width(52),
                text(clock).size(13).font(palette.mono).color(palette.text_dim).width(84),
                text("BPM").size(11).font(palette.medium).color(palette.text_dim),
                tempo,
                Space::with_width(10),
                master,
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

        let home = settings::home_folder(self.folder.as_deref());
        let folder = column![
            text("Loupe folder").size(13).font(palette.medium),
            text("Projects and templates live here by default. You can still save anywhere.")
                .size(12)
                .color(palette.text_dim),
            row![
                text(home.display().to_string()).size(12).font(palette.mono).width(Length::Fill),
                button(text("Change…").size(12.5).font(palette.medium))
                    .padding([6, 12])
                    .style(move |_, status| palette.outlined(status))
                    .on_press(Message::PickFolder),
                button(text("Reset").size(12.5).font(palette.medium))
                    .padding([6, 12])
                    .style(move |_, status| palette.outlined(status))
                    .on_press_maybe(self.folder.is_some().then_some(Message::FolderReset)),
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        ]
        .spacing(8);

        let mut inputs = vec![SYSTEM_INPUT.to_string()];
        inputs.extend(self.input_names.iter().cloned());
        let current_input = self.input_name.clone().unwrap_or_else(|| SYSTEM_INPUT.to_string());
        let recording = column![
            text("Recording input").size(13).font(palette.medium),
            text("The device armed tracks record from.").size(12).color(palette.text_dim),
            pick_list(inputs, Some(current_input), Message::InputChosen).text_size(13).padding([5, 10]).width(Length::Fill),
        ]
        .spacing(8);

        let current = self.settings_tab;
        let tabs = row(SettingsTab::ALL.map(|tab| {
            button(text(tab.label()).size(13).font(palette.medium))
                .padding([6, 12])
                .style(move |_, status| palette.toggled(tab == current, status))
                .on_press(Message::SettingsTab(tab))
                .into()
        }))
        .spacing(4);
        let page = match current {
            SettingsTab::Display => scale,
            SettingsTab::File => folder,
            SettingsTab::Recording => recording,
        };
        let body = column![tabs, rule(palette), container(page).height(SETTINGS_PAGE_HEIGHT)].spacing(14);
        self.window("Settings".to_string(), body.into(), 560.0)
    }

    fn status(&self) -> Option<Element<'_, Message>> {
        let palette = self.palette;
        let line: Element<'_, Message> = if let Some(problem) = self.problem.as_ref().or(self.startup_problem.as_ref()) {
            text(problem.as_str()).size(12).color(palette.danger).into()
        } else if self.exporting {
            let done = self.export_progress.load(Ordering::Relaxed) as f32 / EXPORT_PROGRESS_STEPS as f32;
            row![
                text("Exporting").size(12).color(palette.text_dim),
                progress_bar(0.0..=1.0, done).width(240).height(6),
                text(format!("{:.0}%", done * 100.0)).size(12).font(palette.mono).color(palette.text_dim),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .into()
        } else if let Some(notice) = &self.notice {
            text(notice.as_str()).size(12).color(palette.text).into()
        } else if self.loading > 0 {
            let what = if self.loading == 1 { "1 file".into() } else { format!("{} files", self.loading) };
            text(format!("Loading {what}…")).size(12).color(palette.text_dim).into()
        } else {
            return None;
        };
        Some(
            container(line)
                .padding([0, 16])
                .width(Length::Fill)
                .height(STATUS_HEIGHT)
                .align_y(Alignment::Center)
                .style(move |_| palette.bar())
                .into(),
        )
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
                ("r", false, _) => Some(Message::ToggleRecord),
                ("p", false, _) => Some(Message::SetTool(Tool::Pencil)),
                ("c", false, _) => Some(Message::SetTool(Tool::Razor)),
                ("t", false, _) => Some(Message::SetTool(Tool::Mute)),
                ("d", false, _) => Some(Message::SetTool(Tool::Delete)),
                ("s", true, false) => Some(Message::Save),
                ("s", true, true) => Some(Message::SaveAs),
                ("o", true, _) => Some(Message::OpenProject),
                ("w", true, _) => Some(Message::GoHome),
                ("g", true, _) => Some(Message::Join),
                ("a", true, _) => Some(Message::SelectAll),
                ("e", true, _) => Some(Message::OpenExport),
                ("z", true, false) => Some(Message::Undo),
                ("z", true, true) | ("y", true, _) => Some(Message::Redo),
                ("i", true, _) => Some(Message::Import),
                _ => None,
            }
        }
        _ => None,
    }
}

fn scaled(size: Size, scale: f64) -> Size {
    Size::new(size.width * scale as f32, size.height * scale as f32)
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

fn upright_rule(palette: Palette) -> Element<'static, Message> {
    container(Space::new(1, Length::Fill)).style(move |_| palette.rule()).into()
}

fn rule(palette: Palette) -> Element<'static, Message> {
    container(Space::new(Length::Fill, 1)).style(move |_| palette.rule()).into()
}
