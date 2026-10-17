use std::f32::consts::PI;
use std::time::{Duration, Instant};

use iced::widget::canvas::{self, gradient, Frame, Geometry, LineCap, LineDash, Path, Stroke, Text};
use iced::widget::{button, column, container, pick_list, row, text, Space};
use iced::{alignment, keyboard, mouse, Alignment, Color, Element, Font, Length, Point, Rectangle, Renderer, Size, Theme};
use loupe_stock::{Param, Unit};

use crate::look::{fade, lift, mix, Look, MEDIUM, SEMIBOLD};

const SWEEP: f32 = 1.5 * PI;
const START: f32 = 0.75 * PI;
const ARC_STEPS: usize = 64;
const PIXELS_FOR_FULL_TURN: f32 = 180.0;
const FINE: f32 = 0.2;
const WHEEL_STEP: f32 = 0.02;
const DOUBLE_CLICK: Duration = Duration::from_millis(350);
const WELL_RADIUS: f32 = 6.0;
const CARD_RADIUS: f32 = 6.0;
const CAPS_PX: f32 = 6.3;
const LABEL_SIZE: f32 = 9.5;
pub const CHOICE_WIDTH: f32 = 128.0;
/// Below this window height the hero knobs shrink and the deck keeps to one line.
pub const COMPACT_BELOW: f32 = 470.0;
const HEADER_HEIGHT: f32 = 38.0;
const SECTION_GAP: f32 = 22.0;
const ITEM_GAP: f32 = 4.0;
const SECTION_TITLE: f32 = 22.0;

/// How big a knob is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum KnobSize {
    /// The two or three controls that matter most in a plugin.
    Hero,
    Regular,
    /// A small knob with its label and value beside it, for bars and panels.
    Mini,
}

impl KnobSize {
    fn cap(self) -> f32 {
        match self {
            KnobSize::Hero => 25.0,
            KnobSize::Regular => 16.0,
            KnobSize::Mini => 10.0,
        }
    }

    fn ring(self) -> f32 {
        match self {
            KnobSize::Hero => 33.0,
            KnobSize::Regular => 22.0,
            KnobSize::Mini => 13.5,
        }
    }

    fn height(self) -> f32 {
        match self {
            KnobSize::Hero => 122.0,
            KnobSize::Regular => 92.0,
            KnobSize::Mini => 34.0,
        }
    }

    fn min_width(self) -> f32 {
        match self {
            KnobSize::Hero => 100.0,
            KnobSize::Regular => 64.0,
            KnobSize::Mini => 128.0,
        }
    }
}

pub fn put_text_in(frame: &mut Frame, content: &str, at: Point, size: f32, color: Color, anchor: (f32, f32), font: Font) {
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
    frame.fill_text(Text {
        content: content.to_string(),
        position: Point::new(at.x.round(), at.y.round()),
        color,
        size: size.into(),
        font,
        horizontal_alignment: across,
        vertical_alignment: down,
        ..Text::default()
    });
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
        (Unit::Semitones, _) => format!("{value:+.1} st"),
        (Unit::Ratio, id) if id.ends_with("ratio") => format!("{value:.1} : 1"),
        (Unit::Ratio, "wobble_rate") => format!("{value:.2} Hz"),
        (Unit::Ratio, "bass_decay") => format!("x {value:.2}"),
        (Unit::Ratio, _) => format!("{value:.2}"),
        (Unit::Switch, _) => if value > 0.5 { "On" } else { "Off" }.to_string(),
        (Unit::Choice, _) => param.chosen(value).unwrap_or("").to_string(),
    }
}

fn caps_width(label: &str) -> f32 {
    label.chars().count() as f32 * CAPS_PX
}

fn vertical(top: Point, bottom: Point, from: Color, to: Color) -> gradient::Linear {
    gradient::Linear::new(top, bottom).add_stop(0.0, from).add_stop(1.0, to)
}

fn round(colour: Color, thick: f32) -> Stroke<'static> {
    Stroke::default().with_color(colour).with_width(thick).with_line_cap(LineCap::Round)
}

/// A machined knob cap: a dark body lit from the upper left, a bevelled edge and
/// fine grip lines that turn with the value.
pub fn draw_cap(frame: &mut Frame, centre: Point, radius: f32, angle: f32, lit: bool) {
    frame.fill(&Path::circle(Point::new(centre.x, centre.y + radius * 0.12 + 1.5), radius + 2.5), Color::from_rgba(0.0, 0.0, 0.0, 0.45));
    frame.fill(&Path::circle(centre, radius + 1.0), Color::from_rgb(0.035, 0.037, 0.043));
    let glow = if lit { 0.03 } else { 0.0 };
    let steps = 9;
    for step in 0..steps {
        let t = step as f32 / (steps - 1) as f32;
        let r = radius * (1.0 - 0.82 * t);
        let shift = Point::new(centre.x - radius * 0.22 * t, centre.y - radius * 0.3 * t);
        let shade = 0.115 + 0.13 * t * t + glow;
        frame.fill(&Path::circle(shift, r), Color::from_rgb(shade, shade + 0.004, shade + 0.014));
    }
    let edge = Path::circle(centre, radius - 0.6);
    frame.stroke(&edge, Stroke { style: canvas::Style::Gradient(vertical(Point::new(centre.x, centre.y - radius), Point::new(centre.x, centre.y + radius), Color::from_rgba(1.0, 1.0, 1.0, 0.2), Color::from_rgba(1.0, 1.0, 1.0, 0.0)).into()), ..Stroke::default().with_width(1.0) });
    if radius >= 14.0 {
        let grips = 28;
        for grip in 0..grips {
            let a = angle + grip as f32 / grips as f32 * 2.0 * PI;
            let from = Point::new(centre.x + (radius - 3.2) * a.cos(), centre.y + (radius - 3.2) * a.sin());
            let to = Point::new(centre.x + (radius - 1.2) * a.cos(), centre.y + (radius - 1.2) * a.sin());
            frame.stroke(&Path::line(from, to), Stroke::default().with_color(Color::from_rgba(0.0, 0.0, 0.0, 0.32)).with_width(1.0));
        }
    }
}

