use std::sync::Arc;

use iced::widget::canvas::{self, Geometry, Path, Stroke};
use iced::widget::{column, container, row, Space};
use iced::{mouse, Alignment, Element, Length, Point, Rectangle, Renderer, Size, Theme, Vector};
use loupe_stock::{tune_note_name, History, Moment, Tune, SILENT_NOTE, TUNE_STEPS};

use crate::kit::{choice, dial, graph_frame, knob_row, panel, put_text};
use crate::knobs::{change, Change, Knobs};
use crate::look::{fade, Look};

const SHOWN_MOMENTS: usize = 800;
const RECENT_MOMENTS: usize = 20;
const NOTES_SHOWN: f32 = 14.0;
const READOUT_WIDTH: f32 = 150.0;
const GAP: f32 = 14.0;
const LABEL_WIDTH: f32 = 34.0;

pub struct TuneEditor {
    knobs: Knobs,
    history: Option<Arc<History>>,
    moments: Vec<Moment>,
    middle: f32,
    look: Look,
}

impl TuneEditor {
    pub fn new(history: Option<Arc<History>>, look: Look) -> Self {
        Self { knobs: Knobs::of(&Tune::new()), history, moments: vec![Moment::default(); SHOWN_MOMENTS], middle: 60.0, look }
    }

    pub fn load(&mut self, values: &[f32]) {
        for (slot, value) in values.iter().enumerate().take(self.knobs.values.len()) {
            self.knobs.values[slot] = *value;
        }
    }

    pub fn update(&mut self, change: Change) -> Vec<(usize, f32)> {
        self.knobs.apply(change)
    }

    pub fn tick(&mut self) {
        if let Some(history) = &self.history {
            history.latest(&mut self.moments);
        }
        let sung: Vec<f32> = self.moments.iter().map(|moment| moment.input).filter(|note| *note >= 0.0).collect();
        if let (Some(low), Some(high)) = (sung.iter().copied().reduce(f32::min), sung.iter().copied().reduce(f32::max)) {
            let wanted = (low + high) / 2.0;
            self.middle += (wanted - self.middle) * 0.1;
        }
    }

    pub fn view(&self) -> Element<'_, Change> {
        let look = self.look;
        let values = &self.knobs.values;
        let params = self.knobs.params;
        let key = column![choice(params[0], values[0], |value| Change(0, value)), choice(params[1], values[1], |value| Change(1, value))].spacing(10).width(170);
        let knobs = knob_row(vec![
            dial(look, params, values, 2, look.accent, change),
            dial(look, params, values, 3, look.bands[3], change),
            dial(look, params, values, 4, look.bands[6], change),
        ]);
        let controls = row![key, Space::with_width(18), knobs, Space::with_width(Length::Fill)].align_y(Alignment::Center);
        let graph = canvas::Canvas::new(Lines { editor: self }).width(Length::Fill).height(Length::Fill);
        let backdrop = container(container(graph).padding([12, 12])).width(Length::Fill).height(Length::Fill).style(move |_| container::Style {
            background: Some(look.background.into()),
            ..Default::default()
        });
        column![backdrop, panel(look, controls)].into()
    }
}

struct Lines<'a> {
    editor: &'a TuneEditor,
}

impl canvas::Program<Change> for Lines<'_> {
    type State = ();

    fn draw(&self, _state: &(), renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let editor = self.editor;
        let look = editor.look;
        let size = bounds.size();
        if !crate::kit::roomy(size) {
            return Vec::new();
        }
        let mut frame = graph_frame(renderer, size, &look);
        let rest = Size::new((size.width - READOUT_WIDTH - GAP).max(1.0), size.height);
        draw_notes(&mut frame, rest, editor);
        frame.with_save(|frame| {
            frame.translate(Vector::new(rest.width + GAP, 0.0));
            draw_readout(frame, Size::new(READOUT_WIDTH, size.height), editor);
        });
        vec![frame.into_geometry()]
    }
}

