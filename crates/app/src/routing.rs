use iced::widget::{button, checkbox, column, container, pick_list, row, scrollable, slider, text};
use iced::{Alignment, Element, Length};
use loupe_engine::TrackId;

use crate::clip_window::TrackChoice;
use crate::{rule, App, Message};

impl App {
    pub(crate) fn routing_sheet(&self, from: TrackId) -> Element<'_, Message> {
        let palette = self.palette;
        let Some(track) = self.project.tracks.iter().find(|t| t.id == from) else {
            return text("This track is gone.").size(13).into();
        };
        let heading = text(format!("Routing for {}", track.name)).size(14).font(palette.semibold);
        let master = TrackChoice { id: TrackId(0), name: "Master".into() };
        let mut homes = vec![master.clone()];
        for other in &self.project.tracks {
            if other.id != from && !self.project.descends_from(other.id, from) {
                homes.push(TrackChoice { id: other.id, name: other.name.clone() });
            }
        }
        let home = match track.parent.and_then(|id| self.project.tracks.iter().find(|t| t.id == id)) {
            Some(parent) => TrackChoice { id: parent.id, name: parent.name.clone() },
            None => master,
        };
        let goes_to = row![
            text("Goes into").size(12.5).color(palette.text_dim),
            pick_list(homes, Some(home), move |choice: TrackChoice| Message::SetTrackParent {
                track: from,
                parent: (choice.id != TrackId(0)).then_some(choice.id),
            })
            .text_size(13)
            .padding([5, 10]),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        let mut rows = column![].spacing(10);
        for send in &track.sends {
            let Some(target) = self.project.tracks.iter().find(|t| t.id == send.to) else {
                continue;
            };
            let to = send.to;
            let percent = send.gain * 100.0;
            rows = rows.push(
                row![
                    text(target.name.clone()).size(13).width(Length::Fill),
                    text(format!("{percent:.0}%")).size(11.5).font(palette.mono).color(palette.text_dim),
                    slider(0.0..=125.0, percent, move |percent| Message::SendGain { from, to, gain: percent / 100.0 })
                        .step(1.0)
                        .width(120)
                        .style(move |_, status| palette.slider(status)),
                    checkbox("Pre", send.pre_fader)
                        .size(14)
                        .text_size(12)
                        .on_toggle(move |pre_fader| Message::SendPreFader { from, to, pre_fader }),
                    checkbox("Sidechain", send.sidechain)
                        .size(14)
                        .text_size(12)
                        .on_toggle(move |sidechain| Message::SendSidechain { from, to, sidechain }),
                    {
                        let target = loupe_engine::Target::SendGain { from, to };
                        let on = self.project.envelope(target).is_some();
                        button(text(if on { "Stop automating" } else { "Automate" }).size(12))
                            .padding([3, 8])
                            .style(move |_, status| palette.toggled(on, status))
                            .on_press(if on { Message::RemoveEnvelope(target) } else { Message::AddEnvelope(target) })
                    },
                    button(text("Remove").size(12))
                        .padding([3, 8])
                        .style(move |_, status| palette.outlined(status))
                        .on_press(Message::RemoveSend { from, to }),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            );
        }
        if track.sends.is_empty() {
            rows = rows.push(text("No sends yet.").size(12.5).color(palette.text_dim));
        }
        let mut coming = column![].spacing(6);
        let mut receives = 0;
        for other in &self.project.tracks {
            for send in other.sends.iter().filter(|send| send.to == from) {
                receives += 1;
                let pre = if send.pre_fader { " pre fader" } else { "" };
                coming = coming.push(
                    text(format!("{} at {:.0}%{pre}", other.name, send.gain * 100.0))
                        .size(12.5)
                        .color(palette.text_dim),
                );
            }
        }
        for child in self.project.tracks.iter().filter(|t| t.parent == Some(from)) {
            receives += 1;
            coming = coming.push(text(format!("{} (inside this folder)", child.name)).size(12.5).color(palette.text_dim));
        }
        if receives == 0 {
            coming = coming.push(text("Nothing comes in.").size(12.5).color(palette.text_dim));
        }
        let choices: Vec<TrackChoice> = self
            .project
            .tracks
            .iter()
            .filter(|other| other.id != from && !track.sends.iter().any(|send| send.to == other.id))
            .filter(|other| !self.project.descends_from(other.id, from))
            .map(|other| TrackChoice { id: other.id, name: other.name.clone() })
            .collect();
        let adder: Element<'_, Message> = if choices.is_empty() {
            text("Nothing left to send to.").size(12.5).color(palette.text_dim).into()
        } else {
            pick_list(choices, None::<TrackChoice>, move |choice| Message::AddSend { from, to: choice.id })
                .placeholder("Send to…")
                .text_size(13)
                .padding([6, 10])
                .into()
        };
        container(
            column![
                heading,
                goes_to,
                rule(palette),
                text("Sends out").size(12).font(palette.semibold),
                scrollable(rows).height(Length::Shrink),
                adder,
                rule(palette),
                text("Comes in").size(12).font(palette.semibold),
                coming,
            ]
            .spacing(12),
        )
        .padding(16)
        .width(460)
        .style(move |_| palette.sheet())
        .into()
    }
}

