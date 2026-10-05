use iced::widget::{
    button, center, column, container, horizontal_space, mouse_area, opaque, row, scrollable, text, text_input, Space,
};
use iced::{Alignment, Color, Element, Length, Point};
use loupe_engine::{InputChannels, TrackId};

use crate::{rule, App, Message, Overlay, Screen};

const MENU_WIDTH: f32 = 220.0;
const INPUT_MENU_WIDTH: f32 = 260.0;
const INPUT_MENU_TALLEST: f32 = 250.0;
const TALLEST_MENU: f32 = 290.0;
const EDGE_GAP: f32 = 8.0;
const FILE_MENU_LEFT: f32 = 14.0;
const SCRIPTS_MENU_LEFT: f32 = 72.0;
const VIEW_MENU_LEFT: f32 = 148.0;
const HELP_MENU_LEFT: f32 = 208.0;
const HOME_VIEW_MENU_LEFT: f32 = 76.0;
const HOME_HELP_MENU_LEFT: f32 = 136.0;
const MENU_OVERLAPS_BAR: f32 = 8.0;
pub const ENTRY_ID: &str = "overlay-entry";

impl App {
    pub(crate) fn overlay(&self) -> Element<'_, Message> {
        match &self.overlay {
            Overlay::None => Space::new(0, 0).into(),
            Overlay::Settings => self.centred(self.settings_sheet()),
            Overlay::ConfirmDiscard(_) => self.centred(self.discard_sheet()),
            Overlay::TemplateName => self.centred(self.template_sheet()),
            Overlay::SaveName => self.centred(self.save_sheet()),
            Overlay::Export => self.centred(self.export_sheet()),
            Overlay::Clip(clip) => self.centred(self.clip_sheet(*clip)),
            Overlay::FileMenu => self.floating(self.under_the_bar(FILE_MENU_LEFT), self.file_menu()),
            Overlay::HelpMenu => self.floating(self.under_the_bar(self.help_menu_left()), self.menu(vec![
                self.item("About Loupe", "", Some(Message::OpenAbout)),
            ])),
            Overlay::ViewMenu => self.floating(self.under_the_bar(self.view_menu_left()), self.view_menu()),
            Overlay::Performance => self.centred(self.performance_sheet()),
            Overlay::ScriptsMenu => self.floating(self.under_the_bar(SCRIPTS_MENU_LEFT), self.scripts_menu()),
            Overlay::About => self.centred(self.about_sheet()),
            Overlay::TrackMenu { track, at } => self.floating(*at, self.track_menu(*track)),
            Overlay::MixerMenu { at } => self.floating(*at, self.mixer_menu()),
            Overlay::Chain(spot) => self.centred(self.chain_sheet(*spot)),
            Overlay::Exported(folder, note) => self.centred(self.exported_sheet(folder, note.as_deref())),
            Overlay::Rename { at, .. } => self.floating(*at, self.rename_sheet()),
            Overlay::Colour { track, at } => self.floating(*at, self.colour_sheet(*track)),
            Overlay::Inputs { track, at, inputs } => self.floating(*at, self.input_menu(*track, *inputs)),
            Overlay::Routing(track) => self.centred(self.routing_sheet(*track)),
            Overlay::Sampler(track) => self.centred(self.sampler_sheet(*track)),
            Overlay::Plugins(track) => self.centred(self.plugin_sheet(*track)),
            Overlay::MasterPlugins => self.centred(self.picker("Plugins for Master".to_string(), &Message::AddMasterPlugin)),
            Overlay::ClipPlugins(clip) => self.centred(self.clip_plugin_sheet(*clip)),
            Overlay::Knobs(spot, slot) => self.centred(self.knob_sheet(*spot, *slot)),
            Overlay::Automation(target) => self.centred(self.automation_sheet(*target)),
            Overlay::Stock => self.centred(self.stock_sheet()),
            Overlay::Matrix => self.centred(self.matrix_sheet()),
            Overlay::Recover => self.recover_layer(),
            Overlay::CrashReport => self.crash_report_layer(),
            Overlay::PluginFell => self.fallen_layer(),
            Overlay::Roll(clip) => self.centred(self.roll_sheet(*clip)),
        }
    }

    fn centred<'a>(&self, sheet: Element<'a, Message>) -> Element<'a, Message> {
        let palette = self.palette;
        opaque(
            mouse_area(center(opaque(sheet)).padding(16).style(move |_| palette.backdrop()))
                .on_press(Message::CloseOverlay),
        )
    }

    fn under_the_bar(&self, left: f32) -> Point {
        Point::new(left, self.palette.top_bar_height - MENU_OVERLAPS_BAR)
    }

    fn view_menu_left(&self) -> f32 {
        if self.screen == Screen::Home {
            HOME_VIEW_MENU_LEFT
        } else {
            VIEW_MENU_LEFT
        }
    }

    fn view_menu(&self) -> Element<'_, Message> {
        let mut items = Vec::new();
        if self.screen != Screen::Home {
            let mixer = if self.mixer_open { "Mixer ✓" } else { "Mixer" };
            items.push(self.item(mixer, "F6", Some(Message::ToggleMixer)));
            if !self.project.tracks.is_empty() {
                items.push(self.item("Routing matrix", "F7", Some(Message::OpenMatrix)));
            }
            let audio = if self.pool_open { "All audio ✓" } else { "All audio" };
            items.push(self.item(audio, "", Some(Message::TogglePool)));
            items.push(rule(self.palette));
        }
        items.push(self.item("Performance", "Ctrl+Alt+P", Some(Message::OpenPerformance)));
        self.menu(items)
    }

    fn help_menu_left(&self) -> f32 {
        if self.screen == Screen::Home {
            HOME_HELP_MENU_LEFT
        } else {
            HELP_MENU_LEFT
        }
    }

    fn floating<'a>(&self, at: Point, menu: Element<'a, Message>) -> Element<'a, Message> {
        let left = at.x.min(self.window.width - MENU_WIDTH - EDGE_GAP).max(EDGE_GAP);
        let top = at.y.min(self.window.height - TALLEST_MENU - EDGE_GAP).max(EDGE_GAP);
        let placed = column![Space::with_height(top), row![Space::with_width(left), opaque(menu)]];
        opaque(
            mouse_area(container(placed).width(Length::Fill).height(Length::Fill))
                .on_press(Message::CloseOverlay)
                .on_right_press(Message::CloseOverlay),
        )
    }

    pub(crate) fn window<'a>(&self, title: String, body: Element<'a, Message>, widest: f32) -> Element<'a, Message> {
        let palette = self.palette;
        let close = button(container(crate::icon("x", 13.0)).center(22))
            .padding(0)
            .style(move |_, status| palette.ghost(status))
            .on_press(Message::CloseOverlay);
        let title_bar = container(
            row![text(title).size(13).font(palette.medium), horizontal_space(), close].align_y(Alignment::Center),
        )
        .padding(iced::Padding { top: 4.0, right: 6.0, bottom: 4.0, left: 14.0 })
        .width(Length::Fill)
        .style(move |_| palette.title_bar());
        container(column![title_bar, container(body).padding(18)])
            .padding(1)
            .width(Length::Fill)
            .max_width(widest)
            .style(move |_| palette.sheet())
            .into()
    }

    pub(crate) fn menu<'a>(&self, items: Vec<Element<'a, Message>>) -> Element<'a, Message> {
        self.menu_sized(items, MENU_WIDTH)
    }

    pub(crate) fn menu_sized<'a>(&self, items: Vec<Element<'a, Message>>, width: f32) -> Element<'a, Message> {
        let palette = self.palette;
        container(column(items).spacing(2)).padding(6).width(width).style(move |_| palette.menu()).into()
    }

    pub(crate) fn item<'a>(&self, label: impl text::IntoFragment<'a>, keys: impl text::IntoFragment<'a>, message: Option<Message>) -> Element<'a, Message> {
        let palette = self.palette;
        button(
            row![
                text(label).size(13),
                horizontal_space(),
                text(keys).size(11.5).font(palette.mono).color(palette.text_faint),
            ]
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .padding([7, 10])
        .style(move |_, status| palette.menu_item(status))
        .on_press_maybe(message)
        .into()
    }

    fn file_menu(&self) -> Element<'_, Message> {
        if self.screen == Screen::Home {
            return self.menu(vec![
                self.item("New project", "", Some(Message::NewBlank)),
                self.item("Open project…", "Ctrl+O", Some(Message::OpenProject)),
            ]);
        }
        let has_song = !self.project.tracks.is_empty();
        let mut items = vec![self.item("Open project…", "Ctrl+O", Some(Message::OpenProject))];
        items.push(self.item("Open with plugins off…", "Hold Shift", Some(Message::OpenWithPluginsOff)));
        if self.plugins_are_off() {
            items.push(self.item("Turn plugins back on", "", Some(Message::PluginsBackOn)));
        }
        items.push(rule(self.palette));
        items.push(self.item("Save", "Ctrl+S", Some(Message::Save)));
        items.push(self.item("Save as…", "Ctrl+Shift+S", Some(Message::SaveAs)));
        items.push(self.item("Save as template…", "", Some(Message::SaveAsTemplate)));
        if has_song {
            items.push(self.item("Export…", "Ctrl+E", Some(Message::OpenExport)));
        }
        items.push(rule(self.palette));
        items.push(self.item("Close project", "Ctrl+W", Some(Message::GoHome)));
        self.menu(items)
    }
    fn mixer_menu(&self) -> Element<'_, Message> {
        let alone = self.mixer_window.is_some();
        let move_it = if alone { Message::MixerBackUnderTheSong } else { Message::MixerToItsOwnWindow };
        let items = vec![
            self.item(if alone { "Mixer in its own window ✓" } else { "Mixer in its own window" }, "", Some(move_it)),
            self.item("Hide the mixer", "F6", Some(Message::ToggleMixer)),
        ];
        self.menu(items)
    }


    fn track_menu(&self, track: TrackId) -> Element<'_, Message> {
        let inside = self.project.tracks.iter().find(|t| t.id == track).and_then(|t| t.parent).is_some();
        let above = self
            .project
            .tracks
            .iter()
            .position(|t| t.id == track)
            .filter(|i| *i > 0)
            .map(|i| self.project.tracks[i - 1].id);
        let mut items = vec![
            self.item("Rename", "", Some(Message::StartRename(track))),
            self.item("Change colour", "", Some(Message::StartColour(track))),
            self.item("Duplicate", "", Some(Message::DuplicateTrack(track))),
        ];
        items.push(self.item(
            "Put inside the track above",
            "",
            above.map(|parent| Message::SetTrackParent { track, parent: Some(parent) }),
        ));
        if inside {
            items.push(self.item("Take out of its folder", "", Some(Message::SetTrackParent { track, parent: None })));
        }
        items.push(self.item("Routing…", "", Some(Message::OpenRouting(track))));
        items.push(rule(self.palette));
        items.push(self.item("New note clip", "", Some(Message::NewNotesClip(track))));
        let keys = self.project.track(track).is_some_and(|t| t.records_notes);
        items.push(self.item(if keys { "Record notes from keys ✓" } else { "Record notes from keys" }, "", Some(Message::ToggleRecordsNotes(track))));
        let printing = self.project.track(track).is_some_and(|t| t.print_takes);
        items.push(self.item(if printing { "Keep Rec plugins in takes ✓" } else { "Keep Rec plugins in takes" }, "", Some(Message::TogglePrintTakes(track))));
        if let Some(input) = self.project.track(track).filter(|t| !t.records_notes).map(|t| t.input.name()) {
            items.push(self.item("Record from", input, Some(Message::StartInputs(track))));
        }
        let playing = self.project.track(track).map(|t| t.instrument);
        let synth = matches!(playing, Some(loupe_engine::Instrument::Synth(_)));
        let drums = matches!(playing, Some(loupe_engine::Instrument::Drums));
        let sampler = matches!(playing, Some(loupe_engine::Instrument::Sampler(_)));
        items.push(container(text("Plays notes with").size(11.5).color(self.palette.text_dim)).padding([6, 10]).into());
        items.push(self.item(if synth { "Loupe Synth ✓" } else { "Loupe Synth" }, "", Some(Message::UseInstrument(track, loupe_engine::Instrument::default()))));
        items.push(self.item(if drums { "Loupe Drums ✓" } else { "Loupe Drums" }, "", Some(Message::UseInstrument(track, loupe_engine::Instrument::Drums))));
        items.push(self.item(if sampler { "Loupe Sampler ✓" } else { "Loupe Sampler" }, if sampler { "Settings" } else { "" }, Some(Message::OpenSampler(track))));
        self.menu(items)
    }

    fn input_menu(&self, track: TrackId, inputs: u16) -> Element<'_, Message> {
        let palette = self.palette;
        let current = self.project.track(track).map(|t| t.input).unwrap_or_default();
        let choice = |channels: InputChannels| {
            let name = channels.name();
            self.item(if channels == current { format!("{name} ✓") } else { name }, "", Some(Message::UseInput(track, channels)))
        };
        let (mono, pairs): (Vec<InputChannels>, Vec<InputChannels>) = InputChannels::every(inputs).into_iter().partition(|channels| channels.width() == 1);
        let mut items = vec![container(text("Record from").size(12).color(palette.text_dim)).padding([4, 4]).into()];
        if inputs == 0 {
            items.push(container(text("No recording input found. Check Settings > Recording.").size(12.5)).padding([4, 4]).into());
            return self.menu(items);
        }
        let mono = column(mono.into_iter().map(choice)).spacing(2).width(Length::Fill);
        let pairs = column(pairs.into_iter().map(choice)).spacing(2).width(Length::Fill);
        items.push(container(scrollable(row![mono, pairs].spacing(4))).max_height(INPUT_MENU_TALLEST).into());
        self.menu_sized(items, INPUT_MENU_WIDTH)
    }

    fn entry_field<'a>(&'a self, placeholder: &'a str) -> Element<'a, Message> {
        let palette = self.palette;
        text_input(placeholder, &self.entry)
            .id(ENTRY_ID)
            .on_input(Message::EntryTyped)
            .on_submit(Message::EntryEntered)
            .size(13)
            .padding([6, 8])
            .style(move |_, status| palette.field(status))
            .into()
    }

    fn rename_sheet(&self) -> Element<'_, Message> {
        let palette = self.palette;
        self.menu(vec![
            container(text("Rename track").size(12).color(palette.text_dim)).padding([4, 4]).into(),
            self.entry_field("Track name"),
        ])
    }

    fn colour_sheet(&self, track: TrackId) -> Element<'_, Message> {
        let palette = self.palette;
        let swatches = palette.tracks.iter().map(|colour| {
            let colour = *colour;
            let [r, g, b, _] = colour.into_rgba8();
            button(Space::new(18, 18))
                .padding(0)
                .style(move |_, status| palette.swatch(colour, status))
                .on_press(Message::ColourPicked(track, Some([r, g, b])))
                .into()
        });
        let follow_theme = button(text("Use the theme's colour").size(12.5))
            .width(Length::Fill)
            .padding([6, 10])
            .style(move |_, status| palette.menu_item(status))
            .on_press(Message::ColourPicked(track, None));
        self.menu(vec![
            container(text("Track colour").size(12).color(palette.text_dim)).padding([4, 4]).into(),
            container(row(swatches).spacing(7)).padding([2, 4]).into(),
            self.entry_field("#rrggbb"),
            follow_theme.into(),
        ])
    }

    fn about_sheet(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let credit = |line: &'static str| text(line).size(13).color(palette.text_dim);
        let body = column![
            iced::widget::image(text_logo()).height(44),
            text(concat!("Version ", env!("CARGO_PKG_VERSION"))).size(12).font(palette.mono).color(palette.text_dim),
            rule(palette),
            credit("Made by ash."),
            credit("Built with Rust, Iced, cpal and Symphonia."),
            credit("Typefaces: Inter and JetBrains Mono. Icons: Lucide."),
            rule(palette),
            self.updates_block(),
        ]
        .spacing(10);
        self.window("About".to_string(), body.into(), 460.0)
    }

    fn save_sheet(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let lands_at = match self.named_project_file() {
            Some(file) => file.display().to_string(),
            None => "Type a name for the project.".to_string(),
        };
        let note: Element<'_, Message> = match &self.entry_problem {
            Some(problem) => text(problem.as_str()).size(12).color(palette.danger).into(),
            None => text(lands_at).size(11.5).font(palette.mono).color(palette.text_dim).into(),
        };
        let choices = row![
            button(text("Choose another place…").size(13).font(palette.medium))
                .padding([7, 14])
                .style(move |_, status| palette.ghost(status))
                .on_press(Message::SaveElsewhere),
            horizontal_space(),
            button(text("Cancel").size(13).font(palette.medium))
                .padding([7, 14])
                .style(move |_, status| palette.outlined(status))
                .on_press(Message::CloseOverlay),
            button(text("Save").size(13).font(palette.medium))
                .padding([7, 18])
                .style(move |_, status| palette.solid(status))
                .on_press_maybe(self.named_project_file().map(|_| Message::EntryEntered)),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        container(
            column![
                text("Save project").size(16).font(palette.semibold),
                self.entry_field("Project name"),
                note,
                choices,
            ]
            .spacing(12),
        )
        .padding(20)
        .width(Length::Fill)
        .max_width(520)
        .style(move |_| palette.sheet())
        .into()
    }

    fn template_sheet(&self) -> Element<'_, Message> {
        let palette = self.palette;
        container(
            column![
                text("Save as template").size(16).font(palette.semibold),
                text("The song as it is now becomes a starting point on the start screen.")
                    .size(13)
                    .color(palette.text_dim),
                self.entry_field("Template name"),
            ]
            .spacing(12),
        )
        .padding(20)
        .width(Length::Fill)
        .max_width(420)
        .style(move |_| palette.sheet())
        .into()
    }

    fn discard_sheet(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let choices = row![
            horizontal_space(),
            button(text("Cancel").size(13).font(palette.medium))
                .padding([7, 14])
                .style(move |_, status| palette.outlined(status))
                .on_press(Message::CloseOverlay),
            button(text("Discard").size(13).font(palette.medium).color(Color::WHITE))
                .padding([7, 14])
                .style(move |_, status| palette.destructive(status))
                .on_press(Message::Discard),
        ]
        .spacing(10);
        container(
            column![
                text("Discard changes?").size(16).font(palette.semibold),
                text("This song has changes that are not saved. Carrying on discards them.")
                    .size(13)
                    .color(palette.text_dim),
                choices,
            ]
            .spacing(14),
        )
        .padding(20)
        .width(Length::Fill)
        .max_width(460)
        .style(move |_| palette.sheet())
        .into()
    }
}

