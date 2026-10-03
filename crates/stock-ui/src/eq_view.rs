use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke, Text};
use iced::widget::{button, column, container, horizontal_space, pick_list, row, slider, text};
use iced::{alignment, keyboard, mouse, Alignment, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};
use loupe_stock::{band_design, knob, BandShape, Effect, Equalizer, Knob, Param, Scopes, BANDS, OUTPUT_KNOB, PLACES, SHAPES, SLOPES};

use crate::look::{fade, Look};
use crate::spectrum::Spectrum;

const LOWEST_HZ: f32 = 10.0;
const HIGHEST_HZ: f32 = 30_000.0;
const NODE_RADIUS: f32 = 6.5;
const NODE_REACH: f32 = 12.0;
const EDGE_PAD: f32 = 16.0;
const SPECTRUM_FLOOR_DB: f32 = -84.0;
const SPECTRUM_CEILING_DB: f32 = 18.0;
const RANGES: [&str; 3] = ["6 dB", "12 dB", "30 dB"];
const RANGE_DB: [f32; 3] = [6.0, 12.0, 30.0];
const DOUBLE_CLICK: Duration = Duration::from_millis(350);
const Q_PER_WHEEL_LINE: f32 = 1.12;
const Q_DOUBLING_PX: f32 = 60.0;
const FINE: f32 = 0.2;
const EDGE_CUT_SHARE: f32 = 0.06;
const COLUMN_PX: f32 = 2.0;
const MAJOR_HZ: [(f32, &str); 10] = [
    (20.0, "20"),
    (50.0, "50"),
    (100.0, "100"),
    (200.0, "200"),
    (500.0, "500"),
    (1000.0, "1k"),
    (2000.0, "2k"),
    (5000.0, "5k"),
    (10_000.0, "10k"),
    (20_000.0, "20k"),
];
const MINOR_HZ: [f32; 18] = [
    30.0, 40.0, 60.0, 70.0, 80.0, 90.0, 300.0, 400.0, 600.0, 700.0, 800.0, 900.0, 3000.0, 4000.0, 6000.0, 7000.0, 8000.0, 9000.0,
];

#[derive(Debug, Clone)]
pub enum EqMessage {
    Set(Vec<(usize, f32)>),
    Add { band: usize, changes: Vec<(usize, f32)> },
    Remove(usize),
    Select(Option<usize>),
    Range(&'static str),
}

pub struct EqEditor {
    params: &'static [Param],
    values: Vec<f32>,
    rate: f32,
    selected: Option<usize>,
    range: usize,
    scopes: Option<Arc<Scopes>>,
    before: Spectrum,
    after: Spectrum,
    look: Look,
}

impl EqEditor {
    pub fn new(rate: f32, scopes: Option<Arc<Scopes>>, look: Look) -> Self {
        let params = Equalizer::new().params();
        Self {
            params,
            values: params.iter().map(|param| param.default).collect(),
            rate,
            selected: None,
            range: 1,
            scopes,
            before: Spectrum::new(rate),
            after: Spectrum::new(rate),
            look,
        }
    }

    pub fn values(&self) -> &[f32] {
        &self.values
    }

    fn get(&self, band: usize, which: Knob) -> f32 {
        self.values[knob(band, which)]
    }

    fn in_use(&self, band: usize) -> bool {
        self.get(band, Knob::On) > 0.5
    }

    fn shape(&self, band: usize) -> BandShape {
        BandShape::from_index(self.get(band, Knob::Shape))
    }

    fn apply(&mut self, changes: &[(usize, f32)]) -> Vec<(usize, f32)> {
        changes
            .iter()
            .filter_map(|(index, value)| {
                let param = self.params.get(*index)?;
                let value = param.clamp(*value);
                (self.values[*index] != value).then(|| {
                    self.values[*index] = value;
                    (*index, value)
                })
            })
            .collect()
    }

