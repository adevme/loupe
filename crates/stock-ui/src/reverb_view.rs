use iced::widget::canvas::{self, Geometry, Path, Stroke};
use iced::widget::{column, container, row, Space};
use iced::{mouse, Alignment, Element, Length, Point, Rectangle, Renderer, Size, Theme};
use loupe_stock::{decay_seconds, Reverb};

use crate::kit::{dial, graph_frame, hertz, knob_row, panel, put_text};
use crate::knobs::{change, Change, Knobs};
use crate::look::{fade, Look};

const LOWEST_HZ: f32 = 20.0;
const HIGHEST_HZ: f32 = 20_000.0;
const LABELLED_HZ: [f32; 7] = [50.0, 100.0, 200.0, 500.0, 1000.0, 5000.0, 10_000.0];

pub struct ReverbEditor {
    knobs: Knobs,
    look: Look,
}

impl ReverbEditor {
    pub fn new(look: Look) -> Self {
        Self { knobs: Knobs::of(&Reverb::new()), look }
    }

    pub fn update(&mut self, change: Change) -> Vec<(usize, f32)> {
        self.knobs.apply(change)
    }

    pub fn view(&self) -> Element<'_, Change> {
        let look = self.look;
        let values = &self.knobs.values;
        let params = self.knobs.params;
        let knob = |index: usize, colour| dial(look, params, values, index, colour, change);
        let space = knob_row(vec![knob(0, look.bands[5]), knob(1, look.bands[5]), knob(2, look.accent), knob(8, look.accent)]);
        let colour = knob_row(vec![knob(3, look.bands[4]), knob(4, look.bands[4]), knob(7, look.bands[6])]);
        let output = knob_row(vec![knob(5, look.bands[3]), knob(6, look.bands[3])]);
        let controls = row![space, Space::with_width(18), colour, Space::with_width(18), output, Space::with_width(Length::Fill)].align_y(Alignment::Center);
        let graph = canvas::Canvas::new(Decay { editor: self }).width(Length::Fill).height(Length::Fill);
        let backdrop = container(container(graph).padding([12, 12])).width(Length::Fill).height(Length::Fill).style(move |_| container::Style {
            background: Some(look.background.into()),
            ..Default::default()
        });
        column![backdrop, panel(look, controls)].into()
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
        let x_of = |hz: f32| size.width * (hz / LOWEST_HZ).ln() / (HIGHEST_HZ / LOWEST_HZ).ln();
        let hz_at = |x: f32| LOWEST_HZ * (HIGHEST_HZ / LOWEST_HZ).powf((x / size.width).clamp(0.0, 1.0));
        let columns = (size.width as usize).max(2);
        let seconds: Vec<f32> = (0..=columns).map(|x| decay_seconds(values, hz_at(x as f32))).collect();
        let longest = seconds.iter().copied().fold(0.5f32, f32::max);
        let top = nice_ceiling(longest * 1.2);
        let y_of = |s: f32| size.height - 24.0 - (s / top).clamp(0.0, 1.0) * (size.height - 40.0);

        for hz in LABELLED_HZ {
            let x = x_of(hz).round();
            frame.fill_rectangle(Point::new(x, 0.0), Size::new(1.0, size.height), look.grid);
            put_text(&mut frame, &hertz(hz).replace(".00", ""), Point::new(x + 4.0, size.height - 6.0), 11.0, look.text_dim, (0.0, 1.0));
        }
        let mut mark = 0.0;
        let step = top / 4.0;
        while mark <= top + 0.001 {
            let y = y_of(mark).round();
            frame.fill_rectangle(Point::new(0.0, y), Size::new(size.width, 1.0), look.grid);
            put_text(&mut frame, &format!("{mark:.1} s"), Point::new(size.width - 6.0, y - 2.0), 10.5, look.text_dim, (1.0, 1.0));
            mark += step;
        }
        let area = Path::new(|b| {
            b.move_to(Point::new(0.0, y_of(0.0)));
            for (x, s) in seconds.iter().enumerate() {
                b.line_to(Point::new(x as f32, y_of(*s)));
            }
            b.line_to(Point::new(columns as f32, y_of(0.0)));
            b.close();
        });
        frame.fill(&area, fade(look.accent, 0.16));
        let line = Path::new(|b| {
            for (x, s) in seconds.iter().enumerate() {
                let point = Point::new(x as f32, y_of(*s));
                if x == 0 {
                    b.move_to(point);
                } else {
                    b.line_to(point);
                }
            }
        });
        frame.stroke(&line, Stroke::default().with_color(look.accent).with_width(2.0));
        put_text(&mut frame, "Decay time across the spectrum", Point::new(12.0, 10.0), 12.0, look.text_dim, (0.0, 0.0));
        if let Some(p) = cursor.position_in(bounds) {
            let hz = hz_at(p.x);
            let label = format!("{}  {:.2} s", hertz(hz), decay_seconds(values, hz));
            put_text(&mut frame, &label, Point::new(p.x.clamp(60.0, size.width - 60.0), 30.0), 12.0, look.text, (0.5, 0.0));
            frame.fill_rectangle(Point::new(p.x, 48.0), Size::new(1.0, size.height - 72.0), look.grid_strong);
        }
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
