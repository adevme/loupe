use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{button, column, container, row, scrollable, text, text_input, Space};
use iced::{Alignment, Element, Length};

use loupe_engine::TrackId;
use loupe_plugins::Found;

use crate::{App, Message};

pub const FILTER_ID: &str = "plugin-filter";
const NAME_LENGTH: usize = 9;

pub fn find_plugins() -> Vec<Found> {
    let mut found = loupe_plugins::scan(&loupe_plugins::folders());
    found.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    found
}

impl App {
    pub(crate) fn fx_block(&self, track: TrackId) -> Element<'_, Message> {
        let palette = self.palette;
        let Some(found) = self.project.tracks.iter().find(|t| t.id == track) else {
            return Space::new(Length::Fill, 0).into();
        };
        let mut rows = column![].spacing(2);
        for (slot, fx) in found.fx.iter().enumerate() {
            let short = shorten(&fx.name);
            let on = !fx.bypassed;
            rows = rows.push(
                row![
                    button(text(short).size(10.5))
                        .padding([1, 3])
                        .width(Length::Fill)
                        .style(move |_, status| palette.toggled(on, status))
                        .on_press(Message::ShowPlugin(track, slot)),
                    button(text("b").size(10.5))
                        .padding([1, 3])
                        .style(move |_, status| palette.toggled(!on, status))
                        .on_press(Message::BypassPlugin(track, slot)),
                    button(text("x").size(10.5))
                        .padding([1, 4])
                        .style(move |_, status| palette.ghost(status))
                        .on_press(Message::RemovePlugin(track, slot)),
                ]
                .spacing(2),
            );
        }
        let add = button(text("+ FX").size(10.5))
            .padding([1, 4])
            .width(Length::Fill)
            .style(move |_, status| palette.ghost(status))
            .on_press(Message::OpenPlugins(track));
        column![scrollable(rows).height(Length::Fixed(44.0)).direction(Direction::Vertical(Scrollbar::new().width(4).scroller_width(4))), add]
            .spacing(3)
            .width(Length::Fill)
            .into()
    }

    pub(crate) fn plugin_sheet(&self, track: TrackId) -> Element<'_, Message> {
        let palette = self.palette;
        let Some(found) = self.project.tracks.iter().find(|t| t.id == track) else {
            return text("This track is gone.").size(13).into();
        };
        let heading = text(format!("Plugins for {}", found.name)).size(14).font(palette.semibold);
        let search = text_input("Search", &self.plugin_filter)
            .id(FILTER_ID)
            .on_input(Message::PluginFilter)
            .size(13)
            .padding([5, 10]);
        let needle = self.plugin_filter.trim().to_lowercase();
        let mut list = column![].spacing(4);
        let mut shown = 0;
        for (index, plugin) in self.found.iter().enumerate() {
            let hay = format!("{} {}", plugin.name, plugin.vendor.clone().unwrap_or_default()).to_lowercase();
            if !needle.is_empty() && !hay.contains(&needle) {
                continue;
            }
            shown += 1;
            let label = match &plugin.vendor {
                Some(vendor) => format!("{}  ·  {vendor}  ·  {}", plugin.name, plugin.format.label()),
                None => format!("{}  ·  {}", plugin.name, plugin.format.label()),
            };
            list = list.push(
                button(text(label).size(12.5))
                    .padding([5, 10])
                    .width(Length::Fill)
                    .style(move |_, status| palette.ghost(status))
                    .on_press(Message::AddPlugin(track, index)),
            );
        }
        let body: Element<'_, Message> = if self.scanning {
            text("Looking for plugins…").size(12.5).color(palette.text_dim).into()
        } else if self.found.is_empty() {
            text("No plugins found in the usual folders.").size(12.5).color(palette.text_dim).into()
        } else if shown == 0 {
            text("Nothing matches that.").size(12.5).color(palette.text_dim).into()
        } else {
            scrollable(list).height(Length::Fixed(280.0)).into()
        };
        container(column![heading, search, body].spacing(10).align_x(Alignment::Start))
            .width(Length::Fixed(440.0))
            .into()
    }
}

fn shorten(name: &str) -> String {
    if name.chars().count() <= NAME_LENGTH {
        return name.to_string();
    }
    let words: Vec<&str> = name.split_whitespace().collect();
    for from in 1..words.len() {
        let rest = words[from..].join(" ");
        if rest.chars().count() <= NAME_LENGTH {
            return rest;
        }
    }
    let tail = words.last().copied().unwrap_or(name);
    tail.chars().take(NAME_LENGTH).collect()
}

#[cfg(test)]
mod tests {
    use super::shorten;

    #[test]
    fn a_long_name_loses_the_maker_not_the_model() {
        assert_eq!(shorten("FabFilter Pro-Q 4"), "Pro-Q 4");
        assert_eq!(shorten("FabFilter Saturn 2"), "Saturn 2");
        assert_eq!(shorten("Loupe EQ"), "Loupe EQ");
        assert_eq!(shorten("Valhalla Supermassive"), "Supermass");
        assert_eq!(shorten("Pro-Q 4"), "Pro-Q 4");
    }
}
