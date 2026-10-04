use std::fs;
use std::path::PathBuf;

use iced::widget::{button, container, slider, text_input};
use iced::{font, Background, Border, Color, Font, Theme};

use crate::icons;
use crate::settings::{config_dir, entries};

pub const MIN_TRACK_HEIGHT: f32 = 40.0;
pub const MAX_TRACK_HEIGHT: f32 = 400.0;
pub const BAR_SLOTS: usize = 24;
const POSTSCRIPT_OUTLINES: &[u8] = b"OTTO";
const THEME_EXTENSION: &str = "theme";
pub const REFERENCE_FILE: &str = "default.theme";
const BUNDLED: [(&str, &str); 10] = [
    ("Arctic", include_str!("../themes/Arctic.theme")),
    ("Ember", include_str!("../themes/Ember.theme")),
    ("Forest", include_str!("../themes/Forest.theme")),
    ("Graphite", include_str!("../themes/Graphite.theme")),
    ("High Contrast", include_str!("../themes/High Contrast.theme")),
    ("Midnight", include_str!("../themes/Midnight.theme")),
    ("Ocean", include_str!("../themes/Ocean.theme")),
    ("Paper", include_str!("../themes/Paper.theme")),
    ("Rose", include_str!("../themes/Rose.theme")),
    ("Studio", include_str!("../themes/Studio.theme")),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayheadCap {
    Triangle,
    Square,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmShape {
    Dot,
    Square,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarItem {
    ToStart,
    Play,
    Record,
    Position,
    Clock,
    Tempo,
    Master,
    History,
    Mixer,
    Settings,
    Import,
    Gap,
    Space,
    End,
}

const BAR_NAMES: [(&str, BarItem); 13] = [
    ("to_start", BarItem::ToStart),
    ("play", BarItem::Play),
    ("record", BarItem::Record),
    ("position", BarItem::Position),
    ("clock", BarItem::Clock),
    ("tempo", BarItem::Tempo),
    ("master", BarItem::Master),
    ("history", BarItem::History),
    ("mixer", BarItem::Mixer),
    ("settings", BarItem::Settings),
    ("import", BarItem::Import),
    ("gap", BarItem::Gap),
    ("space", BarItem::Space),
];

#[derive(Clone, Copy)]
pub struct Palette {
    pub background: Color,
    pub panel: Color,
    pub raised: Color,
    pub hover: Color,
    pub line: Color,
    pub grid: Color,
    pub text: Color,
    pub text_dim: Color,
    pub text_faint: Color,
    pub accent: Color,
    pub on_accent: Color,
    pub danger: Color,
    pub meter_low: Color,
    pub meter_mid: Color,
    pub meter_high: Color,
    pub tracks: [Color; 8],
    pub ui: Font,
    pub medium: Font,
    pub semibold: Font,
    pub mono: Font,
    pub track_height: f32,
    pub header_width: f32,
    pub ruler_height: f32,
    pub scrollbar_height: f32,
    pub top_bar_height: f32,
    pub clip_title_height: f32,
    pub clip_padding: f32,
    pub meter_thickness: f32,
    pub clip_title_size: f32,
    pub track_title_size: f32,
    pub ruler_text_size: f32,
    pub corner: f32,
    pub solid_corner: f32,
    pub sheet_corner: f32,
    pub clip_corner: f32,
    pub border_width: f32,
    pub playhead_width: f32,
    pub handle_radius: f32,
    pub fade_flag_size: f32,
    pub playhead_cap: PlayheadCap,
    pub arm_shape: ArmShape,
    pub headers: Side,
    pub all_audio: Side,
    pub top_bar: [BarItem; BAR_SLOTS],
}

const NEUTRAL: Palette = Palette {
    background: Color::from_rgb(0.2, 0.2, 0.208),
    panel: Color::from_rgb(0.157, 0.157, 0.165),
    raised: Color::from_rgb(0.25, 0.25, 0.26),
    hover: Color::from_rgb(0.31, 0.31, 0.322),
    line: Color::from_rgb(0.11, 0.11, 0.118),
    grid: Color::from_rgb(0.37, 0.37, 0.384),
    text: Color::from_rgb(0.925, 0.925, 0.933),
    text_dim: Color::from_rgb(0.67, 0.67, 0.69),
    text_faint: Color::from_rgb(0.47, 0.47, 0.49),
    accent: Color::from_rgb(0.925, 0.925, 0.933),
    on_accent: Color::from_rgb(0.11, 0.11, 0.118),
    danger: Color::from_rgb(0.984, 0.443, 0.522),
    meter_low: Color::from_rgb(0.33, 0.84, 0.5),
    meter_mid: Color::from_rgb(0.96, 0.84, 0.33),
    meter_high: Color::from_rgb(0.97, 0.6, 0.27),
    tracks: [
        Color::from_rgb(0.45, 0.72, 0.95),
        Color::from_rgb(0.73, 0.58, 0.96),
        Color::from_rgb(0.42, 0.82, 0.68),
        Color::from_rgb(0.96, 0.6, 0.55),
        Color::from_rgb(0.93, 0.78, 0.45),
        Color::from_rgb(0.55, 0.62, 0.97),
        Color::from_rgb(0.4, 0.8, 0.85),
        Color::from_rgb(0.9, 0.55, 0.8),
    ],
    ui: Font::with_name("Inter"),
    medium: Font { weight: font::Weight::Medium, ..Font::with_name("Inter") },
    semibold: Font { weight: font::Weight::Semibold, ..Font::with_name("Inter") },
    mono: Font::with_name("JetBrains Mono"),
    track_height: 92.0,
    header_width: 200.0,
    ruler_height: 30.0,
    scrollbar_height: 18.0,
    top_bar_height: 52.0,
    clip_title_height: 21.0,
    clip_padding: 5.0,
    meter_thickness: 3.0,
    clip_title_size: 13.0,
    track_title_size: 13.0,
    ruler_text_size: 11.0,
    corner: 6.0,
    solid_corner: 16.0,
    sheet_corner: 10.0,
    clip_corner: 5.0,
    border_width: 1.0,
    playhead_width: 1.0,
    handle_radius: 4.5,
    fade_flag_size: 10.0,
    playhead_cap: PlayheadCap::Triangle,
    arm_shape: ArmShape::Dot,
    headers: Side::Left,
    all_audio: Side::Right,
    top_bar: [
        BarItem::Master,
        BarItem::ToStart,
        BarItem::Play,
        BarItem::Record,
        BarItem::Space,
        BarItem::Position,
        BarItem::Clock,
        BarItem::Tempo,
        BarItem::Gap,
        BarItem::History,
        BarItem::Mixer,
        BarItem::Settings,
        BarItem::Import,
        BarItem::End,
        BarItem::End,
        BarItem::End,
        BarItem::End,
        BarItem::End,
        BarItem::End,
        BarItem::End,
        BarItem::End,
        BarItem::End,
        BarItem::End,
        BarItem::End,
    ],
};

const COLOUR_KEYS: [&str; 15] = [
    "background",
    "panel",
    "raised",
    "hover",
    "line",
    "grid",
    "text",
    "text_dim",
    "text_faint",
    "accent",
    "on_accent",
    "danger",
    "meter_low",
    "meter_mid",
    "meter_high",
];

const SIZE_KEYS: [(&str, f32, f32); 10] = [
    ("track_height", MIN_TRACK_HEIGHT, MAX_TRACK_HEIGHT),
    ("header_width", 140.0, 480.0),
    ("ruler_height", 18.0, 64.0),
    ("scrollbar_height", 10.0, 40.0),
    ("top_bar_height", 40.0, 96.0),
    ("clip_title_height", 14.0, 40.0),
    ("clip_padding", 0.0, 16.0),
    ("clip_title_size", 8.0, 24.0),
    ("track_title_size", 8.0, 24.0),
    ("ruler_text_size", 8.0, 24.0),
];

const SHAPE_KEYS: [(&str, f32, f32); 9] = [
    ("corner", 0.0, 16.0),
    ("solid_corner", 0.0, 16.0),
    ("sheet_corner", 0.0, 24.0),
    ("clip_corner", 0.0, 16.0),
    ("border_width", 0.0, 3.0),
    ("playhead_width", 1.0, 4.0),
    ("handle_radius", 3.0, 9.0),
    ("fade_flag_size", 6.0, 18.0),
    ("meter_thickness", 1.0, 12.0),
];

pub struct Loaded {
    pub palette: Palette,
    pub problem: Option<String>,
    pub icon_font: Option<Vec<u8>>,
}

impl Palette {
    pub fn load(chosen: Option<&str>) -> Loaded {
        let plain = |problem| {
            icons::choose(None, Vec::new());
            Loaded { palette: NEUTRAL, problem, icon_font: None }
        };
        let Some(themes) = folder() else {
            return plain(None);
        };
        let _ = fs::create_dir_all(&themes);
        let _ = fs::write(themes.join(REFERENCE_FILE), NEUTRAL.to_text());
        for (name, text) in BUNDLED {
            let file = themes.join(format!("{name}.{THEME_EXTENSION}"));
            if !file.exists() {
                let _ = fs::write(file, text);
            }
        }
        let Some(name) = chosen else {
            return plain(None);
        };
        let path = themes.join(format!("{name}.{THEME_EXTENSION}"));
        let Ok(text) = fs::read_to_string(&path) else {
            return plain(Some(format!("Theme \"{name}\" is not at {}", path.display())));
        };
        let mut palette = NEUTRAL;
        let mut problem = None;
        let mut icon_file = None;
        let mut icon_family = None;
        let mut glyphs = Vec::new();
        for (line, key, value) in entries(&text) {
            let read = match (key, key.strip_prefix("icon_")) {
                ("icon_file", _) => {
                    icon_file = Some(value);
                    Ok(())
                }
                ("icon_family", _) => {
                    icon_family = Some(value);
                    Ok(())
                }
                (_, Some(icon)) => icons::code(icon, value).map(|glyph| glyphs.push((icon.to_string(), glyph))),
                _ => palette.set(key, value),
            };
            if let Err(why) = read {
                problem = Some(format!("Theme \"{name}\" line {line}: {why}"));
                break;
            }
        }
        let icon_font = match (icon_file, icon_family) {
            (Some(file), Some(_)) => match fs::read(themes.join(file)) {
                Ok(bytes) if bytes.starts_with(POSTSCRIPT_OUTLINES) => {
                    problem.get_or_insert(format!(
                        "Theme \"{name}\": icon file {file} has PostScript outlines, which Loupe cannot draw. Use a TrueType (.ttf) font"
                    ));
                    None
                }
                Ok(bytes) => Some(bytes),
                Err(why) => {
                    problem.get_or_insert(format!("Theme \"{name}\": icon file {file}: {why}"));
                    None
                }
            },
            (Some(_), None) => {
                problem.get_or_insert(format!("Theme \"{name}\": icon_file needs icon_family, the name inside the font"));
                None
            }
            _ => None,
        };
        let family = icon_font.as_ref().and(icon_family).map(leak);
        let font_is_missing = icon_file.is_some() && icon_font.is_none();
        icons::choose(family, if font_is_missing { Vec::new() } else { glyphs });
        Loaded { palette, problem, icon_font }
    }

    fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        if let Some(index) = key.strip_prefix("track_").and_then(|n| n.parse::<usize>().ok()) {
            if !(1..=self.tracks.len()).contains(&index) {
                return Err(format!("track colours go from track_1 to track_{}", self.tracks.len()));
            }
            self.tracks[index - 1] = colour(value)?;
            return Ok(());
        }
        let range = SIZE_KEYS.iter().chain(&SHAPE_KEYS).find(|(name, ..)| *name == key);
        if let Some((_, lowest, highest)) = range {
            let number: f32 = value.parse().map_err(|_| format!("\"{value}\" is not a number"))?;
            if !(*lowest..=*highest).contains(&number) {
                return Err(format!("{key} goes from {lowest} to {highest}"));
            }
            *self.number_slot(key).expect("every listed number has a slot") = number;
            return Ok(());
        }
        match key {
            "font" => {
                let family = leak(value);
                self.ui = Font::with_name(family);
                self.medium = Font { weight: font::Weight::Medium, ..Font::with_name(family) };
                self.semibold = Font { weight: font::Weight::Semibold, ..Font::with_name(family) };
            }
            "mono" => self.mono = Font::with_name(leak(value)),
            "headers" => self.headers = side(value)?,
            "all_audio" => self.all_audio = side(value)?,
            "playhead_cap" => {
                self.playhead_cap = match value {
                    "triangle" => PlayheadCap::Triangle,
                    "square" => PlayheadCap::Square,
                    "none" => PlayheadCap::None,
                    _ => return Err(format!("playhead_cap is triangle, square or none, not \"{value}\"")),
                }
            }
            "arm_shape" => {
                self.arm_shape = match value {
                    "dot" => ArmShape::Dot,
                    "square" => ArmShape::Square,
                    _ => return Err(format!("arm_shape is dot or square, not \"{value}\"")),
                }
            }
            "top_bar" => self.top_bar = bar(value)?,
            _ => {
                let slot = self.colour_slot(key).ok_or_else(|| format!("\"{key}\" is not a theme key"))?;
                *slot = colour(value)?;
            }
        }
        Ok(())
    }

    fn colour_slot(&mut self, key: &str) -> Option<&mut Color> {
        Some(match key {
            "background" => &mut self.background,
            "panel" => &mut self.panel,
            "raised" => &mut self.raised,
            "hover" => &mut self.hover,
            "line" => &mut self.line,
            "grid" => &mut self.grid,
            "text" => &mut self.text,
            "text_dim" => &mut self.text_dim,
            "text_faint" => &mut self.text_faint,
            "accent" => &mut self.accent,
            "on_accent" => &mut self.on_accent,
            "danger" => &mut self.danger,
            "meter_low" => &mut self.meter_low,
            "meter_mid" => &mut self.meter_mid,
            "meter_high" => &mut self.meter_high,
            _ => return None,
        })
    }

    fn number_slot(&mut self, key: &str) -> Option<&mut f32> {
        Some(match key {
            "track_height" => &mut self.track_height,
            "header_width" => &mut self.header_width,
            "ruler_height" => &mut self.ruler_height,
            "scrollbar_height" => &mut self.scrollbar_height,
            "top_bar_height" => &mut self.top_bar_height,
            "clip_title_height" => &mut self.clip_title_height,
            "clip_padding" => &mut self.clip_padding,
            "clip_title_size" => &mut self.clip_title_size,
            "track_title_size" => &mut self.track_title_size,
            "ruler_text_size" => &mut self.ruler_text_size,
            "corner" => &mut self.corner,
            "solid_corner" => &mut self.solid_corner,
            "sheet_corner" => &mut self.sheet_corner,
            "clip_corner" => &mut self.clip_corner,
            "border_width" => &mut self.border_width,
            "playhead_width" => &mut self.playhead_width,
            "handle_radius" => &mut self.handle_radius,
            "fade_flag_size" => &mut self.fade_flag_size,
            "meter_thickness" => &mut self.meter_thickness,
            _ => return None,
        })
    }

    fn to_text(&self) -> String {
        let mut copy = *self;
        let mut out = String::from(
            "# A Loupe theme: one key per line.\n\
             # Copy this file, change what you like, then put `theme = <file name>` in the settings file next to the themes folder.\n\
             # This file is written again each time Loupe starts, so edit your copy, not this one.\n\n\
             # Colours, as #rrggbb.\n",
        );
        for key in COLOUR_KEYS {
            let value = *copy.colour_slot(key).expect("every listed key has a slot");
            out.push_str(&format!("{key} = {}\n", hex(value)));
        }
        for (i, track) in self.tracks.iter().enumerate() {
            out.push_str(&format!("track_{} = {}\n", i + 1, hex(*track)));
        }
        out.push_str(&format!("\n# Fonts, by family name.\nfont = {}\nmono = {}\n", family(self.ui), family(self.mono)));
        for (title, keys) in [("Sizes", SIZE_KEYS.as_slice()), ("Shapes", SHAPE_KEYS.as_slice())] {
            out.push_str(&format!("\n# {title}, in pixels. Each line shows the allowed range.\n"));
            for (key, lowest, highest) in keys {
                let value = *copy.number_slot(key).expect("every listed number has a slot");
                out.push_str(&format!("{key} = {value}\n#   {lowest} to {highest}\n"));
            }
        }
        out.push_str("playhead_cap = triangle\n#   triangle, square or none\narm_shape = dot\n#   dot or square\n");
        out.push_str(
            "\n# Layout.\n\
             headers = left\n#   left or right: the side the track names sit on\n\
             all_audio = right\n#   left or right: the side the All audio panel opens on\n",
        );
        let arrangement: Vec<&str> = self.top_bar.iter().filter_map(|item| bar_name(*item)).collect();
        out.push_str(&format!(
            "top_bar = {}\n#   what follows File and Help in the top bar, in order. Leave a name out to hide it.\n\
             #   gap pushes what follows to the far side, space is a small gap.\n",
            arrangement.join(" ")
        ));
        out.push_str(
            "\n# Icons. Each icon is a character in the icon font, written as its hex code.\n\
             # To use your own font, put a TrueType (.ttf) file in this folder and name it here:\n\
             #   icon_file = my-icons.ttf\n\
             #   icon_family = My Icons\n\
             # Icons you do not list keep the built in drawing.\n",
        );
        for (name, glyph) in icons::BUILT_IN {
            out.push_str(&format!("# icon_{name} = {:x}\n", glyph as u32));
        }
        out
    }

    pub fn fonts(&self) -> (Font, Font) {
        (self.ui, self.mono)
    }

    pub fn with_fonts_of(mut self, other: &Palette) -> Self {
        (self.ui, self.medium, self.semibold, self.mono) = (other.ui, other.medium, other.semibold, other.mono);
        self
    }

    pub fn track(&self, index: usize) -> Color {
        self.tracks[index % self.tracks.len()]
    }

    pub fn top_bar_items(&self) -> impl Iterator<Item = BarItem> + '_ {
        self.top_bar.iter().copied().take_while(|item| *item != BarItem::End)
    }

    pub fn iced(&self) -> Theme {
        Theme::custom(
            "Loupe".into(),
            iced::theme::Palette {
                background: self.background,
                text: self.text,
                primary: self.accent,
                success: Color::from_rgb(0.204, 0.827, 0.6),
                danger: self.danger,
            },
        )
    }

    fn inner_corner(&self) -> f32 {
        (self.corner - 1.0).max(0.0)
    }

    pub fn backdrop(&self) -> container::Style {
        container::Style { background: Some(alpha(Color::BLACK, 0.55).into()), ..Default::default() }
    }

    pub fn sheet(&self) -> container::Style {
        container::Style {
            background: Some(self.panel.into()),
            border: Border { color: self.line, width: self.border_width, radius: self.sheet_corner.into() },
            ..Default::default()
        }
    }

    pub fn menu(&self) -> container::Style {
        container::Style {
            background: Some(self.panel.into()),
            border: Border { color: self.hover, width: self.border_width, radius: (self.corner + 2.0).into() },
            shadow: iced::Shadow {
                color: alpha(Color::BLACK, 0.4),
                offset: iced::Vector::new(0.0, 6.0),
                blur_radius: 18.0,
            },
            ..Default::default()
        }
    }

    pub fn menu_item(&self, status: button::Status) -> button::Style {
        let (background, text_color) = match status {
            button::Status::Active => (None, self.text),
            button::Status::Hovered | button::Status::Pressed => (Some(self.hover.into()), self.text),
            button::Status::Disabled => (None, self.text_faint),
        };
        button::Style { background, text_color, border: Border::default().rounded(self.inner_corner()), ..Default::default() }
    }

    pub fn swatch(&self, colour: Color, status: button::Status) -> button::Style {
        let border = match status {
            button::Status::Hovered | button::Status::Pressed => Border { color: self.text, width: 2.0, radius: self.inner_corner().into() },
            _ => Border { color: self.line, width: 1.0, radius: self.inner_corner().into() },
        };
        button::Style { background: Some(colour.into()), border, ..Default::default() }
    }

    pub fn destructive(&self, status: button::Status) -> button::Style {
        let background = match status {
            button::Status::Hovered | button::Status::Pressed => mix(self.danger, Color::BLACK, 0.12),
            _ => mix(self.danger, Color::BLACK, 0.25),
        };
        button::Style {
            background: Some(background.into()),
            text_color: Color::WHITE,
            border: Border::default().rounded(self.corner),
            ..Default::default()
        }
    }

    pub fn solo(&self, soloed: bool, status: button::Status) -> button::Style {
        let (background, text_color) = match (soloed, status) {
            (true, _) => (self.accent, self.on_accent),
            (false, button::Status::Hovered | button::Status::Pressed) => (self.hover, self.text),
            (false, _) => (self.raised, self.text_dim),
        };
        button::Style {
            background: Some(background.into()),
            text_color,
            border: Border::default().rounded(self.inner_corner()),
            ..Default::default()
        }
    }

    pub fn mute(&self, muted: bool, status: button::Status) -> button::Style {
        let (background, text_color) = match (muted, status) {
            (true, _) => (self.danger, self.on_accent),
            (false, button::Status::Hovered | button::Status::Pressed) => (self.hover, self.text),
            (false, _) => (self.raised, self.text_dim),
        };
        button::Style {
            background: Some(background.into()),
            text_color,
            border: Border::default().rounded(self.inner_corner()),
            ..Default::default()
        }
    }

    pub fn strip(&self) -> container::Style {
        container::Style {
            background: Some(self.background.into()),
            border: Border { color: self.line, width: self.border_width, radius: self.corner.into() },
            ..Default::default()
        }
    }

    pub fn toggled(&self, on: bool, status: button::Status) -> button::Style {
        if on {
            button::Style {
                background: Some(self.hover.into()),
                text_color: self.text,
                border: Border::default().rounded(self.corner),
                ..Default::default()
            }
        } else {
            self.ghost(status)
        }
    }

    pub fn record(&self, on: bool, status: button::Status) -> button::Style {
        if on {
            button::Style { background: Some(self.danger.into()), border: Border::default().rounded(self.corner), ..Default::default() }
        } else {
            self.ghost(status)
        }
    }

    pub fn record_mark(&self, on: bool) -> container::Style {
        let (colour, corner) = if on { (self.on_accent, 2.0) } else { (self.danger, 6.0) };
        container::Style { background: Some(colour.into()), border: Border::default().rounded(corner), ..Default::default() }
    }

    pub fn title_bar(&self) -> container::Style {
        container::Style {
            background: Some(self.raised.into()),
            border: Border { radius: iced::border::top((self.sheet_corner - 1.0).max(0.0)), ..Border::default() },
            ..Default::default()
        }
    }

    pub fn bar(&self) -> container::Style {
        container::Style { background: Some(self.panel.into()), ..Default::default() }
    }

    pub fn rule(&self) -> container::Style {
        container::Style { background: Some(self.line.into()), ..Default::default() }
    }

    pub fn ghost(&self, status: button::Status) -> button::Style {
        let (background, text_color) = match status {
            button::Status::Active => (None, self.text),
            button::Status::Hovered => (Some(self.raised.into()), self.text),
            button::Status::Pressed => (Some(self.hover.into()), self.text),
            button::Status::Disabled => (None, self.text_faint),
        };
        button::Style { background, text_color, border: Border::default().rounded(self.corner), ..Default::default() }
    }

    pub fn outlined(&self, status: button::Status) -> button::Style {
        let (background, text_color) = match status {
            button::Status::Active => (self.raised, self.text),
            button::Status::Hovered | button::Status::Pressed => (self.hover, self.text),
            button::Status::Disabled => (self.panel, self.text_faint),
        };
        button::Style {
            background: Some(background.into()),
            text_color,
            border: Border { color: self.line, width: self.border_width, radius: self.corner.into() },
            ..Default::default()
        }
    }

    pub fn solid(&self, status: button::Status) -> button::Style {
        let background = match status {
            button::Status::Hovered | button::Status::Pressed => mix(self.accent, self.background, 0.12),
            _ => self.accent,
        };
        button::Style {
            background: Some(background.into()),
            text_color: self.on_accent,
            border: Border::default().rounded(self.solid_corner),
            ..Default::default()
        }
    }

    pub fn field(&self, status: text_input::Status) -> text_input::Style {
        let border = match status {
            text_input::Status::Focused => self.accent,
            text_input::Status::Hovered => self.hover,
            _ => self.line,
        };
        text_input::Style {
            background: Background::Color(self.background),
            border: Border { color: border, width: self.border_width.max(1.0), radius: self.corner.into() },
            icon: self.text_dim,
            placeholder: self.text_faint,
            value: self.text,
            selection: alpha(self.accent, 0.35),
        }
    }

    pub fn slider(&self, status: slider::Status) -> slider::Style {
        let handle = match status {
            slider::Status::Active => self.text,
            _ => Color::WHITE,
        };
        slider::Style {
            rail: slider::Rail {
                backgrounds: (self.accent.into(), self.raised.into()),
                width: 3.0,
                border: Border::default().rounded(2),
            },
            handle: slider::Handle {
                shape: slider::HandleShape::Circle { radius: 6.0 },
                background: handle.into(),
                border_width: 0.0,
                border_color: Color::TRANSPARENT,
            },
        }
    }
}

pub fn alpha(color: Color, a: f32) -> Color {
    Color { a, ..color }
}

pub fn mix(base: Color, tint: Color, amount: f32) -> Color {
    Color::from_rgb(
        base.r + (tint.r - base.r) * amount,
        base.g + (tint.g - base.g) * amount,
        base.b + (tint.b - base.b) * amount,
    )
}

fn colour(value: &str) -> Result<Color, String> {
    let digits = value.strip_prefix('#').unwrap_or(value);
    let bad = || format!("\"{value}\" is not a colour; use #rrggbb");
    if digits.len() != 6 {
        return Err(bad());
    }
    let channel = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).map_err(|_| bad());
    Ok(Color::from_rgb8(channel(0)?, channel(2)?, channel(4)?))
}

