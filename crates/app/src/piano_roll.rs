use iced::widget::canvas::{self, Frame, Geometry, Path, Text};
use iced::widget::{column, container, row, text};
use iced::{alignment, keyboard, mouse, Alignment, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};
use loupe_engine::{drum_name, key_name, Clip, ClipId, Frames, Instrument, Note, Project, HIGHEST_KEY};

use crate::theme::{self, Palette};
use crate::{App, Message};

const KEYS_W: f32 = 72.0;
const RULER_H: f32 = 26.0;
const EDGE_GRIP: f32 = 6.0;
const SHORTEST_BEATS: f64 = 1.0 / 16.0;
const STEP_BEATS: f64 = 0.25;
const DEFAULT_VELOCITY: f32 = 0.8;
const MIN_BEAT_PX: f64 = 12.0;
const MAX_BEAT_PX: f64 = 400.0;
const MIN_KEY_H: f32 = 8.0;
const MAX_KEY_H: f32 = 40.0;
const BLACK_KEYS: [bool; 12] = [false, true, false, true, false, false, true, false, true, false, true, false];
const TYPING_ROWS: [(char, u8); 29] = [
    ('z', 0), ('s', 1), ('x', 2), ('d', 3), ('c', 4), ('v', 5), ('g', 6), ('b', 7), ('h', 8), ('n', 9), ('j', 10), ('m', 11),
    (',', 12), ('l', 13), ('.', 14), ('q', 12), ('2', 13), ('w', 14), ('3', 15), ('e', 16), ('r', 17), ('5', 18),
    ('t', 19), ('6', 20), ('y', 21), ('7', 22), ('u', 23), ('i', 24), ('o', 26),
];
const TYPING_BASE: u8 = 48;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RollView {
    pub beat_px: f64,
    pub key_h: f32,
    pub top_key: f32,
    pub scroll_beats: f64,
}

impl Default for RollView {
    fn default() -> Self {
        Self { beat_px: 64.0, key_h: 16.0, top_key: 84.0, scroll_beats: 0.0 }
    }
}

pub fn typed_key(character: &str) -> Option<u8> {
    let mut chars = character.chars();
    let typed = chars.next()?.to_ascii_lowercase();
    if chars.next().is_some() {
        return None;
    }
    TYPING_ROWS.iter().find(|(c, _)| *c == typed).map(|(_, offset)| TYPING_BASE + offset)
}

pub struct Roll<'a> {
    pub project: &'a Project,
    pub clip: ClipId,
    pub palette: &'a Palette,
    pub view: RollView,
    pub playhead: Frames,
    pub last_beats: f64,
}

#[derive(Default)]
pub struct Hand {
    drag: Option<Drag>,
    sounding: Option<u8>,
    modifiers: keyboard::Modifiers,
}

enum Drag {
    Move { index: usize, before: Vec<Note>, grab_beats: f64 },
    Stretch { index: usize, before: Vec<Note> },
    Erase,
    Pan { from: Point, view: RollView },
}

enum Spot {
    Key(u8),
    Note { index: usize, edge: bool },
    Empty { beats: f64, key: u8 },
    Ruler,
    Nothing,
}

