use iced::widget::canvas::{self, Geometry, Path};

use iced::{mouse, Element, Length, Point, Rectangle, Renderer, Size, Theme};
use loupe_stock::{echo_seconds, Delay};

use crate::compressor_view::INSET;
use crate::kit::*;
use crate::knobs::{change, Change, Knobs};
use crate::look::{fade, Look, MEDIUM};

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
        let look = look.tinted(crate::look::accents::DELAY);
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
        let synced = self.knobs.on(SYNC);
        let linked = self.knobs.on(LINK);
        let flip = move |index: usize| Change(index, if values[index] > 0.5 { 0.0 } else { 1.0 });
        shell(look, "Delay", "Tape and digital echo", move || canvas::Canvas::new(Echoes { editor: self }).width(Length::Fill).height(Length::Fill).into(), move |compact| {
            let p = Panel::new(look, compact, params, values, change);
            let mut time: Vec<Part<'_, Change>> = vec![stack(vec![toggle(look, "Sync", synced, flip(SYNC)), toggle(look, "Link L and R", linked, flip(LINK))])];
            if synced {
                let mut notes = vec![choice(look, params[NOTE_LEFT], values[NOTE_LEFT], |value| Change(NOTE_LEFT, value))];
                if !linked {
                    notes.push(choice(look, params[NOTE_RIGHT], values[NOTE_RIGHT], |value| Change(NOTE_RIGHT, value)));
                }
                time.push(stack(notes));
            } else {
                time.push(p.hero(TIME_LEFT));
                if !linked {
                    time.push(p.hero(TIME_RIGHT));
                }
            }
            vec![vec![
                section(look, "Time", time),
                section(look, "Repeats", vec![p.knob(FEEDBACK), p.knob(CROSS), p.knob(10)]),
                section(look, "Tone", vec![p.knob(8), p.knob(9), p.knob(11), p.knob(12)]),
                section(look, "Output", vec![p.knob(13), p.knob(14), p.knob(15)]),
            ]]
        })
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
        let x_of = |seconds: f32| 28.0 + seconds / span * (size.width - 48.0);
        let beat = 60.0 / editor.bpm;
        let mut at = 0.0;
        let mut count = 0;
        while at <= span {
            let strong = count % 4 == 0;
            let x = x_of(at).round();
            frame.fill_rectangle(Point::new(x, INSET + 26.0), Size::new(1.0, size.height - INSET * 2.0 - 44.0), if strong { look.grid_strong } else { look.grid });
            if strong {
                axis_text(&mut frame, &format!("{}", count / 4 + 1), Point::new(x + 4.0, size.height - INSET - 4.0), &look, (0.0, 1.0));
            }
            at += beat;
            count += 1;
        }
        guide(&mut frame, Point::new(INSET, middle), size.width - INSET * 2.0, look.grid_strong);
        put_text_in(&mut frame, "L", Point::new(INSET + 4.0, middle - 14.0), 10.0, look.accent, (0.0, 0.5), MEDIUM);
        put_text_in(&mut frame, "R", Point::new(INSET + 4.0, middle + 14.0), 10.0, look.neutral(), (0.0, 0.5), MEDIUM);
        let reach = middle - INSET - 34.0;
        let bar = |frame: &mut canvas::Frame, seconds: f32, level: f32, up: bool, colour| {
            if level < QUIETEST_SHOWN || seconds > span {
                return;
            }
            let height = (level.min(1.0) * reach).max(2.0);
            let x = x_of(seconds);
            let top = if up { middle - height } else { middle + 1.0 };
            let body = Path::rounded_rectangle(Point::new(x - 3.0, top), Size::new(6.0, height), 3.0.into());
            frame.fill(&Path::rounded_rectangle(Point::new(x - 6.0, top - 3.0), Size::new(12.0, height + 6.0), 6.0.into()), fade(colour, 0.12));
            frame.fill(&body, colour);
        };
        let dry = Path::rounded_rectangle(Point::new(x_of(0.0) - 3.0, middle - reach), Size::new(6.0, reach * 2.0), 3.0.into());
        frame.fill(&dry, fade(look.text, 0.55));
        let mut levels = [1.0f32, 1.0 - cross];
        for echo in 1..=ECHOES {
            let n = echo as f32;
            bar(&mut frame, left * n, levels[0], true, look.accent);
            bar(&mut frame, right * n, levels[1], false, look.neutral());
            levels = [
                (levels[0] * (1.0 - cross) + levels[1] * cross) * feedback,
                (levels[1] * (1.0 - cross) + levels[0] * cross) * feedback,
            ];
        }
        let tags = [(format!("{:.0} ms", left * 1000.0), look.accent), (format!("{:.0} ms", right * 1000.0), look.neutral()), (format!("{:.0} BPM", editor.bpm), look.text_dim)];
        let mut x = size.width - INSET - 6.0;
        for (label, colour) in tags.iter().rev() {
            put_text_in(&mut frame, label, Point::new(x, INSET + 8.0), 11.0, *colour, (1.0, 0.0), MEDIUM);
            x -= label.chars().count() as f32 * 7.0 + 18.0;
        }
        seal(&mut frame, size, &look);
        vec![frame.into_geometry()]
    }
}
