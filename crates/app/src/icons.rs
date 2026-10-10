use std::sync::{PoisonError, RwLock};

use iced::Font;

const BUILT_IN_FAMILY: &str = "lucide";

pub const BUILT_IN: [(&str, char); 21] = [
    ("play", '\u{e13c}'),
    ("pause", '\u{e12e}'),
    ("skip-back", '\u{e15f}'),
    ("folder-open", '\u{e247}'),
    ("undo-2", '\u{e2a1}'),
    ("redo-2", '\u{e2a0}'),
    ("x", '\u{e1b2}'),
    ("settings", '\u{e154}'),
    ("sliders-vertical", '\u{e162}'),
    ("chevron-down", '\u{e06d}'),
    ("chevron-right", '\u{e06e}'),
    ("plus", '\u{e13d}'),
    ("pencil", '\u{e1f9}'),
    ("slice", '\u{e2f0}'),
    ("volume-x", '\u{e1ac}'),
    ("eraser", '\u{e28f}'),
    ("square", '\u{e167}'),
    ("metronome", '\u{e6bc}'),
    ("keyboard-music", '\u{e560}'),
    ("layers-2", '\u{e52a}'),
    ("eye-off", '\u{e0bb}'),
];

struct Chosen {
    family: &'static str,
    glyphs: Vec<(String, char)>,
}

static CHOSEN: RwLock<Option<Chosen>> = RwLock::new(None);

pub fn choose(family: Option<&'static str>, glyphs: Vec<(String, char)>) {
    let chosen = Chosen { family: family.unwrap_or(BUILT_IN_FAMILY), glyphs };
    *CHOSEN.write().unwrap_or_else(PoisonError::into_inner) = Some(chosen);
}

fn themed(name: &str) -> Option<(char, &'static str)> {
    let chosen = CHOSEN.read().unwrap_or_else(PoisonError::into_inner);
    let chosen = chosen.as_ref()?;
    chosen.glyphs.iter().find(|(known, _)| known == name).map(|(_, glyph)| (*glyph, chosen.family))
}

fn built_in(name: &str) -> Option<char> {
    BUILT_IN.iter().find(|(known, _)| *known == name).map(|(_, glyph)| *glyph)
}

pub fn glyph(name: &str) -> char {
    themed(name).map(|(glyph, _)| glyph).or_else(|| built_in(name)).unwrap_or('?')
}

pub fn font(name: &str) -> Font {
    Font::with_name(themed(name).map_or(BUILT_IN_FAMILY, |(_, family)| family))
}

pub fn code(name: &str, value: &str) -> Result<char, String> {
    if built_in(name).is_none() {
        let known: Vec<&str> = BUILT_IN.iter().map(|(name, _)| *name).collect();
        return Err(format!("\"{name}\" is not an icon; the icons are {}", known.join(", ")));
    }
    let digits = value.strip_prefix("U+").or_else(|| value.strip_prefix("u+")).unwrap_or(value);
    let from_hex = if digits.len() >= 2 { u32::from_str_radix(digits, 16).ok().and_then(char::from_u32) } else { None };
    let mut typed = value.chars();
    match (from_hex, typed.next(), typed.next()) {
        (Some(glyph), ..) => Ok(glyph),
        (None, Some(only), None) => Ok(only),
        _ => Err(format!("\"{value}\" is not a character; use its hex code, such as e13c")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_icon_code_is_hex_or_one_typed_character() {
        assert_eq!(code("play", "e13c"), Ok('\u{e13c}'));
        assert_eq!(code("play", "U+E13C"), Ok('\u{e13c}'));
        assert_eq!(code("play", "▶"), Ok('▶'));
        assert!(code("play", "triangle").unwrap_err().contains("hex code"));
        assert!(code("rocket", "e13c").unwrap_err().contains("not an icon"));
    }

    #[test]
    fn icons_fall_back_to_the_built_in_drawing() {
        assert_eq!(glyph("pause"), '\u{e12e}');
        assert_eq!(font("pause"), Font::with_name(BUILT_IN_FAMILY));
        assert_eq!(glyph("nothing"), '?');
    }
}