impl Roll<'_> {
    fn clip(&self) -> Option<&Clip> {
        self.project.clip(self.clip)
    }

    fn notes(&self) -> Vec<Note> {
        self.clip().and_then(|clip| clip.notes.as_deref().cloned()).unwrap_or_default()
    }

    fn drums(&self) -> bool {
        self.project.track_of(self.clip).is_some_and(|track| track.instrument == Instrument::Drums)
    }

    fn frames_per_beat(&self) -> f64 {
        60.0 / self.project.bpm * self.project.rate as f64
    }

    fn offset(&self) -> Frames {
        self.clip().map_or(0, |clip| clip.offset)
    }

    fn x_of_beats(&self, beats: f64) -> f32 {
        KEYS_W + ((beats - self.view.scroll_beats) * self.view.beat_px) as f32
    }

    fn beats_at(&self, x: f32) -> f64 {
        ((x - KEYS_W) as f64 / self.view.beat_px + self.view.scroll_beats).max(0.0)
    }

    fn beats_of(&self, frames: Frames) -> f64 {
        (frames as f64 - self.offset() as f64) / self.frames_per_beat()
    }

    fn frames_of(&self, beats: f64) -> Frames {
        (self.offset() as f64 + beats.max(0.0) * self.frames_per_beat()).round() as Frames
    }

    fn y_of_key(&self, key: u8) -> f32 {
        RULER_H + (self.view.top_key - key as f32) * self.view.key_h
    }

    fn key_at(&self, y: f32) -> Option<u8> {
        let from_top = ((y - RULER_H) / self.view.key_h).floor();
        let key = self.view.top_key - from_top;
        (0.0..=HIGHEST_KEY as f32).contains(&key).then_some(key as u8)
    }

    fn snap(&self, beats: f64, free: bool) -> f64 {
        if free {
            beats
        } else {
            (beats / STEP_BEATS).floor() * STEP_BEATS
        }
    }

    fn note_box(&self, note: &Note) -> Rectangle {
        let left = self.x_of_beats(self.beats_of(note.start));
        let right = self.x_of_beats(self.beats_of(note.end()));
        Rectangle::new(Point::new(left, self.y_of_key(note.key)), Size::new((right - left).max(3.0), self.view.key_h))
    }

    fn spot(&self, p: Point, notes: &[Note]) -> Spot {
        if p.y < RULER_H {
            return Spot::Ruler;
        }
        let Some(key) = self.key_at(p.y) else {
            return Spot::Nothing;
        };
        if p.x < KEYS_W {
            return Spot::Key(key);
        }
        for (index, note) in notes.iter().enumerate().rev() {
            let shape = self.note_box(note);
            if shape.contains(p) {
                let edge = shape.x + shape.width - p.x <= EDGE_GRIP.min(shape.width / 3.0);
                return Spot::Note { index, edge };
            }
        }
        Spot::Empty { beats: self.beats_at(p.x), key }
    }

    fn label(&self, key: u8) -> String {
        if self.drums() {
            drum_name(key).map(str::to_string).unwrap_or_else(|| key_name(key))
        } else {
            key_name(key)
        }
    }
}

