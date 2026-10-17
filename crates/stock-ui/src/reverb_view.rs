use iced::widget::canvas::{self, Geometry, Path, Stroke};
use iced::{mouse, Element, Length, Point, Rectangle, Renderer, Size, Theme};
use loupe_stock::{decay_seconds, Reverb};

use crate::compressor_view::INSET;
use crate::kit::*;
use crate::knobs::{change, Change, Knobs};
use crate::look::{fade, Look, MEDIUM, SEMIBOLD};

const LOWEST_HZ: f32 = 20.0;
const HIGHEST_HZ: f32 = 20_000.0;
const LABELLED_HZ: [f32; 7] = [50.0, 100.0, 200.0, 500.0, 1000.0, 5000.0, 10_000.0];

pub struct ReverbEditor {
    knobs: Knobs,
    look: Look,
}

impl ReverbEditor {
    pub fn new(look: Look) -> Self {
        let look = look.tinted(crate::look::accents::REVERB);
        Self { knobs: Knobs::of(&Reverb::new()), look }
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
        shell(look, "Reverb", "Algorithmic reverb", move || canvas::Canvas::new(Decay { editor: self }).width(Length::Fill).height(Length::Fill).into(), move |compact| {
            let p = Panel::new(look, compact, params, values, change);
            vec![vec![
                section(look, "Time", vec![p.knob(0), p.hero(2), p.knob(8)]),
                section(look, "Space", vec![p.hero(1), p.knob(5)]),
                section(look, "Colour", vec![p.knob(3), p.knob(4), p.knob(7)]),
                section(look, "Blend", vec![p.hero(6)]),
            ]]
        })
    }
}

struct Decay<'a> {
    editor: &'a ReverbEditor,
}

impl canvas::Program<Change> for Decay<'_> {
    type State = ();

    fn draw(&self, _state: &(), renderer: &Renderer, _theme: &Theme, bounds: Rectangle, cursor: mouse::Cursor) -> Vec<Geometry> {
        let look = self.editor.look;
        let size = bounds.size();
        if !crate::kit::roomy(size) {
            return Vec::new();
        }
        let mut frame = graph_frame(renderer, size, &look);
        let values = &self.editor.knobs.values;
        let left = INSET;
        let right = size.width - INSET - 34.0;
        let width = (right - left).max(1.0);
        let x_of = |hz: f32| left + width * (hz / LOWEST_HZ).ln() / (HIGHEST_HZ / LOWEST_HZ).ln();
        let hz_at = |x: f32| LOWEST_HZ * (HIGHEST_HZ / LOWEST_HZ).powf(((x - left) / width).clamp(0.0, 1.0));
        let columns = (width as usize).max(2);
        let seconds: Vec<f32> = (0..=columns).map(|x| decay_seconds(values, hz_at(left + x as f32))).collect();
        let longest = seconds.iter().copied().fold(0.5f32, f32::max);
        let top = nice_ceiling(longest * 1.2);
        let floor = size.height - INSET - 22.0;
        let ceiling = INSET + 40.0;
        let y_of = |s: f32| floor - (s / top).clamp(0.0, 1.0) * (floor - ceiling);

        for hz in LABELLED_HZ {
            let x = x_of(hz).round();
            frame.fill_rectangle(Point::new(x, ceiling - 10.0), Size::new(1.0, floor - ceiling + 10.0), look.grid);
            axis_text(&mut frame, &hertz(hz).replace(".00", ""), Point::new(x, size.height - INSET - 4.0), &look, (0.5, 1.0));
        }
        let mut mark = 0.0;
        let step = top / 4.0;
        while mark <= top + 0.001 {
            let y = y_of(mark);
            guide(&mut frame, Point::new(left, y), width, if mark == 0.0 { look.grid_strong } else { look.grid });
            axis_text(&mut frame, &format!("{mark:.1} s"), Point::new(size.width - INSET - 2.0, y), &look, (1.0, 0.5));
            mark += step;
        }
        let points: Vec<Point> = seconds.iter().enumerate().map(|(x, s)| Point::new(left + x as f32, y_of(*s))).collect();
        fill_under(&mut frame, &points, floor, fade(look.accent, 0.28), fade(look.accent, 0.02));
        let line = polyline(&points);
        frame.stroke(&line, Stroke::default().with_color(fade(look.accent, 0.16)).with_width(7.0));
        frame.stroke(&line, Stroke::default().with_color(look.accent).with_width(2.0));
        put_text_in(&mut frame, "DECAY ACROSS THE SPECTRUM", Point::new(INSET + 8.0, INSET + 10.0), 9.5, look.text_dim, (0.0, 0.0), SEMIBOLD);
        if let Some(p) = cursor.position_in(bounds).filter(|p| p.x >= left && p.x <= right) {
            let hz = hz_at(p.x);
            let decay = decay_seconds(values, hz);
            frame.fill_rectangle(Point::new(p.x.round(), ceiling - 10.0), Size::new(1.0, floor - ceiling + 10.0), look.grid_strong);
            let dot = Point::new(p.x, y_of(decay));
            frame.fill(&Path::circle(dot, 6.0), fade(look.accent, 0.2));
            frame.fill(&Path::circle(dot, 3.5), look.text);
            let label = format!("{}   {:.2} s", hertz(hz), decay);
            let pill_w = label.chars().count() as f32 * 6.6 + 20.0;
            let x = (p.x - pill_w / 2.0).clamp(INSET + 4.0, size.width - INSET - pill_w - 4.0);
            let pill = Path::rounded_rectangle(Point::new(x, INSET + 6.0), Size::new(pill_w, 22.0), 6.0.into());
            frame.fill(&pill, fade(look.raised, 0.96));
            frame.stroke(&pill, Stroke::default().with_color(look.grid_strong).with_width(1.0));
            put_text_in(&mut frame, &label, Point::new(x + pill_w / 2.0, INSET + 17.0), 11.0, look.text, (0.5, 0.5), MEDIUM);
        }
        seal(&mut frame, size, &look);
        vec![frame.into_geometry()]
    }
}

fn nice_ceiling(value: f32) -> f32 {
    for nice in [0.5, 1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 10.0, 15.0, 20.0, 30.0, 40.0, 60.0, 80.0] {
        if value <= nice {
            return nice;
        }
    }
    value.ceil()
}
