use std::path::Path;
use std::sync::Arc;

use iced::widget::{column, container, row, scrollable, text, Space};
use iced::{Alignment, Border, Color, Element, Length};
use loupe_engine::{Fx, Source};

use crate::racks::Spot;
use crate::resources::{memory_text, Usage};
use crate::theme::Palette;
use crate::{App, Message};

const WIDTH: f32 = 860.0;
const TILE_HEIGHT: f32 = 106.0;
const PLUGIN_LIST_HEIGHT: f32 = 236.0;
const BAR_HEIGHT: f32 = 4.0;
const NUMBER_WIDTH: f32 = 66.0;
const BYTES_PER_FRAME: u64 = 8;

struct PluginRow {
    name: String,
    place: String,
    hosted: bool,
    usage: Option<Usage>,
}

fn percent(value: f32) -> String {
    format!("{:.0}%", value * 100.0)
}

fn bar<'a>(palette: Palette, share: f32, colour: Color) -> Element<'a, Message> {
    let share = share.clamp(0.0, 1.0);
    let filled = container(Space::new(Length::Fill, BAR_HEIGHT)).width(Length::FillPortion((share * 1000.0).round().max(1.0) as u16)).style(move |_| container::Style {
        background: Some(colour.into()),
        border: Border::default().rounded(2),
        ..Default::default()
    });
    let empty = Space::new(Length::FillPortion(((1.0 - share) * 1000.0).round().max(1.0) as u16), BAR_HEIGHT);
    container(row![filled, empty])
        .width(Length::Fill)
        .style(move |_| container::Style { background: Some(palette.raised.into()), border: Border::default().rounded(2), ..Default::default() })
        .into()
}

fn tile<'a>(palette: Palette, label: &'static str, value: String, detail: String, meter: Option<(f32, Color)>) -> Element<'a, Message> {
    let mut body = column![
        text(label).size(11.5).color(palette.text_dim),
        text(value).size(26).font(palette.semibold),
        text(detail).size(11.5).color(palette.text_dim),
    ]
    .spacing(3);
    if let Some((share, colour)) = meter {
        body = body.push(bar(palette, share, colour));
    }
    container(body)
        .padding([12, 14])
        .width(Length::Fill)
        .height(TILE_HEIGHT)
        .style(move |_| container::Style {
            background: Some(palette.panel.into()),
            border: Border::default().rounded(palette.corner).width(1).color(palette.line),
            ..Default::default()
        })
        .into()
}

fn pair<'a>(palette: Palette, label: &'static str, value: String) -> Element<'a, Message> {
    row![text(label).size(12.5).color(palette.text_dim).width(Length::Fill), text(value).size(12.5).font(palette.mono)].spacing(12).into()
}

fn heading<'a>(palette: Palette, words: &'static str) -> Element<'a, Message> {
    text(words).size(12.5).font(palette.semibold).color(palette.text).into()
}

pub fn audio_held(sources: &[Arc<Source>], stretched: impl Iterator<Item = Arc<Source>>) -> u64 {
    let mut seen: Vec<*const Source> = Vec::new();
    let mut bytes = 0;
    for source in sources.iter().cloned().chain(stretched) {
        let at = Arc::as_ptr(&source);
        if seen.contains(&at) {
            continue;
        }
        seen.push(at);
        bytes += source.frames.len() as u64 * BYTES_PER_FRAME;
    }
    bytes
}

fn load_colour(palette: Palette, load: f32) -> Color {
    if load >= 0.9 {
        palette.danger
    } else if load >= 0.6 {
        palette.meter_mid
    } else {
        palette.meter_low
    }
}

impl App {
    fn plugin_rows(&self) -> Vec<PluginRow> {
        let built_in = Path::new(loupe_plugins::BUILT_IN);
        let mut rows = Vec::new();
        let mut add = |fx: &Fx, place: String, spot: Spot| {
            let hosted = fx.path != built_in;
            let usage = if hosted { self.peek_at(spot).host.and_then(|pid| self.resources.of(pid)) } else { None };
            rows.push(PluginRow { name: fx.name.clone(), place, hosted, usage });
        };
        for track in &self.project.tracks {
            for (slot, fx) in track.fx.iter().enumerate() {
                let place = if fx.record { format!("{} · Rec", track.name) } else { track.name.clone() };
                add(fx, place, Spot::Track(track.id, slot));
            }
            for clip in &track.clips {
                for (slot, fx) in clip.fx.iter().enumerate() {
                    add(fx, format!("Clip on {}", track.name), Spot::Clip(clip.id, slot));
                }
            }
        }
        for (slot, fx) in self.project.master_fx.iter().enumerate() {
            add(fx, "Master".to_string(), Spot::Master(slot));
        }
        rows.sort_by(|a, b| {
            let cpu = |row: &PluginRow| row.usage.and_then(|usage| usage.cpu).unwrap_or(-1.0);
            cpu(b).total_cmp(&cpu(a)).then_with(|| b.hosted.cmp(&a.hosted))
        });
        rows
    }

