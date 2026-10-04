use std::f32::consts::PI;
use std::time::{Duration, Instant};

use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke, Text};
use iced::widget::{button, column, container, pick_list, row, text};
use iced::{alignment, keyboard, mouse, Alignment, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};
use loupe_stock::{Param, Unit};

use crate::look::Look;

const DIAL_RADIUS: f32 = 21.0;
const DIAL_WIDTH: f32 = 76.0;
const DIAL_HEIGHT: f32 = 92.0;
const SWEEP: f32 = 1.5 * PI;
const START: f32 = 0.75 * PI;
const ARC_STEPS: usize = 48;
const PIXELS_FOR_FULL_TURN: f32 = 180.0;
const FINE: f32 = 0.2;
const WHEEL_STEP: f32 = 0.02;
const DOUBLE_CLICK: Duration = Duration::from_millis(350);

pub fn put_text(frame: &mut Frame, content: &str, at: Point, size: f32, color: Color, anchor: (f32, f32)) {
    let across = match anchor.0 {
        a if a <= 0.0 => alignment::Horizontal::Left,
        a if a >= 1.0 => alignment::Horizontal::Right,
        _ => alignment::Horizontal::Center,
    };
    let down = match anchor.1 {
        a if a <= 0.0 => alignment::Vertical::Top,
        a if a >= 1.0 => alignment::Vertical::Bottom,
        _ => alignment::Vertical::Center,
    };
    let text = Text {
        content: content.to_string(),
        position: Point::new(at.x.round(), at.y.round()),
        color,
        size: size.into(),
        horizontal_alignment: across,
        vertical_alignment: down,
        ..Text::default()
    };
    frame.fill_text(text);
}

pub fn arc(centre: Point, radius: f32, from: f32, to: f32) -> Path {
    Path::new(|b| {
        let steps = ((ARC_STEPS as f32 * (to - from).abs() / SWEEP).ceil() as usize).max(1);
        for step in 0..=steps {
            let angle = from + (to - from) * step as f32 / steps as f32;
            let at = Point::new(centre.x + radius * angle.cos(), centre.y + radius * angle.sin());
            if step == 0 {
                b.move_to(at);
            } else {
                b.line_to(at);
            }
        }
    })
}

pub fn hertz(hz: f32) -> String {
    if hz >= 1000.0 {
        format!("{:.2} kHz", hz / 1000.0)
    } else if hz < 10.0 {
        format!("{hz:.1} Hz")
    } else {
        format!("{hz:.0} Hz")
    }
}

pub fn shown(param: &Param, value: f32) -> String {
    match (param.unit, param.id) {
        (Unit::Decibels, _) => format!("{value:+.1} dB"),
        (Unit::Hertz, _) => hertz(value),
        (Unit::Milliseconds, _) if value >= 1000.0 => format!("{:.2} s", value / 1000.0),
        (Unit::Milliseconds, _) if value < 10.0 => format!("{value:.2} ms"),
        (Unit::Milliseconds, _) => format!("{value:.0} ms"),
        (Unit::Seconds, _) => format!("{value:.2} s"),
        (Unit::Percent | Unit::Width, _) => format!("{value:.0}%"),
        (Unit::Ratio, "ratio") => format!("{value:.1} : 1"),
        (Unit::Ratio, "wobble_rate") => format!("{value:.2} Hz"),
        (Unit::Ratio, "bass_decay") => format!("x {value:.2}"),
        (Unit::Ratio, _) => format!("{value:.2}"),
        (Unit::Switch, _) => if value > 0.5 { "On" } else { "Off" }.to_string(),
        (Unit::Choice, _) => param.chosen(value).unwrap_or("").to_string(),
    }
}

struct Dial<M> {
    param: Param,
    value: f32,
    colour: Color,
    look: Look,
    index: usize,
    change: fn(usize, f32) -> M,
}

#[derive(Default)]
pub struct Turning {
    from: Option<(f32, f32)>,
    last_press: Option<Instant>,
    modifiers: keyboard::Modifiers,
}

impl<M> Dial<M> {
    fn centre(&self) -> Point {
        Point::new(DIAL_WIDTH / 2.0, 20.0 + DIAL_RADIUS + 4.0)
    }

    fn angle(&self, position: f32) -> f32 {
        START + SWEEP * position
    }
}

impl<M> canvas::Program<M> for Dial<M> {
    type State = Turning;

