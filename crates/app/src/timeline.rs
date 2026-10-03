use std::collections::HashMap;

use iced::widget::canvas::{self, Cache, Frame, Geometry, Path, Stroke, Text};
use iced::{alignment, keyboard, mouse, Color, Point, Rectangle, Renderer, Size, Theme};
use loupe_engine::{Clip, ClipId, Frames, Project, Track, TrackId};

use crate::theme::{self, Palette};
use crate::{icons, Message};

pub const HEADER_W: f32 = 200.0;
const RULER_H: f32 = 30.0;
const ADD_ROW_H: f32 = 40.0;
const CLIP_PAD: f32 = 5.0;
const CLIP_TITLE_H: f32 = 19.0;
const MIN_GRID_PX: f64 = 14.0;
const DRAG_THRESHOLD: f32 = 4.0;
const RESIZE_GRIP: f32 = 5.0;
const ROOMY_HEADER_H: f32 = 72.0;
pub const MIN_ZOOM: f64 = 2.0;

pub type LoopRange = Option<(Frames, Frames)>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub zoom: f64,
    pub scroll: f64,
    pub scroll_y: f32,
}

impl View {
    pub fn max_zoom(rate: u32) -> f64 {
        rate as f64 * 8.0
    }
}

pub struct Timeline<'a> {
    pub project: &'a Project,
    pub palette: &'a Palette,
    pub view: View,
    pub heights: &'a HashMap<TrackId, f32>,
    pub selected: Option<ClipId>,
    pub playhead: Frames,
    pub loop_range: LoopRange,
    pub cache: &'a Cache,
}

#[derive(Default)]
pub struct Interaction {
    drag: Option<Drag>,
    modifiers: keyboard::Modifiers,
}

enum Drag {
    Range { anchor: Frames, origin: Point, moving: bool },
    Clip { id: ClipId, grab: f64, origin: Point, moving: bool },
    Resize { track: TrackId, top: f32 },
}