    pub fn update(&mut self, message: EqMessage) -> Vec<(usize, f32)> {
        match message {
            EqMessage::Set(changes) => self.apply(&changes),
            EqMessage::Add { band, changes } => {
                self.selected = Some(band);
                self.apply(&changes)
            }
            EqMessage::Remove(band) => {
                if self.selected == Some(band) {
                    self.selected = None;
                }
                self.apply(&[(knob(band, Knob::On), 0.0)])
            }
            EqMessage::Select(band) => {
                self.selected = band;
                Vec::new()
            }
            EqMessage::Range(chosen) => {
                self.range = RANGES.iter().position(|range| *range == chosen).unwrap_or(1);
                Vec::new()
            }
        }
    }

    pub fn tick(&mut self) {
        let Some(scopes) = &self.scopes else {
            return;
        };
        scopes.before.latest(self.before.samples());
        scopes.after.latest(self.after.samples());
        self.before.analyse();
        self.after.analyse();
    }

    pub fn view(&self) -> Element<'_, EqMessage> {
        let graph = canvas::Canvas::new(Graph { editor: self }).width(Length::Fill).height(Length::Fill);
        let look = self.look;
        let bar = container(self.controls()).padding([10, 14]).width(Length::Fill).style(move |_| container::Style {
            background: Some(look.panel.into()),
            ..Default::default()
        });
        column![graph, bar].into()
    }

    fn controls(&self) -> Element<'_, EqMessage> {
        let look = self.look;
        let output = self.values[OUTPUT_KNOB];
        let range = pick_list(&RANGES[..], Some(RANGES[self.range]), EqMessage::Range).text_size(12).padding([4, 8]);
        let output_row = row![
            text("Output").size(12).color(look.text_dim),
            slider(-36.0..=36.0, output, |db| EqMessage::Set(vec![(OUTPUT_KNOB, (db * 10.0).round() / 10.0)]))
                .step(0.1)
                .width(120)
                .style(move |_, _| slider::Style {
                    rail: slider::Rail {
                        backgrounds: (look.curve.into(), look.grid_strong.into()),
                        width: 3.0,
                        border: iced::Border::default().rounded(2),
                    },
                    handle: slider::Handle {
                        shape: slider::HandleShape::Circle { radius: 6.0 },
                        background: look.text.into(),
                        border_width: 0.0,
                        border_color: Color::TRANSPARENT,
                    },
                }),
            text(format!("{output:+.1} dB")).size(12).width(64),
            range,
        ]
        .spacing(10)
        .align_y(Alignment::Center);

        let Some(band) = self.selected.filter(|band| self.in_use(*band)) else {
            let hint = text("Double click the graph to add a band. Drag a node to move it, scroll on it for Q, right click to remove.")
                .size(12)
                .color(look.text_dim);
            return row![hint, horizontal_space(), output_row].align_y(Alignment::Center).into();
        };
        let shape = self.shape(band);
        let dot = container(text("")).width(10).height(10).style(move |_| container::Style {
            background: Some(look.band(band).into()),
            border: iced::Border::default().rounded(5),
            ..Default::default()
        });
        let choose = |which: Knob, choices: &'static [&'static str]| {
            let current = choices.get(self.get(band, which) as usize).copied();
            pick_list(choices, current, move |picked: &'static str| {
                let index = choices.iter().position(|choice| *choice == picked).unwrap_or(0);
                EqMessage::Set(vec![(knob(band, which), index as f32)])
            })
            .text_size(12)
            .padding([4, 8])
        };
        let mut band_row = row![dot, text(format!("Band {}", band + 1)).size(12), choose(Knob::Shape, &SHAPES)]
            .spacing(10)
            .align_y(Alignment::Center);
        if shape.has_slope() {
            band_row = band_row.push(choose(Knob::Slope, &SLOPES));
        }
        band_row = band_row.push(choose(Knob::Place, &PLACES));
        let mut readout = format!("{}", hertz_text(self.get(band, Knob::Freq)));
        if shape.has_gain() {
            readout.push_str(&format!("   {:+.1} dB", self.get(band, Knob::Gain)));
        }
        readout.push_str(&format!("   Q {:.2}", self.get(band, Knob::Q)));
        band_row = band_row.push(text(readout).size(12).color(look.text_dim));
        let remove = button(text("Remove").size(12)).padding([4, 10]).on_press(EqMessage::Remove(band));
        row![band_row, horizontal_space(), remove, output_row].spacing(16).align_y(Alignment::Center).into()
    }

    fn designs(&self, band: usize) -> ([loupe_stock::Coefficients; 8], usize) {
        band_design(
            self.shape(band),
            self.rate,
            self.get(band, Knob::Freq),
            self.get(band, Knob::Gain),
            self.get(band, Knob::Q),
            self.get(band, Knob::Slope) as usize,
        )
    }

    fn band_audible(&self, band: usize) -> bool {
        self.in_use(band) && !(self.shape(band).has_gain() && self.get(band, Knob::Gain) == 0.0)
    }
}

