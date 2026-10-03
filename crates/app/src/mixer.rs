use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{button, column, container, scrollable, text, vertical_slider, Space};
use iced::{Alignment, Color, Element, Length};

use crate::{App, Message};

const MIXER_HEIGHT: f32 = 236.0;
const STRIP_WIDTH: f32 = 78.0;
const STRIP_HEIGHT: f32 = 204.0;
const NAME_LENGTH: usize = 9;
pub const SILENT_DB: f32 = -60.0;
pub const LOUDEST_DB: f32 = 6.0;

pub fn gain_from_db(db: f32) -> f32 {
    if db <= SILENT_DB {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

fn db_from_gain(gain: f32) -> f32 {
    if gain <= 0.0 {
        SILENT_DB
    } else {
        (20.0 * gain.log10()).clamp(SILENT_DB, LOUDEST_DB)
    }
}

impl App {
    pub(crate) fn mixer(&self) -> Element<'_, Message> {
        let palette = self.palette;
        if self.project.tracks.is_empty() {
            return container(text("Each track gets a fader here.").size(12.5).color(palette.text_dim))
                .center_x(Length::Fill)
                .center_y(MIXER_HEIGHT)
                .style(move |_| palette.bar())
                .into();
        }
        let strips = self.project.tracks.iter().map(|track| {
            let id = track.id;
            let db = db_from_gain(track.gain);
            let level = if track.gain <= 0.0 { "-inf".to_string() } else { format!("{db:+.1}") };
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
                    text(level).size(11.5).font(palette.mono).color(palette.text_dim),
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
            .height(STRIP_HEIGHT)
            .style(move |_| palette.strip())
            .into()
        });
        let row = iced::widget::row(strips).spacing(8);
        container(scrollable(row).direction(Direction::Horizontal(Scrollbar::new())))
            .padding(10)
            .width(Length::Fill)
            .height(MIXER_HEIGHT)
            .style(move |_| palette.bar())
            .into()
    }
}
