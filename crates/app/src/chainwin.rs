use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{button, column, container, mouse_area, row, scrollable, slider, text, Space};
use iced::{Alignment, Element, Length};

use loupe_engine::Fx;

use crate::stockwin::Spot;
use crate::{icon, App, Message};

const WINDOW_WIDTH: f32 = 560.0;
const LIST_HEIGHT: f32 = 320.0;
const MIX_WIDTH: f32 = 120.0;
const READING_WIDTH: f32 = 44.0;
const DOT: f32 = 11.0;

impl App {
    pub(crate) fn chain_of(&self, spot: Spot) -> Option<&[Fx]> {
        match spot {
            Spot::Track(track) => self.project.tracks.iter().find(|found| found.id == track).map(|found| found.fx.as_slice()),
            Spot::Clip(clip) => self.project.clip(clip).map(|found| found.fx.as_slice()),
            Spot::Master => Some(self.project.master_fx.as_slice()),
            Spot::Monitor => Some(self.project.monitor_fx.as_slice()),
        }
    }

    pub(crate) fn chain_name(&self, spot: Spot) -> String {
        match spot {
            Spot::Track(track) => self
                .project
                .tracks
                .iter()
                .find(|found| found.id == track)
                .map(|found| found.name.clone())
                .unwrap_or_else(|| "Track".to_string()),
            Spot::Clip(clip) => self.project.clip(clip).map(|found| found.called().to_string()).unwrap_or_else(|| "Clip".to_string()),
            Spot::Master => "Master".to_string(),
            Spot::Monitor => "Monitoring".to_string(),
        }
    }

