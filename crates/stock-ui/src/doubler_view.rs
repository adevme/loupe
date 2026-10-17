use iced::widget::canvas::{self, Geometry, Path, Stroke};
use iced::{mouse, Element, Length, Point, Rectangle, Renderer, Size, Theme};
use loupe_stock::{Doubler, MOST_DOUBLER_VOICES};

use crate::compressor_view::INSET;
use crate::kit::*;
use crate::knobs::{change, Change, Knobs};
use crate::look::{fade, Look, MEDIUM};

const WIDEST_MS: f32 = 90.0;
const SIDE_LABELS: [&str; 3] = ["LEFT", "MIDDLE", "RIGHT"];
const VOICE_SIDES: [f32; MOST_DOUBLER_VOICES] = [-1.0, 1.0, -0.6, 0.6];
const DOT: f32 = 7.0;
const TOP_GAP: f32 = 30.0;

pub struct DoublerEditor {
    knobs: Knobs,
    look: Look,
}

impl DoublerEditor {
    pub fn new(look: Look) -> Self {
        let look = look.tinted(crate::look::accents::DOUBLER);
        Self { knobs: Knobs::of(&Doubler::new()), look }
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
        let voices = (values[0].round() as usize).clamp(1, MOST_DOUBLER_VOICES);
        shell(look, "Doubler", "Vocal doubler", move || canvas::Canvas::new(Spread { editor: self }).width(Length::Fill).height(Length::Fill).into(), move |compact| {
            let p = Panel::new(look, compact, params, values, change);
            vec![vec![
                section(look, "Voices", vec![segments(look, (1..=MOST_DOUBLER_VOICES).map(|n| (n.to_string(), Change(0, n as f32), voices == n)).collect())]),
                section(look, "Timing", vec![p.hero(1), p.knob(2), p.knob(3), p.knob(4)]),
                section(look, "Placing", vec![p.hero(5), p.knob(6), p.knob(7)]),
            ]]
        })
    }
}

struct Spread<'a> {
    editor: &'a DoublerEditor,
}

impl canvas::Program<Change> for Spread<'_> {
    type State = ();

    fn draw(&self, _state: &(), renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let look = self.editor.look;
        let size = bounds.size();
        if !crate::kit::roomy(size) {
            return Vec::new();
        }
        let mut frame = graph_frame(renderer, size, &look);
        let values = &self.editor.knobs.values;
        let voices = (values[0].round() as usize).clamp(1, MOST_DOUBLER_VOICES);
        let middle = values[1];
        let spread = values[2] / 100.0;
        let width = values[5] / 100.0;

        let left = INSET + 20.0;
        let span = (size.width - INSET * 2.0 - 40.0 - 44.0).max(1.0);
        let top = INSET + TOP_GAP;
        let tall = (size.height - top - INSET - 24.0).max(1.0);
        let x_at = |side: f32| left + span / 2.0 * (1.0 + side);
        for (at, label) in SIDE_LABELS.iter().enumerate() {
            let x = x_at(at as f32 - 1.0);
            frame.fill_rectangle(Point::new(x.round(), top), Size::new(1.0, tall), if at == 1 { look.grid_strong } else { look.grid });
            put_text_in(&mut frame, label, Point::new(x, INSET + 10.0), 9.5, look.text_dim, (0.5, 0.0), MEDIUM);
        }
        for step in 0..=3 {
            let ms = WIDEST_MS * step as f32 / 3.0;
            let y = top + tall * ms / WIDEST_MS;
            guide(&mut frame, Point::new(left - 10.0, y), span + 20.0, look.grid);
            axis_text(&mut frame, &format!("{ms:.0} ms"), Point::new(size.width - INSET - 2.0, y), &look, (1.0, 0.5));
        }
        let singer = Point::new(x_at(0.0), top);
        frame.fill(&Path::circle(singer, DOT * 2.2), fade(look.text, 0.08));
        frame.fill(&Path::circle(singer, DOT * 1.3), look.text);
        put_text_in(&mut frame, "YOU", Point::new(singer.x + DOT * 2.0 + 4.0, top), 9.5, look.text_dim, (0.0, 0.5), MEDIUM);
        for which in 0..voices {
            let lean = match voices {
                1 => 0.0,
                more => which as f32 / (more - 1) as f32 * 2.0 - 1.0,
            };
            let late = middle * (1.0 + lean * spread * 0.6);
            let side = (VOICE_SIDES[which] * width).clamp(-1.0, 1.0);
            let x = x_at(side);
            let y = top + tall * (late / WIDEST_MS).clamp(0.0, 1.0);
            let at = Point::new(x, y);
            frame.stroke(&Path::line(singer, at), Stroke::default().with_color(fade(look.accent, 0.18)).with_width(1.0));
            frame.fill(&Path::circle(at, DOT * 2.0), fade(look.accent, 0.12));
            frame.fill(&Path::circle(at, DOT), look.accent);
            put_text_in(&mut frame, &format!("{late:.0} ms"), Point::new(x, y + DOT + 6.0), 10.0, look.text_dim, (0.5, 0.0), MEDIUM);
        }
        seal(&mut frame, size, &look);
        vec![frame.into_geometry()]
    }
}
