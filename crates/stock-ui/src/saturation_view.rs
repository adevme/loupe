use iced::widget::canvas::{self, Geometry, Path, Stroke};
use iced::widget::{column, container, row, Space};
use iced::{mouse, Alignment, Element, Length, Point, Rectangle, Renderer, Size, Theme, Vector};
use loupe_stock::{saturation_curve, Saturation};

use crate::kit::{choice, dial, graph_frame, knob_row, panel, put_text, toggle};
use crate::knobs::{change, Change, Knobs};
use crate::look::{fade, Look};

const REACH: f32 = 1.5;
const STEPS: usize = 240;
const MARGIN: f32 = 16.0;

pub struct SaturationEditor {
    knobs: Knobs,
    look: Look,
}

impl SaturationEditor {
    pub fn new(look: Look) -> Self {
        Self { knobs: Knobs::of(&Saturation::new()), look }
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
        let heat = knob_row(vec![knob(0, look.danger), knob(2, look.bands[6])]);
        let level = knob_row(vec![knob(3, look.accent), knob(4, look.bands[3])]);
        let auto = self.knobs.on(5);
        let extras = column![
            choice(params[1], values[1], |value| Change(1, value)),
            toggle(look, "Auto gain", auto, Change(5, if auto { 0.0 } else { 1.0 })),
        ]
        .spacing(10);
        let controls = row![heat, Space::with_width(18), level, Space::with_width(Length::Fill), extras].align_y(Alignment::Center);
        let graph = canvas::Canvas::new(Bend { editor: self }).width(Length::Fill).height(Length::Fill);
        let backdrop = container(container(graph).padding([12, 12])).width(Length::Fill).height(Length::Fill).style(move |_| container::Style {
            background: Some(look.background.into()),
            ..Default::default()
        });
        column![backdrop, panel(look, controls)].into()
    }
}

struct Bend<'a> {
    editor: &'a SaturationEditor,
}

impl canvas::Program<Change> for Bend<'_> {
    type State = ();

    fn draw(&self, _state: &(), renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let look = self.editor.look;
        let size = bounds.size();
        if !crate::kit::roomy(size) {
            return Vec::new();
        }
        let mut frame = graph_frame(renderer, size, &look);
        let side = (size.height - MARGIN * 2.0).min(size.width - MARGIN * 2.0).max(1.0);
        let corner = Vector::new((size.width - side) / 2.0, (size.height - side) / 2.0);
        frame.with_save(|frame| {
            frame.translate(corner);
            frame.fill_rectangle(Point::ORIGIN, Size::new(side, side), look.panel);
            let at = |input: f32, output: f32| {
                let share = |v: f32| (v / REACH + 1.0) / 2.0;
                Point::new(share(input) * side, side - share(output.clamp(-REACH, REACH)) * side)
            };
            for value in [-1.0, -0.5, 0.0, 0.5, 1.0] {
                let p = at(value, value);
                let colour = if value == 0.0 { look.grid_strong } else { look.grid };
                frame.fill_rectangle(Point::new(p.x, 0.0), Size::new(1.0, side), colour);
                frame.fill_rectangle(Point::new(0.0, p.y), Size::new(side, 1.0), colour);
            }
            frame.stroke(&Path::line(at(-REACH, -REACH), at(REACH, REACH)), Stroke::default().with_color(fade(look.text_dim, 0.35)).with_width(1.0));
            let values = &self.editor.knobs.values;
            let line = Path::new(|b| {
                for step in 0..=STEPS {
                    let input = -REACH + 2.0 * REACH * step as f32 / STEPS as f32;
                    let point = at(input, saturation_curve(values, input));
                    if step == 0 {
                        b.move_to(point);
                    } else {
                        b.line_to(point);
                    }
                }
            });
            frame.stroke(&line, Stroke::default().with_color(look.danger).with_width(2.5));
            put_text(frame, "In", Point::new(side - 6.0, side - 6.0), 11.0, look.text_dim, (1.0, 1.0));
            put_text(frame, "Out", Point::new(6.0, 6.0), 11.0, look.text_dim, (0.0, 0.0));
        });
        vec![frame.into_geometry()]
    }
}
