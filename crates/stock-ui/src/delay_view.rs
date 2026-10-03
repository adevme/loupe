use iced::widget::canvas::{self, Geometry, Path};
use iced::widget::{column, container, row, Space};
use iced::{mouse, Alignment, Element, Length, Point, Rectangle, Renderer, Size, Theme};
use loupe_stock::{echo_seconds, Delay};

use crate::kit::{choice, dial, graph_frame, knob_row, panel, put_text, toggle};
use crate::knobs::{change, Change, Knobs};
use crate::look::{fade, Look};

const ECHOES: usize = 16;
const QUIETEST_SHOWN: f32 = 0.01;
const SYNC: usize = 0;
const NOTE_LEFT: usize = 1;
const NOTE_RIGHT: usize = 2;
const TIME_LEFT: usize = 3;
const TIME_RIGHT: usize = 4;
const LINK: usize = 5;
const FEEDBACK: usize = 6;
const CROSS: usize = 7;

pub struct DelayEditor {
    knobs: Knobs,
    bpm: f32,
    look: Look,
}

impl DelayEditor {
    pub fn new(bpm: f32, look: Look) -> Self {
        Self { knobs: Knobs::of(&Delay::new()), bpm, look }
    }

    pub fn set_tempo(&mut self, bpm: f32) {
        self.bpm = bpm;
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
        let synced = self.knobs.on(SYNC);
        let linked = self.knobs.on(LINK);
        let flip = |index: usize| Change(index, if self.knobs.on(index) { 0.0 } else { 1.0 });
        let mut time: Vec<Element<'_, Change>> = Vec::new();
        if synced {
            time.push(choice(params[NOTE_LEFT], values[NOTE_LEFT], |value| Change(NOTE_LEFT, value)));
            if !linked {
                time.push(choice(params[NOTE_RIGHT], values[NOTE_RIGHT], |value| Change(NOTE_RIGHT, value)));
            }
        } else {
            time.push(knob(TIME_LEFT, look.accent));
            if !linked {
                time.push(knob(TIME_RIGHT, look.accent));
            }
        }
        let switches = column![toggle(look, "Sync", synced, flip(SYNC)), toggle(look, "Link L and R", linked, flip(LINK))].spacing(8);
        let timing = row![switches, row(time).spacing(10).align_y(Alignment::Center)].spacing(14).align_y(Alignment::Center);
        let repeats = knob_row(vec![knob(FEEDBACK, look.accent), knob(CROSS, look.accent), knob(10, look.bands[0])]);
        let tone = knob_row(vec![knob(8, look.bands[4]), knob(9, look.bands[4]), knob(11, look.bands[6]), knob(12, look.bands[6])]);
        let output = knob_row(vec![knob(13, look.bands[3]), knob(14, look.bands[3]), knob(15, look.bands[3])]);
        let controls = column![
            row![timing, Space::with_width(Length::Fill), repeats].align_y(Alignment::Center),
            row![tone, Space::with_width(Length::Fill), output].align_y(Alignment::Center),
        ]
        .spacing(8);
        let graph = canvas::Canvas::new(Echoes { editor: self }).width(Length::Fill).height(Length::Fill);
        let backdrop = container(container(graph).padding([12, 12])).width(Length::Fill).height(Length::Fill).style(move |_| container::Style {
            background: Some(look.background.into()),
            ..Default::default()
        });
        column![backdrop, panel(look, controls)].into()
    }
}

struct Echoes<'a> {
    editor: &'a DelayEditor,
}

impl canvas::Program<Change> for Echoes<'_> {
    type State = ();

    fn draw(&self, _state: &(), renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let editor = self.editor;
        let look = editor.look;
        let size = bounds.size();
        if !crate::kit::roomy(size) {
            return Vec::new();
        }
        let mut frame = graph_frame(renderer, size, &look);
        let values = &editor.knobs.values;
        let (left, right) = echo_seconds(values, editor.bpm);
        let feedback = values[FEEDBACK] / 100.0;
        let cross = values[CROSS] / 100.0;
        let span = (left.max(right) * 8.0).clamp(1.0, 12.0);
        let middle = size.height / 2.0;
        let x_of = |seconds: f32| 16.0 + seconds / span * (size.width - 32.0);
        let beat = 60.0 / editor.bpm;
        let mut at = 0.0;
        let mut count = 0;
        while at <= span {
            let strong = count % 4 == 0;
            frame.fill_rectangle(Point::new(x_of(at).round(), 0.0), Size::new(1.0, size.height), if strong { look.grid_strong } else { look.grid });
            if strong {
                put_text(&mut frame, &format!("{}", count / 4 + 1), Point::new(x_of(at) + 4.0, size.height - 6.0), 11.0, look.text_dim, (0.0, 1.0));
            }
            at += beat;
            count += 1;
        }
        frame.fill_rectangle(Point::new(0.0, middle), Size::new(size.width, 1.0), look.grid_strong);
        put_text(&mut frame, "L", Point::new(6.0, 8.0), 12.0, look.text_dim, (0.0, 0.0));
        put_text(&mut frame, "R", Point::new(6.0, size.height - 24.0), 12.0, look.text_dim, (0.0, 1.0));
        let reach = middle - 26.0;
        let bar = |frame: &mut canvas::Frame, seconds: f32, level: f32, up: bool, colour| {
            if level < QUIETEST_SHOWN || seconds > span {
                return;
            }
            let height = level.min(1.0) * reach;
            let x = x_of(seconds);
            let top = if up { middle - height } else { middle };
            let body = Path::rounded_rectangle(Point::new(x - 3.0, top), Size::new(6.0, height), 3.0.into());
            frame.fill(&body, colour);
        };
        frame.fill(&Path::rounded_rectangle(Point::new(x_of(0.0) - 3.0, middle - reach), Size::new(6.0, reach * 2.0), 3.0.into()), fade(look.text, 0.5));
        let mut levels = [1.0f32, 1.0 - cross];
        for echo in 1..=ECHOES {
            let n = echo as f32;
            bar(&mut frame, left * n, levels[0], true, fade(look.accent, 0.9));
            bar(&mut frame, right * n, levels[1], false, fade(look.bands[5], 0.9));
            levels = [
                (levels[0] * (1.0 - cross) + levels[1] * cross) * feedback,
                (levels[1] * (1.0 - cross) + levels[0] * cross) * feedback,
            ];
        }
        let label = format!("{:.0} ms   {:.0} ms   {:.0} BPM", left * 1000.0, right * 1000.0, editor.bpm);
        put_text(&mut frame, &label, Point::new(size.width - 10.0, 8.0), 12.0, look.text_dim, (1.0, 0.0));
        vec![frame.into_geometry()]
    }
}