fn draw_notes(frame: &mut canvas::Frame, size: Size, editor: &TuneEditor) {
    let look = editor.look;
    let low = editor.middle - NOTES_SHOWN / 2.0;
    let per_note = size.height / NOTES_SHOWN;
    let y_of = |note: f32| size.height - (note - low) * per_note;
    let key = editor.knobs.values[0] as i32;
    let scale = TUNE_STEPS[(editor.knobs.values[1] as usize).min(TUNE_STEPS.len() - 1)];
    for note in low.floor() as i32..=(low + NOTES_SHOWN).ceil() as i32 {
        let y = y_of(note as f32);
        if !(0.0..=size.height).contains(&y) {
            continue;
        }
        let in_key = scale.contains(&((note - key).rem_euclid(12) as u8));
        let colour = if in_key { look.grid_strong } else { fade(look.grid, 0.5) };
        frame.fill_rectangle(Point::new(LABEL_WIDTH, y), Size::new(size.width - LABEL_WIDTH, 1.0), colour);
        if in_key {
            put_text(frame, &tune_note_name(note as f32), Point::new(4.0, y), 10.5, look.text_dim, (0.0, 0.5));
        }
    }
    let count = editor.moments.len();
    let width = size.width - LABEL_WIDTH;
    let x_of = |at: usize| LABEL_WIDTH + width * at as f32 / (count - 1).max(1) as f32;
    let trace = |pick: fn(&Moment) -> f32| {
        Path::new(|b| {
            let mut drawing = false;
            for (at, moment) in editor.moments.iter().enumerate() {
                let note = pick(moment);
                if note == SILENT_NOTE || note < 0.0 {
                    drawing = false;
                    continue;
                }
                let point = Point::new(x_of(at), y_of(note).clamp(0.0, size.height));
                if drawing {
                    b.line_to(point);
                } else {
                    b.move_to(point);
                    drawing = true;
                }
            }
        })
    };
    frame.stroke(&trace(|moment| moment.output), Stroke::default().with_color(fade(look.accent, 0.55)).with_width(5.0));
    frame.stroke(&trace(|moment| if moment.input < 0.0 { SILENT_NOTE } else { moment.input + moment.reduction }), Stroke::default().with_color(look.accent).with_width(1.6));
    frame.stroke(&trace(|moment| moment.input), Stroke::default().with_color(fade(look.text_dim, 0.7)).with_width(1.0));
}

fn draw_readout(frame: &mut canvas::Frame, size: Size, editor: &TuneEditor) {
    let look = editor.look;
    frame.fill_rectangle(Point::ORIGIN, size, look.panel);
    let recent = &editor.moments[editor.moments.len().saturating_sub(RECENT_MOMENTS)..];
    let latest = recent.iter().rev().find(|moment| moment.input >= 0.0);
    let middle = size.width / 2.0;
    put_text(frame, "Note", Point::new(middle, 18.0), 11.5, look.text_dim, (0.5, 0.0));
    let (note, off, moved) = match latest {
        Some(moment) => (tune_note_name(moment.output), format!("{:+.0}", (moment.input - moment.output) * 100.0), format!("{:+.0}", moment.reduction * 100.0)),
        None => ("-".to_string(), "-".to_string(), "-".to_string()),
    };
    put_text(frame, &note, Point::new(middle, 36.0), 30.0, look.accent, (0.5, 0.0));
    put_text(frame, "Sung off by", Point::new(middle, 90.0), 11.5, look.text_dim, (0.5, 0.0));
    put_text(frame, &off, Point::new(middle, 108.0), 22.0, look.text, (0.5, 0.0));
    put_text(frame, "cents", Point::new(middle, 134.0), 11.5, look.text_dim, (0.5, 0.0));
    put_text(frame, "Moved by", Point::new(middle, 162.0), 11.5, look.text_dim, (0.5, 0.0));
    put_text(frame, &moved, Point::new(middle, 180.0), 22.0, look.text, (0.5, 0.0));
    put_text(frame, "cents", Point::new(middle, 206.0), 11.5, look.text_dim, (0.5, 0.0));
}