    pub(crate) fn chain_sheet(&self, spot: Spot) -> Element<'_, Message> {
        let palette = self.palette;
        let Some(chain) = self.chain_of(spot) else {
            return text("That is gone.").size(13).into();
        };
        let recording = matches!(spot, Spot::Track(track) if self.rec_shown.contains(&track));
        let shown: Vec<(usize, &Fx)> = chain
            .iter()
            .enumerate()
            .filter(|(_, fx)| matches!(spot, Spot::Track(_)).then_some(fx.record == recording).unwrap_or(true))
            .collect();
        let mut rows = column![].spacing(4);
        for (slot, fx) in &shown {
            rows = rows.push(self.chain_row(spot, *slot, fx));
        }
        if shown.is_empty() {
            rows = rows.push(
                container(text("Nothing on this chain yet.").size(12.5).color(palette.text_dim))
                    .center_x(Length::Fill)
                    .padding(20),
            );
        }
        let listed = container(scrollable(rows).height(Length::Fixed(LIST_HEIGHT)).direction(Direction::Vertical(Scrollbar::new())))
            .padding(8)
            .width(Length::Fill)
            .style(move |_| palette.readout());
        let add = button(row![icon("plus", 13.0), text("Add a plugin").size(13).font(palette.medium)].spacing(7).align_y(Alignment::Center))
            .padding([7, 14])
            .style(move |_, status| palette.solid(status))
            .on_press(self.add_to(spot));
        let mut body = column![].spacing(12);
        if let Spot::Track(track) = spot {
            body = body.push(self.chain_tabs(track, recording));
        }
        body = body.push(listed).push(row![add, Space::with_width(Length::Fill)].align_y(Alignment::Center));
        if let Some(chains) = self.chain_block() {
            body = body.push(chains);
        }
        self.window(format!("FX — {}", self.chain_name(spot)), body.into(), WINDOW_WIDTH)
    }

    fn chain_tabs(&self, track: loupe_engine::TrackId, recording: bool) -> Element<'_, Message> {
        let palette = self.palette;
        let taking = self.project.tracks.iter().find(|found| found.id == track).map_or(0, |found| found.fx.iter().filter(|fx| fx.record).count());
        let tab = |label: String, rec: bool| {
            let on = rec == recording;
            button(text(label).size(12.5).font(palette.medium))
                .padding([6, 12])
                .style(move |_, status| palette.toggled(on, status))
                .on_press(Message::ShowChain(track, rec))
        };
        let mixing = self.project.tracks.iter().find(|found| found.id == track).map_or(0, |found| found.fx.iter().filter(|fx| !fx.record).count());
        let counted = |word: &str, many: usize| if many > 0 { format!("{word} ({many})") } else { word.to_string() };
        row![tab(counted("Mix", mixing), false), tab(counted("Rec", taking), true), Space::with_width(Length::Fill)]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
    }

    fn chain_row(&self, spot: Spot, slot: usize, fx: &Fx) -> Element<'_, Message> {
        let palette = self.palette;
        let on = !fx.bypassed;
        let held = self.chain_drag == Some((spot, slot));
        let over = self.chain_over == Some((spot, slot));
        let lit = if on { palette.meter_low } else { palette.danger };
        let dot = button(Space::new(DOT, DOT))
            .padding(0)
            .style(move |_, status| iced::widget::button::Style {
                background: Some(iced::Background::Color(match status {
                    iced::widget::button::Status::Hovered => crate::theme::alpha(lit, 0.75),
                    _ => lit,
                })),
                border: iced::Border::default().rounded(DOT / 2.0),
                ..Default::default()
            })
            .on_press(self.bypass_of(spot, slot));
        let name = text(fx.name.clone())
            .size(13)
            .font(palette.medium)
            .color(if on { palette.text } else { palette.text_dim })
            .wrapping(iced::widget::text::Wrapping::None);
        let open = mouse_area(container(name).width(Length::Fill).padding([2, 4]))
            .on_press(self.open_of(spot, slot))
            .on_right_press(Message::StartRenameFx(spot, slot))
            .interaction(iced::mouse::Interaction::Pointer);
        let grip = mouse_area(container(icon("layers-2", 12.0).color(palette.text_faint)).padding([2, 4]))
            .on_press(Message::ChainGrab(spot, slot))
            .on_move(move |_| Message::ChainOver(spot, slot))
            .on_release(Message::ChainDrop)
            .interaction(iced::mouse::Interaction::Grab);
        let percent = (fx.mix * 100.0).round();
        let mix = slider(0.0..=100.0, percent, move |now| Message::SetFxMix(spot, slot, now / 100.0))
            .step(1.0)
            .default(100.0)
            .width(MIX_WIDTH)
            .style(move |_, status| palette.slider(status));
        let reading = text(format!("{percent:.0}%")).size(11.5).font(palette.mono).color(palette.text_dim).width(READING_WIDTH);
        let drop = button(icon("x", 12.0))
            .padding([4, 8])
            .style(move |_, status| palette.ghost(status))
            .on_press(self.remove_of(spot, slot));
        let waiting_for_room: Element<'_, Message> = if self.held_back_at(spot, slot) {
            row![
                text("Not loaded").size(11.5).color(palette.text_dim),
                button(text("Load").size(11.5).font(palette.medium))
                    .padding([4, 10])
                    .style(move |_, status| palette.outlined(status))
                    .on_press(Message::LoadHeldBackPlugin(spot, slot)),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
        } else {
            Space::new(0, 0).into()
        };
        let line = row![grip, dot, open, waiting_for_room, mix, reading, drop].spacing(8).align_y(Alignment::Center);
        container(line)
            .padding([4, 6])
            .width(Length::Fill)
            .style(move |_| crate::plugins::slot_look(palette, on, held, over))
            .into()
    }

    fn add_to(&self, spot: Spot) -> Message {
        match spot {
            Spot::Track(track) => Message::OpenPlugins(track),
            Spot::Clip(clip) => Message::OpenClipPlugins(clip),
            Spot::Master => Message::OpenMasterPlugins,
            Spot::Monitor => Message::OpenMonitorPlugins,
        }
    }

    fn bypass_of(&self, spot: Spot, slot: usize) -> Message {
        match spot {
            Spot::Track(track) => Message::BypassPlugin(track, slot),
            Spot::Clip(clip) => Message::BypassClipPlugin(clip, slot),
            Spot::Master => Message::BypassMasterPlugin(slot),
            Spot::Monitor => Message::BypassMonitorPlugin(slot),
        }
    }

    fn open_of(&self, spot: Spot, slot: usize) -> Message {
        match spot {
            Spot::Track(track) => Message::ShowPlugin(track, slot),
            Spot::Clip(clip) => Message::ShowClipPlugin(clip, slot),
            Spot::Master => Message::ShowMasterPlugin(slot),
            Spot::Monitor => Message::ShowMonitorPlugin(slot),
        }
    }

    fn remove_of(&self, spot: Spot, slot: usize) -> Message {
        match spot {
            Spot::Track(track) => Message::RemovePlugin(track, slot),
            Spot::Clip(clip) => Message::RemoveClipPlugin(clip, slot),
            Spot::Master => Message::RemoveMasterPlugin(slot),
            Spot::Monitor => Message::RemoveMonitorPlugin(slot),
        }
    }
}

