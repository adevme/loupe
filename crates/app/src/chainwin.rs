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
        let rec_label = if taking > 0 { format!("Rec ({taking})") } else { "Rec".to_string() };
        row![tab("Mix".to_string(), false), tab(rec_label, true), Space::with_width(Length::Fill)]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
    }

    fn chain_row(&self, spot: Spot, slot: usize, fx: &Fx) -> Element<'_, Message> {
        let palette = self.palette;
        let on = !fx.bypassed;
        let held = self.chain_drag == Some((spot, slot));
        let over = self.chain_over == Some((spot, slot));
        let dot = button(Space::new(DOT, DOT))
            .padding(0)
            .style(move |_, status| palette.toggled(on, status))
            .on_press(self.bypass_of(spot, slot));
        let name = text(fx.name.clone())
            .size(13)
            .font(palette.medium)
            .color(if on { palette.text } else { palette.text_dim })
            .wrapping(iced::widget::text::Wrapping::None);
        let open = mouse_area(container(name).width(Length::Fill).padding([2, 4]))
            .on_press(self.open_of(spot, slot))
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
        let line = row![grip, dot, open, mix, reading, drop].spacing(8).align_y(Alignment::Center);
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
        }
    }

    fn bypass_of(&self, spot: Spot, slot: usize) -> Message {
        match spot {
            Spot::Track(track) => Message::BypassPlugin(track, slot),
            Spot::Clip(clip) => Message::BypassClipPlugin(clip, slot),
            Spot::Master => Message::BypassMasterPlugin(slot),
        }
    }

    fn open_of(&self, spot: Spot, slot: usize) -> Message {
        match spot {
            Spot::Track(track) => Message::ShowPlugin(track, slot),
            Spot::Clip(clip) => Message::ShowClipPlugin(clip, slot),
            Spot::Master => Message::ShowMasterPlugin(slot),
        }
    }

    fn remove_of(&self, spot: Spot, slot: usize) -> Message {
        match spot {
            Spot::Track(track) => Message::RemovePlugin(track, slot),
            Spot::Clip(clip) => Message::RemoveClipPlugin(clip, slot),
            Spot::Master => Message::RemoveMasterPlugin(slot),
        }
    }
}

impl App {
    pub(crate) fn set_fx_mix(&mut self, spot: Spot, slot: usize, mix: f32) {
        let command = match spot {
            Spot::Track(track) => loupe_engine::Command::SetFxMix { track, slot, mix },
            Spot::Clip(clip) => loupe_engine::Command::SetClipFxMix { clip, slot, mix },
            Spot::Master => loupe_engine::Command::SetMasterFxMix { slot, mix },
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
        };
        self.edit(None, command);
    }
}

impl App {
    pub(crate) fn fx_button(&self, spot: Spot) -> Element<'_, Message> {
        let palette = self.palette;
        let count = self.chain_of(spot).map_or(0, <[Fx]>::len);
        let label = if count > 0 { format!("FX {count}") } else { "FX".to_string() };
        button(text(label).size(11).font(palette.medium).width(Length::Fill).center())
            .padding([3, 0])
            .width(Length::Fill)
            .style(move |_, status| palette.outlined(status))
            .on_press(Message::OpenChain(spot))
            .into()
    }
}