struct Dial<M> {
    param: Param,
    value: f32,
    look: Look,
    index: usize,
    size: KnobSize,
    show_value: bool,
    change: fn(usize, f32) -> M,
}

#[derive(Default)]
pub struct Turning {
    from: Option<(f32, f32)>,
    last_press: Option<Instant>,
    modifiers: keyboard::Modifiers,
}

impl<M> Dial<M> {
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
        let accent = look.accent;
        let mut frame = Frame::new(renderer, bounds.size());
        let width = bounds.width;
        let size = self.size;
        let dragging = turning.from.is_some();
        let lit = dragging || cursor.is_over(bounds);
        let ring = size.ring();
        let cap = size.cap();
        let centre = match size {
            KnobSize::Mini => Point::new(ring + 3.0, bounds.height / 2.0),
            KnobSize::Hero => Point::new(width / 2.0, 17.0 + ring + 4.0),
            KnobSize::Regular => Point::new(width / 2.0, 15.0 + ring + 3.0),
        };
        let position = self.param.to_position(self.value);
        let bipolar = self.param.min < 0.0 && self.param.max > 0.0;
        let zero = if bipolar { self.param.to_position(0.0) } else { 0.0 };

        if size != KnobSize::Mini {
            let ticks = if size == KnobSize::Hero { 21 } else { 11 };
            let (inner, outer) = if size == KnobSize::Hero { (ring + 5.0, ring + 8.5) } else { (ring + 4.0, ring + 6.5) };
            for tick in 0..ticks {
                let share = tick as f32 / (ticks - 1) as f32;
                let a = self.angle(share);
                let major = tick == 0 || tick == ticks - 1 || (bipolar && (share - zero).abs() < 0.01) || (size == KnobSize::Hero && tick % 5 == 0);
                let reach = if major { outer } else { outer - 1.5 };
                let from = Point::new(centre.x + inner * a.cos(), centre.y + inner * a.sin());
                let to = Point::new(centre.x + reach * a.cos(), centre.y + reach * a.sin());
                let passed = (share >= zero.min(position) - 0.001) && (share <= zero.max(position) + 0.001);
                let colour = if passed && position != zero { fade(accent, 0.55) } else { fade(look.text, if major { 0.3 } else { 0.14 }) };
                frame.stroke(&Path::line(from, to), Stroke::default().with_color(colour).with_width(1.0));
            }
        }
        let track = if size == KnobSize::Hero { 4.0 } else if size == KnobSize::Regular { 3.0 } else { 2.5 };
        frame.stroke(&arc(centre, ring, START, START + SWEEP), round(Color::from_rgba(0.0, 0.0, 0.0, 0.62), track + 1.5));
        frame.stroke(&arc(centre, ring, START, START + SWEEP), round(fade(look.text, 0.06), track));
        let (from, to) = (self.angle(zero.min(position)), self.angle(zero.max(position)));
        if to - from > 0.001 {
            let value_arc = arc(centre, ring, from, to);
            frame.stroke(&value_arc, round(fade(accent, if dragging { 0.22 } else { 0.13 }), track + 6.0));
            frame.stroke(&value_arc, round(fade(accent, 0.3), track + 2.5));
            frame.stroke(&value_arc, round(accent, track - 0.5));
        }
        let angle = self.angle(position);
        draw_cap(&mut frame, centre, cap, angle, lit);
        let along = |reach: f32| Point::new(centre.x + reach * angle.cos(), centre.y + reach * angle.sin());
        let (near, far, thick) = match size {
            KnobSize::Hero => (cap * 0.42, cap - 3.5, 2.0),
            KnobSize::Regular => (cap * 0.38, cap - 2.8, 1.7),
            KnobSize::Mini => (cap * 0.25, cap - 1.5, 1.5),
        };
        frame.stroke(&Path::line(along(near), along(far)), round(Color::from_rgba(0.0, 0.0, 0.0, 0.5), thick + 1.5));
        frame.stroke(&Path::line(along(near), along(far)), round(Color::from_rgb(0.97, 0.97, 0.98), thick));

