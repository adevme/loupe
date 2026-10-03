use std::sync::Arc;

use iced::widget::canvas::{self, Geometry};
use iced::widget::{column, container, row, Space};
use iced::{mouse, Alignment, Element, Length, Rectangle, Renderer, Size, Theme, Vector};
use loupe_stock::{History, Limiter, Moment};

use crate::compressor_view::{draw_levels, loupe_stock_db};
use crate::kit::{dial, graph_frame, knob_row, panel, put_text, toggle};
use crate::knobs::{change, Change, Knobs};
use crate::look::Look;

const SHOWN_MOMENTS: usize = 900;
const DEEPEST_SHOWN: f32 = 12.0;
const RECENT_MOMENTS: usize = 60;
const READOUT_WIDTH: f32 = 150.0;
const GAP: f32 = 14.0;

pub struct LimiterEditor {
    knobs: Knobs,
    history: Option<Arc<History>>,
    moments: Vec<Moment>,
    look: Look,
}

impl LimiterEditor {
    pub fn new(history: Option<Arc<History>>, look: Look) -> Self {
        Self { knobs: Knobs::of(&Limiter::new()), history, moments: vec![Moment::default(); SHOWN_MOMENTS], look }
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
        let knobs = knob_row(vec![
            dial(look, params, values, 0, look.accent, change),
            dial(look, params, values, 1, look.bands[5], change),
            dial(look, params, values, 2, look.bands[4], change),
        ]);
        let true_peak = self.knobs.on(3);
        let controls = row![knobs, Space::with_width(18), toggle(look, "True peak", true_peak, Change(3, if true_peak { 0.0 } else { 1.0 })), Space::with_width(Length::Fill)]
            .align_y(Alignment::Center);
        let graph = canvas::Canvas::new(Scroll { editor: self }).width(Length::Fill).height(Length::Fill);
        let backdrop = container(container(graph).padding([12, 12])).width(Length::Fill).height(Length::Fill).style(move |_| {
            container::Style { background: Some(look.background.into()), ..Default::default() }
        });
        column![backdrop, panel(look, controls)].into()
    }
}

struct Scroll<'a> {
    editor: &'a LimiterEditor,
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
        let rest = Size::new((size.width - READOUT_WIDTH - GAP).max(1.0), size.height);
        draw_levels(&mut frame, rest, &look, &self.editor.moments, Some(self.editor.knobs.values[1]), DEEPEST_SHOWN);
        frame.with_save(|frame| {
            frame.translate(Vector::new(rest.width + GAP, 0.0));
            draw_readout(frame, Size::new(READOUT_WIDTH, size.height), self.editor);
        });
        vec![frame.into_geometry()]
    }
}

fn draw_readout(frame: &mut canvas::Frame, size: Size, editor: &LimiterEditor) {
        let look = editor.look;
        frame.fill_rectangle(iced::Point::ORIGIN, size, look.panel);
        let recent = &editor.moments[editor.moments.len().saturating_sub(RECENT_MOMENTS)..];
        let deepest = recent.iter().map(|moment| moment.reduction).fold(0.0f32, f32::min);
        let loudest = recent.iter().map(|moment| moment.output).fold(0.0f32, f32::max);
        let middle = size.width / 2.0;
        put_text(frame, "Reduction", iced::Point::new(middle, 18.0), 11.5, look.text_dim, (0.5, 0.0));
        put_text(frame, &format!("{deepest:.1}"), iced::Point::new(middle, 36.0), 30.0, look.danger, (0.5, 0.0));
        put_text(frame, "dB", iced::Point::new(middle, 74.0), 11.5, look.text_dim, (0.5, 0.0));
        put_text(frame, "Output peak", iced::Point::new(middle, 110.0), 11.5, look.text_dim, (0.5, 0.0));
        let peak = if loudest > 0.0 { format!("{:.1}", loupe_stock_db(loudest)) } else { "-inf".to_string() };
        put_text(frame, &peak, iced::Point::new(middle, 128.0), 24.0, look.text, (0.5, 0.0));
        put_text(frame, "dB", iced::Point::new(middle, 158.0), 11.5, look.text_dim, (0.5, 0.0));
}