    pub(crate) fn performance_sheet(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let load = self.engine.load();
        let cpu = self.resources.cpu;
        let memory = self.resources.memory;
        let own = self.resources.own();
        let rows = self.plugin_rows();
        let hosted: Vec<&PluginRow> = rows.iter().filter(|row| row.hosted).collect();
        let hosts_memory: u64 = hosted.iter().filter_map(|row| row.usage.map(|usage| usage.memory)).sum();
        let held = audio_held(&self.project.sources, self.project.tracks.iter().flat_map(|track| track.clips.iter().filter_map(|clip| clip.stretched.clone())));

        let tiles = row![
            tile(palette, "Audio engine", percent(load.average), format!("peak {}", percent(load.peak)), Some((load.peak, load_colour(palette, load.peak)))),
            tile(palette, "CPU", cpu.map_or("…".to_string(), |cpu| format!("{cpu:.0}%")), "Loupe and its plugin hosts".to_string(), cpu.map(|cpu| (cpu / 100.0, palette.accent))),
            tile(palette, "Memory", memory.map_or("…".to_string(), |bytes| memory_text(bytes).trim_start_matches("RAM ").to_string()), "Loupe and its plugin hosts".to_string(), None),
            tile(palette, "Overruns", load.overloads.to_string(), "dropouts since Loupe opened".to_string(), None),
        ]
        .spacing(10);

        let running = self.engine.running();
        let rate = self.engine.rate();
        let buffer = running.and_then(|found| found.buffer);
        let mut device = column![heading(palette, "Audio device")].spacing(7);
        match running {
            Some(found) => {
                device = device.push(pair(palette, "Driver", found.driver.clone()));
                device = device.push(pair(palette, "Output", found.output.clone()));
            }
            None => device = device.push(pair(palette, "Output", "None, playing silently".to_string())),
        }
        device = device.push(pair(palette, "Sample rate", format!("{rate} Hz")));
        device = device.push(pair(
            palette,
            "Buffer",
            buffer.map_or("Driver default".to_string(), |frames| format!("{frames} frames · {:.1} ms", frames as f32 * 1000.0 / rate.max(1) as f32)),
        ));

        let plugin_count = rows.len();
        let clip_count: usize = self.project.tracks.iter().map(|track| track.clips.len()).sum();
        let memory_rows = column![
            heading(palette, "Memory"),
            pair(palette, "Loupe itself", own.map_or("…".to_string(), |usage| memory_text(usage.memory).trim_start_matches("RAM ").to_string())),
            pair(palette, "Audio held for playback", memory_text(held).trim_start_matches("RAM ").to_string()),
            pair(palette, "Plugin hosts", memory_text(hosts_memory).trim_start_matches("RAM ").to_string()),
            heading(palette, "Song"),
            pair(palette, "Tracks", self.project.tracks.len().to_string()),
            pair(palette, "Clips", clip_count.to_string()),
            pair(palette, "Plugins", format!("{plugin_count} ({} in their own host)", hosted.len())),
        ]
        .spacing(7);
        let left = column![device, memory_rows].spacing(18).width(Length::FillPortion(2));

        let header = row![
            text("Plugin").size(11.5).color(palette.text_dim).width(Length::FillPortion(3)),
            text("On").size(11.5).color(palette.text_dim).width(Length::FillPortion(2)),
            text("CPU").size(11.5).color(palette.text_dim).width(NUMBER_WIDTH),
            text("Memory").size(11.5).color(palette.text_dim).width(NUMBER_WIDTH),
        ]
        .spacing(8);
        let mut list = column![].spacing(2);
        for found in &rows {
            let (cpu, memory) = match (found.hosted, found.usage) {
                (false, _) => ("in Loupe".to_string(), "in Loupe".to_string()),
                (true, Some(usage)) => (usage.cpu.map_or("…".to_string(), |cpu| format!("{cpu:.1}%")), memory_text(usage.memory).trim_start_matches("RAM ").to_string()),
                (true, None) => ("loading".to_string(), "…".to_string()),
            };
            list = list.push(
                container(
                    row![
                        text(found.name.clone()).size(12.5).width(Length::FillPortion(3)).wrapping(iced::widget::text::Wrapping::None),
                        text(found.place.clone()).size(12.5).color(palette.text_dim).width(Length::FillPortion(2)).wrapping(iced::widget::text::Wrapping::None),
                        text(cpu).size(12.5).font(palette.mono).width(NUMBER_WIDTH),
                        text(memory).size(12.5).font(palette.mono).width(NUMBER_WIDTH),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .padding([5, 8])
                .clip(true),
            );
        }
        let plugins_body: Element<'_, Message> = if rows.is_empty() {
            text("No plugins in this song.").size(12.5).color(palette.text_dim).into()
        } else {
            scrollable(list).height(PLUGIN_LIST_HEIGHT).into()
        };
        let plugins = column![
            heading(palette, "Plugins"),
            container(column![container(header).padding([0, 8]), plugins_body].spacing(6)).padding(8).style(move |_| container::Style {
                background: Some(palette.panel.into()),
                border: Border::default().rounded(palette.corner).width(1).color(palette.line),
                ..Default::default()
            }),
            text("Plugins from other makers each run in their own host, so their CPU and memory are theirs alone. Loupe's own plugins run inside the audio engine and count towards it.")
                .size(11.5)
                .color(palette.text_dim),
        ]
        .spacing(8)
        .width(Length::FillPortion(3));

        let body = column![
            tiles,
            text("Audio engine is how much of each buffer's time Loupe spends making the sound. Near 100% you will hear clicks and dropouts.").size(11.5).color(palette.text_dim),
            row![left, plugins].spacing(24),
        ]
        .spacing(14);
        self.window("Performance".to_string(), body.into(), WIDTH)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_in_memory_counts_each_source_once() {
        let one = Arc::new(Source::from_frames("one", vec![[0.0; 2]; 1_000]));
        let two = Arc::new(Source::from_frames("two", vec![[0.0; 2]; 500]));
        let stretched = Arc::new(Source::from_frames("one stretched", vec![[0.0; 2]; 2_000]));
        let held = audio_held(&[one.clone(), two, one.clone()], vec![stretched.clone(), stretched].into_iter());
        assert_eq!(held, (1_000 + 500 + 2_000) * BYTES_PER_FRAME);
    }
}