impl canvas::Program<Message> for Roll<'_> {
    type State = Hand;

    fn update(&self, hand: &mut Hand, event: canvas::Event, bounds: Rectangle, cursor: mouse::Cursor) -> (canvas::event::Status, Option<Message>) {
        use canvas::event::Status::{Captured, Ignored};
        let clip = self.clip;
        let free = hand.modifiers.alt();
        match event {
            canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                hand.modifiers = modifiers;
                (Ignored, None)
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(p) = cursor.position_in(bounds) else {
                    return (Ignored, None);
                };
                let notes = self.notes();
                match self.spot(p, &notes) {
                    Spot::Key(key) => {
                        hand.sounding = Some(key);
                        (Captured, Some(Message::RollSound { key, on: true }))
                    }
                    Spot::Note { index, edge: true } => {
                        hand.drag = Some(Drag::Stretch { index, before: notes });
                        (Captured, Some(Message::Refresh))
                    }
                    Spot::Note { index, edge: false } => {
                        let note = notes[index];
                        let grab_beats = self.beats_at(p.x) - self.beats_of(note.start);
                        hand.sounding = Some(note.key);
                        hand.drag = Some(Drag::Move { index, before: notes, grab_beats });
                        (Captured, Some(Message::RollSound { key: note.key, on: true }))
                    }
                    Spot::Empty { beats, key } => {
                        let start = self.frames_of(self.snap(beats, free));
                        let len = (self.last_beats * self.frames_per_beat()).round().max(1.0) as Frames;
                        let mut changed = notes.clone();
                        changed.push(Note { key, start, len, velocity: DEFAULT_VELOCITY });
                        let index = changed.len() - 1;
                        hand.sounding = Some(key);
                        hand.drag = Some(Drag::Move { index, before: changed.clone(), grab_beats: beats - self.beats_of(start) });
                        (Captured, Some(Message::RollPlaced { clip, notes: changed, key }))
                    }
                    Spot::Ruler => {
                        hand.drag = Some(Drag::Pan { from: p, view: self.view });
                        (Captured, None)
                    }
                    Spot::Nothing => (Ignored, None),
                }
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                let Some(p) = cursor.position_in(bounds) else {
                    return (Ignored, None);
                };
                hand.drag = Some(Drag::Erase);
                let notes = self.notes();
                match self.spot(p, &notes) {
                    Spot::Note { index, .. } => {
                        let mut changed = notes;
                        changed.remove(index);
                        (Captured, Some(Message::RollEdit { clip, notes: changed }))
                    }
                    _ => (Captured, None),
                }
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let (Some(drag), Some(p)) = (&hand.drag, cursor.position_from(bounds.position())) else {
                    return (Ignored, None);
                };
                match drag {
                    Drag::Move { index, before, grab_beats, .. } => {
                        let mut changed = before.clone();
                        let note = &mut changed[*index];
                        let beats = self.snap(self.beats_at(p.x) - grab_beats + if free { 0.0 } else { STEP_BEATS / 2.0 }, free);
                        note.start = self.frames_of(beats);
                        if let Some(key) = self.key_at(p.y) {
                            note.key = key;
                        }
                        let key = note.key;
                        let sound = (hand.sounding != Some(key)).then(|| {
                            let old = hand.sounding.replace(key);
                            Message::RollSlide { from: old, to: key }
                        });
                        let edit = Message::RollEdit { clip, notes: changed };
                        (Captured, Some(match sound {
                            Some(slide) => Message::Both(Box::new(edit), Box::new(slide)),
                            None => edit,
                        }))
                    }
                    Drag::Stretch { index, before } => {
                        let mut changed = before.clone();
                        let note = &mut changed[*index];
                        let start_beats = self.beats_of(note.start);
                        let end_beats = self.beats_at(p.x);
                        let end = if free { end_beats } else { (end_beats / STEP_BEATS).round() * STEP_BEATS };
                        let len_beats = (end - start_beats).max(if free { SHORTEST_BEATS } else { STEP_BEATS });
                        note.len = (len_beats * self.frames_per_beat()).round().max(1.0) as Frames;
                        (Captured, Some(Message::RollEdit { clip, notes: changed }))
                    }
                    Drag::Erase => {
                        let notes = self.notes();
                        match self.spot(p, &notes) {
                            Spot::Note { index, .. } => {
                                let mut changed = notes;
                                changed.remove(index);
                                (Captured, Some(Message::RollEdit { clip, notes: changed }))
                            }
                            _ => (Captured, None),
                        }
                    }
                    Drag::Pan { from, view } => {
                        let scroll_beats = (view.scroll_beats - (p.x - from.x) as f64 / view.beat_px).max(0.0);
                        (Captured, Some(Message::RollView(RollView { scroll_beats, ..*view })))
                    }
                }
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(_)) => {
                let finished = hand.drag.take();
                let stretched = match &finished {
                    Some(Drag::Stretch { index, .. }) | Some(Drag::Move { index, .. }) => {
                        self.notes().get(*index).map(|note| note.len as f64 / self.frames_per_beat())
                    }
                    _ => None,
                };
                let quiet = hand.sounding.take().map(|key| Message::RollSound { key, on: false });
                let done = Message::RollDone { remember_beats: stretched };
                match (finished.is_some() || quiet.is_some(), quiet) {
                    (true, Some(quiet)) => (Captured, Some(Message::Both(Box::new(done), Box::new(quiet)))),
                    (true, None) => (Captured, Some(done)),
                    _ => (Ignored, None),
                }
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                if cursor.position_in(bounds).is_none() {
                    return (Ignored, None);
                }
                let (x, y) = match delta {
                    mouse::ScrollDelta::Lines { x, y } => (x * 40.0, y * 40.0),
                    mouse::ScrollDelta::Pixels { x, y } => (x, y),
                };
                let mut view = self.view;
                if hand.modifiers.command() {
                    view.beat_px = (view.beat_px * (1.0015f64).powf(y as f64)).clamp(MIN_BEAT_PX, MAX_BEAT_PX);
                } else if hand.modifiers.alt() {
                    view.key_h = (view.key_h + y / 40.0).clamp(MIN_KEY_H, MAX_KEY_H);
                } else if hand.modifiers.shift() || x != 0.0 {
                    let sideways = if x != 0.0 { x } else { y };
                    view.scroll_beats = (view.scroll_beats - sideways as f64 / view.beat_px).max(0.0);
                } else {
                    view.top_key = (view.top_key + y / view.key_h).clamp(12.0, HIGHEST_KEY as f32);
                }
                (Captured, Some(Message::RollView(view)))
            }
            _ => (Ignored, None),
        }
    }

    fn draw(&self, hand: &Hand, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, cursor: mouse::Cursor) -> Vec<Geometry> {
        let p = self.palette;
        let size = bounds.size();
        let mut frame = Frame::new(renderer, size);
        if size.width < KEYS_W + 20.0 || size.height < RULER_H + 20.0 {
            return vec![frame.into_geometry()];
        }
        frame.fill_rectangle(Point::ORIGIN, size, p.background);
        let colour = self
            .project
            .track_of(self.clip)
            .and_then(|track| self.project.tracks.iter().position(|t| t.id == track.id).map(|i| (track, i)))
            .map_or(p.accent, |(track, i)| match track.colour {
                Some([r, g, b]) => Color::from_rgb8(r, g, b),
                None => p.track(i),
            });

        let first_key = self.key_at(size.height).unwrap_or(0);
        let last_key = self.key_at(RULER_H).unwrap_or(HIGHEST_KEY);
        for key in first_key..=last_key {
            let y = self.y_of_key(key);
            let shade = if BLACK_KEYS[key as usize % 12] { theme::mix(p.background, Color::BLACK, 0.18) } else { p.background };
            frame.fill_rectangle(Point::new(KEYS_W, y), Size::new(size.width - KEYS_W, self.view.key_h), shade);
            let line = if key % 12 == 0 { p.grid } else { theme::mix(p.background, p.grid, 0.35) };
            frame.fill_rectangle(Point::new(KEYS_W, y + self.view.key_h - 1.0), Size::new(size.width - KEYS_W, 1.0), line);
        }

        let first_beat = self.view.scroll_beats.floor() as i64;
        let mut step = STEP_BEATS;
        while step * self.view.beat_px < 10.0 {
            step *= 2.0;
        }
        let mut beat = first_beat as f64;
        while self.x_of_beats(beat) <= size.width {
            let x = self.x_of_beats(beat).round();
            if x >= KEYS_W {
                let strength = if beat % 4.0 == 0.0 {
                    1.0
                } else if beat % 1.0 == 0.0 {
                    0.6
                } else {
                    0.25
                };
                frame.fill_rectangle(Point::new(x, RULER_H), Size::new(1.0, size.height - RULER_H), theme::mix(p.background, p.grid, strength));
                if beat % 4.0 == 0.0 {
                    frame.fill_text(Text {
                        content: format!("{}", (beat / 4.0) as i64 + 1),
                        position: Point::new(x + 4.0, 6.0),
                        color: p.text_dim,
                        size: 11.0.into(),
                        font: p.mono,
                        ..Text::default()
                    });
                }
            }
            beat += step;
        }
        if let Some(clip) = self.clip() {
            let end = self.x_of_beats(clip.len as f64 / self.frames_per_beat());
            if end < size.width {
                frame.fill_rectangle(Point::new(end, RULER_H), Size::new(size.width - end, size.height - RULER_H), theme::alpha(Color::BLACK, 0.25));
                frame.fill_rectangle(Point::new(end - 1.0, 0.0), Size::new(2.0, size.height), p.accent);
            }
        }

        let notes = self.notes();
        let hovered = cursor.position_in(bounds).and_then(|at| match self.spot(at, &notes) {
            Spot::Note { index, .. } => Some(index),
            _ => None,
        });
        for (index, note) in notes.iter().enumerate() {
            let shape = self.note_box(note);
            if shape.x + shape.width < KEYS_W || shape.x > size.width || shape.y + shape.height < RULER_H || shape.y > size.height {
                continue;
            }
            let fill = theme::mix(theme::mix(p.background, colour, 0.45), colour, note.velocity);
            let body = Path::rounded_rectangle(Point::new(shape.x + 0.5, shape.y + 1.0), Size::new(shape.width - 1.0, shape.height - 2.0), 3.0.into());
            frame.fill(&body, if hovered == Some(index) { theme::mix(fill, Color::WHITE, 0.2) } else { fill });
            if shape.width > 34.0 && self.view.key_h >= 12.0 {
                frame.fill_text(Text {
                    content: self.label(note.key),
                    position: Point::new(shape.x + 5.0, shape.y + 1.0),
                    color: theme::mix(colour, Color::BLACK, 0.75),
                    size: (self.view.key_h - 4.0).min(12.0).into(),
                    font: p.medium,
                    ..Text::default()
                });
            }
        }

        if let Some(clip) = self.clip() {
            if self.playhead >= clip.start && self.playhead < clip.end() {
                let x = self.x_of_beats((self.playhead - clip.start) as f64 / self.frames_per_beat()).round();
                if x >= KEYS_W {
                    frame.fill_rectangle(Point::new(x, 0.0), Size::new(1.0, size.height), p.accent);
                }
            }
        }

        frame.fill_rectangle(Point::ORIGIN, Size::new(KEYS_W, size.height), p.panel);
        for key in first_key..=last_key {
            let y = self.y_of_key(key);
            let black = BLACK_KEYS[key as usize % 12];
            let pressed = hand.sounding == Some(key);
            let face = match (pressed, black) {
                (true, _) => colour,
                (false, true) => theme::mix(p.panel, Color::BLACK, 0.45),
                (false, false) => theme::mix(p.panel, Color::WHITE, 0.82),
            };
            let width = if black && !self.drums() { KEYS_W * 0.62 } else { KEYS_W - 1.0 };
            frame.fill_rectangle(Point::new(0.0, y), Size::new(width, self.view.key_h - 1.0), face);
            let named = self.drums() && drum_name(key).is_some();
            if (key % 12 == 0 || named) && self.view.key_h >= 10.0 {
                frame.fill_text(Text {
                    content: self.label(key),
                    position: Point::new(KEYS_W - 6.0, y + self.view.key_h / 2.0),
                    color: if black { p.text } else { theme::mix(p.panel, Color::BLACK, 0.6) },
                    size: (self.view.key_h - 5.0).clamp(8.0, 11.0).into(),
                    font: p.medium,
                    horizontal_alignment: alignment::Horizontal::Right,
                    vertical_alignment: alignment::Vertical::Center,
                    ..Text::default()
                });
            }
        }
        frame.fill_rectangle(Point::new(0.0, 0.0), Size::new(size.width, RULER_H), p.panel);
        frame.fill_rectangle(Point::new(0.0, RULER_H - 1.0), Size::new(size.width, 1.0), p.line);
        frame.fill_rectangle(Point::new(KEYS_W - 1.0, 0.0), Size::new(1.0, size.height), p.line);
        let mut beat = first_beat as f64;
        while self.x_of_beats(beat) <= size.width {
            let x = self.x_of_beats(beat).round();
            if x >= KEYS_W && beat % 4.0 == 0.0 {
                frame.fill_text(Text {
                    content: format!("{}", (beat / 4.0) as i64 + 1),
                    position: Point::new(x + 4.0, 6.0),
                    color: p.text_dim,
                    size: 11.0.into(),
                    font: p.mono,
                    ..Text::default()
                });
            }
            beat += 1.0;
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, hand: &Hand, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        match hand.drag {
            Some(Drag::Move { .. }) => return mouse::Interaction::Grabbing,
            Some(Drag::Stretch { .. }) => return mouse::Interaction::ResizingHorizontally,
            _ => {}
        }
        let Some(at) = cursor.position_in(bounds) else {
            return mouse::Interaction::default();
        };
        match self.spot(at, &self.notes()) {
            Spot::Note { edge: true, .. } => mouse::Interaction::ResizingHorizontally,
            Spot::Note { .. } => mouse::Interaction::Grab,
            Spot::Key(_) => mouse::Interaction::Pointer,
            Spot::Empty { .. } => mouse::Interaction::Crosshair,
            _ => mouse::Interaction::default(),
        }
    }
}

