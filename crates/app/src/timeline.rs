use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::widget::canvas::{self, Cache, Frame, Geometry, Path, Stroke, Text};
use iced::{alignment, keyboard, mouse, Color, Point, Rectangle, Renderer, Size, Theme};
use loupe_engine::{Clip, ClipId, Edge, Fade, Frames, Project, Track, TrackId};

use crate::pointer::EndlessDrag;
use crate::theme::{self, ArmShape, Palette, PlayheadCap, Side};
use crate::{icons, Message};

const LANE_HEIGHT: f32 = 90.0;
const LANE_SHORTEST: f32 = 40.0;
const LANE_TALLEST: f32 = 400.0;
const LANE_EDGE_REACH: f32 = 4.0;
const LANE_INSET: f32 = 9.0;
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
const COLOUR_EDGE: f32 = 3.0;
const REORDER_STARTS_AT: f32 = 6.0;
const LANDING_LINE: f32 = 3.0;
const LANDING_DOT: f32 = 4.0;
const NAME_NARROWEST: f32 = 26.0;
const NAME_LETTER: f32 = 7.0;
const NAME_LINE: f32 = 16.0;
const PAN_KNOB: f32 = 24.0;
const PAN_PER_PX: f32 = 2.0 / crate::knob::SWEEP_PX;
const ARM_DOT_RADIUS: f32 = 5.0;
const PLAYHEAD_CAP: f32 = 11.0;
const PLAYHEAD_CAP_DROP: f32 = 9.0;
const METER_FLOOR_DB: f32 = -60.0;
const METER_MID_DB: f32 = -12.0;
const METER_HIGH_DB: f32 = -6.0;
const HOLD_LINE: f32 = 2.0;
const METER_REACH: f32 = 4.0;
pub(crate) const HEADER_TINT: f32 = 0.42;
pub(crate) const MUTED_HEADER_TINT: f32 = 0.16;
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
const GAIN_LINE_SPAN_DB: f32 = 12.0;
const GAIN_UNITY_SLACK_DB: f32 = 0.05;
const MIN_WAVE_PX_FOR_GAIN_LINE: f32 = 9.0;
const MIN_WAVE_PX_FOR_GAIN_FIGURE: f32 = 28.0;
const MIN_SEEN_PX_FOR_GAIN_LINE: f32 = 3.0;
const GAIN_FIGURE_SIZE: f32 = 9.5;
const GAIN_FIGURE_PAD: f32 = 4.0;
const GAIN_FIGURE_INSET: f32 = 6.0;
const GAIN_FIGURE_LIFT: f32 = 7.0;
const GAIN_PLATE_HEIGHT: f32 = 14.0;
pub const MIN_GAIN_DB: f32 = -24.0;
const TAKE_LANE_INSET: f32 = 3.0;
const BADGE_LETTER: f32 = 6.2;
const BADGE_PAD: f32 = 6.0;
const BADGE_INSET: f32 = 3.0;
const BADGE_CHEVRON: f32 = 7.0;
const BADGE_ROOM: f32 = 8.0;
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
    pub track_meters: &'a [crate::metering::Meter],
    pub opening: bool,
    pub width: f32,
    pub knobs: &'a HashMap<(u64, usize, usize, bool), String>,
    pub cache: &'a Cache,
    pub at: &'a std::cell::Cell<Rectangle>,
    pub pixels_per_point: f32,
}

#[derive(Default)]
pub struct Interaction {
    drag: Option<Drag>,
    last_press: Option<(ClipId, Instant)>,
    last_pan_press: Option<(TrackId, Instant)>,
    last_name_press: Option<(TrackId, Instant)>,
    modifiers: keyboard::Modifiers,
    pointing: Option<(loupe_engine::Target, usize)>,
}

enum Drag {
    Range { anchor: Frames, origin: Point, moving: bool },
    Clip { id: ClipId, grab: f64, origin: Point, moving: bool },
    Comp { clip: ClipId, track: TrackId, take: usize, from: Frames, to: Frames, band: (f32, f32) },
    Resize { track: TrackId, top: f32 },
    LaneResize { target: loupe_engine::Target, top: f32 },
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
}

