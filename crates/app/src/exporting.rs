use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use iced::futures::channel::oneshot;
use iced::widget::{button, checkbox, column, horizontal_space, row, text, Space};
use iced::{Alignment, Element, Length, Task};
use loupe_engine::{next_version_folder, ExportPlan, Format, Normalise, SavedProject};

use crate::theme;
use crate::{App, Message, Overlay};

const EXPORTS_FOLDER: &str = "Exports";
const NORMALISE_CHOICES: [Normalise; 9] = [
    Normalise::Off,
    Normalise::Peak(-0.1),
    Normalise::Peak(-0.3),
    Normalise::Peak(-1.0),
    Normalise::Loudness(-11.0),
    Normalise::Loudness(-14.0),
    Normalise::Loudness(-16.0),
    Normalise::Loudness(-18.0),
    Normalise::Loudness(-23.0),
];

fn normalise_choices(chosen: Normalise) -> Vec<Normalise> {
    let mut choices = NORMALISE_CHOICES.to_vec();
    if !choices.contains(&chosen) {
        choices.push(chosen);
    }
    choices
}

fn rate_words(format: Format, rate: u32) -> String {
    let written = if format == Format::Mp3Cbr320 { Format::mp3_rate(rate) } else { rate };
    if written == rate {
        format!("{rate} Hz, the project's sample rate")
    } else {
        format!("{written} Hz: MP3 stops at 48 kHz, the project runs at {rate} Hz")
    }
}

impl App {
    fn exports_folder(&self) -> Option<PathBuf> {
        let beside_the_song = self.path.as_deref().and_then(Path::parent).map(|folder| folder.join(EXPORTS_FOLDER));
        self.export_elsewhere.clone().or(beside_the_song)
    }

    pub(crate) fn choose_export(&mut self, change: impl FnOnce(&mut crate::settings::ExportChoices)) {
        change(&mut self.export);
        for (key, value) in self.export.entries() {
            if let Err(why) = crate::settings::save(key, &value) {
                self.problem = Some(format!("Could not save settings: {why}"));
                return;
            }
        }
    }

    pub(crate) fn export_sheet(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let Some(exports) = self.exports_folder() else {
            let body = column![
                text("Save the project first. Exports go in the Exports folder beside it.")
                    .size(13)
                    .color(palette.text_dim),
                row![
                    horizontal_space(),
                    button(text("Save…").size(13).font(palette.medium))
                        .padding([7, 18])
                        .style(move |_, status| palette.solid(status))
                        .on_press(Message::SaveAs),
                ],
            ]
            .spacing(16);
            return self.window("Export".to_string(), body.into(), 460.0);
        };

        let chosen = self.export;
        let label = |words: &'static str| text(words).size(13).font(palette.medium);
        let mut options = column![
            text(next_version_folder(&exports).display().to_string()).size(11.5).font(palette.mono).color(palette.text_dim),
            row![
                column![
                    label("Format"),
                    theme::picker(palette, Format::ALL, Some(chosen.format), Message::ExportFormatChosen).width(Length::Fill),
                ]
                .spacing(8),
                column![
                    label("Normalise"),
                    theme::picker(palette, normalise_choices(chosen.normalise), Some(chosen.normalise), Message::ExportNormaliseChosen)
                        .width(Length::Fill),
                ]
                .spacing(8),
            ]
            .spacing(12),
            text(rate_words(chosen.format, self.project.rate)).size(12).color(palette.text_dim),
        ]
        .spacing(14);
        if let Some(bits) = chosen.format.bits() {
            options = options.push(
                checkbox(format!("Dither to {bits} bit (TPDF)"), chosen.dither())
                    .on_toggle(Message::ExportDither)
                    .text_size(13)
                    .size(16),
            );
        }
        options = options.push(
            checkbox("Split export: also write each track as its own file", chosen.split)
                .on_toggle(Message::ExportSplit)
                .text_size(13)
                .size(16),
        );
        if self.loop_range.is_some() {
            options = options.push(
                checkbox("Only the selected range", self.export_range_only)
                    .on_toggle(Message::ExportRangeOnly)
                    .text_size(13)
                    .size(16),
            );
        }

        let choices = row![
            button(text("Choose another place…").size(13).font(palette.medium))
                .padding([7, 14])
                .style(move |_, status| palette.ghost(status))
                .on_press(Message::ExportElsewhere),
            horizontal_space(),
            button(text("Cancel").size(13).font(palette.medium))
                .padding([7, 14])
                .style(move |_, status| palette.outlined(status))
                .on_press(Message::CloseOverlay),
            button(text("Export").size(13).font(palette.medium))
                .padding([7, 18])
                .style(move |_, status| palette.solid(status))
                .on_press(Message::StartExport),
        ]
        .spacing(10)
        .align_y(Alignment::Center);

        self.window("Export".to_string(), column![options, choices].spacing(18).into(), 540.0)
    }

