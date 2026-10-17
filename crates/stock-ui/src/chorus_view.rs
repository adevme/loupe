use iced::widget::canvas::{self, Geometry, Stroke};
use iced::{mouse, Element, Length, Point, Rectangle, Renderer, Size, Theme};
use loupe_stock::{chorus_spread, chorus_sweep, Chorus};

use crate::compressor_view::INSET;
use crate::kit::*;
use crate::knobs::{change, Change, Knobs};
use crate::look::{fade, Look, MEDIUM};

const SHOWN_SECONDS: f32 = 2.0;
const STEPS: usize = 400;
const HIGHEST_MS: f32 = 24.0;
const TOP_GAP: f32 = 34.0;
const LABELS_MS: [f32; 4] = [0.0, 5.0, 10.0, 20.0];

pub struct ChorusEditor {
    knobs: Knobs,
    look: Look,
}

impl ChorusEditor {
    pub fn new(look: Look) -> Self {
        let look = look.tinted(crate::look::accents::CHORUS);
        Self { knobs: Knobs::of(&Chorus::new()), look }
    }

    pub fn load(&mut self, values: &[f32]) {
        for (slot, value) in values.iter().enumerate().take(self.knobs.values.len()) {
            self.knobs.values[slot] = *value;
        }
    }

    pub fn update(&mut self, change: Change) -> Vec<(usize, f32)> {
        self.knobs.apply(change)
    }

    pub fn view(&self) -> Element<'_, Change> {
        let look = self.look;
        let values = &self.knobs.values;
        let params = self.knobs.params;
        shell(look, "Chorus", "Modulation", move || canvas::Canvas::new(Sweep { editor: self }).width(Length::Fill).height(Length::Fill).into(), move |compact| {
            let p = Panel::new(look, compact, params, values, change);
            vec![vec![
                section(look, "Mode", vec![pick(look, params[0], values[0], |value| Change(0, value))]),
                section(look, "Motion", vec![p.hero(1), p.hero(2), p.knob(3)]),
                section(look, "Output", vec![p.knob(4), p.hero(5)]),
            ]]
        })
    }
}

struct Sweep<'a> {
    editor: &'a ChorusEditor,
}

impl canvas::Program<Change> for Sweep<'_> {
    type State = ();

    fn draw(&self, _state: &(), renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let look = self.editor.look;
        let size = bounds.size();
        if !crate::kit::roomy(size) {
            return Vec::new();
        }
        let mut frame = graph_frame(renderer, size, &look);
        let values = &self.editor.knobs.values;
        let (base, swing) = chorus_sweep(values);
        let rate = values[1];
        let offset = chorus_spread(values);
        let left = INSET;
        let width = (size.width - INSET * 2.0 - 40.0).max(1.0);
        let bottom = size.height - INSET - 8.0;
        let tall = (bottom - TOP_GAP - INSET).max(1.0);
        let y_of = |ms: f32| bottom - (ms / HIGHEST_MS).clamp(0.0, 1.0) * tall;
        for ms in LABELS_MS {
            let y = y_of(ms);
            guide(&mut frame, Point::new(left, y), width, if ms == 0.0 { look.grid_strong } else { look.grid });
            axis_text(&mut frame, &format!("{ms:.0} ms"), Point::new(size.width - INSET - 2.0, y), &look, (1.0, 0.5));
        }
        for second in 1..SHOWN_SECONDS as usize {
            let x = (left + second as f32 / SHOWN_SECONDS * width).round();
            frame.fill_rectangle(Point::new(x, TOP_GAP + INSET), Size::new(1.0, tall), look.grid);
        }
        for (shift, colour, label, tag_x) in [(0.0, look.accent, "LEFT", 0.0), (offset, look.neutral(), "RIGHT", 52.0)] {
            let points: Vec<Point> = (0..=STEPS)
                .map(|step| {
                    let seconds = SHOWN_SECONDS * step as f32 / STEPS as f32;
                    let angle = std::f32::consts::TAU * rate * seconds + shift;
                    Point::new(left + width * step as f32 / STEPS as f32, y_of(base + swing * 0.5 * (1.0 + angle.sin())))
                })
                .collect();
            let line = polyline(&points);
            frame.stroke(&line, Stroke::default().with_color(fade(colour, 0.15)).with_width(7.0));
            frame.stroke(&line, Stroke::default().with_color(colour).with_width(2.0));
            let x = INSET + 10.0 + tag_x;
            frame.fill(&iced::widget::canvas::Path::circle(Point::new(x + 3.0, INSET + 16.0), 3.0), colour);
            put_text_in(&mut frame, label, Point::new(x + 11.0, INSET + 16.0), 9.5, look.text_dim, (0.0, 0.5), MEDIUM);
        }
        seal(&mut frame, size, &look);
        vec![frame.into_geometry()]
    }
}
