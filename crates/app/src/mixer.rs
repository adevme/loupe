use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{button, column, container, mouse_area, scrollable, text, text_input, vertical_slider, Space};
use iced::{mouse, Alignment, Color, Element, Length};

use loupe_engine::{ClipId, TrackId};

use crate::{App, Message};

pub const MIXER_HEIGHT: f32 = 236.0;
pub const SHORTEST_MIXER: f32 = 150.0;
const GRAB_BAR: f32 = 6.0;
const STRIP_MARGIN: f32 = 32.0;
const STRIP_WIDTH: f32 = 78.0;
const NAME_LENGTH: usize = 9;
pub const SILENT_DB: f32 = -60.0;
pub const LOUDEST_DB: f32 = 6.0;
pub const LEVEL_ENTRY_ID: &str = "level-entry";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Level {
    Track(TrackId),
    Clip(ClipId),
    Master,
}

pub fn db_from_typed(typed: &str) -> Option<f32> {
    let typed = typed.trim().to_lowercase();
    let number = typed.trim_end_matches("db").trim();
    if number == "-inf" {
        return Some(SILENT_DB);
    }
    number.parse::<f32>().ok().filter(|db| db.is_finite())
}

pub fn gain_from_db(db: f32) -> f32 {
    if db <= SILENT_DB {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

pub fn db_from_gain(gain: f32) -> f32 {
    if gain <= 0.0 {
        SILENT_DB
    } else {
        (20.0 * gain.log10()).clamp(SILENT_DB, LOUDEST_DB)
    }
}

pub fn level_text(gain: f32) -> String {
    if gain <= 0.0 {
        "-inf".to_string()
    } else {
        format!("{:+.1}", (20.0 * gain.log10()).max(SILENT_DB))
    }
}

impl App {
    pub(crate) fn level_readout(&self, level: Level, gain: f32) -> Element<'_, Message> {
        let palette = self.palette;
        if self.editing_level == Some(level) {
            return text_input("", &self.entry)
                .id(LEVEL_ENTRY_ID)
                .on_input(Message::EntryTyped)
                .on_submit(Message::LevelEntered)
                .font(palette.mono)
                .size(11.5)
                .padding([2, 4])
                .width(52)
                .style(move |_, status| palette.field(status))
                .into();
        }
        mouse_area(text(level_text(gain)).size(11.5).font(palette.mono).color(palette.text_dim))
            .on_press(Message::LevelPressed(level))
            .into()
    }

    pub(crate) fn mixer(&self) -> Element<'_, Message> {
        let palette = self.palette;
        if self.project.tracks.is_empty() {
            return container(text("Each track gets a fader here.").size(12.5).color(palette.text_dim))
                .center_x(Length::Fill)
                .center_y(self.mixer_height)
                .style(move |_| palette.bar())
                .into();
        }
        let strips = self.project.tracks.iter().map(|track| {
            let id = track.id;
            let db = db_from_gain(track.gain);
            let colour = match track.colour {
                Some([r, g, b]) => Color::from_rgb8(r, g, b),
                None => palette.track(track.id.0.saturating_sub(1) as usize),
            };
            let name: String = track.name.chars().take(NAME_LENGTH).collect();
            let muted = track.muted;
            container(
                column![
                    container(Space::new(Length::Fill, 3)).style(move |_| container::Style {
                        background: Some(colour.into()),
                        ..Default::default()
                    }),
                    text(name).size(12).font(palette.medium),
                    vertical_slider(SILENT_DB..=LOUDEST_DB, db, move |db| Message::TrackGain(id, db))
                        .step(0.1)
                        .default(0.0)
                        .on_release(Message::DragEnd)
                        .height(Length::Fill)
                        .style(move |_, status| palette.slider(status)),
                    self.level_readout(Level::Track(id), track.gain),
                    button(text("M").size(11.5).font(palette.semibold))
                        .padding([3, 9])
                        .style(move |_, status| palette.mute(muted, status))
                        .on_press(Message::ToggleMute(id)),
                ]
                .spacing(8)
                .align_x(Alignment::Center),
            )
            .padding(8)
            .width(STRIP_WIDTH)
            .height(self.mixer_height - STRIP_MARGIN - GRAB_BAR)
            .style(move |_| palette.strip())
            .into()
        });
        let row = iced::widget::row(strips).spacing(8);
        let grab_bar = mouse_area(container(Space::new(Length::Fill, GRAB_BAR)).style(move |_| palette.bar()))
            .on_press(Message::MixerGrabbed)
            .interaction(mouse::Interaction::ResizingVertically);
        let strips = container(scrollable(row).direction(Direction::Horizontal(Scrollbar::new())))
            .padding([4, 10])
            .width(Length::Fill)
            .height(self.mixer_height - GRAB_BAR)
            .style(move |_| palette.bar());
        column![grab_bar, strips].into()
    }
}
