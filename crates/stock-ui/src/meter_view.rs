use std::sync::Arc;

use iced::widget::canvas::{self, Geometry, Path, Stroke};
use iced::widget::{button, column, container, row, text, Space};
use iced::{mouse, Alignment, Element, Length, Point, Rectangle, Renderer, Size, Theme};
use loupe_stock::{Meter, Moment, Readings, SILENT_LUFS};

use crate::kit::{dial, graph_frame, knob_row, panel, put_text};
use crate::knobs::{change, Change, Knobs};
use crate::look::{fade, Look};

const SHOWN_MOMENTS: usize = 600;
const LOWEST_LUFS: f32 = -40.0;
const HIGHEST_LUFS: f32 = 0.0;
const READOUT_WIDTH: f32 = 210.0;
const GAP: f32 = 14.0;
const MARKS: [f32; 5] = [-36.0, -24.0, -14.0, -9.0, -6.0];

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
        let target = knob_row(vec![dial(look, params, values, 0, look.accent, change)]);
        let pressed = values[1];
        let reset = button(text("Reset").size(12.5))
            .padding([6, 14])
            .style(move |_, status| {
                let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: Some(if hovered { look.raised } else { look.panel }.into()),
                    text_color: look.text,
                    border: iced::Border::default().rounded(6).color(look.grid_strong).width(1.0),
                    ..Default::default()
                }
            })
            .on_press(Change(1, if pressed > 0.5 { 0.0 } else { 1.0 }));
        let controls = row![target, Space::with_width(Length::Fill), reset].align_y(Alignment::Center);
        let graph = canvas::Canvas::new(Loudness { editor: self }).width(Length::Fill).height(Length::Fill);
        let backdrop = container(container(graph).padding([12, 12])).width(Length::Fill).height(Length::Fill).style(move |_| container::Style {
            background: Some(look.background.into()),
            ..Default::default()
        });
        column![backdrop, panel(look, controls)].into()
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
        let graph_w = (size.width - READOUT_WIDTH - GAP).max(1.0);
        let y_of = |lufs: f32| size.height - (lufs.clamp(LOWEST_LUFS, HIGHEST_LUFS) - LOWEST_LUFS) / (HIGHEST_LUFS - LOWEST_LUFS) * size.height;
        for mark in MARKS {
            let y = y_of(mark).round();
            frame.fill_rectangle(Point::new(0.0, y), Size::new(graph_w, 1.0), look.grid);
            put_text(&mut frame, &format!("{mark:.0}"), Point::new(graph_w - 4.0, y - 3.0), 11.0, look.text_dim, (1.0, 1.0));
        }
        let target = editor.knobs.values[0];
        let target_y = y_of(target).round();
        frame.fill_rectangle(Point::new(0.0, target_y), Size::new(graph_w, 2.0), fade(look.accent, 0.8));
        put_text(&mut frame, &format!("Target {target:.0} LUFS"), Point::new(6.0, target_y - 4.0), 11.0, look.accent, (0.0, 1.0));
        let step = graph_w / editor.moments.len().max(1) as f32;
        for (i, moment) in editor.moments.iter().enumerate() {
            if unwritten(moment) || moment.input <= SILENT_LUFS + 0.5 {
                continue;
            }
            let top = y_of(moment.input);
            frame.fill_rectangle(Point::new(i as f32 * step, top), Size::new(step.ceil(), size.height - top), fade(look.spectrum, 0.35));
        }
        let line = Path::new(|b| {
            let mut started = false;
            for (i, moment) in editor.moments.iter().enumerate() {
                if unwritten(moment) || moment.output <= SILENT_LUFS + 0.5 {
                    started = false;
                    continue;
                }
                let point = Point::new(i as f32 * step, y_of(moment.output));
                if started {
                    b.line_to(point);
                } else {
                    b.move_to(point);
                    started = true;
                }
            }
        });
        frame.stroke(&line, Stroke::default().with_color(look.text).with_width(2.0));

        let left = graph_w + GAP;
        frame.fill_rectangle(Point::new(left, 0.0), Size::new(READOUT_WIDTH, size.height), look.panel);
        let middle = left + READOUT_WIDTH / 2.0;
        let shown = &editor.shown;
        let near_target = (shown.integrated - target).abs() <= 1.0;
        put_text(&mut frame, "Integrated", Point::new(middle, 14.0), 11.5, look.text_dim, (0.5, 0.0));
        put_text(&mut frame, &lufs_text(shown.integrated), Point::new(middle, 30.0), 34.0, if near_target { look.accent } else { look.text }, (0.5, 0.0));
        put_text(&mut frame, "LUFS", Point::new(middle, 72.0), 11.5, look.text_dim, (0.5, 0.0));
        let rows = [
            ("Short term", lufs_text(shown.short_term), "LUFS"),
            ("Momentary", lufs_text(shown.momentary), "LUFS"),
            ("Range", format!("{:.1}", shown.range), "LU"),
            ("True peak", lufs_text(shown.peak), "dBTP"),
        ];
        for (n, (label, value, unit)) in rows.iter().enumerate() {
            let y = 104.0 + n as f32 * 44.0;
            put_text(&mut frame, label, Point::new(left + 14.0, y), 11.5, look.text_dim, (0.0, 0.0));
            let colour = if *label == "True peak" && shown.peak > -1.0 { look.danger } else { look.text };
            put_text(&mut frame, &format!("{value} {unit}"), Point::new(left + READOUT_WIDTH - 14.0, y + 14.0), 17.0, colour, (1.0, 0.0));
        }
        vec![frame.into_geometry()]
    }
}
