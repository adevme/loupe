use std::sync::Arc;

use iced::widget::canvas::{self, Geometry, Path, Stroke};
use iced::widget::{column, container, row, Space};
use iced::{mouse, Alignment, Element, Length, Point, Rectangle, Renderer, Size, Theme, Vector};
use loupe_stock::{Compressor, History, Moment};

use crate::kit::{choice, dial, graph_frame, knob_row, panel, put_text, toggle};
use crate::knobs::{change, Change, Knobs};
use crate::look::{fade, Look};

const SHOWN_MOMENTS: usize = 700;
const LOWEST_DB: f32 = -60.0;
const HIGHEST_DB: f32 = 6.0;
const DEEPEST_SHOWN: f32 = 30.0;
const TRANSFER_SIZE: f32 = 250.0;
const GAP: f32 = 14.0;

pub struct CompressorEditor {
    knobs: Knobs,
    history: Option<Arc<History>>,
    moments: Vec<Moment>,
    look: Look,
}

impl CompressorEditor {
    pub fn new(history: Option<Arc<History>>, look: Look) -> Self {
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
        let knob = |index: usize, colour| dial(look, params, values, index, colour, change);
        let shape = knob_row(vec![
            knob(0, look.accent),
            knob(1, look.accent),
            knob(4, look.accent),
            knob(5, look.accent),
            knob(2, look.bands[4]),
            knob(3, look.bands[4]),
        ]);
        let level = knob_row(vec![knob(6, look.bands[3]), knob(8, look.bands[3]), knob(11, look.bands[3]), knob(10, look.bands[6])]);
        let extras = column![
            choice(params[9], values[9], |value| Change(9, value)),
            toggle(look, "Auto gain", self.knobs.on(7), Change(7, if self.knobs.on(7) { 0.0 } else { 1.0 })),
        ]
        .spacing(10);
        let controls = row![shape, Space::with_width(18), level, Space::with_width(Length::Fill), extras].align_y(Alignment::Center);
        let graphs = container(canvas::Canvas::new(Scroll { editor: self }).width(Length::Fill).height(Length::Fill)).padding([12, 12]);
        let backdrop = container(graphs).width(Length::Fill).height(Length::Fill).style(move |_| container::Style {
            background: Some(look.background.into()),
            ..Default::default()
        });
        column![backdrop, panel(look, controls)].into()
    }
}

fn draw_transfer(frame: &mut canvas::Frame, size: Size, editor: &CompressorEditor) {
        let look = editor.look;
        frame.fill_rectangle(Point::ORIGIN, size, look.panel);
        let at = |input: f32, output: f32| {
            let share = |db: f32| ((db - LOWEST_DB) / -LOWEST_DB).clamp(0.0, 1.0);
            Point::new(share(input) * size.width, size.height - share(output) * size.height)
        };
        for db in [-48.0, -36.0, -24.0, -12.0] {
            let p = at(db, db);
            frame.fill_rectangle(Point::new(p.x, 0.0), Size::new(1.0, size.height), look.grid);
            frame.fill_rectangle(Point::new(0.0, p.y), Size::new(size.width, 1.0), look.grid);
        }
        frame.stroke(&Path::line(at(LOWEST_DB, LOWEST_DB), at(0.0, 0.0)), Stroke::default().with_color(look.grid_strong).with_width(1.0));
        let curve = Compressor::curve_of(&editor.knobs.values);
        let makeup = editor.knobs.values[6];
        let line = Path::new(|b| {
            for step in 0..=120 {
                let input = LOWEST_DB + -LOWEST_DB * step as f32 / 120.0;
                let point = at(input, input + curve.reduction(input) + makeup);
                if step == 0 {
                    b.move_to(point);
                } else {
                    b.line_to(point);
                }
            }
        });
        frame.stroke(&line, Stroke::default().with_color(look.accent).with_width(2.0));
        let threshold = at(curve.threshold, curve.threshold);
        frame.fill_rectangle(Point::new(threshold.x, 0.0), Size::new(1.0, size.height), fade(look.accent, 0.3));
        if let Some(last) = editor.moments.last().filter(|moment| moment.input > 1e-4) {
            let input = loupe_stock_db(last.input);
            let dot = at(input, input + curve.reduction(input) + makeup);
            frame.fill(&Path::circle(dot, 5.0), look.text);
        }
        put_text(frame, "In", Point::new(size.width - 6.0, size.height - 6.0), 11.0, look.text_dim, (1.0, 1.0));
        put_text(frame, "Out", Point::new(6.0, 6.0), 11.0, look.text_dim, (0.0, 0.0));
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
        let side = TRANSFER_SIZE.min(size.height - 24.0).max(0.0);
        frame.with_save(|frame| {
            frame.translate(Vector::new(0.0, (size.height - side) / 2.0));
            draw_transfer(frame, Size::new(side, side), self.editor);
        });
        frame.with_save(|frame| {
            frame.translate(Vector::new(side + GAP, 0.0));
            let rest = Size::new((size.width - side - GAP).max(1.0), size.height);
            draw_levels(frame, rest, &look, &self.editor.moments, Some(self.editor.knobs.values[0]), DEEPEST_SHOWN);
        });
        vec![frame.into_geometry()]
    }
}

