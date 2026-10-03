use std::fmt;

use iced::widget::{button, column, pick_list, row, text};
use iced::{Alignment, Element};
use loupe_engine::{ClipId, Frames, TrackId};

use crate::mixer::Level;
use crate::{icon, rule, App, LoopRange, Message};

const LABEL_WIDTH: f32 = 90.0;

#[derive(Debug, Clone, PartialEq)]
pub struct TrackChoice {
    pub id: TrackId,
    pub name: String,
}

impl fmt::Display for TrackChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

pub struct Preview {
    pub clip: ClipId,
    pub loop_range: LoopRange,
    pub playhead: Frames,
}

impl App {
    pub(crate) fn clip_sheet(&self, id: ClipId) -> Element<'_, Message> {
        let palette = self.palette;
        let (Some(clip), Some(track)) = (self.project.clip(id), self.project.track_of(id)) else {
            return text("This clip is gone.").size(13).into();
        };
        let label = |name: &'static str| text(name).size(12.5).color(palette.text_dim).width(LABEL_WIDTH);
        let choices: Vec<TrackChoice> =
            self.project.tracks.iter().map(|t| TrackChoice { id: t.id, name: t.name.clone() }).collect();
        let current = TrackChoice { id: track.id, name: track.name.clone() };
        let seconds = |frames: Frames| frames as f64 / self.project.rate.max(1) as f64;
        let beats = seconds(clip.start) * self.project.bpm / 60.0;
        let starts = format!("bar {}.{}", (beats / 4.0) as u64 + 1, beats as u64 % 4 + 1);
        let previewing = self.preview.as_ref().is_some_and(|preview| preview.clip == id);
        let muted = clip.muted;
        let file = clip.source.path.clone();
        let found = !clip.source.frames.is_empty();

        let preview = button(
            row![
                icon(if previewing { "square" } else { "play" }, 13.0),
                text(if previewing { "Stop" } else { "Play this clip alone" }).size(13).font(palette.medium),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding([7, 14])
        .style(move |_, status| palette.outlined(status))
        .on_press(Message::TogglePreview(id));

        let fields = column![
            row![
                label("Track"),
                pick_list(choices, Some(current), move |choice: TrackChoice| Message::ClipToTrack(id, choice.id))
                    .text_size(13)
                    .padding([5, 10]),
            ]
            .align_y(Alignment::Center),
            row![
                label("File"),
                column![
                    text(file.display().to_string())
                        .size(11.5)
                        .font(palette.mono)
                        .color(palette.text_dim)
                        .wrapping(text::Wrapping::Glyph),
                    row![
                        button(text(if self.copied.is_some() { "Copied" } else { "Copy path" }).size(12).font(palette.medium))
                            .padding([4, 10])
                            .style(move |_, status| palette.outlined(status))
                            .on_press(Message::CopyText(file.display().to_string())),
                        button(text("Show in folder").size(12).font(palette.medium))
                            .padding([4, 10])
                            .style(move |_, status| palette.outlined(status))
                            .on_press_maybe(found.then(|| Message::ShowInFolder(file.clone()))),
                    ]
                    .spacing(8),
                ]
                .spacing(8),
            ],
            row![label("Starts"), text(starts).size(13).font(palette.mono)].align_y(Alignment::Center),
            row![
                label("Length"),
                text(format!("{:.3} s", seconds(clip.len))).size(13).font(palette.mono),
            ]
            .align_y(Alignment::Center),
            row![
                label("Gain"),
                self.level_readout(Level::Clip(id), clip.gain),
                text("dB").size(11.5).color(palette.text_faint),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
            row![
                label("Muted"),
                button(text(if muted { "Yes" } else { "No" }).size(12.5).font(palette.medium))
                    .padding([4, 12])
                    .style(move |_, status| palette.mute(muted, status))
                    .on_press(Message::ToggleClipMute(id)),
            ]
            .align_y(Alignment::Center),
        ]
        .spacing(12);

        let chain = column![
            text("Plugins on this clip").size(12.5).color(palette.text_dim),
            self.clip_fx_block(id),
        ]
        .spacing(8);
        let body = column![fields, rule(palette), chain, rule(palette), preview].spacing(16);
        self.window(clip.source.name.clone(), body.into(), 520.0)
    }

    pub(crate) fn toggle_preview(&mut self, id: ClipId) {
        if self.preview.as_ref().is_some_and(|preview| preview.clip == id) {
            self.stop_preview();
            return;
        }
        self.stop_preview();
        let Some((start, end)) = self.project.clip(id).map(|clip| (clip.start, clip.end())) else {
            return;
        };
        self.preview = Some(Preview { clip: id, loop_range: self.loop_range, playhead: self.playhead });
        self.engine.audition(Some(id));
        self.engine.set_loop(Some((start, end)));
        self.engine.seek(start);
        self.playhead = start;
        self.engine.play();
        self.playing = true;
    }

    pub(crate) fn stop_preview(&mut self) {
        let Some(preview) = self.preview.take() else {
            return;
        };
        self.engine.stop();
        self.playing = false;
        self.engine.audition(None);
        self.engine.set_loop(preview.loop_range);
        self.seek(preview.playhead);
    }
}
