use std::sync::Arc;

use iced::widget::canvas::{self, Geometry, Path, Stroke};
use iced::{mouse, Element, Length, Point, Rectangle, Renderer, Size, Theme, Vector};
use loupe_stock::{detected, key_name, tune_note_name, Detected, History, Moment, Tune, Tuning, FOLLOWED_KEY, FOLLOWED_SCALE, SILENT_NOTE, TUNE_KEY_CHOICES, TUNE_SCALES, TUNE_STEPS};

use crate::compressor_view::{GAP, INSET};
use crate::kit::*;
use crate::knobs::{change, Change, Knobs};
use crate::look::{fade, Look};

const SHOWN_MOMENTS: usize = 800;
const RECENT_MOMENTS: usize = 20;
const NOTES_SHOWN: f32 = 14.0;
const READOUT_WIDTH: f32 = 150.0;
const LABEL_WIDTH: f32 = 34.0;
const STATUS_HEIGHT: f32 = 18.0;

pub struct TuneEditor {
    knobs: Knobs,
    history: Option<Arc<History>>,
    tuning: Option<Arc<Tuning>>,
    detected: Arc<Detected>,
    moments: Vec<Moment>,
    middle: f32,
    following: Option<u8>,
    waiting: Option<u8>,
    heard: Option<u8>,
    sure: f32,
    look: Look,
}

impl TuneEditor {
    pub fn new(history: Option<Arc<History>>, tuning: Option<Arc<Tuning>>, look: Look) -> Self {
        let look = look.tinted(crate::look::accents::TUNE);
        Self {
            knobs: Knobs::of(&Tune::new()),
            history,
            tuning,
            detected: detected(),
            moments: vec![Moment::default(); SHOWN_MOMENTS],
            middle: 60.0,
            following: None,
            waiting: None,
            heard: None,
            sure: 0.0,
            look,
        }
    }

    fn follows_key(&self) -> bool {
        self.knobs.values[0] == FOLLOWED_KEY
    }

    fn follows_scale(&self) -> bool {
        self.knobs.values[1] == FOLLOWED_SCALE
    }

    fn key_in_use(&self) -> i32 {
        if self.follows_key() {
            return self.following.map_or(0, |key| i32::from(key % 12));
        }
        self.knobs.values[0] as i32
    }

    fn scale_in_use(&self) -> usize {
        if self.follows_scale() {
            return match self.following {
                Some(key) if key < 12 => 1,
                Some(_) => 2,
                None => 0,
            };
        }
        (self.knobs.values[1] as usize).min(TUNE_STEPS.len() - 1)
    }

