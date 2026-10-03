use iced::futures::channel::oneshot;
use iced::widget::{button, column, opaque, row, text, text_input, Space};
use iced::{Alignment, Element, Length, Task};
use loupe_engine::{repair_takes, SavedProject};

use crate::backup::{self, Place};
use crate::{App, Message, Overlay};

const AUDIO_FOLDER: &str = "Audio";
const NUMBER_WIDTH: f32 = 56.0;

impl App {
    fn backup_place(&self) -> Place {
        Place::of(self.path.as_deref(), self.folder.as_deref())
    }

    fn snapshot(&self) -> String {
        SavedProject::capture(&self.project, |track| self.heights.get(&track).copied()).to_text()
    }

    pub(crate) fn keep_safe(&mut self) {
        if self.run.is_some() {
            return;
        }
        let nothing_new = self.revision == self.remembered || self.project.tracks.is_empty();
        let known = self.marked.as_ref().is_some_and(|marked| marked.same_as(self.path.as_deref()));
        if nothing_new && known {
            return;
        }
        let place = self.backup_place();
        if !known {
            backup::mark_running(&place);
            self.marked = Some(place.clone());
        }
        if !nothing_new {
            self.remembered = self.revision;
            backup::remember(place, self.snapshot(), self.backups_kept as usize);
        }
    }

    pub(crate) fn autosave(&mut self) -> Task<Message> {
        let nothing_new = self.revision == self.backed_up || self.project.tracks.is_empty();
        if nothing_new || self.overlay == Overlay::Recover {
            return Task::none();
        }
        let revision = self.revision;
        let place = self.backup_place();
        let snapshot = self.snapshot();
        let keep = self.backups_kept as usize;
        let (done, written) = oneshot::channel();
        std::thread::spawn(move || {
            let _ = done.send(backup::write(&place, &snapshot, keep).map_err(|why| format!("{}: {why}", place.folder.display())));
        });
        Task::perform(
            async move { written.await.unwrap_or_else(|_| Err("the backup stopped unexpectedly".to_string())) },
            move |result| Message::AutosaveDone(revision, result),
        )
    }

    fn next_lost(&mut self) {
        self.overlay = if self.lost.is_empty() { Overlay::None } else { Overlay::Recover };
    }

    pub(crate) fn recover(&mut self) -> Task<Message> {
        if self.lost.is_empty() {
            self.next_lost();
            return Task::none();
        }
        let lost = self.lost.remove(0);
        backup::forget(&lost);
        if let Some(folder) = lost.place.project.as_deref().and_then(|file| file.parent()) {
            repair_takes(&folder.join(AUDIO_FOLDER));
        }
        self.recovering = Some(lost.place.project.clone());
        self.next_lost();
        self.read_project(lost.backup, false)
    }

    pub(crate) fn skip_recovery(&mut self) {
        if !self.lost.is_empty() {
            let lost = self.lost.remove(0);
            backup::forget(&lost);
        }
        self.next_lost();
    }

    pub(crate) fn recover_layer(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let Some(lost) = self.lost.first() else {
            return Space::new(0, 0).into();
        };
        let body = column![
            text(format!("Loupe closed unexpectedly while you were working on {}.", lost.place.name)).size(14),
            text(format!("A backup from {} is ready to open.", backup::when(lost.saved_at))).size(13).color(palette.text_dim),
            text(format!("If you skip it, it stays in {}.", lost.place.folder.display())).size(12).color(palette.text_faint),
            row![
                Space::with_width(Length::Fill),
                button(text("Skip").size(13).font(palette.medium)).padding([7, 16]).style(move |_, status| palette.outlined(status)).on_press(Message::SkipRecovery),
                button(text("Recover").size(13).font(palette.medium)).padding([7, 16]).style(move |_, status| palette.solid(status)).on_press(Message::Recover),
            ]
            .spacing(10),
        ]
        .spacing(12);
        let sheet = self.window("Recover your song".to_string(), body.into(), 520.0);
        opaque(iced::widget::center(opaque(sheet)).padding(16).style(move |_| palette.backdrop()))
    }

    pub(crate) fn autosave_settings(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let field = |value: &str, typed: fn(String) -> Message, entered: Message| {
            text_input("", value)
                .on_input(typed)
                .on_submit(entered)
                .font(palette.mono)
                .size(13)
                .padding([5, 8])
                .width(NUMBER_WIDTH)
                .style(move |_, status| palette.field(status))
        };
        let label = |words: &'static str| text(words).size(12.5).color(palette.text_dim);
        let pending = self.autosave_text != self.autosave_minutes.to_string() || self.kept_text != self.backups_kept.to_string();
        let note: Element<'_, Message> = match (&self.autosave_problem, pending) {
            (Some(problem), _) => text(problem.as_str()).size(12).color(palette.danger).into(),
            (None, true) => text("Press Enter to apply.").size(12).color(palette.text_faint).into(),
            (None, false) => Space::new(0, 0).into(),
        };
        column![
            text("Autosave").size(13).font(palette.medium),
            text("Loupe keeps backup copies of your song in its Backup folder while you work. Your own file only changes when you save.")
                .size(12)
                .color(palette.text_dim),
            row![
                label("Every"),
                field(&self.autosave_text, Message::AutosaveTyped, Message::AutosaveEntered),
                label(minutes_hint()),
                Space::with_width(16),
                label("Keep"),
                field(&self.kept_text, Message::KeptTyped, Message::KeptEntered),
                label(kept_hint()),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
            note,
        ]
        .spacing(8)
        .into()
    }
}

fn minutes_hint() -> &'static str {
    "minutes (0 to 60, 0 is off)"
}

fn kept_hint() -> &'static str {
    "backups (1 to 100)"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings;

    #[test]
    fn the_hints_match_the_allowed_ranges() {
        let minutes = format!("({} to {}", settings::AUTOSAVE_MINUTES.start(), settings::AUTOSAVE_MINUTES.end());
        let kept = format!("({} to {}", settings::BACKUPS_KEPT.start(), settings::BACKUPS_KEPT.end());
        assert!(minutes_hint().contains(&minutes));
        assert!(kept_hint().contains(&kept));
    }
}
