use iced::widget::canvas::{self, Geometry, LineDash, Path, Stroke};
use iced::{mouse, Element, Length, Point, Rectangle, Renderer, Size, Theme, Vector};
use loupe_stock::{saturation_curve, Saturation};

use crate::compressor_view::INSET;
use crate::kit::*;
use crate::knobs::{change, Change, Knobs};
use crate::look::{fade, Look, MEDIUM, SEMIBOLD};

const REACH: f32 = 1.5;
const STEPS: usize = 240;

pub struct SaturationEditor {
    knobs: Knobs,
    look: Look,
}

impl SaturationEditor {
    pub fn new(look: Look) -> Self {
        let look = look.tinted(crate::look::accents::SATURATION);
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
        let auto = self.knobs.on(5);
        shell(look, "Saturation", "Harmonic drive", move || canvas::Canvas::new(Bend { editor: self }).width(Length::Fill).height(Length::Fill).into(), move |compact| {
            let p = Panel::new(look, compact, params, values, change);
            vec![vec![
                section(look, "Character", vec![stack(vec![choice(look, params[1], values[1], |value| Change(1, value)), toggle(look, "Auto gain", auto, Change(5, if auto { 0.0 } else { 1.0 }))])]),
                section(look, "Heat", vec![p.hero(0), p.knob(2)]),
                section(look, "Output", vec![p.hero(3), p.knob(4)]),
            ]]
        })
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
        let side = (size.height - INSET * 2.0).min(size.width - INSET * 2.0).max(1.0);
        let corner = Vector::new((size.width - side) / 2.0, (size.height - side) / 2.0);
        frame.with_save(|frame| {
            frame.translate(corner);
            draw_card(frame, Point::ORIGIN, Size::new(side, side), &look);
            let pad = 12.0;
            let inner = side - pad * 2.0;
            let at = |input: f32, output: f32| {
                let share = |v: f32| (v / REACH + 1.0) / 2.0;
                Point::new(pad + share(input) * inner, pad + inner - share(output.clamp(-REACH, REACH)) * inner)
            };
            for value in [-1.0, -0.5, 0.0, 0.5, 1.0] {
                let p = at(value, value);
                let colour = if value == 0.0 { look.grid_strong } else { look.grid };
                frame.fill_rectangle(Point::new(p.x.round(), pad), Size::new(1.0, inner), colour);
                frame.fill_rectangle(Point::new(pad, p.y.round()), Size::new(inner, 1.0), colour);
            }
            let diagonal = Path::line(at(-REACH, -REACH), at(REACH, REACH));
            frame.stroke(&diagonal, Stroke { line_dash: LineDash { segments: &[4.0, 4.0], offset: 0 }, ..Stroke::default().with_color(fade(look.text_dim, 0.4)).with_width(1.0) });
            let values = &self.editor.knobs.values;
            let points: Vec<Point> = (0..=STEPS)
                .map(|step| {
                    let input = -REACH + 2.0 * REACH * step as f32 / STEPS as f32;
                    at(input, saturation_curve(values, input))
                })
                .collect();
            let line = polyline(&points);
            frame.stroke(&line, Stroke::default().with_color(fade(look.accent, 0.18)).with_width(8.0));
            frame.stroke(&line, Stroke::default().with_color(look.accent).with_width(2.2));
            axis_text(frame, "IN", Point::new(side - pad - 2.0, side - pad - 2.0), &look, (1.0, 1.0));
            axis_text(frame, "OUT", Point::new(pad + 2.0, pad + 2.0), &look, (0.0, 0.0));
        });
        let values = &self.editor.knobs.values;
        let style = self.editor.knobs.params[1].chosen(values[1]).unwrap_or("").to_uppercase();
        let room = corner.x;
        if room > 150.0 {
            put_text_in(&mut frame, "STYLE", Point::new(room / 2.0, size.height / 2.0 - 30.0), 9.5, look.text_faint, (0.5, 0.5), SEMIBOLD);
            put_text_in(&mut frame, &style, Point::new(room / 2.0, size.height / 2.0), 26.0, look.text, (0.5, 0.5), MEDIUM);
            let right = size.width - room / 2.0;
            put_text_in(&mut frame, "DRIVE", Point::new(right, size.height / 2.0 - 30.0), 9.5, look.text_faint, (0.5, 0.5), SEMIBOLD);
            put_text_in(&mut frame, &format!("{:+.1} dB", values[0]), Point::new(right, size.height / 2.0), 26.0, look.accent, (0.5, 0.5), MEDIUM);
        }
        seal(&mut frame, size, &look);
        vec![frame.into_geometry()]
    }
}
