use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use iced::widget::canvas::{self, Cache, Frame, Geometry, Path, Stroke, Text};
use iced::{alignment, keyboard, mouse, Color, Point, Rectangle, Renderer, Size, Theme};
use loupe_engine::{Clip, ClipId, Edge, Fade, Frames, Project, Track, TrackId};

use crate::pointer::EndlessDrag;
use crate::theme::{self, ArmShape, Palette, PlayheadCap, Side};
use crate::{icons, Message};

const LANE_HEIGHT: f32 = 54.0;
const INDENT_W: f32 = 14.0;
const MIN_THUMB_PX: f32 = 28.0;
const SONG_SPAN_HEADROOM: f64 = 1.2;
const SHORTEST_SPAN_SECONDS: f64 = 30.0;
const ADD_ROW_H: f32 = 40.0;
const MIN_GRID_PX: f64 = 14.0;
const DRAG_THRESHOLD: f32 = 4.0;
const RESIZE_GRIP: f32 = 5.0;
const ROOMY_HEADER_H: f32 = 72.0;
const ARM_BUTTON: f32 = 22.0;
const ARM_GAP: f32 = 8.0;
const NAME_GAP: f32 = 10.0;
const CHOSEN_EDGE: f32 = 4.0;
const REORDER_STARTS_AT: f32 = 6.0;
const LANDING_LINE: f32 = 3.0;
const LANDING_DOT: f32 = 4.0;
const NAME_NARROWEST: f32 = 26.0;
const NAME_LETTER: f32 = 7.0;
const NAME_LINE: f32 = 16.0;
const PAN_KNOB: f32 = 24.0;
const PAN_PER_PX: f32 = 0.01;
const ARM_DOT_RADIUS: f32 = 5.0;
const PLAYHEAD_CAP: f32 = 11.0;
const PLAYHEAD_CAP_DROP: f32 = 9.0;
const METER_FLOOR_DB: f32 = -60.0;
const METER_MID_DB: f32 = -12.0;
const METER_HIGH_DB: f32 = -6.0;
const HEADER_TINT: f32 = 0.42;
const MUTED_HEADER_TINT: f32 = 0.16;
const TOOL_BUTTON: f32 = 26.0;
const TOOL_GAP: f32 = 6.0;
const TOOLS_LEFT: f32 = 12.0;
const EDGE_GRIP: f32 = 6.0;
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const MIN_CLIP_PX_FOR_EDGES: f32 = 20.0;
const SHORTEST_CLIP: Frames = 16;
const HANDLE_REACH: f32 = 10.0;
const GAIN_HANDLE_INSET: f32 = 7.0;
const MIN_CLIP_PX_FOR_HANDLES: f32 = 36.0;
const MIN_FADE_PX_FOR_SHAPE_HANDLE: f32 = 24.0;
const CURVE_PER_PX: f32 = 1.0 / 50.0;
const GAIN_DB_PER_PX: f32 = 0.1;
pub const MIN_GAIN_DB: f32 = -24.0;
const MIN_TAKE_LANE: f32 = 10.0;
pub const MAX_GAIN_DB: f32 = 12.0;
pub const MIN_ZOOM: f64 = 2.0;

pub type LoopRange = Option<(Frames, Frames)>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    #[default]
    Pencil,
    Razor,
    Mute,
    Delete,
    Comp,
}

impl Tool {
    const ALL: [Tool; 5] = [Tool::Pencil, Tool::Razor, Tool::Mute, Tool::Delete, Tool::Comp];

    fn icon(self) -> &'static str {
        match self {
            Tool::Pencil => "pencil",
            Tool::Razor => "slice",
            Tool::Mute => "volume-x",
            Tool::Delete => "eraser",
            Tool::Comp => "layers-2",
        }
    }
    fn words(self) -> &'static str {
        match self {
            Tool::Pencil => "Pencil: draw and move clips",
            Tool::Razor => "Razor: click a clip to split it",
            Tool::Mute => "Mute: click a clip to silence it",
            Tool::Delete => "Eraser: click a clip to remove it",
            Tool::Comp => "Comp: pick between stacked takes",
        }
    }
}

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
    pub selection: &'a HashSet<ClipId>,
    pub playhead: Frames,
    pub loop_range: LoopRange,
    pub punch: bool,
    pub tool: Tool,
    pub snap: bool,
    pub hinting: Option<&'static str>,
    pub armed: &'a HashSet<TrackId>,
    pub recording_from: Option<Frames>,
    pub taking_shape: &'a [f32],
    pub chosen_tracks: &'a HashSet<TrackId>,
    pub input_levels: &'a [f32],
    pub opening: bool,
    pub width: f32,
    pub knobs: &'a HashMap<(u64, usize, usize, bool), String>,
    pub cache: &'a Cache,
}

#[derive(Default)]
pub struct Interaction {
    drag: Option<Drag>,
    last_press: Option<(ClipId, Instant)>,
    last_pan_press: Option<(TrackId, Instant)>,
    last_name_press: Option<(TrackId, Instant)>,
    modifiers: keyboard::Modifiers,
}

enum Drag {
    Range { anchor: Frames, origin: Point, moving: bool },
    Clip { id: ClipId, grab: f64, origin: Point, moving: bool, lane: Option<usize> },
    Comp { clip: ClipId, track: TrackId, take: usize, from: Frames, to: Frames, band: (f32, f32) },
    Resize { track: TrackId, top: f32 },
    Reorder { track: TrackId, from: Point, at: Option<usize> },
    Scroll { grab_x: f32, span: f64 },
    Slice { from: Point, to: Point },
    Marquee { from: Point, to: Point },
    Trim { clip: ClipId, edge: Edge },
    Stretch { clip: ClipId, edge: Edge },
    Paint { muted: Option<bool>, touched: Vec<ClipId> },
    Point { target: loupe_engine::Target, which: usize },
    Grip { clip: ClipId, grip: Grip, pull: EndlessDrag, curve_at_grab: f32, db_at_grab: f32 },
    Pan { track: TrackId, pull: EndlessDrag, pan_at_grab: f32 },
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
    title: f32,
}

impl ClipBox {
    fn wave_top(&self) -> f32 {
        self.top + self.title + 2.0
    }

    fn wave_height(&self) -> f32 {
        self.height - self.title - 5.0
    }

    fn take_band(&self, clip: &Clip, take: usize) -> Option<(f32, f32)> {
        let lane = take_lane_height(clip, self.wave_height())?;
        Some((self.wave_top() + lane * take as f32, lane))
    }
}

