use std::time::Instant;

use iced::widget::canvas::{self, path, Frame, Geometry, Path, Stroke};
use iced::{mouse, Point, Radians, Rectangle, Renderer, Theme};

use crate::theme::Palette;

const TURNS_PER_SECOND: f32 = 0.9;
const ARC_LENGTH: f32 = 1.5 * std::f32::consts::PI;
const THICKNESS: f32 = 3.5;

pub struct Spinner<'a> {
    pub palette: &'a Palette,
    pub since: Instant,
}

impl<Message> canvas::Program<Message> for Spinner<'_> {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let centre = Point::new(bounds.width / 2.0, bounds.height / 2.0);
        let radius = bounds.width.min(bounds.height) / 2.0 - THICKNESS;
        let turned = self.since.elapsed().as_secs_f32() * TURNS_PER_SECOND * std::f32::consts::TAU;
        let ring = Path::circle(centre, radius);
        frame.stroke(&ring, Stroke::default().with_color(self.palette.hover).with_width(THICKNESS));
        let sweep = Path::new(|b| {
            b.arc(path::Arc {
                center: centre,
                radius,
                start_angle: Radians(turned),
                end_angle: Radians(turned + ARC_LENGTH),
            });
        });
        frame.stroke(&sweep, Stroke::default().with_color(self.palette.accent).with_width(THICKNESS));
        vec![frame.into_geometry()]
    }
}
