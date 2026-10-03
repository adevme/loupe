use std::path::{Path, PathBuf};

use iced::futures::channel::oneshot;
use iced::widget::{button, checkbox, column, horizontal_space, row, text};
use iced::{Alignment, Element, Task};
use loupe_engine::{export, next_version_folder, ExportPlan, SavedProject};

use crate::{App, Message, Overlay};

const EXPORTS_FOLDER: &str = "Exports";

impl App {
    fn exports_folder(&self) -> Option<PathBuf> {
        let beside_the_song = self.path.as_deref().and_then(Path::parent).map(|folder| folder.join(EXPORTS_FOLDER));
        self.export_elsewhere.clone().or(beside_the_song)
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

        let mut options = column![
            text(next_version_folder(&exports).display().to_string()).size(11.5).font(palette.mono).color(palette.text_dim),
            checkbox("Split export: also write each track as its own file", self.export_split)
                .on_toggle(Message::ExportSplit)
                .text_size(13)
                .size(16),
        ]
        .spacing(14);
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
        let folder = next_version_folder(&exports);
        let plan = ExportPlan {
            folder: folder.clone(),
            name: self.path.as_deref().map(crate::home::stem).unwrap_or_else(|| "Untitled".to_string()),
            split: self.export_split,
            range: if self.export_range_only { self.loop_range } else { None },
            project_file: SavedProject::capture(&self.project, |track| self.heights.get(&track).copied()).to_text(),
        };
        let project = self.project.clone();
        self.overlay = Overlay::None;
        self.exporting = true;
        self.problem = None;
        self.notice = None;
        let (done, exported) = oneshot::channel();
        std::thread::spawn(move || {
            let _ = done.send(export(&project, &plan).map(|()| folder));
        });
        Task::perform(
            async move { exported.await.unwrap_or_else(|_| Err("the export stopped unexpectedly".into())) },
            Message::Exported,
        )
    }
}
