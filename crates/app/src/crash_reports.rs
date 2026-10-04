use iced::futures::channel::oneshot;
use iced::widget::{button, column, container, opaque, row, scrollable, text, text_input, Space};
use iced::{Element, Length, Task};
use loupe_engine::{Command, Fx};

use crate::crash::{self, Report, Waiting};
use crate::racks::{Fell, Spot};
use crate::{App, Message, Overlay};

const PREVIEW_HEIGHT: f32 = 220.0;
const SHEET_WIDTH: f32 = 600.0;
const QUESTION_ABOUT_LOUPE: &str = "Loupe closed unexpectedly. Send a crash report?";

pub fn question(report: &Report) -> String {
    if report.is_plugin() {
        format!("{} crashed. Send a crash report?", report.plugin)
    } else {
        QUESTION_ABOUT_LOUPE.to_string()
    }
}

impl App {
    pub(crate) fn note_plugins_for_crashes(&self) {
        let on_tracks = self.project.tracks.iter().flat_map(|track| track.fx.iter());
        let on_clips = self.project.tracks.iter().flat_map(|track| track.clips.iter()).flat_map(|clip| clip.fx.iter());
        crash::note_plugins(on_tracks.chain(on_clips).chain(self.project.master_fx.iter()).map(|fx| fx.name.clone()));
    }

    pub(crate) fn note_audio_for_crashes(&self) {
        match self.engine.running() {
            Some(running) => crash::note_audio(&running.driver, &running.output),
            None => crash::note_audio(
                self.audio.driver.as_deref().unwrap_or("default"),
                self.audio.output.as_deref().unwrap_or("default"),
            ),
        }
    }

    fn fx_at(&self, spot: Spot) -> Option<&Fx> {
        match spot {
            Spot::Track(track, slot) => self.project.track(track)?.fx.get(slot),
            Spot::Clip(clip, slot) => {
                self.project.tracks.iter().flat_map(|track| track.clips.iter()).find(|found| found.id == clip)?.fx.get(slot)
            }
            Spot::Master(slot) => self.project.master_fx.get(slot),
        }
    }

    pub(crate) fn take_falls(&mut self) {
        let fresh: Vec<Fell> = match self.falls.try_lock() {
            Ok(mut held) if !held.is_empty() => held.drain(..).collect(),
            _ => Vec::new(),
        };
        for fell in fresh {
            if !self.fell_named.insert(fell.name.clone()) {
                continue;
            }
            let report = Report::plugin_fell(&fell.name, &fell.why);
            if let Some(file) = crash::keep(&report) {
                self.crash_reports.push(Waiting { file, report });
            }
            if self.fx_at(fell.spot).is_some_and(|fx| fx.name == fell.name && !fx.bypassed) {
                self.fell.push(fell);
            }
            self.crash_sheet_due = true;
        }
        if self.crash_sheet_due && self.overlay == Overlay::None {
            self.crash_sheet_due = false;
            self.next_crash_sheet();
        }
    }

    fn next_crash_sheet(&mut self) {
        self.overlay = if !self.fell.is_empty() {
            Overlay::PluginFell
        } else if !self.crash_reports.is_empty() {
            Overlay::CrashReport
        } else {
            Overlay::None
        };
    }

    pub(crate) fn settle_fallen(&mut self, turn_off: bool) {
        if self.fell.is_empty() {
            self.next_crash_sheet();
            return;
        }
        let fell = self.fell.remove(0);
        let still_there = self.fx_at(fell.spot).is_some_and(|fx| fx.name == fell.name && !fx.bypassed);
        if turn_off && still_there {
            let command = match fell.spot {
                Spot::Track(track, slot) => Command::BypassFx { track, slot, bypassed: true },
                Spot::Clip(clip, slot) => Command::BypassClipFx { clip, slot, bypassed: true },
                Spot::Master(slot) => Command::BypassMasterFx { slot, bypassed: true },
            };
            self.edit(None, command);
        }
        self.next_crash_sheet();
    }

    pub(crate) fn send_crash_report(&mut self) -> Task<Message> {
        if self.crash_sending {
            return Task::none();
        }
        let Some(waiting) = self.crash_reports.first() else {
            self.next_crash_sheet();
            return Task::none();
        };
        let body = waiting.report.body(&self.crash_note);
        self.crash_sending = true;
        let (done, sent) = oneshot::channel();
        std::thread::spawn(move || {
            let _ = done.send(crash::send(body));
        });
        Task::perform(
            async move { sent.await.unwrap_or_else(|_| Err("sending stopped unexpectedly".to_string())) },
            Message::CrashReportSent,
        )
    }

