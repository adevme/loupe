use crate::model::{ClipId, Frames, TrackId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    TrackGain(TrackId),
    TrackPan(TrackId),
    MasterGain,
    SendGain { from: TrackId, to: TrackId },
    TrackFx { track: TrackId, slot: usize, knob: usize },
    MasterFx { slot: usize, knob: usize },
    ClipGain(ClipId),
    ClipFx { clip: ClipId, slot: usize, knob: usize },
}

impl Target {
    pub fn on_clip(&self) -> Option<ClipId> {
        match self {
            Target::ClipGain(clip) => Some(*clip),
            Target::ClipFx { clip, .. } => Some(*clip),
            _ => None,
        }
    }

    pub fn on_track(&self) -> Option<TrackId> {
        match self {
            Target::TrackGain(track) => Some(*track),
            Target::TrackPan(track) => Some(*track),
            Target::SendGain { from, .. } => Some(*from),
            Target::TrackFx { track, .. } => Some(*track),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Linear,
    Hold,
    Curve(f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub at: Frames,
    pub value: f32,
    pub shape: Shape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Touch,
    Latch,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Envelope {
    pub target: Target,
    pub points: Vec<Point>,
    pub armed: Option<Mode>,
    pub lane_open: bool,
    pub lowest: f32,
    pub highest: f32,
}

impl Envelope {
    pub fn new(target: Target, lowest: f32, highest: f32, resting: f32) -> Self {
        Self {
            target,
            points: vec![Point { at: 0, value: resting.clamp(lowest, highest), shape: Shape::Linear }],
            armed: None,
            lane_open: true,
            lowest,
            highest,
        }
    }

    pub fn value_at(&self, at: Frames) -> Option<f32> {
        if self.points.is_empty() {
            return None;
        }
        let first = self.points.first()?;
        if at <= first.at {
            return Some(first.value);
        }
        let last = self.points.last()?;
        if at >= last.at {
            return Some(last.value);
        }
        let after = self.points.iter().position(|point| point.at > at)?;
        let left = self.points[after - 1];
        let right = self.points[after];
        if matches!(left.shape, Shape::Hold) || right.at == left.at {
            return Some(left.value);
        }
        let along = (at - left.at) as f32 / (right.at - left.at) as f32;
        let eased = match left.shape {
            Shape::Hold => 0.0,
            Shape::Linear => along,
            Shape::Curve(bend) => along.powf(4f32.powf(-bend)),
        };
        Some(left.value + (right.value - left.value) * eased)
    }

    pub fn put(&mut self, point: Point) -> usize {
        let value = point.value.clamp(self.lowest, self.highest);
        let point = Point { value, ..point };
        match self.points.iter().position(|kept| kept.at == point.at) {
            Some(at) => {
                let shape = self.points[at].shape;
                self.points[at] = Point { shape, ..point };
                at
            }
            None => {
                let at = self.points.iter().position(|kept| kept.at > point.at).unwrap_or(self.points.len());
                self.points.insert(at, point);
                at
            }
        }
    }

    pub fn drop_point(&mut self, which: usize) {
        if which < self.points.len() && self.points.len() > 1 {
            self.points.remove(which);
        }
    }

    pub fn clear_between(&mut self, from: Frames, to: Frames) {
        self.points.retain(|point| point.at < from || point.at > to);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line() -> Envelope {
        let mut shape = Envelope::new(Target::MasterGain, 0.0, 1.0, 0.0);
        shape.points.clear();
        shape.put(Point { at: 0, value: 0.0, shape: Shape::Linear });
        shape.put(Point { at: 100, value: 1.0, shape: Shape::Linear });
        shape
    }

    #[test]
    fn a_line_reads_halfway_in_the_middle() {
        let shape = line();
        assert_eq!(shape.value_at(0), Some(0.0));
        assert_eq!(shape.value_at(50), Some(0.5));
        assert_eq!(shape.value_at(100), Some(1.0));
    }

    #[test]
    fn before_and_after_hold_the_ends() {
        let shape = line();
        assert_eq!(shape.value_at(200), Some(1.0));
    }

    #[test]
    fn a_hold_stays_flat_until_the_next_point() {
        let mut shape = line();
        shape.points[0].shape = Shape::Hold;
        assert_eq!(shape.value_at(99), Some(0.0));
        assert_eq!(shape.value_at(100), Some(1.0));
    }

    #[test]
    fn a_curve_bends_away_from_the_line() {
        let mut shape = line();
        shape.points[0].shape = Shape::Curve(1.0);
        let bent = shape.value_at(50).unwrap();
        assert!(bent > 0.5, "the curve read {bent}");
    }

    #[test]
    fn points_stay_in_order_and_values_stay_in_range() {
        let mut shape = line();
        shape.put(Point { at: 40, value: 9.0, shape: Shape::Linear });
        let times: Vec<_> = shape.points.iter().map(|point| point.at).collect();
        assert_eq!(times, vec![0, 40, 100]);
        assert_eq!(shape.points[1].value, 1.0);
    }

    #[test]
    fn the_last_point_cannot_be_removed() {
        let mut shape = line();
        shape.drop_point(1);
        shape.drop_point(0);
        assert_eq!(shape.points.len(), 1);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Writer {
    pub target: Target,
    pub mode: Mode,
    pub from: Frames,
    pub was: f32,
    pub value: f32,
}

impl Envelope {
    pub fn start_writing(&mut self, mode: Mode, at: Frames, value: f32) -> Writer {
        let was = self.value_at(at).unwrap_or(value);
        if at > 0 {
            self.put(Point { at: at - 1, value: was, shape: Shape::Linear });
        }
        self.put(Point { at, value, shape: Shape::Linear });
        Writer { target: self.target, mode, from: at, was, value }
    }

    pub fn go_on_writing(&mut self, writer: &mut Writer, at: Frames, value: f32) {
        if at <= writer.from {
            return;
        }
        self.clear_between(writer.from + 1, at);
        self.put(Point { at, value, shape: Shape::Linear });
        writer.value = value;
    }

    pub fn stop_writing(&mut self, writer: &Writer, at: Frames) {
        if writer.mode != Mode::Touch {
            return;
        }
        let at = at.max(writer.from + 1);
        self.put(Point { at: at + 1, value: writer.was, shape: Shape::Linear });
    }
}

#[cfg(test)]
mod writing {
    use super::*;

    fn steady() -> Envelope {
        let mut shape = Envelope::new(Target::MasterGain, 0.0, 1.0, 0.4);
        shape.points.clear();
        shape.put(Point { at: 0, value: 0.4, shape: Shape::Linear });
        shape.put(Point { at: 1000, value: 0.4, shape: Shape::Linear });
        shape
    }

    #[test]
    fn touch_snaps_back_when_you_let_go() {
        let mut shape = steady();
        let mut writer = shape.start_writing(Mode::Touch, 200, 0.9);
        for at in [250, 300] {
            shape.go_on_writing(&mut writer, at, 0.9);
        }
        shape.stop_writing(&writer, 300);
        assert_eq!(shape.value_at(100), Some(0.4), "before the move");
        assert_eq!(shape.value_at(300), Some(0.9), "during the move");
        assert_eq!(shape.value_at(600), Some(0.4), "after letting go");
    }

    #[test]
    fn latch_holds_on_to_the_end() {
        let mut shape = steady();
        let mut writer = shape.start_writing(Mode::Latch, 200, 0.9);
        for at in [250, 300, 500, 800] {
            shape.go_on_writing(&mut writer, at, 0.9);
        }
        shape.stop_writing(&writer, 800);
        assert_eq!(shape.value_at(100), Some(0.4), "before the move");
        assert_eq!(shape.value_at(400), Some(0.9), "latch is still writing after you let go");
        assert_eq!(shape.value_at(800), Some(0.9), "latch wrote right up to the stop");
    }

    #[test]
    fn a_move_wipes_what_was_under_it() {
        let mut shape = steady();
        shape.put(Point { at: 400, value: 0.1, shape: Shape::Linear });
        let mut writer = shape.start_writing(Mode::Latch, 300, 0.8);
        shape.go_on_writing(&mut writer, 500, 0.8);
        assert_eq!(shape.points.iter().filter(|point| point.at == 400).count(), 0);
    }
}
