use std::sync::Arc;

use iced::widget::canvas::{self, Geometry, Stroke};
use iced::{mouse, Element, Length, Point, Rectangle, Renderer, Size, Theme, Vector};
use loupe_stock::{History, Moment, Rider};

use crate::compressor_view::{loupe_stock_db, GAP, INSET};
use crate::kit::*;
use crate::knobs::{change, Change, Knobs};
use crate::look::{fade, Look};

const SHOWN_MOMENTS: usize = 900;
const WIDEST_RIDE_DB: f32 = 24.0;
const RECENT_MOMENTS: usize = 60;
const READOUT_WIDTH: f32 = 150.0;

pub struct RiderEditor {
    knobs: Knobs,
    history: Option<Arc<History>>,
    moments: Vec<Moment>,
    look: Look,
}

impl RiderEditor {
    pub fn new(history: Option<Arc<History>>, look: Look) -> Self {
        let look = look.tinted(crate::look::accents::RIDER);
        Self { knobs: Knobs::of(&Rider::new()), history, moments: vec![Moment::default(); SHOWN_MOMENTS], look }
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
        shell(look, "Rider", "Vocal level rider", move || canvas::Canvas::new(Ride { editor: self }).width(Length::Fill).height(Length::Fill).into(), move |compact| {
            let p = Panel::new(look, compact, params, values, change);
            vec![vec![
                section(look, "Aim", vec![p.hero(0), p.knob(1), p.knob(2)]),
                section(look, "Feel", vec![p.knob(3), p.knob(4), p.knob(5)]),
                section(look, "Output", vec![p.knob(6)]),
            ]]
        })
    }
}

struct Ride<'a> {
    editor: &'a RiderEditor,
}

impl canvas::Program<Change> for Ride<'_> {
    type State = ();

    fn draw(&self, _state: &(), renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let editor = self.editor;
        let look = editor.look;
        let size = bounds.size();
        if !crate::kit::roomy(size) {
            return Vec::new();
        }
        let mut frame = graph_frame(renderer, size, &look);
        frame.with_save(|frame| {
            frame.translate(Vector::new(INSET, 0.0));
            let rest = Size::new((size.width - READOUT_WIDTH - GAP - INSET * 2.0).max(1.0), size.height);
            draw_ride(frame, rest, &look, &editor.moments);
        });
        frame.with_save(|frame| {
            frame.translate(Vector::new(size.width - READOUT_WIDTH - INSET, INSET));
            let recent = &editor.moments[editor.moments.len().saturating_sub(RECENT_MOMENTS)..];
            let riding = recent.last().map_or(0.0, |moment| moment.reduction);
            let loudest = recent.iter().map(|moment| moment.output).fold(0.0f32, f32::max);
            let peak = if loudest > 0.0 { format!("{:.1}", loupe_stock_db(loudest)) } else { "-inf".to_string() };
            let colour = if riding >= 0.0 { look.accent } else { look.neutral() };
            let main = Reading::new("Riding", format!("{riding:+.1}"), "dB", colour);
            let rest = [Reading::new("Going out", peak, "dB", look.text), Reading::new("Target", format!("{:.0}", editor.knobs.values[0]), "dB", look.text_dim)];
            draw_readouts(frame, Size::new(READOUT_WIDTH, size.height - INSET * 2.0), &look, &main, Some((riding.abs() / WIDEST_RIDE_DB, colour)), &rest);
        });
        seal(&mut frame, size, &look);
        vec![frame.into_geometry()]
    }
}

fn draw_ride(frame: &mut canvas::Frame, size: Size, look: &Look, moments: &[Moment]) {
    let label_room = 30.0;
    let width = (size.width - label_room).max(1.0);
    let middle = size.height / 2.0;
    let per_db = (middle - INSET - 6.0) / WIDEST_RIDE_DB;
    for step in 1..=4 {
        let away = step as f32 * 6.0 * per_db;
        for (y, db) in [(middle - away, step as f32 * 6.0), (middle + away, -(step as f32) * 6.0)] {
            guide(frame, Point::new(0.0, y), width, look.grid);
            if step % 2 == 0 {
                axis_text(frame, &format!("{db:+.0}"), Point::new(size.width - 2.0, y), look, (1.0, 0.5));
            }
        }
    }
    guide(frame, Point::new(0.0, middle), width, look.grid_strong);
    axis_text(frame, "0", Point::new(size.width - 2.0, middle), look, (1.0, 0.5));
    let step = width / (moments.len().max(2) - 1) as f32;
    let points: Vec<Point> = moments
        .iter()
        .enumerate()
        .map(|(at, moment)| Point::new(at as f32 * step, middle - moment.reduction.clamp(-WIDEST_RIDE_DB, WIDEST_RIDE_DB) * per_db))
        .collect();
    let above: Vec<Point> = points.iter().map(|p| Point::new(p.x, p.y.min(middle))).collect();
    let below: Vec<Point> = points.iter().map(|p| Point::new(p.x, p.y.max(middle))).collect();
    fill_under(frame, &above, middle, fade(look.accent, 0.45), fade(look.accent, 0.08));
    fill_under(frame, &below, middle, fade(look.neutral(), 0.45), fade(look.neutral(), 0.08));
    frame.stroke(&polyline(&points), Stroke::default().with_color(fade(look.text, 0.85)).with_width(1.4));
}
