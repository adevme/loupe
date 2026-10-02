pub fn glyph(name: &str) -> char {
    match name {
        "play" => '\u{e13c}',
        "pause" => '\u{e12e}',
        "skip-back" => '\u{e15f}',
        "folder-open" => '\u{e247}',
        "trash-2" => '\u{e18e}',
        "undo-2" => '\u{e2a1}',
        "redo-2" => '\u{e2a0}',
        "scissors" => '\u{e14e}',
        "x" => '\u{e1b2}',
        _ => '?',
    }
}
