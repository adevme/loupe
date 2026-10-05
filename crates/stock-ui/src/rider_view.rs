use std::sync::Arc;

use iced::widget::canvas::{self, Geometry};
use iced::widget::{column, container, row, Space};
use iced::{mouse, Alignment, Element, Length, Rectangle, Renderer, Size, Theme, Vector};
use loupe_stock::{History, Moment, Rider};

use crate::compressor_view::loupe_stock_db;
use crate::kit::{dial, graph_frame, knob_row, panel, put_text};
use crate::knobs::{change, Change, Knobs};
use crate::look::Look;

const SHOWN_MOMENTS: usize = 900;
const WIDEST_RIDE_DB: f32 = 24.0;
const RECENT_MOMENTS: usize = 60;
const READOUT_WIDTH: f32 = 150.0;
const GAP: f32 = 14.0;

pub struct RiderEditor {
    knobs: Knobs,
    history: Option<Arc<History>>,
    moments: Vec<Moment>,
    look: Look,
}

impl RiderEditor {
    pub fn new(history: Option<Arc<History>>, look: Look) -> Self {
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
        let aim = knob_row(vec![
            dial(look, params, values, 0, look.accent, change),
            dial(look, params, values, 1, look.bands[5], change),
            dial(look, params, values, 2, look.bands[6], change),
        ]);
        let feel = knob_row(vec![
            dial(look, params, values, 3, look.bands[3], change),
            dial(look, params, values, 4, look.bands[3], change),
            dial(look, params, values, 5, look.bands[2], change),
            dial(look, params, values, 6, look.bands[1], change),
        ]);
        let controls = row![aim, Space::with_width(18), feel, Space::with_width(Length::Fill)].align_y(Alignment::Center);
        let graph = canvas::Canvas::new(Ride { editor: self }).width(Length::Fill).height(Length::Fill);
        let backdrop = container(container(graph).padding([12, 12])).width(Length::Fill).height(Length::Fill).style(move |_| {
            container::Style { background: Some(look.background.into()), ..Default::default() }
        });
        column![backdrop, panel(look, controls)].into()
    }
}

struct Ride<'a> {
    editor: &'a RiderEditor,
}

impl canvas::Program<Change> for Ride<'_> {
    type State = ();

    fn draw(&self, _state: &(), renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let look = self.editor.look;
        let size = bounds.size();
        if !crate::kit::roomy(size) {
            return Vec::new();
        }
        let mut frame = graph_frame(renderer, size, &look);
        let rest = Size::new((size.width - READOUT_WIDTH - GAP).max(1.0), size.height);
        draw_ride(&mut frame, rest, &look, &self.editor.moments);
        frame.with_save(|frame| {
            frame.translate(Vector::new(rest.width + GAP, 0.0));
            draw_readout(frame, Size::new(READOUT_WIDTH, size.height), self.editor);
        });
        vec![frame.into_geometry()]
    }
}

fn draw_ride(frame: &mut canvas::Frame, size: Size, look: &Look, moments: &[Moment]) {
    let middle = size.height / 2.0;
    let per_db = middle / WIDEST_RIDE_DB;
    frame.fill_rectangle(iced::Point::new(0.0, middle - 0.5), Size::new(size.width, 1.0), look.grid_strong);
    for step in 1..4 {
        let away = step as f32 * 6.0 * per_db;
        for side in [middle - away, middle + away] {
            frame.fill_rectangle(iced::Point::new(0.0, side), Size::new(size.width, 1.0), look.grid);
        }
    }
    put_text(frame, "+24", iced::Point::new(4.0, 2.0), 10.0, look.text_dim, (0.0, 0.0));
    put_text(frame, "-24", iced::Point::new(4.0, size.height - 14.0), 10.0, look.text_dim, (0.0, 0.0));
    let step = size.width / moments.len().max(1) as f32;
    for (at, moment) in moments.iter().enumerate() {
        let lift = moment.reduction.clamp(-WIDEST_RIDE_DB, WIDEST_RIDE_DB);
        if lift.abs() < 0.05 {
            continue;
        }
        let high = middle - lift * per_db;
        let (top, tall) = if lift > 0.0 { (high, lift * per_db) } else { (middle, -lift * per_db) };
        let colour = if lift > 0.0 { look.accent } else { look.bands[5] };
        frame.fill_rectangle(iced::Point::new(at as f32 * step, top), Size::new(step.max(1.0), tall.max(1.0)), colour);
    }
}

fn draw_readout(frame: &mut canvas::Frame, size: Size, editor: &RiderEditor) {
    let look = editor.look;
    frame.fill_rectangle(iced::Point::ORIGIN, size, look.panel);
    let recent = &editor.moments[editor.moments.len().saturating_sub(RECENT_MOMENTS)..];
    let riding = recent.last().map_or(0.0, |moment| moment.reduction);
    let loudest = recent.iter().map(|moment| moment.output).fold(0.0f32, f32::max);
    let middle = size.width / 2.0;
    put_text(frame, "Riding", iced::Point::new(middle, 18.0), 11.5, look.text_dim, (0.5, 0.0));
    let colour = if riding >= 0.0 { look.accent } else { look.bands[5] };
    put_text(frame, &format!("{riding:+.1}"), iced::Point::new(middle, 36.0), 30.0, colour, (0.5, 0.0));
    put_text(frame, "dB", iced::Point::new(middle, 74.0), 11.5, look.text_dim, (0.5, 0.0));
    put_text(frame, "Going out", iced::Point::new(middle, 110.0), 11.5, look.text_dim, (0.5, 0.0));
    let peak = if loudest > 0.0 { format!("{:.1}", loupe_stock_db(loudest)) } else { "-inf".to_string() };
    put_text(frame, &peak, iced::Point::new(middle, 128.0), 24.0, look.text, (0.5, 0.0));
    put_text(frame, "dB", iced::Point::new(middle, 158.0), 11.5, look.text_dim, (0.5, 0.0));
}