        let label = self.param.name.to_uppercase();
        let label_colour = if lit { look.text } else { look.text_dim };
        let reading = shown(&self.param, self.value);
        let value_colour = if dragging { accent } else if lit { look.text } else { mix(look.text_dim, look.text, 0.62) };
        match size {
            KnobSize::Mini => {
                let x = centre.x + ring + 8.0;
                put_text_in(&mut frame, &label, Point::new(x, centre.y - 1.0), 9.0, label_colour, (0.0, 1.0), SEMIBOLD);
                if self.show_value {
                    put_text_in(&mut frame, &reading, Point::new(x, centre.y + 1.0), 11.5, value_colour, (0.0, 0.0), MEDIUM);
                }
            }
            KnobSize::Hero => {
                put_text_in(&mut frame, &label, Point::new(width / 2.0, 0.0), 10.5, label_colour, (0.5, 0.0), SEMIBOLD);
                if self.show_value {
                    put_text_in(&mut frame, &reading, Point::new(width / 2.0, bounds.height - 1.0), 13.5, value_colour, (0.5, 1.0), MEDIUM);
                }
            }
            KnobSize::Regular => {
                put_text_in(&mut frame, &label, Point::new(width / 2.0, 0.0), LABEL_SIZE, label_colour, (0.5, 0.0), SEMIBOLD);
                if self.show_value {
                    put_text_in(&mut frame, &reading, Point::new(width / 2.0, bounds.height - 1.0), 11.5, value_colour, (0.5, 1.0), MEDIUM);
                }
            }
        }
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

/// A control together with the width it needs, so a deck can lay out and balance
/// its sections before iced measures anything.
pub struct Part<'a, M> {
    pub element: Element<'a, M>,
    pub width: f32,
    pub height: f32,
}

impl<'a, M: 'a> Part<'a, M> {
    pub fn new(element: impl Into<Element<'a, M>>, width: f32) -> Self {
        Self { element: element.into(), width, height: 30.0 }
    }

    pub fn tall(mut self, height: f32) -> Self {
        self.height = height;
        self
    }
}

/// Builds knobs for one plugin: its look, its parameters and values, and whether
/// the window is short enough that hero knobs should shrink.
pub struct Panel<'v, M> {
    pub look: Look,
    pub compact: bool,
    pub params: &'static [Param],
    pub values: &'v [f32],
    pub change: fn(usize, f32) -> M,
}

impl<M> Clone for Panel<'_, M> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<M> Copy for Panel<'_, M> {}

impl<'v, M: 'v> Panel<'v, M> {
    pub fn new(look: Look, compact: bool, params: &'static [Param], values: &'v [f32], change: fn(usize, f32) -> M) -> Self {
        Self { look, compact, params, values, change }
    }

    pub fn sized(&self, index: usize, size: KnobSize, label: Option<&'static str>, show_value: bool) -> Part<'v, M> {
        let mut param = self.params[index];
        if let Some(label) = label {
            param.name = label;
        }
        let size = if self.compact && size == KnobSize::Hero { KnobSize::Regular } else { size };
        let width = match size {
            KnobSize::Mini => size.min_width().max(ring_room(size) + caps_width(param.name)),
            _ => (caps_width(param.name) + 10.0).max(size.min_width()),
        };
        let dial = Dial { param, value: self.values[index], look: self.look, index, size, show_value, change: self.change };
        let height = if show_value || size == KnobSize::Mini { size.height() } else { size.height() - 15.0 };
        Part::new(canvas::Canvas::new(dial).width(width).height(height), width).tall(height)
    }

    pub fn knob(&self, index: usize) -> Part<'v, M> {
        self.sized(index, KnobSize::Regular, None, true)
    }

    pub fn hero(&self, index: usize) -> Part<'v, M> {
        self.sized(index, KnobSize::Hero, None, true)
    }

    pub fn named(&self, index: usize, label: &'static str) -> Part<'v, M> {
        self.sized(index, KnobSize::Regular, Some(label), true)
    }

    pub fn mini(&self, index: usize, label: &'static str) -> Part<'v, M> {
        self.sized(index, KnobSize::Mini, Some(label), true)
    }
}

fn ring_room(size: KnobSize) -> f32 {
    size.ring() * 2.0 + 16.0
}

/// A small upper case caption, the one used over every section and field.
pub fn caption<'a, M: 'a>(look: Look, words: &str) -> Element<'a, M> {
    text(words.to_uppercase()).size(LABEL_SIZE).font(SEMIBOLD).color(look.text_faint).wrapping(text::Wrapping::None).into()
}

fn led<'a, M: 'a>(look: Look, on: bool) -> Element<'a, M> {
    let colour = look.accent;
    container(Space::new(6, 6))
        .style(move |_| container::Style {
            background: Some(if on { colour } else { Color::from_rgba(0.0, 0.0, 0.0, 0.55) }.into()),
            border: iced::Border { color: if on { lift(colour, 0.15) } else { fade(look.text, 0.12) }, width: 1.0, radius: 3.5.into() },
            shadow: iced::Shadow { color: if on { fade(colour, 0.8) } else { Color::TRANSPARENT }, offset: iced::Vector::ZERO, blur_radius: if on { 7.0 } else { 0.0 } },
            ..Default::default()
        })
        .into()
}

fn key_style(look: Look, on: bool, hovered: bool) -> button::Style {
    let top = if hovered { 0.215 } else { 0.185 };
    button::Style {
        background: Some(iced::gradient::Linear::new(iced::Radians(PI)).add_stop(0.0, Color::from_rgb(top, top + 0.004, top + 0.016)).add_stop(1.0, Color::from_rgb(top - 0.05, top - 0.046, top - 0.034)).into()),
        text_color: if on || hovered { look.text } else { look.text_dim },
        border: iced::Border { color: Color::from_rgba(0.0, 0.0, 0.0, 0.55), width: 1.0, radius: 6.0.into() },
        shadow: iced::Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.35), offset: iced::Vector::new(0.0, 1.5), blur_radius: 2.5 },
    }
}

