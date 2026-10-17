use std::sync::Arc;

use iced::widget::canvas::{self, Geometry, Path, Stroke};
use iced::{mouse, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};
use loupe_stock::{multiband_knob, History, Moment, Multiband, MULTIBAND_BANDS};

use crate::compressor_view::INSET;
use crate::kit::*;
use crate::knobs::{change, Change, Knobs};
use crate::look::{fade, Look, MEDIUM, SEMIBOLD};

const SHOWN_MOMENTS: usize = 400;
const LOWEST_HZ: f32 = 20.0;
const HIGHEST_HZ: f32 = 20_000.0;
const DEEPEST_SHOWN: f32 = 18.0;
const NAMES: [&str; MULTIBAND_BANDS] = ["LOW", "MID", "HIGH"];
const GROUP_NAMES: [&str; MULTIBAND_BANDS] = ["Low band", "Mid band", "High band"];
const LABEL_TOP: f32 = 14.0;
const TRACE_TOP: f32 = 100.0;

pub struct MultibandEditor {
    knobs: Knobs,
    history: Option<Arc<History>>,
    moments: Vec<Moment>,
    look: Look,
}

impl MultibandEditor {
    pub fn new(history: Option<Arc<History>>, look: Look) -> Self {
        let look = look.tinted(crate::look::accents::MULTIBAND);
        Self { knobs: Knobs::of(&Multiband::new()), history, moments: vec![Moment::default(); SHOWN_MOMENTS], look }
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

    fn colour(&self, band: usize) -> Color {
        [self.look.bands[1], self.look.bands[4], self.look.bands[6]][band]
    }

    pub fn view(&self) -> Element<'_, Change> {
        let look = self.look;
        let values = &self.knobs.values;
        let params = self.knobs.params;
        shell(look, "Multiband", "Three band compressor", move || canvas::Canvas::new(Bands { editor: self }).width(Length::Fill).height(Length::Fill).into(), move |compact| {
            let p = Panel::new(look, compact, params, values, change);
            let labels = ["Threshold", "Ratio", "Gain"];
            let bands: Vec<Part<'_, Change>> = (0..MULTIBAND_BANDS).map(|band| section(look, GROUP_NAMES[band], (0..3).map(|which| p.named(multiband_knob(band, which), labels[which])).collect())).collect();
            vec![
                bands,
                vec![
                    section(look, "Crossover", vec![p.knob(0), p.knob(1)]),
                    section(look, "Envelope", vec![p.knob(11), p.knob(12)]),
                    section(look, "Solo", vec![pick(look, params[15], values[15], |value| Change(15, value))]),
                    section(look, "Output", vec![p.knob(13), p.knob(14)]),
                ],
            ]
        })
    }
}

struct Bands<'a> {
    editor: &'a MultibandEditor,
}

fn x_of(hz: f32, width: f32) -> f32 {
    (hz / LOWEST_HZ).ln() / (HIGHEST_HZ / LOWEST_HZ).ln() * width
}

fn band_of(moment: &Moment, band: usize) -> f32 {
    [moment.input, moment.output, moment.reduction][band]
}

impl canvas::Program<Change> for Bands<'_> {
    type State = ();

    fn draw(&self, _state: &(), renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let editor = self.editor;
        let look = editor.look;
        let size = bounds.size();
        if !crate::kit::roomy(size) {
            return Vec::new();
        }
        let mut frame = graph_frame(renderer, size, &look);
        let values = &editor.knobs.values;
        let edges = [0.0, x_of(values[0], size.width), x_of(values[1], size.width), size.width];
        let soloed = values[15].round() as usize;
        let bottom = size.height - INSET - 20.0;
        let trace_h = (bottom - TRACE_TOP).max(1.0);
        for band in 0..MULTIBAND_BANDS {
            let (left, right) = (edges[band], edges[band + 1]);
            let colour = editor.colour(band);
            let lit = soloed == 0 || soloed == band + 1;
            let cap = Path::rounded_rectangle(Point::new(left + 14.0, 6.0), Size::new((right - left - 28.0).max(1.0), 3.0), 1.5.into());
            frame.fill(&cap, fade(colour, if lit { 0.85 } else { 0.25 }));
            let recent = editor.moments.iter().rev().take(20).map(|moment| band_of(moment, band)).fold(0.0f32, f32::min);
            let middle = (left + right) / 2.0;
            put_text_in(&mut frame, NAMES[band], Point::new(middle, LABEL_TOP), 9.5, if lit { colour } else { fade(look.text_dim, 0.5) }, (0.5, 0.0), SEMIBOLD);
            put_text_in(&mut frame, &format!("{recent:.1} dB"), Point::new(middle, LABEL_TOP + 18.0), 26.0, if lit { look.text } else { look.text_faint }, (0.5, 0.0), MEDIUM);
            let detail = format!("{:.0} dB   {:.1}:1   {:+.1} dB", values[multiband_knob(band, 0)], values[multiband_knob(band, 1)], values[multiband_knob(band, 2)]);
            axis_text(&mut frame, &detail, Point::new(middle, LABEL_TOP + 56.0), &look, (0.5, 0.0));
            let inset = 14.0;
            let width = (right - left - inset * 2.0).max(1.0);
            let count = editor.moments.len().max(2);
            let points: Vec<Point> = editor
                .moments
                .iter()
                .enumerate()
                .map(|(i, moment)| {
                    let depth = (-band_of(moment, band)).clamp(0.0, DEEPEST_SHOWN) / DEEPEST_SHOWN;
                    Point::new(left + inset + width * i as f32 / (count - 1) as f32, TRACE_TOP + depth * trace_h)
                })
                .collect();
            guide(&mut frame, Point::new(left + inset, TRACE_TOP), width, look.grid_strong);
            guide(&mut frame, Point::new(left + inset, TRACE_TOP + trace_h / 2.0), width, look.grid);
            fill_under(&mut frame, &points, TRACE_TOP, fade(colour, if lit { 0.3 } else { 0.08 }), fade(colour, 0.02));
            frame.stroke(&polyline(&points), Stroke::default().with_color(if lit { colour } else { fade(colour, 0.3) }).with_width(1.8));
        }
        for (index, hz) in [values[0], values[1]].into_iter().enumerate() {
            let x = edges[index + 1].round();
            frame.fill_rectangle(Point::new(x - 1.0, 0.0), Size::new(3.0, size.height), Color::from_rgba(0.0, 0.0, 0.0, 0.35));
            frame.fill_rectangle(Point::new(x, 0.0), Size::new(1.0, size.height), look.grid_strong);
            let tag = hertz(hz);
            let tag_w = tag.chars().count() as f32 * 6.4 + 14.0;
            let pill = Path::rounded_rectangle(Point::new(x - tag_w / 2.0, size.height - INSET - 18.0), Size::new(tag_w, 18.0), 5.0.into());
            frame.fill(&pill, look.raised);
            frame.stroke(&pill, Stroke::default().with_color(look.grid_strong).with_width(1.0));
            put_text_in(&mut frame, &tag, Point::new(x, size.height - INSET - 9.0), 10.0, look.text, (0.5, 0.5), MEDIUM);
        }
        seal(&mut frame, size, &look);
        vec![frame.into_geometry()]
    }
}
