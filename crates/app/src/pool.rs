use std::sync::Arc;

use iced::widget::{button, column, container, horizontal_space, row, scrollable, text};
use iced::{Alignment, Element, Length};

use crate::{icon, icon_button, rule, App, Message};

pub const POOL_WIDTH: f32 = 270.0;

impl App {
    pub(crate) fn pool(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let heading = row![
            text("All audio").size(13).font(palette.semibold),
            horizontal_space(),
            icon_button(palette, "x", Some(Message::TogglePool)),
        ]
        .align_y(Alignment::Center);

        let entries = self.project.sources.iter().enumerate().map(|(index, source)| {
            let uses = self.project.clips().filter(|clip| Arc::ptr_eq(&clip.source, source)).count();
            let found = !source.frames.is_empty();
            let seconds = source.frames.len() as f64 / self.project.rate.max(1) as f64;
            let length = format!("{}:{:02}", (seconds / 60.0) as u64, seconds as u64 % 60);
            let note = match (found, uses) {
                (false, _) => "file not found".to_string(),
                (true, 0) => format!("{length} · not in the song"),
                (true, 1) => format!("{length} · used once"),
                (true, n) => format!("{length} · used {n} times"),
            };
            let note_colour = if found { palette.text_dim } else { palette.danger };
            row![
                column![
                    text(source.name.as_str()).size(13).font(palette.medium),
                    text(note).size(11.5).color(note_colour),
                ]
                .spacing(3)
                .width(Length::Fill),
                button(icon("plus", 14.0))
                    .padding([5, 8])
                    .style(move |_, status| palette.outlined(status))
                    .on_press_maybe(found.then_some(Message::PlaceSource(index))),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into()
        });

        let list: Element<'_, Message> = if self.project.sources.is_empty() {
            text("Audio you import shows up here and stays, even after its clips are deleted.")
                .size(12.5)
                .color(palette.text_dim)
                .into()
        } else {
            scrollable(column(entries).spacing(12)).height(Length::Fill).into()
        };

        container(column![heading, rule(palette), list].spacing(12))
            .padding(14)
            .width(POOL_WIDTH)
            .height(Length::Fill)
            .style(move |_| palette.bar())
            .into()
    }
}
