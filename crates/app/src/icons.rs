pub fn glyph(name: &str) -> char {
    match name {
        "play" => '\u{e13c}',
        "pause" => '\u{e12e}',
        "skip-back" => '\u{e15f}',
        "folder-open" => '\u{e247}',
        "undo-2" => '\u{e2a1}',
        "redo-2" => '\u{e2a0}',
        "x" => '\u{e1b2}',
        "settings" => '\u{e154}',
        "sliders-vertical" => '\u{e162}',
        "chevron-down" => '\u{e06d}',
        "plus" => '\u{e13d}',
        "pencil" => '\u{e1f9}',
        "slice" => '\u{e2f0}',
        "volume-x" => '\u{e1ac}',
        "eraser" => '\u{e28f}',
        _ => '?',
    }
}