fn label_width(label: &str) -> f32 {
    label.chars().count() as f32 * 6.9
}

/// A latching switch: a raised key with a small lamp that glows while it is on.
pub fn toggle<'a, M: Clone + 'a>(look: Look, label: impl Into<String>, on: bool, message: M) -> Part<'a, M> {
    let label = label.into();
    let width = label_width(&label) + 46.0;
    let face = row![led(look, on), text(label).size(11.5).font(MEDIUM).wrapping(text::Wrapping::None)].spacing(9).align_y(Alignment::Center);
    let element = button(face).padding([7, 13]).style(move |_, status| key_style(look, on, matches!(status, button::Status::Hovered | button::Status::Pressed))).on_press(message);
    Part::new(element, width)
}

/// A plain push key in the same family as [`toggle`].
pub fn push<'a, M: Clone + 'a>(look: Look, label: impl Into<String>, message: Option<M>) -> Part<'a, M> {
    let label = label.into();
    let width = label_width(&label) + 28.0;
    let element = button(text(label).size(11.5).font(MEDIUM).wrapping(text::Wrapping::None))
        .padding([7, 13])
        .style(move |_, status| {
            let mut style = key_style(look, false, matches!(status, button::Status::Hovered | button::Status::Pressed));
            if matches!(status, button::Status::Disabled) {
                style.text_color = look.text_faint;
            }
            style
        })
        .on_press_maybe(message);
    Part::new(element, width)
}

/// The filled call to action, used once per window at most.
pub fn primary<'a, M: Clone + 'a>(look: Look, label: &'static str, message: Option<M>) -> Element<'a, M> {
    button(text(label).size(12.5).font(SEMIBOLD).wrapping(text::Wrapping::None))
        .padding([8, 18])
        .style(move |_, status| {
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            let disabled = matches!(status, button::Status::Disabled);
            let fill = if disabled { fade(look.accent, 0.25) } else if hovered { lift(look.accent, 0.05) } else { look.accent };
            button::Style {
                background: Some(iced::gradient::Linear::new(iced::Radians(PI)).add_stop(0.0, lift(fill, 0.06)).add_stop(1.0, lift(fill, -0.06)).into()),
                text_color: if disabled { fade(look.on_accent, 0.6) } else { look.on_accent },
                border: iced::Border::default().rounded(7),
                shadow: iced::Shadow { color: fade(look.accent, if disabled { 0.0 } else { 0.25 }), offset: iced::Vector::new(0.0, 1.0), blur_radius: 8.0 },
            }
        })
        .on_press_maybe(message)
        .into()
}

pub fn field_style(look: Look) -> impl Fn(&Theme, pick_list::Status) -> pick_list::Style {
    move |_, status| {
        let hovered = !matches!(status, pick_list::Status::Active);
        pick_list::Style {
            text_color: look.text,
            placeholder_color: look.text_faint,
            handle_color: if hovered { look.accent } else { look.text_dim },
            background: Color::from_rgba(0.0, 0.0, 0.0, if hovered { 0.36 } else { 0.3 }).into(),
            border: iced::Border { color: Color::from_rgba(0.0, 0.0, 0.0, 0.0), width: 0.0, radius: 6.0.into() },
        }
    }
}

pub fn menu_style(look: Look) -> impl Fn(&Theme) -> iced::overlay::menu::Style {
    move |_| iced::overlay::menu::Style {
        background: Color::from_rgb(0.13, 0.135, 0.15).into(),
        border: iced::Border { color: Color::from_rgba(0.0, 0.0, 0.0, 0.6), width: 1.0, radius: 6.0.into() },
        text_color: look.text,
        selected_text_color: look.text,
        selected_background: fade(look.accent, 0.22).into(),
    }
}

/// A pick list for a parameter with named choices, captioned with the parameter's name.
pub fn choice<'a, M: Clone + 'a>(look: Look, param: Param, value: f32, change: fn(f32) -> M) -> Part<'a, M> {
    Part::new(column![caption(look, param.name), pick(look, param, value, change).element].spacing(6), CHOICE_WIDTH).tall(50.0)
}

/// The same list without a caption, for a section whose title already names it.
pub fn pick<'a, M: Clone + 'a>(look: Look, param: Param, value: f32, change: fn(f32) -> M) -> Part<'a, M> {
    let current = param.chosen(value);
    let list = pick_list(param.choices, current, move |picked: &'static str| {
        let index = param.choices.iter().position(|choice| *choice == picked).unwrap_or(0);
        change(index as f32)
    })
    .text_size(12)
    .font(MEDIUM)
    .padding([6, 10])
    .width(CHOICE_WIDTH)
    .style(field_style(look))
    .menu_style(menu_style(look));
    Part::new(list, CHOICE_WIDTH)
}

