use std::path::PathBuf;

use crate::file::{bytes_of, fields_of, hex_of};
use crate::model::Fx;

const HEADER: &str = "loupe chain 1";

pub fn chain_text(chain: &[Fx]) -> String {
    let mut out = format!("{HEADER}\n");
    for fx in chain {
        let state = if fx.state.is_empty() { "-".to_string() } else { hex_of(&fx.state) };
        out.push_str(&format!("fxpath {}\n", fx.path.display()));
        out.push_str(&format!("fx index={} bypass={} state={state} name={}\n", fx.index, fx.bypassed as u8, fx.name));
    }
    out
}

pub fn chain_from(text: &str) -> Result<Vec<Fx>, String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some(HEADER) {
        return Err("this is not a Loupe chain".into());
    }
    let mut chain = Vec::new();
    let mut path: Option<PathBuf> = None;
    for line in lines {
        let (kind, rest) = line.split_once(' ').unwrap_or((line, ""));
        match kind {
            "fxpath" => path = Some(PathBuf::from(rest)),
            "fx" => {
                let (fields, name) = rest.split_once("name=").ok_or("a plugin in the chain has no name")?;
                let fields = fields_of(fields);
                let state = match fields.get("state") {
                    Some(&"-") | None => Vec::new(),
                    Some(text) => bytes_of(text).ok_or("a plugin's settings in the chain are not readable")?,
                };
                chain.push(Fx {
                    path: path.take().ok_or("a plugin in the chain has no file")?,
                    index: fields.get("index").and_then(|v| v.parse().ok()).unwrap_or(0),
                    name: name.to_string(),
                    bypassed: fields.get("bypass") == Some(&"1"),
                    state,
                    record: false,
                });
            }
            _ => {}
        }
    }
    Ok(chain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chain_comes_back_as_it_was_saved() {
        let chain = vec![
            Fx { path: PathBuf::from("loupe.loupe"), index: 0, name: "Loupe EQ".into(), bypassed: false, state: vec![1, 2, 250], record: true },
            Fx { path: PathBuf::from("C:\\VST3\\Pro-DS.vst3"), index: 0, name: "FabFilter Pro-DS".into(), bypassed: true, state: Vec::new(), record: false },
        ];
        let back = chain_from(&chain_text(&chain)).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!((back[0].name.as_str(), back[0].state.clone(), back[0].record), ("Loupe EQ", vec![1, 2, 250], false));
        assert_eq!((back[1].path.clone(), back[1].bypassed), (PathBuf::from("C:\\VST3\\Pro-DS.vst3"), true));
        assert!(chain_from("not a chain").is_err());
    }
}
