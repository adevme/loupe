
use iced::widget::{button, column, container, row, scrollable, text, text_input, Space};
use iced::{Alignment, Element, Length};

use loupe_engine::{ClipId, TrackId};
use loupe_plugins::Found;

use crate::{App, Message};

pub const FILTER_ID: &str = "plugin-filter";

pub fn find_plugins() -> Vec<Found> {
    let mut found = newest_shells(loupe_plugins::everything());
    found.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    found
}

impl App {

    pub(crate) fn plugin_sheet(&self, track: TrackId) -> Element<'_, Message> {
        let Some(found) = self.project.tracks.iter().find(|t| t.id == track) else {
            return text("This track is gone.").size(13).into();
        };
        let heading = if self.rec_shown.contains(&track) { format!("Rec plugins for {}", found.name) } else { format!("Plugins for {}", found.name) };
        return self.picker(heading, &move |which| Message::AddPlugin(track, which));
    }

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
        let chains = self.chain_block();
        self.window(heading, column![search, body].push_maybe(chains).spacing(10).into(), 460.0)
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
        let name = found.called().to_string();
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
            crate::stockwin::Spot::Master => (
                self.project.master_fx.get(slot).map(|fx| format!("Master {}", fx.name)).unwrap_or_default(),
                crate::racks::Spot::Master(slot),
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
                crate::stockwin::Spot::Master => loupe_engine::Target::MasterFx { slot, knob },
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
