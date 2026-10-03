#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::time::Duration;

use iced::{Element, Subscription, Task, Theme};
use loupe_stock::{knob, BandShape, Effect, Equalizer, Knob};
use loupe_stock_ui::{EqEditor, EqMessage, Look};

const RATE: f32 = 48_000.0;
const FRAMES_PER_TICK: usize = 768;

struct Preview {
    audio: Equalizer,
    editor: EqEditor,
    clock: u64,
    noise: u32,
}

#[derive(Debug, Clone)]
enum Message {
    Editor(EqMessage),
    Tick,
}

impl Preview {
    fn new() -> (Self, Task<Message>) {
        let mut audio = Equalizer::new();
        audio.prepare(RATE);
        let editor = EqEditor::new(RATE, Some(audio.scopes()), Look::default());
        let mut preview = Self { audio, editor, clock: 0, noise: 1 };
        let start: [(usize, BandShape, f32, f32, f32, f32); 4] = [
            (0, BandShape::LowCut, 60.0, 0.0, 0.707, 3.0),
            (1, BandShape::Bell, 280.0, -3.5, 1.4, 1.0),
            (2, BandShape::Bell, 3200.0, 2.5, 0.8, 1.0),
            (3, BandShape::HighShelf, 10_000.0, 3.0, 0.707, 1.0),
        ];
        if std::env::var("LOUPE_EQ_EMPTY").is_err() {
            for (band, shape, hz, gain, q, slope) in start {
                let changes = vec![
                    (knob(band, Knob::On), 1.0),
                    (knob(band, Knob::Shape), shape.index()),
                    (knob(band, Knob::Freq), hz),
                    (knob(band, Knob::Gain), gain),
                    (knob(band, Knob::Q), q),
                    (knob(band, Knob::Slope), slope),
                ];
                preview.handle(EqMessage::Set(changes));
            }
        }
        (preview, Task::none())
    }

    fn handle(&mut self, message: EqMessage) {
        for (index, value) in self.editor.update(message) {
            self.audio.set(index, value);
        }
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::Editor(message) => self.handle(message),
            Message::Tick => {
                let mut block = vec![[0.0f32; 2]; FRAMES_PER_TICK];
                let mut pink = [0.0f32; 3];
                for frame in &mut block {
                    self.noise ^= self.noise << 13;
                    self.noise ^= self.noise >> 17;
                    self.noise ^= self.noise << 5;
                    let white = self.noise as f32 / u32::MAX as f32 * 2.0 - 1.0;
                    pink[0] = 0.997 * pink[0] + 0.029 * white;
                    pink[1] = 0.985 * pink[1] + 0.032 * white;
                    pink[2] = 0.95 * pink[2] + 0.048 * white;
                    let t = self.clock as f32 / RATE;
                    let tones = 0.08 * (std::f32::consts::TAU * 110.0 * t).sin() + 0.03 * (std::f32::consts::TAU * 2200.0 * t).sin();
                    let value = (pink[0] + pink[1] + pink[2]) * 0.6 + tones;
                    *frame = [value, value];
                    self.clock += 1;
                }
                self.audio.process(&mut block);
                self.editor.tick();
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        self.editor.view().map(Message::Editor)
    }

    fn subscription(&self) -> Subscription<Message> {
        iced::time::every(Duration::from_millis(16)).map(|_| Message::Tick)
    }
}

fn main() -> iced::Result {
    iced::application("Loupe EQ", Preview::update, Preview::view)
        .subscription(Preview::subscription)
        .theme(|_| Theme::Dark)
        .font(include_bytes!("../../app/assets/Inter-Regular.ttf").as_slice())
        .default_font(iced::Font::with_name("Inter"))
        .antialiasing(true)
        .window_size((980.0, 520.0))
        .run_with(Preview::new)
}