/// A row of joined keys choosing one of a few values.
pub fn segments<'a, M: Clone + 'a>(look: Look, options: Vec<(String, M, bool)>) -> Part<'a, M> {
    let count = options.len();
    let width = count as f32 * 34.0;
    let buttons = options.into_iter().enumerate().map(|(at, (label, message, chosen))| {
        let radius = iced::border::Radius {
            top_left: if at == 0 { 6.0 } else { 0.0 },
            bottom_left: if at == 0 { 6.0 } else { 0.0 },
            top_right: if at + 1 == count { 6.0 } else { 0.0 },
            bottom_right: if at + 1 == count { 6.0 } else { 0.0 },
        };
        button(container(text(label).size(12).font(MEDIUM)).center_x(Length::Fill))
            .width(34)
            .padding([7, 0])
            .style(move |_, status| {
                let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                let mut style = key_style(look, chosen, hovered);
                if chosen {
                    style.background = Some(Color::from_rgba(0.0, 0.0, 0.0, 0.42).into());
                    style.text_color = look.accent;
                    style.shadow = iced::Shadow::default();
                }
                style.border.radius = radius;
                style
            })
            .on_press(message)
            .into()
    });
    Part::new(row(buttons), width)
}

/// A stack of switches or lists inside one section.
pub fn stack<'a, M: 'a>(parts: Vec<Part<'a, M>>) -> Part<'a, M> {
    let width = parts.iter().map(|part| part.width).fold(0.0, f32::max);
    let height = parts.iter().map(|part| part.height).sum::<f32>() + 10.0 * parts.len().saturating_sub(1) as f32;
    Part::new(column(parts.into_iter().map(|part| part.element)).spacing(10).align_x(Alignment::Center), width).tall(height)
}

/// A titled run of controls on the deck. No box: sections are told apart by space,
/// their caption and a faint divider.
pub fn section<'a, M: 'a>(look: Look, title: &str, parts: Vec<Part<'a, M>>) -> Part<'a, M> {
    let width = parts.iter().map(|part| part.width).sum::<f32>() + ITEM_GAP * parts.len().saturating_sub(1) as f32;
    let width = width.max(caps_width(title));
    let height = parts.iter().map(|part| part.height).fold(0.0, f32::max) + SECTION_TITLE;
    let body = row(parts.into_iter().map(|part| part.element)).spacing(ITEM_GAP).align_y(Alignment::Center);
    let title = row![rule(look), caption(look, title), rule(look)].spacing(8).align_y(Alignment::Center);
    let body = container(body).height(Length::Fill).center_x(Length::Fill).align_y(alignment::Vertical::Center);
    Part::new(column![title, body].spacing(10).align_x(Alignment::Center).height(Length::Fill), width).tall(height)
}

fn rule<'a, M: 'a>(look: Look) -> Element<'a, M> {
    container(Space::new(Length::Fill, 1))
        .width(Length::Fill)
        .style(move |_| container::Style { background: Some(fade(look.text, 0.06).into()), ..Default::default() })
        .into()
}

fn divider<'a, M: 'a>(look: Look, tall: f32) -> Element<'a, M> {
    container(Space::new(1, tall))
        .style(move |_| container::Style {
            background: Some(
                iced::gradient::Linear::new(iced::Radians(PI))
                    .add_stop(0.0, fade(look.text, 0.0))
                    .add_stop(0.5, fade(look.text, 0.08))
                    .add_stop(1.0, fade(look.text, 0.0))
                    .into(),
            ),
            ..Default::default()
        })
        .into()
}