struct Graph<'a> {
    editor: &'a EqEditor,
}

#[derive(Default)]
struct Grip {
    drag: Option<Drag>,
    last_click: Option<(Instant, Point)>,
    hover: Option<Point>,
    modifiers: keyboard::Modifiers,
}

struct Drag {
    band: usize,
    from: Point,
    hz: f32,
    gain: f32,
    q: f32,
}

impl Graph<'_> {
    fn range_db(&self) -> f32 {
        RANGE_DB[self.editor.range]
    }

    fn x_of(&self, width: f32, hz: f32) -> f32 {
        width * (hz / LOWEST_HZ).ln() / (HIGHEST_HZ / LOWEST_HZ).ln()
    }

    fn hz_at(&self, width: f32, x: f32) -> f32 {
        LOWEST_HZ * (HIGHEST_HZ / LOWEST_HZ).powf((x / width).clamp(0.0, 1.0))
    }

    fn y_of(&self, height: f32, db: f32) -> f32 {
        let half = height / 2.0 - EDGE_PAD;
        height / 2.0 - db / self.range_db() * half
    }

    fn db_at(&self, height: f32, y: f32) -> f32 {
        let half = height / 2.0 - EDGE_PAD;
        (height / 2.0 - y) / half * self.range_db()
    }

    fn node(&self, size: Size, band: usize) -> Point {
        let editor = self.editor;
        let x = self.x_of(size.width, editor.get(band, Knob::Freq));
        let db = if editor.shape(band).has_gain() { editor.get(band, Knob::Gain) } else { 0.0 };
        Point::new(x, self.y_of(size.height, db.clamp(-self.range_db(), self.range_db())))
    }

    fn node_at(&self, size: Size, p: Point) -> Option<usize> {
        (0..BANDS)
            .filter(|band| self.editor.in_use(*band))
            .map(|band| (band, self.node(size, band).distance(p)))
            .filter(|(_, distance)| *distance <= NODE_REACH)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(band, _)| band)
    }

    fn total_db(&self, designs: &[([loupe_stock::Coefficients; 8], usize)], hz: f32) -> f32 {
        let rate = self.editor.rate;
        let bands: f32 = designs.iter().flat_map(|(c, count)| c[..*count].iter()).map(|c| c.response_db(rate, hz)).sum();
        bands + self.editor.values[OUTPUT_KNOB]
    }
}