enum Hit<'a> {
    Ruler,
    Resize(&'a Track),
    Mute(&'a Track),
    Remove(&'a Track),
    AddTrack,
    Clip(&'a Clip),
    Lane,
    Nothing,
}

impl Timeline<'_> {
    fn rate(&self) -> f64 {
        self.project.rate.max(1) as f64
    }

    fn x_of(&self, frames: f64) -> f32 {
        HEADER_W + ((frames / self.rate() - self.view.scroll) * self.view.zoom) as f32
    }

    fn frames_at(&self, x: f32) -> f64 {
        (((x - HEADER_W) as f64 / self.view.zoom + self.view.scroll) * self.rate()).max(0.0)
    }

    fn height_of(&self, track: &Track) -> f32 {
        self.heights.get(&track.id).copied().unwrap_or(self.palette.track_height)
    }

    fn track_top(&self, index: usize) -> f32 {
        let above: f32 = self.project.tracks[..index].iter().map(|t| self.height_of(t)).sum();
        RULER_H + above - self.view.scroll_y
    }

    fn track_at(&self, y: f32) -> Option<usize> {
        if y < RULER_H {
            return None;
        }
        let mut top = RULER_H - self.view.scroll_y;
        for (i, track) in self.project.tracks.iter().enumerate() {
            let bottom = top + self.height_of(track);
            if y >= top && y < bottom {
                return Some(i);
            }
            top = bottom;
        }
        None
    }

    fn row_for_drag(&self, y: f32) -> usize {
        match self.track_at(y) {
            Some(i) => i,
            None if y < RULER_H - self.view.scroll_y => 0,
            None => self.project.tracks.len().saturating_sub(1),
        }
    }

    fn grid_beats(&self) -> f64 {
        let beat_px = 60.0 / self.project.bpm * self.view.zoom;
        let mut beats = 1.0 / 16.0;
        while beats * beat_px < MIN_GRID_PX {
            beats *= 2.0;
        }
        beats
    }

    fn snap(&self, frames: f64, free: bool) -> Frames {
        if free {
            return frames.round().max(0.0) as Frames;
        }
        let step = self.grid_beats() * 60.0 / self.project.bpm * self.rate();
        ((frames / step).round() * step).round().max(0.0) as Frames
    }

    fn mute_button(&self, index: usize) -> Rectangle {
        let top = self.track_top(index);
        let height = self.height_of(&self.project.tracks[index]);
        let corner = if height >= ROOMY_HEADER_H {
            Point::new(16.0, top + height - 36.0)
        } else {
            Point::new(HEADER_W - 68.0, top + 9.0)
        };
        Rectangle::new(corner, Size::new(28.0, 22.0))
    }

    fn remove_button(&self, index: usize) -> Rectangle {
        Rectangle::new(Point::new(HEADER_W - 34.0, self.track_top(index) + 8.0), Size::new(24.0, 24.0))
    }

    fn add_button(&self) -> Rectangle {
        Rectangle::new(
            Point::new(0.0, self.track_top(self.project.tracks.len())),
            Size::new(HEADER_W, ADD_ROW_H),
        )
    }

    fn content_height(&self) -> f32 {
        let tracks: f32 = self.project.tracks.iter().map(|t| self.height_of(t)).sum();
        RULER_H + tracks + ADD_ROW_H
    }

    fn resize_grip_at(&self, p: Point) -> Option<&Track> {
        if p.x >= HEADER_W {
            return None;
        }
        self.project.tracks.iter().enumerate().find_map(|(i, track)| {
            let bottom = self.track_top(i) + self.height_of(track);
            ((p.y - bottom).abs() <= RESIZE_GRIP).then_some(track)
        })
    }

    fn hit(&self, p: Point) -> Hit<'_> {
        if p.y < RULER_H {
            return if p.x >= HEADER_W { Hit::Ruler } else { Hit::Nothing };
        }
        if let Some(track) = self.resize_grip_at(p) {
            return Hit::Resize(track);
        }
        if p.x < HEADER_W {
            if let Some(i) = self.track_at(p.y) {
                let track = &self.project.tracks[i];
                if self.mute_button(i).contains(p) {
                    return Hit::Mute(track);
                }
                if self.remove_button(i).contains(p) {
                    return Hit::Remove(track);
                }
            } else if self.add_button().contains(p) {
                return Hit::AddTrack;
            }
            return Hit::Nothing;
        }
        let Some(i) = self.track_at(p.y) else {
            return Hit::Lane;
        };
        let track = &self.project.tracks[i];
        let top = self.track_top(i);
        if p.y < top + CLIP_PAD || p.y > top + self.height_of(track) - CLIP_PAD {
            return Hit::Lane;
        }
        let at = self.frames_at(p.x);
        match track.clips.iter().rev().find(|c| at >= c.start as f64 && at < c.end() as f64) {
            Some(clip) => Hit::Clip(clip),
            None => Hit::Lane,
        }
    }

    fn zoomed(&self, factor: f64, anchor_x: f32) -> View {
        let anchor = (anchor_x - HEADER_W).max(0.0) as f64;
        let time = self.view.scroll + anchor / self.view.zoom;
        let zoom = (self.view.zoom * factor).clamp(MIN_ZOOM, View::max_zoom(self.project.rate));
        View { zoom, scroll: (time - anchor / zoom).max(0.0), ..self.view }
    }

    fn scrolled(&self, dx: f32, dy: f32, bounds: Rectangle) -> View {
        let most = (self.content_height() - bounds.height).max(0.0);
        View {
            scroll: (self.view.scroll + dx as f64 / self.view.zoom).max(0.0),
            scroll_y: (self.view.scroll_y + dy).clamp(0.0, most),
            ..self.view
        }
    }
}

impl canvas::Program<Message> for Timeline<'_> {
    type State = Interaction;

    fn update(
        &self,
        state: &mut Interaction,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Message>) {
        use canvas::event::Status::{Captured, Ignored};

        let anywhere = cursor.position().map(|p| Point::new(p.x - bounds.x, p.y - bounds.y));
        let free = state.modifiers.alt() || state.modifiers.shift();

        match event {
            canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state.modifiers = modifiers;
                (Ignored, None)
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(p) = cursor.position_in(bounds) else {
                    return (Ignored, None);
                };
                let message = match self.hit(p) {
                    Hit::Ruler => {
                        let anchor = self.snap(self.frames_at(p.x), free);
                        state.drag = Some(Drag::Range { anchor, origin: p, moving: false });
                        None
                    }
                    Hit::Resize(track) => {
                        let index = self.project.tracks.iter().position(|t| t.id == track.id);
                        let top = index.map_or(RULER_H, |i| self.track_top(i));
                        state.drag = Some(Drag::Resize { track: track.id, top });
                        None
                    }
                    Hit::Mute(track) => Some(Message::ToggleMute(track.id)),
                    Hit::Remove(track) => Some(Message::RemoveTrack(track.id)),
                    Hit::AddTrack => Some(Message::AddTrack),
                    Hit::Clip(clip) => {
                        state.drag = Some(Drag::Clip {
                            id: clip.id,
                            grab: self.frames_at(p.x) - clip.start as f64,
                            origin: p,
                            moving: false,
                        });
                        Some(Message::Select(Some(clip.id)))
                    }
                    Hit::Lane => Some(Message::LaneClicked(self.snap(self.frames_at(p.x), free))),
                    Hit::Nothing => None,
                };
                (Captured, message)
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let (Some(p), Some(drag)) = (anywhere, state.drag.as_mut()) else {
                    return (Ignored, None);
                };
                match drag {
                    Drag::Range { anchor, origin, moving } => {
                        if !*moving && p.distance(*origin) < DRAG_THRESHOLD {
                            return (Captured, None);
                        }
                        *moving = true;
                        let here = self.snap(self.frames_at(p.x.max(HEADER_W)), free);
                        let range = (here != *anchor).then(|| (here.min(*anchor), here.max(*anchor)));
                        (Captured, (range != self.loop_range).then_some(Message::SetLoop(range)))
                    }
                    Drag::Clip { id, grab, origin, moving } => {
                        if !*moving && p.distance(*origin) < DRAG_THRESHOLD {
                            return (Captured, None);
                        }
                        *moving = true;
                        let (Some(clip), Some(current)) = (self.project.clip(*id), self.project.track_of(*id))
                        else {
                            return (Captured, None);
                        };
                        let start = self.snap(self.frames_at(p.x) - *grab, free);
                        let track = self.project.tracks[self.row_for_drag(p.y)].id;
                        let changed = start != clip.start || track != current.id;
                        (Captured, changed.then_some(Message::MoveClip { clip: *id, track, start }))
                    }
                    Drag::Resize { track, top } => {
                        let height = (p.y - *top).clamp(theme::MIN_TRACK_HEIGHT, theme::MAX_TRACK_HEIGHT).round();
                        let current = self.project.track(*track).map(|t| self.height_of(t));
                        let changed = current.is_some_and(|h| h != height);
                        (Captured, changed.then_some(Message::ResizeTrack { track: *track, height }))
                    }
                }
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => match state.drag.take() {
                Some(Drag::Range { anchor, moving: false, .. }) => (Captured, Some(Message::RulerClicked(anchor))),
                Some(Drag::Clip { moving: true, .. }) => (Captured, Some(Message::DragEnd)),
                Some(_) => (Captured, None),
                None => (Ignored, None),
            },
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let Some(p) = cursor.position_in(bounds) else {
                    return (Ignored, None);
                };
                let (dx, dy, zoom) = match delta {
                    mouse::ScrollDelta::Lines { x, y } => (-x * 60.0, -y * 46.0, 1.2f64.powf(y as f64)),
                    mouse::ScrollDelta::Pixels { x, y } => (-x, -y, (y as f64 * 0.005).exp()),
                };
                let overflows = self.content_height() > bounds.height;
                let view = if state.modifiers.command() {
                    self.zoomed(zoom, p.x)
                } else if state.modifiers.shift() || !overflows {
                    self.scrolled(if dx != 0.0 { dx } else { dy }, 0.0, bounds)
                } else {
                    self.scrolled(dx, dy, bounds)
                };
                (Captured, (view != self.view).then_some(Message::SetView(view)))
            }
            _ => (Ignored, None),
        }
    }

    fn draw(
        &self,
        _state: &Interaction,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let p = self.palette;
        let content = self.cache.draw(renderer, bounds.size(), |frame| {
            let lanes = Rectangle::new(
                Point::new(HEADER_W, RULER_H),
                Size::new((bounds.width - HEADER_W).max(0.0), (bounds.height - RULER_H).max(0.0)),
            );
            frame.with_clip(lanes, |frame| self.draw_lanes(frame, lanes.size()));
            let ruler = Rectangle::new(Point::new(HEADER_W, 0.0), Size::new(lanes.width, RULER_H));
            frame.with_clip(ruler, |frame| self.draw_ruler(frame, ruler.size()));
            let headers = Rectangle::new(Point::new(0.0, RULER_H), Size::new(HEADER_W, lanes.height));
            frame.with_clip(headers, |frame| self.draw_headers(frame, headers.size()));

            frame.fill_rectangle(Point::ORIGIN, Size::new(HEADER_W, RULER_H), p.panel);
            frame.fill_rectangle(Point::new(0.0, RULER_H - 1.0), Size::new(bounds.width, 1.0), p.line);
            frame.fill_rectangle(Point::new(HEADER_W - 1.0, 0.0), Size::new(1.0, bounds.height), p.line);
        });

        let mut overlay = Frame::new(renderer, bounds.size());
        if let Some((from, to)) = self.loop_range {
            let left = self.x_of(from as f64).max(HEADER_W);
            let right = self.x_of(to as f64).min(bounds.width);
            if right > left {
                let width = right - left;
                overlay.fill_rectangle(
                    Point::new(left, 0.0),
                    Size::new(width, RULER_H - 1.0),
                    theme::mix(p.panel, p.accent, 0.28),
                );
                overlay.fill_rectangle(
                    Point::new(left, RULER_H),
                    Size::new(width, bounds.height - RULER_H),
                    theme::alpha(p.accent, 0.05),
                );
                for x in [left, right] {
                    overlay.fill_rectangle(
                        Point::new(x.round() - 0.5, RULER_H),
                        Size::new(1.0, bounds.height - RULER_H),
                        theme::alpha(p.accent, 0.45),
                    );
                }
            }
        }
        let x = self.x_of(self.playhead as f64).round();
        if x >= HEADER_W && x <= bounds.width {
            overlay.fill_rectangle(Point::new(x - 0.5, 0.0), Size::new(1.0, bounds.height), p.accent);
            let cap = Path::new(|b| {
                b.move_to(Point::new(x - 5.5, 0.0));
                b.line_to(Point::new(x + 5.5, 0.0));
                b.line_to(Point::new(x, 9.0));
                b.close();
            });
            overlay.fill(&cap, p.accent);
        }
        vec![content, overlay.into_geometry()]
    }

    fn mouse_interaction(&self, state: &Interaction, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        match state.drag {
            Some(Drag::Clip { moving: true, .. }) => return mouse::Interaction::Grabbing,
            Some(Drag::Resize { .. }) => return mouse::Interaction::ResizingVertically,
            _ => {}
        }
        match cursor.position_in(bounds).map(|p| self.hit(p)) {
            Some(Hit::Resize(_)) => mouse::Interaction::ResizingVertically,
            Some(Hit::Mute(_) | Hit::Remove(_) | Hit::AddTrack) => mouse::Interaction::Pointer,
            Some(Hit::Clip(_)) => mouse::Interaction::Grab,
            _ => mouse::Interaction::default(),
        }
    }
}

