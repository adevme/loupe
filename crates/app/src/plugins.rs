use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{button, column, container, mouse_area, row, scrollable, text, text_input, Space};
use iced::{Alignment, Element, Length};

use loupe_engine::{ClipId, TrackId};
use loupe_plugins::Found;

use crate::{App, Message};

pub const FILTER_ID: &str = "plugin-filter";
const NAME_LENGTH: usize = 9;

pub fn find_plugins() -> Vec<Found> {
    let mut found = loupe_plugins::everything();
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
            let dragged = self.fx_drag.map(|(held, from, _)| held == track && from == slot).unwrap_or(false);
            let landing = self.fx_drag.map(|(held, _, over)| held == track && over == slot).unwrap_or(false);
            let name = container(text(short).size(10.5).wrapping(iced::widget::text::Wrapping::None))
                .padding(iced::Padding { top: 1.0, right: 3.0, bottom: 1.0, left: 3.0 })
                .width(Length::Fill)
                .style(move |_| crate::plugins::slot_look(palette, on, dragged, landing));
            rows = rows.push(
                row![
                    mouse_area(name)
                        .on_press(Message::FxGrab(track, slot))
                        .on_move(move |_| Message::FxOver(slot))
                        .on_release(Message::FxDrop)
                        .interaction(iced::mouse::Interaction::Grab),
                    button(text("b").size(10.5))
                        .padding([1, 3])
                        .style(move |_, status| palette.toggled(!on, status))
                        .on_press(Message::BypassPlugin(track, slot)),
                    button(text("a").size(10.5))
                        .padding([1, 3])
                        .style(move |_, status| palette.ghost(status))
                        .on_press(Message::OpenKnobs(crate::stockwin::Spot::Track(track), slot)),
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
        let Some(found) = self.project.tracks.iter().find(|t| t.id == track) else {
            return text("This track is gone.").size(13).into();
        };
        let heading = format!("Plugins for {}", found.name);
        return self.picker(heading, &move |which| Message::AddPlugin(track, which));
    }

    pub(crate) fn picker(&self, heading: String, chose: &dyn Fn(usize) -> Message) -> Element<'_, Message> {
        let palette = self.palette;
        let heading = text(heading).size(14).font(palette.semibold);
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
                    .on_press(chose(index)),
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
    let mut cut: String = tail.chars().take(NAME_LENGTH - 1).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::shorten;

    #[test]
    fn a_long_name_loses_the_maker_not_the_model() {
        assert_eq!(shorten("FabFilter Pro-Q 4"), "Pro-Q 4");
        assert_eq!(shorten("FabFilter Saturn 2"), "Saturn 2");
        assert_eq!(shorten("Loupe EQ"), "Loupe EQ");
        assert_eq!(shorten("Valhalla Supermassive"), "Supermas\u{2026}");
        assert_eq!(shorten("Pro-Q 4"), "Pro-Q 4");
    }
}

pub fn slot_look(palette: crate::theme::Palette, on: bool, dragged: bool, landing: bool) -> iced::widget::container::Style {
    let shown = palette.toggled(on, iced::widget::button::Status::Active);
    let mut border = shown.border;
    if landing {
        border = border.width(1.0).color(palette.accent);
    }
    iced::widget::container::Style {
        background: if dragged { Some(palette.accent.scale_alpha(0.35).into()) } else { shown.background },
        text_color: Some(shown.text_color),
        border,
        ..Default::default()
    }
}

impl App {
    pub(crate) fn clip_fx_block(&self, clip: ClipId) -> Element<'_, Message> {
        let palette = self.palette;
        let Some(found) = self.project.clip(clip) else {
            return Space::new(Length::Fill, 0).into();
        };
        let mut rows = column![].spacing(3);
        for (slot, fx) in found.fx.iter().enumerate() {
            let on = !fx.bypassed;
            rows = rows.push(
                row![
                    button(text(fx.name.clone()).size(12.5))
                        .padding([2, 8])
                        .width(Length::Fill)
                        .style(move |_, status| palette.ghost(status))
                        .on_press(Message::ShowClipPlugin(clip, slot)),
                    button(text("Up").size(11.5))
                        .padding([2, 8])
                        .style(move |_, status| palette.outlined(status))
                        .on_press(Message::MoveClipPlugin(clip, slot, slot.saturating_sub(1))),
                    button(text("Down").size(11.5))
                        .padding([2, 8])
                        .style(move |_, status| palette.outlined(status))
                        .on_press(Message::MoveClipPlugin(clip, slot, slot + 1)),
                    button(text(if on { "On" } else { "Bypassed" }).size(11.5))
                        .padding([2, 8])
                        .style(move |_, status| palette.toggled(on, status))
                        .on_press(Message::BypassClipPlugin(clip, slot)),
                    button(text("Automate").size(11.5))
                        .padding([2, 8])
                        .style(move |_, status| palette.outlined(status))
                        .on_press(Message::OpenKnobs(crate::stockwin::Spot::Clip(clip), slot)),
                    button(text("Remove").size(11.5))
                        .padding([2, 8])
                        .style(move |_, status| palette.outlined(status))
                        .on_press(Message::RemoveClipPlugin(clip, slot)),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        }
        if found.fx.is_empty() {
            rows = rows.push(text("None yet.").size(12.5).color(palette.text_dim));
        }
        let add = button(text("Add a plugin").size(12.5))
            .padding([4, 10])
            .style(move |_, status| palette.outlined(status))
            .on_press(Message::OpenClipPlugins(clip));
        column![rows, add].spacing(8).width(Length::Fill).into()
    }
}

impl App {
    pub(crate) fn clip_plugin_sheet(&self, clip: ClipId) -> Element<'_, Message> {
        let Some(found) = self.project.clip(clip) else {
            return text("This clip is gone.").size(13).into();
        };
        let name = found.source.name.clone();
        self.picker(format!("Plugins for {name}"), &move |which| Message::AddClipPlugin(clip, which))
    }
}

impl App {
    pub(crate) fn knob_sheet(&self, spot: crate::stockwin::Spot, slot: usize) -> Element<'_, Message> {
        let palette = self.palette;
        let (name, where_) = match spot {
            crate::stockwin::Spot::Track(track) => (
                self.project
                    .tracks
                    .iter()
                    .find(|t| t.id == track)
                    .and_then(|t| t.fx.get(slot))
                    .map(|fx| fx.name.clone())
                    .unwrap_or_default(),
                crate::racks::Spot::Track(track, slot),
            ),
            crate::stockwin::Spot::Clip(clip) => (
                self.project
                    .clip(clip)
                    .and_then(|found| found.fx.get(slot))
                    .map(|fx| fx.name.clone())
                    .unwrap_or_default(),
                crate::racks::Spot::Clip(clip, slot),
            ),
        };
        let knobs = self.peek_at(where_).knobs;
        let heading = text(format!("Automate on {name}")).size(14).font(palette.semibold);
        let needle = self.plugin_filter.trim().to_lowercase();
        let mut list = column![].spacing(4);
        let mut shown = 0;
        for (knob, label) in knobs.iter().enumerate() {
            if !needle.is_empty() && !label.to_lowercase().contains(&needle) {
                continue;
            }
            shown += 1;
            let target = match spot {
                crate::stockwin::Spot::Track(track) => loupe_engine::Target::TrackFx { track, slot, knob },
                crate::stockwin::Spot::Clip(clip) => loupe_engine::Target::ClipFx { clip, slot, knob },
            };
            let on = self.project.envelope(target).is_some();
            list = list.push(
                button(text(label.clone()).size(12.5))
                    .padding([5, 10])
                    .width(Length::Fill)
                    .style(move |_, status| palette.toggled(on, status))
                    .on_press(if on { Message::RemoveEnvelope(target) } else { Message::AddEnvelope(target) }),
            );
        }
        let body: Element<'_, Message> = if knobs.is_empty() {
            text("This plugin does not tell Loupe what its knobs are called.").size(12.5).color(palette.text_dim).into()
        } else if shown == 0 {
            text("Nothing matches that.").size(12.5).color(palette.text_dim).into()
        } else {
            scrollable(list).height(Length::Fixed(300.0)).into()
        };
        let search = text_input("Search", &self.plugin_filter)
            .id(FILTER_ID)
            .on_input(Message::PluginFilter)
            .size(13)
            .padding([5, 10]);
        container(column![heading, search, body].spacing(10).align_x(Alignment::Start))
            .width(Length::Fixed(440.0))
            .into()
    }
}
