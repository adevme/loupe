use std::sync::Arc;

use iced::widget::canvas::{self, Geometry, Path, Stroke};
use iced::widget::{column, container, row, Space};
use iced::{mouse, Alignment, Element, Length, Point, Rectangle, Renderer, Size, Theme};
use loupe_stock::{History, Moment, Transient};

use crate::kit::{dial, graph_frame, knob_row, panel, put_text};
use crate::knobs::{change, Change, Knobs};
use crate::look::{fade, Look};

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
        let knob = |index: usize, colour| dial(look, params, values, index, colour, change);
        let shape = knob_row(vec![knob(0, look.accent), knob(1, look.bands[5])]);
        let level = knob_row(vec![knob(2, look.bands[3]), knob(3, look.bands[3])]);
        let controls = row![shape, Space::with_width(18), level, Space::with_width(Length::Fill)].align_y(Alignment::Center);
        let graph = canvas::Canvas::new(Hits { editor: self }).width(Length::Fill).height(Length::Fill);
        let backdrop = container(container(graph).padding([12, 12])).width(Length::Fill).height(Length::Fill).style(move |_| container::Style {
            background: Some(look.background.into()),
            ..Default::default()
        });
        column![backdrop, panel(look, controls)].into()
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
        let column_w = size.width / moments.len().max(1) as f32;
        let middle = size.height / 2.0;
        let level_h = |gain: f32| {
            let db = (20.0 * gain.max(1e-6).log10()).clamp(LOWEST_DB, 0.0);
            (db - LOWEST_DB) / -LOWEST_DB * size.height
        };
        for (i, moment) in moments.iter().enumerate() {
            let x = i as f32 * column_w;
            let tall = level_h(moment.input);
            frame.fill_rectangle(Point::new(x, size.height - tall), Size::new(column_w.ceil(), tall), fade(look.spectrum, 0.2));
        }
        frame.fill_rectangle(Point::new(0.0, middle), Size::new(size.width, 1.0), look.grid_strong);
        for db in [-WIDEST_CHANGE_DB, -6.0, 6.0, WIDEST_CHANGE_DB] {
            let y = middle - db / WIDEST_CHANGE_DB * (middle - 8.0);
            frame.fill_rectangle(Point::new(0.0, y.round()), Size::new(size.width, 1.0), look.grid);
            put_text(&mut frame, &format!("{db:+.0} dB"), Point::new(size.width - 6.0, y - 3.0), 11.0, look.text_dim, (1.0, 1.0));
        }
        for (i, moment) in moments.iter().enumerate() {
            let change = moment.reduction.clamp(-WIDEST_CHANGE_DB, WIDEST_CHANGE_DB);
            if change.abs() < 0.05 {
                continue;
            }
            let x = i as f32 * column_w;
            let y = middle - change / WIDEST_CHANGE_DB * (middle - 8.0);
            let colour = fade(if change > 0.0 { look.accent } else { look.danger }, 0.75);
            frame.fill_rectangle(Point::new(x, y.min(middle)), Size::new(column_w.ceil(), (y - middle).abs()), colour);
        }
        frame.stroke(&Path::line(Point::new(0.0, middle), Point::new(size.width, middle)), Stroke::default().with_color(look.grid_strong).with_width(1.0));
        put_text(&mut frame, "Boost", Point::new(8.0, 6.0), 11.5, look.accent, (0.0, 0.0));
        put_text(&mut frame, "Cut", Point::new(8.0, size.height - 6.0), 11.5, look.danger, (0.0, 1.0));
        vec![frame.into_geometry()]
    }
}
