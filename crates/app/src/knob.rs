use std::time::{Duration, Instant};

use iced::widget::canvas::{self, path, Frame, Geometry, Path, Stroke};
use iced::{keyboard, mouse, Point, Radians, Rectangle, Renderer, Theme};

use crate::pointer::EndlessDrag;
use crate::theme::Palette;
use crate::Message;

const START_ANGLE: f32 = 0.75 * std::f32::consts::PI;
const SWEEP: f32 = 1.5 * std::f32::consts::PI;
const DOUBLE_CLICK: Duration = Duration::from_millis(350);
const PIXELS_PER_NOTCH: f32 = 50.0;
pub const SWEEP_PX: f32 = 400.0;
pub const FINE: f32 = 0.2;

pub struct Knob<'a> {
    pub palette: &'a Palette,
    pub value: f32,
    pub lowest: f32,
    pub highest: f32,
    pub resting: f32,
    pub step: f32,
    pub pixels_per_point: f32,
    pub centred: bool,
    pub on_turn: Box<dyn Fn(f32) -> Message + 'a>,
}

#[derive(Default)]
pub struct Turning {
    pull: Option<(EndlessDrag, f32)>,
    last_press: Option<Instant>,
    wheeled: bool,
    modifiers: keyboard::Modifiers,
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
            canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state.modifiers = modifiers;
                (Ignored, None)
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(p) = cursor.position_in(bounds) else {
                    return (Ignored, None);
                };
                let pressed_again = state.last_press.is_some_and(|at| at.elapsed() < DOUBLE_CLICK);
                state.last_press = Some(Instant::now());
                if pressed_again {
                    return (Captured, Some((self.on_turn)(self.resting)));
                }
                state.pull = Some((EndlessDrag::start(p, true, self.pixels_per_point), self.value));
                (Captured, std::mem::take(&mut state.wheeled).then_some(Message::DragEnd))
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::CursorLeft)
                if state.wheeled && state.pull.is_none() && !cursor.is_over(bounds) =>
            {
                state.wheeled = false;
                (Ignored, Some(Message::DragEnd))
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let at = cursor.position().map(|p| Point::new(p.x - bounds.x, p.y - bounds.y));
                let (Some((pull, value_at_grab)), Some(p)) = (state.pull.as_mut(), at) else {
                    return (Ignored, None);
                };
                let pace = if state.modifiers.shift() { FINE } else { 1.0 };
                let Some(travel_up) = pull.moved(p, pace) else {
                    return (Captured, None);
                };
                let value = turned(value_at_grab, travel_up, self.lowest, self.highest);
                (Captured, (value != self.value).then(|| (self.on_turn)(value)))
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => match state.pull.take() {
                Some(_) => (Captured, Some(Message::DragEnd)),
                None => (Ignored, None),
            },
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                if !cursor.is_over(bounds) {
                    return (Ignored, None);
                }
                if state.pull.is_some() {
                    return (Captured, None);
                }
                let pace = if state.modifiers.shift() { FINE } else { 1.0 };
                let value = wheeled(self.value, delta, self.step * pace, self.lowest, self.highest);
                if value == self.value {
                    return (Captured, None);
                }
                state.wheeled = true;
                (Captured, Some((self.on_turn)(value)))
            }
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
        let lit = if self.centred {
            let middle = START_ANGLE + SWEEP / 2.0;
            Path::new(|b| {
                b.arc(path::Arc { center: centre, radius, start_angle: Radians(middle.min(angle)), end_angle: Radians(middle.max(angle)) });
            })
        } else {
            sweep(angle)
        };
        frame.stroke(&lit, Stroke::default().with_color(p.accent).with_width(2.0));
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

fn rate(lowest: f32, highest: f32) -> f32 {
    (highest - lowest) / SWEEP_PX
}

fn turned(from: &mut f32, travel_up: f32, lowest: f32, highest: f32) -> f32 {
    let wanted = *from + travel_up * rate(lowest, highest);
    let value = wanted.clamp(lowest, highest);
    *from += value - wanted;
    value
}

