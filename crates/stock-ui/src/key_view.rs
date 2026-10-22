use std::sync::Arc;

use iced::widget::{column, container, row, text, Space};
use iced::{Alignment, Color, Element, Length};
use loupe_stock::{key_name, relative_of, Findings, Heard, SETTLED_SECONDS, SURE_ENOUGH};

use crate::kit::*;
use crate::look::{Look, MEDIUM};

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
        let look = look.tinted(crate::look::accents::KEY);
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
        let big = |label: &'static str, value: String, colour| {
            card(look, column![caption(look, label), text(value).size(56).font(MEDIUM).color(colour)].spacing(6).align_x(Alignment::Center))
                .padding([18, 20])
                .width(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill)
        };
        let key_text = key.map_or("-".to_string(), key_name);
        let bpm_text = bpm.map_or("-".to_string(), |bpm| if bpm.fract() == 0.0 { format!("{bpm:.0}") } else { format!("{bpm:.1}") });
        let readouts = row![big("Key", key_text, look.accent), big("BPM", bpm_text, look.text)].spacing(14).width(Length::Fill).height(Length::Fill);
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
        let settled = self.live.sure >= SURE_ENOUGH && self.live.seconds >= SETTLED_SECONDS;
        let follows = match self.live.key {
            Some(key) if settled => format!("Every Loupe Tune set to Detected follows {}", key_name(key)),
            Some(_) => "Not settled enough yet for a Loupe Tune to follow".to_string(),
            None => String::new(),
        };
        let relative = heard.key.map_or("Relative key".to_string(), |key| format!("Use {}", key_name(relative_of(if self.relative { relative_of(key) } else { key }))));
        let tweaks = row![
            toggle(look, "BPM × 2", self.factor > 1.0, KeyMessage::Double).element,
            toggle(look, "BPM ÷ 2", self.factor < 1.0, KeyMessage::Half).element,
            toggle(look, relative, self.relative, KeyMessage::Relative).element,
        ]
        .spacing(8);
        let again = match &self.file {
            Some(_) => push(look, "Back to listening", Some(KeyMessage::ForgetFile)).element,
            None => push(look, "Start listening again", Some(KeyMessage::ListenAgain)).element,
        };
        let ready = bpm.is_some() || key.is_some();
        let send = primary(look, "Send to the song", ready.then_some(KeyMessage::Send));
        let drop = container(text("Drop an audio file here to read its key and BPM").size(12).font(MEDIUM).color(look.text_dim))
            .padding(16)
            .width(Length::Fill)
            .center_x(Length::Fill)
            .style(move |_| container::Style {
                background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.18).into()),
                border: iced::Border::default().rounded(8).width(1).color(look.grid_strong),
                ..Default::default()
            });
        let status = column![
            text(source).size(12.5).color(look.text_dim),
            text(sure).size(12).font(MEDIUM).color(look.text_faint),
            text(follows).size(11.5).font(MEDIUM).color(if settled { look.accent } else { look.text_faint }),
        ]
        .spacing(4)
        .align_x(Alignment::Center);
        let body = column![
            readouts,
            status,
            Space::with_height(4),
            row![tweaks, Space::with_width(Length::Fill), again].spacing(8).align_y(Alignment::Center),
            drop,
            row![text("Send sets the song's BPM, and the key and scale on every Loupe Tune.").size(11.5).color(look.text_faint), Space::with_width(Length::Fill), send]
                .spacing(12)
                .align_y(Alignment::Center),
        ]
        .spacing(12)
        .align_x(Alignment::Center);
        let body = container(body).padding([18, 20]).width(Length::Fill).height(Length::Fill).style(move |_| container::Style {
            background: Some(iced::gradient::Linear::new(iced::Radians(std::f32::consts::PI)).add_stop(0.0, crate::look::lift(look.deck, 0.018)).add_stop(1.0, crate::look::lift(look.deck, -0.02)).into()),
            ..Default::default()
        });
        column![header(look, "Key", "Key and tempo finder"), body].into()
    }
}
