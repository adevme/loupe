use std::sync::Arc;

use iced::widget::canvas::{self, Geometry, Path, Stroke};
use iced::widget::{column, container, row, Space};
use iced::{mouse, Alignment, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};
use loupe_stock::{multiband_knob, History, Moment, Multiband, MULTIBAND_BANDS};

use crate::kit::{choice, dial, dial_labelled, graph_frame, hertz, knob_row, panel, put_text};
use crate::knobs::{change, Change, Knobs};
use crate::look::{fade, Look};

const SHOWN_MOMENTS: usize = 400;
const LOWEST_HZ: f32 = 20.0;
const HIGHEST_HZ: f32 = 20_000.0;
const DEEPEST_SHOWN: f32 = 18.0;
const NAMES: [&str; MULTIBAND_BANDS] = ["Low", "Mid", "High"];
const LABEL_TOP: f32 = 14.0;
const TRACE_TOP: f32 = 96.0;

pub struct MultibandEditor {
    knobs: Knobs,
    history: Option<Arc<History>>,
    moments: Vec<Moment>,
    look: Look,
}

impl MultibandEditor {
    pub fn new(history: Option<Arc<History>>, look: Look) -> Self {
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
        let knob = |index: usize, colour| dial(look, params, values, index, colour, change);
        let shared = knob_row(vec![knob(0, look.text_dim), knob(1, look.text_dim), knob(11, look.accent), knob(12, look.accent), knob(13, look.bands[3]), knob(14, look.bands[3])]);
        let mut bands = row![].spacing(18).align_y(Alignment::End);
        for band in 0..MULTIBAND_BANDS {
            let colour = self.colour(band);
            let labels = ["Threshold", "Ratio", "Gain"];
            bands = bands.push(knob_row((0..3).map(|which| dial_labelled(look, params, values, multiband_knob(band, which), colour, labels[which], change)).collect()));
        }
        let controls = column![
            row![shared, Space::with_width(Length::Fill), choice(params[15], values[15], |value| Change(15, value))].align_y(Alignment::Center),
            bands,
        ]
        .spacing(8);
        let graph = canvas::Canvas::new(Bands { editor: self }).width(Length::Fill).height(Length::Fill);
        let backdrop = container(container(graph).padding([12, 12])).width(Length::Fill).height(Length::Fill).style(move |_| container::Style {
            background: Some(look.background.into()),
            ..Default::default()
        });
        column![backdrop, panel(look, controls)].into()
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
        let trace_h = size.height - TRACE_TOP - 8.0;
        for band in 0..MULTIBAND_BANDS {
            let (left, right) = (edges[band], edges[band + 1]);
            let colour = editor.colour(band);
            let lit = soloed == 0 || soloed == band + 1;
            frame.fill_rectangle(Point::new(left, 0.0), Size::new(right - left, size.height), fade(colour, if lit { 0.06 } else { 0.02 }));
            let recent = editor.moments.iter().rev().take(20).map(|moment| band_of(moment, band)).fold(0.0f32, f32::min);
            let middle = (left + right) / 2.0;
            put_text(&mut frame, NAMES[band], Point::new(middle, LABEL_TOP), 12.0, if lit { colour } else { fade(look.text_dim, 0.5) }, (0.5, 0.0));
            put_text(&mut frame, &format!("{recent:.1} dB"), Point::new(middle, LABEL_TOP + 22.0), 26.0, if lit { look.text } else { look.text_dim }, (0.5, 0.0));
            let detail = format!("{:.0} dB  {:.1}:1  {:+.1} dB", values[multiband_knob(band, 0)], values[multiband_knob(band, 1)], values[multiband_knob(band, 2)]);
            put_text(&mut frame, &detail, Point::new(middle, LABEL_TOP + 58.0), 11.0, look.text_dim, (0.5, 0.0));
            let width = (right - left - 24.0).max(1.0);
            let count = editor.moments.len().max(2);
            let line = Path::new(|b| {
                for (i, moment) in editor.moments.iter().enumerate() {
                    let depth = (-band_of(moment, band)).clamp(0.0, DEEPEST_SHOWN) / DEEPEST_SHOWN;
                    let point = Point::new(left + 12.0 + width * i as f32 / (count - 1) as f32, TRACE_TOP + depth * trace_h);
                    if i == 0 {
                        b.move_to(point);
                    } else {
                        b.line_to(point);
                    }
                }
            });
            frame.fill_rectangle(Point::new(left + 12.0, TRACE_TOP), Size::new(width, 1.0), look.grid);
            frame.stroke(&line, Stroke::default().with_color(if lit { colour } else { fade(colour, 0.3) }).with_width(2.0));
        }
        for (index, hz) in [values[0], values[1]].into_iter().enumerate() {
            let x = edges[index + 1].round();
            frame.fill_rectangle(Point::new(x, 0.0), Size::new(2.0, size.height), look.grid_strong);
            put_text(&mut frame, &hertz(hz), Point::new(x + 6.0, size.height - 8.0), 11.0, look.text_dim, (0.0, 1.0));
        }
        vec![frame.into_geometry()]
    }
}
