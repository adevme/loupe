use std::io::{BufRead, BufReader, Read, Write};

#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    Classes(String),
    Load { path: String, index: usize, rate: u32, block: usize },
    Process,
    ProcessWithSide,
    ProcessAt(i64),
    Region(Region),
    Show,
    Hide,
    Save,
    Knobs,
    Turn { knob: usize, value: f32 },
    Restore(Vec<u8>),
    Quit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    Classes(Vec<String>),
    Loaded { inputs: usize, outputs: usize, latency: usize, ara: bool },
    State(Vec<u8>),
    Knobs(Vec<String>),
    Fine,
    Trouble(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub file: String,
    pub name: String,
    pub start: f64,
    pub offset: f64,
    pub length: f64,
    pub stretch: f64,
    pub tempo: f64,
}

impl Region {
    fn line(&self) -> String {
        let tidy = |text: &str| text.replace(['\t', '\n'], " ");
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            tidy(&self.file),
            tidy(&self.name),
            self.start,
            self.offset,
            self.length,
            self.stretch,
            self.tempo
        )
    }

    fn from_parts<'a>(parts: &mut impl Iterator<Item = &'a str>) -> Option<Self> {
        Some(Self {
            file: parts.next()?.to_string(),
            name: parts.next()?.to_string(),
            start: parts.next()?.parse().ok()?,
            offset: parts.next()?.parse().ok()?,
            length: parts.next()?.parse().ok()?,
            stretch: parts.next()?.parse().ok()?,
            tempo: parts.next()?.parse().ok()?,
        })
    }
}

impl Ask {
    pub fn write(&self, out: &mut impl Write) -> std::io::Result<()> {
        let line = match self {
            Ask::Classes(path) => format!("classes\t{path}"),
            Ask::Load { path, index, rate, block } => format!("load\t{path}\t{index}\t{rate}\t{block}"),
            Ask::Process => "process".to_string(),
            Ask::ProcessWithSide => "process2".to_string(),
            Ask::ProcessAt(at) => format!("processat\t{at}"),
            Ask::Region(region) => format!("region\t{}", region.line()),
            Ask::Show => "show".to_string(),
            Ask::Hide => "hide".to_string(),
            Ask::Save => "save".to_string(),
            Ask::Knobs => "knobs".to_string(),
            Ask::Turn { knob, value } => format!("turn\t{knob}\t{value}"),
            Ask::Restore(state) => format!("restore\t{}", hex_of(state)),
            Ask::Quit => "quit".to_string(),
        };
        writeln!(out, "{line}")?;
        out.flush()
    }

    pub fn read(line: &str) -> Option<Self> {
        let mut parts = line.trim_end().split('\t');
        match parts.next()? {
            "classes" => Some(Ask::Classes(parts.next()?.to_string())),
            "load" => Some(Ask::Load {
                path: parts.next()?.to_string(),
                index: parts.next()?.parse().ok()?,
                rate: parts.next()?.parse().ok()?,
                block: parts.next()?.parse().ok()?,
            }),
            "process" => Some(Ask::Process),
            "process2" => Some(Ask::ProcessWithSide),
            "processat" => Some(Ask::ProcessAt(parts.next()?.parse().ok()?)),
            "region" => Some(Ask::Region(Region::from_parts(&mut parts)?)),
            "show" => Some(Ask::Show),
            "hide" => Some(Ask::Hide),
            "save" => Some(Ask::Save),
            "knobs" => Some(Ask::Knobs),
            "turn" => Some(Ask::Turn { knob: parts.next()?.parse().ok()?, value: parts.next()?.parse().ok()? }),
            "restore" => Some(Ask::Restore(bytes_of(parts.next().unwrap_or(""))?)),
            "quit" => Some(Ask::Quit),
            _ => None,
        }
    }
}

impl Reply {
    pub fn write(&self, out: &mut impl Write) -> std::io::Result<()> {
        let line = match self {
            Reply::Classes(names) => format!("classes\t{}", names.join("\x1f")),
            Reply::Loaded { inputs, outputs, latency, ara } => format!("loaded\t{inputs}\t{outputs}\t{latency}\t{}", *ara as u8),
            Reply::State(state) => format!("state\t{}", hex_of(state)),
            Reply::Knobs(names) => format!("knobs\t{}", names.join("\x1f")),
            Reply::Fine => "fine".to_string(),
            Reply::Trouble(why) => format!("trouble\t{}", why.replace('\n', " ")),
        };
        writeln!(out, "{line}")?;
        out.flush()
    }

    pub fn read(line: &str) -> Option<Self> {
        let mut parts = line.trim_end().split('\t');
        match parts.next()? {
            "classes" => {
                let rest = parts.next().unwrap_or("");
                let names = if rest.is_empty() { Vec::new() } else { rest.split('\x1f').map(str::to_string).collect() };
                Some(Reply::Classes(names))
            }
            "loaded" => Some(Reply::Loaded {
                inputs: parts.next()?.parse().ok()?,
                outputs: parts.next()?.parse().ok()?,
                latency: parts.next().and_then(|got| got.parse().ok()).unwrap_or(0),
                ara: parts.next() == Some("1"),
            }),
            "state" => Some(Reply::State(bytes_of(parts.next().unwrap_or(""))?)),
            "knobs" => {
                let rest = parts.next().unwrap_or("");
                let names = if rest.is_empty() { Vec::new() } else { rest.split('\x1f').map(str::to_string).collect() };
                Some(Reply::Knobs(names))
            }
            "fine" => Some(Reply::Fine),
            "trouble" => Some(Reply::Trouble(parts.next().unwrap_or("something went wrong").to_string())),
            _ => None,
        }
    }
}