impl Timeline<'_> {
    fn draw_lanes(&self, frame: &mut Frame, size: Size) {
        let p = self.palette;
        frame.fill_rectangle(Point::ORIGIN, size, p.background);
        let beat = 60.0 / self.project.bpm;
        let step = self.grid_beats();
        let first = (self.view.scroll / (step * beat)).floor() as i64;
        for k in first.. {
            let beats = k as f64 * step;
            let x = ((beats * beat - self.view.scroll) * self.view.zoom) as f32;
            if x > size.width {
                break;
            }
            let strength = if beats % 4.0 == 0.0 {
                0.085
            } else if beats % 1.0 == 0.0 {
                0.045
            } else {
                0.022
            };
            frame.fill_rectangle(
                Point::new(x.round(), 0.0),
                Size::new(1.0, size.height),
                theme::mix(p.background, p.text, strength),
            );
        }

        if self.project.tracks.is_empty() {
            frame.fill_text(Text {
                content: "Import audio to begin".into(),
                position: Point::new(size.width / 2.0, size.height / 2.0 - 10.0),
                color: p.text_dim,
                size: 15.0.into(),
                font: p.medium,
                horizontal_alignment: alignment::Horizontal::Center,
                vertical_alignment: alignment::Vertical::Center,
                ..Text::default()
            });
            return;
        }

        for (i, track) in self.project.tracks.iter().enumerate() {
            let top = self.track_top(i) - RULER_H;
            let height = self.height_of(track);
            if top > size.height || top + height < 0.0 {
                continue;
            }
            frame.fill_rectangle(Point::new(0.0, top + height - 1.0), Size::new(size.width, 1.0), p.line);
            let colour = if track.muted { p.text_faint } else { p.track(i) };
            for clip in &track.clips {
                self.draw_clip(frame, size, clip, top, height, colour);
            }
        }
    }

    fn draw_clip(&self, frame: &mut Frame, size: Size, clip: &Clip, lane_top: f32, lane_height: f32, colour: Color) {
        let p = self.palette;
        let left = self.x_of(clip.start as f64) - HEADER_W;
        let right = self.x_of(clip.end() as f64) - HEADER_W;
        if right < 0.0 || left > size.width {
            return;
        }
        let selected = self.selected == Some(clip.id);
        let top = lane_top + CLIP_PAD;
        let height = lane_height - CLIP_PAD * 2.0;
        let shown_left = left.max(-8.0);
        let shown_right = right.min(size.width + 8.0);
        let shown_width = (shown_right - shown_left).max(1.0);
        let body = Path::new(|b| {
            b.rounded_rectangle(Point::new(shown_left, top), Size::new(shown_width, height), 5.0.into());
        });
        let tint = if selected { 0.26 } else { 0.16 };
        frame.fill(&body, theme::mix(p.background, colour, tint));
        let title = Path::new(|b| {
            b.rounded_rectangle(Point::new(shown_left, top), Size::new(shown_width, CLIP_TITLE_H), iced::border::top(5));
        });
        frame.fill(&title, theme::mix(p.background, colour, tint + 0.18));

        let wave_top = top + CLIP_TITLE_H + 2.0;
        let wave_height = height - CLIP_TITLE_H - 5.0;
        if wave_height > 4.0 {
            self.draw_waveform(frame, clip, left, shown_left.max(0.0), shown_right.min(size.width), wave_top, wave_height, colour);
        }

        let outline = if selected {
            Stroke::default().with_color(p.accent).with_width(1.5)
        } else {
            Stroke::default().with_color(theme::mix(p.background, colour, 0.5)).with_width(1.0)
        };
        frame.stroke(&body, outline);

        let title_on_canvas = Rectangle::new(
            Point::new(HEADER_W + shown_left, RULER_H + top),
            Size::new(shown_width, CLIP_TITLE_H),
        );
        let lanes_on_canvas = Rectangle::new(Point::new(HEADER_W, RULER_H), size);
        if let Some(visible) = title_on_canvas.intersection(&lanes_on_canvas) {
            frame.with_clip(visible, |name_region| {
                name_region.fill_text(Text {
                    content: clip.source.name.clone(),
                    position: Point::new(
                        HEADER_W + left.max(0.0) + 8.0 - visible.x,
                        title_on_canvas.y - visible.y + CLIP_TITLE_H / 2.0,
                    ),
                    color: p.text,
                    size: 11.5.into(),
                    font: p.medium,
                    vertical_alignment: alignment::Vertical::Center,
                    ..Text::default()
                });
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_waveform(
        &self,
        frame: &mut Frame,
        clip: &Clip,
        clip_left: f32,
        from_x: f32,
        to_x: f32,
        top: f32,
        height: f32,
        colour: Color,
    ) {
        if to_x <= from_x {
            return;
        }
        let middle = top + height / 2.0;
        let reach = height / 2.0;
        let frames_per_px = self.rate() / self.view.zoom;
        let source_at = |x: f32| clip.offset as f64 + (x - clip_left) as f64 * frames_per_px;
        let source_end = (clip.offset + clip.len) as f64;

        if frames_per_px < 1.0 {
            let first = source_at(from_x).floor().max(clip.offset as f64) as usize;
            let last = (source_at(to_x).ceil().min(source_end - 1.0)) as usize;
            let samples = &clip.source.frames;
            let point = |i: usize| {
                let value = ((samples[i][0] + samples[i][1]) * 0.5 * clip.gain).clamp(-1.0, 1.0);
                Point::new(
                    clip_left + ((i as f64 - clip.offset as f64) / frames_per_px) as f32,
                    middle - value * reach,
                )
            };
            let last = last.min(samples.len().saturating_sub(1));
            if samples.is_empty() || first > last {
                return;
            }
            let line = Path::new(|b| {
                b.move_to(point(first));
                for i in first + 1..=last {
                    b.line_to(point(i));
                }
            });
            frame.stroke(&line, Stroke::default().with_color(colour).with_width(1.5));
            if frames_per_px < 0.2 {
                for i in first..=last {
                    let p = point(i);
                    frame.fill_rectangle(Point::new(p.x - 2.0, p.y - 2.0), Size::new(4.0, 4.0), colour);
                }
            }
            return;
        }

        let columns = (to_x - from_x).ceil() as usize;
        let mut highs = Vec::with_capacity(columns);
        let mut lows = Vec::with_capacity(columns);
        for c in 0..columns {
            let x = from_x + c as f32;
            let a = source_at(x).max(clip.offset as f64);
            let b = (a + frames_per_px).min(source_end);
            if b <= a {
                break;
            }
            let (lo, hi) = clip.source.peak(a as usize, (b.ceil() as usize).max(a as usize + 1));
            let hi = middle - (hi * clip.gain).clamp(-1.0, 1.0) * reach;
            let lo = middle - (lo * clip.gain).clamp(-1.0, 1.0) * reach;
            let thin = (1.0 - (lo - hi)).max(0.0) / 2.0;
            highs.push(Point::new(x, hi - thin));
            lows.push(Point::new(x, lo + thin));
        }
        if highs.len() < 2 {
            return;
        }
        let shape = Path::new(|b| {
            b.move_to(highs[0]);
            for p in &highs[1..] {
                b.line_to(*p);
            }
            for p in lows.iter().rev() {
                b.line_to(*p);
            }
            b.close();
        });
        frame.fill(&shape, colour);
    }

    fn draw_ruler(&self, frame: &mut Frame, size: Size) {
        let p = self.palette;
        frame.fill_rectangle(Point::ORIGIN, size, p.panel);
        let bar = 4.0 * 60.0 / self.project.bpm;
        let bar_px = bar * self.view.zoom;
        let mut every = 1.0;
        while bar_px * every < 64.0 {
            every *= 2.0;
        }
        let first = (self.view.scroll / (bar * every)).floor() as i64;
        for k in first.. {
            let bars = k as f64 * every;
            let x = ((bars * bar - self.view.scroll) * self.view.zoom) as f32;
            if x > size.width {
                break;
            }
            frame.fill_rectangle(Point::new(x.round(), size.height - 9.0), Size::new(1.0, 9.0), p.text_faint);
            frame.fill_text(Text {
                content: format!("{}", bars as i64 + 1),
                position: Point::new(x.round() + 5.0, size.height / 2.0 - 1.0),
                color: p.text_dim,
                size: 11.0.into(),
                font: p.mono,
                vertical_alignment: alignment::Vertical::Center,
                ..Text::default()
            });
        }
        let beat = bar / 4.0;
        if beat * self.view.zoom >= MIN_GRID_PX {
            let first = (self.view.scroll / beat).floor() as i64;
            for k in first.. {
                let x = ((k as f64 * beat - self.view.scroll) * self.view.zoom) as f32;
                if x > size.width {
                    break;
                }
                if k % 4 != 0 {
                    frame.fill_rectangle(
                        Point::new(x.round(), size.height - 5.0),
                        Size::new(1.0, 5.0),
                        theme::mix(p.panel, p.text_faint, 0.6),
                    );
                }
            }
        }
    }

    fn draw_headers(&self, frame: &mut Frame, size: Size) {
        let p = self.palette;
        frame.fill_rectangle(Point::ORIGIN, size, p.panel);
        for (i, track) in self.project.tracks.iter().enumerate() {
            let top = self.track_top(i) - RULER_H;
            let height = self.height_of(track);
            if top > size.height || top + height < 0.0 {
                continue;
            }
            frame.fill_rectangle(Point::new(0.0, top), Size::new(3.0, height - 1.0), p.track(i));
            frame.fill_rectangle(Point::new(0.0, top + height - 1.0), Size::new(size.width, 1.0), p.line);
            frame.fill_text(Text {
                content: shorten(&track.name, if height >= ROOMY_HEADER_H { 20 } else { 14 }),
                position: Point::new(16.0, top + 20.0),
                color: if track.muted { p.text_dim } else { p.text },
                size: 13.0.into(),
                font: p.medium,
                vertical_alignment: alignment::Vertical::Center,
                ..Text::default()
            });

            let remove = self.remove_button(i);
            frame.fill_text(Text {
                content: icons::glyph("x").to_string(),
                position: Point::new(remove.center_x(), remove.center_y() - RULER_H),
                color: p.text_faint,
                size: 14.0.into(),
                font: theme::ICONS,
                horizontal_alignment: alignment::Horizontal::Center,
                vertical_alignment: alignment::Vertical::Center,
                shaping: iced::widget::text::Shaping::Advanced,
                ..Text::default()
            });

            let mute = self.mute_button(i);
            let shape = Path::new(|b| {
                b.rounded_rectangle(Point::new(mute.x, mute.y - RULER_H), mute.size(), 5.0.into());
            });
            frame.fill(&shape, if track.muted { p.danger } else { p.raised });
            frame.fill_text(Text {
                content: "M".into(),
                position: Point::new(mute.center_x(), mute.center_y() - RULER_H),
                color: if track.muted { p.on_accent } else { p.text_dim },
                size: 11.5.into(),
                font: p.semibold,
                horizontal_alignment: alignment::Horizontal::Center,
                vertical_alignment: alignment::Vertical::Center,
                ..Text::default()
            });
        }

        let add = self.add_button();
        frame.fill_text(Text {
            content: "+  Add track".into(),
            position: Point::new(16.0, add.center_y() - RULER_H),
            color: p.text_dim,
            size: 12.5.into(),
            font: p.ui,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        });
    }
}

fn shorten(name: &str, most: usize) -> String {
    if name.chars().count() <= most {
        name.to_string()
    } else {
        let mut short: String = name.chars().take(most - 1).collect();
        short.push('…');
        short
    }
}
