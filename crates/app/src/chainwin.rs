use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{button, column, container, mouse_area, row, scrollable, slider, text, Space};
use iced::{window, Alignment, Element, Length, Size, Task};

use loupe_engine::Fx;

use crate::stockwin::Spot;
use crate::{icon, App, Message};

pub const FX_WINDOW_WIDTH: f32 = 560.0;
const NARROWEST_FX_WINDOW: f32 = 440.0;
const SHORTEST_FX_WINDOW: f32 = 280.0;
const EDGE: f32 = 16.0;
const GAP: f32 = 12.0;
const HEADER: f32 = 30.0;
const DRY_NOTE: f32 = 28.0;
const SLOT_HEIGHT: f32 = 32.0;
const SLOT_GAP: f32 = 4.0;
const LIST_PADDING: f32 = 8.0;
const EMPTY_LIST: f32 = 56.0;
const ADD_BUTTON: f32 = 32.0;
const CHAINS: f32 = 84.0;
pub const SLOTS_ID: &str = "fx-window-slots";
pub const PINNED_ID: &str = "fx-window-pinned";
const MIX_WIDTH: f32 = 120.0;
const READING_WIDTH: f32 = 44.0;
const DOT: f32 = 11.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FxWindow {
    pub id: window::Id,
    pub spot: Spot,
    pub home: window::Id,
}

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

    fn shown_slots(&self, spot: Spot) -> Vec<(usize, &Fx)> {
        let recording = matches!(spot, Spot::Track(track) if self.rec_shown.contains(&track));
        self.chain_of(spot)
            .unwrap_or_default()
            .iter()
            .enumerate()
            .filter(|(_, fx)| !matches!(spot, Spot::Track(_)) || fx.record == recording)
            .collect()
    }

    pub(crate) fn fx_window_view(&self, spot: Spot) -> Element<'_, Message> {
        let palette = self.palette;
        let shown = self.shown_slots(spot);
        let mut rows = column![].spacing(SLOT_GAP);
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
        let slots = container(
            scrollable(rows).width(Length::Fill).height(Length::Fill).direction(Direction::Vertical(Scrollbar::new())),
        )
        .id(container::Id::new(SLOTS_ID))
        .padding(LIST_PADDING)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| palette.readout());
        let add = button(row![icon("plus", 13.0), text("Add a plugin").size(13).font(palette.medium)].spacing(7).align_y(Alignment::Center))
            .padding([7, 14])
            .style(move |_, status| palette.solid(status))
            .on_press(self.add_to(spot));
        let chains = self.chain_block(spot).unwrap_or_else(|| Space::new(0, 0).into());
        let pinned = container(column![row![add, Space::with_width(Length::Fill)].align_y(Alignment::Center), chains].spacing(GAP))
            .id(container::Id::new(PINNED_ID))
            .width(Length::Fill);
        let body = column![self.chain_header(spot), slots, pinned].spacing(GAP);
        container(body)
            .padding(EDGE)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style { background: Some(palette.panel.into()), ..Default::default() })
            .into()
    }

    pub(crate) fn closing_window_view(&self) -> Element<'_, Message> {
        let palette = self.palette;
        container(Space::new(Length::Fill, Length::Fill))
            .style(move |_| container::Style { background: Some(palette.panel.into()), ..Default::default() })
            .into()
    }

    fn chain_header(&self, spot: Spot) -> Element<'_, Message> {
        let palette = self.palette;
        let top: Element<'_, Message> = match spot {
            Spot::Track(track) => {
                let recording = self.rec_shown.contains(&track);
                row![self.chain_tabs(track, recording), self.chain_switch(spot)].spacing(6).align_y(Alignment::Center).into()
            }
            Spot::Master | Spot::Monitor => row![Space::with_width(Length::Fill), self.chain_switch(spot)].align_y(Alignment::Center).into(),
            Spot::Clip(_) => row![Space::with_width(Length::Fill)].into(),
        };
        let dry: Element<'_, Message> = match self.project.all_fx_on {
            true => Space::new(0, 0).into(),
            false => container(text("Every plugin in the song is off while you listen dry.").size(12).color(palette.danger))
                .padding(iced::Padding { top: GAP, ..iced::Padding::ZERO })
                .into(),
        };
        column![top, dry].into()
    }

    fn fx_window_size(&self, spot: Spot) -> Size {
        let slots = self.shown_slots(spot).len() as f32;
        let list = match slots > 0.0 {
            true => slots * SLOT_HEIGHT + (slots - 1.0) * SLOT_GAP,
            false => EMPTY_LIST,
        } + LIST_PADDING * 2.0;
        let dry = if self.project.all_fx_on { 0.0 } else { DRY_NOTE };
        let wanted = EDGE * 2.0 + HEADER + dry + GAP + list + GAP + ADD_BUTTON + GAP + CHAINS;
        Size::new(FX_WINDOW_WIDTH, wanted.clamp(SHORTEST_FX_WINDOW, self.tallest_fx_window()))
    }

    fn tallest_fx_window(&self) -> f32 {
        let mixer = self.mixer_window.map_or(0.0, |_| self.mixer_size.height);
        self.window.height.max(mixer).max(SHORTEST_FX_WINDOW)
    }

    pub(crate) fn fx_window_at(&self, window: window::Id) -> Option<&FxWindow> {
        self.fx_windows.iter().find(|fx| fx.id == window)
    }

    pub(crate) fn open_fx_window(&mut self, spot: Spot) -> Task<Message> {
        if self.chain_of(spot).is_none() {
            return Task::none();
        }
        if let Some(open) = self.fx_windows.iter().find(|fx| fx.spot == spot) {
            return window::gain_focus(open.id);
        }
        let home = self.opens_in_front();
        let (id, opening) = window::open(window::Settings {
            size: self.fx_window_size(spot),
            min_size: Some(Size::new(NARROWEST_FX_WINDOW, SHORTEST_FX_WINDOW)),
            icon: window::icon::from_file_data(include_bytes!("../assets/icon.png"), None).ok(),
            exit_on_close_request: false,
            ..window::Settings::default()
        });
        self.fx_windows.push(FxWindow { id, spot, home });
        opening.map(Message::WindowFocused)
    }

    pub(crate) fn settle_fx_windows(&mut self) -> Task<Message> {
        let lost: Vec<window::Id> = self.fx_windows.iter().filter(|fx| self.chain_of(fx.spot).is_none()).map(|fx| fx.id).collect();
        self.fx_windows.retain(|fx| !lost.contains(&fx.id));
        self.fx_windows_closing.extend(lost);
        if self.fx_windows_closing.contains(&self.focused) {
            self.focused = self.main_window;
        }
        Task::batch(self.fx_windows_closing.drain(..).map(window::close))
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

    fn vox_with(slots: usize) -> (App, TrackId) {
        let mut app = App::bare();
        let Ok(Outcome::Track(track)) = app.project.apply(Command::AddTrack { name: "Vox".into() }) else { panic!("no track") };
        for slot in 0..slots {
            app.project.apply(Command::AddFx { track, fx: plugin(&format!("Plugin {slot}")) }).unwrap();
        }
        (app, track)
    }

    fn window_for(app: &App, spot: Spot) -> window::Id {
        app.fx_windows.iter().find(|fx| fx.spot == spot).map(|fx| fx.id).expect("no window for that chain")
    }

    #[test]
    fn opening_the_same_chain_twice_gives_one_window_and_other_chains_get_their_own() {
        let (mut app, track) = vox_with(2);
        let _ = app.update(Message::OpenChain(Spot::Track(track)));
        let first = window_for(&app, Spot::Track(track));
        let _ = app.update(Message::OpenChain(Spot::Track(track)));
        assert_eq!(app.fx_windows.len(), 1, "the same chain opened a second window");
        assert_eq!(window_for(&app, Spot::Track(track)), first, "the chain moved to a new window");
        let _ = app.update(Message::OpenChain(Spot::Master));
        let _ = app.update(Message::OpenChain(Spot::Monitor));
        let _ = app.update(Message::OpenChain(Spot::Master));
        assert_eq!(app.fx_windows.len(), 3, "different chains did not get a window each");
        assert_eq!(app.overlay, crate::Overlay::None, "opening a chain still drew something over the song");
    }

    #[test]
    fn the_window_is_titled_after_its_chain() {
        let (mut app, track) = vox_with(1);
        let _ = app.update(Message::OpenChain(Spot::Track(track)));
        let _ = app.update(Message::OpenChain(Spot::Master));
        assert_eq!(app.title_of(window_for(&app, Spot::Track(track))), "FX — Vox");
        assert_eq!(app.title_of(window_for(&app, Spot::Master)), "FX — Master");
    }

    #[test]
    fn deleting_a_track_closes_its_fx_window_and_leaves_the_others() {
        let (mut app, track) = vox_with(2);
        let _ = app.update(Message::OpenChain(Spot::Track(track)));
        let _ = app.update(Message::OpenChain(Spot::Master));
        let gone = window_for(&app, Spot::Track(track));
        let _ = app.update(Message::WindowFocused(gone));
        let _ = app.update(Message::RemoveTrack(track));
        assert!(app.fx_window_at(gone).is_none(), "the deleted track kept its FX window");
        assert!(app.fx_windows_closing.is_empty(), "the window was never asked to close");
        assert_eq!(app.fx_windows.len(), 1, "the master window went with the track");
        assert_ne!(app.focused, gone, "the keys stayed with a window that is closing");
        assert_eq!(app.title_of(window_for(&app, Spot::Master)), "FX — Master");
    }

    #[test]
    fn a_new_song_closes_every_fx_window() {
        let (mut app, track) = vox_with(1);
        let _ = app.update(Message::OpenChain(Spot::Track(track)));
        let _ = app.update(Message::OpenChain(Spot::Master));
        app.replace_project(loupe_engine::Project::new(app.project.rate), std::collections::HashMap::new());
        let _ = app.update(Message::Tick);
        assert!(app.fx_windows.is_empty(), "FX windows outlived their song");
        assert!(app.fx_windows_closing.is_empty(), "the windows were never asked to close");
    }

    #[test]
    fn closing_the_window_forgets_it_so_the_chain_opens_again() {
        let (mut app, track) = vox_with(1);
        let _ = app.update(Message::OpenChain(Spot::Track(track)));
        let first = window_for(&app, Spot::Track(track));
        let _ = app.update(Message::WindowClosed(first));
        assert!(app.fx_windows.is_empty(), "the closed window was kept");
        assert!(!app.going, "closing an FX window started to quit Loupe");
        let _ = app.update(Message::OpenChain(Spot::Track(track)));
        assert_ne!(window_for(&app, Spot::Track(track)), first, "the chain did not open a fresh window");
    }

    #[test]
    fn sheets_opened_from_an_fx_window_land_in_the_window_it_came_from() {
        let (mut app, track) = vox_with(1);
        let mixer = window::Id::unique();
        app.mixer_open = true;
        app.mixer_window = Some(mixer);
        app.mixer_size = crate::mixer::MIXER_WINDOW;
        app.focused = mixer;
        let _ = app.update(Message::OpenChain(Spot::Track(track)));
        let fx = window_for(&app, Spot::Track(track));
        let _ = app.update(Message::WindowFocused(fx));
        let _ = app.update(Message::OpenPlugins(track));
        assert_eq!(app.overlay, crate::Overlay::Plugins(track));
        assert!(app.overlay_is_in(mixer), "the plugin picker did not open in the mixer the chain came from");
        let _ = app.update(Message::CloseOverlay);
        let _ = app.update(Message::WindowFocused(app.main_window));
        let _ = app.update(Message::OpenChain(Spot::Master));
        let master = window_for(&app, Spot::Master);
        let _ = app.update(Message::WindowFocused(master));
        let _ = app.update(Message::OpenMasterPlugins);
        assert!(app.overlay_is_in(app.main_window), "the master picker did not open in the main window it came from");
    }

    #[test]
    fn each_chain_keeps_its_own_name_being_typed() {
        let (mut app, track) = vox_with(1);
        let _ = app.update(Message::ChainName(Spot::Track(track), "Vox chain".into()));
        let _ = app.update(Message::ChainName(Spot::Master, "Bus".into()));
        assert_eq!(app.chain_names.get(&Spot::Track(track)).map(String::as_str), Some("Vox chain"));
        assert_eq!(app.chain_names.get(&Spot::Master).map(String::as_str), Some("Bus"));
    }

    #[derive(Default)]
    struct Survey {
        scroll_next: bool,
        scrolling: usize,
        found: Vec<(String, iced::Rectangle, bool)>,
        scrolls: Vec<(iced::Rectangle, iced::Rectangle)>,
    }

    impl iced::advanced::widget::Operation for Survey {
        fn container(
            &mut self,
            id: Option<&iced::advanced::widget::Id>,
            bounds: iced::Rectangle,
            operate_on_children: &mut dyn FnMut(&mut dyn iced::advanced::widget::Operation<()>),
        ) {
            let scroll = std::mem::take(&mut self.scroll_next);
            for name in [SLOTS_ID, PINNED_ID] {
                if id == Some(&iced::advanced::widget::Id::new(name)) {
                    self.found.push((name.to_string(), bounds, self.scrolling > 0));
                }
            }
            self.scrolling += usize::from(scroll);
            operate_on_children(self);
            self.scrolling -= usize::from(scroll);
        }

        fn scrollable(
            &mut self,
            _state: &mut dyn iced::advanced::widget::operation::Scrollable,
            _id: Option<&iced::advanced::widget::Id>,
            bounds: iced::Rectangle,
            content_bounds: iced::Rectangle,
            _translation: iced::Vector,
        ) {
            self.scroll_next = true;
            self.scrolls.push((bounds, content_bounds));
        }
    }

    fn survey(app: &App, spot: Spot, size: Size) -> Survey {
        let mut renderer = iced::Renderer::Secondary(iced_tiny_skia::Renderer::new(iced::Font::DEFAULT, iced::Pixels(16.0)));
        let mut ui = iced_runtime::UserInterface::build(app.fx_window_view(spot), size, iced_runtime::user_interface::Cache::default(), &mut renderer);
        let mut seen = Survey::default();
        ui.operate(&renderer, &mut seen);
        seen
    }

    fn part<'a>(seen: &'a Survey, name: &str) -> &'a (String, iced::Rectangle, bool) {
        seen.found.iter().find(|(found, _, _)| found == name).unwrap_or_else(|| panic!("{name} is missing from the window"))
    }

    #[test]
    fn a_long_chain_scrolls_while_add_and_chains_stay_pinned_in_view() {
        let (app, track) = vox_with(40);
        let size = Size::new(FX_WINDOW_WIDTH, 360.0);
        let seen = survey(&app, Spot::Track(track), size);
        let (_, slots, slots_scroll) = part(&seen, SLOTS_ID);
        let (_, pinned, pinned_scroll) = part(&seen, PINNED_ID);
        assert!(!slots_scroll, "the slot list sits inside another scroller");
        assert!(!pinned_scroll, "Add a plugin and Chains are inside the scrolling list");
        assert!(pinned.y >= slots.y + slots.height, "Add a plugin and Chains are not under the list");
        assert!(pinned.y + pinned.height <= size.height, "Add a plugin and Chains are cut off at the bottom");
        assert!(pinned.height > 0.0, "Add a plugin and Chains have no room");
        let (viewport, content) = seen.scrolls.first().copied().expect("the list does not scroll");
        assert!(content.height > viewport.height, "forty plugins fit without scrolling, so this proves nothing");
        assert!(viewport.y + viewport.height <= pinned.y, "the list runs under the pinned part");
    }

    #[test]
    fn a_short_chain_opens_tall_enough_to_show_everything_without_scrolling() {
        for slots in [0, 1, 4, 8] {
            let (app, track) = vox_with(slots);
            let size = app.fx_window_size(Spot::Track(track));
            let seen = survey(&app, Spot::Track(track), size);
            let (_, pinned, _) = part(&seen, PINNED_ID);
            let (viewport, content) = seen.scrolls.first().copied().expect("no list");
            assert!(content.height <= viewport.height + 0.5, "{slots} plugins did not fit: {} of {}", content.height, viewport.height);
            assert!(pinned.y + pinned.height <= size.height, "with {slots} plugins the bottom was cut off");
        }
    }

    #[test]
    fn a_huge_chain_opens_no_taller_than_the_tallest_loupe_window() {
        let (app, track) = vox_with(80);
        let size = app.fx_window_size(Spot::Track(track));
        assert!(size.height <= app.window.height, "the window opened taller than the screen Loupe is on");
        assert!(size.height >= SHORTEST_FX_WINDOW);
        let seen = survey(&app, Spot::Track(track), size);
        let (_, pinned, _) = part(&seen, PINNED_ID);
        assert!(pinned.y + pinned.height <= size.height, "the bottom was cut off");
    }
}
