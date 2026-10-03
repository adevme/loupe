use std::fs;

use iced::widget::{button, container, slider, text_input};
use iced::{font, Background, Border, Color, Font, Theme};

use crate::settings::{config_dir, entries};

pub const ICONS: Font = Font::with_name("lucide");
pub const MIN_TRACK_HEIGHT: f32 = 40.0;
pub const MAX_TRACK_HEIGHT: f32 = 400.0;

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

pub struct Loaded {
    pub palette: Palette,
    pub problem: Option<String>,
}

impl Palette {
    pub fn load(chosen: Option<&str>) -> Loaded {
        let Some(dir) = config_dir() else {
            return Loaded { palette: NEUTRAL, problem: None };
        };
        let themes = dir.join("themes");
        let _ = fs::create_dir_all(&themes);
        let _ = fs::write(themes.join("default.theme"), NEUTRAL.to_text());
        let Some(name) = chosen else {
            return Loaded { palette: NEUTRAL, problem: None };
        };
        let path = themes.join(format!("{name}.theme"));
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(_) => {
                let problem = format!("Theme \"{name}\" is not at {}", path.display());
                return Loaded { palette: NEUTRAL, problem: Some(problem) };
            }
        };
        let mut palette = NEUTRAL;
        let mut problem = None;
        for (line, key, value) in entries(&text) {
            if let Err(why) = palette.set(key, value) {
                problem = Some(format!("Theme \"{name}\" line {line}: {why}"));
                break;
            }
        }
        Loaded { palette, problem }
    }

    fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        if let Some(index) = key.strip_prefix("track_").and_then(|n| n.parse::<usize>().ok()) {
            if !(1..=self.tracks.len()).contains(&index) {
                return Err(format!("track colours go from track_1 to track_{}", self.tracks.len()));
            }
            self.tracks[index - 1] = colour(value)?;
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
            "track_height" => {
                let height: f32 = value.parse().map_err(|_| format!("\"{value}\" is not a number"))?;
                if !(MIN_TRACK_HEIGHT..=MAX_TRACK_HEIGHT).contains(&height) {
                    return Err(format!("track_height goes from {MIN_TRACK_HEIGHT} to {MAX_TRACK_HEIGHT}"));
                }
                self.track_height = height;
            }
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

    fn to_text(&self) -> String {
        let mut copy = *self;
        let mut out = String::from(
            "# A Loupe theme: one key per line, colours as #rrggbb.\n\
             # Copy this file, change what you like, then put `theme = <file name>` in the settings file next to the themes folder.\n\n",
        );
        for key in COLOUR_KEYS {
            let value = *copy.colour_slot(key).expect("every listed key has a slot");
            out.push_str(&format!("{key} = {}\n", hex(value)));
        }
        for (i, track) in self.tracks.iter().enumerate() {
            out.push_str(&format!("track_{} = {}\n", i + 1, hex(*track)));
        }
        out.push_str(&format!("font = {}\nmono = {}\ntrack_height = {}\n", family(self.ui), family(self.mono), self.track_height));
        out
    }

    pub fn track(&self, index: usize) -> Color {
        self.tracks[index % self.tracks.len()]
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

    pub fn backdrop(&self) -> container::Style {
        container::Style { background: Some(alpha(Color::BLACK, 0.55).into()), ..Default::default() }
    }

    pub fn sheet(&self) -> container::Style {
        container::Style {
            background: Some(self.panel.into()),
            border: Border { color: self.line, width: 1.0, radius: 10.0.into() },
            ..Default::default()
        }
    }

    pub fn menu(&self) -> container::Style {
        container::Style {
            background: Some(self.panel.into()),
            border: Border { color: self.hover, width: 1.0, radius: 8.0.into() },
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
        button::Style { background, text_color, border: Border::default().rounded(5), ..Default::default() }
    }

    pub fn swatch(&self, colour: Color, status: button::Status) -> button::Style {
        let border = match status {
            button::Status::Hovered | button::Status::Pressed => Border { color: self.text, width: 2.0, radius: 5.0.into() },
            _ => Border { color: self.line, width: 1.0, radius: 5.0.into() },
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
            border: Border::default().rounded(6),
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
            border: Border::default().rounded(5),
            ..Default::default()
        }
    }

    pub fn strip(&self) -> container::Style {
        container::Style {
            background: Some(self.background.into()),
            border: Border { color: self.line, width: 1.0, radius: 6.0.into() },
            ..Default::default()
        }
    }

    pub fn toggled(&self, on: bool, status: button::Status) -> button::Style {
        if on {
            button::Style {
                background: Some(self.hover.into()),
                text_color: self.text,
                border: Border::default().rounded(6),
                ..Default::default()
            }
        } else {
            self.ghost(status)
        }
    }

    pub fn title_bar(&self) -> container::Style {
        container::Style {
            background: Some(self.raised.into()),
            border: Border { radius: iced::border::top(9), ..Border::default() },
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
        button::Style { background, text_color, border: Border::default().rounded(6), ..Default::default() }
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
            border: Border { color: self.line, width: 1.0, radius: 6.0.into() },
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
            border: Border::default().rounded(16),
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
            border: Border { color: border, width: 1.0, radius: 6.0.into() },
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
