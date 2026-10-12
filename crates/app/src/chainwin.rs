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

    pub(crate) fn chain_is_on(&self, spot: Spot) -> bool {
        match spot {
            Spot::Track(track) => self.project.tracks.iter().find(|found| found.id == track).is_none_or(|found| found.fx_on),
            Spot::Clip(_) => true,
            Spot::Master => self.project.master_fx_on,
            Spot::Monitor => self.project.monitor_fx_on,
        }
    }

    pub(crate) fn chain_is_running(&self, spot: Spot) -> bool {
        self.project.all_fx_on && self.chain_is_on(spot)
    }

    fn chain_switch(&self, spot: Spot) -> Element<'_, Message> {
        let palette = self.palette;
        let on = self.chain_is_on(spot);
        let lit = if on { palette.meter_low } else { palette.danger };
        let dot = container(Space::new(DOT, DOT)).style(move |_| iced::widget::container::Style {
            background: Some(iced::Background::Color(lit)),
            border: iced::Border::default().rounded(DOT / 2.0),
            ..Default::default()
        });
        button(
            row![dot, text(if on { "Chain on" } else { "Chain off" }).size(12.5).font(palette.medium)]
                .spacing(7)
                .align_y(Alignment::Center),
        )
        .padding([6, 12])
        .style(move |_, status| palette.outlined(status))
        .on_press(Message::BypassChain(spot))
        .into()
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
        if !self.project.all_fx_on {
            body = body.push(text("Every plugin in the song is off while you listen dry.").size(12).color(palette.danger));
        }
        match spot {
            Spot::Track(track) => {
                body = body.push(row![self.chain_tabs(track, recording), self.chain_switch(spot)].spacing(6).align_y(Alignment::Center));
            }
            Spot::Master | Spot::Monitor => {
                body = body.push(row![Space::with_width(Length::Fill), self.chain_switch(spot)].align_y(Alignment::Center));
            }
            Spot::Clip(_) => {}
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
        let on = self.chain_is_running(spot);
        let word = if spot == Spot::Monitor { "Mon FX" } else { "FX" };
        let label = if count > 0 { format!("{word} {count}") } else { word.to_string() };
        let wording = text(label).size(11).font(palette.medium).width(Length::Fill).center();
        let pressed = button(if on { wording } else { wording.color(palette.danger) })
            .padding([3, 0])
            .width(Length::Fill)
            .style(move |_, status| palette.outlined(status))
            .on_press(Message::OpenChain(spot));
        mouse_area(pressed).on_right_press(Message::BypassChain(spot)).into()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use loupe_engine::{Command, Outcome, TrackId};

    use super::*;

    fn plugin(name: &str) -> Fx {
        Fx { path: PathBuf::from(loupe_plugins::BUILT_IN), index: 1, name: name.into(), bypassed: false, state: Vec::new(), record: false, mix: 1.0 }
    }

    fn song_with_a_chain() -> (App, TrackId) {
        let mut app = App::bare();
        let Ok(Outcome::Track(track)) = app.project.apply(Command::AddTrack { name: "Vox".into() }) else { panic!("no track") };
        app.project.apply(Command::AddFx { track, fx: plugin("One") }).unwrap();
        app.project.apply(Command::AddFx { track, fx: plugin("Two") }).unwrap();
        app.project.apply(Command::AddMasterFx(plugin("Glue"))).unwrap();
        (app, track)
    }

    fn bypassed_slots(app: &App, track: TrackId) -> Vec<bool> {
        app.project.tracks.iter().find(|found| found.id == track).expect("the track").fx.iter().map(|fx| fx.bypassed).collect()
    }

    #[test]
    fn turning_a_chain_off_leaves_the_slots_underneath_alone() {
        let (mut app, track) = song_with_a_chain();
        let _ = app.update(Message::BypassPlugin(track, 1));
        assert_eq!(bypassed_slots(&app, track), vec![false, true]);
        let _ = app.update(Message::BypassChain(Spot::Track(track)));
        assert!(!app.chain_is_on(Spot::Track(track)), "the chain is still on");
        assert_eq!(bypassed_slots(&app, track), vec![false, true], "turning the chain off changed the slots");
        let _ = app.update(Message::BypassChain(Spot::Track(track)));
        assert!(app.chain_is_on(Spot::Track(track)), "the chain did not come back on");
        assert_eq!(bypassed_slots(&app, track), vec![false, true], "the slots did not come back as they were");
    }

    #[test]
    fn a_chain_turned_off_is_undone_and_redone() {
        let (mut app, track) = song_with_a_chain();
        let _ = app.update(Message::BypassChain(Spot::Track(track)));
        assert!(!app.chain_is_on(Spot::Track(track)));
        let _ = app.update(Message::Undo);
        assert!(app.chain_is_on(Spot::Track(track)), "undo did not put the chain back on");
        let _ = app.update(Message::Redo);
        assert!(!app.chain_is_on(Spot::Track(track)), "redo did not turn the chain off again");
    }

    fn every_switch(app: &App) -> Vec<bool> {
        let mut found = vec![app.project.master_fx_on];
        found.extend(app.project.master_fx.iter().map(|fx| !fx.bypassed));
        for track in &app.project.tracks {
            found.push(track.fx_on);
            found.extend(track.fx.iter().map(|fx| !fx.bypassed));
        }
        found
    }

    #[test]
    fn listening_dry_puts_every_switch_back_exactly_as_it_was() {
        let (mut app, track) = song_with_a_chain();
        let Ok(Outcome::Track(other)) = app.project.apply(Command::AddTrack { name: "Gtr".into() }) else { panic!("no track") };
        app.project.apply(Command::AddFx { track: other, fx: plugin("Three") }).unwrap();
        let _ = app.update(Message::BypassPlugin(track, 1));
        let _ = app.update(Message::BypassChain(Spot::Track(other)));
        let before = every_switch(&app);
        let _ = app.update(Message::BypassAllFx);
        assert!(!app.project.all_fx_on, "listening dry never took hold");
        assert!(!app.chain_is_running(Spot::Track(track)), "a chain is still running");
        assert!(!app.chain_is_running(Spot::Master), "the master chain is still running");
        assert_eq!(every_switch(&app), before, "listening dry moved the switches underneath");
        let _ = app.update(Message::BypassAllFx);
        assert!(app.project.all_fx_on);
        assert_eq!(every_switch(&app), before, "the switches did not come back as they were");
        assert!(app.chain_is_running(Spot::Track(track)), "the first chain did not come back");
        assert!(!app.chain_is_running(Spot::Track(other)), "a chain that was off came back on");
    }

    #[test]
    fn listening_dry_is_undone_and_redone() {
        let (mut app, _) = song_with_a_chain();
        let _ = app.update(Message::BypassAllFx);
        assert!(!app.project.all_fx_on);
        let _ = app.update(Message::Undo);
        assert!(app.project.all_fx_on, "undo did not put the plugins back in circuit");
        let _ = app.update(Message::Redo);
        assert!(!app.project.all_fx_on, "redo did not take them back out");
    }

    #[test]
    fn the_master_chain_goes_off_on_its_own_and_undoes() {
        let (mut app, track) = song_with_a_chain();
        let _ = app.update(Message::BypassChain(Spot::Master));
        assert!(!app.chain_is_on(Spot::Master), "the master chain is still on");
        assert!(app.chain_is_on(Spot::Track(track)), "the track chain went off with the master");
        let _ = app.update(Message::Undo);
        assert!(app.chain_is_on(Spot::Master), "undo did not put the master chain back on");
    }

    fn monitoring_names(app: &App) -> Vec<String> {
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
        assert_eq!(monitoring_names(&app), ["RVox", "Pro-R 2", "Denoiser"], "dragging did not reorder the chain");

        let _ = app.update(Message::StartRenameFx(Spot::Monitor, 0));
        assert_eq!(app.overlay, crate::Overlay::RenameFx(Spot::Monitor, 0));
        let _ = app.update(Message::EntryTyped("Waves RVox".into()));
        let _ = app.update(Message::EntryEntered);
        assert_eq!(monitoring_names(&app)[0], "Waves RVox");

        let _ = app.update(Message::RemoveMonitorPlugin(1));
        assert_eq!(monitoring_names(&app), ["Waves RVox", "Denoiser"]);
        assert!(app.project.master_fx.is_empty(), "the monitoring chain reached into the master chain");
    }

    #[test]
    fn the_monitoring_chain_goes_off_on_its_own_and_follows_listening_dry() {
        let (mut app, track) = song_with_a_chain();
        app.project.apply(Command::AddMonitorFx(plugin("Denoiser"))).unwrap();
        let _ = app.update(Message::BypassChain(Spot::Monitor));
        assert!(!app.chain_is_on(Spot::Monitor), "the monitoring chain is still on");
        assert!(app.chain_is_on(Spot::Master), "the master chain went off with the monitoring chain");
        assert!(app.chain_is_on(Spot::Track(track)), "the track chain went off with the monitoring chain");
        let _ = app.update(Message::Undo);
        assert!(app.chain_is_on(Spot::Monitor), "undo did not put the monitoring chain back on");
        let _ = app.update(Message::BypassAllFx);
        assert!(!app.chain_is_running(Spot::Monitor), "listening dry left the monitoring chain running");
        assert!(app.chain_is_on(Spot::Monitor), "listening dry moved the monitoring switch underneath");
    }
}
