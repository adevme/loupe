#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backup;
mod audio_settings;
mod autosaving;
mod clip_window;
mod clipboard;
mod crash;
mod crash_reports;
mod exporting;
mod files;
mod home;
mod icons;
mod knob;
mod menus;
mod mixer;
mod piano_roll;
mod pointer;
mod pool;
mod routing;
mod safe_mode;
mod plugins;
mod racks;
mod stretching;
mod stockwin;
mod printing;
mod chains;
mod keying;
mod recording;
mod takes;
mod selection;
mod sampler_sheet;
mod scripting;
mod scripts;
mod settings;
mod spinner;
mod starting;
mod theme;
mod theming;
mod timeline;
mod usage;
mod versions;

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
    Chains, ClipId, Command, CommandError, Edge, Engine, Fade, Frames, Input, InputChannels, InputChoice, Instrument, Note, Outcome,
    Output, Device, Project, Source, TrackId,
};

use settings::{Settings, MAX_SCALE, MIN_SCALE};
use theme::{BarItem, Palette, Side};
use timeline::{LoopRange, Timeline, Tool, View};

const AUDIO_TYPES: [&str; 8] = ["wav", "mp3", "flac", "m4a", "aac", "ogg", "aif", "aiff"];
const UNDO_STEPS: usize = 200;
const SETTLE_TICKS: u8 = 6;
const STATUS_HEIGHT: f32 = 30.0;
/// Ticks between looking in on plugins that are still opening, about a fifth of a second.
const LOOK_IN_EVERY: u8 = 12;
/// Stands in for a track id when naming the knobs of a master plugin.
pub const MASTER_OWNER: u64 = u64::MAX;
const METER_FALL_PER_TICK: f32 = 0.86;
const FADER_STEP_DB: f32 = 0.5;
const MASTER_STEP_PERCENT: f32 = 1.0;
const SYSTEM_INPUT: &str = "System default";
const COPIED_SHOWN_FOR: Duration = Duration::from_millis(1500);
const EXPORT_PROGRESS_STEPS: u32 = 1000;
const MASTER_PERCENT_PER_PX: f32 = 0.5;
const EMPTY_SONG_ZOOM: f64 = 100.0;
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const SETTINGS_PAGE_HEIGHT: f32 = 330.0;

fn main() -> iced::Result {
    let shift_at_start = safe_mode::shift_held();
    crash::keep_a_record();
    let settings = Settings::load();
    let mut loaded = Palette::load(settings.theme.as_deref());
    let icon_font = loaded.icon_font.take();
    let ui_font = loaded.palette.ui;
    let opens_a_song = std::env::args_os().len() > 1;
    let first_size = if opens_a_song { START_SIZE } else { scaled(HOME_SIZE, settings.scale) };
    let mut loupe = iced::application(starting::Loupe::title, starting::Loupe::update, starting::Loupe::view)
        .subscription(starting::Loupe::subscription)
        .theme(starting::Loupe::theme)
        .scale_factor(starting::Loupe::scale)
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
        });
    if let Some(font) = icon_font {
        loupe = loupe.font(font);
    }
    let ran = loupe.run_with(move || starting::Loupe::starting(loaded, settings, shift_at_start));
    backup::mark_closed();
    usage::finish();
    ran
}

const START_SIZE: Size = Size::new(1280.0, 760.0);
const HOME_SIZE: Size = Size::new(940.0, 600.0);

#[derive(Debug, Clone)]
pub enum Message {
    TogglePlay,
    ToggleRecord,
    TakeReady { start: i64, keep_from: i64, passes: Option<recording::Passes>, punch: Option<(Frames, Frames)>, warning: Option<String>, result: Result<(Vec<(TrackId, Arc<Source>)>, Option<String>), String> },
    ToStart,
    Seek(Frames),
    Tick,
    Import,
    Picked(Vec<PathBuf>),
    Dropped(PathBuf),
    KeyTold(loupe_stock_ui::KeyMessage),
    KeyFileHeard(String, Result<loupe_stock::Heard, String>),
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
    ToggleSolo(TrackId),
    TrackPan(TrackId, f32),
    ModifiersChanged(keyboard::Modifiers),
    FirstFrame,
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
    OpenScriptsMenu,
    RunScript(PathBuf),
    OpenScriptsFolder,
    ScriptKey(String, keyboard::Modifiers),
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
    ToggleMasterMute,
    ToggleCollapsed(loupe_engine::TrackId),
    SetTrackParent { track: TrackId, parent: Option<TrackId> },
    OpenRouting(TrackId),
    OpenPlugins(TrackId),
    PluginFilter(String),
    PluginsFound(Vec<loupe_plugins::Found>),
    AddPlugin(TrackId, usize),
    RemovePlugin(TrackId, usize),
    BypassPlugin(TrackId, usize),
    WheelOverFader(mixer::Level, iced::mouse::ScrollDelta),
    ShowPlugin(TrackId, usize),
    OpenClipPlugins(ClipId),
    ShowClipPlugin(ClipId, usize),
    MoveClipPlugin(ClipId, usize, usize),
    OpenKnobs(stockwin::Spot, usize),
    AddClipPlugin(ClipId, usize),
    RemoveClipPlugin(ClipId, usize),
    BypassClipPlugin(ClipId, usize),
    PutPoint { target: loupe_engine::Target, at: Frames, value: f32 },
    DragPoint { target: loupe_engine::Target, which: usize, at: Frames, value: f32 },
    DropPoint { target: loupe_engine::Target, which: usize },
    AddEnvelope(loupe_engine::Target),
    RemoveEnvelope(loupe_engine::Target),
    ArmEnvelope(loupe_engine::Target, Option<loupe_engine::Mode>),
    OpenAutomation(loupe_engine::Target),
    PluginHighlight(i32),
    PluginChosen,
    Hint(Option<&'static str>),
    FxGrab(TrackId, usize),
    FxOver(usize),
    FxDrop,
    StockTurned(loupe_stock_ui::Change),
    StockTold(loupe_stock_ui::EqMessage),
    OpenMatrix,
    AddSend { from: TrackId, to: TrackId },
    RemoveSend { from: TrackId, to: TrackId },
    SendGain { from: TrackId, to: TrackId, gain: f32 },
    SendPreFader { from: TrackId, to: TrackId, pre_fader: bool },
    SendSidechain { from: TrackId, to: TrackId, sidechain: bool },
    LevelPressed(mixer::Level),
    OpenClip(ClipId),
    Autosave,
    AutosaveDone(u64, Result<PathBuf, String>),
    AutosaveTyped(String),
    AutosaveEntered,
    KeptTyped(String),
    KeptEntered,
    Recover,
    SkipRecovery,
    CrashNoteTyped(String),
    SendCrashReport,
    CrashReportSent(Result<(), String>),
    SkipCrashReport,
    TurnFallenOff,
    LeaveFallenOn,
    NewNotesClip(TrackId),
    UseInstrument(TrackId, Instrument),
    ToggleRecordsNotes(TrackId),
    TogglePrintTakes(TrackId),
    StartInputs(TrackId),
    UseInput(TrackId, InputChannels),
    UseTake(ClipId, usize),
    Comp { track: TrackId, take: usize, from: Frames, to: Frames },
    ShowChain(TrackId, bool),
    HearInputToggled,
    OpenSampler(TrackId),
    SamplerChanged(TrackId, loupe_engine::Sampler),
    PickSample(TrackId),
    SampleFile(TrackId, PathBuf),
    SampleLoaded(TrackId, Result<Arc<Source>, String>),
    RollPlaced { clip: ClipId, notes: Vec<Note>, key: u8, chosen: Vec<Note> },
    RollEdit { clip: ClipId, notes: Vec<Note>, chosen: Option<Vec<Note>> },
    RollChoose(Vec<Note>),
    RollAction(piano_roll::RollAction),
    RollSound { key: u8, on: bool },
    RollSlide { from: Option<u8>, to: u8 },
    RollDone { remember_beats: Option<f64> },
    RollView(piano_roll::RollView),
    TypedKey { key: u8, down: bool },
    ToggleTypingKeys,
    Both(Box<Message>, Box<Message>),
    OpenVersions,
    CheckForUpdates,
    UpdateChecked(Result<Option<versions::Update>, String>),
    InstallUpdate,
    UpdateDownloaded(Result<PathBuf, String>),
    UpdateLater,
    UsageToggled(bool),
    CheckUpdatesOnStart(bool),
    ToggleMetronome,
    CopyClips,
    CutClips,
    PasteClips,
    DuplicateClips,
    CountInChosen(CountIn),
    PrerollChosen(CountIn),
    TogglePunch,
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
    StretchClip { clip: ClipId, start: Frames, len: Frames },
    Stretched { source: Arc<Source>, stretch: f64, made: Option<Arc<Source>> },
    ToggleSnap,
    MidiInputToggled(String),
    OpenMasterPlugins,
    AddMasterPlugin(usize),
    ChainName(String),
    SaveChain,
    UseChain(PathBuf),
    RemoveMasterPlugin(usize),
    BypassMasterPlugin(usize),
    ShowMasterPlugin(usize),
    AudioDriverChosen(audio_settings::Driver),
    AudioOutputChosen(String),
    AudioRateChosen(audio_settings::Rate),
    AudioBufferChosen(audio_settings::Buffer),
    Reopened(Result<files::Opened, String>),
    Slice { cuts: Vec<(TrackId, Frames)> },
    TrackGain(TrackId, f32),
    TogglePool,
    PlaceSource(usize),
    OpenProject,
    OpenWithPluginsOff,
    PluginsBackOn,
    Discard,
    GoHome,
    NewBlank,
    NewFromTemplate(PathBuf),
    OpenRecent(PathBuf),
    SaveAsTemplate,
    ProjectPicked(Option<PathBuf>),
    ProjectRead(PathBuf, bool, bool, Result<files::Opened, String>),
    Save,
    SaveAs,
    SavePicked(Option<PathBuf>),
    SaveElsewhere,
    OpenExport,
    ExportSplit(bool),
    ExportFormatChosen(loupe_engine::Format),
    ExportDither(bool),
    ExportNormaliseChosen(loupe_engine::Normalise),
    ExportRangeOnly(bool),
    ExportElsewhere,
    ExportFolderPicked(Option<PathBuf>),
    StartExport,
    Exported(Result<(PathBuf, Option<String>), String>),
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
    ThemeChosen(String),
    ThemeFontLoaded,
    ShowThemes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    Display,
    File,
    Recording,
    Audio,
    Privacy,
}

impl SettingsTab {
    const ALL: [Self; 5] = [Self::Display, Self::File, Self::Recording, Self::Audio, Self::Privacy];