fn hex(color: Color) -> String {
    let [r, g, b, _] = color.into_rgba8();
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn family(font: Font) -> &'static str {
    match font.family {
        font::Family::Name(name) => name,
        _ => "Inter",
    }
}

fn leak(value: &str) -> &'static str {
    Box::leak(value.to_string().into_boxed_str())
}

pub fn folder() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("themes"))
}

pub fn available() -> Vec<String> {
    let Some(themes) = folder() else {
        return Vec::new();
    };
    let mut names: Vec<String> = fs::read_dir(themes)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension == THEME_EXTENSION))
        .filter(|path| path.file_name().is_some_and(|name| name != REFERENCE_FILE))
        .filter_map(|path| path.file_stem().map(|name| name.to_string_lossy().into_owned()))
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names
}

fn side(value: &str) -> Result<Side, String> {
    match value {
        "left" => Ok(Side::Left),
        "right" => Ok(Side::Right),
        _ => Err(format!("\"{value}\" is not a side; use left or right")),
    }
}

fn bar_name(item: BarItem) -> Option<&'static str> {
    BAR_NAMES.iter().find(|(_, known)| *known == item).map(|(name, _)| *name)
}

fn bar(value: &str) -> Result<[BarItem; BAR_SLOTS], String> {
    let mut items = [BarItem::End; BAR_SLOTS];
    let mut filled = 0;
    for word in value.split_whitespace() {
        let known: Vec<&str> = BAR_NAMES.iter().map(|(name, _)| *name).collect();
        let item = BAR_NAMES
            .iter()
            .find(|(name, _)| *name == word)
            .map(|(_, item)| *item)
            .ok_or_else(|| format!("\"{word}\" is not a top bar item; the items are {}", known.join(", ")))?;
        let repeats = !matches!(item, BarItem::Gap | BarItem::Space);
        if repeats && items[..filled].contains(&item) {
            return Err(format!("{word} is in top_bar twice"));
        }
        if filled == BAR_SLOTS {
            return Err(format!("top_bar holds at most {BAR_SLOTS} items"));
        }
        items[filled] = item;
        filled += 1;
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_written_reference_theme_reads_back_as_the_default() {
        let mut read = NEUTRAL;
        read.top_bar = [BarItem::End; BAR_SLOTS];
        read.header_width = 300.0;
        for (_, key, value) in entries(&NEUTRAL.to_text()) {
            read.set(key, value).unwrap_or_else(|why| panic!("{key}: {why}"));
        }
        assert_eq!(read.top_bar, NEUTRAL.top_bar);
        assert_eq!(read.header_width, NEUTRAL.header_width);
        assert_eq!(read.headers, Side::Left);
    }

    #[test]
    fn every_bundled_theme_reads_without_a_problem() {
        for (name, text) in BUNDLED {
            let mut theme = NEUTRAL;
            for (line, key, value) in entries(text) {
                theme.set(key, value).unwrap_or_else(|why| panic!("{name} line {line}: {why}"));
            }
        }
    }

    #[test]
    fn sizes_outside_their_range_are_refused_with_the_range() {
        let mut theme = NEUTRAL;
        assert_eq!(theme.set("header_width", "90"), Err("header_width goes from 140 to 480".into()));
        assert_eq!(theme.set("corner", "soft"), Err("\"soft\" is not a number".into()));
        assert!(theme.set("corner", "0").is_ok());
        assert_eq!(theme.corner, 0.0);
    }

    #[test]
    fn the_top_bar_takes_an_order_and_leaves_out_what_is_not_named() {
        let mut theme = NEUTRAL;
        theme.set("top_bar", "import gap play record").unwrap();
        let shown: Vec<BarItem> = theme.top_bar_items().collect();
        assert_eq!(shown, [BarItem::Import, BarItem::Gap, BarItem::Play, BarItem::Record]);
        assert!(theme.set("top_bar", "play play").unwrap_err().contains("twice"));
        assert!(theme.set("top_bar", "play volume").unwrap_err().contains("not a top bar item"));
        assert!(theme.set("top_bar", "space gap space gap").is_ok());
    }

    #[test]
    fn layout_and_shape_choices_name_their_options_when_wrong() {
        let mut theme = NEUTRAL;
        theme.set("headers", "right").unwrap();
        theme.set("playhead_cap", "none").unwrap();
        theme.set("arm_shape", "square").unwrap();
        assert_eq!((theme.headers, theme.playhead_cap, theme.arm_shape), (Side::Right, PlayheadCap::None, ArmShape::Square));
        assert!(theme.set("headers", "top").unwrap_err().contains("left or right"));
        assert!(theme.set("wobble", "1").unwrap_err().contains("not a theme key"));
    }
}
