#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::f32::consts::TAU;
use std::time::Duration;

use iced::widget::{button, column, container, row, text, Space};
use iced::{Element, Length, Subscription, Task, Theme};
use loupe_stock::{Chorus, Compressor, Deesser, Saturation, Delay, Effect, Equalizer, Limiter, Reverb};
use loupe_stock_ui::{Change, ChorusEditor, CompressorEditor, DeesserEditor, SaturationEditor, DelayEditor, EqEditor, EqMessage, LimiterEditor, Look, ReverbEditor};

const RATE: f32 = 48_000.0;
const BPM: f32 = 120.0;
const FRAMES_PER_TICK: usize = 768;
const TABS: [&str; 8] = ["Loupe EQ", "Loupe Compressor", "Loupe Limiter", "Loupe Delay", "Loupe Reverb", "Loupe De-esser", "Loupe Saturation", "Loupe Chorus"];

struct Beat {
    clock: u64,
    noise: u32,
    hat_last: f32,
}

impl Beat {
    fn next(&mut self) -> [f32; 2] {
        let t = self.clock as f32 / RATE;
        let beat_length = 60.0 / BPM;
        let in_beat = t % beat_length;
        let beat = (t / beat_length) as u64;
        let eighth = t % (beat_length / 2.0);
        self.noise ^= self.noise << 13;
        self.noise ^= self.noise >> 17;
        self.noise ^= self.noise << 5;
        let white = self.noise as f32 / u32::MAX as f32 * 2.0 - 1.0;
        let sweep = 45.0 + 90.0 * (-in_beat * 30.0).exp();
        let kick = 0.9 * (-in_beat * 9.0).exp() * (TAU * sweep * in_beat).sin();
        let snare = if beat % 2 == 1 { (-in_beat * 18.0).exp() * (0.35 * white + 0.2 * (TAU * 185.0 * in_beat).sin()) } else { 0.0 };
        let bright = white - self.hat_last;
        self.hat_last = white;
        let hat = 0.08 * (-eighth * 90.0).exp() * bright;
        let notes = [220.0, 277.18, 329.63, 440.0, 329.63, 277.18];
        let half_bar = beat_length * 2.0;
        let note = notes[((t / half_bar) as usize) % notes.len()];
        let since = t % half_bar;
        let pluck = 0.22 * (-since * 5.0).exp() * ((TAU * note * since).sin() + 0.4 * (TAU * 2.0 * note * since).sin());
        self.clock += 1;
        let middle = kick + snare + pluck;
        [middle + hat, middle - hat * 0.5]
    }
}

struct Preview {
    tab: usize,
    beat: Beat,
    eq: Equalizer,
    compressor: Compressor,
    limiter: Limiter,
    delay: Delay,
    reverb: Reverb,
    deesser: Deesser,
    saturation: Saturation,
    chorus: Chorus,
    eq_editor: EqEditor,
    compressor_editor: CompressorEditor,
    limiter_editor: LimiterEditor,
    delay_editor: DelayEditor,
    reverb_editor: ReverbEditor,
    deesser_editor: DeesserEditor,
    saturation_editor: SaturationEditor,
    chorus_editor: ChorusEditor,
    look: Look,
}

#[derive(Debug, Clone)]
enum Message {
    Tab(usize),
    Eq(EqMessage),
    Knob(Change),
    Tick,
}

impl Preview {
    fn new() -> (Self, Task<Message>) {
        let look = Look::default();
        let mut eq = Equalizer::new();
        let mut compressor = Compressor::new();
        let mut limiter = Limiter::new();
        let mut delay = Delay::new();
        let mut reverb = Reverb::new();
        let mut deesser = Deesser::new();
        let mut saturation = Saturation::new();
        let mut chorus = Chorus::new();
        for effect in [&mut eq as &mut dyn Effect, &mut compressor, &mut limiter, &mut delay, &mut reverb, &mut deesser, &mut saturation, &mut chorus] {
            effect.prepare(RATE);
            effect.set_tempo(BPM);
        }
        let tab = std::env::args().nth(1).and_then(|tab| tab.parse().ok()).unwrap_or(0usize).min(TABS.len() - 1);
        let preview = Self {
            tab,
            beat: Beat { clock: 0, noise: 7, hat_last: 0.0 },
            eq_editor: EqEditor::new(RATE, Some(eq.scopes()), look),
            compressor_editor: CompressorEditor::new(Some(compressor.history()), look),
            limiter_editor: LimiterEditor::new(Some(limiter.history()), look),
            delay_editor: DelayEditor::new(BPM, look),
            reverb_editor: ReverbEditor::new(look),
            deesser_editor: DeesserEditor::new(Some(deesser.history()), look),
            saturation_editor: SaturationEditor::new(look),
            chorus_editor: ChorusEditor::new(look),
            eq,
            compressor,
            limiter,
            delay,
            reverb,
            deesser,
            saturation,
            chorus,
            look,
        };
        (preview, Task::none())
    }