pub fn text_logo() -> iced::widget::image::Handle {
    static LOGO: std::sync::OnceLock<iced::widget::image::Handle> = std::sync::OnceLock::new();
    LOGO.get_or_init(|| iced::widget::image::Handle::from_bytes(include_bytes!("../assets/text-logo.png").as_slice()))
        .clone()
}

pub fn colour_from_hex(typed: &str) -> Option<[u8; 3]> {
    let digits = typed.trim().trim_start_matches('#');
    if digits.len() != 6 {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

impl App {
    pub(crate) fn automation_sheet(&self, target: loupe_engine::Target) -> Element<'_, Message> {
        let palette = self.palette;
        let shape = self.project.envelope(target);
        let armed = shape.and_then(|shape| shape.armed);
        let touch = armed == Some(loupe_engine::Mode::Touch);
        let latch = armed == Some(loupe_engine::Mode::Latch);
        let line = |words: String, message: Message| -> Element<'_, Message> {
            button(text(words).size(13))
                .width(Length::Fill)
                .padding([7, 10])
                .style(move |_, status| palette.menu_item(status))
                .on_press(message)
                .into()
        };
        let mut body = column![line(
            if shape.is_some() { "Remove the envelope".to_string() } else { "Automate".to_string() },
            if shape.is_some() { Message::RemoveEnvelope(target) } else { Message::AddEnvelope(target) },
        )]
        .spacing(2);
        if shape.is_some() {
            body = body
                .push(line(
                    if touch { "Stop writing on touch".to_string() } else { "Write on touch".to_string() },
                    Message::ArmEnvelope(target, (!touch).then_some(loupe_engine::Mode::Touch)),
                ))
                .push(line(
                    if latch { "Stop writing on latch".to_string() } else { "Write on latch".to_string() },
                    Message::ArmEnvelope(target, (!latch).then_some(loupe_engine::Mode::Latch)),
                ));
        }
        self.window(self.target_name(target), body.into(), 300.0)
    }

    fn target_name(&self, target: loupe_engine::Target) -> String {
        let named = |track: loupe_engine::TrackId| {
            self.project.track(track).map(|t| t.name.clone()).unwrap_or_else(|| "Track".to_string())
        };
        match target {
            loupe_engine::Target::TrackGain(track) => format!("{} volume", named(track)),
            loupe_engine::Target::TrackPan(track) => format!("{} pan", named(track)),
            loupe_engine::Target::MasterGain => "Master volume".to_string(),
            _ => "Automation".to_string(),
        }
    }
}
