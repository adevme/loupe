use std::sync::Arc;

use iced::widget::canvas::{self, Geometry, Stroke};
use iced::{mouse, Element, Length, Point, Rectangle, Renderer, Theme};
use loupe_stock::{History, Moment, Transient};

use crate::compressor_view::INSET;
use crate::kit::*;
use crate::knobs::{change, Change, Knobs};
use crate::look::{fade, Look, MEDIUM};

const SHOWN_MOMENTS: usize = 600;
const LOWEST_DB: f32 = -48.0;
const WIDEST_CHANGE_DB: f32 = 15.0;

pub struct TransientEditor {
    knobs: Knobs,
    history: Option<Arc<History>>,
    moments: Vec<Moment>,
    look: Look,
}

impl TransientEditor {
    pub fn new(history: Option<Arc<History>>, look: Look) -> Self {
        let look = look.tinted(crate::look::accents::TRANSIENT);
        Self { knobs: Knobs::of(&Transient::new()), history, moments: vec![Moment::default(); SHOWN_MOMENTS], look }
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
        if let Some(history) = &self.history {
            history.latest(&mut self.moments);
        }
    }

    pub fn view(&self) -> Element<'_, Change> {
        let look = self.look;
        let values = &self.knobs.values;
        let params = self.knobs.params;
        shell(look, "Transient", "Transient shaper", move || canvas::Canvas::new(Hits { editor: self }).width(Length::Fill).height(Length::Fill).into(), move |compact| {
            let p = Panel::new(look, compact, params, values, change);
            vec![vec![
                section(look, "Shape", vec![p.hero(0), p.hero(1)]),
                section(look, "Output", vec![p.knob(2), p.knob(3)]),
            ]]
        })
    }
}

struct Hits<'a> {
    editor: &'a TransientEditor,
}

impl canvas::Program<Change> for Hits<'_> {
    type State = ();

    fn draw(&self, _state: &(), renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let look = self.editor.look;
        let size = bounds.size();
        if !crate::kit::roomy(size) {
            return Vec::new();
        }
        let mut frame = graph_frame(renderer, size, &look);
        let moments = &self.editor.moments;
        let left = INSET;
        let width = (size.width - INSET * 2.0 - 42.0).max(1.0);
        let top = INSET + 8.0;
        let bottom = size.height - INSET - 8.0;
        let middle = (top + bottom) / 2.0;
        let half = (bottom - top) / 2.0;
        let step = width / (moments.len().max(2) - 1) as f32;
        let level_y = |gain: f32| {
            let db = (20.0 * gain.max(1e-6).log10()).clamp(LOWEST_DB, 0.0);
            bottom - (db - LOWEST_DB) / -LOWEST_DB * (bottom - top)
        };
        let input: Vec<Point> = moments.iter().enumerate().map(|(i, moment)| Point::new(left + i as f32 * step, level_y(moment.input))).collect();
        frame.stroke(&polyline(&input), Stroke::default().with_color(fade(look.text, 0.16)).with_width(1.0));
        for db in [-WIDEST_CHANGE_DB, -6.0, 6.0, WIDEST_CHANGE_DB] {
            let y = middle - db / WIDEST_CHANGE_DB * half;
            guide(&mut frame, Point::new(left, y), width, look.grid);
            axis_text(&mut frame, &format!("{db:+.0}"), Point::new(size.width - INSET - 2.0, y), &look, (1.0, 0.5));
        }
        guide(&mut frame, Point::new(left, middle), width, look.grid_strong);
        axis_text(&mut frame, "0", Point::new(size.width - INSET - 2.0, middle), &look, (1.0, 0.5));
        let change_y = |moment: &Moment| middle - moment.reduction.clamp(-WIDEST_CHANGE_DB, WIDEST_CHANGE_DB) / WIDEST_CHANGE_DB * half;
        let up: Vec<Point> = moments.iter().enumerate().map(|(i, moment)| Point::new(left + i as f32 * step, change_y(moment).min(middle))).collect();
        let down: Vec<Point> = moments.iter().enumerate().map(|(i, moment)| Point::new(left + i as f32 * step, change_y(moment).max(middle))).collect();
        fill_under(&mut frame, &up, middle, fade(look.accent, 0.85), fade(look.accent, 0.2));
        fill_under(&mut frame, &down, middle, fade(look.neutral(), 0.6), fade(look.neutral(), 0.1));
        put_text_in(&mut frame, "BOOST", Point::new(left + 8.0, top + 4.0), 9.5, look.accent, (0.0, 0.0), MEDIUM);
        put_text_in(&mut frame, "CUT", Point::new(left + 8.0, bottom - 4.0), 9.5, look.neutral(), (0.0, 1.0), MEDIUM);
        seal(&mut frame, size, &look);
        vec![frame.into_geometry()]
    }
}