pub fn next_line(from: &mut BufReader<impl Read>) -> Option<String> {
    let mut line = String::new();
    match from.read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line),
    }
}

pub fn write_block(out: &mut impl Write, audio: &[[f32; 2]]) -> std::io::Result<()> {
    let frames = audio.len() as u32;
    out.write_all(b"B")?;
    out.write_all(&frames.to_le_bytes())?;
    for frame in audio {
        out.write_all(&frame[0].to_le_bytes())?;
        out.write_all(&frame[1].to_le_bytes())?;
    }
    out.flush()
}

pub fn read_block(from: &mut impl Read, audio: &mut Vec<[f32; 2]>) -> std::io::Result<()> {
    let mut mark = [0u8; 1];
    from.read_exact(&mut mark)?;
    if mark[0] != b'B' {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "that was not a block of audio"));
    }
    let mut count = [0u8; 4];
    from.read_exact(&mut count)?;
    let frames = u32::from_le_bytes(count) as usize;
    audio.clear();
    audio.reserve(frames);
    let mut pair = [0u8; 8];
    for _ in 0..frames {
        from.read_exact(&mut pair)?;
        let left = f32::from_le_bytes([pair[0], pair[1], pair[2], pair[3]]);
        let right = f32::from_le_bytes([pair[4], pair[5], pair[6], pair[7]]);
        audio.push([left, right]);
    }
    Ok(())
}

pub fn hex_of(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from_digit((byte >> 4) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((byte & 15) as u32, 16).unwrap_or('0'));
    }
    out
}

pub fn bytes_of(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    let raw = text.as_bytes();
    let mut out = Vec::with_capacity(text.len() / 2);
    for pair in raw.chunks(2) {
        let two = std::str::from_utf8(pair).ok()?;
        out.push(u8::from_str_radix(two, 16).ok()?);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_ask_survives_the_wire() {
        let asks = [
            Ask::Classes("/a/b.vst3".into()),
            Ask::Load { path: "/a/b.vst3".into(), index: 2, rate: 48_000, block: 512 },
            Ask::Process,
            Ask::ProcessWithSide,
            Ask::ProcessAt(96_000),
            Ask::ProcessAt(-12),
            Ask::Region(Region {
                file: "C:\\Songs\\lead vocal.wav".into(),
                name: "Lead".into(),
                start: 12.5,
                offset: 0.125,
                length: 3.0000000000000004,
                stretch: 1.25,
                tempo: 140.0,
            }),
            Ask::Show,
            Ask::Hide,
            Ask::Save,
            Ask::Knobs,
            Ask::Turn { knob: 3, value: 0.25 },
            Ask::Restore(vec![0, 15, 16, 255]),
            Ask::Quit,
        ];
        for ask in asks {
            let mut written = Vec::new();
            ask.write(&mut written).unwrap();
            let text = String::from_utf8(written).unwrap();
            assert_eq!(Ask::read(&text), Some(ask));
        }
    }

    #[test]
    fn every_reply_survives_the_wire() {
        let replies = [
            Reply::Classes(vec!["One".into(), "Two".into()]),
            Reply::Loaded { inputs: 2, outputs: 2, latency: 64, ara: false },
            Reply::Loaded { inputs: 2, outputs: 2, latency: 0, ara: true },
            Reply::State(vec![1, 2, 3, 250]),
            Reply::Knobs(vec!["Threshold".into(), "Ratio".into()]),
            Reply::Fine,
            Reply::Trouble("it broke".into()),
        ];
        for reply in replies {
            let mut written = Vec::new();
            reply.write(&mut written).unwrap();
            let text = String::from_utf8(written).unwrap();
            assert_eq!(Reply::read(&text), Some(reply));
        }
    }
}

#[cfg(test)]
mod older_tests {
    use super::*;

    #[test]
    fn a_host_that_says_nothing_about_ara_is_not_ara() {
        assert_eq!(Reply::read("loaded\t2\t2\t64\n"), Some(Reply::Loaded { inputs: 2, outputs: 2, latency: 64, ara: false }));
        assert_eq!(Reply::read("loaded\t2\t2\n"), Some(Reply::Loaded { inputs: 2, outputs: 2, latency: 0, ara: false }));
    }

    #[test]
    fn a_tab_in_a_clip_name_does_not_break_the_line() {
        let region = Region {
            file: "a.wav".into(),
            name: "two\twords".into(),
            start: 0.0,
            offset: 0.0,
            length: 1.0,
            stretch: 1.0,
            tempo: 120.0,
        };
        let mut written = Vec::new();
        Ask::Region(region.clone()).write(&mut written).unwrap();
        let Some(Ask::Region(back)) = Ask::read(&String::from_utf8(written).unwrap()) else { panic!("it did not come back") };
        assert_eq!(back.name, "two words");
        assert_eq!(back.length, region.length);
    }
}