impl App {
    pub(crate) fn roll_sheet(&self, clip: ClipId) -> Element<'_, Message> {
        let palette = self.palette;
        let Some(found) = self.project.clip(clip) else {
            return self.window("Piano roll".to_string(), text("This clip is gone.").into(), 400.0);
        };
        let instrument = self.project.track_of(clip).map_or(Instrument::default(), |track| track.instrument);
        let hint = text("Click to add a note, drag to move, drag its right edge to stretch, right click to delete. Type on your keyboard to play. Alt for no snapping, Ctrl and scroll to zoom.")
            .size(12)
            .color(palette.text_dim);
        let roll = canvas::Canvas::new(Roll {
            project: &self.project,
            clip,
            palette: &self.palette,
            view: self.roll_view,
            playhead: self.playhead,
            last_beats: self.roll_beats,
        })
        .width(Length::Fill)
        .height(Length::Fill);
        let body = column![
            row![text(instrument.name()).size(13).font(palette.medium), hint].spacing(12).align_y(Alignment::Center),
            container(roll).width(Length::Fill).height(Length::Fill).style(move |_| palette.strip()),
        ]
        .spacing(10);
        let tall = (self.window.height - 120.0).max(300.0);
        let wide = (self.window.width - 80.0).max(500.0);
        let sheet = container(body).width(Length::Fill).height(tall);
        self.window(format!("Piano roll: {}", found.source.name), sheet.into(), wide)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_typing_keyboard_follows_two_rows_of_piano_keys() {
        assert_eq!(typed_key("z"), Some(48));
        assert_eq!(typed_key("Z"), Some(48));
        assert_eq!(typed_key("s"), Some(49));
        assert_eq!(typed_key("q"), Some(60));
        assert_eq!(typed_key("i"), Some(72));
        assert_eq!(typed_key("a"), None);
        assert_eq!(typed_key("zz"), None);
    }
}