enum Hit<'a> {
    Tool(Tool),
    Scrollbar,
    Ruler,
    Resize(&'a Track),
    Mute(&'a Track),
    Solo(&'a Track),
    Pan(&'a Track),
    Arm(&'a Track),
    Route(&'a Track),
    Fx(&'a Track),
    Remove(&'a Track),
    Collapse(&'a Track),
    AddTrack,
    Grip(&'a Clip, Grip),
    Edge(&'a Clip, Edge),
    Clip(&'a Clip),
    Envelope(loupe_engine::Target, Option<usize>, Frames, f32),
    Lane,
    Nothing,
}

impl Timeline<'_> {
    fn rate(&self) -> f64 {
        self.project.rate.max(1) as f64
    }

    fn lanes_top(&self) -> f32 {
        self.palette.scrollbar_height + self.palette.ruler_height
    }

    fn header_left(&self) -> f32 {
        match self.palette.headers {
            Side::Left => 0.0,
            Side::Right => (self.width - self.palette.header_width).max(0.0),
        }
    }

    fn header_right(&self) -> f32 {
        self.header_left() + self.palette.header_width
    }

    fn lanes_left(&self) -> f32 {
        match self.palette.headers {
            Side::Left => self.palette.header_width,
            Side::Right => 0.0,
        }
    }

    fn lanes_right(&self) -> f32 {
        self.lanes_left() + self.lanes_width()
    }

    fn in_header(&self, x: f32) -> bool {
        x >= self.header_left() && x < self.header_right()
    }

    fn in_lanes(&self, x: f32) -> bool {
        x >= self.lanes_left() && x <= self.lanes_right()
    }

    fn tool_button(&self, index: usize) -> Rectangle {
        Rectangle::new(
            Point::new(
                self.header_left() + TOOLS_LEFT + index as f32 * (TOOL_BUTTON + TOOL_GAP),
                (self.lanes_top() - TOOL_BUTTON) / 2.0,
            ),
            Size::new(TOOL_BUTTON, TOOL_BUTTON),
        )
    }


    fn draw_taking_shape(&self, overlay: &mut Frame, left: f32, right: f32, top: f32, bottom: f32, from: Frames) {
        if self.taking_shape.is_empty() {
            return;
        }
        let p = self.palette;
        let step = loupe_engine::Input::shape_step().as_secs_f64();
        let middle = (top + bottom) / 2.0;
        let reach = (bottom - top) / 2.0 - 2.0;
        if reach <= 0.0 {
            return;
        }
        let began = from as f64 / self.rate();
        for (index, loudest) in self.taking_shape.iter().enumerate() {
            let at = self.x_of(((began + index as f64 * step) * self.rate()).max(0.0));
            let next = self.x_of(((began + (index + 1) as f64 * step) * self.rate()).max(0.0));
            if next < left || at > right {
                continue;
            }
            let wide = (next - at).max(1.0);
            let tall = (loudest * reach).max(1.0);
            overlay.fill_rectangle(Point::new(at.max(left), middle - tall), Size::new(wide, tall * 2.0), theme::alpha(p.text, 0.65));
        }
    }

    fn x_of(&self, frames: f64) -> f32 {
        self.lanes_left() + ((frames / self.rate() - self.view.scroll) * self.view.zoom) as f32
    }

    fn frames_at(&self, x: f32) -> f64 {
        (((x - self.lanes_left()) as f64 / self.view.zoom + self.view.scroll) * self.rate()).max(0.0)
    }

    fn height_of(&self, track: &Track) -> f32 {
        if self.hidden(track) {
            return 0.0;
        }
        self.clips_height(track) + self.lanes_of(track).len() as f32 * LANE_HEIGHT
    }

    fn clips_height(&self, track: &Track) -> f32 {
        if self.hidden(track) {
            return 0.0;
        }
        self.heights.get(&track.id).copied().unwrap_or(self.palette.track_height)
    }

    fn lanes_of(&self, track: &Track) -> Vec<&loupe_engine::Envelope> {
        if self.hidden(track) {
            return Vec::new();
        }
        self.project
            .envelopes
            .iter()
            .filter(|shape| shape.lane_open && self.belongs(shape.target, track))
            .collect()
    }

    fn belongs(&self, target: loupe_engine::Target, track: &Track) -> bool {
        if target.on_track() == Some(track.id) {
            return true;
        }
        match target.on_clip() {
            Some(clip) => track.clips.iter().any(|kept| kept.id == clip),
            None => false,
        }
    }

    fn hidden(&self, track: &Track) -> bool {
        if track.hidden {
            return true;
        }
        let mut at = track.parent;
        let mut steps = 0;
        while let Some(id) = at {
            let Some(parent) = self.project.tracks.iter().find(|t| t.id == id) else {
                return false;
            };
            if parent.collapsed || parent.hidden {
                return true;
            }
            steps += 1;
            if steps > self.project.tracks.len() {
                return false;
            }
            at = parent.parent;
        }
        false
    }

    fn track_top(&self, index: usize) -> f32 {
        let above: f32 = self.project.tracks[..index].iter().map(|t| self.height_of(t)).sum();
        self.lanes_top() + above - self.view.scroll_y
    }

    fn track_at(&self, y: f32) -> Option<usize> {
        if y < self.lanes_top() {
            return None;
        }
        let mut top = self.lanes_top() - self.view.scroll_y;
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
            None if y < self.lanes_top() - self.view.scroll_y => 0,
            None => self.project.tracks.len().saturating_sub(1),
        }
    }

    fn clips_between(&self, from: Point, to: Point) -> Vec<ClipId> {
        let (left, right) = (from.x.min(to.x), from.x.max(to.x));
        let (top, bottom) = (from.y.min(to.y), from.y.max(to.y));
        let mut inside = Vec::new();
        for (i, track) in self.project.tracks.iter().enumerate() {
            let lane_top = self.track_top(i) + self.palette.clip_padding;
            let lane_bottom = self.track_top(i) + self.height_of(track) - self.palette.clip_padding;
            if lane_bottom < top || lane_top > bottom {
                continue;
            }
            for clip in &track.clips {
                let touches = self.x_of(clip.start as f64) <= right && self.x_of(clip.end() as f64) >= left;
                if touches {
                    inside.push(clip.id);
                }
            }
        }
        inside
    }

    fn slice_line(&self, from: Point, to: Point, free: bool) -> (Point, Point) {
        let on_grid = |at: Point| Point::new(self.x_of(self.snap(self.frames_at(at.x), free) as f64), at.y);
        (on_grid(from), on_grid(to))
    }

    fn slice_cuts(&self, from: Point, to: Point, free: bool) -> Vec<(usize, Frames)> {
        let aims_at_nothing = from == to && self.track_at(from.y).is_none();
        if self.project.tracks.is_empty() || aims_at_nothing {
            return Vec::new();
        }
        let (from, to) = self.slice_line(from, to, free);
        let first = self.row_for_drag(from.y.min(to.y));
        let last = self.row_for_drag(from.y.max(to.y));
        (first..=last)
            .map(|row| {
                let middle = self.track_top(row) + self.height_of(&self.project.tracks[row]) / 2.0;
                let along = if from.y == to.y { 0.0 } else { ((middle - from.y) / (to.y - from.y)).clamp(0.0, 1.0) };
                let x = from.x + (to.x - from.x) * along;
                (row, self.frames_at(x).round() as Frames)
            })
            .collect()
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

    fn hidden_buttons(&self, index: usize) -> usize {
        let track = &self.project.tracks[index];
        if self.height_of(track) >= ROOMY_HEADER_H {
            return 0;
        }
        let has_children = self.project.tracks.iter().any(|t| t.parent == Some(track.id));
        let indent = self.project.depth_of(track.id).min(4) as f32 * INDENT_W;
        let name_left = 16.0 + indent + if has_children { 14.0 } else { 0.0 };
        let mute_x = self.header_right() - 68.0;
        for hidden in 0..=3usize {
            let leftmost = mute_x - (4 - hidden) as f32 * (ARM_GAP + ARM_BUTTON);
            if leftmost - self.header_left() - name_left - NAME_GAP >= NAME_NARROWEST {
                return hidden;
            }
        }
        3
    }

    fn shows_fx(&self, index: usize) -> bool {
        self.hidden_buttons(index) < 1
    }

    fn shows_route(&self, index: usize) -> bool {
        self.hidden_buttons(index) < 2
    }

    fn shows_arm(&self, index: usize) -> bool {
        self.hidden_buttons(index) < 3
    }

    fn mute_button(&self, index: usize) -> Rectangle {
        let top = self.track_top(index);
        let height = self.height_of(&self.project.tracks[index]);
        let corner = if height >= ROOMY_HEADER_H {
            Point::new(self.header_left() + 16.0, top + height - 36.0)
        } else {
            Point::new(self.header_right() - 68.0, top + 9.0)
        };
        Rectangle::new(corner, Size::new(ARM_BUTTON, 22.0))
    }

    fn route_button(&self, index: usize) -> Rectangle {
        let arm = self.arm_button(index);
        let roomy = self.height_of(&self.project.tracks[index]) >= ROOMY_HEADER_H;
        let left = if roomy { arm.x + arm.width + ARM_GAP } else { arm.x - ARM_GAP - ARM_BUTTON };
        Rectangle::new(Point::new(left, arm.y), Size::new(ARM_BUTTON, arm.height))
    }

    fn fx_button(&self, index: usize) -> Rectangle {
        let route = self.route_button(index);
        let roomy = self.height_of(&self.project.tracks[index]) >= ROOMY_HEADER_H;
        let left = if roomy { route.x + route.width + ARM_GAP } else { route.x - ARM_GAP - ARM_BUTTON };
        Rectangle::new(Point::new(left, route.y), Size::new(ARM_BUTTON, route.height))
    }

    fn solo_button(&self, index: usize) -> Rectangle {
        let mute = self.mute_button(index);
        let roomy = self.height_of(&self.project.tracks[index]) >= ROOMY_HEADER_H;
        let left = if roomy { mute.x + mute.width + ARM_GAP } else { mute.x - ARM_GAP - mute.width };
        Rectangle::new(Point::new(left, mute.y), mute.size())
    }

    fn pan_knob(&self, index: usize) -> Option<Rectangle> {
        let roomy = self.height_of(&self.project.tracks[index]) >= ROOMY_HEADER_H;
        let route = self.route_button(index);
        let after_fx = self.fx_button(index);
        let left = after_fx.x + after_fx.width + ARM_GAP;
        (roomy && left + PAN_KNOB <= self.header_right() - 6.0).then(|| Rectangle::new(Point::new(left, route.center_y() - PAN_KNOB / 2.0), Size::new(PAN_KNOB, PAN_KNOB)))
    }

    fn arm_button(&self, index: usize) -> Rectangle {
        let mute = self.solo_button(index);
        let roomy = self.height_of(&self.project.tracks[index]) >= ROOMY_HEADER_H;
        let left = if roomy { mute.x + mute.width + ARM_GAP } else { mute.x - ARM_GAP - ARM_BUTTON };
        Rectangle::new(Point::new(left, mute.y), Size::new(ARM_BUTTON, mute.height))
    }

    fn meter_bar(&self, index: usize) -> Rectangle {
        let top = self.track_top(index);
        let height = self.height_of(&self.project.tracks[index]);
        let inset = if height >= ROOMY_HEADER_H { 9.0 } else { 5.0 };
        Rectangle::new(
            Point::new(self.header_left() + 16.0, top + height - inset),
            Size::new(self.palette.header_width - 32.0, self.palette.meter_thickness),
        )
    }

    fn collapse_button(&self, index: usize) -> Rectangle {
        let depth = self.project.depth_of(self.project.tracks[index].id).min(4) as f32;
        Rectangle::new(Point::new(depth * INDENT_W, self.track_top(index) + 8.0), Size::new(22.0, 24.0))
    }

    fn remove_button(&self, index: usize) -> Rectangle {
        Rectangle::new(Point::new(self.header_right() - 34.0, self.track_top(index) + 8.0), Size::new(24.0, 24.0))
    }

    fn add_button(&self) -> Rectangle {
        Rectangle::new(
            Point::new(self.header_left(), self.track_top(self.project.tracks.len())),
            Size::new(self.palette.header_width, ADD_ROW_H),
        )
    }

    fn content_height(&self) -> f32 {
        let tracks: f32 = self.project.tracks.iter().map(|t| self.height_of(t)).sum();
        self.lanes_top() + tracks + ADD_ROW_H
    }

    fn colour_of(&self, index: usize) -> Color {
        match self.project.tracks[index].colour {
            Some([r, g, b]) => Color::from_rgb8(r, g, b),
            None => self.palette.track(self.project.tracks[index].id.0.saturating_sub(1) as usize),
        }
    }

    fn folder_at(&self, to: usize, moving: TrackId) -> Option<TrackId> {
        let above = self.project.tracks[..to.min(self.project.tracks.len())].iter().rev().find(|track| track.id != moving)?;
        let parent = match self.project.tracks.iter().any(|track| track.parent == Some(above.id)) {
            true => Some(above.id),
            false => above.parent,
        };
        parent.filter(|id| *id != moving && !self.project.descends_from(*id, moving))
    }

    fn landing_at(&self, y: f32) -> usize {
        let mut nearest = 0;
        let mut gap = f32::MAX;
        for index in 0..=self.project.tracks.len() {
            let edge = self.track_top(index);
            let how_far = (edge - y).abs();
            if how_far < gap {
                gap = how_far;
                nearest = index;
            }
        }
        nearest
    }

    fn name_spot(&self, index: usize) -> (f32, f32, f32) {
        let track = &self.project.tracks[index];
        let has_children = self.project.tracks.iter().any(|t| t.parent == Some(track.id));
        let indent = self.project.depth_of(track.id).min(4) as f32 * INDENT_W;
        let name_left = 16.0 + indent + if has_children { 14.0 } else { 0.0 };
        let top = self.track_top(index);
        let whole_width = self.header_right() - self.header_left() - name_left - NAME_GAP;
        if self.height_of(track) >= ROOMY_HEADER_H {
            return (name_left, whole_width, top + 20.0);
        }
        let leftmost = match self.hidden_buttons(index) {
            0 => self.fx_button(index).x,
            1 => self.route_button(index).x,
            2 => self.arm_button(index).x,
            _ => self.solo_button(index).x,
        };
        let beside = leftmost - self.header_left() - name_left - NAME_GAP;
        if beside >= NAME_NARROWEST {
            return (name_left, beside, top + 20.0);
        }
        let buttons = self.mute_button(index);
        (name_left, whole_width, buttons.y + buttons.height + NAME_GAP + NAME_LINE / 2.0)
    }

    fn name_box(&self, index: usize) -> Option<Rectangle> {
        let (name_left, room, middle) = self.name_spot(index);
        (self.height_of(&self.project.tracks[index]) > 0.0 && room >= NAME_NARROWEST).then(|| Rectangle {
            x: self.header_left() + name_left,
            y: middle - NAME_LINE / 2.0,
            width: room,
            height: 20.0,
        })
    }

    fn track_header_at(&self, p: Point) -> Option<&Track> {
        if !self.in_header(p.x) {
            return None;
        }
        self.track_at(p.y).map(|i| &self.project.tracks[i])
    }

    fn lanes_width(&self) -> f32 {
        (self.width - self.palette.header_width).max(1.0)
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
        let left = self.lanes_left() + (self.view.scroll / span) as f32 * track;
        (left.min(self.lanes_right() - width), width)
    }

    fn scrolled_to_thumb_left(&self, left: f32, span: f64) -> View {
        let scroll = ((left - self.lanes_left()) / self.lanes_width()) as f64 * span;
        View { scroll: self.within_span(scroll, self.view.zoom), ..self.view }
    }

    fn clip_box(&self, clip: &Clip) -> Option<ClipBox> {
        let index = self.project.tracks.iter().position(|t| t.clips.iter().any(|c| c.id == clip.id))?;
        Some(ClipBox {
            left: self.x_of(clip.start as f64),
            right: self.x_of(clip.end() as f64),
            top: self.track_top(index) + self.palette.clip_padding,
            height: self.height_of(&self.project.tracks[index]) - self.palette.clip_padding * 2.0,
            title: self.palette.clip_title_height,
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
        let seen_left = shape.left.max(self.lanes_left());
        let seen_right = shape.right.min(self.lanes_right());
        grips.push((Grip::Gain, Point::new((seen_left + seen_right) / 2.0, shape.top + shape.height - GAIN_HANDLE_INSET)));
        grips
    }

    fn take_at(&self, clip: &Clip, p: Point) -> Option<usize> {
        let shape = self.clip_box(clip)?;
        let lane = take_lane_height(clip, shape.wave_height())?;
        let row = ((p.y - shape.wave_top()) / lane).floor();
        (row >= 0.0 && (row as usize) < clip.takes.len()).then_some(row as usize)
    }

    fn resize_grip_at(&self, p: Point) -> Option<&Track> {
        if !self.in_header(p.x) {
            return None;
        }
        self.project.tracks.iter().enumerate().find_map(|(i, track)| {
            let bottom = self.track_top(i) + self.height_of(track);
            ((p.y - bottom).abs() <= RESIZE_GRIP).then_some(track)
        })
    }

    fn hit(&self, p: Point) -> Hit<'_> {
        if p.y < self.lanes_top() {
            return match (!self.in_header(p.x), p.y < self.palette.scrollbar_height) {
                (true, true) => Hit::Scrollbar,
                (true, false) => Hit::Ruler,

                (false, _) => Tool::ALL
                    .into_iter()
                    .enumerate()
                    .find(|(i, _)| self.tool_button(*i).contains(p))
                    .map_or(Hit::Nothing, |(_, tool)| Hit::Tool(tool)),
            };
        }
        if let Some(track) = self.resize_grip_at(p) {
            return Hit::Resize(track);
        }
        if self.in_header(p.x) {
            if let Some(i) = self.track_at(p.y) {
                let track = &self.project.tracks[i];
                if self.mute_button(i).contains(p) {
                    return Hit::Mute(track);
                }
                if self.solo_button(i).contains(p) {
                    return Hit::Solo(track);
                }
                if self.pan_knob(i).is_some_and(|knob| knob.contains(p)) {
                    return Hit::Pan(track);
                }
                if self.shows_arm(i) && self.arm_button(i).contains(p) {
                    return Hit::Arm(track);
                }
                if self.shows_route(i) && self.route_button(i).contains(p) {
                    return Hit::Route(track);
                }
                if self.shows_fx(i) && self.fx_button(i).contains(p) {
                    return Hit::Fx(track);
                }
                if self.remove_button(i).contains(p) {
                    return Hit::Remove(track);
                }
                if self.collapse_button(i).contains(p) && self.project.tracks.iter().any(|t| t.parent == Some(track.id)) {
                    return Hit::Collapse(track);
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
        if let Some(found) = self.envelope_at(i, p) {
            return found;
        }
        if p.y < top + self.palette.clip_padding || p.y > top + self.height_of(track) - self.palette.clip_padding {
            return Hit::Lane;
        }
        let at = self.frames_at(p.x);
        let Some(clip) = track.clips.iter().rev().find(|c| at >= c.start as f64 && at < c.end() as f64) else {
            return Hit::Lane;
        };
        let left = self.x_of(clip.start as f64);
        let right = self.x_of(clip.end() as f64);
        if self.tool != Tool::Pencil || right - left < MIN_CLIP_PX_FOR_EDGES {
            return Hit::Clip(clip);
        }
        if p.x - left <= EDGE_GRIP {
            Hit::Edge(clip, Edge::In)
        } else if right - p.x <= EDGE_GRIP {
            Hit::Edge(clip, Edge::Out)
        } else {
            Hit::Clip(clip)
        }
    }

    fn zoomed(&self, factor: f64, anchor_x: f32) -> View {
        let anchor = (anchor_x - self.lanes_left()).max(0.0) as f64;
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
        let free = !self.snap || state.modifiers.shift();

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
                    Hit::Pan(track) => Some(Message::OpenAutomation(loupe_engine::Target::TrackPan(track.id))),
                    Hit::Envelope(target, Some(which), ..) => Some(Message::DropPoint { target, which }),
                    Hit::Envelope(..) => None,
                    Hit::Clip(clip) | Hit::Grip(clip, _) | Hit::Edge(clip, _) => Some(Message::DeleteClip(clip.id)),
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
                if self.tool == Tool::Pencil && state.modifiers.command() {
                    match self.hit(p) {
                        Hit::Clip(clip) | Hit::Edge(clip, _) | Hit::Grip(clip, _) => {
                            return (Captured, Some(Message::ToggleSelect(clip.id)));
                        }
                        Hit::Lane => {
                            state.drag = Some(Drag::Marquee { from: p, to: p });
                            return (Captured, Some(Message::SelectMany(Vec::new())));
                        }
                        _ => {}
                    }
                }
                if let (Tool::Pencil, Hit::Clip(clip) | Hit::Edge(clip, _)) = (self.tool, self.hit(p)) {
                    let pressed_twice = state.last_press.is_some_and(|(last, at)| last == clip.id && at.elapsed() < DOUBLE_CLICK);
                    state.last_press = (!pressed_twice).then(|| (clip.id, Instant::now()));
                    if pressed_twice {
                        return (Captured, Some(Message::OpenClip(clip.id)));
                    }
                }
                if let (Hit::Nothing, Some(i)) = (self.hit(p), self.track_at(p.y)) {
                    if self.name_box(i).is_some_and(|name| name.contains(p)) {
                        let track = self.project.tracks[i].id;
                        let pressed_twice = state.last_name_press.is_some_and(|(last, at)| last == track && at.elapsed() < DOUBLE_CLICK);
                        state.last_name_press = (!pressed_twice).then(|| (track, Instant::now()));
                        if pressed_twice {
                            return (Captured, Some(Message::RenameTrackAt { track, at: Point::new(bounds.x + p.x, bounds.y + p.y) }));
                        }
                    }
                }
                let message = match (self.tool, self.hit(p)) {
                    (_, Hit::Tool(tool)) => Some(Message::SetTool(tool)),
                    (_, Hit::Envelope(target, near, at, value)) => {
                        let which = match near {
                            Some(which) => which,
                            None => {
                                state.drag = Some(Drag::Point { target, which: usize::MAX });
                                return (Captured, Some(Message::PutPoint { target, at, value }));
                            }
                        };
                        state.drag = Some(Drag::Point { target, which });
                        Some(Message::Refresh)
                    }
                    (Tool::Razor, Hit::Clip(_) | Hit::Grip(..) | Hit::Lane) => {
                        state.drag = Some(Drag::Slice { from: p, to: p });
                        Some(Message::Refresh)
                    }
                    (Tool::Comp, Hit::Clip(clip) | Hit::Grip(clip, _)) => {
                        let (Some(take), Some(shape), Some(track)) = (self.take_at(clip, p), self.clip_box(clip), self.project.track_of(clip.id)) else {
                            return (Captured, None);
                        };
                        let Some(band) = shape.take_band(clip, take) else {
                            return (Captured, None);
                        };
                        let at = self.snap(self.frames_at(p.x), free);
                        state.drag = Some(Drag::Comp { clip: clip.id, track: track.id, take, from: at, to: at, band });
                        Some(Message::Refresh)
                    }
                    (Tool::Mute, Hit::Clip(clip) | Hit::Grip(clip, _)) => {
                        let muted = !clip.muted;
                        state.drag = Some(Drag::Paint { muted: Some(muted), touched: vec![clip.id] });
                        Some(Message::PaintMute { clip: clip.id, muted })
                    }
                    (Tool::Delete, Hit::Clip(clip) | Hit::Grip(clip, _)) => {
                        state.drag = Some(Drag::Paint { muted: None, touched: Vec::new() });
                        Some(Message::PaintDelete(clip.id))
                    }
                    (Tool::Mute | Tool::Delete, Hit::Lane) => {
                        state.drag = Some(Drag::Paint { muted: None, touched: Vec::new() });
                        None
                    }
                    (_, Hit::Scrollbar) => {
                        let span = self.scrollbar_span();
                        let (left, width) = self.thumb(span);
                        let on_thumb = p.x >= left && p.x <= left + width;
                        let grab_x = if on_thumb { p.x - left } else { width / 2.0 };
                        state.drag = Some(Drag::Scroll { grab_x, span });
                        let view = self.scrolled_to_thumb_left(p.x - grab_x, span);
                        (view != self.view).then_some(Message::SetView(view))
                    }
                    (_, Hit::Ruler) => {
                        let anchor = self.snap(self.frames_at(p.x), free);
                        state.drag = Some(Drag::Range { anchor, origin: p, moving: false });
                        None
                    }
                    (_, Hit::Resize(track)) => {
                        let index = self.project.tracks.iter().position(|t| t.id == track.id);
                        let top = index.map_or(self.lanes_top(), |i| self.track_top(i));
                        state.drag = Some(Drag::Resize { track: track.id, top });
                        None
                    }
                    (_, Hit::Nothing | Hit::Lane) if self.in_header(p.x) => match self.track_header_at(p) {
                        Some(track) => {
                            state.drag = Some(Drag::Reorder { track: track.id, from: p, at: None });
                            Some(Message::ChooseTrack { track: track.id, as_well: state.modifiers.command() })
                        }
                        None => None,
                    },
                    (_, Hit::Mute(track)) => Some(Message::ToggleMute(track.id)),
                    (_, Hit::Solo(track)) => Some(Message::ToggleSolo(track.id)),
                    (_, Hit::Fx(track)) => Some(Message::OpenChain(crate::stockwin::Spot::Track(track.id))),
                    (_, Hit::Pan(track)) => {
                        let again = state.last_pan_press.is_some_and(|(id, at)| id == track.id && at.elapsed() < DOUBLE_CLICK);
                        state.last_pan_press = Some((track.id, Instant::now()));
                        if again {
                            Some(Message::TrackPan(track.id, 0.0))
                        } else {
                            state.drag = Some(Drag::Pan { track: track.id, pull: EndlessDrag::start(p, true), pan_at_grab: track.pan });
                            None
                        }
                    }
                    (_, Hit::Arm(track)) => Some(Message::ToggleArm(track.id)),
                    (_, Hit::Route(track)) => Some(Message::OpenRouting(track.id)),
                    (_, Hit::Remove(track)) => Some(Message::RemoveTrack(track.id)),
                    (_, Hit::Collapse(track)) => Some(Message::ToggleCollapsed(track.id)),
                    (_, Hit::AddTrack) => Some(Message::AddTrack),
                    (_, Hit::Grip(clip, grip)) => {
                        let curve_at_grab = match grip {
                            Grip::ShapeOut => clip.fade_out.curve,
                            _ => clip.fade_in.curve,
                        };
                        let db_at_grab = (20.0 * clip.gain.max(1e-6).log10()).clamp(MIN_GAIN_DB, MAX_GAIN_DB);
                        let pull = EndlessDrag::start(p, !grip.moves_sideways());
                        state.drag = Some(Drag::Grip { clip: clip.id, grip, pull, curve_at_grab, db_at_grab });
                        None
                    }
                    (_, Hit::Edge(clip, edge)) => {
                        state.drag = Some(match state.modifiers.alt() && clip.notes.is_none() {
                            true => Drag::Stretch { clip: clip.id, edge },
                            false => Drag::Trim { clip: clip.id, edge },
                        });
                        Some(Message::Select(Some(clip.id)))
                    }
                    (_, Hit::Clip(clip)) => {
                        state.drag = Some(Drag::Clip {
                            id: clip.id,
                            grab: self.frames_at(p.x) - clip.start as f64,
                            origin: p,
                            moving: false,
                            lane: self.take_at(clip, p).filter(|take| *take != clip.take),
                        });
                        (!self.selection.contains(&clip.id)).then_some(Message::Select(Some(clip.id)))
                    }
                    (_, Hit::Lane) => Some(Message::LaneClicked(self.snap(self.frames_at(p.x), free))),
                    (_, Hit::Nothing) => None,
                };
                (Captured, message)
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let (Some(p), Some(drag)) = (anywhere, state.drag.as_mut()) else {
                    if let Some(p) = anywhere {
                        let over = Tool::ALL.into_iter().enumerate().find(|(i, _)| self.tool_button(*i).contains(p)).map(|(_, tool)| tool.words());
                        if over != self.hinting {
                            return (Ignored, Some(Message::Hint(over)));
                        }
                    }
                    let aiming = self.tool == Tool::Razor && cursor.is_over(bounds);
                    return (Ignored, aiming.then_some(Message::Refresh));
                };
                match drag {
                    Drag::Point { target, which } => {
                        let target = *target;
                        let Some(shape) = self.project.envelope(target) else {
                            return (Captured, None);
                        };
                        let offset = self.lane_offset(target);
                        let at = (self.frames_at(p.x).max(0.0) as Frames).saturating_sub(offset);
                        let Some(top) = self.lane_top_of(target) else {
                            return (Captured, None);
                        };
                        let inset = 7.0;
                        let span = (LANE_HEIGHT - inset * 2.0).max(1.0);
                        let reach = (shape.highest - shape.lowest).max(f32::EPSILON);
                        let value = shape.lowest + reach * (1.0 - ((p.y - top - inset) / span).clamp(0.0, 1.0));
                        if *which == usize::MAX {
                            *which = shape.points.iter().position(|point| point.at == at).unwrap_or(usize::MAX);
                        }
                        if *which == usize::MAX {
                            return (Captured, None);
                        }
                        let moving = *which;
                        *which = shape.points.iter().position(|point| point.at >= at).unwrap_or(shape.points.len() - 1);
                        (Captured, Some(Message::DragPoint { target, which: moving, at, value }))
                    }
                    Drag::Range { anchor, origin, moving } => {
                        if !*moving && p.distance(*origin) < DRAG_THRESHOLD {
                            return (Captured, None);
                        }
                        *moving = true;
                        let here = self.snap(self.frames_at(p.x.clamp(self.lanes_left(), self.lanes_right())), free);
                        let range = (here != *anchor).then(|| (here.min(*anchor), here.max(*anchor)));
                        (Captured, (range != self.loop_range).then_some(Message::SetLoop(range)))
                    }
                    Drag::Comp { to, .. } => {
                        *to = self.snap(self.frames_at(p.x), free);
                        (Captured, Some(Message::Refresh))
                    }
                    Drag::Clip { id, grab, origin, moving, .. } => {
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
                    Drag::Pan { track, pull, pan_at_grab } => {
                        let Some(travel_up) = pull.moved(p) else {
                            return (Captured, None);
                        };
                        let pan = (*pan_at_grab + travel_up * PAN_PER_PX).clamp(-1.0, 1.0);
                        let now = self.project.track(*track).map_or(0.0, |t| t.pan);
                        (Captured, (pan != now).then_some(Message::TrackPan(*track, pan)))
                    }
                    Drag::Grip { clip, grip, pull, curve_at_grab, db_at_grab } => {
                        let Some(clip) = self.project.clip(*clip) else {
                            return (Captured, None);
                        };
                        let Some(travel_up) = pull.moved(p) else {
                            return (Captured, None);
                        };
                        let at = self.snap(self.frames_at(p.x), free);
                        let bent = (*curve_at_grab + travel_up * CURVE_PER_PX).clamp(-1.0, 1.0);
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
                                let db = (*db_at_grab + travel_up * GAIN_DB_PER_PX).clamp(MIN_GAIN_DB, MAX_GAIN_DB);
                                Some(Message::ClipGain(db))
                            }
                        };
                        (Captured, message)
                    }
                    Drag::Trim { clip, edge } => {
                        let Some(clip) = self.project.clip(*clip) else {
                            return (Captured, None);
                        };
                        let at = self.snap(self.frames_at(p.x), free);
                        let recorded = clip.audio_len();
                        let (start, offset, len) = match edge {
                            Edge::In => {
                                let earliest = clip.start.saturating_sub(clip.offset);
                                let latest = clip.end().saturating_sub(SHORTEST_CLIP).max(earliest);
                                let start = at.clamp(earliest, latest);
                                (start, clip.offset + start - clip.start, clip.end() - start)
                            }
                            Edge::Out => {
                                let longest = recorded.saturating_sub(clip.offset).max(clip.len);
                                let end = at.clamp(clip.start + SHORTEST_CLIP.min(clip.len), clip.start + longest);
                                (clip.start, clip.offset, end - clip.start)
                            }
                        };
                        let changed = start != clip.start || len != clip.len;
                        (Captured, changed.then_some(Message::TrimClip { clip: clip.id, start, offset, len }))
                    }
                    Drag::Stretch { clip, edge } => {
                        let Some(clip) = self.project.clip(*clip) else {
                            return (Captured, None);
                        };
                        let at = self.snap(self.frames_at(p.x), free);
                        let unstretched = clip.len as f64 / clip.stretch;
                        let shortest = (unstretched * loupe_engine::SHORTEST_STRETCH).ceil() as Frames;
                        let longest = (unstretched * loupe_engine::LONGEST_STRETCH).floor() as Frames;
                        let (start, len) = match edge {
                            Edge::In => {
                                let start = at.clamp(clip.end().saturating_sub(longest), clip.end().saturating_sub(shortest.max(SHORTEST_CLIP)));
                                (start, clip.end() - start)
                            }
                            Edge::Out => (clip.start, at.saturating_sub(clip.start).clamp(shortest.max(SHORTEST_CLIP), longest)),
                        };
                        let changed = start != clip.start || len != clip.len;
                        (Captured, changed.then_some(Message::StretchClip { clip: clip.id, start, len }))
                    }
                    Drag::Marquee { from, to } => {
                        *to = p;
                        (Captured, Some(Message::SelectMany(self.clips_between(*from, p))))
                    }
                    Drag::Slice { to, .. } => {
                        *to = p;
                        (Captured, Some(Message::Refresh))
                    }
                    Drag::Paint { muted, touched } => {
                        let (Hit::Clip(clip) | Hit::Grip(clip, _)) = self.hit(p) else {
                            return (Captured, None);
                        };
                        if touched.contains(&clip.id) {
                            return (Captured, None);
                        }
                        touched.push(clip.id);
                        let message = if self.tool == Tool::Delete {
                            Message::PaintDelete(clip.id)
                        } else {
                            Message::PaintMute { clip: clip.id, muted: *muted.get_or_insert(!clip.muted) }
                        };
                        (Captured, Some(message))
                    }
                    Drag::Scroll { grab_x, span } => {
                        let view = self.scrolled_to_thumb_left(p.x - *grab_x, *span);
                        (Captured, (view != self.view).then_some(Message::SetView(view)))
                    }
                    Drag::Reorder { from, at, .. } => {
                        let moved = (p.y - from.y).abs() > REORDER_STARTS_AT;
                        *at = moved.then(|| self.landing_at(p.y));
                        (Captured, None)
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
                Some(Drag::Reorder { track, at: Some(to), .. }) => {
                    (Captured, Some(Message::MoveTrack { track, to, parent: self.folder_at(to, track) }))
                }
                Some(Drag::Clip { id, moving: false, lane: Some(take), .. }) => (Captured, Some(Message::UseTake(id, take))),
                Some(Drag::Comp { clip, track, take, from, to, .. }) => {
                    let message = match from == to {
                        true => Message::UseTake(clip, take),
                        false => Message::Comp { track, take, from: from.min(to), to: from.max(to) },
                    };
                    (Captured, Some(message))
                }
                Some(Drag::Range { anchor, moving: false, .. }) => (Captured, Some(Message::RulerClicked(anchor))),
                Some(
                    Drag::Clip { moving: true, .. }
                    | Drag::Grip { .. }
                    | Drag::Paint { .. }
                    | Drag::Trim { .. }
                    | Drag::Stretch { .. }
                    | Drag::Pan { .. },
                ) => {
                    (Captured, Some(Message::DragEnd))
                }
                Some(Drag::Marquee { .. }) => (Captured, Some(Message::Refresh)),
                Some(Drag::Slice { from, to }) => {
                    let cuts = self
                        .slice_cuts(from, to, free)
                        .into_iter()
                        .map(|(row, at)| (self.project.tracks[row].id, at))
                        .collect();
                    (Captured, Some(Message::Slice { cuts }))
                }
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
                if state.modifiers.command() && p.x < self.lanes_left() && !self.project.tracks.is_empty() {
                    return (Captured, Some(Message::ScaleTrackHeights(zoom as f32)));
                }
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
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let p = self.palette;
        let content = self.cache.draw(renderer, bounds.size(), |frame| {
            let lanes = Rectangle::new(
                Point::new(self.lanes_left(), self.lanes_top()),
                Size::new(self.lanes_width(), (bounds.height - self.lanes_top()).max(0.0)),
            );
            frame.with_clip(lanes, |frame| self.draw_lanes(frame, lanes.size()));
            let ruler = Rectangle::new(Point::new(self.lanes_left(), self.palette.scrollbar_height), Size::new(lanes.width, self.palette.ruler_height));
            let scrollbar = Rectangle::new(Point::new(self.lanes_left(), 0.0), Size::new(lanes.width, self.palette.scrollbar_height));
            frame.with_clip(scrollbar, |frame| self.draw_scrollbar(frame, scrollbar.size()));
            frame.with_clip(ruler, |frame| self.draw_ruler(frame, ruler.size()));
            let headers =
                Rectangle::new(Point::new(self.header_left(), self.lanes_top()), Size::new(p.header_width, lanes.height));
            frame.with_clip(headers, |frame| self.draw_headers(frame, headers.size()));

            frame.fill_rectangle(Point::new(self.header_left(), 0.0), Size::new(p.header_width, self.lanes_top()), p.panel);
            for (i, tool) in Tool::ALL.into_iter().enumerate() {
                let button = self.tool_button(i);
                if tool == self.tool {
                    let chosen = Path::new(|b| b.rounded_rectangle(button.position(), button.size(), p.corner.into()));
                    frame.fill(&chosen, p.hover);
                }
                frame.fill_text(Text {
                    content: icons::glyph(tool.icon()).to_string(),
                    position: button.center(),
                    color: if tool == self.tool { p.text } else { p.text_dim },
                    size: 14.0.into(),
                    font: icons::font(tool.icon()),
                    horizontal_alignment: alignment::Horizontal::Center,
                    vertical_alignment: alignment::Vertical::Center,
                    shaping: iced::widget::text::Shaping::Advanced,
                    ..Text::default()
                });
            }
            frame.fill_rectangle(Point::new(0.0, self.lanes_top() - 1.0), Size::new(bounds.width, 1.0), p.line);
            let divider = if p.headers == Side::Left { p.header_width - 1.0 } else { self.header_left() };
            frame.fill_rectangle(Point::new(divider, 0.0), Size::new(1.0, bounds.height), p.line);
        });

        let mut overlay = Frame::new(renderer, bounds.size());
        if let Some((from, to)) = self.loop_range {
            let marked = if self.punch { p.danger } else { p.accent };
            let left = self.x_of(from as f64).max(self.lanes_left());
            let right = self.x_of(to as f64).min(self.lanes_right());
            if right > left {
                let width = right - left;
                overlay.fill_rectangle(
                    Point::new(left, self.palette.scrollbar_height),
                    Size::new(width, self.palette.ruler_height - 1.0),
                    theme::mix(p.panel, marked, 0.28),
                );
                overlay.fill_rectangle(
                    Point::new(left, self.lanes_top()),
                    Size::new(width, bounds.height - self.lanes_top()),
                    theme::alpha(marked, 0.05),
                );
                for x in [left, right] {
                    overlay.fill_rectangle(
                        Point::new(x.round() - 0.5, self.lanes_top()),
                        Size::new(1.0, bounds.height - self.lanes_top()),
                        theme::alpha(marked, 0.45),
                    );
                }
            }
        }
        if let Some(Drag::Marquee { from, to }) = &state.drag {
            let corner = Point::new(from.x.min(to.x).max(self.lanes_left()), from.y.min(to.y).max(self.lanes_top()));
            let reach = from.x.max(to.x).min(self.lanes_right());
            let size = Size::new((reach - corner.x).max(0.0), (from.y.max(to.y) - corner.y).max(0.0));
            overlay.fill_rectangle(corner, size, theme::alpha(p.accent, 0.08));
            overlay.stroke(
                &Path::rectangle(corner, size),
                Stroke::default().with_color(theme::alpha(p.accent, 0.6)).with_width(1.0),
            );
        }
        let free = !self.snap || state.modifiers.shift();
        let slicing = match (&state.drag, self.tool, cursor.position_in(bounds)) {
            (Some(Drag::Slice { from, to }), _, _) => Some((*from, *to)),
            (None, Tool::Razor, Some(at)) if self.in_lanes(at.x) && at.y >= self.lanes_top() => Some((at, at)),
            _ => None,
        };
        if let Some((from, to)) = slicing {
            let rows = self.slice_cuts(from, to, free);
            let (from, to) = self.slice_line(from, to, free);
            let aimed_row = rows.first().filter(|_| from == to).map(|(row, _)| *row);
            let (start, end) = match aimed_row {
                Some(row) => {
                    let top = self.track_top(row).max(self.lanes_top());
                    let bottom = self.track_top(row) + self.height_of(&self.project.tracks[row]);
                    (Point::new(from.x, top), Point::new(from.x, bottom))
                }
                None => (from, to),
            };
            if !rows.is_empty() && self.in_lanes(start.x) && start.x == end.x {
                let upright = Size::new(1.0, (end.y - start.y).abs());
                overlay.fill_rectangle(Point::new(start.x.round() - 0.5, start.y.min(end.y)), upright, p.accent);
            } else if !rows.is_empty() && self.in_lanes(start.x) {
                overlay.stroke(&Path::line(start, end), Stroke::default().with_color(p.accent).with_width(1.0));
            }
        }
        if let Some(Drag::Comp { from, to, band: (top, height), .. }) = &state.drag {
            let left = self.x_of(*from.min(to) as f64).max(self.lanes_left());
            let right = self.x_of(*from.max(to) as f64).min(self.lanes_right());
            if right > left {
                overlay.fill_rectangle(Point::new(left, *top), Size::new(right - left, *height), theme::alpha(p.accent, 0.22));
                overlay.stroke(&Path::rectangle(Point::new(left, *top), Size::new(right - left, *height)), Stroke::default().with_color(p.accent).with_width(1.0));
            }
        }
        if let Some(Drag::Reorder { at: Some(landing), .. }) = &state.drag {
            let y = self.track_top(*landing).max(self.lanes_top());
            if y <= bounds.height {
                overlay.fill_rectangle(Point::new(self.header_left(), y - LANDING_LINE / 2.0), Size::new(p.header_width, LANDING_LINE), p.accent);
                overlay.fill(&Path::circle(Point::new(self.header_left() + LANDING_DOT, y), LANDING_DOT), p.accent);
            }
        }
        if let Some(from) = self.recording_from {
            let left = self.x_of(from as f64).max(self.lanes_left());
            let right = self.x_of(self.playhead as f64).min(self.lanes_right());
            for (i, track) in self.project.tracks.iter().enumerate() {
                let top = self.track_top(i).max(self.lanes_top());
                let bottom = (self.track_top(i) + self.height_of(track)).min(bounds.height);
                if self.armed.contains(&track.id) && right > left && bottom > top {
                    let taken = Size::new(right - left, bottom - top);
                    overlay.fill_rectangle(Point::new(left, top), taken, theme::alpha(p.danger, 0.3));
                    self.draw_taking_shape(&mut overlay, left, right, top, bottom, from);
                }
            }
        }
        for (i, track) in self.project.tracks.iter().enumerate() {
            let bar = self.meter_bar(i);
            if !self.armed.contains(&track.id) || bar.y < self.lanes_top() || bar.y > bounds.height {
                continue;
            }
            let level = track.input.level(self.input_levels);
            let db = 20.0 * level.max(1e-6).log10();
            let along = |db: f32| ((db - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0) * bar.width;
            let filled = along(db);
            let clipping = level >= 1.0;
            overlay.fill_rectangle(bar.position(), bar.size(), theme::mix(p.panel, p.background, 0.6));
            let zones = [(METER_FLOOR_DB, p.meter_low), (METER_MID_DB, p.meter_mid), (METER_HIGH_DB, p.meter_high)];
            for (i, (from_db, colour)) in zones.iter().enumerate() {
                let from = along(*from_db);
                let to = zones.get(i + 1).map_or(bar.width, |(next, _)| along(*next)).min(filled);
                if to > from {
                    let colour = if clipping { p.danger } else { *colour };
                    overlay.fill_rectangle(Point::new(bar.x + from, bar.y), Size::new(to - from, bar.height), colour);
                }
            }
        }
        let x = self.x_of(self.playhead as f64).round();
        if self.in_lanes(x) {
            let top = p.scrollbar_height;
            let line = Size::new(p.playhead_width, bounds.height - top);
            overlay.fill_rectangle(Point::new(x - p.playhead_width / 2.0, top), line, p.accent);
            match p.playhead_cap {
                PlayheadCap::Triangle => {
                    let cap = Path::new(|b| {
                        b.move_to(Point::new(x - PLAYHEAD_CAP / 2.0, top));
                        b.line_to(Point::new(x + PLAYHEAD_CAP / 2.0, top));
                        b.line_to(Point::new(x, top + PLAYHEAD_CAP_DROP));
                        b.close();
                    });
                    overlay.fill(&cap, p.accent);
                }
                PlayheadCap::Square => {
                    let cap = Size::new(PLAYHEAD_CAP, PLAYHEAD_CAP_DROP);
                    overlay.fill_rectangle(Point::new(x - PLAYHEAD_CAP / 2.0, top), cap, p.accent);
                }
                PlayheadCap::None => {}
            }
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
            Some(Drag::Reorder { at: Some(_), .. }) => return mouse::Interaction::Grabbing,
            Some(Drag::Grip { grip, .. }) => return grip.pointer(),
            Some(Drag::Trim { .. }) => return mouse::Interaction::ResizingHorizontally,
            Some(Drag::Stretch { .. }) => return mouse::Interaction::ResizingHorizontally,
            Some(Drag::Pan { .. }) => return mouse::Interaction::ResizingVertically,
            _ => {}
        }
        match (self.tool, cursor.position_in(bounds).map(|p| self.hit(p))) {
            (_, Some(Hit::Resize(_))) => mouse::Interaction::ResizingVertically,
            (
                _,
                Some(
                    Hit::Mute(_)
                    | Hit::Solo(_)
                    | Hit::Pan(_)
                    | Hit::Arm(_)
                    | Hit::Route(_)
                    | Hit::Fx(_)
                    | Hit::Remove(_)
                    | Hit::AddTrack
                    | Hit::Tool(_),
                ),
            ) => {
                mouse::Interaction::Pointer
            }
            (Tool::Razor, Some(Hit::Clip(_) | Hit::Grip(..) | Hit::Lane)) => mouse::Interaction::Crosshair,
            (Tool::Mute | Tool::Delete, Some(Hit::Clip(_) | Hit::Grip(..))) => mouse::Interaction::Pointer,
            (Tool::Comp, Some(Hit::Clip(_) | Hit::Grip(..))) => mouse::Interaction::Crosshair,
            (Tool::Pencil, Some(Hit::Grip(_, grip))) => grip.pointer(),
            (_, Some(Hit::Edge(..))) => mouse::Interaction::ResizingHorizontally,
            (Tool::Pencil, Some(Hit::Clip(_))) => mouse::Interaction::Grab,
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

        if self.project.tracks.is_empty() && self.opening {
            return;
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
            let top = self.track_top(i) - self.lanes_top();
            let height = self.height_of(track);
            if height <= 0.0 {
                continue;
            }
            if top > size.height || top + height < 0.0 {
                continue;
            }
            frame.fill_rectangle(Point::new(0.0, top + height - 1.0), Size::new(size.width, 1.0), p.line);
            let track_colour = if track.muted { p.text_faint } else { self.colour_of(i) };
            let clips_height = self.clips_height(track);
            for clip in &track.clips {
                let colour = if clip.muted { p.text_faint } else { track_colour };
                self.draw_clip(frame, size, clip, top, clips_height, colour);
            }
            let mut lane_top = top + clips_height;
            for shape in self.lanes_of(track) {
                self.draw_envelope(frame, size, shape, lane_top);
                lane_top += LANE_HEIGHT;
            }
        }
    }

    fn envelope_at(&self, index: usize, p: Point) -> Option<Hit<'_>> {
        let track = &self.project.tracks[index];
        let mut top = self.track_top(index) + self.clips_height(track);
        for shape in self.lanes_of(track) {
            if p.y >= top && p.y < top + LANE_HEIGHT {
                let inset = 7.0;
                let span = (LANE_HEIGHT - inset * 2.0).max(1.0);
                let reach = (shape.highest - shape.lowest).max(f32::EPSILON);
                let value = shape.lowest + reach * (1.0 - ((p.y - top - inset) / span).clamp(0.0, 1.0));
                let offset = self.lane_offset(shape.target);
                let at = (self.frames_at(p.x).max(0.0) as Frames).saturating_sub(offset);
                let near = shape.points.iter().position(|point| {
                    let seen = Point::new(self.x_of((point.at + offset) as f64), {
                        top + inset + span * (1.0 - (point.value - shape.lowest) / reach)
                    });
                    seen.distance(p) <= HANDLE_REACH
                });
                return Some(Hit::Envelope(shape.target, near, at, value));
            }
            top += LANE_HEIGHT;
        }
        None
    }

    fn lane_top_of(&self, target: loupe_engine::Target) -> Option<f32> {
        for (i, track) in self.project.tracks.iter().enumerate() {
            let mut top = self.track_top(i) + self.clips_height(track);
            for shape in self.lanes_of(track) {
                if shape.target == target {
                    return Some(top);
                }
                top += LANE_HEIGHT;
            }
        }
        None
    }

    fn lane_label(&self, target: loupe_engine::Target) -> String {
        use loupe_engine::Target;
        let track_name = |want: TrackId| {
            self.project.tracks.iter().find(|t| t.id == want).map(|t| t.name.clone()).unwrap_or_default()
        };
        match target {
            Target::MasterGain => "Master volume".into(),
            Target::TrackGain(_) => "Volume".into(),
            Target::TrackPan(_) => "Pan".into(),
            Target::SendGain { to, .. } => format!("Send to {}", track_name(to)),
            Target::TrackFx { slot, knob, track } => {
                let name = self
                    .project
                    .tracks
                    .iter()
                    .find(|t| t.id == track)
                    .and_then(|t| t.fx.get(slot))
                    .map(|fx| fx.name.clone())
                    .unwrap_or_default();
                format!("{name}: {}", self.knob_name(track.0, slot, knob, false))
            }
            Target::MasterFx { slot, knob } => {
                let name = self.project.master_fx.get(slot).map(|fx| fx.name.clone()).unwrap_or_default();
                format!("Master {name}: {}", self.knob_name(crate::MASTER_OWNER, slot, knob, false))
            }
            Target::ClipGain(_) => "Clip gain".into(),
            Target::ClipFx { clip, slot, knob } => {
                let name = self
                    .project
                    .clip(clip)
                    .and_then(|found| found.fx.get(slot))
                    .map(|fx| fx.name.clone())
                    .unwrap_or_default();
                format!("Clip {name}: {}", self.knob_name(clip.0, slot, knob, true))
            }
        }
    }

    fn knob_name(&self, owner: u64, slot: usize, knob: usize, on_clip: bool) -> String {
        match self.knobs.get(&(owner, slot, knob, on_clip)) {
            Some(found) => found.clone(),
            None => format!("knob {knob}"),
        }
    }

    fn lane_offset(&self, target: loupe_engine::Target) -> Frames {
        match target.on_clip().and_then(|id| self.project.clip(id)) {
            Some(clip) => clip.start,
            None => 0,
        }
    }

    fn draw_envelope(&self, frame: &mut Frame, size: Size, shape: &loupe_engine::Envelope, top: f32) {
        let p = self.palette;
        frame.fill_rectangle(Point::new(0.0, top), Size::new(size.width, LANE_HEIGHT), theme::mix(p.background, p.panel, 0.5));
        frame.fill_rectangle(Point::new(0.0, top + LANE_HEIGHT - 1.0), Size::new(size.width, 1.0), p.line);
        let inset = 7.0;
        let span = (LANE_HEIGHT - inset * 2.0).max(1.0);
        let reach = (shape.highest - shape.lowest).max(f32::EPSILON);
        let y_of = |value: f32| top + inset + span * (1.0 - (value - shape.lowest) / reach);
        let offset = self.lane_offset(shape.target);
        let x_of = |at: Frames| self.x_of((at + offset) as f64) - self.lanes_left();
        let mut line = iced::widget::canvas::path::Builder::new();
        let mut started = false;
        let mut step = 0.0f32;
        while step <= size.width {
            let at = self.frames_at(step + self.lanes_left()).max(0.0) as Frames;
            let value = shape.value_at(at.saturating_sub(offset)).unwrap_or(shape.lowest);
            let at = Point::new(step, y_of(value));
            if started {
                line.line_to(at);
            } else {
                line.move_to(at);
                started = true;
            }
            step += 2.0;
        }
        if started {
            frame.stroke(&line.build(), Stroke::default().with_color(p.accent).with_width(1.6));
        }
        for point in &shape.points {
            let at = Point::new(x_of(point.at), y_of(point.value));
            if at.x < -6.0 || at.x > size.width + 6.0 {
                continue;
            }
            frame.fill(&Path::circle(at, 3.5), p.accent);
        }
        frame.fill_text(Text {
            content: self.lane_label(shape.target),
            position: Point::new(6.0, top + 4.0),
            color: p.text_dim,
            size: 10.5.into(),
            font: p.medium,
            ..Text::default()
        });
    }

    fn draw_clip(&self, frame: &mut Frame, size: Size, clip: &Clip, lane_top: f32, lane_height: f32, colour: Color) {
        let p = self.palette;
        let left = self.x_of(clip.start as f64) - self.lanes_left();
        let right = self.x_of(clip.end() as f64) - self.lanes_left();
        if right < 0.0 || left > size.width {
            return;
        }
        let selected = self.selection.contains(&clip.id);
        let top = lane_top + self.palette.clip_padding;
        let height = lane_height - self.palette.clip_padding * 2.0;
        let shown_left = left.max(-8.0);
        let shown_right = right.min(size.width + 8.0);
        let shown_width = (shown_right - shown_left).max(1.0);
        let body = Path::new(|b| {
            b.rounded_rectangle(Point::new(shown_left, top), Size::new(shown_width, height), p.clip_corner.into());
        });
        let tint = if selected { 0.26 } else { 0.16 };
        for (drop, strength) in [(1.0, 0.55), (2.5, 0.25)] {
            let shadow = Path::new(|b| {
                b.rounded_rectangle(Point::new(shown_left, top + drop), Size::new(shown_width, height), p.clip_corner.into());
            });
            frame.fill(&shadow, theme::alpha(Color::BLACK, p.shade() * strength));
        }
        frame.fill(&body, theme::mix(p.background, colour, tint));
        let title = Path::new(|b| {
            b.rounded_rectangle(Point::new(shown_left, top), Size::new(shown_width, p.clip_title_height), iced::border::top(p.clip_corner));
        });
        frame.fill(
            &title,
            canvas::gradient::Linear::new(Point::new(0.0, top), Point::new(0.0, top + p.clip_title_height))
                .add_stop(0.0, theme::mix(p.background, colour, tint + 0.25))
                .add_stop(1.0, theme::mix(p.background, colour, tint + 0.17)),
        );

        let wave_top = top + self.palette.clip_title_height + 2.0;
        let wave_height = height - self.palette.clip_title_height - 5.0;
        if wave_height > 4.0 {
            match &clip.notes {
                Some(notes) => self.draw_notes(frame, clip, notes, left, shown_left.max(0.0), shown_right.min(size.width), wave_top, wave_height, colour),
                None => match take_lane_height(clip, wave_height) {
                    Some(lane) => {
                        let (from_x, to_x) = (shown_left.max(0.0), shown_right.min(size.width));
                        for take in 0..clip.takes.len() {
                            let lane_top = wave_top + lane * take as f32;
                            if take == clip.take {
                                frame.fill_rectangle(Point::new(from_x, lane_top), Size::new((to_x - from_x).max(0.0), lane), theme::alpha(colour, 0.16));
                            }
                            if take > 0 {
                                frame.fill_rectangle(Point::new(from_x, lane_top), Size::new((to_x - from_x).max(0.0), 1.0), theme::alpha(colour, 0.25));
                            }
                            let Some(offset) = clip.take_offset(take) else { continue };
                            let tone = if take == clip.take { colour } else { theme::mix(p.background, colour, 0.45) };
                            self.draw_waveform(frame, clip, offset, left, from_x, to_x, lane_top + 1.0, lane - 2.0, tone);
                        }
                    }
                    None => self.draw_waveform(frame, clip, clip.offset, left, shown_left.max(0.0), shown_right.min(size.width), wave_top, wave_height, colour),
                },
            }
        }

        let outline = if selected {
            Stroke::default().with_color(p.accent).with_width(1.5)
        } else {
            Stroke::default().with_color(theme::mix(p.background, colour, 0.5)).with_width(1.0)
        };
        frame.stroke(&body, outline);
        self.draw_fades_and_grips(frame, clip, colour);

        let title_on_canvas = Rectangle::new(
            Point::new(self.lanes_left() + shown_left, self.lanes_top() + top),
            Size::new(shown_width, self.palette.clip_title_height),
        );
        let lanes_on_canvas = Rectangle::new(Point::new(self.lanes_left(), self.lanes_top()), size);
        if let Some(visible) = title_on_canvas.intersection(&lanes_on_canvas) {
            frame.with_clip(visible, |name_region| {
                name_region.fill_text(Text {
                    content: match (clip.is_stretched(), clip.waiting_for_stretch()) {
                        (false, _) => clip.called().to_string(),
                        (true, false) => format!("{}  {:.0}%", clip.called(), clip.stretch * 100.0),
                        (true, true) => format!("{}  {:.0}%  stretching…", clip.called(), clip.stretch * 100.0),
                    } + &if clip.has_takes() { format!("  ·  take {} of {}", clip.take + 1, clip.takes.len()) } else { String::new() },
                    position: Point::new(
                        self.lanes_left() + left.max(0.0) + 8.0 - visible.x,
                        title_on_canvas.y - visible.y + self.palette.clip_title_height / 2.0,
                    ),
                    color: p.text,
                    size: p.clip_title_size.into(),
                    font: p.medium,
                    vertical_alignment: alignment::Vertical::Center,
                    ..Text::default()
                });
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_notes(
        &self,
        frame: &mut Frame,
        clip: &Clip,
        notes: &[loupe_engine::Note],
        clip_left: f32,
        from_x: f32,
        to_x: f32,
        top: f32,
        height: f32,
        colour: Color,
    ) {
        let shown: Vec<&loupe_engine::Note> = notes.iter().filter(|note| note.end() > clip.offset && note.start < clip.offset + clip.len).collect();
        let (Some(low), Some(high)) = (shown.iter().map(|n| n.key).min(), shown.iter().map(|n| n.key).max()) else {
            return;
        };
        let rows = (high - low) as f32 + 1.0;
        let row_h = (height / rows.max(6.0)).clamp(1.5, 6.0);
        let span = row_h * rows;
        let first_y = top + (height - span) / 2.0;
        let px_per_frame = (self.view.zoom / self.rate()) as f32;
        for note in shown {
            let start = note.start.max(clip.offset) - clip.offset;
            let end = note.end().min(clip.offset + clip.len) - clip.offset;
            let x0 = (clip_left + start as f32 * px_per_frame).max(from_x);
            let x1 = (clip_left + end as f32 * px_per_frame).min(to_x);
            if x1 <= x0 {
                continue;
            }
            let y = first_y + (high - note.key) as f32 * row_h;
            frame.fill_rectangle(Point::new(x0, y), Size::new((x1 - x0 - 1.0).max(1.5), (row_h - 0.5).max(1.0)), colour);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_waveform(
        &self,
        frame: &mut Frame,
        clip: &Clip,
        offset: Frames,
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
        let source_at = |x: f32| offset as f64 + (x - clip_left) as f64 * frames_per_px;
        let source_end = (offset + clip.len) as f64;

        if frames_per_px < 1.0 {
            let first = source_at(from_x).floor().max(offset as f64) as usize;
            let last = (source_at(to_x).ceil().min(source_end - 1.0)) as usize;
            let samples = clip.audio();
            let point = |i: usize| {
                let level = clip.gain * clip.fade_level(i as Frames - offset);
                let value = ((samples[i][0] + samples[i][1]) * 0.5 * level).clamp(-1.0, 1.0);
                Point::new(
                    clip_left + ((i as f64 - offset as f64) / frames_per_px) as f32,
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
            let a = source_at(x).max(offset as f64);
            let b = (a + frames_per_px).min(source_end);
            if b <= a {
                break;
            }
            let (lo, hi) = clip.peak(a as usize, (b.ceil() as usize).max(a as usize + 1));
            let level = clip.gain * clip.fade_level((a - offset as f64) as Frames);
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
        let in_lanes = |at: Point| Point::new(at.x - self.lanes_left(), at.y - self.lanes_top());
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
        let on_screen = |at: &Point| at.x >= self.lanes_left() - HANDLE_REACH && at.x <= self.lanes_right() + HANDLE_REACH;
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
                        b.line_to(Point::new(at.x + self.palette.fade_flag_size * inward, at.y));
                        b.line_to(Point::new(at.x, at.y + self.palette.fade_flag_size));
                        b.close();
                    });
                    frame.fill(&flag, p.text);
                }
                Grip::ShapeIn | Grip::ShapeOut => {
                    let ring = Path::circle(at, self.palette.handle_radius);
                    frame.fill(&ring, selected_body);
                    frame.stroke(&ring, Stroke::default().with_color(p.text).with_width(1.5));
                }
                Grip::Gain => {
                    let dot = Path::circle(at, self.palette.handle_radius);
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
            b.rounded_rectangle(Point::new(left - self.lanes_left(), 2.0), Size::new(width, size.height - 5.0), 4.0.into());
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
                size: p.ruler_text_size.into(),
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
        let small_corner = (p.corner - 1.0).max(0.0);
        frame.fill_rectangle(Point::ORIGIN, size, p.panel);
        for (i, track) in self.project.tracks.iter().enumerate() {
            let top = self.track_top(i) - self.lanes_top();
            let height = self.height_of(track);
            if height <= 0.0 {
                continue;
            }
            if top > size.height || top + height < 0.0 {
                continue;
            }
            let tint = if track.muted { MUTED_HEADER_TINT } else { HEADER_TINT };
            frame.fill_rectangle(
                Point::new(0.0, top),
                Size::new(size.width, height - 1.0),
                theme::mix(p.panel, self.colour_of(i), tint),
            );
            frame.fill_rectangle(Point::new(0.0, top), Size::new(size.width, height - 1.0), sheen_fill(p, top, height - 1.0));
            if self.chosen_tracks.contains(&track.id) {
                frame.fill_rectangle(Point::new(0.0, top), Size::new(size.width, height - 1.0), theme::alpha(p.accent, 0.14));
                frame.fill_rectangle(Point::new(0.0, top), Size::new(CHOSEN_EDGE, height - 1.0), p.accent);
            }
            frame.fill_rectangle(Point::new(0.0, top + height - 1.0), Size::new(size.width, 1.0), p.line);
            let depth = self.project.depth_of(track.id).min(4) as f32;
            let indent = depth * INDENT_W;
            if depth > 0.0 {
                frame.fill_rectangle(
                    Point::new(indent - 7.0, top + 6.0),
                    Size::new(2.0, height - 13.0),
                    theme::alpha(self.colour_of(i), 0.7),
                );
            }
            if self.project.tracks.iter().any(|t| t.parent == Some(track.id)) {
                frame.fill_text(Text {
                    content: icons::glyph(if track.collapsed { "chevron-right" } else { "chevron-down" }).to_string(),
                    position: Point::new(indent + 6.0, top + 20.0),
                    color: p.text_dim,
                    size: 13.0.into(),
                    font: icons::font(if track.collapsed { "chevron-right" } else { "chevron-down" }),
                    vertical_alignment: alignment::Vertical::Center,
                    ..Text::default()
                });
            }
            let (name_left, room_for_name, name_top) = self.name_spot(i);
            if room_for_name >= NAME_NARROWEST && name_top + NAME_LINE / 2.0 - self.lanes_top() <= top + height {
                frame.fill_text(Text {
                    content: shorten(&track.name, (room_for_name / NAME_LETTER) as usize),
                    position: Point::new(name_left, name_top - self.lanes_top()),
                    color: if track.muted { p.text_dim } else { p.text },
                    size: p.track_title_size.into(),
                    font: p.medium,
                    vertical_alignment: alignment::Vertical::Center,
                    ..Text::default()
                });
            }

            let remove = self.remove_button(i);
            frame.fill_text(Text {
                content: icons::glyph("x").to_string(),
                position: Point::new(remove.center_x() - self.header_left(), remove.center_y() - self.lanes_top()),
                color: p.text_dim,
                size: 14.0.into(),
                font: icons::font("x"),
                horizontal_alignment: alignment::Horizontal::Center,
                vertical_alignment: alignment::Vertical::Center,
                shaping: iced::widget::text::Shaping::Advanced,
                ..Text::default()
            });

            let mute = self.mute_button(i);
            let shape = Path::new(|b| {
                b.rounded_rectangle(Point::new(mute.x - self.header_left(), mute.y - self.lanes_top()), mute.size(), small_corner.into());
            });
            frame.fill(&shape, if track.muted { p.danger.into() } else { raised_fill(p, p.raised, mute.y - self.lanes_top(), mute.height) });
            frame.fill_text(Text {
                content: "M".into(),
                position: Point::new(mute.center_x() - self.header_left(), mute.center_y() - self.lanes_top()),
                color: if track.muted { p.on_accent } else { p.text_dim },
                size: 11.5.into(),
                font: p.semibold,
                horizontal_alignment: alignment::Horizontal::Center,
                vertical_alignment: alignment::Vertical::Center,
                ..Text::default()
            });

            let solo = self.solo_button(i);
            let shape = Path::new(|b| {
                b.rounded_rectangle(Point::new(solo.x - self.header_left(), solo.y - self.lanes_top()), solo.size(), small_corner.into());
            });
            frame.fill(&shape, if track.solo { p.accent.into() } else { raised_fill(p, p.raised, solo.y - self.lanes_top(), solo.height) });
            frame.fill_text(Text {
                content: "S".into(),
                position: Point::new(solo.center_x() - self.header_left(), solo.center_y() - self.lanes_top()),
                color: if track.solo { p.on_accent } else { p.text_dim },
                size: 11.5.into(),
                font: p.semibold,
                horizontal_alignment: alignment::Horizontal::Center,
                vertical_alignment: alignment::Vertical::Center,
                ..Text::default()
            });

            if let Some(knob) = self.pan_knob(i) {
                let centre = Point::new(knob.center_x() - self.header_left(), knob.center_y() - self.lanes_top());
                let radius = knob.width / 2.0 - 1.5;
                let top = -std::f32::consts::FRAC_PI_2;
                let angle = top + track.pan * 0.75 * std::f32::consts::PI;
                let arc = |from: f32, to: f32| {
                    Path::new(|b| {
                        b.arc(canvas::path::Arc { center: centre, radius, start_angle: iced::Radians(from.min(to)), end_angle: iced::Radians(from.max(to)) });
                    })
                };
                frame.fill(&Path::circle(centre, radius - 3.0), p.raised);
                frame.stroke(&arc(top - 0.75 * std::f32::consts::PI, top + 0.75 * std::f32::consts::PI), Stroke::default().with_color(p.hover).with_width(2.0));
                if track.pan != 0.0 {
                    frame.stroke(&arc(top, angle), Stroke::default().with_color(p.accent).with_width(2.0));
                }
                let along = |distance: f32| Point::new(centre.x + angle.cos() * distance, centre.y + angle.sin() * distance);
                frame.stroke(&Path::line(along(radius * 0.25), along(radius - 4.0)), Stroke::default().with_color(p.text).with_width(2.0));
            }

            if self.shows_arm(i) {
            let arm = self.arm_button(i);
            let armed = self.armed.contains(&track.id);
            let surround = Path::new(|b| {
                b.rounded_rectangle(Point::new(arm.x - self.header_left(), arm.y - self.lanes_top()), arm.size(), small_corner.into());
            });
            frame.fill(&surround, raised_fill(p, p.raised, arm.y - self.lanes_top(), arm.height));
            let middle = Point::new(arm.center_x() - self.header_left(), arm.center_y() - self.lanes_top());
            let dot = match p.arm_shape {
                ArmShape::Dot => Path::circle(middle, ARM_DOT_RADIUS),
                ArmShape::Square => Path::rectangle(
                    Point::new(middle.x - ARM_DOT_RADIUS, middle.y - ARM_DOT_RADIUS),
                    Size::new(ARM_DOT_RADIUS * 2.0, ARM_DOT_RADIUS * 2.0),
                ),
            };
            if armed {
                frame.fill(&dot, p.danger);
            } else {
                frame.stroke(&dot, Stroke::default().with_color(p.text_dim).with_width(1.5));
            }
            }
            if self.shows_route(i) {

            let route = self.route_button(i);
            let wired = !track.sends.is_empty() || track.parent.is_some();
            let pad = Path::new(|b| {
                b.rounded_rectangle(Point::new(route.x, route.y - self.lanes_top()), route.size(), 5.0.into());
            });
            frame.fill(&pad, raised_fill(p, p.raised, route.y - self.lanes_top(), route.height));
            frame.fill_text(Text {
                content: "→".into(),
                position: Point::new(route.center_x(), route.center_y() - self.lanes_top()),
                color: if wired { p.accent } else { p.text_dim },
                size: 13.0.into(),
                font: p.medium,
                horizontal_alignment: alignment::Horizontal::Center,
                vertical_alignment: alignment::Vertical::Center,
                ..Text::default()
            });
            }

            if self.shows_fx(i) {
            let fx_at = self.fx_button(i);
            let loaded = track.fx.iter().filter(|fx| !fx.record).count();
            let pad = Path::new(|b| {
                b.rounded_rectangle(Point::new(fx_at.x, fx_at.y - self.lanes_top()), fx_at.size(), 5.0.into());
            });
            frame.fill(&pad, raised_fill(p, p.raised, fx_at.y - self.lanes_top(), fx_at.height));
            frame.fill_text(Text {
                content: "FX".into(),
                position: Point::new(fx_at.center_x(), fx_at.center_y() - self.lanes_top()),
                color: if loaded > 0 { p.accent } else { p.text_dim },
                size: 11.0.into(),
                font: p.medium,
                horizontal_alignment: alignment::Horizontal::Center,
                vertical_alignment: alignment::Vertical::Center,
                ..Text::default()
            });
            }
        }

        let add = self.add_button();
        frame.fill_text(Text {
            content: "+  Add track".into(),
            position: Point::new(16.0, add.center_y() - self.lanes_top()),
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

fn take_lane_height(clip: &Clip, wave_height: f32) -> Option<f32> {
    if !clip.has_takes() {
        return None;
    }
    let lane = wave_height / clip.takes.len() as f32;
    (lane >= MIN_TAKE_LANE).then_some(lane)
}

fn raised_fill(p: &Palette, base: Color, top: f32, height: f32) -> canvas::Fill {
    canvas::gradient::Linear::new(Point::new(0.0, top), Point::new(0.0, top + height))
        .add_stop(0.0, theme::mix(base, Color::WHITE, p.glint() * 1.4))
        .add_stop(1.0, theme::mix(base, Color::BLACK, p.shade() * 0.2))
        .into()
}

fn sheen_fill(p: &Palette, top: f32, height: f32) -> canvas::Fill {
    let light = if p.is_light() { 0.28 } else { 0.08 };
    canvas::gradient::Linear::new(Point::new(0.0, top), Point::new(0.0, top + height))
        .add_stop(0.0, theme::alpha(Color::WHITE, light))
        .add_stop(0.6, theme::alpha(Color::WHITE, 0.0))
        .add_stop(1.0, theme::alpha(Color::WHITE, 0.0))
        .into()
}
