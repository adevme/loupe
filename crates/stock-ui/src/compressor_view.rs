use std::sync::Arc;

use iced::widget::canvas::{self, Geometry, Path, Stroke};
use iced::{mouse, Element, Length, Point, Rectangle, Renderer, Size, Theme, Vector};
use loupe_stock::{Compressor, History, Moment};

use crate::kit::*;
use crate::knobs::{change, Change, Knobs};
use crate::look::{fade, Look};

const SHOWN_MOMENTS: usize = 700;
const LOWEST_DB: f32 = -60.0;
const HIGHEST_DB: f32 = 6.0;
const DEEPEST_SHOWN: f32 = 30.0;
const TRANSFER_SIZE: f32 = 260.0;
const READOUT_WIDTH: f32 = 132.0;
const RECENT_MOMENTS: usize = 30;
pub(crate) const INSET: f32 = 10.0;
pub(crate) const GAP: f32 = 10.0;

pub struct CompressorEditor {
    knobs: Knobs,
    history: Option<Arc<History>>,
    moments: Vec<Moment>,
    look: Look,
}

impl CompressorEditor {
    pub fn new(history: Option<Arc<History>>, look: Look) -> Self {
        let look = look.tinted(crate::look::accents::COMPRESSOR);
        Self { knobs: Knobs::of(&Compressor::new()), history, moments: vec![Moment::default(); SHOWN_MOMENTS], look }
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
        let auto = self.knobs.on(7);
        shell(look, "Compressor", "Dynamics", move || canvas::Canvas::new(Scroll { editor: self }).width(Length::Fill).height(Length::Fill).into(), move |compact| {
            let p = Panel::new(look, compact, params, values, change);
            vec![vec![
                section(look, "Compression", vec![p.hero(0), p.hero(1)]),
                section(look, "Envelope", vec![p.knob(2), p.knob(3), p.knob(4), p.knob(5)]),
                section(look, "Character", vec![stack(vec![choice(look, params[9], values[9], |value| Change(9, value)), toggle(look, "Auto gain", auto, Change(7, if auto { 0.0 } else { 1.0 }))])]),
                section(look, "Output", vec![p.named(10, "SC low cut"), p.knob(6), p.knob(8), p.knob(11)]),
            ]]
        })
    }
}

fn draw_transfer(frame: &mut canvas::Frame, size: Size, editor: &CompressorEditor) {
    let look = editor.look;
    draw_card(frame, Point::ORIGIN, size, &look);
    let pad = 12.0;
    let inner = Size::new(size.width - pad * 2.0, size.height - pad * 2.0);
    let at = |input: f32, output: f32| {
        let share = |db: f32| ((db - LOWEST_DB) / -LOWEST_DB).clamp(0.0, 1.0);
        Point::new(pad + share(input) * inner.width, pad + inner.height - share(output) * inner.height)
    };
    for db in [-48.0, -36.0, -24.0, -12.0] {
        let p = at(db, db);
        frame.fill_rectangle(Point::new(p.x.round(), pad), Size::new(1.0, inner.height), look.grid);
        frame.fill_rectangle(Point::new(pad, p.y.round()), Size::new(inner.width, 1.0), look.grid);
    }
    frame.stroke(&Path::line(at(LOWEST_DB, LOWEST_DB), at(0.0, 0.0)), Stroke::default().with_color(look.grid_strong).with_width(1.0));
    let curve = Compressor::curve_of(&editor.knobs.values);
    let makeup = editor.knobs.values[6];
    let points: Vec<Point> = (0..=120)
        .map(|step| {
            let input = LOWEST_DB + -LOWEST_DB * step as f32 / 120.0;
            at(input, input + curve.reduction(input) + makeup)
        })
        .collect();
    fill_under(frame, &points, pad + inner.height, fade(look.accent, 0.16), fade(look.accent, 0.0));
    let line = polyline(&points);
    frame.stroke(&line, Stroke::default().with_color(fade(look.accent, 0.18)).with_width(6.0));
    frame.stroke(&line, Stroke::default().with_color(look.accent).with_width(2.0));
    let threshold = at(curve.threshold, curve.threshold);
    frame.fill_rectangle(Point::new(threshold.x.round(), pad), Size::new(1.0, inner.height), fade(look.accent, 0.3));
    if let Some(last) = editor.moments.last().filter(|moment| moment.input > 1e-4) {
        let input = loupe_stock_db(last.input);
        let dot = at(input, input + curve.reduction(input) + makeup);
        frame.fill(&Path::circle(dot, 7.0), fade(look.text, 0.15));
        frame.fill(&Path::circle(dot, 4.0), look.text);
    }
    axis_text(frame, "IN", Point::new(size.width - pad - 2.0, size.height - pad - 2.0), &look, (1.0, 1.0));
    axis_text(frame, "OUT", Point::new(pad + 2.0, pad + 2.0), &look, (0.0, 0.0));
}