    pub(crate) fn pick_export_folder(&mut self) -> Task<Message> {
        let start_in = self.exports_folder().unwrap_or_default();
        Task::perform(
            async move {
                rfd::AsyncFileDialog::new()
                    .set_title("Choose where exports go")
                    .set_directory(start_in)
                    .pick_folder()
                    .await
                    .map(|folder| folder.path().to_path_buf())
            },
            Message::ExportFolderPicked,
        )
    }

    pub(crate) fn start_export(&mut self) -> Task<Message> {
        let Some(exports) = self.exports_folder() else {
            return Task::none();
        };
        let still_stretching = self.project.clips().filter(|clip| clip.waiting_for_stretch()).count();
        if still_stretching > 0 {
            self.problem = Some(format!(
                "Still working out the time stretch on {still_stretching} clip{}. Try again in a moment.",
                if still_stretching == 1 { "" } else { "s" }
            ));
            return self.stretch_waiting();
        }
        let folder = next_version_folder(&exports);
        let plan = ExportPlan {
            folder: folder.clone(),
            name: self.path.as_deref().map(crate::home::stem).unwrap_or_else(|| "Untitled".to_string()),
            split: self.export.split,
            range: if self.export_range_only { self.loop_range } else { None },
            project_file: {
                self.gather_fx_state();
                SavedProject::capture(&self.project, |track| self.heights.get(&track).copied()).to_text()
            },
            format: self.export.format,
            dither: self.export.dither(),
            normalise: self.export.normalise,
        };
        let project = self.project.clone();
        let plugins_off = self.plugins_off.clone();
        let rate = self.engine.rate();
        self.overlay = Overlay::None;
        self.exporting = true;
        self.problem = None;
        self.notice = None;
        self.export_progress.store(0, Ordering::Relaxed);
        self.stop_export.store(false, Ordering::Relaxed);
        self.export_began = Some(std::time::Instant::now());
        let progress = self.export_progress.clone();
        let stop = self.stop_export.clone();
        let (done, exported) = oneshot::channel();
        std::thread::spawn(move || {
            let report = |fraction: f32| {
                progress.store((fraction * crate::EXPORT_PROGRESS_STEPS as f32) as u32, Ordering::Relaxed);
                !stop.load(Ordering::Relaxed)
            };
            let mut racks: Box<dyn loupe_engine::Chains> =
                Box::new(crate::racks::Racks::new(rate, 512, crate::racks::Peeks::default(), plugins_off, crate::racks::Falls::default()).away_from_the_audio_thread());
            racks.follow(&project);
            let went = loupe_engine::export_through(&project, &plan, &report, Some(racks.as_mut()));
            drop(racks);
            let _ = done.send(went.map(|note| (folder, note)));
        });
        Task::perform(
            async move { exported.await.unwrap_or_else(|_| Err("the export stopped unexpectedly".into())) },
            Message::Exported,
        )
    }
}

impl App {
    pub(crate) fn exported_sheet(&self, folder: &std::path::Path, note: Option<&str>) -> Element<'_, Message> {
        let palette = self.palette;
        let where_it_went = folder.display().to_string();
        let mut body = column![
            text("Your song is exported.").size(15).font(palette.medium),
            text(where_it_went.clone()).size(12.5).font(palette.mono).color(palette.text_dim).wrapping(text::Wrapping::Glyph),
        ]
        .spacing(10);
        if let Some(note) = note {
            body = body.push(text(note.to_string()).size(12.5).color(palette.text_dim));
        }
        let open = button(text("Show in folder").size(13).font(palette.medium))
            .padding([7, 16])
            .style(move |_, status| palette.outlined(status))
            .on_press(Message::ShowInFolder(folder.to_path_buf()));
        let copy = button(text(if self.copied.is_some() { "Copied" } else { "Copy path" }).size(13).font(palette.medium))
            .padding([7, 16])
            .style(move |_, status| palette.outlined(status))
            .on_press(Message::CopyText(where_it_went));
        body = body.push(row![open, copy, Space::with_width(Length::Fill)].spacing(10).align_y(Alignment::Center));
        self.window("Exported".to_string(), body.into(), 520.0)
    }
}