    pub(crate) fn crash_report_sent(&mut self, result: Result<(), String>) {
        self.crash_sending = false;
        if self.crash_reports.is_empty() {
            return;
        }
        let waiting = self.crash_reports.remove(0);
        match result {
            Ok(()) => {
                crash::forget(&waiting);
                self.notice = Some("Crash report sent. Thank you.".into());
            }
            Err(why) => {
                self.notice = Some(format!("Could not send the crash report: {why}. Loupe will ask again next time it starts."));
            }
        }
        self.crash_note.clear();
        if matches!(self.overlay, Overlay::CrashReport | Overlay::None) {
            self.next_crash_sheet();
        }
    }

    pub(crate) fn skip_crash_report(&mut self) {
        if self.crash_sending {
            return;
        }
        if !self.crash_reports.is_empty() {
            let waiting = self.crash_reports.remove(0);
            crash::forget(&waiting);
        }
        self.crash_note.clear();
        self.next_crash_sheet();
    }

    pub(crate) fn crash_report_layer(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let Some(waiting) = self.crash_reports.first() else {
            return Space::new(0, 0).into();
        };
        let report = &waiting.report;
        let shown = container(
            scrollable(text(report.shown(&self.crash_note)).size(11.5).font(palette.mono).width(Length::Fill)).height(PREVIEW_HEIGHT),
        )
        .padding(10)
        .width(Length::Fill)
        .style(move |_| palette.strip());
        let note = text_input("Optional", &self.crash_note)
            .on_input(Message::CrashNoteTyped)
            .size(13)
            .padding([6, 8])
            .style(move |_, status| palette.field(status));
        let send_label = if self.crash_sending { "Sending…" } else { "Send" };
        let body = column![
            text(question(report)).size(14),
            text("This is everything the report holds. Nothing is sent unless you press Send.").size(12.5).color(palette.text_dim),
            shown,
            text("What were you doing?").size(12.5).color(palette.text_dim),
            note,
            row![
                Space::with_width(Length::Fill),
                button(text("Don't send").size(13).font(palette.medium))
                    .padding([7, 16])
                    .style(move |_, status| palette.outlined(status))
                    .on_press_maybe((!self.crash_sending).then_some(Message::SkipCrashReport)),
                button(text(send_label).size(13).font(palette.medium))
                    .padding([7, 16])
                    .style(move |_, status| palette.solid(status))
                    .on_press_maybe((!self.crash_sending).then_some(Message::SendCrashReport)),
            ]
            .spacing(10),
        ]
        .spacing(10);
        let sheet = self.window("Crash report".to_string(), body.into(), SHEET_WIDTH);
        opaque(iced::widget::center(opaque(sheet)).padding(16).style(move |_| palette.backdrop()))
    }

    pub(crate) fn fallen_layer(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let Some(fell) = self.fell.first() else {
            return Space::new(0, 0).into();
        };
        let quit = fell.why.contains("crashed");
        let said = match quit {
            true => format!("{} crashed, and Loupe carried on without it.", fell.name),
            false => format!("{} stopped answering, and Loupe carried on without it.", fell.name),
        };
        let body = column![
            text(said).size(14),
            text("Turn it off in this song? Its settings are kept, and you can turn it back on from its plugin list.")
                .size(12.5)
                .color(palette.text_dim),
            row![
                Space::with_width(Length::Fill),
                button(text("Leave it on").size(13).font(palette.medium))
                    .padding([7, 16])
                    .style(move |_, status| palette.outlined(status))
                    .on_press(Message::LeaveFallenOn),
                button(text("Turn it off").size(13).font(palette.medium))
                    .padding([7, 16])
                    .style(move |_, status| palette.solid(status))
                    .on_press(Message::TurnFallenOff),
            ]
            .spacing(10),
        ]
        .spacing(12);
        let sheet = self.window(if quit { "A plugin crashed" } else { "A plugin stopped answering" }.to_string(), body.into(), 520.0);
        opaque(iced::widget::center(opaque(sheet)).padding(16).style(move |_| palette.backdrop()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_question_says_what_crashed() {
        let plugin = Report::plugin_fell("Serum", "the plugin crashed");
        assert_eq!(question(&plugin), "Serum crashed. Send a crash report?");
        let loupe = Report::panicked("crates/app/src/main.rs:1", "boom", "");
        assert_eq!(question(&loupe), QUESTION_ABOUT_LOUPE);
    }
}
