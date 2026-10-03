use std::collections::HashMap;

use iced::widget::canvas::{self, Cache, Frame, Geometry, Path, Stroke, Text};
use iced::{alignment, keyboard, mouse, Color, Point, Rectangle, Renderer, Size, Theme};
use loupe_engine::{Clip, ClipId, Edge, Fade, Frames, Project, Track, TrackId};

use crate::pointer::Anchor;
use crate::theme::{self, Palette};
use crate::{icons, Message};

pub const HEADER_W: f32 = 200.0;
const SCROLLBAR_H: f32 = 18.0;
const RULER_H: f32 = 30.0;
const LANES_TOP: f32 = SCROLLBAR_H + RULER_H;
const MIN_THUMB_PX: f32 = 28.0;
const SONG_SPAN_HEADROOM: f64 = 1.2;
const SHORTEST_SPAN_SECONDS: f64 = 30.0;
const POINTER_LEASH_PX: f32 = 40.0;
const POINTER_HOME_SLACK_PX: f32 = 1.5;
const ADD_ROW_H: f32 = 40.0;
const CLIP_PAD: f32 = 5.0;
const CLIP_TITLE_H: f32 = 21.0;
const MIN_GRID_PX: f64 = 14.0;
const DRAG_THRESHOLD: f32 = 4.0;
const RESIZE_GRIP: f32 = 5.0;
const ROOMY_HEADER_H: f32 = 72.0;
const HANDLE_RADIUS: f32 = 4.5;
const HANDLE_REACH: f32 = 10.0;
const FADE_FLAG_SIZE: f32 = 10.0;
const GAIN_HANDLE_INSET: f32 = 7.0;
const MIN_CLIP_PX_FOR_HANDLES: f32 = 36.0;
const MIN_FADE_PX_FOR_SHAPE_HANDLE: f32 = 24.0;
const CURVE_PER_PX: f32 = 1.0 / 50.0;
const GAIN_DB_PER_PX: f32 = 0.1;
pub const MIN_GAIN_DB: f32 = -24.0;
pub const MAX_GAIN_DB: f32 = 12.0;
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
    pub width: f32,
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
    Scroll { grab_x: f32, span: f64 },
    Grip {
        clip: ClipId,
        grip: Grip,
        origin: Point,
        last: Point,
        travel_up: f32,
        awaiting_return: bool,
        anchor: Option<Anchor>,
        curve_at_grab: f32,
        db_at_grab: f32,
    },
}

#[derive(Clone, Copy, PartialEq)]
enum Grip {
    FadeIn,
    FadeOut,
    ShapeIn,
    ShapeOut,
    Gain,
}

struct ClipBox {
    left: f32,
    right: f32,
    top: f32,
    height: f32,
}

impl ClipBox {
    fn wave_top(&self) -> f32 {
        self.top + CLIP_TITLE_H + 2.0
    }

    fn wave_height(&self) -> f32 {
        self.height - CLIP_TITLE_H - 5.0
    }
}

