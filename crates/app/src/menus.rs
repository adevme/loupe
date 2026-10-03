use iced::widget::{
    button, center, column, container, horizontal_space, mouse_area, opaque, row, text, text_input, Space,
};
use iced::{Alignment, Element, Length, Point};
use loupe_engine::TrackId;

use crate::{App, Message, Overlay};

const MENU_WIDTH: f32 = 220.0;
const TALLEST_MENU: f32 = 190.0;
const EDGE_GAP: f32 = 8.0;
pub const ENTRY_ID: &str = "overlay-entry";

impl App {
    pub(crate) fn overlay(&self) -> Element<'_, Message> {
        match &self.overlay {
            Overlay::None => Space::new(0, 0).into(),
            Overlay::Settings => self.centred(self.settings_sheet()),
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

}

pub fn colour_from_hex(typed: &str) -> Option<[u8; 3]> {
    let digits = typed.trim().trim_start_matches('#');
    if digits.len() != 6 {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}