/// One line of sections. When the width allows, the sections share it in proportion
/// to what they hold, each centred in its share; otherwise the line scrolls sideways.
pub fn deck_line<'a, M: 'a>(look: Look, parts: Vec<Part<'a, M>>, room: f32) -> Element<'a, M> {
    let natural: f32 = parts.iter().map(|part| part.width).sum::<f32>() + SECTION_GAP * parts.len().saturating_sub(1) as f32;
    let count = parts.len();
    let tall = parts.iter().map(|part| part.height).fold(0.0, f32::max);
    if natural <= room {
        let mut line = row![].align_y(Alignment::Center);
        for (at, part) in parts.into_iter().enumerate() {
            let share = (part.width.max(40.0) + SECTION_GAP) as u16;
            line = line.push(container(part.element).center_x(Length::FillPortion(share)).height(Length::Fill));
            if at + 1 < count {
                line = line.push(divider(look, 64.0));
            }
        }
        line.width(Length::Fill).height(tall).into()
    } else {
        let mut line = row![].spacing(SECTION_GAP / 2.0).align_y(Alignment::Center).height(tall);
        for (at, part) in parts.into_iter().enumerate() {
            line = line.push(container(part.element).width(part.width).height(Length::Fill));
            if at + 1 < count {
                line = line.push(divider(look, 64.0));
            }
        }
        sideways(look, line.into())
    }
}

/// A thin, quiet horizontal scroller for a row of controls that does not fit.
pub fn sideways<'a, M: 'a>(look: Look, content: Element<'a, M>) -> Element<'a, M> {
    use iced::widget::scrollable;
    scrollable(content)
        .direction(scrollable::Direction::Horizontal(scrollable::Scrollbar::new().width(3).scroller_width(3).spacing(6)))
        .width(Length::Fill)
        .style(move |_, status| {
            let hovered = matches!(status, scrollable::Status::Hovered { .. } | scrollable::Status::Dragged { .. });
            let rail = scrollable::Rail {
                background: Some(fade(look.text, 0.03).into()),
                border: iced::Border::default().rounded(2),
                scroller: scrollable::Scroller { color: fade(look.text, if hovered { 0.3 } else { 0.16 }), border: iced::Border::default().rounded(2) },
            };
            scrollable::Style { container: container::Style::default(), vertical_rail: rail, horizontal_rail: rail, gap: None }
        })
        .into()
}

/// The faceplate under the display: a softly lit tonal plate holding lines of sections.
pub fn deck_plate<'a, M: 'a>(look: Look, content: Element<'a, M>) -> Element<'a, M> {
    container(content)
        .padding(iced::Padding { top: 14.0, right: 18.0, bottom: 14.0, left: 18.0 })
        .width(Length::Fill)
        .style(move |_| container::Style {
            background: Some(iced::gradient::Linear::new(iced::Radians(PI)).add_stop(0.0, lift(look.deck, 0.018)).add_stop(1.0, lift(look.deck, -0.02)).into()),
            ..Default::default()
        })
        .into()
}

/// The strip across the top of every plugin window: an accent mark and the name.
pub fn header<'a, M: 'a>(look: Look, name: &'static str, kind: &'static str) -> Element<'a, M> {
    let accent = look.accent;
    let mark = container(Space::new(9, 9)).style(move |_| container::Style {
        background: Some(iced::gradient::Linear::new(iced::Radians(PI * 0.75)).add_stop(0.0, lift(accent, 0.12)).add_stop(1.0, lift(accent, -0.12)).into()),
        border: iced::Border::default().rounded(2.5),
        shadow: iced::Shadow { color: fade(accent, 0.55), offset: iced::Vector::ZERO, blur_radius: 8.0 },
        ..Default::default()
    });
    let title = row![
        mark,
        text("LOUPE").size(11.5).font(MEDIUM).color(look.text_faint),
        text(name).size(13.5).font(SEMIBOLD).color(look.text),
        Space::with_width(Length::Fill),
        text(kind.to_uppercase()).size(9.5).font(SEMIBOLD).color(look.text_faint).wrapping(text::Wrapping::None),
    ]
    .spacing(9)
    .align_y(Alignment::Center);
    container(title)
        .padding([0, 16])
        .height(HEADER_HEIGHT)
        .width(Length::Fill)
        .center_y(HEADER_HEIGHT)
        .style(move |_| container::Style {
            background: Some(iced::gradient::Linear::new(iced::Radians(PI)).add_stop(0.0, lift(look.deck, 0.035)).add_stop(1.0, lift(look.deck, 0.005)).into()),
            border: iced::Border { color: Color::from_rgba(0.0, 0.0, 0.0, 0.55), width: 0.0, radius: 0.0.into() },
            shadow: iced::Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.5), offset: iced::Vector::new(0.0, 1.0), blur_radius: 4.0 },
            ..Default::default()
        })
        .into()
}


/// A tonal panel for widget content: lifted from the plate, lit along the top, no outline.
pub fn card<'a, M: 'a>(look: Look, content: impl Into<Element<'a, M>>) -> container::Container<'a, M> {
    container(content).padding([12, 14]).style(move |_| container::Style {
        background: Some(iced::gradient::Linear::new(iced::Radians(PI)).add_stop(0.0, lift(look.background, 0.03)).add_stop(1.0, look.well_low).into()),
        border: iced::Border { color: Color::from_rgba(0.0, 0.0, 0.0, 0.6), width: 1.0, radius: 8.0.into() },
        shadow: iced::Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.3), offset: iced::Vector::new(0.0, 1.0), blur_radius: 3.0 },
        ..Default::default()
    })
}
/// The recessed screen holding a plugin's graph.
pub fn display<'a, M: 'a>(look: Look, content: impl Into<Element<'a, M>>) -> Element<'a, M> {
    container(content)
        .padding(iced::Padding { top: 12.0, right: 12.0, bottom: 0.0, left: 12.0 })
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| container::Style { background: Some(lift(look.deck, 0.018).into()), ..Default::default() })
        .into()
}

/// The whole window: header, display, and the deck. Display and deck are built on
/// demand so the layout can follow the room the window gives it.
pub fn shell<'a, M: 'a>(
    look: Look,
    name: &'static str,
    kind: &'static str,
    graph: impl Fn() -> Element<'a, M> + 'a,
    lines: impl Fn(bool) -> Vec<Vec<Part<'a, M>>> + 'a,
) -> Element<'a, M> {
    iced::widget::responsive(move |size| {
        let compact = size.height < COMPACT_BELOW;
        let mut lines = lines(compact);
        if compact && lines.len() > 1 {
            lines = vec![lines.into_iter().flatten().collect()];
        }
        let room = size.width - 36.0;
        let deck = column(lines.into_iter().map(|line| deck_line(look, line, room))).spacing(14);
        column![header(look, name, kind), display(look, graph()), deck_plate(look, deck.into())].into()
    })
    .into()
}

pub fn roomy(size: Size) -> bool {
    size.width >= 8.0 && size.height >= 8.0 && size.width.is_finite() && size.height.is_finite()
}

fn well(size: Size) -> Path {
    Path::rounded_rectangle(Point::ORIGIN, size, WELL_RADIUS.into())
}

/// Starts a graph: the deep, softly lit screen every display draws on.
pub fn graph_frame(renderer: &Renderer, size: Size, look: &Look) -> Frame {
    let mut frame = Frame::new(renderer, size);
    frame.fill(&well(size), vertical(Point::ORIGIN, Point::new(0.0, size.height), look.background, look.well_low));
    frame
}