enum Hit<'a> {
    Scrollbar,
    Ruler,
    Resize(&'a Track),
    Mute(&'a Track),
    Remove(&'a Track),
    AddTrack,
    Grip(&'a Clip, Grip),
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
        LANES_TOP + above - self.view.scroll_y
    }

    fn track_at(&self, y: f32) -> Option<usize> {
        if y < LANES_TOP {
            return None;
        }
        let mut top = LANES_TOP - self.view.scroll_y;
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
            None if y < LANES_TOP - self.view.scroll_y => 0,
            None => self.project.tracks.len().saturating_sub(1),
        }
    }

    fn grid_beats(&self) -> f64 {
        let beat_px = 60.0 / self.project.bpm * self.view.zoom;
        let frames_per_beat = 60.0 / self.project.bpm * self.rate();
        let mut beats = 1.0;
        while beats * beat_px < MIN_GRID_PX {
            beats *= 2.0;
        }
        while beats * beat_px >= MIN_GRID_PX * 2.0 && beats * frames_per_beat >= 2.0 {
            beats /= 2.0;
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
        LANES_TOP + tracks + ADD_ROW_H
    }

    fn colour_of(&self, index: usize) -> Color {
        match self.project.tracks[index].colour {
            Some([r, g, b]) => Color::from_rgb8(r, g, b),
            None => self.palette.track(self.project.tracks[index].id.0.saturating_sub(1) as usize),
        }
    }

    fn track_header_at(&self, p: Point) -> Option<&Track> {
        if p.x >= HEADER_W {
            return None;
        }
        self.track_at(p.y).map(|i| &self.project.tracks[i])
    }

    fn lanes_width(&self) -> f32 {
        (self.width - HEADER_W).max(1.0)
    }

    fn scrollbar_span(&self) -> f64 {
        let song = self.project.length() as f64 / self.rate();
        (song * SONG_SPAN_HEADROOM).max(SHORTEST_SPAN_SECONDS)
    }

    fn within_span(&self, scroll: f64, zoom: f64) -> f64 {
        let seen = self.lanes_width() as f64 / zoom;
        scroll.clamp(0.0, (self.scrollbar_span() - seen).max(0.0))
    }

    fn thumb(&self, span: f64) -> (f32, f32) {
        let track = self.lanes_width();
        let seen = track as f64 / self.view.zoom;
        let width = ((seen / span) as f32 * track).clamp(MIN_THUMB_PX.min(track), track);
        let left = HEADER_W + (self.view.scroll / span) as f32 * track;
        (left.min(self.width - width), width)
    }

    fn scrolled_to_thumb_left(&self, left: f32, span: f64) -> View {
        let scroll = ((left - HEADER_W) / self.lanes_width()) as f64 * span;
        View { scroll: self.within_span(scroll, self.view.zoom), ..self.view }
    }

    fn clip_box(&self, clip: &Clip) -> Option<ClipBox> {
        let index = self.project.tracks.iter().position(|t| t.clips.iter().any(|c| c.id == clip.id))?;
        Some(ClipBox {
            left: self.x_of(clip.start as f64),
            right: self.x_of(clip.end() as f64),
            top: self.track_top(index) + CLIP_PAD,
            height: self.height_of(&self.project.tracks[index]) - CLIP_PAD * 2.0,
        })
    }

    fn fade_curve_point(&self, clip: &Clip, shape: &ClipBox, edge: Edge, progress: f32) -> Point {
        let (fade, x) = match edge {
            Edge::In => {
                let width = self.x_of((clip.start + clip.fade_in.len) as f64) - shape.left;
                (clip.fade_in, shape.left + width * progress)
            }
            Edge::Out => {
                let width = shape.right - self.x_of((clip.end() - clip.fade_out.len) as f64);
                (clip.fade_out, shape.right - width * progress)
            }
        };
        Point::new(x, shape.wave_top() + shape.wave_height() * (1.0 - fade.level(progress)))
    }

    fn grips(&self, clip: &Clip) -> Vec<(Grip, Point)> {
        let Some(shape) = self.clip_box(clip) else {
            return Vec::new();
        };
        if self.selected != Some(clip.id)
            || shape.right - shape.left < MIN_CLIP_PX_FOR_HANDLES
            || shape.wave_height() < 20.0
        {
            return Vec::new();
        }
        let fade_in_end = self.fade_curve_point(clip, &shape, Edge::In, 1.0);
        let fade_out_start = self.fade_curve_point(clip, &shape, Edge::Out, 1.0);
        let mut grips = Vec::with_capacity(5);
        if fade_in_end.x - shape.left >= MIN_FADE_PX_FOR_SHAPE_HANDLE {
            grips.push((Grip::ShapeIn, self.fade_curve_point(clip, &shape, Edge::In, 0.5)));
        }
        if shape.right - fade_out_start.x >= MIN_FADE_PX_FOR_SHAPE_HANDLE {
            grips.push((Grip::ShapeOut, self.fade_curve_point(clip, &shape, Edge::Out, 0.5)));
        }
        grips.push((Grip::FadeIn, fade_in_end));
        grips.push((Grip::FadeOut, fade_out_start));
        let seen_left = shape.left.max(HEADER_W);
        let seen_right = shape.right.min(self.width);
        grips.push((Grip::Gain, Point::new((seen_left + seen_right) / 2.0, shape.top + shape.height - GAIN_HANDLE_INSET)));
        grips
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
        if p.y < LANES_TOP {
            return match (p.x >= HEADER_W, p.y < SCROLLBAR_H) {
                (true, true) => Hit::Scrollbar,
                (true, false) => Hit::Ruler,
                (false, _) => Hit::Nothing,
            };
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
        if let Some(clip) = self.selected.and_then(|id| self.project.clip(id)) {
            let reached = self.grips(clip).into_iter().find(|(_, at)| at.distance(p) <= HANDLE_REACH);
            if let Some((grip, _)) = reached {
                return Hit::Grip(clip, grip);
            }
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
        View { zoom, scroll: self.within_span(time - anchor / zoom, zoom), ..self.view }
    }

    fn scrolled(&self, dx: f32, dy: f32, bounds: Rectangle) -> View {
        let most = (self.content_height() - bounds.height).max(0.0);
        View {
            scroll: self.within_span(self.view.scroll + dx as f64 / self.view.zoom, self.view.zoom),
            scroll_y: (self.view.scroll_y + dy).clamp(0.0, most),
            ..self.view
        }
    }
}

impl Grip {
    fn moves_sideways(self) -> bool {
        matches!(self, Grip::FadeIn | Grip::FadeOut)
    }

    fn pointer(self) -> mouse::Interaction {
        match self {
            Grip::FadeIn | Grip::FadeOut => mouse::Interaction::ResizingHorizontally,
            Grip::ShapeIn | Grip::ShapeOut | Grip::Gain => mouse::Interaction::ResizingVertically,
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
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                let Some(p) = cursor.position_in(bounds) else {
                    return (Ignored, None);
                };
                let message = match self.hit(p) {
                    Hit::Clip(clip) | Hit::Grip(clip, _) => Some(Message::DeleteClip(clip.id)),
                    _ => self.track_header_at(p).map(|track| Message::TrackMenu {
                        track: track.id,
                        at: Point::new(bounds.x + p.x, bounds.y + p.y),
                    }),
                };
                (Captured, message)
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(p) = cursor.position_in(bounds) else {
                    return (Ignored, None);
                };
                let message = match self.hit(p) {
                    Hit::Scrollbar => {
                        let span = self.scrollbar_span();
                        let (left, width) = self.thumb(span);
                        let on_thumb = p.x >= left && p.x <= left + width;
                        let grab_x = if on_thumb { p.x - left } else { width / 2.0 };
                        state.drag = Some(Drag::Scroll { grab_x, span });
                        let view = self.scrolled_to_thumb_left(p.x - grab_x, span);
                        (view != self.view).then_some(Message::SetView(view))
                    }
                    Hit::Ruler => {
                        let anchor = self.snap(self.frames_at(p.x), free);
                        state.drag = Some(Drag::Range { anchor, origin: p, moving: false });
                        None
                    }
                    Hit::Resize(track) => {
                        let index = self.project.tracks.iter().position(|t| t.id == track.id);
                        let top = index.map_or(LANES_TOP, |i| self.track_top(i));
                        state.drag = Some(Drag::Resize { track: track.id, top });
                        None
                    }
                    Hit::Mute(track) => Some(Message::ToggleMute(track.id)),
                    Hit::Remove(track) => Some(Message::RemoveTrack(track.id)),
                    Hit::AddTrack => Some(Message::AddTrack),
                    Hit::Grip(clip, grip) => {
                        let curve_at_grab = match grip {
                            Grip::ShapeOut => clip.fade_out.curve,
                            _ => clip.fade_in.curve,
                        };
                        let db_at_grab = (20.0 * clip.gain.max(1e-6).log10()).clamp(MIN_GAIN_DB, MAX_GAIN_DB);
                        let anchor = if grip.moves_sideways() { None } else { Anchor::here() };
                        state.drag = Some(Drag::Grip {
                            clip: clip.id,
                            grip,
                            origin: p,
                            last: p,
                            travel_up: 0.0,
                            awaiting_return: false,
                            anchor,
                            curve_at_grab,
                            db_at_grab,
                        });
                        None
                    }
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
                    Drag::Grip { clip, grip, origin, last, travel_up, awaiting_return, anchor, curve_at_grab, db_at_grab } => {
                        let Some(clip) = self.project.clip(*clip) else {
                            return (Captured, None);
                        };
                        if *awaiting_return && p.distance(*origin) <= POINTER_HOME_SLACK_PX {
                            *awaiting_return = false;
                            *last = p;
                            return (Captured, None);
                        }
                        *travel_up += last.y - p.y;
                        *last = p;
                        if p.distance(*origin) > POINTER_LEASH_PX && !*awaiting_return {
                            *awaiting_return = anchor.as_ref().is_some_and(Anchor::bring_pointer_back);
                        }
                        let at = self.snap(self.frames_at(p.x), free);
                        let bent = (*curve_at_grab + *travel_up * CURVE_PER_PX).clamp(-1.0, 1.0);
                        let message = match grip {
                            Grip::FadeIn => {
                                let len = at.saturating_sub(clip.start).min(clip.len - clip.fade_out.len);
                                let fade = Fade { len, ..clip.fade_in };
                                (fade != clip.fade_in).then_some(Message::SetFade { clip: clip.id, edge: Edge::In, fade })
                            }
                            Grip::FadeOut => {
                                let len = clip.end().saturating_sub(at).min(clip.len - clip.fade_in.len);
                                let fade = Fade { len, ..clip.fade_out };
                                (fade != clip.fade_out).then_some(Message::SetFade { clip: clip.id, edge: Edge::Out, fade })
                            }
                            Grip::ShapeIn => {
                                let fade = Fade { curve: bent, ..clip.fade_in };
                                (fade != clip.fade_in).then_some(Message::SetFade { clip: clip.id, edge: Edge::In, fade })
                            }
                            Grip::ShapeOut => {
                                let fade = Fade { curve: bent, ..clip.fade_out };
                                (fade != clip.fade_out).then_some(Message::SetFade { clip: clip.id, edge: Edge::Out, fade })
                            }
                            Grip::Gain => {
                                let db = (*db_at_grab + *travel_up * GAIN_DB_PER_PX).clamp(MIN_GAIN_DB, MAX_GAIN_DB);
                                Some(Message::ClipGain(db))
                            }
                        };
                        (Captured, message)
                    }
                    Drag::Scroll { grab_x, span } => {
                        let view = self.scrolled_to_thumb_left(p.x - *grab_x, *span);
                        (Captured, (view != self.view).then_some(Message::SetView(view)))
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
                Some(Drag::Clip { moving: true, .. } | Drag::Grip { .. }) => (Captured, Some(Message::DragEnd)),
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
        state: &Interaction,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let p = self.palette;
        let content = self.cache.draw(renderer, bounds.size(), |frame| {
            let lanes = Rectangle::new(
                Point::new(HEADER_W, LANES_TOP),
                Size::new((bounds.width - HEADER_W).max(0.0), (bounds.height - LANES_TOP).max(0.0)),
            );
            frame.with_clip(lanes, |frame| self.draw_lanes(frame, lanes.size()));
            let ruler = Rectangle::new(Point::new(HEADER_W, SCROLLBAR_H), Size::new(lanes.width, RULER_H));
            let scrollbar = Rectangle::new(Point::new(HEADER_W, 0.0), Size::new(lanes.width, SCROLLBAR_H));
            frame.with_clip(scrollbar, |frame| self.draw_scrollbar(frame, scrollbar.size()));
            frame.with_clip(ruler, |frame| self.draw_ruler(frame, ruler.size()));
            let headers = Rectangle::new(Point::new(0.0, LANES_TOP), Size::new(HEADER_W, lanes.height));
            frame.with_clip(headers, |frame| self.draw_headers(frame, headers.size()));

            frame.fill_rectangle(Point::ORIGIN, Size::new(HEADER_W, LANES_TOP), p.panel);
            frame.fill_rectangle(Point::new(0.0, LANES_TOP - 1.0), Size::new(bounds.width, 1.0), p.line);
            frame.fill_rectangle(Point::new(HEADER_W - 1.0, 0.0), Size::new(1.0, bounds.height), p.line);
        });

        let mut overlay = Frame::new(renderer, bounds.size());
        if let Some((from, to)) = self.loop_range {
            let left = self.x_of(from as f64).max(HEADER_W);
            let right = self.x_of(to as f64).min(bounds.width);
            if right > left {
                let width = right - left;
                overlay.fill_rectangle(
                    Point::new(left, SCROLLBAR_H),
                    Size::new(width, RULER_H - 1.0),
                    theme::mix(p.panel, p.accent, 0.28),
                );
                overlay.fill_rectangle(
                    Point::new(left, LANES_TOP),
                    Size::new(width, bounds.height - LANES_TOP),
                    theme::alpha(p.accent, 0.05),
                );
                for x in [left, right] {
                    overlay.fill_rectangle(
                        Point::new(x.round() - 0.5, LANES_TOP),
                        Size::new(1.0, bounds.height - LANES_TOP),
                        theme::alpha(p.accent, 0.45),
                    );
                }
            }
        }
        let x = self.x_of(self.playhead as f64).round();
        if x >= HEADER_W && x <= bounds.width {
            overlay.fill_rectangle(Point::new(x - 0.5, SCROLLBAR_H), Size::new(1.0, bounds.height - SCROLLBAR_H), p.accent);
            let cap = Path::new(|b| {
                b.move_to(Point::new(x - 5.5, SCROLLBAR_H));
                b.line_to(Point::new(x + 5.5, SCROLLBAR_H));
                b.line_to(Point::new(x, SCROLLBAR_H + 9.0));
                b.close();
            });
            overlay.fill(&cap, p.accent);
        }
        if let Some(Drag::Grip { clip, grip: Grip::Gain, .. }) = &state.drag {
            let handle = self
                .project
                .clip(*clip)
                .and_then(|clip| self.grips(clip).into_iter().find(|(grip, _)| *grip == Grip::Gain).map(|(_, at)| (clip, at)));
            if let Some((clip, at)) = handle {
                let db = 20.0 * clip.gain.max(1e-6).log10();
                let label = Size::new(64.0, 20.0);
                let centre = Point::new(at.x, at.y - 24.0);
                let pill = Path::new(|b| {
                    b.rounded_rectangle(
                        Point::new(centre.x - label.width / 2.0, centre.y - label.height / 2.0),
                        label,
                        5.0.into(),
                    );
                });
                overlay.fill(&pill, p.raised);
                overlay.stroke(&pill, Stroke::default().with_color(p.line).with_width(1.0));
                overlay.fill_text(Text {
                    content: format!("{db:+.1} dB"),
                    position: centre,
                    color: p.text,
                    size: 11.5.into(),
                    font: p.mono,
                    horizontal_alignment: alignment::Horizontal::Center,
                    vertical_alignment: alignment::Vertical::Center,
                    ..Text::default()
                });
            }
        }
        vec![content, overlay.into_geometry()]
    }

    fn mouse_interaction(&self, state: &Interaction, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        match state.drag {
            Some(Drag::Clip { moving: true, .. }) => return mouse::Interaction::Grabbing,
            Some(Drag::Resize { .. }) => return mouse::Interaction::ResizingVertically,
            Some(Drag::Grip { grip, .. }) => return grip.pointer(),
            _ => {}
        }
        match cursor.position_in(bounds).map(|p| self.hit(p)) {
            Some(Hit::Resize(_)) => mouse::Interaction::ResizingVertically,
            Some(Hit::Grip(_, grip)) => grip.pointer(),
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
                1.0
            } else if beats % 1.0 == 0.0 {
                0.55
            } else {
                0.28
            };
            frame.fill_rectangle(
                Point::new(x.round(), 0.0),
                Size::new(1.0, size.height),
                theme::mix(p.background, p.grid, strength),
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
            let top = self.track_top(i) - LANES_TOP;
            let height = self.height_of(track);
            if top > size.height || top + height < 0.0 {
                continue;
            }
            frame.fill_rectangle(Point::new(0.0, top + height - 1.0), Size::new(size.width, 1.0), p.line);
            let colour = if track.muted { p.text_faint } else { self.colour_of(i) };
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
        self.draw_fades_and_grips(frame, clip, colour);

        let title_on_canvas = Rectangle::new(
            Point::new(HEADER_W + shown_left, LANES_TOP + top),
            Size::new(shown_width, CLIP_TITLE_H),
        );
        let lanes_on_canvas = Rectangle::new(Point::new(HEADER_W, LANES_TOP), size);
        if let Some(visible) = title_on_canvas.intersection(&lanes_on_canvas) {
            frame.with_clip(visible, |name_region| {
                name_region.fill_text(Text {
                    content: clip.source.name.clone(),
                    position: Point::new(
                        HEADER_W + left.max(0.0) + 8.0 - visible.x,
                        title_on_canvas.y - visible.y + CLIP_TITLE_H / 2.0,
                    ),
                    color: p.text,
                    size: 13.0.into(),
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
                let level = clip.gain * clip.fade_level(i as Frames - clip.offset);
                let value = ((samples[i][0] + samples[i][1]) * 0.5 * level).clamp(-1.0, 1.0);
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
            let level = clip.gain * clip.fade_level((a - clip.offset as f64) as Frames);
            let hi = middle - (hi * level).clamp(-1.0, 1.0) * reach;
            let lo = middle - (lo * level).clamp(-1.0, 1.0) * reach;
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

    fn draw_fades_and_grips(&self, frame: &mut Frame, clip: &Clip, colour: Color) {
        let p = self.palette;
        let Some(shape) = self.clip_box(clip) else {
            return;
        };
        let in_lanes = |at: Point| Point::new(at.x - HEADER_W, at.y - LANES_TOP);
        let curve_colour = theme::mix(colour, p.text, 0.55);
        for (edge, fade) in [(Edge::In, clip.fade_in), (Edge::Out, clip.fade_out)] {
            if fade.len == 0 || shape.wave_height() < 8.0 {
                continue;
            }
            let steps = 24;
            let curve = Path::new(|b| {
                b.move_to(in_lanes(self.fade_curve_point(clip, &shape, edge, 0.0)));
                for step in 1..=steps {
                    b.line_to(in_lanes(self.fade_curve_point(clip, &shape, edge, step as f32 / steps as f32)));
                }
            });
            frame.stroke(&curve, Stroke::default().with_color(curve_colour).with_width(1.25));
        }
        let selected_body = theme::mix(p.background, colour, 0.26);
        let on_screen = |at: &Point| at.x >= HEADER_W - HANDLE_REACH && at.x <= self.width + HANDLE_REACH;
        for (grip, at) in self.grips(clip).into_iter().filter(|(_, at)| on_screen(at)) {
            let at = in_lanes(at);
            match grip {
                Grip::FadeIn | Grip::FadeOut => {
                    let (inward, fade) = if grip == Grip::FadeIn { (1.0, clip.fade_in) } else { (-1.0, clip.fade_out) };
                    if fade.len > 0 {
                        frame.fill_rectangle(
                            Point::new(at.x - 0.5, at.y),
                            Size::new(1.0, shape.wave_height()),
                            theme::mix(colour, p.text, 0.35),
                        );
                    }
                    let flag = Path::new(|b| {
                        b.move_to(at);
                        b.line_to(Point::new(at.x + FADE_FLAG_SIZE * inward, at.y));
                        b.line_to(Point::new(at.x, at.y + FADE_FLAG_SIZE));
                        b.close();
                    });
                    frame.fill(&flag, p.text);
                }
                Grip::ShapeIn | Grip::ShapeOut => {
                    let ring = Path::circle(at, HANDLE_RADIUS);
                    frame.fill(&ring, selected_body);
                    frame.stroke(&ring, Stroke::default().with_color(p.text).with_width(1.5));
                }
                Grip::Gain => {
                    let dot = Path::circle(at, HANDLE_RADIUS);
                    frame.fill(&dot, p.text);
                    frame.stroke(&dot, Stroke::default().with_color(p.background).with_width(1.5));
                }
            }
        }
    }

    fn draw_scrollbar(&self, frame: &mut Frame, size: Size) {
        let p = self.palette;
        frame.fill_rectangle(Point::ORIGIN, size, theme::mix(p.panel, p.background, 0.5));
        let span = self.scrollbar_span();
        let per_second = size.width as f64 / span;
        let rows = self.project.tracks.len().max(1) as f32;
        let row_step = ((size.height - 8.0) / rows).min(3.0);
        for (i, track) in self.project.tracks.iter().enumerate() {
            let y = 4.0 + i as f32 * row_step;
            for clip in &track.clips {
                let from = (clip.start as f64 / self.rate() * per_second) as f32;
                let to = (clip.end() as f64 / self.rate() * per_second) as f32;
                frame.fill_rectangle(
                    Point::new(from, y),
                    Size::new((to - from).max(1.0), (row_step - 1.0).max(1.0)),
                    theme::mix(p.background, self.colour_of(i), 0.7),
                );
            }
        }
        let (left, width) = self.thumb(span);
        let thumb = Path::new(|b| {
            b.rounded_rectangle(Point::new(left - HEADER_W, 2.0), Size::new(width, size.height - 5.0), 4.0.into());
        });
        frame.fill(&thumb, theme::alpha(p.text, 0.13));
        frame.stroke(&thumb, Stroke::default().with_color(theme::alpha(p.text, 0.3)).with_width(1.0));
        frame.fill_rectangle(Point::new(0.0, size.height - 1.0), Size::new(size.width, 1.0), p.line);
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
            let top = self.track_top(i) - LANES_TOP;
            let height = self.height_of(track);
            if top > size.height || top + height < 0.0 {
                continue;
            }
            frame.fill_rectangle(Point::new(0.0, top), Size::new(3.0, height - 1.0), self.colour_of(i));
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
                position: Point::new(remove.center_x(), remove.center_y() - LANES_TOP),
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
                b.rounded_rectangle(Point::new(mute.x, mute.y - LANES_TOP), mute.size(), 5.0.into());
            });
            frame.fill(&shape, if track.muted { p.danger } else { p.raised });
            frame.fill_text(Text {
                content: "M".into(),
                position: Point::new(mute.center_x(), mute.center_y() - LANES_TOP),
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
            position: Point::new(16.0, add.center_y() - LANES_TOP),
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