const CELL: f32 = 30.0;
const NAME_COL: f32 = 130.0;

impl App {
    pub(crate) fn matrix_sheet(&self) -> Element<'_, Message> {
        let palette = self.palette;
        if self.project.tracks.is_empty() {
            return container(text("Add a track first.").size(13).color(palette.text_dim))
                .padding(16)
                .style(move |_| palette.sheet())
                .into();
        }
        let short = |name: &str| name.chars().take(10).collect::<String>();
        let mut head = row![container(text("Sends down, into across").size(11).color(palette.text_faint)).width(NAME_COL)]
            .spacing(2)
            .align_y(Alignment::Center);
        for target in &self.project.tracks {
            head = head.push(
                container(text(short(&target.name)).size(10).color(palette.text_dim))
                    .width(CELL)
                    .center_x(CELL)
                    .clip(true),
            );
        }
        let mut grid = column![head].spacing(2);
        for source in &self.project.tracks {
            let from = source.id;
            let depth = self.project.depth_of(from).min(4) as f32;
            let mut line = row![container(
                row![
                    iced::widget::Space::new(depth * 10.0, 0),
                    text(short(&source.name)).size(12).color(palette.text),
                ]
                .align_y(Alignment::Center)
            )
            .width(NAME_COL)
            .clip(true)]
            .spacing(2)
            .align_y(Alignment::Center);
            for target in &self.project.tracks {
                let to = target.id;
                let sending = source.sends.iter().any(|send| send.to == to);
                let blocked = from == to || self.project.descends_from(to, from);
                let label = if from == to {
                    "·"
                } else if sending {
                    "●"
                } else {
                    ""
                };
                let press = if blocked {
                    None
                } else if sending {
                    Some(Message::RemoveSend { from, to })
                } else {
                    Some(Message::AddSend { from, to })
                };
                line = line.push(
                    button(container(text(label).size(13).color(palette.accent)).center_x(Length::Fill))
                        .width(CELL)
                        .height(CELL)
                        .padding(0)
                        .style(move |_, status| palette.outlined(status))
                        .on_press_maybe(press),
                );
            }
            grid = grid.push(line);
        }
        container(
            column![
                text("Routing matrix").size(14).font(palette.semibold),
                text("A dot is a send. Click a square to make or break one.").size(12).color(palette.text_dim),
                rule(palette),
                scrollable(grid).height(Length::Shrink),
            ]
            .spacing(12),
        )
        .padding(16)
        .style(move |_| palette.sheet())
        .into()
    }
}
