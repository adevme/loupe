use std::sync::Arc;

use iced::widget::canvas::{self, Geometry, Path, Stroke};
use iced::widget::{column, container, row, Space};
use iced::{mouse, Alignment, Element, Length, Point, Rectangle, Renderer, Size, Theme};
use loupe_stock::{Denoise, Spectra};

use crate::kit::{dial, graph_frame, knob_row, panel, put_text, toggle};
use crate::knobs::{change, Change, Knobs};
use crate::look::Look;

const LOWEST_HZ: f32 = 30.0;
const HIGHEST_HZ: f32 = 20_000.0;
const LOUDEST_DB: f32 = 0.0;
const QUIETEST_DB: f32 = -96.0;
const MARKS: [f32; 6] = [50.0, 200.0, 1_000.0, 3_000.0, 8_000.0, 16_000.0];
const TOP_GAP: f32 = 18.0;

pub struct DenoiseEditor {
    knobs: Knobs,
    spectra: Option<Arc<Spectra>>,
    heard: Vec<f32>,
    room: Vec<f32>,
    kept: Vec<f32>,
    rate: f32,
    look: Look,
}

impl DenoiseEditor {
    pub fn new(spectra: Option<Arc<Spectra>>, rate: f32, look: Look) -> Self {
        let bins = Spectra::bins();
        Self { knobs: Knobs::of(&Denoise::new()), spectra, heard: vec![0.0; bins], room: vec![0.0; bins], kept: vec![1.0; bins], rate, look }
    }

    pub fn load(&mut self, values: &[f32]) {
        for (slot, value) in values.iter().enumerate().take(self.knobs.values.len()) {
            self.knobs.values[slot] = *value;
        }
    }

    pub fn update(&mut self, change: Change) -> Vec<(usize, f32)> {
        self.knobs.apply(change)
    }

    pub fn tick(&mut self) {
        if let Some(spectra) = &self.spectra {
            spectra.heard.latest(&mut self.heard);
            spectra.room.latest(&mut self.room);
            spectra.kept.latest(&mut self.kept);
        }
    }

    pub fn view(&self) -> Element<'_, Change> {
        let look = self.look;
        let values = &self.knobs.values;
        let params = self.knobs.params;
        let knob = |index: usize, colour| dial(look, params, values, index, colour, change);
        let amount = knob_row(vec![knob(0, look.accent), knob(1, look.danger), knob(2, look.bands[3])]);
        let shape = knob_row(vec![knob(4, look.bands[5]), knob(5, look.bands[1])]);
        let follow = toggle(look, "Follow the room", values[3] >= 0.5, Change(3, if values[3] >= 0.5 { 0.0 } else { 1.0 }));
        let controls = row![amount, Space::with_width(18), shape, Space::with_width(Length::Fill), follow].align_y(Alignment::Center);
        let graph = canvas::Canvas::new(Bands { editor: self }).width(Length::Fill).height(Length::Fill);
        let backdrop = container(container(graph).padding([12, 12])).width(Length::Fill).height(Length::Fill).style(move |_| {
            container::Style { background: Some(look.background.into()), ..Default::default() }
        });
        column![backdrop, panel(look, controls)].into()
    }
}

fn across(hz: f32, width: f32) -> f32 {
    let share = (hz.max(LOWEST_HZ) / LOWEST_HZ).log10() / (HIGHEST_HZ / LOWEST_HZ).log10();
    share.clamp(0.0, 1.0) * width
}

fn down(db: f32, top: f32, tall: f32) -> f32 {
    let share = ((db - LOUDEST_DB) / (QUIETEST_DB - LOUDEST_DB)).clamp(0.0, 1.0);
    top + share * tall
}

struct Bands<'a> {
    editor: &'a DenoiseEditor,
}

impl Bands<'_> {
    fn curve(&self, frame: &mut canvas::Frame, size: Size, levels: &[f32], colour: iced::Color, thick: f32) {
        let tall = (size.height - TOP_GAP - 16.0).max(1.0);
        let per_bin = self.editor.rate / 2.0 / (levels.len().max(2) - 1) as f32;
        let mut path = canvas::path::Builder::new();
        let mut started = false;
        for (bin, level) in levels.iter().enumerate().skip(1) {
            let hz = bin as f32 * per_bin;
            if hz < LOWEST_HZ {
                continue;
            }
            let db = 20.0 * level.max(1e-9).log10();
            let at = Point::new(across(hz, size.width), down(db, TOP_GAP, tall));
            match started {
                false => {
                    path.move_to(at);
                    started = true;
                }
                true => path.line_to(at),
            }
        }
        if started {
            frame.stroke(&path.build(), Stroke::default().with_color(colour).with_width(thick));
        }
    }
}

impl canvas::Program<Change> for Bands<'_> {
    type State = ();

    fn draw(&self, _state: &(), renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let look = self.editor.look;
        let size = bounds.size();
        if !crate::kit::roomy(size) {
            return Vec::new();
        }
        let mut frame = graph_frame(renderer, size, &look);
        let tall = (size.height - TOP_GAP - 16.0).max(1.0);
        for hz in MARKS {
            let x = across(hz, size.width);
            frame.fill_rectangle(Point::new(x - 0.5, TOP_GAP), Size::new(1.0, tall), look.grid);
            let said = if hz >= 1000.0 { format!("{:.0}k", hz / 1000.0) } else { format!("{hz:.0}") };
            put_text(&mut frame, &said, Point::new(x, size.height - 14.0), 10.0, look.text_dim, (0.5, 0.0));
        }
        for db in [-24.0, -48.0, -72.0] {
            let y = down(db, TOP_GAP, tall);
            frame.fill_rectangle(Point::new(0.0, y - 0.5), Size::new(size.width, 1.0), look.grid);
            put_text(&mut frame, &format!("{db:.0}"), Point::new(4.0, y + 2.0), 10.0, look.text_dim, (0.0, 0.0));
        }
        let keeping = Path::rectangle(Point::new(0.0, 0.0), Size::new(size.width, TOP_GAP));
        frame.fill(&keeping, look.panel);
        let cut = self.editor.kept.iter().map(|keep| 20.0 * keep.max(1e-6).log10()).fold(0.0f32, f32::min);
        put_text(&mut frame, &format!("Taking out up to {:.0} dB", -cut), Point::new(size.width / 2.0, 3.0), 11.0, look.text_dim, (0.5, 0.0));
        self.curve(&mut frame, size, &self.editor.heard, look.spectrum, 1.4);
        self.curve(&mut frame, size, &self.editor.room, look.danger, 1.6);
        vec![frame.into_geometry()]
    }
}
