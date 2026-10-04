use iced::widget::{button, column, row, text};
use iced::{Alignment, Element, Length, Task};

use crate::theme::{self, Palette};
use crate::{settings, App, Message};

const BUILT_IN_THEME: &str = "Default";

impl App {
    pub(crate) fn use_theme(&mut self, picked: String) -> Task<Message> {
        let chosen = (picked != BUILT_IN_THEME).then_some(picked);
        let saved = match &chosen {
            Some(name) => settings::save("theme", name),
            None => settings::forget("theme"),
        };
        let loaded = Palette::load(chosen.as_deref());
        let fonts_wait = loaded.palette.fonts() != self.palette.fonts();
        self.palette = loaded.palette.with_fonts_of(&self.palette);
        self.theme_name = chosen;
        self.theme_problem = loaded.problem;
        self.problem = saved.err().map(|why| format!("Could not save settings: {why}"));
        self.notice = fonts_wait.then(|| "This theme's fonts show after restarting Loupe.".to_string());
        self.cache.clear();
        match loaded.icon_font {
            Some(font) => iced::font::load(font).map(|_| Message::ThemeFontLoaded),
            None => Task::none(),
        }
    }

    pub(crate) fn show_themes_folder(&mut self) {
        let Some(folder) = theme::folder() else {
            return;
        };
        if let Err(why) = crate::files::show_in_folder(&folder.join(theme::REFERENCE_FILE)) {
            self.problem = Some(format!("Could not open the folder: {why}"));
        }
    }

    pub(crate) fn theme_picker(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let mut names = vec![BUILT_IN_THEME.to_string()];
        names.extend(self.theme_names.iter().cloned());
        let current = self.theme_name.clone().unwrap_or_else(|| BUILT_IN_THEME.to_string());
        column![
            text("Theme").size(13).font(palette.medium),
            text("Colours, sizes, shapes, icons and layout. Each theme is a file in the themes folder.")
                .size(12)
                .color(palette.text_dim),
            row![
                theme::picker(palette, names, Some(current), Message::ThemeChosen).width(Length::Fill),
                button(text("Open folder").size(12.5).font(palette.medium))
                    .padding([6, 12])
                    .style(move |_, status| palette.outlined(status))
                    .on_press(Message::ShowThemes),
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        ]
        .spacing(8)
        .into()
    }
}