enum Hit<'a> {
    Tool(Tool),
    Scrollbar,
    Ruler,
    Resize(&'a Track),
    LaneEdge(loupe_engine::Target),
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
    Take(&'a Clip, usize),
    Folder(&'a Track),
    TakeHeader(&'a Track, usize),
    RemoveTake(&'a Track, usize),
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

    fn tool_at(&self, p: Point) -> Option<&'static str> {
        Tool::ALL.into_iter().enumerate().find(|(i, _)| self.tool_button(*i).contains(p)).map(|(_, tool)| tool.words())
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
        self.clips_height(track) + self.takes_height(track) + self.lanes_of(track).iter().map(|shape| self.lane_tall(shape.target)).sum::<f32>()
    }

    fn take_lanes(&self, track: &Track) -> usize {
        if self.hidden(track) {
            return 0;
        }
        open_take_lanes(track)
    }

    fn takes_height(&self, track: &Track) -> f32 {
        self.take_lanes(track) as f32 * self.take_tall(track)
    }

    fn take_tall(&self, track: &Track) -> f32 {
        self.clips_height(track)
    }

    fn header_height(&self, track: &Track) -> f32 {
        if self.take_lanes(track) > 0 {
            return self.clips_height(track);
        }
        self.height_of(track) - self.takes_height(track)
    }

    fn envelopes_top(&self, track: &Track) -> f32 {
        self.clips_height(track) + self.takes_height(track)
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
        out_of_sight(self.project, track)
    }
}

fn ours(words: &'static str) -> bool {
    Tool::ALL.into_iter().any(|tool| tool.words() == words)
}

fn hint_now(hinting: Option<&'static str>, over: Option<&'static str>, inside: bool) -> Option<Option<&'static str>> {
    if !inside {
        return hinting.is_some_and(ours).then_some(None);
    }
    (over != hinting).then_some(over)
}

fn out_of_sight(project: &Project, track: &Track) -> bool {
    if track.hidden {
        return true;
    }
    let mut at = track.parent;
    let mut steps = 0;
    while let Some(id) = at {
        let Some(parent) = project.tracks.iter().find(|t| t.id == id) else {
            return false;
        };
        if parent.collapsed {
            return true;
        }
        steps += 1;
        if steps > project.tracks.len() {
            return false;
        }
        at = parent.parent;
    }
    false
}

impl Timeline<'_> {
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
        if self.header_height(track) >= ROOMY_HEADER_H {
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
        let height = self.header_height(&self.project.tracks[index]);
        let corner = if height >= ROOMY_HEADER_H {
            Point::new(self.header_left() + 16.0, top + height - 36.0)
        } else {
            Point::new(self.header_right() - 68.0, top + 9.0)
        };
        Rectangle::new(corner, Size::new(ARM_BUTTON, 22.0))
    }

    fn route_button(&self, index: usize) -> Rectangle {
        let arm = self.arm_button(index);
        let roomy = self.header_height(&self.project.tracks[index]) >= ROOMY_HEADER_H;
        let left = if roomy { arm.x + arm.width + ARM_GAP } else { arm.x - ARM_GAP - ARM_BUTTON };
        Rectangle::new(Point::new(left, arm.y), Size::new(ARM_BUTTON, arm.height))
    }

    fn fx_button(&self, index: usize) -> Rectangle {
        let route = self.route_button(index);
        let roomy = self.header_height(&self.project.tracks[index]) >= ROOMY_HEADER_H;
        let left = if roomy { route.x + route.width + ARM_GAP } else { route.x - ARM_GAP - ARM_BUTTON };
        Rectangle::new(Point::new(left, route.y), Size::new(ARM_BUTTON, route.height))
    }

    fn solo_button(&self, index: usize) -> Rectangle {
        let mute = self.mute_button(index);
        let roomy = self.header_height(&self.project.tracks[index]) >= ROOMY_HEADER_H;
        let left = if roomy { mute.x + mute.width + ARM_GAP } else { mute.x - ARM_GAP - mute.width };
        Rectangle::new(Point::new(left, mute.y), mute.size())
    }

    fn pan_knob(&self, index: usize) -> Option<Rectangle> {
        let roomy = self.header_height(&self.project.tracks[index]) >= ROOMY_HEADER_H;
        let route = self.route_button(index);
        let after_fx = self.fx_button(index);
        let left = after_fx.x + after_fx.width + ARM_GAP;
        (roomy && left + PAN_KNOB <= self.header_right() - 6.0).then(|| Rectangle::new(Point::new(left, route.center_y() - PAN_KNOB / 2.0), Size::new(PAN_KNOB, PAN_KNOB)))
    }

    fn arm_button(&self, index: usize) -> Rectangle {
        let mute = self.solo_button(index);
        let roomy = self.header_height(&self.project.tracks[index]) >= ROOMY_HEADER_H;
        let left = if roomy { mute.x + mute.width + ARM_GAP } else { mute.x - ARM_GAP - ARM_BUTTON };
        Rectangle::new(Point::new(left, mute.y), Size::new(ARM_BUTTON, mute.height))
    }

    fn meter_at(&self, p: Point) -> Option<TrackId> {
        self.project.tracks.iter().enumerate().find_map(|(index, track)| {
            let bar = self.meter_bar(index);
            let reach = Rectangle::new(Point::new(bar.x, bar.y - METER_REACH), Size::new(bar.width, bar.height + METER_REACH * 2.0));
            (self.armed.contains(&track.id) && bar.y >= self.lanes_top() && reach.contains(p)).then_some(track.id)
        })
    }

    fn meter_bar(&self, index: usize) -> Rectangle {
        let top = self.track_top(index);
        let height = self.header_height(&self.project.tracks[index]);
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

    fn take_remove_button(&self, index: usize, take: usize) -> Rectangle {
        Rectangle::new(Point::new(self.header_right() - 34.0, self.take_lane_top(index, take) + 8.0), Size::new(24.0, 24.0))
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
        if self.header_height(track) >= ROOMY_HEADER_H {
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
        (self.header_height(&self.project.tracks[index]) > 0.0 && room >= NAME_NARROWEST).then(|| Rectangle {
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
            height: self.clips_height(&self.project.tracks[index]) - self.palette.clip_padding * 2.0,
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

    fn take_lane_top(&self, index: usize, take: usize) -> f32 {
        let track = &self.project.tracks[index];
        self.track_top(index) + self.clips_height(track) + take as f32 * self.take_tall(track)
    }

    fn take_row_at(&self, index: usize, y: f32) -> Option<usize> {
        let track = &self.project.tracks[index];
        let lanes = self.take_lanes(track);
        let row = (y - self.take_lane_top(index, 0)) / self.take_tall(track);
        (lanes > 0 && row >= 0.0 && row < lanes as f32).then_some(row as usize)
    }

    fn draw_take_headers(&self, frame: &mut Frame, size: Size) {
        let p = self.palette;
        for (i, track) in self.project.tracks.iter().enumerate() {
            let lanes = self.take_lanes(track);
            if lanes == 0 {
                continue;
            }
            let tall = self.take_tall(track);
            let colour = self.colour_of(i);
            let depth = self.project.depth_of(track.id).min(4) as f32;
            let indent = COLOUR_EDGE;
            let name_left = indent + (depth + 1.0) * INDENT_W + 6.0;
            for take in 0..lanes {
                let top = self.take_lane_top(i, take) - self.lanes_top();
                if top > size.height || top + tall < 0.0 {
                    continue;
                }
                let used = track.clips.iter().any(|clip| clip.takes.len() > take && clip.take == take);
                let row = Point::new(indent, top);
                let row_size = Size::new((size.width - indent).max(0.0), tall - 1.0);
                frame.fill_rectangle(row, row_size, theme::mix(p.panel, colour, if used { 0.32 } else { 0.16 }));
                frame.fill_rectangle(Point::new(indent, top + tall - 1.0), Size::new(row_size.width, 1.0), p.line);
                frame.fill_rectangle(Point::new(indent, top), Size::new(3.0, tall - 1.0), if used { colour } else { theme::alpha(colour, 0.35) });
                let middle = top + (tall / 2.0).min(20.0);
                frame.fill(&Path::circle(Point::new(name_left, middle), 4.5), if used { p.accent } else { theme::alpha(p.text_dim, 0.4) });
                let remove = self.take_remove_button(i, take);
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
                frame.fill_text(Text {
                    content: format!("Take {}", take + 1),
                    position: Point::new(name_left + 12.0, middle),
                    color: if used { p.text } else { p.text_dim },
                    size: p.track_title_size.into(),
                    font: p.medium,
                    vertical_alignment: alignment::Vertical::Center,
                    ..Text::default()
                });
            }
        }
    }

    fn take_lane_at(&self, index: usize, p: Point) -> Option<Hit<'_>> {
        let track = &self.project.tracks[index];
        let lanes = self.take_lanes(track);
        let row = (p.y - self.take_lane_top(index, 0)) / self.take_tall(track);
        if lanes == 0 || row < 0.0 || row >= lanes as f32 {
            return None;
        }
        let take = row as usize;
        let at = self.frames_at(p.x);
        let found = track.clips.iter().rev().find(|clip| clip.takes.len() > take && at >= clip.start as f64 && at < clip.end() as f64);
        Some(found.map_or(Hit::Lane, |clip| Hit::Take(clip, take)))
    }

    fn take_badges(&self, index: usize) -> Vec<(Rectangle, usize)> {
        let track = &self.project.tracks[index];
        if self.hidden(track) {
            return Vec::new();
        }
        let mut folders: Vec<&Clip> = track.clips.iter().filter(|clip| clip.has_takes() && !clip.is_notes()).collect();
        folders.sort_by_key(|clip| clip.start);
        let mut runs: Vec<(Frames, Frames, &Clip)> = Vec::new();
        for clip in folders {
            match runs.last_mut() {
                Some((_, end, last)) if follows_in_folder(last, clip) => {
                    *end = clip.end();
                    *last = clip;
                }
                _ => runs.push((clip.start, clip.end(), clip)),
            }
        }
        let top = self.track_top(index) + self.palette.clip_padding + BADGE_INSET;
        let height = (self.palette.clip_title_height - BADGE_INSET * 2.0).max(1.0);
        runs.into_iter()
            .filter_map(|(start, end, last)| {
                let count = last.takes.len();
                let left = self.x_of(start as f64).max(self.lanes_left());
                let right = self.x_of(end as f64).min(self.lanes_right());
                let width = badge_width(count);
                (right - left >= width + BADGE_ROOM * 2.0).then(|| (Rectangle::new(Point::new(left + BADGE_INSET, top), Size::new(width, height)), count))
            })
            .collect()
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
        if !self.in_header(p.x) {
            if let Some(target) = self.lane_edge_at(p) {
                return Hit::LaneEdge(target);
            }
        }
        if self.in_header(p.x) {
            if let Some(i) = self.track_at(p.y) {
                let track = &self.project.tracks[i];
                if let Some(take) = self.take_row_at(i, p.y) {
                    if self.take_remove_button(i, take).contains(p) {
                        return Hit::RemoveTake(track, take);
                    }
                    return Hit::TakeHeader(track, take);
                }
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
        if let Some(i) = self.track_at(p.y) {
            if self.take_badges(i).iter().any(|(badge, _)| badge.contains(p)) {
                return Hit::Folder(&self.project.tracks[i]);
            }
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
        if let Some(found) = self.take_lane_at(i, p) {
            return found;
        }
        if p.y < top + self.palette.clip_padding || p.y > top + self.clips_height(track) - self.palette.clip_padding {
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
        let free = !self.snap || state.modifiers.alt();
        let pace = if state.modifiers.alt() { crate::knob::FINE } else { 1.0 };

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
                    _ if self.meter_at(p).is_some() => self.meter_at(p).map(|track| Message::MeterMenu { track, at: Some(Point::new(bounds.x + p.x, bounds.y + p.y)) }),
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
                    (Tool::Razor, Hit::Clip(_) | Hit::Grip(..) | Hit::Lane | Hit::Take(..)) => {
                        state.drag = Some(Drag::Slice { from: p, to: p });
                        Some(Message::Refresh)
                    }
                    (Tool::Pencil | Tool::Comp, Hit::Take(clip, take)) => {
                        let Some(index) = self.project.tracks.iter().position(|t| t.clips.iter().any(|c| c.id == clip.id)) else {
                            return (Captured, None);
                        };
                        let band = (self.take_lane_top(index, take), self.take_tall(&self.project.tracks[index]));
                        let at = self.snap(self.frames_at(p.x), free);
                        state.drag = Some(Drag::Comp { clip: clip.id, track: self.project.tracks[index].id, take, from: at, to: at, band });
                        Some(Message::Refresh)
                    }
                    (Tool::Comp, Hit::Clip(clip) | Hit::Grip(clip, _)) if clip.has_takes() => {
                        let closed = self.project.track_of(clip.id).filter(|track| !track.takes_open);
                        closed.map(|track| Message::ToggleTakes(track.id))
                    }
                    (Tool::Comp, Hit::Clip(_) | Hit::Grip(..)) => None,
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
                    (_, Hit::LaneEdge(target)) => {
                        let top = self.lane_top_of(target).unwrap_or(p.y);
                        state.drag = Some(Drag::LaneResize { target, top });
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
                    (_, Hit::Fx(track)) if state.modifiers.command() => Some(Message::BypassChain(crate::stockwin::Spot::Track(track.id))),
                    (_, Hit::Fx(track)) => Some(Message::OpenChain(crate::stockwin::Spot::Track(track.id))),
                    (_, Hit::Pan(track)) => {
                        let again = state.last_pan_press.is_some_and(|(id, at)| id == track.id && at.elapsed() < DOUBLE_CLICK);
                        state.last_pan_press = Some((track.id, Instant::now()));
                        if again {
                            Some(Message::TrackPan(track.id, 0.0))
                        } else {
                            state.drag = Some(Drag::Pan { track: track.id, pull: EndlessDrag::start(p, true, self.pixels_per_point), pan_at_grab: track.pan });
                            None
                        }
                    }
                    (_, Hit::Arm(track)) => Some(Message::ToggleArm(track.id)),
                    (_, Hit::Route(track)) => Some(Message::OpenRouting(track.id)),
                    (_, Hit::Remove(track)) => Some(Message::RemoveTrack(track.id)),
                    (_, Hit::Collapse(track)) => Some(Message::ToggleCollapsed(track.id)),
                    (_, Hit::Folder(track)) => Some(Message::ToggleTakes(track.id)),
                    (_, Hit::TakeHeader(track, take)) => Some(Message::UseTakeOnTrack(track.id, take)),
                    (_, Hit::RemoveTake(track, take)) => Some(Message::DeleteTake(track.id, take)),
                    (_, Hit::Take(..)) => None,
                    (_, Hit::AddTrack) => Some(Message::AddTrack),
                    (_, Hit::Grip(clip, grip)) => {
                        let curve_at_grab = match grip {
                            Grip::ShapeOut => clip.fade_out.curve,
                            _ => clip.fade_in.curve,
                        };
                        let db_at_grab = (20.0 * clip.gain.max(1e-6).log10()).clamp(MIN_GAIN_DB, MAX_GAIN_DB);
                        let pull = EndlessDrag::start(p, !grip.moves_sideways(), self.pixels_per_point);
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
                    let inside = cursor.is_over(bounds);
                    let pointing = match anywhere.filter(|_| inside).map(|p| self.hit(p)) {
                        Some(Hit::Envelope(target, Some(which), ..)) => Some((target, which)),
                        _ => None,
                    };
                    if pointing != state.pointing {
                        state.pointing = pointing;
                        return (Ignored, Some(Message::Refresh));
                    }
                    let over = anywhere.filter(|_| inside).and_then(|p| self.tool_at(p));
                    if let Some(words) = hint_now(self.hinting, over, inside) {
                        return (Ignored, Some(Message::Hint(words)));
                    }
                    let aiming = self.tool == Tool::Razor && inside;
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
                        let value = self.value_at_y(shape, top, p.y);
                        if *which == usize::MAX {
                            *which = shape.points.iter().position(|point| point.at == at).unwrap_or(usize::MAX);
                        }
                        if *which == usize::MAX {
                            return (Captured, None);
                        }
                        let moving = *which;
                        let lowest = moving.checked_sub(1).and_then(|before| shape.points.get(before)).map_or(0, |before| before.at + 1);
                        let highest = shape.points.get(moving + 1).map_or(Frames::MAX, |after| after.at.saturating_sub(1));
                        let at = at.clamp(lowest, highest.max(lowest));
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
                        let Some(travel_up) = pull.moved(p, pace) else {
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
                        let Some(travel_up) = pull.moved(p, pace) else {
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
                    Drag::LaneResize { target, top } => {
                        let height = (p.y - *top).clamp(LANE_SHORTEST, LANE_TALLEST).round();
                        let changed = self.lane_tall(*target) != height;
                        (Captured, changed.then_some(Message::ResizeLane { target: *target, height }))
                    }
                    Drag::Resize { track, top } => {
                        let below = self.project.track(*track).map_or(0.0, |t| self.height_of(t) - self.clips_height(t));
                        let height = (p.y - *top - below).clamp(theme::MIN_TRACK_HEIGHT, theme::MAX_TRACK_HEIGHT).round();
                        let current = self.project.track(*track).map(|t| self.clips_height(t));
                        let changed = current.is_some_and(|h| h != height);
                        (Captured, changed.then_some(Message::ResizeTrack { track: *track, height }))
                    }
                }
            }
            canvas::Event::Mouse(mouse::Event::CursorLeft) => (Ignored, hint_now(self.hinting, None, false).map(Message::Hint)),
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => match state.drag.take() {
                Some(Drag::Reorder { track, at: Some(to), .. }) => {
                    (Captured, Some(Message::MoveTrack { track, to, parent: self.folder_at(to, track) }))
                }
                Some(Drag::Comp { clip, track, take, from, to, .. }) => {
                    let message = match from == to {
                        true => Message::UseTake(clip, take),
                        false => Message::Comp { track, take, from: from.min(to), to: from.max(to) },
                    };
                    (Captured, Some(message))
                }
                Some(Drag::Range { anchor, moving: false, .. }) => (Captured, Some(Message::RulerClicked(anchor))),
                Some(Drag::Range { moving: true, .. }) => (Captured, Some(Message::RangeMade)),
                Some(
                    Drag::Clip { moving: true, .. }
                    | Drag::Grip { .. }
                    | Drag::Paint { .. }
                    | Drag::Trim { .. }
                    | Drag::Stretch { .. }
                    | Drag::Pan { .. }
                    | Drag::LaneResize { .. },
                ) => {
                    (Captured, Some(Message::DragEnd))
                }
                Some(Drag::Point { .. }) => (Captured, Some(Message::PointLetGo)),
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
        self.at.set(bounds);
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
            frame.with_clip(headers, |frame| {
                self.draw_headers(frame, headers.size());
                self.draw_take_headers(frame, headers.size());
            });

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
        let free = !self.snap || state.modifiers.alt();
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
            let reading = self.track_meters.get(i).copied().unwrap_or_default();
            let level = reading.shown;
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
            let hold_db = 20.0 * reading.hold.max(1e-6).log10();
            if hold_db > METER_FLOOR_DB {
                let colour = crate::metering::hold_colour(p, reading.hold, hold_db, zones);
                let at = (bar.x + along(hold_db) - HOLD_LINE / 2.0).clamp(bar.x, bar.x + bar.width - HOLD_LINE);
                overlay.fill_rectangle(Point::new(at, bar.y), Size::new(HOLD_LINE, bar.height), colour);
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
        if let Some((target, at, value, spot)) = self.point_tip(state) {
            self.draw_point_tip(&mut overlay, target, at, value, spot);
        }
        vec![content, overlay.into_geometry()]
    }

    fn mouse_interaction(&self, state: &Interaction, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        match state.drag {
            Some(Drag::Clip { moving: true, .. }) => return mouse::Interaction::Grabbing,
            Some(Drag::Resize { .. }) => return mouse::Interaction::ResizingVertically,
            Some(Drag::LaneResize { .. }) => return mouse::Interaction::ResizingVertically,
            Some(Drag::Reorder { at: Some(_), .. }) => return mouse::Interaction::Grabbing,
            Some(Drag::Grip { grip, .. }) => return grip.pointer(),
            Some(Drag::Trim { .. }) => return mouse::Interaction::ResizingHorizontally,
            Some(Drag::Stretch { .. }) => return mouse::Interaction::ResizingHorizontally,
            Some(Drag::Pan { .. }) => return mouse::Interaction::ResizingVertically,
            _ => {}
        }
        match (self.tool, cursor.position_in(bounds).map(|p| self.hit(p))) {
            (_, Some(Hit::Resize(_))) => mouse::Interaction::ResizingVertically,
            (_, Some(Hit::LaneEdge(_))) => mouse::Interaction::ResizingVertically,
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
            let badges = self.take_badges(i);
            for clip in &track.clips {
                let colour = if clip.muted { p.text_faint } else { track_colour };
                self.draw_clip(frame, size, clip, top, clips_height, colour, &badges);
            }
            self.draw_take_lanes(frame, size, track, top + clips_height, track_colour);
            for (badge, count) in badges {
                self.draw_badge(frame, badge, count, track.takes_open, track_colour);
            }
            let mut lane_top = top + self.envelopes_top(track);
            for shape in self.lanes_of(track) {
                self.draw_envelope(frame, size, shape, lane_top);
                lane_top += self.lane_tall(shape.target);
            }
        }
    }

    fn envelope_at(&self, index: usize, p: Point) -> Option<Hit<'_>> {
        let track = &self.project.tracks[index];
        let mut top = self.track_top(index) + self.envelopes_top(track);
        for shape in self.lanes_of(track) {
            let tall = self.lane_tall(shape.target);
            if p.y >= top && p.y < top + tall {
                let value = self.value_at_y(shape, top, p.y);
                let offset = self.lane_offset(shape.target);
                let at = (self.frames_at(p.x).max(0.0) as Frames).saturating_sub(offset);
                let near = shape.points.iter().position(|point| {
                    let seen = Point::new(self.x_of((point.at + offset) as f64), self.y_of_value(shape, top, point.value));
                    seen.distance(p) <= HANDLE_REACH
                });
                return Some(Hit::Envelope(shape.target, near, at, value));
            }
            top += tall;
        }
        None
    }

    fn lane_top_of(&self, target: loupe_engine::Target) -> Option<f32> {
        for (i, track) in self.project.tracks.iter().enumerate() {
            let mut top = self.track_top(i) + self.envelopes_top(track);
            for shape in self.lanes_of(track) {
                if shape.target == target {
                    return Some(top);
                }
                top += self.lane_tall(shape.target);
            }
        }
        None
    }

    fn lane_label(&self, target: loupe_engine::Target) -> String {
        crate::readings::lane_label(self.project, self.knobs, target)
    }

    fn lane_tall(&self, target: loupe_engine::Target) -> f32 {
        self.project.envelope(target).and_then(|shape| shape.lane_height).unwrap_or(LANE_HEIGHT)
    }

    fn lane_span(&self, shape: &loupe_engine::Envelope) -> f32 {
        (self.lane_tall(shape.target) - LANE_INSET * 2.0).max(1.0)
    }

    fn value_at_y(&self, shape: &loupe_engine::Envelope, top: f32, y: f32) -> f32 {
        let reach = (shape.highest - shape.lowest).max(f32::EPSILON);
        shape.lowest + reach * (1.0 - ((y - top - LANE_INSET) / self.lane_span(shape)).clamp(0.0, 1.0))
    }

    fn y_of_value(&self, shape: &loupe_engine::Envelope, top: f32, value: f32) -> f32 {
        let reach = (shape.highest - shape.lowest).max(f32::EPSILON);
        top + LANE_INSET + self.lane_span(shape) * (1.0 - (value - shape.lowest) / reach)
    }

    fn point_tip(&self, state: &Interaction) -> Option<(loupe_engine::Target, Frames, f32, Point)> {
        let (target, at, value) = match &state.drag {
            Some(Drag::Point { target, which }) => {
                let point = self.project.envelope(*target)?.points.get(*which)?;
                (*target, point.at, point.value)
            }
            None => {
                let (target, which) = state.pointing?;
                let point = self.project.envelope(target)?.points.get(which)?;
                (target, point.at, point.value)
            }
            _ => return None,
        };
        let shape = self.project.envelope(target)?;
        let top = self.lane_top_of(target)?;
        let value = value.clamp(shape.lowest, shape.highest);
        let spot = Point::new(self.x_of((at + self.lane_offset(target)) as f64), self.y_of_value(shape, top, value));
        Some((target, at, value, spot))
    }

    fn draw_point_tip(&self, overlay: &mut Frame, target: loupe_engine::Target, at: Frames, value: f32, spot: Point) {
        let p = self.palette;
        if spot.x < self.lanes_left() || spot.x > self.lanes_right() {
            return;
        }
        overlay.stroke(&Path::circle(spot, 6.0), Stroke::default().with_color(p.accent).with_width(1.5));
        let words = crate::readings::point_tip(self.project, target, at, value);
        let size = 12.0;
        let wide = words.chars().count() as f32 * size * 0.58 + 16.0;
        let tall = 24.0;
        let lane_top = self.lane_top_of(target).unwrap_or(spot.y);
        let mut left = spot.x + 12.0;
        if left + wide > self.lanes_right() {
            left = spot.x - 12.0 - wide;
        }
        let mut top = spot.y - tall - 8.0;
        if top < lane_top.max(self.lanes_top()) {
            top = spot.y + 10.0;
        }
        let corner = Point::new(left.round(), top.round());
        overlay.fill(&Path::rounded_rectangle(corner, Size::new(wide, tall), 5.0.into()), p.panel);
        overlay.stroke(&Path::rounded_rectangle(corner, Size::new(wide, tall), 5.0.into()), Stroke::default().with_color(p.line).with_width(1.0));
        overlay.fill_text(Text {
            content: words,
            position: Point::new(corner.x + 8.0, corner.y + tall / 2.0),
            color: p.text,
            size: size.into(),
            font: p.medium,
            vertical_alignment: iced::alignment::Vertical::Center,
            ..Text::default()
        });
    }

    fn lane_edge_at(&self, p: Point) -> Option<loupe_engine::Target> {
        for (i, track) in self.project.tracks.iter().enumerate() {
            let mut top = self.track_top(i) + self.envelopes_top(track);
            for shape in self.lanes_of(track) {
                top += self.lane_tall(shape.target);
                if (p.y - top).abs() <= LANE_EDGE_REACH {
                    return Some(shape.target);
                }
            }
        }
        None
    }

    fn lane_offset(&self, target: loupe_engine::Target) -> Frames {
        match target.on_clip().and_then(|id| self.project.clip(id)) {
            Some(clip) => clip.start,
            None => 0,
        }
    }

    fn draw_envelope(&self, frame: &mut Frame, size: Size, shape: &loupe_engine::Envelope, top: f32) {
        let p = self.palette;
        let tall = self.lane_tall(shape.target);
        frame.fill_rectangle(Point::new(0.0, top), Size::new(size.width, tall), theme::mix(p.background, p.panel, 0.5));
        frame.fill_rectangle(Point::new(0.0, top + tall - 1.0), Size::new(size.width, 1.0), p.line);
        let y_of = |value: f32| self.y_of_value(shape, top, value);
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

    #[allow(clippy::too_many_arguments)]
    fn draw_clip(&self, frame: &mut Frame, size: Size, clip: &Clip, lane_top: f32, lane_height: f32, colour: Color, badges: &[(Rectangle, usize)]) {
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
                None => self.draw_waveform(frame, clip, clip.offset, left, shown_left.max(0.0), shown_right.min(size.width), wave_top, wave_height, colour),
            }
        }

        if clip.notes.is_none() {
            self.draw_gain(frame, clip, shown_left.max(0.0), shown_right.min(size.width), wave_top, wave_height, colour);
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
        let continues_a_folder = self.project.track_of(clip.id).is_some_and(|track| track.clips.iter().any(|earlier| follows_in_folder(earlier, clip)));
        let name_from = left.max(0.0);
        let under_badge = badges.iter().find(|(badge, _)| (name_from - 1.0..right).contains(&(badge.x - self.lanes_left() - BADGE_INSET)));
        let name_inset = under_badge.map_or(0.0, |(badge, _)| badge.width + BADGE_INSET);
        let title = match (clip.is_stretched(), clip.waiting_for_stretch(), continues_a_folder) {
            (_, _, true) => format!("take {}", clip.take + 1),
            (false, _, false) => clip.called().to_string(),
            (true, false, false) => format!("{}  {:.0}%", clip.called(), clip.stretch * 100.0),
            (true, true, false) => format!("{}  {:.0}%  stretching…", clip.called(), clip.stretch * 100.0),
        } + &if clip.has_takes() && !continues_a_folder { format!("  ·  take {}", clip.take + 1) } else { String::new() };
        let room = right.min(size.width) - name_from - name_inset - 12.0;
        let title = match (clip.has_takes(), (room / NAME_LETTER).max(0.0) as usize) {
            (false, _) => title,
            (true, 0) => String::new(),
            (true, most) => shorten(&title, most),
        };
        if let Some(visible) = title_on_canvas.intersection(&lanes_on_canvas) {
            frame.with_clip(visible, |name_region| {
                name_region.fill_text(Text {
                    content: title,
                    position: Point::new(
                        self.lanes_left() + name_from + name_inset + 8.0 - visible.x,
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

    fn draw_take_lanes(&self, frame: &mut Frame, size: Size, track: &Track, lanes_top: f32, track_colour: Color) {
        let p = self.palette;
        let tall = self.take_tall(track);
        for take in 0..self.take_lanes(track) {
            let top = lanes_top + take as f32 * tall;
            if top > size.height || top + tall < 0.0 {
                continue;
            }
            frame.fill_rectangle(Point::new(0.0, top), Size::new(size.width, tall), theme::mix(p.background, p.panel, 0.5));
            frame.fill_rectangle(Point::new(0.0, top + tall - 1.0), Size::new(size.width, 1.0), p.line);
            let box_top = top + TAKE_LANE_INSET;
            let box_height = tall - TAKE_LANE_INSET * 2.0;
            for clip in track.clips.iter().filter(|clip| clip.takes.len() > take && !clip.is_notes()) {
                let left = self.x_of(clip.start as f64) - self.lanes_left();
                let right = self.x_of(clip.end() as f64) - self.lanes_left();
                if right < 0.0 || left > size.width {
                    continue;
                }
                let (from_x, to_x) = (left.max(0.0), right.min(size.width));
                let colour = if clip.muted { p.text_faint } else { track_colour };
                let chosen = take == clip.take;
                let span = Size::new((to_x - from_x).max(1.0), box_height);
                frame.fill_rectangle(Point::new(from_x, box_top), span, theme::mix(p.background, colour, if chosen { 0.3 } else { 0.1 }));
                if chosen {
                    frame.stroke(&Path::rectangle(Point::new(from_x, box_top), span), Stroke::default().with_color(colour).with_width(1.0));
                }
                if let Some(offset) = clip.take_offset(take) {
                    let tone = if chosen { colour } else { theme::mix(p.background, colour, 0.45) };
                    self.draw_waveform(frame, clip, offset, left, from_x, to_x, box_top + 1.0, box_height - 2.0, tone);
                }
            }
        }
    }

    fn draw_badge(&self, frame: &mut Frame, badge: Rectangle, count: usize, open: bool, colour: Color) {
        let p = self.palette;
        let at = Point::new(badge.x - self.lanes_left(), badge.y - self.lanes_top());
        let body = Path::new(|b| b.rounded_rectangle(at, badge.size(), (badge.height / 2.0).into()));
        frame.fill(&body, theme::mix(p.background, colour, 0.55));
        frame.stroke(&body, Stroke::default().with_color(theme::mix(p.background, colour, 0.8)).with_width(1.0));
        let middle = at.y + badge.height / 2.0;
        let x = at.x + BADGE_PAD;
        let reach = BADGE_CHEVRON / 2.0;
        let chevron = Path::new(|b| {
            if open {
                b.move_to(Point::new(x, middle - reach * 0.6));
                b.line_to(Point::new(x + BADGE_CHEVRON, middle - reach * 0.6));
                b.line_to(Point::new(x + reach, middle + reach * 0.8));
            } else {
                b.move_to(Point::new(x + reach * 0.4, middle - reach));
                b.line_to(Point::new(x + reach * 0.4 + reach * 1.4, middle));
                b.line_to(Point::new(x + reach * 0.4, middle + reach));
            }
            b.close();
        });
        frame.fill(&chevron, p.text);
        frame.fill_text(Text {
            content: badge_label(count),
            position: Point::new(x + BADGE_CHEVRON + BADGE_PAD / 2.0, middle),
            color: p.text,
            size: 10.5.into(),
            font: p.medium,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_gain(
        &self,
        frame: &mut Frame,
        clip: &Clip,
        from_x: f32,
        to_x: f32,
        wave_top: f32,
        wave_height: f32,
        colour: Color,
    ) {
        let seen = to_x - from_x;
        if !room_for_gain_line(seen, wave_height) {
            return;
        }
        let p = self.palette;
        let db = 20.0 * clip.gain.max(1e-6).log10();
        let pushed = off_unity(db);
        let y = gain_y(db, wave_top, wave_height);
        let ink = theme::mix(colour, p.text, if pushed { 0.8 } else { 0.3 });
        frame.fill_rectangle(Point::new(from_x, y.round()), Size::new(seen, 1.0), ink);
        if !pushed || !room_for_gain_figure(seen, wave_height, db) {
            return;
        }
        let width = gain_figure_width(db);
        let right = to_x - GAIN_FIGURE_INSET;
        let middle = gain_figure_y(y, wave_top, wave_height);
        frame.fill_rectangle(
            Point::new(right - width, middle - GAIN_PLATE_HEIGHT / 2.0),
            Size::new(width, GAIN_PLATE_HEIGHT),
            theme::alpha(p.background, 0.72),
        );
        frame.fill_text(Text {
            content: format!("{db:+.1} dB"),
            position: Point::new(right - GAIN_FIGURE_PAD, middle),
            color: p.text,
            size: GAIN_FIGURE_SIZE.into(),
            font: p.mono,
            horizontal_alignment: alignment::Horizontal::Right,
            vertical_alignment: alignment::Vertical::Center,
            ..Text::default()
        });
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
        let reach = height / 2.0 * wave_boost(clip);
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
            let (hi, lo) = (hi.max(top), lo.min(top + height));
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
            frame.fill_rectangle(
                Point::new(0.0, top),
                Size::new(COLOUR_EDGE, height - 1.0),
                theme::alpha(self.colour_of(i), 0.7),
            );
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
                color: if (!track.fx_on || !self.project.all_fx_on) && !track.fx.is_empty() {
                    p.danger
                } else if loaded > 0 {
                    p.accent
                } else {
                    p.text_dim
                },
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

fn follows_in_folder(earlier: &Clip, later: &Clip) -> bool {
    earlier.has_takes() && earlier.end() == later.start && Arc::ptr_eq(&earlier.source, &later.source) && earlier.takes.len() == later.takes.len()
}

fn open_take_lanes(track: &Track) -> usize {
    if !track.takes_open {
        return 0;
    }
    track.clips.iter().filter(|clip| clip.has_takes() && !clip.is_notes()).map(|clip| clip.takes.len()).max().unwrap_or(0)
}

fn badge_label(count: usize) -> String {
    format!("{count} takes")
}

fn badge_width(count: usize) -> f32 {
    BADGE_PAD + BADGE_CHEVRON + BADGE_PAD / 2.0 + badge_label(count).chars().count() as f32 * BADGE_LETTER + BADGE_PAD
}

fn off_unity(db: f32) -> bool {
    db.abs() > GAIN_UNITY_SLACK_DB
}

fn gain_y(db: f32, top: f32, height: f32) -> f32 {
    let along = (db / GAIN_LINE_SPAN_DB).clamp(-1.0, 1.0);
    (top + height * (1.0 - along) / 2.0).clamp(top + 0.5, top + height - 1.5)
}

fn gain_figure_width(db: f32) -> f32 {
    let letters = if db.abs() >= 10.0 { 8.0 } else { 7.0 };
    letters * GAIN_FIGURE_SIZE * 0.62 + GAIN_FIGURE_PAD * 2.0
}

fn gain_figure_y(line: f32, top: f32, height: f32) -> f32 {
    let half = GAIN_PLATE_HEIGHT / 2.0;
    let above = line - GAIN_FIGURE_LIFT;
    if above - half >= top {
        above
    } else {
        (line + GAIN_FIGURE_LIFT).min(top + height - half)
    }
}

fn room_for_gain_line(seen_width: f32, wave_height: f32) -> bool {
    seen_width >= MIN_SEEN_PX_FOR_GAIN_LINE && wave_height >= MIN_WAVE_PX_FOR_GAIN_LINE
}

fn room_for_gain_figure(seen_width: f32, wave_height: f32, db: f32) -> bool {
    wave_height >= MIN_WAVE_PX_FOR_GAIN_FIGURE && seen_width >= gain_figure_width(db) + GAIN_FIGURE_INSET * 2.0
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

#[cfg(test)]
mod hiding {
    use super::*;
    use loupe_engine::{Command, Outcome};

    fn track_called(project: &Project, name: &str) -> Track {
        project.tracks.iter().find(|track| track.name == name).cloned().expect("no such track")
    }

    #[test]
    fn hiding_a_parent_leaves_its_children_in_the_arrangement() {
        let mut project = Project::new(48_000);
        let Ok(Outcome::Track(chain_b)) = project.apply(Command::AddTrack { name: "Vox Chain B".into() }) else { panic!() };
        let Ok(Outcome::Track(chain)) = project.apply(Command::AddTrack { name: "Vox Chain".into() }) else { panic!() };
        let Ok(Outcome::Track(vox)) = project.apply(Command::AddTrack { name: "Vox 1".into() }) else { panic!() };
        project.apply(Command::SetTrackParent { track: chain, parent: Some(chain_b) }).unwrap();
        project.apply(Command::SetTrackParent { track: vox, parent: Some(chain) }).unwrap();
        project.apply(Command::SetTrackHidden { track: chain_b, hidden: true }).unwrap();

        assert!(out_of_sight(&project, &track_called(&project, "Vox Chain B")), "the hidden track itself goes");
        assert!(!out_of_sight(&project, &track_called(&project, "Vox Chain")), "its child stays");
        assert!(!out_of_sight(&project, &track_called(&project, "Vox 1")), "its grandchild stays");
    }

    #[test]
    fn collapsing_a_folder_still_tucks_its_children_away() {
        let mut project = Project::new(48_000);
        let Ok(Outcome::Track(folder)) = project.apply(Command::AddTrack { name: "Vox Chain".into() }) else { panic!() };
        let Ok(Outcome::Track(vox)) = project.apply(Command::AddTrack { name: "Vox 1".into() }) else { panic!() };
        project.apply(Command::SetTrackParent { track: vox, parent: Some(folder) }).unwrap();
        project.apply(Command::ToggleCollapsed(folder)).unwrap();

        assert!(!out_of_sight(&project, &track_called(&project, "Vox Chain")));
        assert!(out_of_sight(&project, &track_called(&project, "Vox 1")));
    }
}

#[cfg(test)]
mod hints {
    use super::*;

    const MIXER_HINT: &str = "Show or hide the mixer";

    #[test]
    fn a_hint_from_elsewhere_survives_the_pointer_moving_outside_the_arrangement() {
        let resting = Some(MIXER_HINT);
        assert_eq!(hint_now(resting, None, false), None, "the first move after the hint appeared must leave it alone");
        assert_eq!(hint_now(resting, None, false), None, "and every move after that");
    }

    #[test]
    fn a_tool_hint_stays_while_the_pointer_rests_on_the_button() {
        let razor = Some(Tool::Razor.words());
        assert_eq!(hint_now(None, razor, true), Some(razor), "moving onto the button shows it");
        assert_eq!(hint_now(razor, razor, true), None, "resting on it says nothing new");
    }

    #[test]
    fn a_tool_hint_goes_when_the_pointer_leaves_the_button_or_the_window() {
        let razor = Some(Tool::Razor.words());
        assert_eq!(hint_now(razor, None, true), Some(None), "sliding off the button inside the arrangement");
        assert_eq!(hint_now(razor, None, false), Some(None), "leaving the arrangement altogether");
    }
}

#[cfg(test)]
mod gain {
    use super::*;

    const WAVE_TOP: f32 = 100.0;
    const WAVE_HEIGHT: f32 = 60.0;

    fn y(db: f32) -> f32 {
        gain_y(db, WAVE_TOP, WAVE_HEIGHT)
    }

    #[test]
    fn unity_sits_on_the_centre_of_the_waveform() {
        assert_eq!(y(0.0), WAVE_TOP + WAVE_HEIGHT / 2.0);
    }

    #[test]
    fn louder_reads_higher_and_quieter_reads_lower() {
        assert!(y(6.0) < y(0.0), "a boosted clip draws above the middle");
        assert!(y(-6.0) > y(0.0), "a pulled clip draws below the middle");
        assert_eq!(y(6.0) - y(0.0), y(0.0) - y(-6.0), "the mapping is even either side of unity");
    }

    #[test]
    fn the_span_clamps_at_twelve_decibels_either_way() {
        assert_eq!(y(MAX_GAIN_DB), y(GAIN_LINE_SPAN_DB));
        assert_eq!(y(MIN_GAIN_DB), y(-GAIN_LINE_SPAN_DB));
        assert_eq!(y(-18.0), y(-GAIN_LINE_SPAN_DB), "anything under the span rests on the floor");
    }

    #[test]
    fn the_line_stays_inside_the_waveform_band() {
        for step in 0..=72 {
            let db = MIN_GAIN_DB + step as f32 * 0.5;
            let at = y(db);
            assert!(at >= WAVE_TOP, "{db} dB drew above the band");
            assert!(at + 1.0 <= WAVE_TOP + WAVE_HEIGHT, "{db} dB drew below the band");
        }
    }

    #[test]
    fn a_thin_band_still_holds_the_line() {
        let thin = MIN_WAVE_PX_FOR_GAIN_LINE;
        for db in [MIN_GAIN_DB, -6.0, 0.0, MAX_GAIN_DB] {
            let at = gain_y(db, WAVE_TOP, thin);
            assert!(at >= WAVE_TOP && at + 1.0 <= WAVE_TOP + thin, "{db} dB left the thin band");
        }
    }

    #[test]
    fn only_a_clip_away_from_unity_counts_as_pushed() {
        assert!(!off_unity(0.0), "untouched gain is unity");
        assert!(!off_unity(20.0 * 1.0_f32.log10()), "unity gain works out as unity");
        assert!(!off_unity(0.02), "float drift is not a push");
        assert!(off_unity(GAIN_DB_PER_PX), "the smallest drag step is a push");
        assert!(off_unity(-GAIN_DB_PER_PX));
    }

    #[test]
    fn a_clip_scrolled_almost_off_screen_drops_the_line() {
        assert!(!room_for_gain_line(1.0, WAVE_HEIGHT), "a sliver of a clip draws nothing");
        assert!(room_for_gain_line(MIN_SEEN_PX_FOR_GAIN_LINE, WAVE_HEIGHT));
    }

    #[test]
    fn a_short_track_drops_the_line() {
        assert!(!room_for_gain_line(400.0, 6.0));
        assert!(room_for_gain_line(400.0, MIN_WAVE_PX_FOR_GAIN_LINE));
    }

    #[test]
    fn the_figure_goes_before_the_line_does() {
        let narrow = 40.0;
        let squat = 14.0;
        assert!(room_for_gain_line(narrow, WAVE_HEIGHT) && !room_for_gain_figure(narrow, WAVE_HEIGHT, 3.0), "a narrow clip keeps the line only");
        assert!(room_for_gain_line(400.0, squat) && !room_for_gain_figure(400.0, squat, 3.0), "a squat clip keeps the line only");
    }

    #[test]
    fn a_wide_enough_clip_takes_the_figure() {
        assert!(room_for_gain_figure(gain_figure_width(3.0) + GAIN_FIGURE_INSET * 2.0, MIN_WAVE_PX_FOR_GAIN_FIGURE, 3.0));
        assert!(gain_figure_width(11.4) > gain_figure_width(3.0), "two digits ask for more room than one");
        assert!(!room_for_gain_figure(gain_figure_width(3.0) + GAIN_FIGURE_INSET * 2.0, MIN_WAVE_PX_FOR_GAIN_FIGURE, 11.4), "and a clip that only fits one digit turns the wider figure away");
    }

    #[test]
    fn the_figure_keeps_clear_of_the_line_without_leaving_the_band() {
        for height in [MIN_WAVE_PX_FOR_GAIN_FIGURE, 40.0, WAVE_HEIGHT, 300.0] {
            for step in 0..=72 {
                let db = MIN_GAIN_DB + step as f32 * 0.5;
                let line = gain_y(db, WAVE_TOP, height);
                let middle = gain_figure_y(line, WAVE_TOP, height);
                assert!((middle - line).abs() >= GAIN_FIGURE_LIFT - 0.001, "{db} dB in {height} px wrote the figure over its own line");
                assert!(middle - GAIN_PLATE_HEIGHT / 2.0 >= WAVE_TOP, "{db} dB in {height} px wrote above the band");
                assert!(middle + GAIN_PLATE_HEIGHT / 2.0 <= WAVE_TOP + height, "{db} dB in {height} px wrote below the band");
            }
        }
    }
}

impl crate::App {
    pub(crate) fn dropped_onto(&self, in_window: iced::Point) -> Option<(TrackId, Frames)> {
        let canvas = self.canvas_at.get();
        if canvas.width <= 0.0 || canvas.height <= 0.0 {
            return None;
        }
        let at = iced::Point::new(in_window.x - canvas.x, in_window.y - canvas.y);
        let top = self.palette.scrollbar_height + self.palette.ruler_height;
        let (left, right) = match self.palette.headers {
            Side::Left => (self.palette.header_width, canvas.width),
            Side::Right => (0.0, (canvas.width - self.palette.header_width).max(0.0)),
        };
        if at.y < top || at.y >= canvas.height || at.x < left || at.x >= right {
            return None;
        }
        let mut y = top - self.view.scroll_y;
        for track in &self.project.tracks {
            if out_of_sight(&self.project, track) {
                continue;
            }
            let lanes = self
                .project
                .envelopes
                .iter()
                .filter(|shape| shape.lane_open && (shape.target.on_track() == Some(track.id) || shape.target.on_clip().is_some_and(|clip| track.clips.iter().any(|kept| kept.id == clip))))
                .map(|shape| shape.lane_height.unwrap_or(LANE_HEIGHT))
                .sum::<f32>();
            let main = self.heights.get(&track.id).copied().unwrap_or(self.palette.track_height);
            let tall = main + open_take_lanes(track) as f32 * main + lanes;
            if at.y >= y && at.y < y + tall {
                let seconds = (at.x - left) as f64 / self.view.zoom + self.view.scroll;
                let start = (seconds * self.project.rate as f64).max(0.0) as Frames;
                return Some((track.id, start));
            }
            y += tall;
        }
        None
    }

    pub(crate) fn below(&self, track: TrackId, down: usize) -> Option<TrackId> {
        self.project
            .tracks
            .iter()
            .filter(|found| !out_of_sight(&self.project, found))
            .skip_while(|found| found.id != track)
            .nth(down)
            .map(|found| found.id)
    }
}

#[cfg(test)]
mod dropping {
    use super::*;
    use crate::App;
    use loupe_engine::{Command, Outcome};

    const CANVAS: Rectangle = Rectangle { x: 0.0, y: 100.0, width: 1200.0, height: 700.0 };
    const TOP: f32 = theme::NEUTRAL.scrollbar_height + theme::NEUTRAL.ruler_height;
    const TALL: f32 = theme::NEUTRAL.track_height;
    const LEFT: f32 = theme::NEUTRAL.header_width;

    fn a_song_with(names: &[&str]) -> (App, Vec<TrackId>) {
        let mut app = App::bare();
        app.canvas_at.set(CANVAS);
        app.view = View { zoom: 100.0, scroll: 0.0, scroll_y: 0.0 };
        let made = names
            .iter()
            .map(|name| match app.project.apply(Command::AddTrack { name: (*name).into() }) {
                Ok(Outcome::Track(track)) => track,
                _ => panic!("no track"),
            })
            .collect();
        (app, made)
    }

    fn in_window(x: f32, y: f32) -> Point {
        Point::new(CANVAS.x + x, CANVAS.y + y)
    }

    fn row(index: usize) -> f32 {
        TOP + TALL * index as f32 + TALL / 2.0
    }

    #[test]
    fn a_drop_on_the_second_track_lands_on_that_track() {
        let (app, tracks) = a_song_with(&["Drums", "Bass", "Vox"]);
        let (track, _) = app.dropped_onto(in_window(LEFT + 10.0, row(1))).expect("a track");
        assert_eq!(track, tracks[1]);
    }

    #[test]
    fn a_drop_takes_the_frame_under_the_pointer() {
        let (app, _) = a_song_with(&["Drums"]);
        let rate = app.project.rate as Frames;
        let (_, start) = app.dropped_onto(in_window(LEFT + 200.0, row(0))).expect("a track");
        assert_eq!(start, rate * 2);
    }

    #[test]
    fn the_window_offset_comes_off_before_the_hit_test() {
        let (app, tracks) = a_song_with(&["Drums", "Bass"]);
        assert_eq!(app.dropped_onto(in_window(LEFT + 10.0, row(0))).map(|(track, _)| track), Some(tracks[0]));
        assert_eq!(
            app.dropped_onto(Point::new(CANVAS.x + LEFT + 10.0, row(0))),
            None,
            "a point read as if the canvas sat at the window origin lands on the ruler"
        );
    }

    #[test]
    fn a_drop_below_the_last_track_asks_for_a_new_one() {
        let (app, _) = a_song_with(&["Drums", "Bass"]);
        assert_eq!(app.dropped_onto(in_window(LEFT + 10.0, row(2))), None);
    }

    #[test]
    fn a_drop_on_the_ruler_or_the_headers_asks_for_a_new_track() {
        let (app, _) = a_song_with(&["Drums"]);
        assert_eq!(app.dropped_onto(in_window(LEFT + 10.0, TOP - 1.0)), None, "the ruler");
        assert_eq!(app.dropped_onto(in_window(LEFT - 1.0, row(0))), None, "the headers");
        assert_eq!(app.dropped_onto(in_window(CANVAS.width + 10.0, row(0))), None, "past the right edge");
        assert_eq!(app.dropped_onto(in_window(LEFT + 10.0, CANVAS.height + 10.0)), None, "below the canvas");
    }

    #[test]
    fn headers_on_the_right_leave_the_tracks_at_the_left_edge() {
        let (mut app, tracks) = a_song_with(&["Drums", "Bass"]);
        app.palette.headers = Side::Right;
        let rate = app.project.rate as Frames;
        let (track, start) = app.dropped_onto(in_window(300.0, row(1))).expect("a track");
        assert_eq!(track, tracks[1]);
        assert_eq!(start, rate * 3);
        assert_eq!(app.dropped_onto(in_window(CANVAS.width - LEFT + 10.0, row(1))), None, "the headers on the right");
    }

    #[test]
    fn a_scrolled_arrangement_still_finds_the_track_under_the_pointer() {
        let (mut app, tracks) = a_song_with(&["Drums", "Bass", "Vox"]);
        app.view = View { zoom: 100.0, scroll: 4.0, scroll_y: TALL };
        let rate = app.project.rate as Frames;
        let (track, start) = app.dropped_onto(in_window(LEFT + 100.0, row(0))).expect("a track");
        assert_eq!(track, tracks[1], "scrolling down a track brings the second one to the top row");
        assert_eq!(start, rate * 5, "the horizontal scroll counts too");
    }

    #[test]
    fn a_taller_track_claims_its_extra_room() {
        let (mut app, tracks) = a_song_with(&["Drums", "Bass"]);
        app.heights.insert(tracks[0], 200.0);
        assert_eq!(app.dropped_onto(in_window(LEFT + 10.0, TOP + 150.0)).map(|(track, _)| track), Some(tracks[0]));
        assert_eq!(app.dropped_onto(in_window(LEFT + 10.0, TOP + 240.0)).map(|(track, _)| track), Some(tracks[1]));
    }

    #[test]
    fn a_collapsed_folder_hands_the_row_below_to_the_next_track_in_sight() {
        let (mut app, tracks) = a_song_with(&["Vox Chain", "Vox 1", "Drums"]);
        app.project.apply(Command::SetTrackParent { track: tracks[1], parent: Some(tracks[0]) }).unwrap();
        app.project.apply(Command::ToggleCollapsed(tracks[0])).unwrap();
        assert_eq!(app.dropped_onto(in_window(LEFT + 10.0, row(1))).map(|(track, _)| track), Some(tracks[2]));
    }

    #[test]
    fn a_hidden_track_takes_up_no_room() {
        let (mut app, tracks) = a_song_with(&["Drums", "Bass", "Vox"]);
        app.project.apply(Command::SetTrackHidden { track: tracks[1], hidden: true }).unwrap();
        assert_eq!(app.dropped_onto(in_window(LEFT + 10.0, row(1))).map(|(track, _)| track), Some(tracks[2]));
        assert_eq!(app.dropped_onto(in_window(LEFT + 10.0, row(2))), None);
    }

    #[test]
    fn an_unmeasured_canvas_takes_no_drops() {
        let (mut app, _) = a_song_with(&["Drums"]);
        app.canvas_at.set(Rectangle { x: 0.0, y: 0.0, width: 0.0, height: 0.0 });
        assert_eq!(app.dropped_onto(in_window(LEFT + 10.0, row(0))), None);
    }

    #[test]
    fn the_next_file_in_a_drop_goes_on_the_track_below() {
        let (app, tracks) = a_song_with(&["Drums", "Bass", "Vox"]);
        assert_eq!(app.below(tracks[0], 0), Some(tracks[0]));
        assert_eq!(app.below(tracks[0], 1), Some(tracks[1]));
        assert_eq!(app.below(tracks[0], 2), Some(tracks[2]));
        assert_eq!(app.below(tracks[0], 3), None, "once the tracks run out the drop makes a new one");
    }

    #[test]
    fn going_down_skips_the_tracks_out_of_sight() {
        let (mut app, tracks) = a_song_with(&["Drums", "Bass", "Vox"]);
        app.project.apply(Command::SetTrackHidden { track: tracks[1], hidden: true }).unwrap();
        assert_eq!(app.below(tracks[0], 1), Some(tracks[2]));
    }
}

#[cfg(test)]
mod take_folders {
    use std::sync::Arc;

    use super::*;
    use crate::App;
    use loupe_engine::{Command, Outcome, Source};

    const CANVAS: Rectangle = Rectangle { x: 0.0, y: 0.0, width: 1200.0, height: 900.0 };
    const TOP: f32 = theme::NEUTRAL.scrollbar_height + theme::NEUTRAL.ruler_height;
    const TALL: f32 = theme::NEUTRAL.track_height;
    const LEFT: f32 = theme::NEUTRAL.header_width;
    const PASSES: usize = 13;

    fn a_looped_vocal_and_drums() -> (App, TrackId, TrackId) {
        let mut app = App::bare();
        app.canvas_at.set(CANVAS);
        app.view = View { zoom: 100.0, scroll: 0.0, scroll_y: 0.0 };
        let pass = app.project.rate as Frames * 2;
        let Ok(Outcome::Track(vox)) = app.project.apply(Command::AddTrack { name: "Vox".into() }) else { panic!() };
        let Ok(Outcome::Track(drums)) = app.project.apply(Command::AddTrack { name: "Drums".into() }) else { panic!() };
        let source = Arc::new(Source::from_frames("Vox (take 14)", vec![[0.2, 0.2]; pass as usize * PASSES]));
        let Ok(Outcome::Clip(clip)) = app.project.apply(Command::AddClip { track: vox, source, start: 0 }) else { panic!() };
        app.project.apply(Command::TrimClip { clip, offset: pass * (PASSES as Frames - 1), len: pass }).unwrap();
        let offsets = (0..PASSES).map(|at| at as i64 * pass as i64).collect();
        app.project.apply(Command::SetTakes { clip, offsets, active: 0 }).unwrap();
        (app, vox, drums)
    }

    #[test]
    fn deleting_a_take_drops_it_from_the_folder_and_undo_brings_it_back() {
        let (mut app, vox, _) = a_looped_vocal_and_drums();
        let _ = app.update(crate::Message::UseTakeOnTrack(vox, 4));
        let clip = |app: &App| app.project.track(vox).unwrap().clips[0].clone();
        let heard = clip(&app).offset;
        let _ = app.update(crate::Message::DeleteTake(vox, 2));
        assert_eq!(clip(&app).takes.len(), PASSES - 1, "the take was not removed");
        assert_eq!(clip(&app).take, 3, "the playing take lost its place after an earlier one went");
        assert_eq!(clip(&app).offset, heard, "the clip now plays different audio");
        let _ = app.update(crate::Message::DeleteTake(vox, 3));
        assert_eq!(clip(&app).takes.len(), PASSES - 2);
        assert_eq!(clip(&app).take, 2, "deleting the playing take should fall back to the one before");
        let _ = app.update(crate::Message::Undo);
        let _ = app.update(crate::Message::Undo);
        assert_eq!(clip(&app).takes.len(), PASSES);
        assert_eq!(clip(&app).take, 4);
    }

    fn timeline_of(app: &App) -> Timeline<'_> {
        Timeline {
            knobs: &app.knob_names,
            project: &app.project,
            palette: &app.palette,
            view: app.view,
            heights: &app.heights,
            selected: app.selected,
            selection: &app.selection,
            playhead: app.playhead,
            loop_range: app.loop_range,
            punch: app.punch,
            tool: app.tool,
            snap: app.snap,
            hinting: app.hint,
            armed: &app.armed,
            recording_from: None,
            taking_shape: &app.taking_shape,
            chosen_tracks: &app.chosen_tracks,
            track_meters: &app.track_meters,
            opening: false,
            width: CANVAS.width,
            cache: &app.cache,
            at: &app.canvas_at,
            pixels_per_point: 1.0,
        }
    }

    #[test]
    fn a_folded_track_keeps_its_height_and_shows_one_badge_for_all_its_takes() {
        let (app, vox, _) = a_looped_vocal_and_drums();
        let timeline = timeline_of(&app);
        let track = app.project.track(vox).unwrap();
        assert_eq!(timeline.height_of(track), TALL, "thirteen takes made a folded track taller");
        let badges = timeline.take_badges(0);
        assert_eq!(badges.len(), 1);
        assert_eq!(badges[0].1, PASSES);
        assert!(matches!(timeline.hit(badges[0].0.center()), Hit::Folder(found) if found.id == vox));
        let middle = Point::new(LEFT + 100.0, TOP + TALL / 2.0 + 8.0);
        assert!(matches!(timeline.hit(middle), Hit::Clip(_)), "the comp row is an ordinary clip while folded");
        assert!(matches!(timeline.hit(Point::new(LEFT + 100.0, TOP + TALL + TALL / 2.0)), Hit::Lane), "the next track starts right under the fold");
    }

    #[test]
    fn an_open_folder_gives_every_take_a_readable_lane_under_the_comp() {
        let (mut app, vox, drums) = a_looped_vocal_and_drums();
        let _ = app.update(crate::Message::ToggleTakes(vox));
        let below = TOP + TALL + PASSES as f32 * TALL + TALL / 2.0;
        let timeline = timeline_of(&app);
        let track = app.project.track(vox).unwrap();
        assert_eq!(timeline.height_of(track), TALL + PASSES as f32 * TALL);
        for take in [0, 6, PASSES - 1] {
            let y = TOP + TALL + take as f32 * TALL + TALL / 2.0;
            assert!(matches!(timeline.hit(Point::new(LEFT + 100.0, y)), Hit::Take(_, found) if found == take), "lane {take} did not answer as take {take}");
        }
        assert_eq!(timeline.track_at(below), Some(1));
        assert_eq!(app.dropped_onto(Point::new(LEFT + 10.0, TOP + TALL + 10.0)).map(|(track, _)| track), Some(vox), "a drop on the take lanes went to another track");
        let scrolled = below - CANVAS.height / 2.0;
        app.view.scroll_y = scrolled;
        assert_eq!(app.dropped_onto(Point::new(LEFT + 10.0, below - scrolled)).map(|(track, _)| track), Some(drums), "a drop under the last take lane missed the next track");
    }

    #[test]
    fn swiping_a_lane_comps_that_take_and_a_click_picks_it() {
        let (mut app, vox, _) = a_looped_vocal_and_drums();
        let _ = app.update(crate::Message::ToggleTakes(vox));
        app.tool = Tool::Pencil;
        let timeline = timeline_of(&app);
        let mut state = Interaction::default();
        let y = TOP + TALL + 3.0 * TALL + TALL / 2.0;
        let cursor = |x: f32| mouse::Cursor::Available(Point::new(CANVAS.x + x, CANVAS.y + y));
        let press = canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let moved = canvas::Event::Mouse(mouse::Event::CursorMoved { position: Point::new(LEFT + 150.0, y) });
        let released = canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
        let _ = canvas::Program::update(&timeline, &mut state, press.clone(), CANVAS, cursor(LEFT + 50.0));
        let _ = canvas::Program::update(&timeline, &mut state, moved, CANVAS, cursor(LEFT + 150.0));
        let (_, swiped) = canvas::Program::update(&timeline, &mut state, released.clone(), CANVAS, cursor(LEFT + 150.0));
        assert!(matches!(swiped, Some(crate::Message::Comp { track, take: 3, from, to }) if track == vox && from < to), "a swipe on lane 4 did not comp take 4");
        let _ = canvas::Program::update(&timeline, &mut state, press, CANVAS, cursor(LEFT + 60.0));
        let (_, clicked) = canvas::Program::update(&timeline, &mut state, released, CANVAS, cursor(LEFT + 60.0));
        assert!(matches!(clicked, Some(crate::Message::UseTake(_, 3))), "a click on lane 4 did not pick take 4");
    }
}

const MOST_WAVE_BOOST: f32 = 8.0;

fn wave_boost(clip: &Clip) -> f32 {
    let (lo, hi) = clip.peak(clip.offset as usize, (clip.offset + clip.len) as usize);
    let loudest = lo.abs().max(hi.abs());
    if loudest <= 1e-4 {
        return 1.0;
    }
    (1.0 / loudest).clamp(1.0, MOST_WAVE_BOOST)
}

#[cfg(test)]
mod wave_boost_tests {
    use std::sync::Arc;

    use loupe_engine::{Command, Outcome, Project, Source};

    use super::wave_boost;

    #[test]
    fn turning_clip_gain_down_still_shrinks_the_waveform() {
        let mut project = Project::new(48_000);
        let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: "Vox".into() }) else { panic!() };
        let source = Arc::new(Source::from_frames("take", vec![[0.125, 0.125]; 4_000]));
        let Ok(Outcome::Clip(id)) = project.apply(Command::AddClip { track, source, start: 0 }) else { panic!() };
        let mut clip = project.clip(id).unwrap().clone();
        let full = wave_boost(&clip);
        clip.gain = 0.5;
        assert_eq!(wave_boost(&clip), full, "gain changed the boost, so the waveform hid the gain change");
    }

    use super::MOST_WAVE_BOOST;

    #[test]
    fn the_boost_never_shrinks_and_never_passes_the_ceiling() {
        let boost = |loudest: f32| (1.0 / loudest).clamp(1.0, MOST_WAVE_BOOST);
        assert_eq!(boost(1.0), 1.0, "a full-scale beat is drawn as it is");
        assert!((boost(0.125) - 8.0).abs() < 1e-6, "a -18 dB take fills the lane");
        assert_eq!(boost(0.01), MOST_WAVE_BOOST, "near silence is not blown up past 18 dB");
        assert_eq!(boost(1.5), 1.0, "clipping is never shrunk");
    }
}