impl canvas::Program<EqMessage> for Graph<'_> {
    type State = Grip;

    fn update(
        &self,
        grip: &mut Grip,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<EqMessage>) {
        use canvas::event::Status::{Captured, Ignored};
        let size = bounds.size();
        let editor = self.editor;
        if size.width < 2.0 || size.height < EDGE_PAD * 3.0 {
            return (Ignored, None);
        }
        match event {
            canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                grip.modifiers = modifiers;
                (Ignored, None)
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(p) = cursor.position_in(bounds) else {
                    return (Ignored, None);
                };
                if let Some(band) = self.node_at(size, p) {
                    grip.drag = Some(Drag {
                        band,
                        from: p,
                        hz: editor.get(band, Knob::Freq),
                        gain: editor.get(band, Knob::Gain),
                        q: editor.get(band, Knob::Q),
                    });
                    return (Captured, Some(EqMessage::Select(Some(band))));
                }
                let twice = grip.last_click.is_some_and(|(at, from)| at.elapsed() < DOUBLE_CLICK && from.distance(p) < 6.0);
                grip.last_click = Some((Instant::now(), p));
                if !twice {
                    return (Captured, Some(EqMessage::Select(None)));
                }
                grip.last_click = None;
                let Some(band) = (0..BANDS).find(|band| !editor.in_use(*band)) else {
                    return (Captured, None);
                };
                let share = p.x / size.width;
                let shape = if share < EDGE_CUT_SHARE {
                    BandShape::LowCut
                } else if share > 1.0 - EDGE_CUT_SHARE {
                    BandShape::HighCut
                } else {
                    BandShape::Bell
                };
                let gain = if shape.has_gain() { (self.db_at(size.height, p.y) * 10.0).round() / 10.0 } else { 0.0 };
                let changes = vec![
                    (knob(band, Knob::On), 1.0),
                    (knob(band, Knob::Shape), shape.index()),
                    (knob(band, Knob::Freq), self.hz_at(size.width, p.x)),
                    (knob(band, Knob::Gain), gain),
                    (knob(band, Knob::Q), if shape.has_slope() { std::f32::consts::FRAC_1_SQRT_2 } else { 1.0 }),
                    (knob(band, Knob::Slope), 1.0),
                    (knob(band, Knob::Place), 0.0),
                ];
                (Captured, Some(EqMessage::Add { band, changes }))
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                let Some(p) = cursor.position_in(bounds) else {
                    return (Ignored, None);
                };
                match self.node_at(size, p) {
                    Some(band) => (Captured, Some(EqMessage::Remove(band))),
                    None => (Ignored, None),
                }
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                grip.hover = cursor.position_in(bounds);
                let (Some(drag), Some(p)) = (&grip.drag, cursor.position_from(bounds.position())) else {
                    return (if grip.hover.is_some() { Captured } else { Ignored }, None);
                };
                let scale = if grip.modifiers.shift() { FINE } else { 1.0 };
                let dx = (p.x - drag.from.x) * scale;
                let dy = (p.y - drag.from.y) * scale;
                let band = drag.band;
                let hz = drag.hz * (HIGHEST_HZ / LOWEST_HZ).powf(dx / size.width);
                let mut changes = vec![(knob(band, Knob::Freq), hz.clamp(LOWEST_HZ, HIGHEST_HZ))];
                if editor.shape(band).has_gain() {
                    let per_px = self.range_db() / (size.height / 2.0 - EDGE_PAD);
                    let gain = ((drag.gain - dy * per_px) * 10.0).round() / 10.0;
                    changes.push((knob(band, Knob::Gain), gain));
                } else {
                    changes.push((knob(band, Knob::Q), drag.q * 2f32.powf(-dy / Q_DOUBLING_PX)));
                }
                (Captured, Some(EqMessage::Set(changes)))
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => match grip.drag.take() {
                Some(_) => (Captured, None),
                None => (Ignored, None),
            },
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let Some(p) = cursor.position_in(bounds) else {
                    return (Ignored, None);
                };
                let Some(band) = self.node_at(size, p).or(editor.selected.filter(|band| editor.in_use(*band))) else {
                    return (Ignored, None);
                };
                let lines = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => y,
                    mouse::ScrollDelta::Pixels { y, .. } => y / 40.0,
                };
                let q = editor.get(band, Knob::Q) * Q_PER_WHEEL_LINE.powf(lines);
                (Captured, Some(EqMessage::Set(vec![(knob(band, Knob::Q), q)])))
            }
            _ => (Ignored, None),
        }
    }

    fn draw(&self, grip: &Grip, renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let editor = self.editor;
        let look = editor.look;
        let size = bounds.size();
        let mut frame = Frame::new(renderer, size);
        if size.width < 2.0 || size.height < EDGE_PAD * 3.0 {
            return vec![frame.into_geometry()];
        }
        frame.fill_rectangle(Point::ORIGIN, size, look.background);

        for hz in MINOR_HZ {
            let x = self.x_of(size.width, hz).round();
            frame.fill_rectangle(Point::new(x, 0.0), Size::new(1.0, size.height), look.grid);
        }
        for (hz, label) in MAJOR_HZ {
            let x = self.x_of(size.width, hz).round();
            frame.fill_rectangle(Point::new(x, 0.0), Size::new(1.0, size.height), look.grid_strong);
            put_text(&mut frame, label, Point::new(x + 4.0, size.height - 6.0), 11.0, look.text_dim, (0.0, 1.0));
        }
        let range = self.range_db();
        let step = match editor.range {
            0 => 2.0,
            1 => 3.0,
            _ => 6.0,
        };
        let mut db = -range;
        while db <= range + 0.01 {
            let y = self.y_of(size.height, db).round();
            let colour = if db == 0.0 { look.grid_strong } else { look.grid };
            frame.fill_rectangle(Point::new(0.0, y), Size::new(size.width, 1.0), colour);
            if db != 0.0 && db.abs() < range + 0.01 {
                put_text(&mut frame, &format!("{db:+.0}"), Point::new(size.width - 6.0, y - 2.0), 10.5, look.text_dim, (1.0, 1.0));
            }
            db += step;
        }

        if editor.scopes.is_some() {
            for (spectrum, colour) in [(&editor.before, look.spectrum_before), (&editor.after, look.spectrum)] {
                let shape = Path::new(|b| {
                    b.move_to(Point::new(0.0, size.height));
                    let mut x = 0.0;
                    while x <= size.width {
                        let from = self.hz_at(size.width, x);
                        let to = self.hz_at(size.width, x + COLUMN_PX);
                        let level = spectrum.level_at(from, to);
                        let share = (SPECTRUM_CEILING_DB - level) / (SPECTRUM_CEILING_DB - SPECTRUM_FLOOR_DB);
                        b.line_to(Point::new(x, (share * size.height).clamp(0.0, size.height)));
                        x += COLUMN_PX;
                    }
                    b.line_to(Point::new(size.width, size.height));
                    b.close();
                });
                frame.fill(&shape, colour);
            }
        }

        let audible: Vec<usize> = (0..BANDS).filter(|band| editor.band_audible(*band)).collect();
        let designs: Vec<_> = audible.iter().map(|band| editor.designs(*band)).collect();
        let curve_of = |designs: &[([loupe_stock::Coefficients; 8], usize)], output: f32| {
            let mut points = Vec::with_capacity(size.width as usize + 2);
            let mut x = 0.0;
            while x <= size.width + 1.0 {
                let db = self.total_db(designs, self.hz_at(size.width, x)) - editor.values[OUTPUT_KNOB] + output;
                let y = self.y_of(size.height, db);
                let y = if y.is_nan() { size.height + 4.0 } else { y.clamp(-4.0, size.height + 4.0) };
                points.push(Point::new(x, y));
                x += 1.0;
            }
            points
        };
        let zero = self.y_of(size.height, 0.0);
        for (slot, band) in audible.iter().enumerate() {
            let points = curve_of(std::slice::from_ref(&designs[slot]), 0.0);
            let colour = look.band(*band);
            let selected = editor.selected == Some(*band);
            if selected {
                let area = Path::new(|b| {
                    b.move_to(Point::new(0.0, zero));
                    for point in &points {
                        b.line_to(*point);
                    }
                    b.line_to(Point::new(size.width + 1.0, zero));
                    b.close();
                });
                frame.fill(&area, fade(colour, 0.18));
            }
            let line = Path::new(|b| {
                b.move_to(points[0]);
                for point in &points[1..] {
                    b.line_to(*point);
                }
            });
            frame.stroke(&line, Stroke::default().with_color(fade(colour, if selected { 0.9 } else { 0.35 })).with_width(1.0));
        }
        let total = curve_of(&designs, editor.values[OUTPUT_KNOB]);
        let line = Path::new(|b| {
            b.move_to(total[0]);
            for point in &total[1..] {
                b.line_to(*point);
            }
        });
        frame.stroke(&line, Stroke::default().with_color(look.curve).with_width(2.0));

        let hovered = grip.hover.and_then(|p| self.node_at(size, p));
        for band in (0..BANDS).filter(|band| editor.in_use(*band)) {
            let at = self.node(size, band);
            let colour = look.band(band);
            let lit = hovered == Some(band) || grip.drag.as_ref().is_some_and(|drag| drag.band == band);
            let radius = if lit { NODE_RADIUS + 1.5 } else { NODE_RADIUS };
            let dot = Path::circle(at, radius);
            frame.fill(&dot, if editor.band_audible(band) { colour } else { fade(colour, 0.45) });
            if editor.selected == Some(band) {
                frame.stroke(&Path::circle(at, radius + 3.0), Stroke::default().with_color(look.text).with_width(1.5));
            }
            let place = editor.get(band, Knob::Place) as usize;
            if place > 0 {
                put_text(&mut frame, ["", "L", "R", "M", "S"][place.min(4)], at, 9.5, Color::BLACK, (0.5, 0.5));
            }
        }

        let told = grip.drag.as_ref().map(|drag| drag.band).or(hovered);
        if let Some(band) = told {
            let at = self.node(size, band);
            let mut label = hertz_text(editor.get(band, Knob::Freq));
            if editor.shape(band).has_gain() {
                label.push_str(&format!("  {:+.1} dB", editor.get(band, Knob::Gain)));
            }
            label.push_str(&format!("  Q {:.2}", editor.get(band, Knob::Q)));
            let width = label.chars().count() as f32 * 6.6 + 16.0;
            let left = (at.x - width / 2.0).clamp(4.0, size.width - width - 4.0);
            let top = if at.y > 40.0 { at.y - 34.0 } else { at.y + 16.0 };
            let pill = Path::rounded_rectangle(Point::new(left, top), Size::new(width, 20.0), 5.0.into());
            frame.fill(&pill, fade(look.panel, 0.95));
            put_text(&mut frame, &label, Point::new(left + width / 2.0, top + 10.0), 11.5, look.text, (0.5, 0.5));
        }

        if !(0..BANDS).any(|band| editor.in_use(band)) {
            let middle = Point::new(size.width / 2.0, size.height / 2.0 - 22.0);
            put_text(&mut frame, "Double click to add a band", middle, 13.0, look.text_dim, (0.5, 0.5));
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, grip: &Grip, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if grip.drag.is_some() {
            return mouse::Interaction::Grabbing;
        }
        match cursor.position_in(bounds).and_then(|p| self.node_at(bounds.size(), p)) {
            Some(_) => mouse::Interaction::Grab,
            None => mouse::Interaction::default(),
        }
    }
}

fn put_text(frame: &mut Frame, content: &str, at: Point, size: f32, color: Color, anchor: (f32, f32)) {
    let across = match anchor.0 {
        a if a <= 0.0 => alignment::Horizontal::Left,
        a if a >= 1.0 => alignment::Horizontal::Right,
        _ => alignment::Horizontal::Center,
    };
    let down = match anchor.1 {
        a if a <= 0.0 => alignment::Vertical::Top,
        a if a >= 1.0 => alignment::Vertical::Bottom,
        _ => alignment::Vertical::Center,
    };
    let text = Text {
        content: content.to_string(),
        position: Point::new(at.x.round(), at.y.round()),
        color,
        size: size.into(),
        horizontal_alignment: across,
        vertical_alignment: down,
        ..Text::default()
    };
    text.draw_with(|glyph, colour| frame.fill(&glyph, colour));
}

fn hertz_text(hz: f32) -> String {
    if hz >= 1000.0 {
        format!("{:.2} kHz", hz / 1000.0)
    } else {
        format!("{hz:.0} Hz")
    }
}