fn edge_shade(frame: &mut Frame, size: Size, from: Point, to: Point, reach: f32, strength: f32) {
    let length = from.distance(to).max(1.0);
    let depth = (reach / length).min(1.0);
    let shade = gradient::Linear::new(from, to)
        .add_stop(0.0, Color::from_rgba(0.0, 0.0, 0.0, strength))
        .add_stop(depth, Color::from_rgba(0.0, 0.0, 0.0, 0.0))
        .add_stop(1.0, Color::from_rgba(0.0, 0.0, 0.0, 0.0));
    frame.fill(&well(size), shade);
}

/// Finishes a graph: a vignette darkening every edge, a deeper shadow under the top
/// lip and a fine rim.
pub fn seal(frame: &mut Frame, size: Size, look: &Look) {
    let (w, h) = (size.width, size.height);
    edge_shade(frame, size, Point::new(0.0, 0.0), Point::new(0.0, h), 22.0, 0.45);
    edge_shade(frame, size, Point::new(0.0, h), Point::new(0.0, 0.0), 40.0, 0.28);
    edge_shade(frame, size, Point::new(0.0, 0.0), Point::new(w, 0.0), 70.0, 0.25);
    edge_shade(frame, size, Point::new(w, 0.0), Point::new(0.0, 0.0), 70.0, 0.25);
    let rim = Path::rounded_rectangle(Point::new(0.5, 0.5), Size::new(w - 1.0, h - 1.0), (WELL_RADIUS - 0.5).into());
    frame.stroke(&rim, Stroke::default().with_color(Color::from_rgba(0.0, 0.0, 0.0, 0.7)).with_width(1.0));
    let lip = Path::line(Point::new(WELL_RADIUS, h - 0.5), Point::new(w - WELL_RADIUS, h - 0.5));
    frame.stroke(&lip, Stroke::default().with_color(fade(look.text, 0.06)).with_width(1.0));
}

/// A tonal panel inside a graph, for readouts and side panels: lighter than the
/// screen, with a lit top edge and no outline.
pub fn draw_card(frame: &mut Frame, at: Point, size: Size, look: &Look) {
    let body = Path::rounded_rectangle(at, size, CARD_RADIUS.into());
    frame.fill(&Path::rounded_rectangle(Point::new(at.x, at.y + 2.0), size, CARD_RADIUS.into()), Color::from_rgba(0.0, 0.0, 0.0, 0.35));
    frame.fill(&body, vertical(at, Point::new(at.x, at.y + size.height), fade(lift(look.panel, -0.02), 0.92), fade(lift(look.panel, -0.06), 0.92)));
    let top = Path::line(Point::new(at.x + CARD_RADIUS, at.y + 0.5), Point::new(at.x + size.width - CARD_RADIUS, at.y + 0.5));
    frame.stroke(&top, Stroke::default().with_color(fade(look.text, 0.07)).with_width(1.0));
}

/// A horizontal guide line across a graph.
pub fn guide(frame: &mut Frame, from: Point, length: f32, colour: Color) {
    frame.fill_rectangle(Point::new(from.x, from.y.round()), Size::new(length, 1.0), colour);
}

/// A dashed horizontal line, used for thresholds and targets.
pub fn dashed(frame: &mut Frame, from: Point, length: f32, colour: Color) {
    let y = from.y.round() + 0.5;
    let line = Path::line(Point::new(from.x, y), Point::new(from.x + length, y));
    frame.stroke(&line, Stroke { line_dash: LineDash { segments: &[4.0, 4.0], offset: 0 }, ..Stroke::default().with_color(colour).with_width(1.0) });
}

/// Fills the area between a traced line and a base line with a fade that is
/// strongest at the line and clear at the base.
pub fn fill_under(frame: &mut Frame, points: &[Point], base: f32, near_colour: Color, base_colour: Color) {
    if points.len() < 2 {
        return;
    }
    let far = points.iter().map(|p| p.y).fold(base, |far, y| if (y - base).abs() > (far - base).abs() { y } else { far });
    if (far - base).abs() < 0.5 {
        return;
    }
    let shape = Path::new(|b| {
        b.move_to(Point::new(points[0].x, base));
        for point in points {
            b.line_to(*point);
        }
        b.line_to(Point::new(points[points.len() - 1].x, base));
        b.close();
    });
    frame.fill(&shape, vertical(Point::new(0.0, far), Point::new(0.0, base), near_colour, base_colour));
}

pub fn polyline(points: &[Point]) -> Path {
    Path::new(|b| {
        if let Some(first) = points.first() {
            b.move_to(*first);
            for point in &points[1..] {
                b.line_to(*point);
            }
        }
    })
}

/// A value shown in a readout panel.
pub struct Reading {
    pub label: &'static str,
    pub value: String,
    pub unit: &'static str,
    pub colour: Color,
}

impl Reading {
    pub fn new(label: &'static str, value: String, unit: &'static str, colour: Color) -> Self {
        Self { label, value, unit, colour }
    }
}