    fn effect(&mut self) -> &mut dyn Effect {
        match self.tab {
            0 => &mut self.eq,
            1 => &mut self.compressor,
            2 => &mut self.limiter,
            3 => &mut self.delay,
            4 => &mut self.reverb,
            5 => &mut self.deesser,
            6 => &mut self.saturation,
            _ => &mut self.chorus,
        }
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::Tab(tab) => self.tab = tab,
            Message::Eq(message) => {
                for (index, value) in self.eq_editor.update(message) {
                    self.eq.set(index, value);
                }
            }
            Message::Knob(change) => {
                let changes = match self.tab {
                    1 => self.compressor_editor.update(change),
                    2 => self.limiter_editor.update(change),
                    3 => self.delay_editor.update(change),
                    4 => self.reverb_editor.update(change),
                    5 => self.deesser_editor.update(change),
                    6 => self.saturation_editor.update(change),
                    _ => self.chorus_editor.update(change),
                };
                for (index, value) in changes {
                    self.effect().set(index, value);
                }
            }
            Message::Tick => {
                let mut block: Vec<[f32; 2]> = (0..FRAMES_PER_TICK).map(|_| self.beat.next()).collect();
                self.effect().process(&mut block);
                match self.tab {
                    0 => self.eq_editor.tick(),
                    1 => self.compressor_editor.tick(),
                    2 => self.limiter_editor.tick(),
                    5 => self.deesser_editor.tick(),
                    _ => {}
                }
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let look = self.look;
        let tabs = row(TABS.iter().enumerate().map(|(index, name)| {
            let chosen = index == self.tab;
            button(text(*name).size(13))
                .padding([6, 14])
                .style(move |_, status| {
                    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                    button::Style {
                        background: Some(if chosen { look.raised } else if hovered { look.panel } else { look.background }.into()),
                        text_color: if chosen { look.text } else { look.text_dim },
                        border: iced::Border::default().rounded(6),
                        ..Default::default()
                    }
                })
                .on_press(Message::Tab(index))
                .into()
        }))
        .spacing(4);
        let bar = container(row![tabs, Space::with_width(Length::Fill), text("Test beat at 120 BPM").size(12).color(look.text_dim)].align_y(iced::Alignment::Center))
            .padding([8, 12])
            .width(Length::Fill)
            .style(move |_| container::Style { background: Some(look.background.into()), ..Default::default() });
        let body = match self.tab {
            0 => self.eq_editor.view().map(Message::Eq),
            1 => self.compressor_editor.view().map(Message::Knob),
            2 => self.limiter_editor.view().map(Message::Knob),
            3 => self.delay_editor.view().map(Message::Knob),
            4 => self.reverb_editor.view().map(Message::Knob),
            5 => self.deesser_editor.view().map(Message::Knob),
            6 => self.saturation_editor.view().map(Message::Knob),
            _ => self.chorus_editor.view().map(Message::Knob),
        };
        column![bar, body].into()
    }

    fn subscription(&self) -> Subscription<Message> {
        iced::time::every(Duration::from_millis(16)).map(|_| Message::Tick)
    }
}

fn main() -> iced::Result {
    iced::application("Loupe plugins", Preview::update, Preview::view)
        .subscription(Preview::subscription)
        .theme(|_| Theme::Dark)
        .font(include_bytes!("../../app/assets/Inter-Regular.ttf").as_slice())
        .default_font(iced::Font::with_name("Inter"))
        .antialiasing(true)
        .window_size((1040.0, 640.0))
        .run_with(Preview::new)
}
