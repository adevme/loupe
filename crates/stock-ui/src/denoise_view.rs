use std::sync::Arc;

use iced::widget::canvas::{self, Geometry, Stroke};
use iced::{mouse, Element, Length, Point, Rectangle, Renderer, Size, Theme};
use loupe_stock::{Denoise, Spectra};

use crate::compressor_view::INSET;
use crate::kit::*;
use crate::knobs::{change, Change, Knobs};
use crate::look::{fade, Look, MEDIUM};

const LOWEST_HZ: f32 = 30.0;
const HIGHEST_HZ: f32 = 20_000.0;
const LOUDEST_DB: f32 = 0.0;
const QUIETEST_DB: f32 = -96.0;
const MARKS: [f32; 6] = [50.0, 200.0, 1_000.0, 3_000.0, 8_000.0, 16_000.0];
const TOP_GAP: f32 = 34.0;
const BOTTOM_GAP: f32 = 24.0;

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
        let look = look.tinted(crate::look::accents::DENOISE);
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
        let following = values[3] >= 0.5;
        shell(look, "De-noise", "Noise reduction", move || canvas::Canvas::new(Bands { editor: self }).width(Length::Fill).height(Length::Fill).into(), move |compact| {
            let p = Panel::new(look, compact, params, values, change);
            vec![vec![
                section(look, "Amount", vec![p.hero(0), p.knob(1), p.knob(2)]),
                section(look, "Shape", vec![p.knob(4), p.knob(5)]),
                section(look, "Learning", vec![toggle(look, "Follow the room", following, Change(3, if following { 0.0 } else { 1.0 }))]),
            ]]
        })
    }
}

struct Plot {
    left: f32,
    width: f32,
    top: f32,
    tall: f32,
}

impl Plot {
    fn across(&self, hz: f32) -> f32 {
        let share = (hz.max(LOWEST_HZ) / LOWEST_HZ).log10() / (HIGHEST_HZ / LOWEST_HZ).log10();
        self.left + share.clamp(0.0, 1.0) * self.width
    }

    fn down(&self, db: f32) -> f32 {
        let share = ((db - LOUDEST_DB) / (QUIETEST_DB - LOUDEST_DB)).clamp(0.0, 1.0);
        self.top + share * self.tall
    }
}

struct Bands<'a> {
    editor: &'a DenoiseEditor,
}

impl Bands<'_> {
    fn points(&self, plot: &Plot, levels: &[f32]) -> Vec<Point> {
        let per_bin = self.editor.rate / 2.0 / (levels.len().max(2) - 1) as f32;
        levels
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(bin, _)| *bin as f32 * per_bin >= LOWEST_HZ)
            .map(|(bin, level)| Point::new(plot.across(bin as f32 * per_bin), plot.down(20.0 * level.max(1e-9).log10())))
            .collect()
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
        let plot = Plot { left: INSET, width: (size.width - INSET * 2.0 - 34.0).max(1.0), top: INSET + TOP_GAP, tall: (size.height - INSET * 2.0 - TOP_GAP - BOTTOM_GAP).max(1.0) };
        for hz in MARKS {
            let x = plot.across(hz).round();
            frame.fill_rectangle(Point::new(x, plot.top), Size::new(1.0, plot.tall), look.grid);
            let said = if hz >= 1000.0 { format!("{:.0}k", hz / 1000.0) } else { format!("{hz:.0}") };
            axis_text(&mut frame, &said, Point::new(x, size.height - INSET - 4.0), &look, (0.5, 1.0));
        }
        for db in [-24.0, -48.0, -72.0] {
            let y = plot.down(db);
            guide(&mut frame, Point::new(plot.left, y), plot.width, look.grid);
            axis_text(&mut frame, &format!("{db:.0}"), Point::new(size.width - INSET - 2.0, y), &look, (1.0, 0.5));
        }
        let cut = self.editor.kept.iter().map(|keep| 20.0 * keep.max(1e-6).log10()).fold(0.0f32, f32::min);
        put_text_in(&mut frame, &format!("TAKING OUT UP TO {:.0} dB", -cut), Point::new(INSET + 10.0, INSET + 12.0), 9.5, look.text_dim, (0.0, 0.0), MEDIUM);
        for (x, colour, label) in [(size.width - INSET - 150.0, look.neutral(), "HEARD"), (size.width - INSET - 80.0, look.danger, "ROOM")] {
            frame.fill(&canvas::Path::circle(Point::new(x, INSET + 17.0), 3.0), colour);
            put_text_in(&mut frame, label, Point::new(x + 8.0, INSET + 17.0), 9.5, look.text_dim, (0.0, 0.5), MEDIUM);
        }
        let floor = plot.top + plot.tall;
        let heard = self.points(&plot, &self.editor.heard);
        fill_under(&mut frame, &heard, floor, fade(look.neutral(), 0.16), fade(look.neutral(), 0.01));
        frame.stroke(&polyline(&heard), Stroke::default().with_color(fade(look.neutral(), 0.9)).with_width(1.3));
        let room = self.points(&plot, &self.editor.room);
        let room_line = polyline(&room);
        frame.stroke(&room_line, Stroke::default().with_color(fade(look.danger, 0.16)).with_width(6.0));
        frame.stroke(&room_line, Stroke::default().with_color(look.danger).with_width(1.6));
        seal(&mut frame, size, &look);
        vec![frame.into_geometry()]
    }
}
