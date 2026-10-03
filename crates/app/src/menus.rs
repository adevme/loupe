use iced::widget::{
    button, center, column, container, horizontal_space, mouse_area, opaque, row, text, text_input, Space,
};
use iced::{Alignment, Color, Element, Length, Point};
use loupe_engine::TrackId;

use crate::{rule, App, Message, Overlay};

const MENU_WIDTH: f32 = 220.0;
const TALLEST_MENU: f32 = 250.0;
const EDGE_GAP: f32 = 8.0;
const FILE_MENU_AT: Point = Point::new(76.0, 44.0);
const HELP_MENU_AT: Point = Point::new(134.0, 44.0);
pub const ENTRY_ID: &str = "overlay-entry";

impl App {
    pub(crate) fn overlay(&self) -> Element<'_, Message> {
        match &self.overlay {
            Overlay::None => Space::new(0, 0).into(),
            Overlay::Settings => self.centred(self.settings_sheet()),
            Overlay::ConfirmDiscard(_) => self.centred(self.discard_sheet()),
            Overlay::TemplateName => self.centred(self.template_sheet()),
            Overlay::SaveName => self.centred(self.save_sheet()),
            Overlay::Clip(clip) => self.centred(self.clip_sheet(*clip)),
            Overlay::FileMenu => self.floating(FILE_MENU_AT, self.file_menu()),
            Overlay::HelpMenu => self.floating(HELP_MENU_AT, self.menu(vec![self.item("About", "", Some(Message::OpenAbout))])),
            Overlay::About => self.centred(self.about_sheet()),
            Overlay::TrackMenu { track, at } => self.floating(*at, self.track_menu(*track)),
            Overlay::Rename { at, .. } => self.floating(*at, self.rename_sheet()),
            Overlay::Colour { track, at } => self.floating(*at, self.colour_sheet(*track)),
        }
    }

    fn centred<'a>(&self, sheet: Element<'a, Message>) -> Element<'a, Message> {
        let palette = self.palette;
        opaque(
            mouse_area(center(opaque(sheet)).padding(16).style(move |_| palette.backdrop()))
                .on_press(Message::CloseOverlay),
        )
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

    fn menu<'a>(&self, items: Vec<Element<'a, Message>>) -> Element<'a, Message> {
        let palette = self.palette;
        container(column(items).spacing(2)).padding(6).width(MENU_WIDTH).style(move |_| palette.menu()).into()
    }

    fn item<'a>(&self, label: &'a str, keys: &'a str, message: Option<Message>) -> Element<'a, Message> {
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
        let has_song = !self.project.tracks.is_empty();
        let mut items = vec![self.item("Open project…", "Ctrl+O", Some(Message::OpenProject))];
        if has_song {
            items.push(self.item("Save", "Ctrl+S", Some(Message::Save)));
            items.push(self.item("Save as…", "Ctrl+Shift+S", Some(Message::SaveAs)));
            items.push(self.item("Save as template…", "", Some(Message::SaveAsTemplate)));
        }
        items.push(rule(self.palette));
        let audio = if self.pool_open { "Hide all audio" } else { "Show all audio" };
        items.push(self.item(audio, "", Some(Message::TogglePool)));
        items.push(rule(self.palette));
        items.push(self.item("Close project", "Ctrl+W", Some(Message::GoHome)));
        self.menu(items)
    }

    fn track_menu(&self, track: TrackId) -> Element<'_, Message> {
        self.menu(vec![
            self.item("Rename", "", Some(Message::StartRename(track))),
            self.item("Change colour", "", Some(Message::StartColour(track))),
            self.item("Duplicate", "", Some(Message::DuplicateTrack(track))),
        ])
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
        container(
            column![
                row![
                    text("Loupe").size(22).font(palette.semibold),
                    horizontal_space(),
                    crate::icon_button(palette, "x", Some(Message::CloseOverlay)),
                ]
                .align_y(Alignment::Center),
                text(concat!("Version ", env!("CARGO_PKG_VERSION"))).size(12).font(palette.mono).color(palette.text_dim),
                rule(palette),
                credit("Made by ash."),
                credit("Built with Rust, Iced, cpal and Symphonia."),
                credit("Typefaces: Inter and JetBrains Mono. Icons: Lucide."),
            ]
            .spacing(10),
        )
        .padding(22)
        .width(Length::Fill)
        .max_width(420)
        .style(move |_| palette.sheet())
        .into()
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

pub fn colour_from_hex(typed: &str) -> Option<[u8; 3]> {
    let digits = typed.trim().trim_start_matches('#');
    if digits.len() != 6 {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}