pub(crate) fn loupe_stock_db(gain: f32) -> f32 {
    20.0 * gain.max(1e-6).log10()
}

pub(crate) fn draw_levels(frame: &mut canvas::Frame, size: Size, look: &Look, moments: &[Moment], line_at: Option<f32>, deepest: f32) {
    let level_y = |gain: f32| {
        let db = loupe_stock_db(gain).clamp(LOWEST_DB, HIGHEST_DB);
        size.height - (db - LOWEST_DB) / (HIGHEST_DB - LOWEST_DB) * size.height
    };
    for db in [0.0, -6.0, -12.0, -24.0, -36.0, -48.0] {
        let y = level_y(10f32.powf(db / 20.0)).round();
        frame.fill_rectangle(Point::new(0.0, y), Size::new(size.width, 1.0), look.grid);
        put_text(frame, &format!("{db:.0}"), Point::new(size.width - 6.0, y - 2.0), 10.5, look.text_dim, (1.0, 1.0));
    }
    let step = size.width / moments.len().max(1) as f32;
    let bars = |frame: &mut canvas::Frame, pick: fn(&Moment) -> f32, colour| {
        for (i, moment) in moments.iter().enumerate() {
            let top = level_y(pick(moment));
            if top < size.height {
                frame.fill_rectangle(Point::new(i as f32 * step, top), Size::new(step + 1.0, size.height - top), colour);
            }
        }
    };
    bars(frame, |moment| moment.input, fade(look.text, 0.12));
    bars(frame, |moment| moment.output, fade(look.bands[5], 0.45));
    if let Some(db) = line_at {
        let y = level_y(10f32.powf(db / 20.0));
        frame.fill_rectangle(Point::new(0.0, y), Size::new(size.width, 1.0), fade(look.accent, 0.6));
    }
    let reduction_y = |db: f32| (-db).clamp(0.0, deepest) / deepest * size.height * 0.5;
    let reduction = Path::new(|b| {
        for (i, moment) in moments.iter().enumerate() {
            let point = Point::new(i as f32 * step, reduction_y(moment.reduction));
            if i == 0 {
                b.move_to(point);
            } else {
                b.line_to(point);
            }
        }
    });
    frame.stroke(&reduction, Stroke::default().with_color(look.danger).with_width(1.5));
    if let Some(now) = moments.last() {
        let text = format!("{:.1} dB", now.reduction.min(0.0));
        put_text(frame, &text, Point::new(12.0, 10.0), 13.0, look.danger, (0.0, 0.0));
    }
}
