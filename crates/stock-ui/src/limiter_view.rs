use std::sync::Arc;

use iced::widget::canvas::{self, Geometry};
use iced::{mouse, Element, Length, Rectangle, Renderer, Size, Theme, Vector};
use loupe_stock::{History, Limiter, Moment};

use crate::compressor_view::{draw_levels, loupe_stock_db, GAP, INSET};
use crate::kit::*;
use crate::knobs::{change, Change, Knobs};
use crate::look::Look;

const SHOWN_MOMENTS: usize = 900;
const DEEPEST_SHOWN: f32 = 12.0;
const RECENT_MOMENTS: usize = 60;
const READOUT_WIDTH: f32 = 150.0;

pub struct LimiterEditor {
    knobs: Knobs,
    history: Option<Arc<History>>,
    moments: Vec<Moment>,
    look: Look,
}

impl LimiterEditor {
    pub fn new(history: Option<Arc<History>>, look: Look) -> Self {
        let look = look.tinted(crate::look::accents::LIMITER);
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
        let true_peak = self.knobs.on(3);
        shell(look, "Limiter", "Mastering limiter", move || canvas::Canvas::new(Scroll { editor: self }).width(Length::Fill).height(Length::Fill).into(), move |compact| {
            let p = Panel::new(look, compact, params, values, change);
            vec![vec![
                section(look, "Limit", vec![p.hero(0), p.hero(1)]),
                section(look, "Recovery", vec![p.knob(2)]),
                section(look, "Detection", vec![toggle(look, "True peak", true_peak, Change(3, if true_peak { 0.0 } else { 1.0 }))]),
            ]]
        })
    }
}

struct Scroll<'a> {
    editor: &'a LimiterEditor,
}

impl canvas::Program<Change> for Scroll<'_> {
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
            draw_levels(frame, rest, &look, &editor.moments, Some(editor.knobs.values[1]), DEEPEST_SHOWN);
        });
        frame.with_save(|frame| {
            frame.translate(Vector::new(size.width - READOUT_WIDTH - INSET, INSET));
            let recent = &editor.moments[editor.moments.len().saturating_sub(RECENT_MOMENTS)..];
            let deepest = recent.iter().map(|moment| moment.reduction).fold(0.0f32, f32::min);
            let loudest = recent.iter().map(|moment| moment.output).fold(0.0f32, f32::max);
            let peak = if loudest > 0.0 { format!("{:.1}", loupe_stock_db(loudest)) } else { "-inf".to_string() };
            let main = Reading::new("Reduction", format!("{deepest:.1}"), "dB", look.danger);
            let rest = [Reading::new("Output peak", peak, "dB", look.text), Reading::new("Ceiling", format!("{:.1}", editor.knobs.values[1]), "dB", look.text_dim)];
            draw_readouts(frame, Size::new(READOUT_WIDTH, size.height - INSET * 2.0), &look, &main, Some((-deepest / DEEPEST_SHOWN, look.danger)), &rest);
        });
        seal(&mut frame, size, &look);
        vec![frame.into_geometry()]
    }
}
