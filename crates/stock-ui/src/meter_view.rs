use std::sync::Arc;

use iced::widget::canvas::{self, Geometry, Stroke};
use iced::{mouse, Element, Length, Point, Rectangle, Renderer, Size, Theme, Vector};
use loupe_stock::{Meter, Moment, Readings, SILENT_LUFS};

use crate::compressor_view::{GAP, INSET};
use crate::kit::*;
use crate::knobs::{change, Change, Knobs};
use crate::look::{fade, Look, MEDIUM};

const SHOWN_MOMENTS: usize = 600;
const LOWEST_LUFS: f32 = -40.0;
const HIGHEST_LUFS: f32 = 0.0;
const READOUT_WIDTH: f32 = 200.0;
const MARKS: [f32; 6] = [-36.0, -30.0, -24.0, -18.0, -12.0, -6.0];

#[derive(Default)]
struct Shown {
    momentary: f32,
    short_term: f32,
    integrated: f32,
    range: f32,
    peak: f32,
}

pub struct MeterEditor {
    knobs: Knobs,
    readings: Option<Arc<Readings>>,
    moments: Vec<Moment>,
    shown: Shown,
    look: Look,
}

impl MeterEditor {
    pub fn new(readings: Option<Arc<Readings>>, look: Look) -> Self {
        let look = look.tinted(crate::look::accents::METER);
        let silent = Shown { momentary: SILENT_LUFS, short_term: SILENT_LUFS, integrated: SILENT_LUFS, range: 0.0, peak: SILENT_LUFS };
        Self { knobs: Knobs::of(&Meter::new()), readings, moments: vec![Moment::default(); SHOWN_MOMENTS], shown: silent, look }
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
        if let Some(readings) = &self.readings {
            readings.history.latest(&mut self.moments);
            self.shown = Shown {
                momentary: readings.momentary(),
                short_term: readings.short_term(),
                integrated: readings.integrated(),
                range: readings.range(),
                peak: readings.peak(),
            };
        }
    }

    pub fn view(&self) -> Element<'_, Change> {
        let look = self.look;
        let values = &self.knobs.values;
        let params = self.knobs.params;
        let pressed = values[1];
        shell(look, "Meter", "Loudness meter", move || canvas::Canvas::new(Loudness { editor: self }).width(Length::Fill).height(Length::Fill).into(), move |compact| {
            let p = Panel::new(look, compact, params, values, change);
            vec![vec![
                section(look, "Goal", vec![p.hero(0)]),
                section(look, "Measurement", vec![push(look, "Reset", Some(Change(1, if pressed > 0.5 { 0.0 } else { 1.0 })))]),
            ]]
        })
    }
}

fn unwritten(moment: &Moment) -> bool {
    moment.input == 0.0 && moment.output == 0.0
}

fn lufs_text(value: f32) -> String {
    if value <= SILENT_LUFS + 0.5 {
        "-inf".to_string()
    } else {
        format!("{value:.1}")
    }
}

struct Loudness<'a> {
    editor: &'a MeterEditor,
}

impl canvas::Program<Change> for Loudness<'_> {
    type State = ();

    fn draw(&self, _state: &(), renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let editor = self.editor;
        let look = editor.look;
        let size = bounds.size();
        if !crate::kit::roomy(size) {
            return Vec::new();
        }
        let mut frame = graph_frame(renderer, size, &look);
        let label_room = 30.0;
        let graph_w = (size.width - READOUT_WIDTH - GAP - INSET * 2.0 - label_room).max(1.0);
        let top = INSET + 6.0;
        let bottom = size.height - INSET - 6.0;
        let y_of = |lufs: f32| bottom - (lufs.clamp(LOWEST_LUFS, HIGHEST_LUFS) - LOWEST_LUFS) / (HIGHEST_LUFS - LOWEST_LUFS) * (bottom - top);
        let target = editor.knobs.values[0];
        frame.with_save(|frame| {
            frame.translate(Vector::new(INSET, 0.0));
            for mark in MARKS {
                let y = y_of(mark);
                guide(frame, Point::new(0.0, y), graph_w, look.grid);
                axis_text(frame, &format!("{mark:.0}"), Point::new(graph_w + label_room - 2.0, y), &look, (1.0, 0.5));
            }
            let target_y = y_of(target);
            frame.fill_rectangle(Point::new(0.0, target_y - 6.0), Size::new(graph_w, 12.0), fade(look.accent, 0.02));
            dashed(frame, Point::new(0.0, target_y), graph_w, fade(look.accent, 0.85));
            put_text_in(frame, &format!("TARGET  {target:.0} LUFS"), Point::new(8.0, target_y - 9.0), 9.5, look.accent, (0.0, 1.0), MEDIUM);
            let step = graph_w / (editor.moments.len().max(2) - 1) as f32;
            let mut run: Vec<Point> = Vec::new();
            let mut runs: Vec<Vec<Point>> = Vec::new();
            for (i, moment) in editor.moments.iter().enumerate() {
                if unwritten(moment) || moment.input <= SILENT_LUFS + 0.5 {
                    if !run.is_empty() {
                        runs.push(std::mem::take(&mut run));
                    }
                    continue;
                }
                run.push(Point::new(i as f32 * step, y_of(moment.input)));
            }
            if !run.is_empty() {
                runs.push(run);
            }
            for run in &runs {
                fill_under(frame, run, bottom, fade(look.accent, 0.32), fade(look.accent, 0.02));
            }
            let mut line: Vec<Point> = Vec::new();
            let mut lines: Vec<Vec<Point>> = Vec::new();
            for (i, moment) in editor.moments.iter().enumerate() {
                if unwritten(moment) || moment.output <= SILENT_LUFS + 0.5 {
                    if !line.is_empty() {
                        lines.push(std::mem::take(&mut line));
                    }
                    continue;
                }
                line.push(Point::new(i as f32 * step, y_of(moment.output)));
            }
            if !line.is_empty() {
                lines.push(line);
            }
            for line in &lines {
                let path = polyline(line);
                frame.stroke(&path, Stroke::default().with_color(fade(look.text, 0.12)).with_width(5.0));
                frame.stroke(&path, Stroke::default().with_color(look.text).with_width(1.8));
            }
        });
        frame.with_save(|frame| {
            frame.translate(Vector::new(size.width - READOUT_WIDTH - INSET, INSET));
            let shown = &editor.shown;
            let near_target = (shown.integrated - target).abs() <= 1.0;
            let main = Reading::new("Integrated", lufs_text(shown.integrated), "LUFS", if near_target { look.accent } else { look.text });
            let peak_colour = if shown.peak > -1.0 { look.danger } else { look.text };
            let rest = [
                Reading::new("Short term", lufs_text(shown.short_term), "LUFS", look.text),
                Reading::new("Momentary", lufs_text(shown.momentary), "LUFS", look.text),
                Reading::new("Range", format!("{:.1}", shown.range), "LU", look.text),
                Reading::new("True peak", lufs_text(shown.peak), "dBTP", peak_colour),
            ];
            let share = (shown.short_term - LOWEST_LUFS) / (HIGHEST_LUFS - LOWEST_LUFS);
            draw_readouts(frame, Size::new(READOUT_WIDTH, size.height - INSET * 2.0), &look, &main, Some((share, look.accent)), &rest);
        });
        seal(&mut frame, size, &look);
        vec![frame.into_geometry()]
    }
}
