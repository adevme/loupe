use iced::font::Weight;
use iced::{Color, Font};

#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub background: Color,
    pub well_low: Color,
    pub deck: Color,
    pub panel: Color,
    pub raised: Color,
    pub hairline: Color,
    pub shadow: Color,
    pub grid: Color,
    pub grid_strong: Color,
    pub text: Color,
    pub text_dim: Color,
    pub text_faint: Color,
    pub accent: Color,
    pub on_accent: Color,
    pub danger: Color,
    pub curve: Color,
    pub spectrum: Color,
    pub spectrum_before: Color,
    pub bands: [Color; 8],
}

impl Default for Look {
    fn default() -> Self {
        Self {
            background: Color::from_rgb(0.062, 0.066, 0.077),
            well_low: Color::from_rgb(0.038, 0.04, 0.048),
            deck: Color::from_rgb(0.118, 0.122, 0.137),
            panel: Color::from_rgb(0.149, 0.153, 0.171),
            raised: Color::from_rgb(0.196, 0.2, 0.222),
            hairline: Color::from_rgba(1.0, 1.0, 1.0, 0.075),
            shadow: Color::from_rgba(0.0, 0.0, 0.0, 0.5),
            grid: Color::from_rgba(1.0, 1.0, 1.0, 0.022),
            grid_strong: Color::from_rgba(1.0, 1.0, 1.0, 0.045),
            text: Color::from_rgb(0.925, 0.928, 0.94),
            text_dim: Color::from_rgb(0.6, 0.615, 0.65),
            text_faint: Color::from_rgb(0.42, 0.435, 0.47),
            accent: Color::from_rgb(1.0, 0.84, 0.45),
            on_accent: Color::from_rgb(0.11, 0.11, 0.118),
            danger: Color::from_rgb(0.98, 0.39, 0.42),
            curve: Color::from_rgb(1.0, 0.84, 0.45),
            spectrum: Color::from_rgba(0.5, 0.66, 0.98, 0.2),
            spectrum_before: Color::from_rgba(1.0, 1.0, 1.0, 0.045),
            bands: [
                Color::from_rgb(0.98, 0.47, 0.47),
                Color::from_rgb(0.98, 0.66, 0.34),
                Color::from_rgb(0.95, 0.85, 0.4),
                Color::from_rgb(0.47, 0.86, 0.55),
                Color::from_rgb(0.38, 0.8, 0.86),
                Color::from_rgb(0.47, 0.62, 0.98),
                Color::from_rgb(0.71, 0.53, 0.98),
                Color::from_rgb(0.95, 0.52, 0.8),
            ],
        }
    }
}

impl Look {
    pub fn band(&self, index: usize) -> Color {
        self.bands[index % self.bands.len()]
    }
}

pub fn fade(colour: Color, alpha: f32) -> Color {
    Color { a: alpha, ..colour }
}

pub fn lift(colour: Color, by: f32) -> Color {
    Color { r: (colour.r + by).clamp(0.0, 1.0), g: (colour.g + by).clamp(0.0, 1.0), b: (colour.b + by).clamp(0.0, 1.0), a: colour.a }
}

pub fn mix(a: Color, b: Color, share: f32) -> Color {
    let s = share.clamp(0.0, 1.0);
    Color { r: a.r + (b.r - a.r) * s, g: a.g + (b.g - a.g) * s, b: a.b + (b.b - a.b) * s, a: a.a + (b.a - a.a) * s }
}

pub const MEDIUM: Font = Font { weight: Weight::Medium, ..Font::with_name("Inter") };
pub const SEMIBOLD: Font = Font { weight: Weight::Semibold, ..Font::with_name("Inter") };

impl Look {
    pub fn tinted(self, accent: Color) -> Self {
        Self { accent, curve: accent, ..self }
    }
}

pub mod accents {
    use iced::Color;

    pub const EQ: Color = Color::from_rgb(1.0, 0.8, 0.42);
    pub const COMPRESSOR: Color = Color::from_rgb(0.42, 0.72, 1.0);
    pub const LIMITER: Color = Color::from_rgb(1.0, 0.7, 0.34);
    pub const DELAY: Color = Color::from_rgb(0.38, 0.88, 0.72);
    pub const REVERB: Color = Color::from_rgb(0.68, 0.56, 1.0);
    pub const DEESSER: Color = Color::from_rgb(0.4, 0.86, 0.88);
    pub const SATURATION: Color = Color::from_rgb(1.0, 0.46, 0.36);
    pub const CHORUS: Color = Color::from_rgb(0.56, 0.62, 1.0);
    pub const TRANSIENT: Color = Color::from_rgb(0.74, 0.9, 0.38);
    pub const GATE: Color = Color::from_rgb(0.4, 0.88, 0.58);
    pub const METER: Color = Color::from_rgb(1.0, 0.8, 0.42);
    pub const MULTIBAND: Color = Color::from_rgb(0.5, 0.78, 1.0);
    pub const TUNE: Color = Color::from_rgb(1.0, 0.52, 0.74);
    pub const KEY: Color = Color::from_rgb(1.0, 0.8, 0.42);
    pub const RIDER: Color = Color::from_rgb(0.38, 0.84, 0.92);
    pub const DOUBLER: Color = Color::from_rgb(0.78, 0.6, 1.0);
    pub const DENOISE: Color = Color::from_rgb(0.45, 0.9, 0.78);
}

impl Look {
    pub fn neutral(&self) -> Color {
        Color::from_rgb(0.74, 0.77, 0.84)
    }
}
