use std::sync::Arc;

use iced::widget::{button, column, container, row, text, Space};
use iced::{Alignment, Element, Length};
use loupe_stock::{key_name, relative_of, Findings, Heard};

use crate::look::{fade, Look};

#[derive(Clone, Debug, PartialEq)]
pub enum KeyMessage {
    Double,
    Half,
    Relative,
    ListenAgain,
    ForgetFile,
    Send,
}

pub struct KeyEditor {
    findings: Option<Arc<Findings>>,
    live: Heard,
    file: Option<(String, Option<Heard>)>,
    factor: f32,
    relative: bool,
    look: Look,
}

impl KeyEditor {
    pub fn new(findings: Option<Arc<Findings>>, look: Look) -> Self {
        Self { findings, live: Heard::default(), file: None, factor: 1.0, relative: false, look }
    }

    pub fn tick(&mut self) {
        if let Some(findings) = &self.findings {
            self.live = Heard { bpm: findings.bpm(), key: findings.key(), sure: findings.sure(), seconds: findings.heard_seconds() };
        }
    }

    pub fn reading(&mut self, name: String) {
        self.file = Some((name, None));
        self.factor = 1.0;
        self.relative = false;
    }

    pub fn file_heard(&mut self, name: String, heard: Heard) {
        self.file = Some((name, Some(heard)));
    }

    fn shown(&self) -> Heard {
        match &self.file {
            Some((_, Some(heard))) => *heard,
            Some((_, None)) => Heard::default(),
            None => self.live,
        }
    }

    pub fn chosen(&self) -> (Option<f32>, Option<u8>) {
        let heard = self.shown();
        let bpm = heard.bpm.map(|bpm| bpm * self.factor);
        let key = heard.key.map(|key| if self.relative { relative_of(key) } else { key });
        (bpm, key)
    }

    pub fn update(&mut self, message: KeyMessage) {
        match message {
            KeyMessage::Double => self.factor = (self.factor * 2.0).min(4.0),
            KeyMessage::Half => self.factor = (self.factor / 2.0).max(0.25),
            KeyMessage::Relative => self.relative = !self.relative,
            KeyMessage::ListenAgain => {
                self.file = None;
                self.factor = 1.0;
                self.relative = false;
                if let Some(findings) = &self.findings {
                    findings.start_again();
                }
            }
            KeyMessage::ForgetFile => {
                self.file = None;
                self.factor = 1.0;
                self.relative = false;
            }
            KeyMessage::Send => {}
        }
    }

    pub fn view(&self) -> Element<'_, KeyMessage> {
        let look = self.look;
        let heard = self.shown();
        let (bpm, key) = self.chosen();
        let big = |label: &'static str, value: String, colour| column![text(label).size(12).color(look.text_dim), text(value).size(44).color(colour)].spacing(4).align_x(Alignment::Center).width(Length::Fill);
        let key_text = key.map_or("-".to_string(), key_name);
        let bpm_text = bpm.map_or("-".to_string(), |bpm| if bpm.fract() == 0.0 { format!("{bpm:.0}") } else { format!("{bpm:.1}") });
        let readouts = row![big("Key", key_text, look.accent), big("BPM", bpm_text, look.text)].spacing(20).width(Length::Fill);
        let source = match &self.file {
            Some((name, Some(_))) => format!("From the file {name}"),
            Some((name, None)) => format!("Reading {name}…"),
            None if heard.seconds <= 0.0 => "Play the song and Loupe Key listens to this track.".to_string(),
            None => format!("Listening to this track · {:.0} s heard", heard.seconds),
        };
        let sure = match heard.key {
            Some(_) if heard.sure >= 0.5 => "Sure of the key",
            Some(_) if heard.sure >= 0.2 => "Fairly sure of the key",
            Some(_) => "Not sure yet, let it hear more",
            None => "",
        };
        let small = |label: String, message: KeyMessage, on: bool| {
            button(text(label).size(12.5))
                .padding([5, 12])
                .style(move |_, status| {
                    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                    button::Style {
                        background: Some(if on { look.raised } else if hovered { look.panel } else { look.background }.into()),
                        text_color: look.text,
                        border: iced::Border::default().rounded(6).width(1).color(look.grid_strong),
                        ..Default::default()
                    }
                })
                .on_press(message)
        };
        let relative = heard.key.map_or("Relative key".to_string(), |key| format!("Use {}", key_name(relative_of(if self.relative { relative_of(key) } else { key }))));
        let tweaks = row![
            small("BPM × 2".to_string(), KeyMessage::Double, self.factor > 1.0),
            small("BPM ÷ 2".to_string(), KeyMessage::Half, self.factor < 1.0),
            small(relative, KeyMessage::Relative, self.relative),
        ]
        .spacing(8);
        let again = match &self.file {
            Some(_) => small("Back to listening".to_string(), KeyMessage::ForgetFile, false),
            None => small("Start listening again".to_string(), KeyMessage::ListenAgain, false),
        };
        let ready = bpm.is_some() || key.is_some();
        let send = button(text("Send to the song").size(13.5))
            .padding([8, 18])
            .style(move |_, status| {
                let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: Some(if hovered { fade(look.accent, 0.85) } else { look.accent }.into()),
                    text_color: look.on_accent,
                    border: iced::Border::default().rounded(7),
                    ..Default::default()
                }
            })
            .on_press_maybe(ready.then_some(KeyMessage::Send));
        let drop = container(text("Drop an audio file here to read its key and BPM").size(12.5).color(look.text_dim))
            .padding(14)
            .width(Length::Fill)
            .center_x(Length::Fill)
            .style(move |_| container::Style {
                border: iced::Border::default().rounded(8).width(1).color(look.grid_strong),
                ..Default::default()
            });
        let body = column![
            readouts,
            text(source).size(12.5).color(look.text_dim),
            text(sure).size(12).color(look.text_dim),
            Space::with_height(6),
            tweaks,
            row![again, Space::with_width(Length::Fill), send].align_y(Alignment::Center),
            Space::with_height(Length::Fill),
            drop,
            text("Send sets the song's BPM, and the key and scale on every Loupe Tune.").size(11.5).color(look.text_dim),
        ]
        .spacing(10)
        .align_x(Alignment::Center);
        container(body).padding([24, 28]).width(Length::Fill).height(Length::Fill).style(move |_| container::Style {
            background: Some(look.background.into()),
            ..Default::default()
        })
        .into()
    }
}
