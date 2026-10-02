use iced::widget::{button, container, slider, text_input};
use iced::{font, Background, Border, Color, Font, Theme};

pub const BG: Color = Color::from_rgb(0.043, 0.043, 0.051);
pub const PANEL: Color = Color::from_rgb(0.067, 0.067, 0.082);
pub const RAISED: Color = Color::from_rgb(0.11, 0.11, 0.135);
pub const HOVER: Color = Color::from_rgb(0.15, 0.15, 0.18);
pub const LINE: Color = Color::from_rgb(0.125, 0.125, 0.155);
pub const TEXT: Color = Color::from_rgb(0.906, 0.906, 0.925);
pub const TEXT_DIM: Color = Color::from_rgb(0.6, 0.6, 0.66);
pub const TEXT_FAINT: Color = Color::from_rgb(0.4, 0.4, 0.46);
pub const ACCENT: Color = Color::from_rgb(0.925, 0.725, 0.38);
pub const ON_ACCENT: Color = Color::from_rgb(0.08, 0.06, 0.02);
pub const ROSE: Color = Color::from_rgb(0.984, 0.443, 0.522);

pub const TRACKS: [Color; 6] = [
    Color::from_rgb(0.45, 0.72, 0.95),
    Color::from_rgb(0.73, 0.58, 0.96),
    Color::from_rgb(0.42, 0.82, 0.68),
    Color::from_rgb(0.96, 0.6, 0.55),
    Color::from_rgb(0.93, 0.78, 0.45),
    Color::from_rgb(0.55, 0.62, 0.97),
];

pub const INTER: Font = Font::with_name("Inter");
pub const MEDIUM: Font = Font { weight: font::Weight::Medium, ..Font::with_name("Inter") };
pub const SEMIBOLD: Font = Font { weight: font::Weight::Semibold, ..Font::with_name("Inter") };
pub const MONO: Font = Font::with_name("JetBrains Mono");
pub const ICONS: Font = Font::with_name("lucide");

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

pub fn theme() -> Theme {
    Theme::custom(
        "Loupe".into(),
        iced::theme::Palette {
            background: BG,
            text: TEXT,
            primary: ACCENT,
            success: Color::from_rgb(0.204, 0.827, 0.6),
            danger: ROSE,
        },
    )
}

pub fn bar(_: &Theme) -> container::Style {
    container::Style { background: Some(PANEL.into()), ..Default::default() }
}

pub fn line(_: &Theme) -> container::Style {
    container::Style { background: Some(LINE.into()), ..Default::default() }
}

pub fn ghost(_: &Theme, status: button::Status) -> button::Style {
    let (background, text_color) = match status {
        button::Status::Active => (None, TEXT),
        button::Status::Hovered => (Some(RAISED.into()), TEXT),
        button::Status::Pressed => (Some(HOVER.into()), TEXT),
        button::Status::Disabled => (None, TEXT_FAINT),
    };
    button::Style { background, text_color, border: Border::default().rounded(6), ..Default::default() }
}

pub fn outlined(_: &Theme, status: button::Status) -> button::Style {
    let (background, text_color) = match status {
        button::Status::Active => (RAISED, TEXT),
        button::Status::Hovered | button::Status::Pressed => (HOVER, TEXT),
        button::Status::Disabled => (PANEL, TEXT_FAINT),
    };
    button::Style {
        background: Some(background.into()),
        text_color,
        border: Border { color: LINE, width: 1.0, radius: 6.0.into() },
        ..Default::default()
    }
}

pub fn solid(_: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => Color::from_rgb(0.97, 0.79, 0.47),
        _ => ACCENT,
    };
    button::Style {
        background: Some(background.into()),
        text_color: ON_ACCENT,
        border: Border::default().rounded(16),
        ..Default::default()
    }
}

pub fn field(_: &Theme, status: text_input::Status) -> text_input::Style {
    let border = match status {
        text_input::Status::Focused => ACCENT,
        text_input::Status::Hovered => HOVER,
        _ => LINE,
    };
    text_input::Style {
        background: Background::Color(BG),
        border: Border { color: border, width: 1.0, radius: 6.0.into() },
        icon: TEXT_DIM,
        placeholder: TEXT_FAINT,
        value: TEXT,
        selection: alpha(ACCENT, 0.35),
    }
}

pub fn gain(_: &Theme, status: slider::Status) -> slider::Style {
    let handle = match status {
        slider::Status::Active => TEXT,
        _ => Color::WHITE,
    };
    slider::Style {
        rail: slider::Rail {
            backgrounds: (ACCENT.into(), RAISED.into()),
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