fn wheeled(value: f32, delta: mouse::ScrollDelta, step: f32, lowest: f32, highest: f32) -> f32 {
    let turned = match delta {
        mouse::ScrollDelta::Lines { y, .. } if y != 0.0 && y.fract() == 0.0 => {
            let notch = value / step;
            let nearest = notch.round();
            let from = if (notch - nearest).abs() < 0.001 {
                nearest
            } else if y > 0.0 {
                notch.floor()
            } else {
                notch.ceil()
            };
            (from + y) * step
        }
        mouse::ScrollDelta::Lines { y, .. } => value + y * step,
        mouse::ScrollDelta::Pixels { y, .. } => value + y / PIXELS_PER_NOTCH * step,
    };
    turned.clamp(lowest, highest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notches(y: f32) -> mouse::ScrollDelta {
        mouse::ScrollDelta::Lines { x: 0.0, y }
    }

    fn dragged(from: f32, lowest: f32, highest: f32, scale: f32, pace: f32, risen: &[f32]) -> Vec<f32> {
        let mut pull = EndlessDrag::start(Point::new(0.0, 0.0), false, scale);
        let mut grab = from;
        let mut seen = vec![];
        for up in risen {
            if let Some(travel_up) = pull.moved(Point::new(0.0, -up / scale), pace) {
                seen.push(turned(&mut grab, travel_up, lowest, highest));
            }
        }
        seen
    }

    #[test]
    fn a_whole_sweep_takes_the_whole_travel() {
        assert_eq!(dragged(0.0, 0.0, 125.0, 1.0, 1.0, &[SWEEP_PX]), vec![125.0]);
        assert_eq!(dragged(0.0, 0.0, 125.0, 1.0, 1.0, &[SWEEP_PX / 2.0]), vec![62.5]);
        assert_eq!(dragged(0.0, -100.0, 100.0, 1.0, 1.0, &[SWEEP_PX / 2.0]), vec![100.0]);
    }

    #[test]
    fn the_same_hand_movement_turns_the_same_at_every_scale() {
        for scale in [1.0, 2.0, 2.5] {
            assert_eq!(dragged(0.0, 0.0, 125.0, scale, 1.0, &[200.0]), vec![62.5], "at {scale}");
            assert_eq!(dragged(0.0, -100.0, 100.0, scale, 1.0, &[100.0, 200.0]), vec![50.0, 100.0], "at {scale}");
        }
    }

    #[test]
    fn holding_shift_turns_a_fifth_as_far() {
        assert_eq!(dragged(0.0, 0.0, 125.0, 1.0, FINE, &[200.0]), vec![12.5]);
        assert_eq!(dragged(0.0, 0.0, 125.0, 2.5, FINE, &[200.0]), vec![12.5]);
    }

    #[test]
    fn the_drag_stops_at_the_ends_and_turns_back_at_once() {
        assert_eq!(dragged(100.0, 0.0, 125.0, 1.0, 1.0, &[1000.0, 990.0]), vec![125.0, 121.875]);
        assert_eq!(dragged(10.0, 0.0, 125.0, 1.0, 1.0, &[-1000.0, -990.0]), vec![0.0, 3.125]);
    }

    #[test]
    fn a_steady_drag_climbs_in_steady_steps() {
        let risen: Vec<f32> = (1..=40).map(|step| step as f32 * 10.0).collect();
        let seen = dragged(0.0, 0.0, 125.0, 2.5, 1.0, &risen);
        assert_eq!(seen.len(), 40);
        assert_eq!(seen.last(), Some(&125.0));
        let mut before = 0.0;
        for value in seen {
            assert!(value > before, "{value} did not climb past {before}");
            assert!(value - before <= 10.0 * rate(0.0, 125.0) + 1e-4, "{value} jumped from {before}");
            before = value;
        }
    }

    #[test]
    fn each_notch_moves_one_even_step() {
        assert_eq!(wheeled(100.0, notches(1.0), 1.0, 0.0, 125.0), 101.0);
        assert_eq!(wheeled(100.0, notches(-1.0), 1.0, 0.0, 125.0), 99.0);
        assert_eq!(wheeled(100.0, notches(3.0), 1.0, 0.0, 125.0), 103.0);
        assert_eq!(wheeled(0.0, notches(-1.0), 2.0, -100.0, 100.0), -2.0);
        assert_eq!(wheeled(100.4, notches(0.0), 1.0, 0.0, 125.0), 100.4);
    }

    #[test]
    fn a_value_between_steps_lands_on_the_next_step() {
        assert_eq!(wheeled(100.4, notches(1.0), 1.0, 0.0, 125.0), 101.0);
        assert_eq!(wheeled(100.4, notches(-1.0), 1.0, 0.0, 125.0), 100.0);
        assert_eq!(wheeled(99.99999, notches(1.0), 1.0, 0.0, 125.0), 101.0);
    }

    #[test]
    fn the_wheel_stops_at_the_ends() {
        assert_eq!(wheeled(125.0, notches(1.0), 1.0, 0.0, 125.0), 125.0);
        assert_eq!(wheeled(0.5, notches(-2.0), 1.0, 0.0, 125.0), 0.0);
    }

    #[test]
    fn pixel_scrolling_turns_smoothly() {
        let pixels = |y| mouse::ScrollDelta::Pixels { x: 0.0, y };
        assert!((wheeled(100.0, pixels(5.0), 1.0, 0.0, 125.0) - 100.1).abs() < 1e-4);
        assert_eq!(wheeled(100.0, pixels(-25.0), 1.0, 0.0, 125.0), 99.5);
        assert_eq!(wheeled(100.0, notches(0.5), 1.0, 0.0, 125.0), 100.5);
    }
}
