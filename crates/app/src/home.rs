use std::path::Path;

use iced::widget::{button, column, container, horizontal_space, image, row, scrollable, text};
use iced::{Alignment, Element, Length};

use crate::{icon_button, rule, App, Message};

const START_WIDTH: f32 = 290.0;
const PAGE_WIDTH: f32 = 860.0;
const LOGO_HEIGHT: f32 = 56.0;
const PAGE_TOP: f32 = 26.0;
const PAGE_SIDE: f32 = 32.0;

impl App {
    pub(crate) fn home(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let heading = |label: &'static str| text(label).size(11).font(palette.semibold).color(palette.text_dim);
        let choice = |title: String, note: String, message: Message| -> Element<'_, Message> {
            button(
                column![
                    text(title).size(13.5).font(palette.medium),
                    text(note).size(11.5).color(palette.text_dim),
                ]
                .spacing(3),
            )
            .width(Length::Fill)
            .padding([9, 12])
            .style(move |_, status| palette.menu_item(status))
            .on_press(message)
            .into()
        };

        let mut start = column![heading("START")].spacing(4);
        start = start.push(choice("Blank project".into(), "An empty song".into(), Message::NewBlank));
        for template in &self.templates {
            let note = "Template".to_string();
            start = start.push(choice(stem(template), note, Message::NewFromTemplate(template.clone())));
        }
        start = start.push(choice("Open project…".into(), "Pick a .lp file".into(), Message::OpenProject));

        let recent: Element<'_, Message> = if self.recent.is_empty() {
            text("Projects you open or save show up here.").size(12.5).color(palette.text_dim).into()
        } else {
            let entries = self.recent.iter().map(|project| {
                let folder = project.parent().map(|parent| parent.display().to_string()).unwrap_or_default();
                choice(stem(project), folder, Message::OpenRecent(project.clone()))
            });
            scrollable(column(entries).spacing(4)).height(Length::Fill).into()
        };

        let title = row![
            image(crate::menus::text_logo()).height(LOGO_HEIGHT),
            horizontal_space(),
            icon_button(palette, "settings", Some(Message::OpenSettings)),
        ]
        .align_y(Alignment::End);

        let lists = row![
            start.width(START_WIDTH),
            column![heading("RECENT"), recent].spacing(4).width(Length::Fill),
        ]
        .spacing(40);

        let mut page = column![title, rule(palette), lists].spacing(20);
        if let Some(status) = self.status() {
            page = page.push(status);
        }
        let bar = container(row![self.file_button(), self.view_button(), self.help_button()].spacing(8).align_y(Alignment::Center))
            .padding([0, 16])
            .height(palette.top_bar_height)
            .align_y(Alignment::Center)
            .width(Length::Fill)
            .style(move |_| palette.bar());
        let page = container(container(page).width(Length::Fill).max_width(PAGE_WIDTH))
            .style(move |_| container::Style { background: Some(palette.background.into()), ..Default::default() })
            .center_x(Length::Fill)
            .padding(iced::Padding { top: PAGE_TOP, right: PAGE_SIDE, bottom: PAGE_SIDE, left: PAGE_SIDE })
            .height(Length::Fill);
        column![bar, rule(palette), page].into()
    }
}

pub fn stem(path: &Path) -> String {
    path.file_stem().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
}
