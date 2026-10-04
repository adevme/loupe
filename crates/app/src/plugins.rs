use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{button, column, container, mouse_area, row, scrollable, text, text_input, Space};
use iced::{Alignment, Element, Length};

use loupe_engine::{ClipId, TrackId};
use loupe_plugins::Found;

use crate::{App, Message};

pub const FILTER_ID: &str = "plugin-filter";
const NAME_LENGTH: usize = 20;
const DOT: f32 = 9.0;
const MASTER_SLOT_HINT: &str = "Click to open it, right click to remove. These run over the whole mix.";
const SLOT_HINT: &str = "Click to open it, drag to reorder, right click to remove, middle click for its knobs";

pub fn find_plugins() -> Vec<Found> {
    let mut found = newest_shells(loupe_plugins::everything());
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
            // The strip is narrow, so the name gets the whole row. Everything else is
            // the dot on the left, or a click of the right button.
            let dot = button(Space::new(DOT, DOT))
                .padding(0)
                .style(move |_, status| palette.toggled(!on, status))
                .on_press(Message::BypassPlugin(track, slot));
            let name = container(text(short).size(10.5).wrapping(iced::widget::text::Wrapping::None))
                .padding(iced::Padding { top: 1.0, right: 3.0, bottom: 1.0, left: 3.0 })
                .width(Length::Fill)
                .style(move |_| crate::plugins::slot_look(palette, on, dragged, landing));
            rows = rows.push(
                row![
                    dot,
                    mouse_area(name)
                        .on_press(Message::FxGrab(track, slot))
                        .on_move(move |_| Message::FxOver(slot))
                        .on_release(Message::FxDrop)
                        .on_right_press(Message::RemovePlugin(track, slot))
                        .on_middle_press(Message::OpenKnobs(crate::stockwin::Spot::Track(track), slot))
                        .on_enter(Message::Hint(Some(SLOT_HINT)))
                        .on_exit(Message::Hint(None))
                        .interaction(iced::mouse::Interaction::Grab),
                ]
                .spacing(3)
                .align_y(iced::Alignment::Center),
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

    /// The plugins on offer, the ones reached for most often first, with the keyboard
    /// able to walk the list and pick without touching the mouse.
    pub(crate) fn in_the_picker(&self) -> Vec<usize> {
        let needle = self.plugin_filter.trim().to_lowercase();
        let mut order: Vec<usize> = self
            .found
            .iter()
            .enumerate()
            .filter(|(_, plugin)| {
                needle.is_empty()
                    || format!("{} {}", plugin.name, plugin.vendor.clone().unwrap_or_default()).to_lowercase().contains(&needle)
            })
            .map(|(index, _)| index)
            .collect();
        order.sort_by(|a, b| {
            let (one, two) = (&self.found[*a], &self.found[*b]);
            self.plugin_uses
                .count(&two.name)
                .cmp(&self.plugin_uses.count(&one.name))
                .then_with(|| one.name.to_lowercase().cmp(&two.name.to_lowercase()))
        });
        order
    }

    pub(crate) fn picker(&self, heading: String, chose: &dyn Fn(usize) -> Message) -> Element<'_, Message> {
        let palette = self.palette;
        let search = text_input("Search", &self.plugin_filter)
            .id(FILTER_ID)
            .on_input(Message::PluginFilter)
            .on_submit(Message::PluginChosen)
            .size(13)
            .padding([5, 10])
            .style(move |_, status| palette.field(status));
        let order = self.in_the_picker();
        let mut list = column![].spacing(2);
        for (place, index) in order.iter().enumerate() {
            let plugin = &self.found[*index];
            let label = match &plugin.vendor {
                Some(vendor) => format!("{}  ·  {vendor}  ·  {}", plugin.name, plugin.format.label()),
                None => format!("{}  ·  {}", plugin.name, plugin.format.label()),
            };
            let under_the_keys = place == self.plugin_highlight;
            list = list.push(
                button(text(label).size(12.5))
                    .padding([6, 10])
                    .width(Length::Fill)
                    .style(move |_, status| palette.menu_item(if under_the_keys { button::Status::Hovered } else { status }))
                    .on_press(chose(*index)),
            );
        }
        let body: Element<'_, Message> = if self.scanning {
            text("Looking for plugins…").size(12.5).color(palette.text_dim).into()
        } else if self.found.is_empty() {
            text("No plugins found in the usual folders.").size(12.5).color(palette.text_dim).into()
        } else if order.is_empty() {
            text("Nothing matches that.").size(12.5).color(palette.text_dim).into()
        } else {
            container(scrollable(list).height(Length::Fixed(280.0)))
                .padding(4)
                .style(move |_| palette.menu())
                .into()
        };
        self.window(heading, column![search, body].spacing(10).into(), 460.0)
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
        // The slot now has the whole strip width, so most names arrive whole.
        assert_eq!(shorten("FabFilter Pro-Q 4"), "FabFilter Pro-Q 4");
        assert_eq!(shorten("Loupe EQ"), "Loupe EQ");
        assert_eq!(shorten("Valhalla Supermassive"), "Supermassive");
        assert_eq!(shorten("Auburn Sounds Graillon 3"), "Sounds Graillon 3");
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
        if spot == crate::stockwin::Spot::Master {
            // Master plugins run, but their knobs have no envelope target yet.
            return self.window(
                "Automate".to_string(),
                text("Automating master plugins is not built yet.").size(12.5).color(palette.text_dim).into(),
                360.0,
            );
        }
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
            crate::stockwin::Spot::Master => (String::new(), crate::racks::Spot::Master(slot)),
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
                crate::stockwin::Spot::Master => continue,
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

const USES_FILE: &str = "plugin-uses";

/// How often each plugin has been reached for, so the ones actually used come first.
#[derive(Default)]
pub struct Uses(std::collections::HashMap<String, u32>);

impl Uses {
    pub fn load() -> Self {
        let Some(path) = crate::settings::config_dir().map(|dir| dir.join(USES_FILE)) else {
            return Self::default();
        };
        let text = std::fs::read_to_string(path).unwrap_or_default();
        Self(
            text.lines()
                .filter_map(|line| line.split_once('\t'))
                .filter_map(|(count, name)| count.trim().parse().ok().map(|count| (name.to_string(), count)))
                .collect(),
        )
    }

    pub fn count(&self, name: &str) -> u32 {
        self.0.get(name).copied().unwrap_or(0)
    }

    pub fn reached_for(&mut self, name: &str) {
        *self.0.entry(name.to_string()).or_insert(0) += 1;
        let Some(path) = crate::settings::config_dir().map(|dir| dir.join(USES_FILE)) else {
            return;
        };
        let mut lines: Vec<String> = self.0.iter().map(|(name, count)| format!("{count}\t{name}")).collect();
        lines.sort();
        let _ = std::fs::write(path, lines.join("\n"));
    }
}

/// A Waves shell holds every Waves plugin, and one is installed per version. Showing
/// each of them is showing the same plugins over and over, so keep the newest shell.
fn newest_shells(found: Vec<Found>) -> Vec<Found> {
    let family = |name: &str| name.split_whitespace().next().unwrap_or(name).to_string();
    let is_shell = |name: &str| name.to_lowercase().starts_with("waveshell");
    let mut best: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for plugin in found.iter().filter(|plugin| is_shell(&plugin.name)) {
        let family = family(&plugin.name);
        match best.get(&family) {
            Some(kept) if kept.as_str() >= plugin.name.as_str() => {}
            _ => {
                best.insert(family, plugin.name.clone());
            }
        }
    }
    found
        .into_iter()
        .filter(|plugin| !is_shell(&plugin.name) || best.get(&family(&plugin.name)) == Some(&plugin.name))
        .collect()
}

impl App {
    /// The same slot rows as a track, for the plugins over the whole mix.
    pub(crate) fn master_fx_block(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let mut rows = column![].spacing(2);
        for (slot, fx) in self.project.master_fx.iter().enumerate() {
            let on = !fx.bypassed;
            let dot = button(Space::new(DOT, DOT))
                .padding(0)
                .style(move |_, status| palette.toggled(!on, status))
                .on_press(Message::BypassMasterPlugin(slot));
            let name = container(text(shorten(&fx.name)).size(10.5).wrapping(iced::widget::text::Wrapping::None))
                .padding(iced::Padding { top: 1.0, right: 3.0, bottom: 1.0, left: 3.0 })
                .width(Length::Fill)
                .style(move |_| crate::plugins::slot_look(palette, on, false, false));
            rows = rows.push(
                row![
                    dot,
                    mouse_area(name)
                        .on_press(Message::ShowMasterPlugin(slot))
                        .on_right_press(Message::RemoveMasterPlugin(slot))
                        .on_enter(Message::Hint(Some(MASTER_SLOT_HINT)))
                        .on_exit(Message::Hint(None))
                        .interaction(iced::mouse::Interaction::Pointer),
                ]
                .spacing(3)
                .align_y(iced::Alignment::Center),
            );
        }
        let add = button(text("+ FX").size(10.5))
            .padding([1, 4])
            .width(Length::Fill)
            .style(move |_, status| palette.ghost(status))
            .on_press(Message::OpenMasterPlugins);
        column![scrollable(rows).height(Length::Fixed(44.0)).direction(Direction::Vertical(Scrollbar::new().width(4).scroller_width(4))), add]
            .spacing(3)
            .width(Length::Fill)
            .into()
    }
}
