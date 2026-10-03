use iced::Color;

#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub background: Color,
    pub panel: Color,
    pub grid: Color,
    pub grid_strong: Color,
    pub text: Color,
    pub text_dim: Color,
    pub curve: Color,
    pub spectrum: Color,
    pub spectrum_before: Color,
    pub bands: [Color; 8],
}

impl Default for Look {
    fn default() -> Self {
        Self {
            background: Color::from_rgb(0.105, 0.107, 0.118),
            panel: Color::from_rgb(0.157, 0.157, 0.165),
            grid: Color::from_rgba(1.0, 1.0, 1.0, 0.05),
            grid_strong: Color::from_rgba(1.0, 1.0, 1.0, 0.11),
            text: Color::from_rgb(0.925, 0.925, 0.933),
            text_dim: Color::from_rgb(0.55, 0.56, 0.6),
            curve: Color::from_rgb(1.0, 0.86, 0.45),
            spectrum: Color::from_rgba(0.55, 0.68, 0.95, 0.16),
            spectrum_before: Color::from_rgba(1.0, 1.0, 1.0, 0.05),
            bands: [
                Color::from_rgb(0.98, 0.45, 0.45),
                Color::from_rgb(0.98, 0.65, 0.3),
                Color::from_rgb(0.95, 0.85, 0.35),
                Color::from_rgb(0.45, 0.85, 0.5),
                Color::from_rgb(0.35, 0.8, 0.85),
                Color::from_rgb(0.45, 0.6, 0.98),
                Color::from_rgb(0.7, 0.5, 0.98),
                Color::from_rgb(0.95, 0.5, 0.8),
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
