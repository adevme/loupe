use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::widget::text::Wrapping;
use iced::widget::{column, container, pick_list, row, text, Space};
use iced::{keyboard, mouse, Alignment, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};
use loupe_stock::{band_design, dynamic_knob, knob, BandShape, Dynamic, Effect, Equalizer, Knob, Param, Scopes, ATTACK_KNOB, BANDS, OUTPUT_KNOB, PLACES, RELEASE_KNOB, SHAPES, SLOPES};

use crate::look::{fade, mix, Look, MEDIUM, SEMIBOLD};
use crate::kit::*;
use crate::spectrum::Spectrum;

const LOWEST_HZ: f32 = 10.0;
const HIGHEST_HZ: f32 = 30_000.0;
const NODE_RADIUS: f32 = 6.5;
const NODE_REACH: f32 = 12.0;
const RANGE_BAR: f32 = 3.0;
const RANGE_WIDTH: f32 = 96.0;
const FIELD_LIST_WIDTH: f32 = 116.0;
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
    Typing { knob: usize, text: String },
    Typed(usize),
}

pub struct EqEditor {
    typing: Option<(usize, String)>,
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
        let look = look.tinted(crate::look::accents::EQ);
        let params = Equalizer::new().params();
        Self {
            params,
            typing: None,
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

    pub fn load(&mut self, values: &[f32]) {
        for (slot, value) in values.iter().enumerate().take(self.values.len()) {
            self.values[slot] = *value;
        }
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

    fn dynamic(&self, band: usize) -> bool {
        self.values[dynamic_knob(band, Dynamic::On)] > 0.5 && self.shape(band).has_gain()
    }

    fn moved_db(&self, band: usize) -> f32 {
        match (&self.scopes, self.dynamic(band)) {
            (Some(scopes), true) => scopes.moved_db(band),
            _ => 0.0,
        }
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
            EqMessage::Typing { knob, text } => {
                self.typing = Some((knob, text));
                Vec::new()
            }
            EqMessage::Typed(knob) => {
                let Some((which, text)) = self.typing.take() else {
                    return Vec::new();
                };
                if which != knob {
                    return Vec::new();
                }
                match crate::kit::read_value(&text, &self.params[knob]) {
                    Some(value) => self.apply(&[(knob, value)]),
                    None => Vec::new(),
                }
            }
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
        let look = self.look;
        iced::widget::responsive(move |size| {
            let compact = size.height < COMPACT_BELOW;
            let graph = canvas::Canvas::new(Graph { editor: self }).width(Length::Fill).height(Length::Fill);
            let mut screen = iced::widget::Stack::new().push(graph).width(Length::Fill).height(Length::Fill);
            if let Some(band) = self.selected.filter(|band| self.in_use(*band)) {
                let (panel, width) = self.band_panel(band, compact);
                let graph_width = (size.width - 24.0).max(1.0);
                let node_x = graph_width * (self.get(band, Knob::Freq) / LOWEST_HZ).ln() / (HIGHEST_HZ / LOWEST_HZ).ln();
                let left = (node_x - width / 2.0).clamp(10.0, (graph_width - width - 10.0).max(10.0));
                let placed = container(panel)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_y(iced::alignment::Vertical::Bottom)
                    .padding(iced::Padding { top: 0.0, right: 0.0, bottom: 12.0, left });
                screen = screen.push(placed);
            }
            column![header(look, "EQ", "Parametric equalizer"), display(look, screen), deck_plate(look, self.bar(compact))].into()
        })
        .into()
    }

    fn bar(&self, compact: bool) -> Element<'_, EqMessage> {
        let look = self.look;
        let range = pick_list(&RANGES[..], Some(RANGES[self.range]), EqMessage::Range)
            .text_size(12)
            .font(MEDIUM)
            .padding([5, 10])
            .width(RANGE_WIDTH)
            .style(field_style(look))
            .menu_style(menu_style(look));
        let used = (0..BANDS).filter(|band| self.in_use(*band)).count();
        let hint = if used == 0 {
            "Double click the display to add a band".to_string()
        } else {
            format!("{used} {} · drag a node to move it · scroll for Q · right click removes", if used == 1 { "band" } else { "bands" })
        };
        let p = Panel::new(look, compact, self.params, &self.values, set_one);
        row![
            text(hint).size(11.5).font(MEDIUM).color(look.text_faint).wrapping(Wrapping::None),
            Space::with_width(Length::Fill),
            caption(look, "Range"),
            range,
            Space::with_width(18),
            p.mini(OUTPUT_KNOB, "Output").element,
        ]
        .spacing(10)
        .align_y(Alignment::Center)
        .into()
    }

    fn band_panel(&self, band: usize, compact: bool) -> (Element<'_, EqMessage>, f32) {
        let look = self.look;
        let shape = self.shape(band);
        let colour = look.band(band);
        let p = Panel::new(look, compact, self.params, &self.values, set_one);
        let dot = container(Space::new(9, 9)).style(move |_| container::Style {
            background: Some(colour.into()),
            border: iced::Border::default().rounded(5),
            shadow: iced::Shadow { color: fade(colour, 0.7), offset: iced::Vector::ZERO, blur_radius: 7.0 },
            ..Default::default()
        });
        let choose = |which: Knob, choices: &'static [&'static str]| -> Element<'_, EqMessage> {
            let current = choices.get(self.get(band, which) as usize).copied();
            pick_list(choices, current, move |picked: &'static str| {
                let index = choices.iter().position(|choice| *choice == picked).unwrap_or(0);
                EqMessage::Set(vec![(knob(band, which), index as f32)])
            })
            .text_size(11.5)
            .font(MEDIUM)
            .padding([4, 9])
            .width(FIELD_LIST_WIDTH)
            .style(field_style(look))
            .menu_style(menu_style(look))
            .into()
        };
        let mut lists = column![row![dot, text(format!("BAND {}", band + 1)).size(10.5).font(SEMIBOLD).color(look.text)].spacing(8).align_y(Alignment::Center), choose(Knob::Shape, &SHAPES)].spacing(7);
        if shape.has_slope() {
            lists = lists.push(choose(Knob::Slope, &SLOPES));
        }
        lists = lists.push(choose(Knob::Place, &PLACES));
        let typed = |which: Knob, label: &'static str, shown: String| -> (Element<'_, EqMessage>, f32) {
            let slot = knob(band, which);
            let dial = p.sized(slot, KnobSize::Regular, Some(label), false);
            let showing = match &self.typing {
                Some((at, text)) if *at == slot => text.clone(),
                _ => shown,
            };
            let input = iced::widget::text_input("", &showing)
                .size(11.5)
                .font(MEDIUM)
                .padding([3, 4])
                .width(dial.width)
                .align_x(iced::alignment::Horizontal::Center)
                .on_input(move |text| EqMessage::Typing { knob: slot, text })
                .on_submit(EqMessage::Typed(slot))
                .style(move |_, status| iced::widget::text_input::Style {
                    background: match status {
                        iced::widget::text_input::Status::Focused => Color::from_rgba(0.0, 0.0, 0.0, 0.4).into(),
                        iced::widget::text_input::Status::Hovered => Color::from_rgba(0.0, 0.0, 0.0, 0.22).into(),
                        _ => Color::TRANSPARENT.into(),
                    },
                    border: iced::Border { color: if matches!(status, iced::widget::text_input::Status::Focused) { fade(look.accent, 0.5) } else { Color::TRANSPARENT }, width: 1.0, radius: 4.0.into() },
                    icon: look.text_dim,
                    placeholder: look.text_dim,
                    value: mix(look.text_dim, look.text, 0.7),
                    selection: fade(look.accent, 0.4),
                });
            (column![dial.element, input].spacing(0).align_x(Alignment::Center).into(), dial.width)
        };
        let mut width = FIELD_LIST_WIDTH + 28.0;
        let mut typed_knobs = vec![typed(Knob::Freq, "Freq", hertz(self.get(band, Knob::Freq)))];
        if shape.has_gain() {
            typed_knobs.push(typed(Knob::Gain, "Gain", format!("{:+.1} dB", self.get(band, Knob::Gain))));
        }
        typed_knobs.push(typed(Knob::Q, "Q", format!("{:.2}", self.get(band, Knob::Q))));
        width += typed_knobs.iter().map(|(_, w)| w + 4.0).sum::<f32>();
        let knobs = row(typed_knobs.into_iter().map(|(element, _)| element)).spacing(4).align_y(Alignment::End);
        let dynamic_on = self.dynamic(band);
        let mut more = row![].spacing(4).align_y(Alignment::End);
        if dynamic_on {
            for part in [p.named(dynamic_knob(band, Dynamic::Threshold), "Threshold"), p.named(dynamic_knob(band, Dynamic::Range), "Range"), p.named(ATTACK_KNOB, "Attack"), p.named(RELEASE_KNOB, "Release")] {
                width += part.width + 4.0;
                more = more.push(part.element);
            }
            width += 24.0;
        }
        let mut keys = column![].spacing(8).align_x(Alignment::Center);
        let mut keys_width: f32 = 0.0;
        if shape.has_gain() {
            let toggle = toggle(look, "Dynamic", dynamic_on, EqMessage::Set(vec![(dynamic_knob(band, Dynamic::On), if dynamic_on { 0.0 } else { 1.0 })]));
            keys_width = keys_width.max(toggle.width);
            keys = keys.push(toggle.element);
            if dynamic_on {
                keys = keys.push(text(format!("now {:+.1} dB", self.moved_db(band))).size(11).font(MEDIUM).color(look.text_dim).wrapping(Wrapping::None));
            }
        }
        let remove = push(look, "Remove", Some(EqMessage::Remove(band)));
        keys_width = keys_width.max(remove.width);
        keys = keys.push(remove.element);
        width += keys_width + 24.0 + 32.0;
        let mut body = row![lists, panel_rule(look), knobs].spacing(12).align_y(Alignment::Center);
        if dynamic_on {
            body = body.push(panel_rule(look)).push(more);
        }
        body = body.push(panel_rule(look)).push(keys);
        let panel = container(body).padding([10, 14]).style(move |_| container::Style {
            background: Some(iced::gradient::Linear::new(iced::Radians(std::f32::consts::PI)).add_stop(0.0, Color::from_rgba(0.13, 0.135, 0.155, 0.96)).add_stop(1.0, Color::from_rgba(0.085, 0.09, 0.105, 0.96)).into()),
            border: iced::Border { color: Color::from_rgba(0.0, 0.0, 0.0, 0.7), width: 1.0, radius: 10.0.into() },
            shadow: iced::Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.55), offset: iced::Vector::new(0.0, 6.0), blur_radius: 22.0 },
            ..Default::default()
        });
        (panel.into(), width)
    }

    fn designs(&self, band: usize) -> ([loupe_stock::Coefficients; 8], usize) {
        band_design(
            self.shape(band),
            self.rate,
            self.get(band, Knob::Freq),
            self.get(band, Knob::Gain) + self.moved_db(band),
            self.get(band, Knob::Q),
            self.get(band, Knob::Slope) as usize,
        )
    }

    fn band_audible(&self, band: usize) -> bool {
        self.in_use(band) && !(self.shape(band).has_gain() && self.get(band, Knob::Gain) == 0.0 && !self.dynamic(band))
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
        if size.width < 2.0 || size.height < EDGE_PAD * 3.0 {
            return vec![Frame::new(renderer, size).into_geometry()];
        }
        let mut frame = graph_frame(renderer, size, &look);

        for hz in MINOR_HZ {
            let x = self.x_of(size.width, hz).round();
            frame.fill_rectangle(Point::new(x, 0.0), Size::new(1.0, size.height), look.grid);
        }
        for (hz, label) in MAJOR_HZ {
            let x = self.x_of(size.width, hz).round();
            frame.fill_rectangle(Point::new(x, 0.0), Size::new(1.0, size.height), look.grid_strong);
            axis_text(&mut frame, label, Point::new(x + 4.0, size.height - 6.0), &look, (0.0, 1.0));
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
            let colour = if db == 0.0 { fade(look.text, 0.16) } else { look.grid };
            frame.fill_rectangle(Point::new(0.0, y), Size::new(size.width, 1.0), colour);
            if db.abs() < range + 0.01 {
                let label = if db == 0.0 { "0".to_string() } else { format!("{db:+.0}") };
                axis_text(&mut frame, &label, Point::new(size.width - 8.0, y), &look, (1.0, 0.5));
            }
            db += step;
        }

        if editor.scopes.is_some() {
            for (spectrum, colour, line) in [(&editor.before, look.spectrum_before, None), (&editor.after, look.spectrum, Some(fade(look.spectrum, 0.55)))] {
                let mut points = Vec::with_capacity((size.width / COLUMN_PX) as usize + 2);
                let mut x = 0.0;
                while x <= size.width {
                    let from = self.hz_at(size.width, x);
                    let to = self.hz_at(size.width, x + COLUMN_PX);
                    let level = spectrum.level_at(from, to);
                    let share = (SPECTRUM_CEILING_DB - level) / (SPECTRUM_CEILING_DB - SPECTRUM_FLOOR_DB);
                    points.push(Point::new(x, (share * size.height).clamp(0.0, size.height)));
                    x += COLUMN_PX;
                }
                let shape = Path::new(|b| {
                    b.move_to(Point::new(0.0, size.height));
                    for point in &points {
                        b.line_to(*point);
                    }
                    b.line_to(Point::new(size.width, size.height));
                    b.close();
                });
                let top = points.iter().map(|p| p.y).fold(size.height, f32::min);
                let fill = canvas::gradient::Linear::new(Point::new(0.0, top), Point::new(0.0, size.height)).add_stop(0.0, colour).add_stop(1.0, fade(colour, colour.a * 0.25));
                frame.fill(&shape, fill);
                if let Some(line) = line {
                    frame.stroke(&polyline(&points), Stroke::default().with_color(line).with_width(1.0));
                }
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
        let area_to_zero = |points: &[Point]| {
            Path::new(|b| {
                b.move_to(Point::new(0.0, zero));
                for point in points {
                    b.line_to(*point);
                }
                b.line_to(Point::new(size.width + 1.0, zero));
                b.close();
            })
        };
        for (slot, band) in audible.iter().enumerate() {
            let points = curve_of(std::slice::from_ref(&designs[slot]), 0.0);
            let colour = look.band(*band);
            let selected = editor.selected == Some(*band);
            frame.fill(&area_to_zero(&points), fade(colour, if selected { 0.2 } else { 0.06 }));
            frame.stroke(&polyline(&points), Stroke::default().with_color(fade(colour, if selected { 0.95 } else { 0.4 })).with_width(if selected { 1.4 } else { 1.0 }));
        }
        let total = curve_of(&designs, editor.values[OUTPUT_KNOB]);
        let line = polyline(&total);
        frame.stroke(&line, Stroke::default().with_color(fade(look.curve, 0.06)).with_width(8.0));
        frame.stroke(&line, Stroke::default().with_color(fade(look.curve, 0.13)).with_width(4.0));
        frame.stroke(&line, Stroke::default().with_color(look.curve).with_width(2.0));

        let hovered = grip.hover.and_then(|p| self.node_at(size, p));
        for band in (0..BANDS).filter(|band| editor.in_use(*band)) {
            let at = self.node(size, band);
            let colour = look.band(band);
            let lit = hovered == Some(band) || grip.drag.as_ref().is_some_and(|drag| drag.band == band);
            let radius = if lit { NODE_RADIUS + 1.5 } else { NODE_RADIUS };
            if editor.dynamic(band) {
                let gain = editor.get(band, Knob::Gain);
                let reach = self.range_db();
                let y_at = |db: f32| self.y_of(size.height, db.clamp(-reach, reach));
                let end = y_at(gain + editor.values[dynamic_knob(band, Dynamic::Range)]);
                let bar = Path::rounded_rectangle(Point::new(at.x - RANGE_BAR / 2.0, at.y.min(end)), Size::new(RANGE_BAR, (end - at.y).abs()), (RANGE_BAR / 2.0).into());
                frame.fill(&bar, fade(colour, 0.45));
                frame.stroke(&Path::circle(Point::new(at.x, y_at(gain + editor.moved_db(band))), NODE_RADIUS - 2.0), Stroke::default().with_color(colour).with_width(1.5));
            }
            let selected = editor.selected == Some(band);
            if selected || lit {
                frame.fill(&Path::circle(at, radius + 7.0), fade(colour, 0.14));
            }
            frame.fill(&Path::circle(Point::new(at.x, at.y + 1.5), radius + 1.0), Color::from_rgba(0.0, 0.0, 0.0, 0.5));
            let body = if editor.band_audible(band) { colour } else { fade(colour, 0.45) };
            let top = Point::new(at.x, at.y - radius);
            let bottom = Point::new(at.x, at.y + radius);
            frame.fill(&Path::circle(at, radius), canvas::gradient::Linear::new(top, bottom).add_stop(0.0, crate::look::lift(body, 0.12)).add_stop(1.0, body));
            frame.stroke(&Path::circle(at, radius - 0.5), Stroke::default().with_color(Color::from_rgba(0.0, 0.0, 0.0, 0.35)).with_width(1.0));
            if selected {
                frame.stroke(&Path::circle(at, radius + 3.0), Stroke::default().with_color(look.text).with_width(1.5));
            }
            let place = editor.get(band, Knob::Place) as usize;
            if place > 0 {
                put_text_in(&mut frame, ["", "L", "R", "M", "S"][place.min(4)], at, 9.5, Color::from_rgb(0.08, 0.08, 0.09), (0.5, 0.5), SEMIBOLD);
            }
        }

        let told = grip.drag.as_ref().map(|drag| drag.band).or(hovered);
        if let Some(band) = told {
            let at = self.node(size, band);
            let mut label = hertz(editor.get(band, Knob::Freq));
            if editor.shape(band).has_gain() {
                label.push_str(&format!("   {:+.1} dB", editor.get(band, Knob::Gain)));
            }
            label.push_str(&format!("   Q {:.2}", editor.get(band, Knob::Q)));
            let width = label.chars().count() as f32 * 6.4 + 22.0;
            let left = (at.x - width / 2.0).clamp(6.0, size.width - width - 6.0);
            let top = if at.y > 44.0 { at.y - 38.0 } else { at.y + 18.0 };
            let pill = Path::rounded_rectangle(Point::new(left, top), Size::new(width, 22.0), 6.0.into());
            frame.fill(&Path::rounded_rectangle(Point::new(left, top + 2.0), Size::new(width, 22.0), 6.0.into()), Color::from_rgba(0.0, 0.0, 0.0, 0.35));
            frame.fill(&pill, fade(look.raised, 0.97));
            frame.stroke(&pill, Stroke::default().with_color(fade(look.band(band), 0.45)).with_width(1.0));
            put_text_in(&mut frame, &label, Point::new(left + width / 2.0, top + 11.0), 11.0, look.text, (0.5, 0.5), MEDIUM);
        }

        if !(0..BANDS).any(|band| editor.in_use(band)) {
            let middle = Point::new(size.width / 2.0, size.height / 2.0 - 24.0);
            put_text_in(&mut frame, "Double click to add a band", middle, 13.0, look.text_dim, (0.5, 0.5), MEDIUM);
        }
        seal(&mut frame, size, &look);
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



fn set_one(index: usize, value: f32) -> EqMessage {
    EqMessage::Set(vec![(index, value)])
}

fn panel_rule<'a>(look: Look) -> Element<'a, EqMessage> {
    container(Space::new(1, 84))
        .style(move |_| container::Style {
            background: Some(
                iced::gradient::Linear::new(iced::Radians(std::f32::consts::PI))
                    .add_stop(0.0, fade(look.text, 0.0))
                    .add_stop(0.5, fade(look.text, 0.09))
                    .add_stop(1.0, fade(look.text, 0.0))
                    .into(),
            ),
            ..Default::default()
        })
        .into()
}
