use iced::widget::{button, checkbox, column, container, mouse_area, row, scrollable, slider, text, text_input};
use iced::{Alignment, Element, Length};
use loupe_engine::TrackId;

use crate::clip_window::TrackChoice;
use crate::mixer;
use crate::theme;
use crate::{rule, App, Message};

pub const SEND_ENTRY_ID: &str = "send-entry";

pub fn loudest_send() -> f32 {
    mixer::gain_from_db(mixer::LOUDEST_DB)
}

pub fn gain_from(typed: &str) -> Option<f32> {
    let typed = typed.trim();
    if let Some(percent) = typed.strip_suffix('%') {
        let percent = percent.trim().parse::<f32>().ok().filter(|percent| percent.is_finite())?;
        return Some((percent / 100.0).clamp(0.0, loudest_send()));
    }
    let db = mixer::db_from_typed(typed)?;
    Some(mixer::gain_from_db(db.clamp(mixer::SILENT_DB, mixer::LOUDEST_DB)))
}

impl App {
    fn send_level(&self, from: TrackId, to: TrackId, gain: f32) -> Element<'_, Message> {
        let palette = self.palette;
        let shown: Element<'_, Message> = if self.editing_send == Some((from, to)) {
            text_input("", &self.entry)
                .id(SEND_ENTRY_ID)
                .on_input(Message::EntryTyped)
                .on_submit(Message::SendLevelEntered)
                .font(palette.mono)
                .size(11.5)
                .padding([2, 4])
                .width(Length::Fill)
                .style(move |_, status| palette.field(status))
                .into()
        } else {
            mouse_area(text(mixer::level_text(gain)).size(11.5).font(palette.mono).color(palette.text_dim))
                .on_press(Message::SendLevelPressed { from, to })
                .interaction(iced::mouse::Interaction::Text)
                .into()
        };
        container(shown).width(SEND_LEVEL).center_y(22).into()
    }

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
            theme::picker(palette, homes, Some(home), move |choice: TrackChoice| Message::SetTrackParent {
                track: from,
                parent: (choice.id != TrackId(0)).then_some(choice.id),
            }),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        let mut rows = column![].spacing(10);
        for send in &track.sends {
            let Some(target) = self.project.tracks.iter().find(|t| t.id == send.to) else {
                continue;
            };
            let to = send.to;
            let db = mixer::db_from_gain(send.gain);
            rows = rows.push(
                row![
                    text(target.name.clone()).size(13).width(SEND_NAME).wrapping(iced::widget::text::Wrapping::None),
                    self.send_level(from, to, send.gain),
                    slider(mixer::SILENT_DB..=mixer::LOUDEST_DB, db, move |db| Message::SendGain {
                        from,
                        to,
                        gain: mixer::gain_from_db(db),
                    })
                    .step(0.1)
                    .default(0.0)
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
            theme::picker(palette, choices, None::<TrackChoice>, move |choice| Message::AddSend { from, to: choice.id })
                .placeholder("Send to…")
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
        .width(720)
        .style(move |_| palette.sheet())
        .into()
    }
}

const CELL: f32 = 30.0;
const NAME_COL: f32 = 130.0;
const SEND_NAME: f32 = 150.0;
const SEND_LEVEL: f32 = 56.0;

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

#[cfg(test)]
mod tests {
    use super::*;
    use loupe_engine::{Command, Outcome, Project};

    fn close(got: Option<f32>, want: f32) {
        let got = got.expect("a level");
        assert!((got - want).abs() < 1e-3, "got {got}, wanted {want}");
    }

    #[test]
    fn a_bare_number_reads_as_decibels() {
        close(gain_from("0"), 1.0);
        close(gain_from("-6"), 0.5012);
        close(gain_from("-19.3"), 0.1086);
        close(gain_from("+6"), 1.9953);
        close(gain_from("  -12.0  "), 0.2512);
    }

    #[test]
    fn a_db_suffix_reads_as_decibels_too() {
        close(gain_from("0db"), 1.0);
        close(gain_from("-6 dB"), 0.5012);
        close(gain_from("-19.3 DB"), 0.1086);
    }

    #[test]
    fn a_percent_sign_still_reads_as_percent() {
        assert_eq!(gain_from("35%"), Some(0.35));
        assert_eq!(gain_from("  36 % "), Some(0.36));
        assert_eq!(gain_from("100%"), Some(1.0));
        assert_eq!(gain_from("0%"), Some(0.0));
    }

    #[test]
    fn the_bottom_of_the_range_is_silence() {
        assert_eq!(gain_from("-inf"), Some(0.0));
        assert_eq!(gain_from("-inf dB"), Some(0.0));
        assert_eq!(gain_from("-60"), Some(0.0));
        assert_eq!(gain_from("-90 dB"), Some(0.0));
    }

    #[test]
    fn gain_and_db_make_the_round_trip() {
        for gain in [1.0f32, 0.5012, 0.1086, 1.9953, 0.01] {
            close(Some(mixer::gain_from_db(mixer::db_from_gain(gain))), gain);
        }
        assert_eq!(mixer::db_from_gain(0.0), mixer::SILENT_DB);
        assert_eq!(mixer::gain_from_db(mixer::db_from_gain(0.0)), 0.0);
        assert_eq!(mixer::level_text(0.0), "-inf");
        assert_eq!(mixer::level_text(1.0), "+0.0");
        assert_eq!(mixer::level_text(0.5012), "-6.0");
        assert_eq!(mixer::level_text(0.1086), "-19.3");
    }

    #[test]
    fn values_past_the_ends_come_back_clamped() {
        close(gain_from("24 dB"), loudest_send());
        close(gain_from("400%"), loudest_send());
        assert_eq!(gain_from("-20%"), Some(0.0));
        assert_eq!(gain_from("-400 dB"), Some(0.0));
    }

    #[test]
    fn rubbish_is_turned_away() {
        assert_eq!(gain_from(""), None);
        assert_eq!(gain_from("loud"), None);
        assert_eq!(gain_from("%"), None);
        assert_eq!(gain_from("3 5"), None);
        assert_eq!(gain_from("nan"), None);
        assert_eq!(gain_from("inf"), None);
        assert_eq!(gain_from("db"), None);
    }

    #[test]
    fn a_typed_level_sets_the_send_gain() {
        let mut project = Project::new(48_000);
        project.apply(Command::AddTrack { name: "Vox".into() }).unwrap();
        project.apply(Command::AddTrack { name: "Verb".into() }).unwrap();
        let from = project.tracks[0].id;
        let to = project.tracks[1].id;
        project.apply(Command::AddSend { from, to }).unwrap();
        let gain = gain_from("-6.0").unwrap();
        assert_eq!(project.apply(Command::SetSendGain { from, to, gain }).unwrap(), Outcome::Done);
        close(Some(project.tracks[0].sends[0].gain), 0.5012);
        let gain = gain_from("0.0").unwrap();
        project.apply(Command::SetSendGain { from, to, gain }).unwrap();
        assert_eq!(project.tracks[0].sends[0].gain, 1.0);
        let gain = gain_from("-inf").unwrap();
        project.apply(Command::SetSendGain { from, to, gain }).unwrap();
        assert_eq!(project.tracks[0].sends[0].gain, 0.0);
        assert_eq!(mixer::level_text(project.tracks[0].sends[0].gain), "-inf");
    }
}
