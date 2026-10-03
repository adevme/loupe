use std::io::{BufRead, BufReader, Read, Write};

#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    Classes(String),
    Load { path: String, index: usize, rate: u32, block: usize },
    Quit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    Classes(Vec<String>),
    Loaded { inputs: usize, outputs: usize },
    Trouble(String),
}

impl Ask {
    pub fn write(&self, out: &mut impl Write) -> std::io::Result<()> {
        let line = match self {
            Ask::Classes(path) => format!("classes\t{path}"),
            Ask::Load { path, index, rate, block } => format!("load\t{path}\t{index}\t{rate}\t{block}"),
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
            "quit" => Some(Ask::Quit),
            _ => None,
        }
    }
}

impl Reply {
    pub fn write(&self, out: &mut impl Write) -> std::io::Result<()> {
        let line = match self {
            Reply::Classes(names) => format!("classes\t{}", names.join("\x1f")),
            Reply::Loaded { inputs, outputs } => format!("loaded\t{inputs}\t{outputs}"),
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
            "loaded" => Some(Reply::Loaded { inputs: parts.next()?.parse().ok()?, outputs: parts.next()?.parse().ok()? }),
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
