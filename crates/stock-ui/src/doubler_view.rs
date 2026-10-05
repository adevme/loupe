use iced::widget::canvas::{self, Geometry};
use iced::widget::{column, container, row, Space};
use iced::{mouse, Alignment, Element, Length, Point, Rectangle, Renderer, Size, Theme};
use loupe_stock::{Doubler, MOST_DOUBLER_VOICES};

use crate::kit::{choice, dial, graph_frame, knob_row, panel, put_text};
use crate::knobs::{change, Change, Knobs};
use crate::look::Look;

const WIDEST_MS: f32 = 90.0;
const SIDE_LABELS: [&str; 3] = ["Left", "Middle", "Right"];
const VOICE_SIDES: [f32; MOST_DOUBLER_VOICES] = [-1.0, 1.0, -0.6, 0.6];
const DOT: f32 = 7.0;
const TOP_GAP: f32 = 26.0;

pub struct DoublerEditor {
    knobs: Knobs,
    look: Look,
}

impl DoublerEditor {
    pub fn new(look: Look) -> Self {
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
        let knob = |index: usize, colour| dial(look, params, values, index, colour, change);
        let timing = knob_row(vec![knob(1, look.accent), knob(2, look.accent), knob(3, look.danger), knob(4, look.danger)]);
        let placing = knob_row(vec![knob(5, look.bands[5]), knob(6, look.bands[3]), knob(7, look.bands[1])]);
        let controls = row![timing, Space::with_width(18), placing, Space::with_width(Length::Fill), choice(params[0], values[0], |value| Change(0, value))]
            .align_y(Alignment::Center);
        let graph = canvas::Canvas::new(Spread { editor: self }).width(Length::Fill).height(Length::Fill);
        let backdrop = container(container(graph).padding([12, 12])).width(Length::Fill).height(Length::Fill).style(move |_| {
            container::Style { background: Some(look.background.into()), ..Default::default() }
        });
        column![backdrop, panel(look, controls)].into()
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

        let top = TOP_GAP;
        let tall = (size.height - top - 16.0).max(1.0);
        for (at, label) in SIDE_LABELS.iter().enumerate() {
            let x = size.width * at as f32 / 2.0;
            let x = x.clamp(18.0, size.width - 18.0);
            frame.fill_rectangle(Point::new(x - 0.5, top), Size::new(1.0, tall), look.grid);
            put_text(&mut frame, label, Point::new(x, 6.0), 10.5, look.text_dim, (0.5, 0.0));
        }
        for step in 0..=3 {
            let ms = WIDEST_MS * step as f32 / 3.0;
            let y = top + tall * ms / WIDEST_MS;
            frame.fill_rectangle(Point::new(0.0, y - 0.5), Size::new(size.width, 1.0), look.grid);
            put_text(&mut frame, &format!("{ms:.0} ms"), Point::new(size.width - 4.0, y + 2.0), 10.0, look.text_dim, (1.0, 0.0));
        }
        let singer = Point::new(size.width / 2.0, top);
        frame.fill(&canvas::Path::circle(singer, DOT * 1.3), look.text);
        put_text(&mut frame, "you", Point::new(size.width / 2.0, top + 10.0), 10.5, look.text_dim, (0.5, 0.0));
        for which in 0..voices {
            let lean = match voices {
                1 => 0.0,
                more => which as f32 / (more - 1) as f32 * 2.0 - 1.0,
            };
            let late = middle * (1.0 + lean * spread * 0.6);
            let side = (VOICE_SIDES[which] * width).clamp(-1.0, 1.0);
            let x = (size.width / 2.0 * (1.0 + side)).clamp(DOT + 2.0, size.width - DOT - 2.0);
            let y = top + tall * (late / WIDEST_MS).clamp(0.0, 1.0);
            frame.fill(&canvas::Path::circle(Point::new(x, y), DOT), look.accent);
            let said = Point::new(x.min(size.width - 40.0), y + DOT + 2.0);
            put_text(&mut frame, &format!("{late:.0} ms"), said, 10.0, look.text_dim, (0.5, 0.0));
        }
        vec![frame.into_geometry()]
    }
}
