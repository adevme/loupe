use iced::widget::canvas::{self, Cache, Frame, Geometry, Path, Stroke, Text};
use iced::{alignment, keyboard, mouse, Color, Point, Rectangle, Renderer, Size, Theme};
use loupe_engine::{Clip, ClipId, Frames, Project, Track};

use crate::{icons, theme, Message};

pub const HEADER_W: f32 = 200.0;
const RULER_H: f32 = 30.0;
const TRACK_H: f32 = 92.0;
const ADD_ROW_H: f32 = 40.0;
const CLIP_PAD: f32 = 5.0;
const CLIP_TITLE_H: f32 = 19.0;
const MIN_GRID_PX: f64 = 14.0;
const DRAG_THRESHOLD: f32 = 4.0;
pub const MIN_ZOOM: f64 = 2.0;

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
    pub view: View,
    pub selected: Option<ClipId>,
    pub playhead: Frames,
    pub cache: &'a Cache,
}

#[derive(Default)]
pub struct Interaction {
    drag: Option<Drag>,
    modifiers: keyboard::Modifiers,
}

enum Drag {
    Scrub,
    Clip {
        id: ClipId,
        grab: f64,
        origin: Point,
        moving: bool,
    },
}

enum Hit<'a> {
    Ruler,
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

    fn track_top(&self, index: usize) -> f32 {
        RULER_H + index as f32 * TRACK_H - self.view.scroll_y
    }

    fn track_at(&self, y: f32) -> Option<usize> {
        let row = (y - RULER_H + self.view.scroll_y) / TRACK_H;
        (y >= RULER_H && row >= 0.0 && (row as usize) < self.project.tracks.len())
            .then_some(row as usize)
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
        Rectangle::new(
            Point::new(16.0, self.track_top(index) + TRACK_H - 36.0),
            Size::new(28.0, 22.0),
        )
    }

    fn remove_button(&self, index: usize) -> Rectangle {
        Rectangle::new(
            Point::new(HEADER_W - 34.0, self.track_top(index) + 8.0),
            Size::new(24.0, 24.0),
        )
    }

    fn add_button(&self) -> Rectangle {
        Rectangle::new(
            Point::new(0.0, self.track_top(self.project.tracks.len())),
            Size::new(HEADER_W, ADD_ROW_H),
        )
    }

    fn content_height(&self) -> f32 {
        RULER_H + self.project.tracks.len() as f32 * TRACK_H + ADD_ROW_H
    }

    fn hit(&self, p: Point) -> Hit<'_> {
        if p.y < RULER_H {
            return if p.x >= HEADER_W { Hit::Ruler } else { Hit::Nothing };
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
        let top = self.track_top(i);
        if p.y < top + CLIP_PAD || p.y > top + TRACK_H - CLIP_PAD {
            return Hit::Lane;
        }
        let at = self.frames_at(p.x);
        match self.project.tracks[i]
            .clips
            .iter()
            .rev()
            .find(|c| at >= c.start as f64 && at < c.end() as f64)
        {
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

        let anywhere = cursor
            .position()
            .map(|p| Point::new(p.x - bounds.x, p.y - bounds.y));
        let free = state.modifiers.shift();

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
                        state.drag = Some(Drag::Scrub);
                        Some(Message::Seek(self.snap(self.frames_at(p.x), free)))
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
                    Hit::Lane => {
                        Some(Message::LaneClicked(self.snap(self.frames_at(p.x), free)))
                    }
                    Hit::Nothing => None,
                };
                (Captured, message)
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let (Some(p), Some(drag)) = (anywhere, state.drag.as_mut()) else {
                    return (Ignored, None);
                };
                match drag {
                    Drag::Scrub => {
                        let to = self.snap(self.frames_at(p.x.max(HEADER_W)), free);
                        (Captured, (to != self.playhead).then_some(Message::Seek(to)))
                    }
                    Drag::Clip { id, grab, origin, moving } => {
                        if !*moving && p.distance(*origin) < DRAG_THRESHOLD {
                            return (Captured, None);
                        }
                        *moving = true;
                        let (Some(clip), Some(current)) =
                            (self.project.clip(*id), self.project.track_of(*id))
                        else {
                            return (Captured, None);
                        };
                        let start = self.snap(self.frames_at(p.x) - *grab, free);
                        let row = ((p.y - RULER_H + self.view.scroll_y) / TRACK_H).floor();
                        let row = (row.max(0.0) as usize).min(self.project.tracks.len() - 1);
                        let track = self.project.tracks[row].id;
                        let changed = start != clip.start || track != current.id;
                        (Captured, changed.then_some(Message::MoveClip { clip: *id, track, start }))
                    }
                }
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                match state.drag.take() {
                    Some(Drag::Clip { moving: true, .. }) => (Captured, Some(Message::DragEnd)),
                    Some(_) => (Captured, None),
                    None => (Ignored, None),
                }
            }
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
        let content = self.cache.draw(renderer, bounds.size(), |frame| {
            let lanes = Rectangle::new(
                Point::new(HEADER_W, RULER_H),
                Size::new((bounds.width - HEADER_W).max(0.0), (bounds.height - RULER_H).max(0.0)),
            );
            frame.with_clip(lanes, |frame| self.draw_lanes(frame, lanes.size()));
            let ruler =
                Rectangle::new(Point::new(HEADER_W, 0.0), Size::new(lanes.width, RULER_H));
            frame.with_clip(ruler, |frame| self.draw_ruler(frame, ruler.size()));
            let headers =
                Rectangle::new(Point::new(0.0, RULER_H), Size::new(HEADER_W, lanes.height));
            frame.with_clip(headers, |frame| self.draw_headers(frame, headers.size()));

            frame.fill_rectangle(Point::ORIGIN, Size::new(HEADER_W, RULER_H), theme::PANEL);
            frame.fill_rectangle(
                Point::new(0.0, RULER_H - 1.0),
                Size::new(bounds.width, 1.0),
                theme::LINE,
            );
            frame.fill_rectangle(
                Point::new(HEADER_W - 1.0, 0.0),
                Size::new(1.0, bounds.height),
                theme::LINE,
            );
        });

        let mut overlay = Frame::new(renderer, bounds.size());
        let x = self.x_of(self.playhead as f64).round();
        if x >= HEADER_W && x <= bounds.width {
            overlay.fill_rectangle(
                Point::new(x - 0.5, 0.0),
                Size::new(1.0, bounds.height),
                theme::ACCENT,
            );
            let cap = Path::new(|b| {
                b.move_to(Point::new(x - 5.5, 0.0));
                b.line_to(Point::new(x + 5.5, 0.0));
                b.line_to(Point::new(x, 9.0));
                b.close();
            });
            overlay.fill(&cap, theme::ACCENT);
        }
        let _ = state;
        vec![content, overlay.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &Interaction,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if let Some(Drag::Clip { moving: true, .. }) = state.drag {
            return mouse::Interaction::Grabbing;
        }
        match cursor.position_in(bounds).map(|p| self.hit(p)) {
            Some(Hit::Mute(_) | Hit::Remove(_) | Hit::AddTrack) => mouse::Interaction::Pointer,
            Some(Hit::Clip(_)) => mouse::Interaction::Grab,
            _ => mouse::Interaction::default(),
        }
    }
}