struct Scroll<'a> {
    editor: &'a CompressorEditor,
}

impl canvas::Program<Change> for Scroll<'_> {
    type State = ();

    fn draw(&self, _state: &(), renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let look = self.editor.look;
        let size = bounds.size();
        if !crate::kit::roomy(size) {
            return Vec::new();
        }
        let mut frame = graph_frame(renderer, size, &look);
        let side = TRANSFER_SIZE.min(size.height - INSET * 2.0).max(0.0);
        frame.with_save(|frame| {
            frame.translate(Vector::new(INSET, INSET));
            draw_transfer(frame, Size::new(side, side), self.editor);
        });
        let left = INSET + side + GAP;
        let levels = Size::new((size.width - left - READOUT_WIDTH - GAP - INSET).max(1.0), size.height);
        frame.with_save(|frame| {
            frame.translate(Vector::new(left, 0.0));
            draw_levels(frame, levels, &look, &self.editor.moments, Some(self.editor.knobs.values[0]), DEEPEST_SHOWN);
        });
        frame.with_save(|frame| {
            frame.translate(Vector::new(size.width - READOUT_WIDTH - INSET, INSET));
            let recent = &self.editor.moments[self.editor.moments.len().saturating_sub(RECENT_MOMENTS)..];
            let deepest = recent.iter().map(|moment| moment.reduction).fold(0.0f32, f32::min);
            let main = Reading::new("Reduction", format!("{deepest:.1}"), "dB", look.danger);
            let curve = Compressor::curve_of(&self.editor.knobs.values);
            let rest = [Reading::new("Threshold", format!("{:.1}", curve.threshold), "dB", look.text)];
            draw_readouts(frame, Size::new(READOUT_WIDTH, size.height - INSET * 2.0), &look, &main, Some((-deepest / DEEPEST_SHOWN, look.danger)), &rest);
        });
        seal(&mut frame, size, &look);
        vec![frame.into_geometry()]
    }
}

pub(crate) fn loupe_stock_db(gain: f32) -> f32 {
    20.0 * gain.max(1e-6).log10()
}

pub(crate) fn draw_levels(frame: &mut canvas::Frame, size: Size, look: &Look, moments: &[Moment], line_at: Option<f32>, deepest: f32) {
    let top = INSET + 4.0;
    let bottom = size.height - INSET - 4.0;
    let tall = (bottom - top).max(1.0);
    let label_room = 28.0;
    let width = (size.width - label_room).max(1.0);
    let level_y = |gain: f32| {
        let db = loupe_stock_db(gain).clamp(LOWEST_DB, HIGHEST_DB);
        bottom - (db - LOWEST_DB) / (HIGHEST_DB - LOWEST_DB) * tall
    };
    for db in [0.0, -6.0, -12.0, -24.0, -36.0, -48.0] {
        let y = level_y(10f32.powf(db / 20.0));
        guide(frame, Point::new(0.0, y), width, if db == 0.0 { look.grid_strong } else { look.grid });
        axis_text(frame, &format!("{db:.0}"), Point::new(size.width - 2.0, y), look, (1.0, 0.5));
    }
    let step = width / (moments.len().max(2) - 1) as f32;
    let trace = |pick: fn(&Moment) -> f32| -> Vec<Point> { moments.iter().enumerate().map(|(i, moment)| Point::new(i as f32 * step, level_y(pick(moment)))).collect() };
    let input = trace(|moment| moment.input);
    fill_under(frame, &input, bottom, fade(look.text, 0.06), fade(look.text, 0.01));
    let output = trace(|moment| moment.output);
    let blue = look.accent;
    fill_under(frame, &output, bottom, fade(blue, 0.38), fade(blue, 0.05));
    frame.stroke(&polyline(&output), Stroke::default().with_color(fade(blue, 0.9)).with_width(1.2));
    if let Some(db) = line_at {
        let y = level_y(10f32.powf(db / 20.0));
        dashed(frame, Point::new(0.0, y), width, fade(look.accent, 0.75));
    }
    let reduction_y = |db: f32| top + (-db).clamp(0.0, deepest) / deepest * tall * 0.5;
    let reduction: Vec<Point> = moments.iter().enumerate().map(|(i, moment)| Point::new(i as f32 * step, reduction_y(moment.reduction))).collect();
    fill_under(frame, &reduction, top, fade(look.danger, 0.3), fade(look.danger, 0.04));
    frame.stroke(&polyline(&reduction), Stroke::default().with_color(look.danger).with_width(1.6));
}
