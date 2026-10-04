use iced::widget::canvas::{self, Geometry, Path, Stroke};
use iced::widget::{column, container, row, Space};
use iced::{mouse, Alignment, Element, Length, Point, Rectangle, Renderer, Size, Theme};
use loupe_stock::{chorus_spread, chorus_sweep, Chorus};

use crate::kit::{choice, dial, graph_frame, knob_row, panel, put_text};
use crate::knobs::{change, Change, Knobs};
use crate::look::Look;

const SHOWN_SECONDS: f32 = 2.0;
const STEPS: usize = 400;
const HIGHEST_MS: f32 = 24.0;
const TOP_GAP: f32 = 24.0;
const LABELS_MS: [f32; 4] = [0.0, 5.0, 10.0, 20.0];

pub struct ChorusEditor {
    knobs: Knobs,
    look: Look,
}

impl ChorusEditor {
    pub fn new(look: Look) -> Self {
        Self { knobs: Knobs::of(&Chorus::new()), look }
    }

    pub fn load(&mut self, values: &[f32]) {
        for (slot, value) in values.iter().enumerate().take(self.knobs.values.len()) {
            self.knobs.values[slot] = *value;
        }
    }

    pub fn update(&mut self, change: Change) -> Vec<(usize, f32)> {
        self.knobs.apply(change)
    }

    pub fn view(&self) -> Element<'_, Change> {
        let look = self.look;
        let values = &self.knobs.values;
        let params = self.knobs.params;
        let knob = |index: usize, colour| dial(look, params, values, index, colour, change);
        let motion = knob_row(vec![knob(1, look.accent), knob(2, look.accent), knob(3, look.danger)]);
        let space = knob_row(vec![knob(4, look.bands[5]), knob(5, look.bands[3])]);
        let controls = row![motion, Space::with_width(18), space, Space::with_width(Length::Fill), choice(params[0], values[0], |value| Change(0, value))]
            .align_y(Alignment::Center);
        let graph = canvas::Canvas::new(Sweep { editor: self }).width(Length::Fill).height(Length::Fill);
        let backdrop = container(container(graph).padding([12, 12])).width(Length::Fill).height(Length::Fill).style(move |_| container::Style {
            background: Some(look.background.into()),
            ..Default::default()
        });
        column![backdrop, panel(look, controls)].into()
    }
}

struct Sweep<'a> {
    editor: &'a ChorusEditor,
}

impl canvas::Program<Change> for Sweep<'_> {
    type State = ();

    fn draw(&self, _state: &(), renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let look = self.editor.look;
        let size = bounds.size();
        if !crate::kit::roomy(size) {
            return Vec::new();
        }
        let mut frame = graph_frame(renderer, size, &look);
        let values = &self.editor.knobs.values;
        let (base, swing) = chorus_sweep(values);
        let rate = values[1];
        let offset = chorus_spread(values);
        let tall = size.height - TOP_GAP;
        let y_of = |ms: f32| size.height - (ms / HIGHEST_MS).clamp(0.0, 1.0) * tall;
        for ms in LABELS_MS {
            let y = y_of(ms).round();
            frame.fill_rectangle(Point::new(0.0, y), Size::new(size.width, 1.0), look.grid);
            put_text(&mut frame, &format!("{ms:.0} ms"), Point::new(size.width - 6.0, y - 3.0), 11.0, look.text_dim, (1.0, 1.0));
        }
        for second in 1..SHOWN_SECONDS as usize {
            let x = (second as f32 / SHOWN_SECONDS * size.width).round();
            frame.fill_rectangle(Point::new(x, TOP_GAP), Size::new(1.0, tall), look.grid);
        }
        for (shift, colour, label) in [(0.0, look.accent, "Left"), (offset, look.bands[5], "Right")] {
            let line = Path::new(|b| {
                for step in 0..=STEPS {
                    let seconds = SHOWN_SECONDS * step as f32 / STEPS as f32;
                    let angle = std::f32::consts::TAU * rate * seconds + shift;
                    let point = Point::new(size.width * step as f32 / STEPS as f32, y_of(base + swing * 0.5 * (1.0 + angle.sin())));
                    if step == 0 {
                        b.move_to(point);
                    } else {
                        b.line_to(point);
                    }
                }
            });
            frame.stroke(&line, Stroke::default().with_color(colour).with_width(2.0));
            let x = if shift == 0.0 { 8.0 } else { 56.0 };
            put_text(&mut frame, label, Point::new(x, 6.0), 11.5, colour, (0.0, 0.0));
        }
        vec![frame.into_geometry()]
    }
}