impl App {
    pub(crate) fn set_fx_mix(&mut self, spot: Spot, slot: usize, mix: f32) {
        let command = match spot {
            Spot::Track(track) => loupe_engine::Command::SetFxMix { track, slot, mix },
            Spot::Clip(clip) => loupe_engine::Command::SetClipFxMix { clip, slot, mix },
            Spot::Master => loupe_engine::Command::SetMasterFxMix { slot, mix },
            Spot::Monitor => loupe_engine::Command::SetMonitorFxMix { slot, mix },
        };
        self.edit(Some(crate::Run::FxMix(spot, slot)), command);
    }

    pub(crate) fn drop_chain(&mut self) {
        let (Some((spot, from)), Some((_, to))) = (self.chain_drag.take(), self.chain_over.take()) else {
            return;
        };
        if from == to {
            return;
        }
        let command = match spot {
            Spot::Track(track) => loupe_engine::Command::MoveFx { track, slot: from, to },
            Spot::Clip(clip) => loupe_engine::Command::MoveClipFx { clip, slot: from, to },
            Spot::Master => loupe_engine::Command::MoveMasterFx { slot: from, to },
            Spot::Monitor => loupe_engine::Command::MoveMonitorFx { slot: from, to },
        };
        self.edit(None, command);
    }
}

impl App {
    pub(crate) fn fx_button(&self, spot: Spot) -> Element<'_, Message> {
        let palette = self.palette;
        let count = self.chain_of(spot).map_or(0, <[Fx]>::len);
        let word = if spot == Spot::Monitor { "Mon FX" } else { "FX" };
        let label = if count > 0 { format!("{word} {count}") } else { word.to_string() };
        button(text(label).size(11).font(palette.medium).width(Length::Fill).center())
            .padding([3, 0])
            .width(Length::Fill)
            .style(move |_, status| palette.outlined(status))
            .on_press(Message::OpenChainInMixer(spot))
            .into()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use loupe_engine::Command;

    use super::*;
    use crate::Overlay;

    fn plugin(name: &str) -> Fx {
        Fx { path: PathBuf::from(format!("/nowhere/{name}.vst3")), index: 0, name: name.into(), bypassed: false, state: Vec::new(), record: false, mix: 1.0 }
    }

    fn named(app: &App) -> Vec<String> {
        app.project.monitor_fx.iter().map(|fx| fx.name.clone()).collect()
    }

    #[test]
    fn the_chain_window_edits_the_monitoring_chain_the_way_it_edits_the_master() {
        let mut app = App::bare();
        assert_eq!(app.chain_name(Spot::Monitor), "Monitoring");
        assert!(matches!(app.add_to(Spot::Monitor), Message::OpenMonitorPlugins));
        for name in ["Denoiser", "RVox", "Pro-R 2"] {
            app.project.apply(Command::AddMonitorFx(plugin(name))).unwrap();
        }
        assert_eq!(app.chain_of(Spot::Monitor).map(<[Fx]>::len), Some(3));

        let _ = app.update(Message::BypassMonitorPlugin(1));
        assert!(app.project.monitor_fx[1].bypassed, "the dot did not switch the plugin off");
        let _ = app.update(Message::SetFxMix(Spot::Monitor, 2, 0.35));
        assert!((app.project.monitor_fx[2].mix - 0.35).abs() < 1e-6, "the mix slider did nothing");

        app.chain_drag = Some((Spot::Monitor, 0));
        app.chain_over = Some((Spot::Monitor, 2));
        app.drop_chain();
        assert_eq!(named(&app), ["RVox", "Pro-R 2", "Denoiser"], "dragging did not reorder the chain");

        let _ = app.update(Message::StartRenameFx(Spot::Monitor, 0));
        assert_eq!(app.overlay, Overlay::RenameFx(Spot::Monitor, 0));
        let _ = app.update(Message::EntryTyped("Waves RVox".into()));
        let _ = app.update(Message::EntryEntered);
        assert_eq!(named(&app)[0], "Waves RVox");

        let _ = app.update(Message::RemoveMonitorPlugin(1));
        assert_eq!(named(&app), ["Waves RVox", "Denoiser"]);
        assert!(app.project.master_fx.is_empty(), "the monitoring chain reached into the master chain");
    }
}