    fn label(self) -> &'static str {
        match self {
            Self::Display => "Display",
            Self::File => "File",
            Self::Recording => "Recording",
            Self::Audio => "Audio",
            Self::Privacy => "Privacy",
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
    Pan(TrackId),
    Paint,
    Trim(ClipId),
    Stretch(ClipId),
    Send(TrackId, TrackId),
    Point(loupe_engine::Target),
    Notes(ClipId),
    Sampler(TrackId),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Overlay {
    None,
    Settings,
    FileMenu,
    HelpMenu,
    ScriptsMenu,
    About,
    TrackMenu { track: TrackId, at: Point },
    Rename { track: TrackId, at: Point },
    Colour { track: TrackId, at: Point },
    Inputs { track: TrackId, at: Point, inputs: u16 },
    ConfirmDiscard(Pending),
    TemplateName,
    SaveName,
    Export,
    Clip(ClipId),
    Routing(TrackId),
    Sampler(TrackId),
    Plugins(TrackId),
    ClipPlugins(ClipId),
    Knobs(stockwin::Spot, usize),
    Automation(loupe_engine::Target),
    MasterPlugins,
    Stock,
    Matrix,
    Recover,
    CrashReport,
    PluginFell,
    Roll(ClipId),
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
    input_levels: Vec<f32>,
    track_levels: [f32; loupe_engine::METERS],
    master_level: f32,
    input_name: Option<String>,
    theme_name: Option<String>,
    theme_names: Vec<String>,
    theme_problem: Option<String>,
    input_names: Vec<String>,
    practice_input: bool,
    engine: Engine,
    racks: Option<Box<dyn Chains>>,
    fx_was: u64,
    peeks: racks::Peeks,
    stock: Option<stockwin::Window>,
    fx_drag: Option<(TrackId, usize, usize)>,
    writing: Option<loupe_engine::Writer>,
    knob_names: HashMap<(u64, usize, usize, bool), String>,
    hint: Option<&'static str>,
    plugins_opening: bool,
    since_looked_in: u8,
    reading: Option<String>,
    found: Vec<loupe_plugins::Found>,
    scanning: bool,
    plugin_filter: String,
    chains: Vec<(String, PathBuf)>,
    chain_name: String,
    plugin_highlight: usize,
    plugin_uses: plugins::Uses,
    falls: racks::Falls,
    fell: Vec<racks::Fell>,
    fell_named: HashSet<String>,
    crash_reports: Vec<crash::Waiting>,
    crash_note: String,
    crash_sending: bool,
    crash_sheet_due: bool,
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
    export: settings::ExportChoices,
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
    autosave_minutes: u32,
    backups_kept: u32,
    autosave_text: String,
    kept_text: String,
    autosave_problem: Option<String>,
    revision: u64,
    backed_up: u64,
    remembered: u64,
    lost: Vec<backup::Lost>,
    recovering: Option<Option<PathBuf>>,
    marked: Option<backup::Place>,
    roll_view: piano_roll::RollView,
    roll_beats: f64,
    roll_chosen: Vec<Note>,
    typing_keys: bool,
    roll_copied: Vec<Note>,
    typing: HashSet<u8>,
    midi_keys: Option<loupe_engine::MidiKeys>,
    keys_aimed_at: Option<TrackId>,
    update_state: versions::UpdateState,
    quiet_check: bool,
    check_updates: bool,
    update_dismissed: bool,
    usage: usage::Usage,
    metronome: bool,
    count_in_bars: u32,
    preroll_bars: u32,
    punch: bool,
    hear_input: bool,
    rec_shown: HashSet<TrackId>,
    copied_clips: Option<clipboard::Copied>,
    scripts: Vec<scripts::Script>,
    snap: bool,
    midi_chosen: Vec<String>,
    midi_around: Vec<String>,
    stretches: stretching::Stretches,
    modifiers: keyboard::Modifiers,
    audio: loupe_engine::Device,
    audio_lists: audio_settings::Lists,
    silent: bool,
    plugins_off: racks::PluginsOff,
    open_next_safely: bool,
}

impl App {
    fn new(loaded: theme::Loaded, settings: Settings, shift_at_start: bool) -> (Self, Task<Message>) {
        let scale = settings.scale;
        let silent = std::env::var("LOUPE_AUDIO").as_deref() == Ok("silent");
        let mut engine = Engine::start(if silent { Output::Silent } else { Output::Device(settings.audio.clone()) });
        let project = Project::new(engine.rate());
        engine.set_project(&project);
        engine.set_metronome(settings.metronome);
        let no_sound = engine.output_error().map(|e| format!("No sound: {e}"));
        let no_folder = settings::make_folders(settings.folder.as_deref()).err().map(|why| format!("Could not make the Loupe folder: {why}"));
        loupe_plugins::presets::keep_in(settings::presets_folder(settings.folder.as_deref()));
        let (usage_now, _) = usage::Usage::begin(&settings);
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
            input_levels: Vec::new(),
            track_levels: [0.0; loupe_engine::METERS],
            master_level: 0.0,
            input_name: settings.input.clone(),
            theme_name: settings.theme.clone(),
            theme_names: Vec::new(),
            theme_problem: loaded.problem,
            input_names: Vec::new(),
            practice_input: silent,
            problem: None,
            notice: None,
            export: settings.export,
            export_range_only: false,
            export_elsewhere: None,
            exporting: false,
            export_progress: Arc::new(AtomicU32::new(0)),
            startup_problem: no_sound.or(no_folder),
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
            autosave_minutes: settings.autosave_minutes,
            backups_kept: settings.backups_kept,
            autosave_text: settings.autosave_minutes.to_string(),
            kept_text: settings.backups_kept.to_string(),
            autosave_problem: None,
            revision: 0,
            backed_up: 0,
            remembered: 0,
            lost: Vec::new(),
            recovering: None,
            marked: None,
            roll_view: piano_roll::RollView::default(),
            roll_beats: 1.0,
            roll_chosen: Vec::new(),
            typing_keys: false,
            roll_copied: Vec::new(),
            typing: HashSet::new(),
            midi_keys: None,
            keys_aimed_at: None,
            update_state: versions::UpdateState::default(),
            quiet_check: false,
            check_updates: settings.check_updates,
            update_dismissed: false,
            usage: usage_now,
            metronome: settings.metronome,
            count_in_bars: settings.count_in_bars,
            preroll_bars: settings.preroll_bars,
            punch: settings.punch,
            hear_input: settings.hear_input,
            rec_shown: HashSet::new(),
            copied_clips: None,
            scripts: Vec::new(),
            snap: settings.snap,
            midi_chosen: settings.midi_inputs.clone().unwrap_or_default(),
            midi_around: loupe_engine::MidiKeys::around(),
            stretches: stretching::Stretches::default(),
            modifiers: keyboard::Modifiers::default(),
            audio: settings.audio.clone(),
            audio_lists: audio_settings::Lists::default(),
            silent,
            racks: None,
            fx_was: 0,
            peeks: racks::Peeks::default(),
            stock: None,
            fx_drag: None,
            writing: None,
            knob_names: HashMap::new(),
            found: Vec::new(),
            scanning: true,
            hint: None,
            plugins_opening: false,
            since_looked_in: 0,
            reading: None,
            plugin_filter: String::new(),
            chains: Vec::new(),
            chain_name: String::new(),
            plugin_highlight: 0,
            plugin_uses: plugins::Uses::load(),
            plugins_off: racks::PluginsOff::default(),
            open_next_safely: false,
            falls: racks::Falls::default(),
            fell: Vec::new(),
            fell_named: HashSet::new(),
            crash_reports: crash::waiting(),
            crash_note: String::new(),
            crash_sending: false,
            crash_sheet_due: false,
        };
        app.crash_sheet_due = !app.crash_reports.is_empty();
        app.note_audio_for_crashes();
        let (projects, audio): (Vec<PathBuf>, Vec<PathBuf>) = std::env::args_os()
            .skip(1)
            .map(PathBuf::from)
            .partition(|path| path.extension().is_some_and(|extension| extension == files::EXTENSION));
        let task = match projects.into_iter().next() {
            Some(project) => {
                let safely = shift_at_start || app.wants_safe_open();
                app.read_project(project, false, safely)
            }
            None if audio.is_empty() => {
                app.screen = Screen::Home;
                app.window = Size::new(HOME_SIZE.width, HOME_SIZE.height);
                app.refresh_home();
                Task::none()
            }
            None => app.import(audio),
        };
        app.lost = backup::find_lost();
        if !app.lost.is_empty() {
            app.overlay = Overlay::Recover;
        }
        app.find_scripts();
        app.keep_safe();
        app.listen_to_keyboards();
        // The window is open before Loupe is told how big it is, and the timeline draws
        // to that width, so ask for it rather than waiting for the first resize.
        let measure = window::get_latest().and_then(window::get_size).map(Message::Resized);
        let hunt = Task::perform(async { plugins::find_plugins() }, Message::PluginsFound);
        let look = if app.check_updates {
            app.quiet_check = true;
            app.check_for_updates()
        } else {
            Task::none()
        };
        (app, Task::batch([task, measure, hunt, look]))
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        let before = self.screen;
        let task = self.handle(message);
        let task = Task::batch([task, self.stretch_waiting()]);
        self.keep_safe();
        self.aim_keys();
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
        let message = match (&self.overlay, message) {
            (Overlay::Roll(_), Message::Delete) => Message::RollAction(piano_roll::RollAction::Delete),
            (Overlay::Roll(_), Message::CopyClips) => Message::RollAction(piano_roll::RollAction::Copy),
            (Overlay::Roll(_), Message::CutClips) => Message::RollAction(piano_roll::RollAction::Cut),
            (Overlay::Roll(_), Message::PasteClips) => Message::RollAction(piano_roll::RollAction::Paste),
            (Overlay::Roll(_), Message::DuplicateClips) => Message::RollAction(piano_roll::RollAction::Duplicate),
            (Overlay::Roll(_), Message::SelectAll) => Message::RollAction(piano_roll::RollAction::SelectAll),
            (Overlay::Roll(_), Message::CloseOverlay) if !self.roll_chosen.is_empty() => Message::RollAction(piano_roll::RollAction::Clear),
            (_, message) => message,
        };
        let roll_open = matches!(self.overlay, Overlay::Roll(_));
        let fine_in_the_roll = roll_open && matches!(message, Message::TogglePlay | Message::ToStart | Message::Undo | Message::Redo);
        let belongs_to_the_song = matches!(
            message,
            Message::TogglePlay
                | Message::ToggleRecord
                | Message::ToStart
                | Message::Split
                | Message::Delete
                | Message::CopyClips
                | Message::CutClips
                | Message::PasteClips
                | Message::DuplicateClips
                | Message::Undo
                | Message::Redo
                | Message::Import
        );
        if (self.overlay != Overlay::None || self.screen == Screen::Home) && belongs_to_the_song && !fine_in_the_roll {
            return Task::none();
        }
        let would_break_the_take = matches!(
            message,
            Message::ToggleArm(_)
                | Message::RemoveTrack(_)
                | Message::InputChosen(_)
                | Message::GoHome
                | Message::OpenProject
                | Message::OpenWithPluginsOff
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
            Message::TakeReady { start, keep_from, passes, punch, warning, result } => {
                let printing = result.as_ref().ok().and_then(|(_, trouble)| trouble.clone());
                self.place_take(start, keep_from, passes, punch, result.map(|(sources, _)| sources));
                if let Some(why) = printing {
                    self.problem = Some(format!("The take is kept dry, the Rec plugins could not be printed: {why}"));
                }
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
                    self.writing = None;
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
                let punched_out = self.recording.as_ref().and_then(|recording| recording.punch).is_some_and(|(_, to)| self.engine.position() >= to);
                if punched_out {
                    return self.finish_recording();
                }
                // Taking the racks back from the sound thread is not free, so look in
                // every so often rather than on every tick.
                if self.plugins_opening {
                    self.since_looked_in += 1;
                    if self.since_looked_in >= LOOK_IN_EVERY {
                        self.since_looked_in = 0;
                        self.nudge_racks();
                    }
                }
                if let Some(recording) = self.recording.as_mut() {
                    recording.taped.extend(self.engine.taped_keys());
                    if recording.began.is_none() {
                        recording.began = self.input.as_ref().and_then(Input::take_began).map(|at| self.engine.position_at(at));
                    }
                }
                self.keep_writing();
                self.take_falls();
                if let Some(window) = self.stock.as_mut() {
                    window.tick();
                }
                if self.copied.is_some_and(|at| at.elapsed() > COPIED_SHOWN_FOR) {
                    self.copied = None;
                }
                if let Some(input) = &self.input {
                    let peaks = input.take_peaks();
                    self.input_levels.resize(peaks.len(), 0.0);
                    for (shown, now) in self.input_levels.iter_mut().zip(peaks) {
                        *shown = now.max(*shown * METER_FALL_PER_TICK);
                    }
                }
                let (tracks, master) = self.engine.levels();
                for (shown, now) in self.track_levels.iter_mut().zip(tracks) {
                    *shown = now.max(*shown * METER_FALL_PER_TICK);
                }
                self.master_level = master.max(self.master_level * METER_FALL_PER_TICK);
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
            Message::KeyTold(message) => self.key_told(message),
            Message::KeyFileHeard(name, result) => self.key_file_heard(name, result),
            Message::Dropped(path) => {
                if self.key_wants_file() {
                    return self.read_key_from(path);
                }
                if let Overlay::Sampler(track) = self.overlay {
                    return self.load_sample(track, path);
                }
                return self.import(vec![path]);
            }
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
            Message::DragEnd => {
                self.run = None;
                self.reading = None;
                self.stop_writing();
            }
            Message::Split => self.split(),
            Message::CopyClips => {
                self.copy_clips();
            }
            Message::CutClips => self.cut_clips(),
            Message::PasteClips => self.paste_clips(),
            Message::DuplicateClips => self.duplicate_clips(),
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
            Message::ModifiersChanged(modifiers) => self.modifiers = modifiers,
            Message::FirstFrame => {}
            Message::TrackPan(track, pan) => {
                let pan = pan.clamp(-1.0, 1.0);
                self.edit(Some(Run::Pan(track)), Command::SetTrackPan { track, pan });
                let name = self.project.track(track).map(|found| found.name.clone()).unwrap_or_default();
                self.reading = Some(format!("{name}  pan {}", mixer::pan_text(pan)));
            }
            Message::ToggleSolo(track) => self.toggle_solo(track),
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
            Message::Resized(size) => {
                if size != self.window {
                    self.window = size;
                    self.cache.clear();
                }
            }
            Message::OpenSettings => {
                self.input_names = loupe_engine::input_devices();
                self.theme_names = theme::available();
                self.overlay = Overlay::Settings;
                self.pending_scale = self.scale;
                self.scale_text = format_scale(self.scale);
            }
            Message::CloseOverlay if self.overlay == Overlay::PluginFell => self.settle_fallen(false),
            Message::CloseOverlay => {
                if matches!(self.overlay, Overlay::Roll(_)) {
                    self.engine.silence_notes();
                    self.typing.clear();
                }
                if self.overlay == Overlay::None {
                    self.choose(None);
                }
                self.overlay = Overlay::None;
                self.stock = None;
                self.editing_level = None;
                self.stop_preview();
            }
            Message::OpenClip(clip) => {
                self.choose([clip]);
                if self.project.clip(clip).is_some_and(|found| found.is_notes()) {
                    self.open_roll(clip);
                } else {
                    self.overlay = Overlay::Clip(clip);
                }
            }
            Message::NewNotesClip(track) => self.new_notes_clip(track),
            Message::OpenSampler(track) => self.open_sampler(track),
            Message::SamplerChanged(track, sampler) => self.change_sampler(track, sampler),
            Message::PickSample(track) => return self.pick_sample(track),
            Message::SampleFile(track, path) => return self.load_sample(track, path),
            Message::SampleLoaded(track, result) => self.sample_loaded(track, result),
            Message::UseTake(clip, take) => {
                if self.project.clip(clip).is_some_and(|found| found.take != take) {
                    self.edit(None, Command::UseTake { clip, take });
                }
            }
            Message::Comp { track, take, from, to } => self.comp(track, take, from, to),
            Message::TogglePrintTakes(track) => {
                self.overlay = Overlay::None;
                if let Some(on) = self.project.track(track).map(|t| !t.print_takes) {
                    self.edit(None, Command::SetPrintTakes { track, on });
                }
            }
            Message::StartInputs(track) => {
                if let Overlay::TrackMenu { at, .. } = &self.overlay {
                    let inputs = self.input.as_ref().map(Input::inputs).or_else(|| loupe_engine::input_count(&self.input_choice())).unwrap_or(0);
                    self.overlay = Overlay::Inputs { track, at: *at, inputs };
                }
            }
            Message::UseInput(track, input) => {
                self.overlay = Overlay::None;
                self.edit(None, Command::SetTrackInput { track, input });
            }
            Message::ShowChain(track, record) => {
                if record {
                    self.rec_shown.insert(track);
                } else {
                    self.rec_shown.remove(&track);
                }
            }
            Message::HearInputToggled => {
                self.hear_input = !self.hear_input;
                if let Err(why) = settings::save("hear_input", if self.hear_input { "on" } else { "off" }) {
                    self.problem = Some(format!("Could not save settings: {why}"));
                }
                self.listen_if_armed();
            }
            Message::ToggleRecordsNotes(track) => {
                self.overlay = Overlay::None;
                if let Some(on) = self.project.track(track).map(|t| !t.records_notes) {
                    self.edit(None, Command::SetRecordsNotes { track, on });
                    self.listen_if_armed();
                }
            }
            Message::UseInstrument(track, instrument) => {
                self.overlay = Overlay::None;
                self.edit(None, Command::SetInstrument { track, instrument });
            }
            Message::RollPlaced { clip, notes, key, chosen } => {
                self.edit(Some(Run::Notes(clip)), Command::SetNotes { clip, notes });
                self.roll_chosen = chosen;
                self.sound(key, true);
            }
            Message::RollEdit { clip, notes, chosen } => {
                self.edit(Some(Run::Notes(clip)), Command::SetNotes { clip, notes });
                if let Some(chosen) = chosen {
                    self.roll_chosen = chosen;
                }
            }
            Message::RollChoose(chosen) => self.roll_chosen = chosen,
            Message::RollAction(action) => self.roll_action(action),
            Message::RollSound { key, on } => self.sound(key, on),
            Message::RollSlide { from, to } => {
                if let Some(from) = from {
                    self.sound(from, false);
                }
                self.sound(to, true);
            }
            Message::RollDone { remember_beats } => {
                self.run = None;
                if let Some(beats) = remember_beats {
                    self.roll_beats = beats;
                }
            }
            Message::RollView(view) => self.roll_view = view,
            Message::ToggleTypingKeys => {
                self.typing_keys = !self.typing_keys;
                if !self.typing_keys {
                    for key in std::mem::take(&mut self.typing) {
                        self.sound(key, false);
                    }
                }
                self.notice = self.typing_keys.then(|| "Your letter keys now play notes on the armed note track. Ctrl+T turns this off, Ctrl+R records.".into());
            }
            Message::TypedKey { key, down } => {
                let fresh = if down { self.typing.insert(key) } else { self.typing.remove(&key) };
                if fresh {
                    self.sound(key, down);
                }
            }
            Message::Both(first, second) => {
                let first = self.handle(*first);
                let second = self.handle(*second);
                return Task::batch([first, second]);
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
            Message::OpenScriptsMenu => {
                self.find_scripts();
                self.overlay = Overlay::ScriptsMenu;
            }
            Message::RunScript(path) => return self.run_script(&path),
            Message::OpenScriptsFolder => self.open_scripts_folder(),
            Message::ScriptKey(key, modifiers) => {
                if self.overlay == Overlay::None && self.screen == Screen::Song {
                    return self.script_key(&key, modifiers);
                }
            }
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
                let tallest = (self.window.height - self.palette.top_bar_height - 1.0 - below).max(mixer::SHORTEST_MIXER);
                self.mixer_height = (self.window.height - below - pointer_y).clamp(mixer::SHORTEST_MIXER, tallest);
            }
            Message::MixerReleased => {
                self.resizing_mixer = false;
                if let Err(why) = settings::save("mixer_height", &self.mixer_height.round().to_string()) {
                    self.problem = Some(format!("Could not save settings: {why}"));
                }
            }
            Message::WheelOverFader(level, delta) => {
                let notches = match delta {
                    iced::mouse::ScrollDelta::Lines { y, .. } => y.signum() * (y != 0.0) as u8 as f32,
                    iced::mouse::ScrollDelta::Pixels { y, .. } => y / 50.0,
                };
                if notches != 0.0 {
                    match level {
                        mixer::Level::Track(track) => {
                            let now = self.project.tracks.iter().find(|t| t.id == track).map(|t| t.gain).unwrap_or(1.0);
                            let db = (mixer::db_from_gain(now) + notches * FADER_STEP_DB)
                                .clamp(mixer::SILENT_DB, mixer::LOUDEST_DB);
                            let gain = mixer::gain_from_db(db);
                            self.edit(Some(Run::TrackGain(track)), Command::SetTrackGain { track, gain });
                        }
                        mixer::Level::Master => {
                            let percent = (self.project.master * 100.0 + notches * MASTER_STEP_PERCENT)
                                .clamp(0.0, mixer::LOUDEST_MASTER_PERCENT);
                            self.edit(Some(Run::Master), Command::SetMasterGain(percent / 100.0));
                        }
                        mixer::Level::Clip(_) => {}
                    }
                }
            }
            Message::TrackGain(track, db) => {
                let gain = mixer::gain_from_db(db);
                let name = self.project.track(track).map(|found| found.name.clone()).unwrap_or_default();
                self.reading = Some(format!("{name}  {db:+.1} dB"));
                let target = loupe_engine::Target::TrackGain(track);
                match self.writing_to(target) {
                    Some(mode) => self.write_point(target, gain, mode),
                    None => {
                        self.edit(Some(Run::TrackGain(track)), Command::SetTrackGain { track, gain });
                    }
                }
            }
            Message::SetTool(_) if matches!(self.overlay, Overlay::Roll(_)) => {}
            Message::SetTool(tool) => {
                self.tool = tool;
                self.cache.clear();
            }
            Message::Refresh => {}
            Message::PaintMute { clip, muted } => self.mute_clips(self.affected_by(clip), muted),
            Message::PaintDelete(clip) => self.delete_clips(self.affected_by(clip), Some(Run::Paint)),
            Message::StretchClip { clip, start, len } => self.stretch_clip(clip, start, len),
            Message::Stretched { source, stretch, made } => self.stretched(source, stretch, made),
            Message::ChainName(typed) => self.chain_name = typed,
            Message::SaveChain => self.save_chain(),
            Message::UseChain(file) => self.use_chain(file),
            Message::OpenMasterPlugins => {
                self.refresh_chains();
                self.plugin_filter.clear();
                self.plugin_highlight = 0;
                self.overlay = Overlay::MasterPlugins;
                return text_input::focus(plugins::FILTER_ID);
            }
            Message::AddMasterPlugin(which) => {
                if let Some(plugin) = self.found.get(which).cloned() {
                    let fx = loupe_engine::Fx {
                        path: plugin.path.clone(),
                        index: plugin.index,
                        name: plugin.name.clone(),
                        bypassed: false,
                        state: Vec::new(),
                        record: false,
                    };
                    self.overlay = Overlay::None;
                    self.plugin_uses.reached_for(&plugin.name);
                    self.edit(None, Command::AddMasterFx(fx));
                }
            }
            Message::RemoveMasterPlugin(slot) => {
                self.edit(None, Command::RemoveMasterFx(slot));
            }
            Message::BypassMasterPlugin(slot) => {
                if let Some(bypassed) = self.project.master_fx.get(slot).map(|fx| !fx.bypassed) {
                    self.edit(None, Command::BypassMasterFx { slot, bypassed });
                }
            }
            Message::ShowMasterPlugin(slot) => self.open_master_window(slot),
            Message::MidiInputToggled(port) => self.toggle_midi_input(port),
            Message::ToggleSnap => {
                self.snap = !self.snap;
                if let Err(why) = settings::save("snap", if self.snap { "on" } else { "off" }) {
                    self.problem = Some(format!("Could not save settings: {why}"));
                }
            }
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
                self.reading = Some(format!("Master  {percent:.0}%"));
                let target = loupe_engine::Target::MasterGain;
                match self.writing_to(target) {
                    Some(mode) => self.write_point(target, percent / 100.0, mode),
                    None => {
                        self.edit(Some(Run::Master), Command::SetMasterGain(percent / 100.0));
                    }
                }
            }
            Message::ToggleMasterMute => {
                self.edit(None, Command::ToggleMasterMute);
            }
            Message::ToggleCollapsed(track) => {
                self.edit(None, Command::ToggleCollapsed(track));
            }
            Message::SetTrackParent { track, parent } => {
                if matches!(self.overlay, Overlay::TrackMenu { .. }) {
                    self.overlay = Overlay::None;
                }
                self.edit(None, Command::SetTrackParent { track, parent });
            }
            Message::OpenRouting(track) => self.overlay = Overlay::Routing(track),
            Message::OpenPlugins(track) => {
                self.refresh_chains();
                self.plugin_filter.clear();
                self.plugin_highlight = 0;
                self.overlay = Overlay::Plugins(track);
                return text_input::focus(plugins::FILTER_ID);
            }
            Message::PluginFilter(typed) => {
                self.plugin_filter = typed;
                self.plugin_highlight = 0;
            }
            Message::PluginsFound(found) => {
                self.found = found;
                self.scanning = false;
            }
            Message::AddPlugin(track, which) => {
                if let Some(plugin) = self.found.get(which).cloned() {
                    let fx = loupe_engine::Fx {
                        path: plugin.path.clone(),
                        index: plugin.index,
                        name: plugin.name.clone(),
                        bypassed: false,
                        state: Vec::new(),
                        record: self.rec_shown.contains(&track),
                    };
                    self.overlay = Overlay::None;
                    self.plugin_uses.reached_for(&plugin.name);
                    self.edit(None, Command::AddFx { track, fx });
                }
            }
            Message::RemovePlugin(track, slot) => {
                self.edit(None, Command::RemoveFx { track, slot });
            }
            Message::ShowPlugin(track, slot) => self.open_plugin_window(track, slot),
            Message::OpenClipPlugins(clip) => {
                self.refresh_chains();
                self.plugin_filter.clear();
                self.plugin_highlight = 0;
                self.overlay = Overlay::ClipPlugins(clip);
                return text_input::focus(plugins::FILTER_ID);
            }
            Message::AddClipPlugin(clip, which) => {
                if let Some(plugin) = self.found.get(which).cloned() {
                    let fx = loupe_engine::Fx {
                        path: plugin.path.clone(),
                        index: plugin.index,
                        name: plugin.name.clone(),
                        bypassed: false,
                        state: Vec::new(),
                        record: false,
                    };
                    self.overlay = Overlay::Clip(clip);
                    self.plugin_uses.reached_for(&plugin.name);
                    self.edit(None, Command::AddClipFx { clip, fx });
                }
            }
            Message::MoveClipPlugin(clip, slot, to) => {
                let most = self.project.clip(clip).map(|found| found.fx.len()).unwrap_or(0);
                if to < most && to != slot {
                    self.edit(None, Command::MoveClipFx { clip, slot, to });
                }
            }
            Message::ShowClipPlugin(clip, slot) => self.open_clip_plugin_window(clip, slot),
            Message::OpenKnobs(spot, slot) => {
                self.plugin_filter.clear();
                self.overlay = Overlay::Knobs(spot, slot);
                return text_input::focus(plugins::FILTER_ID);
            }
            Message::RemoveClipPlugin(clip, slot) => {
                self.edit(None, Command::RemoveClipFx { clip, slot });
            }
            Message::BypassClipPlugin(clip, slot) => {
                let bypassed = self
                    .project
                    .clip(clip)
                    .and_then(|found| found.fx.get(slot))
                    .map(|fx| !fx.bypassed)
                    .unwrap_or(false);
                self.edit(None, Command::BypassClipFx { clip, slot, bypassed });
            }
            Message::PutPoint { target, at, value } => {
                let point = loupe_engine::Point { at, value, shape: loupe_engine::Shape::Linear };
                self.edit(Some(Run::Point(target)), Command::PutPoint { target, point });
            }
            Message::DragPoint { target, which, at, value } => {
                let shape = self.project.envelope(target).and_then(|found| found.points.get(which)).map(|point| point.shape);
                let shape = shape.unwrap_or(loupe_engine::Shape::Linear);
                self.edit(Some(Run::Point(target)), Command::DropPoint { target, which });
                let point = loupe_engine::Point { at, value, shape };
                self.edit(Some(Run::Point(target)), Command::PutPoint { target, point });
            }
            Message::DropPoint { target, which } => {
                self.edit(None, Command::DropPoint { target, which });
            }
            Message::PluginHighlight(step) => {
                let count = self.in_the_picker().len();
                if count > 0 {
                    let last = count - 1;
                    self.plugin_highlight = (self.plugin_highlight as i32 + step).clamp(0, last as i32) as usize;
                }
            }
            Message::Hint(words) => self.hint = words,
            Message::PluginChosen => {
                let picked = self.in_the_picker().get(self.plugin_highlight).copied();
                let next = match (picked, &self.overlay) {
                    (Some(which), Overlay::Plugins(track)) => Some(Message::AddPlugin(*track, which)),
                    (Some(which), Overlay::ClipPlugins(clip)) => Some(Message::AddClipPlugin(*clip, which)),
                    _ => None,
                };
                if let Some(next) = next {
                    return self.update(next);
                }
            }
            Message::OpenAutomation(target) => {
                self.overlay = Overlay::Automation(target);
            }
            Message::AddEnvelope(target) => {
                self.edit(None, Command::AddEnvelope { target });
            }
            Message::RemoveEnvelope(target) => {
                self.edit(None, Command::RemoveEnvelope { target });
            }
            Message::ArmEnvelope(target, mode) => {
                self.overlay = Overlay::None;
                self.writing = None;
                self.edit(None, Command::ArmEnvelope { target, mode });
            }
            Message::FxGrab(track, slot) => self.fx_drag = Some((track, slot, slot)),
            Message::FxOver(slot) => {
                if let Some((_, _, over)) = self.fx_drag.as_mut() {
                    *over = slot;
                }
            }
            Message::FxDrop => {
                if let Some((track, from, to)) = self.fx_drag.take() {
                    if from == to {
                        self.open_plugin_window(track, from);
                    } else {
                        self.edit(None, Command::MoveFx { track, slot: from, to });
                    }
                }
            }
            Message::StockTurned(change) => {
                let changes = self.stock.as_mut().map(|window| window.turned(change)).unwrap_or_default();
                self.plugin_changed(changes);
            }
            Message::StockTold(told) => {
                let changes = self.stock.as_mut().map(|window| window.told(told)).unwrap_or_default();
                self.plugin_changed(changes);
            }
            Message::BypassPlugin(track, slot) => {
                let bypassed = self
                    .project
                    .tracks
                    .iter()
                    .find(|t| t.id == track)
                    .and_then(|t| t.fx.get(slot))
                    .map(|fx| !fx.bypassed)
                    .unwrap_or(false);
                self.edit(None, Command::BypassFx { track, slot, bypassed });
            }
            Message::OpenMatrix => self.overlay = Overlay::Matrix,
            Message::AddSend { from, to } => {
                self.edit(None, Command::AddSend { from, to });
            }
            Message::RemoveSend { from, to } => {
                self.edit(None, Command::RemoveSend { from, to });
            }
            Message::SendGain { from, to, gain } => {
                self.edit(Some(Run::Send(from, to)), Command::SetSendGain { from, to, gain });
            }
            Message::SendPreFader { from, to, pre_fader } => {
                self.edit(None, Command::SetSendPreFader { from, to, pre_fader });
            }
            Message::SendSidechain { from, to, sidechain } => {
                self.edit(None, Command::SetSendSidechain { from, to, sidechain });
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
            Message::OpenProject => {
                self.open_next_safely = self.wants_safe_open();
                return self.ask_to_open();
            }
            Message::OpenWithPluginsOff => {
                self.open_next_safely = true;
                return self.ask_to_open();
            }
            Message::PluginsBackOn => self.plugins_back_on(),
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
                self.set_plugins_off(false);
                self.replace_project(Project::new(self.project.rate), HashMap::new());
                self.overlay = Overlay::None;
                self.screen = Screen::Song;
            }
            Message::NewFromTemplate(template) => return self.read_project(template, true, self.wants_safe_open()),
            Message::OpenRecent(project) => return self.read_project(project, false, self.wants_safe_open()),
            Message::SaveAsTemplate => {
                self.overlay = Overlay::TemplateName;
                self.entry = self.path.as_deref().map(home::stem).unwrap_or_default();
                return Task::batch([text_input::focus(menus::ENTRY_ID), text_input::select_all(menus::ENTRY_ID)]);
            }
            Message::ProjectPicked(path) => {
                let safely = std::mem::take(&mut self.open_next_safely) || self.wants_safe_open();
                if let Some(path) = path {
                    return self.read_project(path, false, safely);
                }
            }
            Message::ProjectRead(path, as_template, safely, result) => {
                self.loading = self.loading.saturating_sub(1);
                self.opening = None;
                match result {
                    Ok(opened) => {
                        self.set_plugins_off(safely);
                        self.adopt(path, opened, as_template);
                        if let Some(original) = self.recovering.take() {
                            self.path = original;
                            self.dirty = true;
                            self.revision += 1;
                        }
                    }
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
            Message::ExportSplit(split) => self.choose_export(|choices| choices.split = split),
            Message::ExportFormatChosen(format) => self.choose_export(|choices| choices.format = format),
            Message::ExportDither(dither) => self.choose_export(|choices| match choices.format.bits() {
                Some(16) => choices.dither_16 = dither,
                _ => choices.dither_24 = dither,
            }),
            Message::ExportNormaliseChosen(normalise) => self.choose_export(|choices| choices.normalise = normalise),
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
                    Ok((folder, None)) => self.notice = Some(format!("Exported to {}", folder.display())),
                    Ok((folder, Some(note))) => self.notice = Some(format!("Exported to {}. {note}", folder.display())),
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
            Message::Autosave => return self.autosave(),
            Message::AutosaveDone(revision, result) => match result {
                Ok(_) => self.backed_up = self.backed_up.max(revision),
                Err(why) => self.problem = Some(format!("Could not autosave: {why}")),
            },
            Message::AutosaveTyped(typed) => {
                if settings::typed_number(&typed) {
                    self.autosave_text = typed;
                    self.autosave_problem = None;
                }
            }
            Message::AutosaveEntered => {
                match settings::autosave_minutes_from(&self.autosave_text) {
                    Ok(minutes) => {
                        self.autosave_minutes = minutes;
                        self.autosave_text = minutes.to_string();
                        self.autosave_problem = settings::save("autosave_minutes", &minutes.to_string()).err();
                    }
                    Err(why) => self.autosave_problem = Some(why),
                }
                return unfocus();
            }
            Message::KeptTyped(typed) => {
                if settings::typed_number(&typed) {
                    self.kept_text = typed;
                    self.autosave_problem = None;
                }
            }
            Message::KeptEntered => {
                match settings::backups_kept_from(&self.kept_text) {
                    Ok(kept) => {
                        self.backups_kept = kept;
                        self.kept_text = kept.to_string();
                        self.autosave_problem = settings::save("backups_kept", &kept.to_string()).err();
                    }
                    Err(why) => self.autosave_problem = Some(why),
                }
                return unfocus();
            }
            Message::Recover => return self.recover(),
            Message::SkipRecovery => self.skip_recovery(),
            Message::OpenVersions => self.overlay = Overlay::About,
            Message::CheckForUpdates => return self.check_for_updates(),
            Message::UpdateChecked(result) => {
                let quiet = std::mem::replace(&mut self.quiet_check, false);
                self.update_state = match result {
                    Err(_) if quiet => versions::UpdateState::Idle,
                    Ok(None) if quiet => versions::UpdateState::Idle,
                    Ok(Some(update)) => versions::UpdateState::Found(update),
                    Ok(None) => versions::UpdateState::Current,
                    Err(why) => versions::UpdateState::Failed(format!("Could not check for updates: {why}.")),
                };
            }
            Message::InstallUpdate => return self.install_update(),
            Message::UpdateLater => self.update_dismissed = true,
            Message::UsageToggled(on) => self.set_usage(on),
            Message::CrashNoteTyped(note) => self.crash_note = note,
            Message::SendCrashReport => return self.send_crash_report(),
            Message::CrashReportSent(result) => self.crash_report_sent(result),
            Message::SkipCrashReport => self.skip_crash_report(),
            Message::TurnFallenOff => self.settle_fallen(true),
            Message::LeaveFallenOn => self.settle_fallen(false),
            Message::CheckUpdatesOnStart(on) => {
                self.check_updates = on;
                if let Err(why) = settings::save("check_updates", if on { "on" } else { "off" }) {
                    self.problem = Some(format!("Could not save settings: {why}"));
                }
            }
            Message::UpdateDownloaded(result) => match result {
                Ok(setup) => return self.run_setup(setup),
                Err(why) => self.update_state = versions::UpdateState::Failed(format!("Could not download the update: {why}.")),
            },
            Message::SettingsTab(tab) => {
                self.settings_tab = tab;
                if tab == SettingsTab::Audio {
                    self.refresh_audio_lists();
                }
            }
            Message::AudioDriverChosen(driver) => {
                let chosen = Device { driver: Some(driver.0), output: None, ..self.audio.clone() };
                if chosen.driver.as_deref() == Some("ASIO") && chosen != self.audio {
                    self.audio = chosen;
                    let _ = settings::save("audio_driver", "ASIO");
                    let _ = settings::forget("audio_output");
                    self.refresh_audio_lists();
                    self.notice = Some("Now pick your ASIO device under Output.".into());
                    return Task::none();
                }
                return self.choose_audio(chosen);
            }
            Message::AudioOutputChosen(name) => {
                let output = (name != audio_settings::SYSTEM_OUTPUT).then_some(name);
                return self.choose_audio(Device { output, ..self.audio.clone() });
            }
            Message::AudioRateChosen(rate) => return self.choose_audio(Device { rate: rate.0, ..self.audio.clone() }),
            Message::AudioBufferChosen(buffer) => return self.choose_audio(Device { buffer: buffer.size, ..self.audio.clone() }),
            Message::Reopened(result) => self.reopened(result),
            Message::ToggleMetronome => {
                self.metronome = !self.metronome;
                self.engine.set_metronome(self.metronome);
                if let Err(why) = settings::save("metronome", if self.metronome { "on" } else { "off" }) {
                    self.problem = Some(format!("Could not save settings: {why}"));
                }
            }
            Message::PrerollChosen(CountIn(bars)) => {
                self.preroll_bars = bars;
                if let Err(why) = settings::save("preroll", &bars.to_string()) {
                    self.problem = Some(format!("Could not save settings: {why}"));
                }
            }
            Message::TogglePunch => {
                self.punch = !self.punch;
                if let Err(why) = settings::save("punch", if self.punch { "on" } else { "off" }) {
                    self.problem = Some(format!("Could not save settings: {why}"));
                }
                self.notice = self.punch.then(|| "Punch is on: mark the part to redo on the ruler, then record.".to_string());
                self.cache.clear();
            }
            Message::CountInChosen(CountIn(bars)) => {
                self.count_in_bars = bars;
                if let Err(why) = settings::save("count_in", &bars.to_string()) {
                    self.problem = Some(format!("Could not save settings: {why}"));
                }
            }
            Message::ThemeChosen(name) => return self.use_theme(name),
            Message::ThemeFontLoaded => self.cache.clear(),
            Message::ShowThemes => self.show_themes_folder(),
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        // The arrows move notes in the piano roll, so they only become shortcuts while it is open.
        let in_the_roll = matches!(self.overlay, Overlay::Roll(_));
        let typing_keys = self.typing_keys;
        let shortcuts = keyboard::on_key_press(if typing_keys { shortcut_while_typing } else { shortcut });
        // The arrows move the chosen notes, so they are only shortcuts while the roll is open.
        let roll_keys = if in_the_roll { keyboard::on_key_press(transpose_key) } else { Subscription::none() };
        let window = iced::event::listen_with(|event, _status, _window| match event {
            iced::Event::Window(window::Event::FileDropped(path)) => Some(Message::Dropped(path)),
            iced::Event::Window(window::Event::Resized(size)) => Some(Message::Resized(size)),
            iced::Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => Some(Message::ModifiersChanged(modifiers)),
            _ => None,
        });
        let watching = self.exporting
            || self.copied.is_some()
            || self.input.is_some()
            || self.opening.is_some()
            || self.stock.is_some()
            || self.plugins_opening
            || self.crash_sheet_due
            || self.master_level > 0.0005;
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
        let picking = if matches!(self.overlay, Overlay::Plugins(_) | Overlay::ClipPlugins(_)) {
            keyboard::on_key_press(|key, _modifiers| match key {
                keyboard::Key::Named(keyboard::key::Named::ArrowDown) => Some(Message::PluginHighlight(1)),
                keyboard::Key::Named(keyboard::key::Named::ArrowUp) => Some(Message::PluginHighlight(-1)),
                keyboard::Key::Named(keyboard::key::Named::Enter) => Some(Message::PluginChosen),
                _ => None,
            })
        } else {
            Subscription::none()
        };
        let autosave = match self.autosave_minutes {
            0 => Subscription::none(),
            minutes => iced::time::every(Duration::from_secs(minutes as u64 * 60)).map(|_| Message::Autosave),
        };
        let typing = if matches!(self.overlay, Overlay::Roll(_)) || self.typing_keys {
            iced::event::listen_with(|event, status, _window| match event {
                iced::Event::Keyboard(keyboard::Event::KeyPressed { key: keyboard::Key::Character(c), modifiers, .. })
                    if !modifiers.command() && status == iced::event::Status::Ignored =>
                {
                    piano_roll::typed_key(c.as_str()).map(|key| Message::TypedKey { key, down: true })
                }
                iced::Event::Keyboard(keyboard::Event::KeyReleased { key: keyboard::Key::Character(c), .. }) => {
                    piano_roll::typed_key(c.as_str()).map(|key| Message::TypedKey { key, down: false })
                }
                _ => None,
            })
        } else {
            Subscription::none()
        };
        Subscription::batch([shortcuts, window, ticks, mixer_drag, typing, autosave, picking, roll_keys])
    }

    fn edit(&mut self, run: Option<Run>, command: Command) -> Option<Outcome> {
        self.transact(run, |project| project.apply(command))
    }

    fn remember(&mut self, before: Project, run: Option<Run>) {
        if run.is_none() || run != self.run {
            self.undo.push(before);
            if self.undo.len() > UNDO_STEPS {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.run = run;
        self.dirty = true;
        self.revision += 1;
        self.engine.set_project(&self.project);
        self.cache.clear();
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
                self.revision += 1;
                self.engine.set_project(&self.project);
                self.chains_if_changed();
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
        self.chains_if_changed();
    }

    fn chains_if_changed(&mut self) {
        let shape = self.fx_shape();
        if shape != self.fx_was {
            self.fx_was = shape;
            self.follow_chains();
        }
    }

    pub(crate) fn gather_fx_state(&mut self) {
        let bare = self.project.tracks.iter().all(|track| track.fx.is_empty())
            && self.project.tracks.iter().flat_map(|track| track.clips.iter()).all(|clip| clip.fx.is_empty());
        if bare {
            return;
        }
        let Some(mut racks) = self.borrow_racks() else { return };
        let found = racks.harvest();
        let on_clips = racks.harvest_clips();
        self.racks = Some(racks);
        for (track, slot, state) in found {
            let _ = self.project.apply(Command::SetFxState { track, slot, state });
        }
        for (clip, slot, state) in on_clips {
            let _ = self.project.apply(Command::SetClipFxState { clip, slot, state });
        }
        self.hand_racks_over();
    }

    pub(crate) fn offline_racks(&self, project: &Project) -> Box<dyn Chains> {
        let mut racks: Box<dyn Chains> =
            Box::new(racks::Racks::new(self.engine.rate(), 512, racks::Peeks::default(), self.plugins_off.clone(), self.falls.clone()));
        racks.follow(project);
        racks
    }

    fn borrow_racks(&mut self) -> Option<Box<dyn Chains>> {
        if let Some(racks) = self.racks.take() {
            return Some(racks);
        }
        self.engine.drop_chains();
        for _ in 0..500 {
            if let Some(got) = self.engine.chains_back() {
                return Some(got);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        None
    }

    fn hand_racks_over(&mut self) {
        if let Some(mut racks) = self.racks.take() {
            racks.nudge();
            self.engine.use_chains(racks);
        }
        self.take_falls();
    }

    fn open_plugin_window(&mut self, track: TrackId, slot: usize) {
        if self.plugins_are_off() {
            self.problem = Some(safe_mode::PLUGINS_OFF_PROBLEM.into());
            return;
        }
        let Some(fx) = self.project.tracks.iter().find(|t| t.id == track).and_then(|t| t.fx.get(slot)).cloned() else {
            return;
        };
        if !loupe_plugins::rack::is_built_in(&fx.path) {
            // One borrow for both: settling the chains and then opening the window.
            // Handing them back between the two can leave the second borrow empty,
            // and the window request would be dropped without a word.
            if let Some(mut racks) = self.borrow_racks() {
                racks.follow(&self.project);
                if let Err(why) = racks.show(track, slot) {
                    self.problem = Some(why);
                }
                self.plugins_opening = racks.still_opening();
                self.racks = Some(racks);
                self.hand_racks_over();
            } else {
                self.problem = Some("Loupe could not reach its plugins just now. Try again.".into());
            }
            return;
        }
        let values: Vec<f32> = fx.state.chunks_exact(4).map(|four| f32::from_le_bytes([four[0], four[1], four[2], four[3]])).collect();
        let peek = self.peek_at(racks::Spot::Track(track, slot));
        let rate = self.engine.rate() as f32;
        let bpm = self.project.bpm as f32;
        match stockwin::Window::open(stockwin::Spot::Track(track), slot, fx.index, &fx.name, &values, peek, rate, bpm) {
            Some(window) => {
                self.stock = Some(window);
                self.overlay = Overlay::Stock;
            }
            None => self.problem = Some("Loupe has no window for that plugin".into()),
        }
    }

    fn peek_at(&self, spot: racks::Spot) -> racks::Peek {
        self.peeks.lock().ok().and_then(|held| held.get(&spot).cloned()).unwrap_or_default()
    }

    pub(crate) fn open_clip_plugin_window(&mut self, clip: ClipId, slot: usize) {
        if self.plugins_are_off() {
            self.problem = Some(safe_mode::PLUGINS_OFF_PROBLEM.into());
            return;
        }
        let Some(fx) = self.project.clip(clip).and_then(|found| found.fx.get(slot)).cloned() else {
            return;
        };
        if !loupe_plugins::rack::is_built_in(&fx.path) {
            if let Some(mut racks) = self.borrow_racks() {
                if let Err(why) = racks.show_clip(clip, slot) {
                    self.problem = Some(why);
                }
                self.racks = Some(racks);
                self.hand_racks_over();
            }
            return;
        }
        let values: Vec<f32> = fx.state.chunks_exact(4).map(|four| f32::from_le_bytes([four[0], four[1], four[2], four[3]])).collect();
        let peek = self.peek_at(racks::Spot::Clip(clip, slot));
        let rate = self.engine.rate() as f32;
        let bpm = self.project.bpm as f32;
        let spot = stockwin::Spot::Clip(clip);
        match stockwin::Window::open(spot, slot, fx.index, &fx.name, &values, peek, rate, bpm) {
            Some(window) => {
                self.stock = Some(window);
                self.overlay = Overlay::Stock;
            }
            None => self.problem = Some("Loupe has no window for that plugin".into()),
        }
    }

    fn plugin_changed(&mut self, changes: Vec<(usize, f32)>) {
        let Some(window) = self.stock.as_ref() else { return };
        let (spot, slot) = (window.spot, window.slot);
        if changes.is_empty() {
            return;
        }
        let mut state = match spot {
            stockwin::Spot::Track(track) => self
                .project
                .tracks
                .iter()
                .find(|t| t.id == track)
                .and_then(|t| t.fx.get(slot))
                .map(|fx| fx.state.clone())
                .unwrap_or_default(),
            stockwin::Spot::Clip(clip) => self
                .project
                .clip(clip)
                .and_then(|found| found.fx.get(slot))
                .map(|fx| fx.state.clone())
                .unwrap_or_default(),
            stockwin::Spot::Master => self.project.master_fx.get(slot).map(|fx| fx.state.clone()).unwrap_or_default(),
        };
        for (knob, value) in &changes {
            match spot {
                stockwin::Spot::Track(track) => self.engine.tweak(track, slot, *knob, *value),
                stockwin::Spot::Clip(clip) => self.engine.tweak_clip(clip, slot, *knob, *value),
                // Master plugins take their new settings when the rack next settles.
                stockwin::Spot::Master => {}
            }
            let at = knob * 4;
            if state.len() < at + 4 {
                state.resize(at + 4, 0);
            }
            state[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        let _ = match spot {
            stockwin::Spot::Track(track) => self.project.apply(Command::SetFxState { track, slot, state }),
            stockwin::Spot::Clip(clip) => self.project.apply(Command::SetClipFxState { clip, slot, state }),
            stockwin::Spot::Master => self.project.apply(Command::SetMasterFxState { slot, state }),
        };
        self.dirty = true;
        self.revision += 1;
    }

    fn writing_to(&self, target: loupe_engine::Target) -> Option<loupe_engine::Mode> {
        if !self.playing {
            return None;
        }
        self.project.envelope(target).and_then(|shape| shape.armed)
    }

    pub(crate) fn write_point(&mut self, target: loupe_engine::Target, value: f32, mode: loupe_engine::Mode) {
        let at = self.playhead;
        let mut writer = self.writing.filter(|held| held.target == target);
        let before = self.project.clone();
        match self.project.envelopes.iter_mut().find(|shape| shape.target == target) {
            Some(shape) => match writer.as_mut() {
                Some(writer) => shape.go_on_writing(writer, at, value),
                None => writer = Some(shape.start_writing(mode, at, value)),
            },
            None => return,
        }
        self.writing = writer;
        self.remember(before, Some(Run::Point(target)));
    }

    pub(crate) fn keep_writing(&mut self) {
        let Some(writer) = self.writing else { return };
        if writer.mode != loupe_engine::Mode::Latch || !self.playing {
            return;
        }
        self.write_point(writer.target, writer.value, loupe_engine::Mode::Latch);
    }

    pub(crate) fn stop_writing(&mut self) {
        let Some(writer) = self.writing.take() else { return };
        let at = self.playhead;
        let before = self.project.clone();
        if let Some(shape) = self.project.envelopes.iter_mut().find(|shape| shape.target == writer.target) {
            shape.stop_writing(&writer, at);
        }
        if writer.mode == loupe_engine::Mode::Latch {
            self.writing = Some(writer);
            return;
        }
        self.remember(before, None);
    }

    fn fx_shape(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for track in &self.project.tracks {
            if track.fx.is_empty() {
                continue;
            }
            track.id.hash(&mut hasher);
            for fx in &track.fx {
                fx.path.hash(&mut hasher);
                fx.index.hash(&mut hasher);
                fx.bypassed.hash(&mut hasher);
            }
        }
        for clip in self.project.tracks.iter().flat_map(|track| track.clips.iter()) {
            if clip.fx.is_empty() {
                continue;
            }
            clip.id.hash(&mut hasher);
            clip.start.hash(&mut hasher);
            clip.offset.hash(&mut hasher);
            clip.len.hash(&mut hasher);
            clip.stretch.to_bits().hash(&mut hasher);
            clip.source.path.hash(&mut hasher);
            self.project.bpm.to_bits().hash(&mut hasher);
            for fx in &clip.fx {
                fx.path.hash(&mut hasher);
                fx.index.hash(&mut hasher);
                fx.bypassed.hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    fn follow_chains(&mut self) {
        let mut racks = match self.borrow_racks() {
            Some(racks) => racks,
            None => Box::new(racks::Racks::new(self.engine.rate(), 512, self.peeks.clone(), self.plugins_off.clone(), self.falls.clone())) as Box<dyn Chains>,
        };
        let troubles = racks.follow(&self.project);
        self.note_plugins_for_crashes();
        self.plugins_opening = racks.still_opening();
        self.knob_names.clear();
        if let Ok(held) = self.peeks.lock() {
            for (spot, peek) in held.iter() {
                let (owner, slot, on_clip) = match spot {
                    racks::Spot::Track(track, slot) => (track.0, *slot, false),
                    racks::Spot::Clip(clip, slot) => (clip.0, *slot, true),
                    // Master has no track id, so it uses one no track can have.
                    racks::Spot::Master(slot) => (MASTER_OWNER, *slot, false),
                };
                for (knob, name) in peek.knobs.iter().enumerate() {
                    self.knob_names.insert((owner, slot, knob, on_clip), name.clone());
                }
            }
        }
        if let Some(first) = troubles.first() {
            self.problem = Some(first.clone());
        }
        self.engine.use_chains(racks);
        self.take_falls();
    }

    fn restored(&mut self) {
        self.run = None;
        self.dirty = true;
        self.revision += 1;
        self.forget_gone_clips();
        self.bpm = format_bpm(self.project.bpm);
        self.changed();
    }

    fn input_choice(&self) -> InputChoice {
        match (self.practice_input, &self.input_name) {
            (true, _) => InputChoice::Practice,
            (false, Some(name)) => InputChoice::Named(name.clone()),
            (false, None) => InputChoice::SystemDefault,
        }
    }

    pub(crate) fn listen_if_armed(&mut self) {
        let heard: Vec<TrackId> = self.project.tracks.iter().filter(|track| self.armed.contains(&track.id) && !track.records_notes).map(|track| track.id).collect();
        if heard.is_empty() {
            self.engine.hear_on(&[]);
            self.engine.stop_hearing();
            self.input = None;
            self.input_levels.clear();
            return;
        }
        // You only hear yourself while the tape is rolling. Armed and stopped, or
        // armed and playing back, your microphone stays out of the mix.
        let rolling = self.recording.is_some();
        self.engine.hear_on(if self.hear_input && rolling { &heard } else { &[] });
        if self.input.is_some() {
            return;
        }
        match Input::open(self.input_choice()) {
            Ok(mut input) => {
                self.engine.hear(&mut input);
                self.input = Some(input);
            }
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
        loupe_plugins::presets::keep_in(settings::presets_folder(self.folder.as_deref()));
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
        (self.window.width - beside).max(self.palette.header_width + 1.0)
    }

    fn show_whole_song(&mut self) {
        let seconds = self.project.length() as f64 / self.project.rate.max(1) as f64;
        let room = (self.canvas_width() - self.palette.header_width - 48.0).max(100.0) as f64;
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
        let width = (self.canvas_width() - self.palette.header_width) as f64;
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
            knobs: &self.knob_names,
            project: &self.project,
            palette: &self.palette,
            view: self.view,
            heights: &self.heights,
            selected: self.selected,
            selection: &self.selection,
            playhead: self.playhead,
            loop_range: self.loop_range,
            punch: self.punch,
            tool: self.tool,
            snap: self.snap,
            armed: &self.armed,
            recording_from: self.recording.as_ref().map(|recording| recording.from),
            input_levels: &self.input_levels,
            opening: self.opening.is_some(),
            width: self.canvas_width(),
            cache: &self.cache,
        })
        .width(Length::Fill)
        .height(Length::Fill);

        let palette = self.palette;
        let middle = match (self.pool_open, palette.all_audio) {
            (false, _) => row![timeline],
            (true, Side::Right) => row![timeline, upright_rule(palette), self.pool()],
            (true, Side::Left) => row![self.pool(), upright_rule(palette), timeline],
        };
        let mut song = column![self.transport(), rule(palette)];
        if let Some(banner) = self.safe_banner() {
            song = song.push(banner).push(rule(palette));
        }
        let lid = container(Space::new(Length::Fill, 9)).style(move |_| palette.shadow_below(1.3));
        song = song.push(stack![middle.height(Length::Fill), lid]);
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

    pub(crate) fn file_button(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let file_menu_open = self.overlay == Overlay::FileMenu;
        button(
            row![text("File").size(13).font(palette.medium), icon("chevron-down", 12.0)]
                .spacing(5)
                .align_y(Alignment::Center),
        )
        .padding([6, 10])
        .style(move |_, status| palette.toggled(file_menu_open, status))
        .on_press(Message::OpenFileMenu)
        .into()
    }

    pub(crate) fn help_button(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let help_menu_open = self.overlay == Overlay::HelpMenu;
        button(text("Help").size(13).font(palette.medium))
            .padding([6, 10])
            .style(move |_, status| palette.toggled(help_menu_open, status))
            .on_press(Message::OpenHelpMenu)
            .into()
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

        let metronome_on = self.metronome;
        let metronome = button(container(icon("metronome", 15.0)).center(30))
            .padding(0)
            .style(move |_, status| palette.toggled(metronome_on, status))
            .on_press(Message::ToggleMetronome);
        let punch_on = self.punch;
        let punch = button(container(text("Punch").size(11.5).font(palette.medium)).center_y(30).padding([0, 8]))
            .padding(0)
            .style(move |_, status| palette.toggled(punch_on, status))
            .on_press(Message::TogglePunch);
        let typing_on = self.typing_keys;
        let typing_keys = button(container(icon("keyboard-music", 15.0)).center(30))
            .padding(0)
            .style(move |_, status| palette.toggled(typing_on, status))
            .on_press(Message::ToggleTypingKeys);
        let record = row![
            hinted(record, "Record onto every armed track. Ctrl+R does the same."),
            hinted(punch, "Punch: record only inside the part marked on the ruler, replacing what was there."),
            hinted(metronome, "The click you record to. Ctrl+M turns it on and off."),
            hinted(typing_keys, "Play notes with your letter keys, A to L like a keyboard. Ctrl+T turns it on and off."),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        let tempo = text_input("", &self.bpm)
            .on_input(Message::BpmTyped)
            .on_submit(Message::BpmEntered)
            .font(palette.mono)
            .size(13)
            .padding([5, 8])
            .width(46)
            .style(move |_, status| palette.field(status));

        let mixer_open = self.mixer_open;

        let scripts_menu_open = self.overlay == Overlay::ScriptsMenu;
        let scripts = button(text("Scripts").size(13).font(palette.medium))
            .padding([6, 10])
            .style(move |_, status| palette.toggled(scripts_menu_open, status))
            .on_press(Message::OpenScriptsMenu);

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
                centred: false,
                on_turn: Box::new(Message::MasterPercent),
            })
            .width(30)
            .height(30),
            container(self.level_readout(mixer::Level::Master, self.project.master)).width(44),
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

        let tempo = row![text("BPM").size(11).font(palette.medium).color(palette.text_dim), tempo]
            .spacing(8)
            .align_y(Alignment::Center);
        let mut pieces: Vec<(BarItem, Element<'_, Message>)> = vec![
            (BarItem::ToStart, icon_button(palette, "skip-back", Some(Message::ToStart))),
            (BarItem::Play, play.into()),
            (BarItem::Record, record.into()),
            (BarItem::Position, text(position).size(13).font(palette.mono).width(52).into()),
            (BarItem::Clock, text(clock).size(13).font(palette.mono).color(palette.text_dim).width(84).into()),
            (BarItem::Tempo, tempo.into()),
            (BarItem::Master, master.into()),
            (BarItem::History, history.into()),
            (BarItem::Mixer, mixer.into()),
            (BarItem::Settings, icon_button(palette, "settings", Some(Message::OpenSettings))),
            (BarItem::Import, import.into()),
        ];
        let mut bar = row![self.file_button(), scripts, self.help_button(), Space::with_width(6)].spacing(8).align_y(Alignment::Center);
        for item in palette.top_bar_items() {
            let piece = match item {
                BarItem::Gap => Some(horizontal_space().into()),
                BarItem::Space => Some(Space::with_width(10).into()),
                _ => pieces.iter().position(|(kind, _)| *kind == item).map(|at| pieces.swap_remove(at).1),
            };
            let piece = piece.map(|piece| match hint_for(item) {
                Some(words) => hinted(piece, words),
                None => piece,
            });
            bar = bar.push_maybe(piece);
        }
        container(bar)
        .padding([0, 16])
        .height(palette.top_bar_height)
        .align_y(Alignment::Center)
        .style(move |_| palette.top_bar())
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
            text("MIDI keyboards").size(13).font(palette.medium),
            text(self.keyboards_found()).size(12).color(palette.text_dim),
            self.midi_picker(),
            text("Hear yourself").size(13).font(palette.medium),
            text("While a take is recording, armed tracks play what the input hears, through their Rec plugins. Use headphones, or speakers will howl.").size(12).color(palette.text_dim),
            iced::widget::checkbox("Hear yourself while recording", self.hear_input)
                .on_toggle(|_| Message::HearInputToggled)
                .text_size(13),
            text("Count in").size(13).font(palette.medium),
            text("Bars of clicks before recording starts, when Loupe is stopped. The take begins where the playhead was.").size(12).color(palette.text_dim),
            pick_list(COUNT_INS, Some(CountIn(self.count_in_bars)), Message::CountInChosen).text_size(13).padding([5, 10]).width(160),
            text("Pre-roll").size(13).font(palette.medium),
            text("With Punch on, how many bars play before the marked part, so you can catch the beat.").size(12).color(palette.text_dim),
            pick_list(COUNT_INS, Some(CountIn(self.preroll_bars)), Message::PrerollChosen).text_size(13).padding([5, 10]).width(160),
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
            SettingsTab::Display => column![self.theme_picker(), rule(palette), scale].spacing(16),
            SettingsTab::File => column![folder, rule(palette), self.autosave_settings()].spacing(16),
            SettingsTab::Recording => recording,
            SettingsTab::Audio => column![self.audio_settings()],
            SettingsTab::Privacy => column![self.privacy_settings(), rule(palette), self.update_settings()].spacing(16),
        };
        let body = column![tabs, rule(palette), container(page).height(SETTINGS_PAGE_HEIGHT)].spacing(14);
        self.window("Settings".to_string(), body.into(), 560.0)
    }

    fn status(&self) -> Option<Element<'_, Message>> {
        let palette = self.palette;
        let line: Element<'_, Message> = if let Some(problem) = self.problem.as_ref().or(self.startup_problem.as_ref()).or(self.theme_problem.as_ref()) {
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
        } else if let (versions::UpdateState::Found(update), false) = (&self.update_state, self.update_dismissed) {
            row![
                text(format!("Loupe {} is available.", update.version)).size(12).color(palette.text),
                button(text("Update").size(12).font(palette.medium)).padding([3, 12]).style(move |_, status| palette.solid(status)).on_press(Message::InstallUpdate),
                button(text("What's new").size(12)).padding([3, 10]).style(move |_, status| palette.ghost(status)).on_press(Message::OpenVersions),
                button(text("Later").size(12)).padding([3, 10]).style(move |_, status| palette.ghost(status)).on_press(Message::UpdateLater),
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .into()
        } else if self.loading > 0 {
            let what = if self.loading == 1 { "1 file".into() } else { format!("{} files", self.loading) };
            text(format!("Loading {what}…")).size(12).color(palette.text_dim).into()
        } else if let Some(reading) = &self.reading {
            text(reading.as_str()).size(12).font(palette.mono).color(palette.text).into()
        } else {
            text(self.hint.unwrap_or_default()).size(12).color(palette.text_dim).into()
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

fn shortcut_while_typing(key: keyboard::Key, modifiers: keyboard::Modifiers) -> Option<Message> {
    match &key {
        keyboard::Key::Character(_) if !modifiers.command() => None,
        _ => shortcut(key, modifiers),
    }
}

fn shortcut(key: keyboard::Key, modifiers: keyboard::Modifiers) -> Option<Message> {
    use keyboard::key::Named;
    match key {
        keyboard::Key::Named(Named::Space) => Some(Message::TogglePlay),
        keyboard::Key::Named(Named::Home) => Some(Message::ToStart),
        keyboard::Key::Named(Named::Escape) => Some(Message::CloseOverlay),
        keyboard::Key::Named(Named::F6) => Some(Message::ToggleMixer),
        keyboard::Key::Named(Named::F7) => Some(Message::OpenMatrix),
        keyboard::Key::Named(Named::Delete | Named::Backspace) => Some(Message::Delete),
        keyboard::Key::Character(c) => {
            match (c.to_lowercase().as_str(), modifiers.command(), modifiers.shift()) {
                ("s", false, _) => Some(Message::Split),
                ("n", false, _) => Some(Message::ToggleSnap),
                ("r", _, _) => Some(Message::ToggleRecord),
                ("p", false, _) => Some(Message::SetTool(Tool::Pencil)),
                ("c", false, _) => Some(Message::SetTool(Tool::Razor)),
                ("t", false, _) => Some(Message::SetTool(Tool::Mute)),
                ("d", false, _) => Some(Message::SetTool(Tool::Delete)),
                ("k", false, _) => Some(Message::SetTool(Tool::Comp)),
                ("m", true, _) => Some(Message::ToggleMetronome),
                ("t", true, _) => Some(Message::ToggleTypingKeys),
                ("q", true, _) => Some(Message::RollAction(piano_roll::RollAction::Quantize)),
                ("c", true, _) => Some(Message::CopyClips),
                ("x", true, _) => Some(Message::CutClips),
                ("v", true, _) => Some(Message::PasteClips),
                ("d", true, _) => Some(Message::DuplicateClips),
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
                _ if modifiers.command() || modifiers.alt() => Some(Message::ScriptKey(c.to_lowercase(), modifiers)),
                _ => None,
            }
        }
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CountIn(u32);

const COUNT_INS: [CountIn; 4] = [CountIn(settings::COUNT_IN_BARS[0]), CountIn(settings::COUNT_IN_BARS[1]), CountIn(settings::COUNT_IN_BARS[2]), CountIn(settings::COUNT_IN_BARS[3])];

impl std::fmt::Display for CountIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            0 => write!(f, "Off"),
            1 => write!(f, "1 bar"),
            bars => write!(f, "{bars} bars"),
        }
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

pub(crate) fn format_bpm(bpm: f64) -> String {
    let text = format!("{bpm:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn icon(name: &str, size: f32) -> iced::widget::Text<'static> {
    text(icons::glyph(name).to_string())
        .font(icons::font(name))
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

/// What the hint panel says about each thing in the top bar. One line, plain words,
/// the way FL Studio's hint bar and Ableton's info view explain what is under the mouse.
fn hint_for(item: BarItem) -> Option<&'static str> {
    Some(match item {
        BarItem::ToStart => "Jump back to the start of the song",
        BarItem::Play => "Play or pause the song. Space does the same.",
        BarItem::Record => return None,
        BarItem::Position => "Where the playhead is, as bar and beat",
        BarItem::Clock => "Where the playhead is, as minutes and seconds",
        BarItem::Tempo => "The tempo of the song in beats per minute",
        BarItem::Master => "The level everything leaves Loupe at. Right click to automate it.",
        BarItem::History => "Undo and redo your last edits",
        BarItem::Mixer => "Show or hide the mixer",
        BarItem::Settings => "Settings: themes, folders, recording, audio and privacy",
        BarItem::Import => "Bring audio files into the song",
        BarItem::Gap | BarItem::Space | BarItem::End => return None,
    })
}

/// Up and down move the chosen notes by a semitone, or by an octave with shift.
fn transpose_key(key: keyboard::Key, modifiers: keyboard::Modifiers) -> Option<Message> {
    let steps = match key {
        keyboard::Key::Named(keyboard::key::Named::ArrowUp) => 1,
        keyboard::Key::Named(keyboard::key::Named::ArrowDown) => -1,
        _ => return None,
    };
    Some(Message::RollAction(piano_roll::RollAction::Transpose(steps * if modifiers.shift() { 12 } else { 1 })))
}

/// Wraps one thing in the top bar so the hint panel can say what it is.
fn hinted<'a>(piece: impl Into<Element<'a, Message>>, words: &'static str) -> Element<'a, Message> {
    iced::widget::mouse_area(piece.into()).on_enter(Message::Hint(Some(words))).on_exit(Message::Hint(None)).into()
}

impl App {
    /// Opens the window of a plugin over the whole mix.
    fn open_master_window(&mut self, slot: usize) {
        if self.plugins_are_off() {
            self.problem = Some(safe_mode::PLUGINS_OFF_PROBLEM.into());
            return;
        }
        let Some(fx) = self.project.master_fx.get(slot).cloned() else {
            return;
        };
        if loupe_plugins::rack::is_built_in(&fx.path) {
            let values: Vec<f32> = fx.state.chunks_exact(4).map(|four| f32::from_le_bytes([four[0], four[1], four[2], four[3]])).collect();
            let peek = self.peek_at(racks::Spot::Master(slot));
            let rate = self.engine.rate() as f32;
            let bpm = self.project.bpm as f32;
            if let Some(window) = stockwin::Window::open(stockwin::Spot::Master, slot, fx.index, &fx.name, &values, peek, rate, bpm) {
                self.stock = Some(window);
            }
            return;
        }
        // One borrow for both, as above.
        if let Some(mut racks) = self.borrow_racks() {
            racks.follow(&self.project);
            if let Err(why) = racks.show_master(slot) {
                self.problem = Some(why);
            }
            self.plugins_opening = racks.still_opening();
            self.racks = Some(racks);
            self.hand_racks_over();
        } else {
            self.problem = Some("Loupe could not reach its plugins just now. Try again.".into());
        }
    }
}

impl App {
    /// While plugins are opening in the background, let the racks publish what has
    /// arrived and open any window that was asked for early.
    fn nudge_racks(&mut self) {
        let Some(mut racks) = self.borrow_racks() else { return };
        racks.nudge();
        self.plugins_opening = racks.still_opening();
        self.racks = Some(racks);
        self.hand_racks_over();
    }
}