/// A readout panel: one large reading, an optional bar meter under it, then smaller
/// rows. The bar follows its share from 0 to 1 smoothly, with no peak flashing.
pub fn draw_readouts(frame: &mut Frame, size: Size, look: &Look, main: &Reading, bar: Option<(f32, Color)>, rest: &[Reading]) {
    draw_card(frame, Point::ORIGIN, size, look);
    let middle = size.width / 2.0;
    put_text_in(frame, &main.label.to_uppercase(), Point::new(middle, 16.0), LABEL_SIZE, look.text_dim, (0.5, 0.0), SEMIBOLD);
    put_text_in(frame, &main.value, Point::new(middle, 31.0), 32.0, main.colour, (0.5, 0.0), MEDIUM);
    put_text_in(frame, main.unit, Point::new(middle, 70.0), 10.0, look.text_faint, (0.5, 0.0), SEMIBOLD);
    let mut y = 92.0;
    if let Some((share, colour)) = bar {
        let inset = 16.0;
        let length = (size.width - inset * 2.0).max(1.0);
        let groove = Path::rounded_rectangle(Point::new(inset, y), Size::new(length, 4.0), 2.0.into());
        frame.fill(&groove, Color::from_rgba(0.0, 0.0, 0.0, 0.5));
        let filled = share.clamp(0.0, 1.0) * length;
        if filled >= 1.0 {
            let lit = Path::rounded_rectangle(Point::new(inset, y), Size::new(filled.max(4.0), 4.0), 2.0.into());
            frame.fill(&Path::rounded_rectangle(Point::new(inset, y - 2.0), Size::new(filled.max(4.0), 8.0), 4.0.into()), fade(colour, 0.15));
            frame.fill(&lit, gradient::Linear::new(Point::new(inset, 0.0), Point::new(inset + length, 0.0)).add_stop(0.0, fade(colour, 0.5)).add_stop(1.0, colour));
        }
        y += 20.0;
    } else {
        y += 6.0;
    }
    let stacked = size.width < 180.0;
    let row_height = if stacked { 48.0 } else { 36.0 };
    for reading in rest {
        if y + row_height - 4.0 > size.height {
            break;
        }
        guide(frame, Point::new(14.0, y), size.width - 28.0, fade(look.text, 0.05));
        put_text_in(frame, &reading.label.to_uppercase(), Point::new(16.0, y + 10.0), 9.0, look.text_faint, (0.0, 0.0), SEMIBOLD);
        let value = if reading.unit.is_empty() { reading.value.clone() } else { format!("{} {}", reading.value, reading.unit) };
        if stacked {
            put_text_in(frame, &value, Point::new(16.0, y + 23.0), 15.0, reading.colour, (0.0, 0.0), MEDIUM);
        } else {
            put_text_in(frame, &value, Point::new(size.width - 16.0, y + 8.0), 14.0, reading.colour, (1.0, 0.0), MEDIUM);
        }
        y += row_height;
    }
}

/// Axis text inside a graph, in small capitals.
pub fn axis_text(frame: &mut Frame, content: &str, at: Point, look: &Look, anchor: (f32, f32)) {
    put_text_in(frame, &content.to_uppercase(), at, 9.0, look.text_faint, anchor, SEMIBOLD);
}

pub fn read_value(typed: &str, param: &Param) -> Option<f32> {
    let cleaned: String = typed.chars().filter(|c| !c.is_whitespace()).collect();
    let lower = cleaned.to_lowercase();
    let body = lower
        .trim_end_matches("hz")
        .trim_end_matches("db")
        .trim_end_matches("ms")
        .trim_end_matches('%')
        .trim_end_matches("st")
        .trim_end_matches(':')
        .trim_end_matches('1')
        .trim_end_matches(':');
    let body = if body.is_empty() { lower.as_str() } else { body };
    let (digits, times) = match body.strip_suffix('k') {
        Some(rest) => (rest, 1000.0),
        None => match body.strip_suffix('s') {
            Some(rest) if param.unit == Unit::Milliseconds => (rest, 1000.0),
            _ => (body, 1.0),
        },
    };
    let digits = digits.trim_start_matches('+');
    let read: f32 = digits.parse().ok()?;
    let found = read * times;
    found.is_finite().then(|| found.clamp(param.min, param.max))
}

#[cfg(test)]
mod reading_tests {
    use super::*;

    fn hz() -> Param {
        Param::new("freq", "Frequency", 20.0, 20_000.0, 1000.0, Unit::Hertz)
    }

    fn db() -> Param {
        Param::new("gain", "Gain", -30.0, 30.0, 0.0, Unit::Decibels)
    }

    #[test]
    fn a_k_means_a_thousand() {
        assert_eq!(read_value("10k", &hz()), Some(10_000.0));
        assert_eq!(read_value("1.5k", &hz()), Some(1500.0));
        assert_eq!(read_value("10K", &hz()), Some(10_000.0));
        assert_eq!(read_value(" 4.5 k ", &hz()), Some(4500.0));
    }

    #[test]
    fn the_unit_can_be_typed_or_left_out() {
        assert_eq!(read_value("440", &hz()), Some(440.0));
        assert_eq!(read_value("440hz", &hz()), Some(440.0));
        assert_eq!(read_value("440 Hz", &hz()), Some(440.0));
        assert_eq!(read_value("-6 dB", &db()), Some(-6.0));
        assert_eq!(read_value("+3dB", &db()), Some(3.0));
    }

    #[test]
    fn what_is_typed_stays_inside_the_knob() {
        assert_eq!(read_value("99k", &hz()), Some(20_000.0));
        assert_eq!(read_value("1", &hz()), Some(20.0));
        assert_eq!(read_value("-99", &db()), Some(-30.0));
    }

    #[test]
    fn nonsense_is_refused() {
        assert_eq!(read_value("", &hz()), None);
        assert_eq!(read_value("loud", &hz()), None);
        assert_eq!(read_value("k", &hz()), None);
    }
}
