use std::sync::RwLock;

pub const CHROME_VARIABLE: &str = "LOUPE_CHROME";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chrome {
    pub background: u32,
    pub panel: u32,
    pub line: u32,
    pub text: u32,
    pub text_dim: u32,
    pub accent: u32,
}

impl Default for Chrome {
    fn default() -> Self {
        Self { background: 0x1A1A1A, panel: 0x242424, line: 0x3A3A3A, text: 0xE6E6E6, text_dim: 0x9A9A9A, accent: 0x6E8BFF }
    }
}

static WORN: RwLock<Option<Chrome>> = RwLock::new(None);

pub fn wear(chrome: Chrome) {
    if let Ok(mut held) = WORN.write() {
        *held = Some(chrome);
    }
}

pub fn worn() -> Option<Chrome> {
    WORN.read().ok().and_then(|held| *held)
}

impl Chrome {
    pub fn to_text(self) -> String {
        format!(
            "{:06x},{:06x},{:06x},{:06x},{:06x},{:06x}",
            self.background, self.panel, self.line, self.text, self.text_dim, self.accent
        )
    }

    pub fn from_text(text: &str) -> Option<Self> {
        let mut parts = text.split(',').map(|part| u32::from_str_radix(part.trim(), 16).ok());
        let mut next = || parts.next().flatten().filter(|value| *value <= 0xFF_FFFF);
        Some(Self {
            background: next()?,
            panel: next()?,
            line: next()?,
            text: next()?,
            text_dim: next()?,
            accent: next()?,
        })
    }

    pub fn from_the_app() -> Self {
        std::env::var(CHROME_VARIABLE).ok().and_then(|text| Self::from_text(&text)).unwrap_or_default()
    }

    pub fn windows_order(colour: u32) -> u32 {
        let (red, green, blue) = (colour >> 16 & 0xFF, colour >> 8 & 0xFF, colour & 0xFF);
        blue << 16 | green << 8 | red
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_survive_the_trip_to_the_host() {
        let chrome = Chrome { background: 0x101112, panel: 0x202122, line: 0x303132, text: 0xF0F1F2, text_dim: 0x808182, accent: 0x405060 };
        assert_eq!(Chrome::from_text(&chrome.to_text()), Some(chrome));
    }

    #[test]
    fn anything_that_is_not_six_colours_is_turned_away() {
        assert_eq!(Chrome::from_text(""), None);
        assert_eq!(Chrome::from_text("101112,202122"), None);
        assert_eq!(Chrome::from_text("101112,202122,303132,f0f1f2,808182,zzzzzz"), None);
        assert_eq!(Chrome::from_text("1101112,202122,303132,f0f1f2,808182,405060"), None);
    }

    #[test]
    fn windows_wants_the_blue_first() {
        assert_eq!(Chrome::windows_order(0x11_22_33), 0x33_22_11);
    }
}
