use std::time::{Duration, Instant};

use iced::widget::canvas::{self, path, Frame, Geometry, Path, Stroke};
use iced::{mouse, Point, Radians, Rectangle, Renderer, Theme};

use crate::pointer::EndlessDrag;
use crate::theme::Palette;
use crate::Message;

const START_ANGLE: f32 = 0.75 * std::f32::consts::PI;
const SWEEP: f32 = 1.5 * std::f32::consts::PI;
const DOUBLE_CLICK: Duration = Duration::from_millis(350);

pub struct Knob<'a> {
    pub palette: &'a Palette,
    pub value: f32,
    pub lowest: f32,
    pub highest: f32,
    pub resting: f32,
    pub per_px: f32,
    pub on_turn: fn(f32) -> Message,
}

#[derive(Default)]
pub struct Turning {
    pull: Option<(EndlessDrag, f32)>,
    last_press: Option<Instant>,
}

impl canvas::Program<Message> for Knob<'_> {
    type State = Turning;

    fn update(
        &self,
        state: &mut Turning,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Message>) {
        use canvas::event::Status::{Captured, Ignored};
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(p) = cursor.position_in(bounds) else {
                    return (Ignored, None);
                };
                let pressed_again = state.last_press.is_some_and(|at| at.elapsed() < DOUBLE_CLICK);
                state.last_press = Some(Instant::now());
                if pressed_again {
                    return (Captured, Some((self.on_turn)(self.resting)));
                }
                state.pull = Some((EndlessDrag::start(p, true), self.value));
                (Captured, None)
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let at = cursor.position().map(|p| Point::new(p.x - bounds.x, p.y - bounds.y));
                let (Some((pull, value_at_grab)), Some(p)) = (state.pull.as_mut(), at) else {
                    return (Ignored, None);
                };
                let Some(travel_up) = pull.moved(p) else {
                    return (Captured, None);
                };
                let value = (*value_at_grab + travel_up * self.per_px).clamp(self.lowest, self.highest);
                (Captured, (value != self.value).then(|| (self.on_turn)(value)))
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => match state.pull.take() {
                Some(_) => (Captured, Some(Message::DragEnd)),
                None => (Ignored, None),
            },
            _ => (Ignored, None),
        }
    }

    fn draw(
        &self,
        _state: &Turning,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let p = self.palette;
        let mut frame = Frame::new(renderer, bounds.size());
        let centre = Point::new(bounds.width / 2.0, bounds.height / 2.0);
        let radius = bounds.width.min(bounds.height) / 2.0 - 1.5;
        let turned = ((self.value - self.lowest) / (self.highest - self.lowest)).clamp(0.0, 1.0);
        let angle = START_ANGLE + SWEEP * turned;
        let sweep = |until: f32| {
            Path::new(|b| {
                b.arc(path::Arc {
                    center: centre,
                    radius,
                    start_angle: Radians(START_ANGLE),
                    end_angle: Radians(until),
                });
            })
        };
        frame.fill(&Path::circle(centre, radius - 3.0), p.raised);
        frame.stroke(&sweep(START_ANGLE + SWEEP), Stroke::default().with_color(p.hover).with_width(2.0));
        frame.stroke(&sweep(angle), Stroke::default().with_color(p.accent).with_width(2.0));
        let along = |distance: f32| Point::new(centre.x + angle.cos() * distance, centre.y + angle.sin() * distance);
        let pointer = Path::line(along(radius * 0.25), along(radius - 4.0));
        frame.stroke(&pointer, Stroke::default().with_color(p.text).with_width(2.0));
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, state: &Turning, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if state.pull.is_some() || cursor.is_over(bounds) {
            mouse::Interaction::ResizingVertically
        } else {
            mouse::Interaction::default()
        }
    }
}