impl App {
    pub(crate) fn open_roll(&mut self, clip: ClipId) {
        let notes = self.project.clip(clip).and_then(|found| found.notes.as_deref().cloned()).unwrap_or_default();
        let drums = self.project.track_of(clip).is_some_and(|track| track.instrument == Instrument::Drums);
        let middle = match (notes.iter().map(|n| n.key).max(), drums) {
            (Some(top), _) => top as f32 + 6.0,
            (None, true) => 52.0,
            (None, false) => 78.0,
        };
        self.roll_view = RollView { top_key: middle.min(HIGHEST_KEY as f32), ..RollView::default() };
        self.roll_beats = if drums { 0.25 } else { 1.0 };
        self.overlay = crate::Overlay::Roll(clip);
    }

    pub(crate) fn new_notes_clip(&mut self, track: loupe_engine::TrackId) {
        self.overlay = crate::Overlay::None;
        let bar = (4.0 * 60.0 / self.project.bpm * self.project.rate as f64).round() as Frames;
        let start = self.playhead / bar.max(1) * bar;
        let name = self.project.track(track).map_or("Notes".to_string(), |t| t.name.clone());
        let made = loupe_engine::Command::AddNotesClip { track, name, start, len: bar * 4, notes: Vec::new() };
        if let Some(loupe_engine::Outcome::Clip(clip)) = self.edit(None, made) {
            self.choose([clip]);
            self.open_roll(clip);
        }
    }

    pub(crate) fn sound(&mut self, key: u8, on: bool) {
        let crate::Overlay::Roll(clip) = self.overlay else {
            return;
        };
        let Some(track) = self.project.track_of(clip).map(|track| track.id) else {
            return;
        };
        if on {
            self.engine.note_on(track, key, DEFAULT_VELOCITY);
        } else {
            self.engine.note_off(track, key);
        }
    }
}