impl Timeline<'_> {
    fn draw_lanes(&self, frame: &mut Frame, size: Size) {
        frame.fill_rectangle(Point::ORIGIN, size, theme::BG);
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
                theme::mix(theme::BG, Color::WHITE, strength),
            );
        }

        if self.project.tracks.is_empty() {
            frame.fill_text(Text {
                content: "Import audio to begin".into(),
                position: Point::new(size.width / 2.0, size.height / 2.0 - 10.0),
                color: theme::TEXT_DIM,
                size: 15.0.into(),
                font: theme::MEDIUM,
                horizontal_alignment: alignment::Horizontal::Center,
                vertical_alignment: alignment::Vertical::Center,
                ..Text::default()
            });
            return;
        }

        for (i, track) in self.project.tracks.iter().enumerate() {
            let top = self.track_top(i) - RULER_H;
            if top > size.height || top + TRACK_H < 0.0 {
                continue;
            }
            frame.fill_rectangle(
                Point::new(0.0, top + TRACK_H - 1.0),
                Size::new(size.width, 1.0),
                theme::LINE,
            );
            let colour = if track.muted {
                theme::TEXT_FAINT
            } else {
                theme::TRACKS[i % theme::TRACKS.len()]
            };
            for clip in &track.clips {
                self.draw_clip(frame, size, clip, top, colour);
            }
        }
    }

    fn draw_clip(&self, frame: &mut Frame, size: Size, clip: &Clip, lane_top: f32, colour: Color) {
        let left = self.x_of(clip.start as f64) - HEADER_W;
        let right = self.x_of(clip.end() as f64) - HEADER_W;
        if right < 0.0 || left > size.width {
            return;
        }
        let selected = self.selected == Some(clip.id);
        let top = lane_top + CLIP_PAD;
        let height = TRACK_H - CLIP_PAD * 2.0;
        let shown_left = left.max(-8.0);
        let shown_right = right.min(size.width + 8.0);
        let body = Path::new(|b| {
            b.rounded_rectangle(
                Point::new(shown_left, top),
                Size::new((shown_right - shown_left).max(1.0), height),
                5.0.into(),
            );
        });
        let tint = if selected { 0.26 } else { 0.16 };
        frame.fill(&body, theme::mix(theme::BG, colour, tint));
        let title = Path::new(|b| {
            b.rounded_rectangle(
                Point::new(shown_left, top),
                Size::new((shown_right - shown_left).max(1.0), CLIP_TITLE_H),
                iced::border::top(5),
            );
        });
        frame.fill(&title, theme::mix(theme::BG, colour, tint + 0.18));

        let wave_top = top + CLIP_TITLE_H + 2.0;
        let wave_height = height - CLIP_TITLE_H - 5.0;
        self.draw_waveform(
            frame,
            clip,
            left,
            shown_left.max(0.0),
            shown_right.min(size.width),
            wave_top,
            wave_height,
            colour,
        );

        let outline = if selected {
            Stroke::default().with_color(theme::ACCENT).with_width(1.5)
        } else {
            Stroke::default().with_color(theme::mix(theme::BG, colour, 0.5)).with_width(1.0)
        };
        frame.stroke(&body, outline);

        let title_on_canvas = Rectangle::new(
            Point::new(HEADER_W + shown_left, RULER_H + top),
            Size::new((shown_right - shown_left).max(1.0), CLIP_TITLE_H),
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
                    color: theme::TEXT,
                    size: 11.5.into(),
                    font: theme::MEDIUM,
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
        frame.fill_rectangle(Point::ORIGIN, size, theme::PANEL);
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
            frame.fill_rectangle(
                Point::new(x.round(), size.height - 9.0),
                Size::new(1.0, 9.0),
                theme::TEXT_FAINT,
            );
            frame.fill_text(Text {
                content: format!("{}", bars as i64 + 1),
                position: Point::new(x.round() + 5.0, size.height / 2.0 - 1.0),
                color: theme::TEXT_DIM,
                size: 11.0.into(),
                font: theme::MONO,
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
                        theme::mix(theme::PANEL, theme::TEXT_FAINT, 0.6),
                    );
                }
            }
        }
    }

    fn draw_headers(&self, frame: &mut Frame, size: Size) {
        frame.fill_rectangle(Point::ORIGIN, size, theme::PANEL);
        for (i, track) in self.project.tracks.iter().enumerate() {
            let top = self.track_top(i) - RULER_H;
            if top > size.height || top + TRACK_H < 0.0 {
                continue;
            }
            frame.fill_rectangle(
                Point::new(0.0, top),
                Size::new(3.0, TRACK_H - 1.0),
                theme::TRACKS[i % theme::TRACKS.len()],
            );
            frame.fill_rectangle(
                Point::new(0.0, top + TRACK_H - 1.0),
                Size::new(size.width, 1.0),
                theme::LINE,
            );
            frame.fill_text(Text {
                content: shorten(&track.name, 20),
                position: Point::new(16.0, top + 20.0),
                color: if track.muted { theme::TEXT_DIM } else { theme::TEXT },
                size: 13.0.into(),
                font: theme::MEDIUM,
                vertical_alignment: alignment::Vertical::Center,
                ..Text::default()
            });

            let remove = self.remove_button(i);
            frame.fill_text(Text {
                content: icons::glyph("x").to_string(),
                position: Point::new(remove.center_x(), remove.center_y() - RULER_H),
                color: theme::TEXT_FAINT,
                size: 14.0.into(),
                font: theme::ICONS,
                horizontal_alignment: alignment::Horizontal::Center,
                vertical_alignment: alignment::Vertical::Center,
                shaping: iced::widget::text::Shaping::Advanced,
                ..Text::default()
            });

            let mute = self.mute_button(i);
            let shape = Path::new(|b| {
                b.rounded_rectangle(
                    Point::new(mute.x, mute.y - RULER_H),
                    mute.size(),
                    5.0.into(),
                );
            });
            frame.fill(&shape, if track.muted { theme::ROSE } else { theme::RAISED });
            frame.fill_text(Text {
                content: "M".into(),
                position: Point::new(mute.center_x(), mute.center_y() - RULER_H),
                color: if track.muted { theme::ON_ACCENT } else { theme::TEXT_DIM },
                size: 11.5.into(),
                font: theme::SEMIBOLD,
                horizontal_alignment: alignment::Horizontal::Center,
                vertical_alignment: alignment::Vertical::Center,
                ..Text::default()
            });
        }

        let add = self.add_button();
        frame.fill_text(Text {
            content: "+  Add track".into(),
            position: Point::new(16.0, add.center_y() - RULER_H),
            color: theme::TEXT_DIM,
            size: 12.5.into(),
            font: theme::INTER,
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
