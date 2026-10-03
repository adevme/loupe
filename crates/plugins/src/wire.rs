use std::io::{BufRead, BufReader, Read, Write};

#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    Classes(String),
    Load { path: String, index: usize, rate: u32, block: usize },
    Process,
    Show,
    Hide,
    Save,
    Restore(Vec<u8>),
    Quit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    Classes(Vec<String>),
    Loaded { inputs: usize, outputs: usize, latency: usize },
    State(Vec<u8>),
    Fine,
    Trouble(String),
}

impl Ask {
    pub fn write(&self, out: &mut impl Write) -> std::io::Result<()> {
        let line = match self {
            Ask::Classes(path) => format!("classes\t{path}"),
            Ask::Load { path, index, rate, block } => format!("load\t{path}\t{index}\t{rate}\t{block}"),
            Ask::Process => "process".to_string(),
            Ask::Show => "show".to_string(),
            Ask::Hide => "hide".to_string(),
            Ask::Save => "save".to_string(),
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
            "show" => Some(Ask::Show),
            "hide" => Some(Ask::Hide),
            "save" => Some(Ask::Save),
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
            Reply::Loaded { inputs, outputs, latency } => format!("loaded\t{inputs}\t{outputs}\t{latency}"),
            Reply::State(state) => format!("state\t{}", hex_of(state)),
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
            }),
            "state" => Some(Reply::State(bytes_of(parts.next().unwrap_or(""))?)),
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
            Ask::Show,
            Ask::Hide,
            Ask::Save,
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
            Reply::Loaded { inputs: 2, outputs: 2, latency: 64 },
            Reply::State(vec![1, 2, 3, 250]),
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