    fn steps_in_use(&self) -> &'static [u8] {
        TUNE_STEPS[self.scale_in_use()]
    }

    fn name_in_use(&self) -> String {
        format!("{} {}", TUNE_KEY_CHOICES[self.key_in_use().rem_euclid(12) as usize], TUNE_SCALES[self.scale_in_use()].to_lowercase())
    }

    fn followed(&self) -> bool {
        self.following.is_some() && (self.follows_key() || self.follows_scale())
    }

    fn in_use(&self) -> (String, &'static str) {
        let name = self.name_in_use();
        if !self.follows_key() && !self.follows_scale() {
            return (name, "");
        }
        match (self.following, self.waiting) {
            (Some(_), Some(_)) => (name, "moving"),
            (Some(_), None) => (name, "followed"),
            (None, Some(_)) => (name, "settling"),
            (None, None) => (name, "waiting"),
        }
    }

    fn status(&self) -> String {
        if !self.follows_key() && !self.follows_scale() {
            return "Pick Detected to follow Loupe Key.".to_string();
        }
        match (self.following, self.waiting) {
            (Some(key), Some(next)) => format!("On {}, moving to {} between phrases", key_name(key), key_name(next)),
            (Some(key), None) => format!("Following Loupe Key · {}", key_name(key)),
            (None, Some(next)) => format!("Loupe Key heard {}, settling", key_name(next)),
            (None, None) => match self.heard {
                Some(key) => format!("Loupe Key is not sure yet · {} at {:.0}%", key_name(key), self.sure * 100.0),
                None => "No Loupe Key has found a key yet".to_string(),
            },
        }
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
        if let Some(tuning) = &self.tuning {
            self.following = tuning.following();
            self.waiting = tuning.waiting();
        }
        self.heard = self.detected.key();
        self.sure = self.detected.sure();
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
        let lit = self.followed();
        let status = self.status();
        shell(look, "Tune", "Pitch correction", move || canvas::Canvas::new(Lines { editor: self }).width(Length::Fill).height(Length::Fill).into(), move |compact| {
            let p = Panel::new(look, compact, params, values, change);
            let told = Part::new(
                iced::widget::text(status.clone()).size(11.0).font(crate::look::MEDIUM).color(if lit { look.accent } else { look.text_faint }).wrapping(iced::widget::text::Wrapping::None),
                CHOICE_WIDTH,
            )
            .tall(STATUS_HEIGHT);
            vec![vec![
                section(look, "Scale", vec![stack(vec![choice(look, params[0], values[0], |value| Change(0, value)), choice(look, params[1], values[1], |value| Change(1, value)), told])]),
                section(look, "Correction", vec![p.hero(2), p.hero(3)]),
                section(look, "Voice", vec![p.knob(4)]),
            ]]
        })
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
        frame.with_save(|frame| {
            frame.translate(Vector::new(INSET, INSET));
            let rest = Size::new((size.width - READOUT_WIDTH - GAP - INSET * 2.0).max(1.0), (size.height - INSET * 2.0).max(1.0));
            draw_notes(frame, rest, editor);
        });
        frame.with_save(|frame| {
            frame.translate(Vector::new(size.width - READOUT_WIDTH - INSET, INSET));
            let recent = &editor.moments[editor.moments.len().saturating_sub(RECENT_MOMENTS)..];
            let latest = recent.iter().rev().find(|moment| moment.input >= 0.0);
            let (note, off, moved) = match latest {
                Some(moment) => (tune_note_name(moment.output), format!("{:+.0}", (moment.input - moment.output) * 100.0), format!("{:+.0}", moment.reduction * 100.0)),
                None => ("-".to_string(), "-".to_string(), "-".to_string()),
            };
            let main = Reading::new("Note", note, "", look.accent);
            let (in_use, how) = editor.in_use();
            let keyed = if editor.followed() { look.accent } else { look.text };
            let rest = [
                Reading::new("Sung off by", off, "cents", look.text),
                Reading::new("Moved by", moved, "cents", look.text),
                Reading::new("Key", in_use, how, keyed),
            ];
            draw_readouts(frame, Size::new(READOUT_WIDTH, size.height - INSET * 2.0), &look, &main, None, &rest);
        });
        seal(&mut frame, size, &look);
        vec![frame.into_geometry()]
    }
}

fn draw_notes(frame: &mut canvas::Frame, size: Size, editor: &TuneEditor) {
    let look = editor.look;
    let low = editor.middle - NOTES_SHOWN / 2.0;
    let per_note = size.height / NOTES_SHOWN;
    let y_of = |note: f32| size.height - (note - low) * per_note;
    let key = editor.key_in_use();
    let scale = editor.steps_in_use();
    for note in low.floor() as i32..=(low + NOTES_SHOWN).ceil() as i32 {
        let y = y_of(note as f32);
        if !(0.0..=size.height).contains(&y) {
            continue;
        }
        let in_key = scale.contains(&((note - key).rem_euclid(12) as u8));
        let colour = if in_key { look.grid_strong } else { look.grid };
        frame.fill_rectangle(Point::new(LABEL_WIDTH, y.round()), Size::new(size.width - LABEL_WIDTH, 1.0), colour);
        if in_key {
            axis_text(frame, &tune_note_name(note as f32), Point::new(4.0, y), &look, (0.0, 0.5));
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
    let target = trace(|moment| moment.output);
    frame.stroke(&target, Stroke::default().with_color(fade(look.accent, 0.16)).with_width(9.0));
    frame.stroke(&target, Stroke::default().with_color(fade(look.accent, 0.45)).with_width(4.0));
    frame.stroke(&trace(|moment| if moment.input < 0.0 { SILENT_NOTE } else { moment.input + moment.reduction }), Stroke::default().with_color(look.accent).with_width(1.8));
    frame.stroke(&trace(|moment| moment.input), Stroke::default().with_color(fade(look.text_dim, 0.7)).with_width(1.0));
}