    fn update(&self, turning: &mut Turning, event: canvas::Event, bounds: Rectangle, cursor: mouse::Cursor) -> (canvas::event::Status, Option<M>) {
        use canvas::event::Status::{Captured, Ignored};
        let param = &self.param;
        match event {
            canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                turning.modifiers = modifiers;
                (Ignored, None)
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(p) = cursor.position_in(bounds) else {
                    return (Ignored, None);
                };
                if turning.last_press.is_some_and(|at| at.elapsed() < DOUBLE_CLICK) {
                    turning.last_press = None;
                    turning.from = None;
                    return (Captured, Some((self.change)(self.index, param.default)));
                }
                turning.last_press = Some(Instant::now());
                turning.from = Some((p.y, param.to_position(self.value)));
                (Captured, None)
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let (Some((from_y, from_position)), Some(p)) = (turning.from, cursor.position_from(bounds.position())) else {
                    return (Ignored, None);
                };
                let scale = if turning.modifiers.shift() { FINE } else { 1.0 };
                let position = from_position + (from_y - p.y) * scale / PIXELS_FOR_FULL_TURN;
                (Captured, Some((self.change)(self.index, param.from_position(position))))
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => match turning.from.take() {
                Some(_) => (Captured, None),
                None => (Ignored, None),
            },
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                if cursor.position_in(bounds).is_none() {
                    return (Ignored, None);
                }
                let lines = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => y,
                    mouse::ScrollDelta::Pixels { y, .. } => y / 40.0,
                };
                let scale = if turning.modifiers.shift() { FINE } else { 1.0 };
                let position = param.to_position(self.value) + lines * WHEEL_STEP * scale;
                (Captured, Some((self.change)(self.index, param.from_position(position))))
            }
            _ => (Ignored, None),
        }
    }

    fn draw(&self, turning: &Turning, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, cursor: mouse::Cursor) -> Vec<Geometry> {
        let look = self.look;
        let mut frame = Frame::new(renderer, bounds.size());
        let centre = self.centre();
        let lit = turning.from.is_some() || cursor.is_over(bounds);
        put_text(&mut frame, self.param.name, Point::new(DIAL_WIDTH / 2.0, 2.0), 11.5, look.text_dim, (0.5, 0.0));
        frame.stroke(&arc(centre, DIAL_RADIUS, START, START + SWEEP), Stroke::default().with_color(look.grid_strong).with_width(3.0));
        let position = self.param.to_position(self.value);
        let zero = if self.param.min < 0.0 && self.param.max > 0.0 { self.param.to_position(0.0) } else { 0.0 };
        let (from, to) = (self.angle(zero.min(position)), self.angle(zero.max(position)));
        if to - from > 0.001 {
            frame.stroke(&arc(centre, DIAL_RADIUS, from, to), Stroke::default().with_color(self.colour).with_width(3.0));
        }
        let face = Path::circle(centre, DIAL_RADIUS - 5.0);
        frame.fill(&face, if lit { look.raised } else { look.panel });
        let angle = self.angle(position);
        let tip = Point::new(centre.x + (DIAL_RADIUS - 8.0) * angle.cos(), centre.y + (DIAL_RADIUS - 8.0) * angle.sin());
        let inner = Point::new(centre.x + 4.0 * angle.cos(), centre.y + 4.0 * angle.sin());
        frame.stroke(&Path::line(inner, tip), Stroke::default().with_color(look.text).with_width(2.0));
        let reading = shown(&self.param, self.value);
        let colour = if lit { look.text } else { look.text_dim };
        put_text(&mut frame, &reading, Point::new(DIAL_WIDTH / 2.0, DIAL_HEIGHT - 4.0), 11.5, colour, (0.5, 1.0));
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, turning: &Turning, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if turning.from.is_some() {
            mouse::Interaction::ResizingVertically
        } else if cursor.is_over(bounds) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}

pub fn dial<'a, M: 'a>(look: Look, params: &'static [Param], values: &[f32], index: usize, colour: Color, change: fn(usize, f32) -> M) -> Element<'a, M> {
    canvas::Canvas::new(Dial { param: params[index], value: values[index], colour, look, index, change })
        .width(DIAL_WIDTH)
        .height(DIAL_HEIGHT)
        .into()
}

pub fn toggle<'a, M: Clone + 'a>(look: Look, label: &'static str, on: bool, message: M) -> Element<'a, M> {
    button(text(label).size(12))
        .padding([5, 12])
        .style(move |_, status| {
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            let background = match (on, hovered) {
                (true, _) => look.accent,
                (false, true) => look.raised,
                (false, false) => look.panel,
            };
            button::Style {
                background: Some(background.into()),
                text_color: if on { look.on_accent } else { look.text },
                border: iced::Border { color: look.grid_strong, width: 1.0, radius: 6.0.into() },
                ..Default::default()
            }
        })
        .on_press(message)
        .into()
}

pub fn choice<'a, M: Clone + 'a>(param: Param, value: f32, change: fn(f32) -> M) -> Element<'a, M> {
    let current = param.chosen(value);
    column![
        text(param.name).size(11.5),
        pick_list(param.choices, current, move |picked: &'static str| {
            let index = param.choices.iter().position(|choice| *choice == picked).unwrap_or(0);
            change(index as f32)
        })
        .text_size(12)
        .padding([4, 8]),
    ]
    .spacing(4)
    .into()
}

pub fn panel<'a, M: 'a>(look: Look, content: impl Into<Element<'a, M>>) -> Element<'a, M> {
    container(content)
        .padding([10, 14])
        .width(Length::Fill)
        .style(move |_| container::Style { background: Some(look.panel.into()), ..Default::default() })
        .into()
}

pub fn knob_row<'a, M: 'a>(items: Vec<Element<'a, M>>) -> Element<'a, M> {
    row(items).spacing(6).align_y(Alignment::End).into()
}

pub fn roomy(size: Size) -> bool {
    size.width >= 8.0 && size.height >= 8.0 && size.width.is_finite() && size.height.is_finite()
}

pub fn graph_frame(renderer: &Renderer, size: Size, look: &Look) -> Frame {
    let mut frame = Frame::new(renderer, size);
    frame.fill_rectangle(Point::ORIGIN, size, look.background);
    frame
}
