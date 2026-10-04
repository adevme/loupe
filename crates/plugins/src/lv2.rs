use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};

#[repr(C)]
struct Feature {
    uri: *const c_char,
    data: *mut c_void,
}

#[repr(C)]
struct Descriptor {
    uri: *const c_char,
    instantiate: Option<
        unsafe extern "C" fn(
            descriptor: *const Descriptor,
            rate: f64,
            bundle: *const c_char,
            features: *const *const Feature,
        ) -> *mut c_void,
    >,
    connect_port: Option<unsafe extern "C" fn(handle: *mut c_void, port: u32, data: *mut c_void)>,
    activate: Option<unsafe extern "C" fn(handle: *mut c_void)>,
    run: Option<unsafe extern "C" fn(handle: *mut c_void, count: u32)>,
    deactivate: Option<unsafe extern "C" fn(handle: *mut c_void)>,
    cleanup: Option<unsafe extern "C" fn(handle: *mut c_void)>,
    extension_data: Option<unsafe extern "C" fn(uri: *const c_char) -> *const c_void>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Port {
    pub index: u32,
    pub audio: bool,
    pub input: bool,
    pub default: f32,
    pub latency: bool,
    pub sidechain: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Described {
    pub uri: String,
    pub name: String,
    pub binary: PathBuf,
    pub ports: Vec<Port>,
}

pub fn plugins_in(bundle: &Path) -> Vec<Described> {
    let Ok(manifest) = std::fs::read_to_string(bundle.join("manifest.ttl")) else {
        return Vec::new();
    };
    let prefixes = prefixes_of(&manifest);
    let mut out = Vec::new();
    for block in blocks_of(&manifest) {
        if !block.body.contains("lv2:Plugin") && !block.body.contains("#Plugin") {
            continue;
        }
        let Some(binary) = angled_after(&block.body, "lv2:binary") else { continue };
        let also = angled_after(&block.body, "rdfs:seeAlso");
        let long = expand(&block.subject, &prefixes);
        let mut name = long.trim_end_matches('/').rsplit('/').next().unwrap_or(&long).to_string();
        let mut ports = Vec::new();
        if let Some(also) = also {
            if let Ok(detail) = std::fs::read_to_string(bundle.join(&also)) {
                let region = region_for(&detail, &block.subject, &long);
                if let Some(said) = quoted_after(region, "doap:name") {
                    name = said;
                }
                ports = ports_in(region);
            }
        }
        out.push(Described { uri: long, name, binary: bundle.join(binary), ports });
    }
    out
}

fn prefixes_of(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("@prefix") else { continue };
        let rest = rest.trim();
        let Some((short, tail)) = rest.split_once(':') else { continue };
        let Some(open) = tail.find('<') else { continue };
        let Some(close) = tail[open + 1..].find('>') else { continue };
        out.push((short.trim().to_string(), tail[open + 1..open + 1 + close].to_string()));
    }
    out
}

fn expand(subject: &str, prefixes: &[(String, String)]) -> String {
    if subject.starts_with("http") {
        return subject.to_string();
    }
    let Some((short, rest)) = subject.split_once(':') else { return subject.to_string() };
    for (name, base) in prefixes {
        if name == short {
            return format!("{base}{rest}");
        }
    }
    subject.to_string()
}

fn region_for<'a>(detail: &'a str, short: &str, long: &str) -> &'a str {
    let at = detail
        .find(&format!("\n{short}\n"))
        .or_else(|| detail.find(&format!("\n{short}\t")))
        .or_else(|| detail.find(&format!("\n<{long}>")))
        .or_else(|| detail.find(short))
        .unwrap_or(0);
    let rest = &detail[at..];
    match rest.find("\n.") {
        Some(end) => &rest[..end],
        None => rest,
    }
}

struct Block {
    subject: String,
    body: String,
}

fn blocks_of(text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut subject = String::new();
    let mut body = String::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim_end();
        if line.trim_start().starts_with('@') {
            continue;
        }
        if subject.is_empty() {
            let bare = line.trim();
            if bare.is_empty() {
                continue;
            }
            if line.starts_with(char::is_whitespace) {
                continue;
            }
            let (named, rest) = match bare.strip_prefix('<').and_then(|rest| rest.split_once('>')) {
                Some((uri, rest)) => (uri.to_string(), rest.to_string()),
                None => {
                    let cut = bare.find(char::is_whitespace).unwrap_or(bare.len());
                    (bare[..cut].to_string(), bare[cut..].to_string())
                }
            };
            subject = named;
            body = format!("{rest}\n");
            if rest.trim_end().ends_with('.') {
                blocks.push(Block { subject: std::mem::take(&mut subject), body: std::mem::take(&mut body) });
            }
            continue;
        }
        body.push_str(line);
        body.push('\n');
        if line.trim_end().ends_with('.') {
            blocks.push(Block { subject: std::mem::take(&mut subject), body: std::mem::take(&mut body) });
        }
    }
    if !subject.is_empty() {
        blocks.push(Block { subject, body });
    }
    blocks
}

fn angled_after(text: &str, key: &str) -> Option<String> {
    let at = text.find(key)?;
    let rest = &text[at + key.len()..];
    let open = rest.find('<')?;
    let close = rest[open + 1..].find('>')?;
    Some(rest[open + 1..open + 1 + close].to_string())
}

fn quoted_after(text: &str, key: &str) -> Option<String> {
    let at = text.find(key)?;
    let rest = &text[at + key.len()..];
    let open = rest.find('"')?;
    let close = rest[open + 1..].find('"')?;
    Some(rest[open + 1..open + 1 + close].to_string())
}

fn number_after(text: &str, key: &str) -> Option<f32> {
    let at = text.find(key)?;
    let rest = text[at + key.len()..].trim_start();
    let end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+' || c == 'e'))?;
    rest[..end].parse().ok()
}

fn ports_in(region: &str) -> Vec<Port> {
    let mut ports = Vec::new();
    // A plugin lists its ports once, so the first list is the whole of it.
    if let Some(start) = region.find("lv2:port") {
        let rest = &region[start + "lv2:port".len()..];
        let stop = rest.find("\n\t.").or_else(|| rest.find("\n.")).unwrap_or(rest.len());
        let region = &rest[..stop];
        for chunk in region.split('[').skip(1) {
            let chunk = chunk.split(']').next().unwrap_or("");
            let Some(index) = number_after(chunk, "lv2:index") else { continue };
            ports.push(Port {
                index: index as u32,
                audio: chunk.contains("AudioPort"),
                input: chunk.contains("InputPort"),
                default: number_after(chunk, "lv2:default").unwrap_or(0.0),
                latency: chunk.contains("lv2:latency") || chunk.contains("reportsLatency"),
                sidechain: chunk.contains("isSideChain") || chunk.contains("sidechain"),
            });
        }
    }
    ports.sort_by_key(|port| port.index);
    ports
}

pub struct Library {
    lib: libloading::Library,
    bundle: PathBuf,
    pub described: Vec<Described>,
}

impl Library {
    pub fn open(bundle: &Path) -> Result<Self, String> {
        let described = plugins_in(bundle);
        let first = described.first().ok_or("this bundle holds no LV2 plugin")?;
        let lib = unsafe { libloading::Library::new(&first.binary) }
            .map_err(|why| format!("{} could not be opened: {why}", first.binary.display()))?;
        Ok(Self { lib, bundle: bundle.to_path_buf(), described })
    }

    pub fn classes(&self) -> Vec<crate::vst3::Class> {
        self.described
            .iter()
            .map(|one| crate::vst3::Class { name: one.name.clone(), category: "Fx".into(), id: [0; 16] })
            .collect()
    }
}

pub struct Effect {
    handle: *mut c_void,
    descriptor: *const Descriptor,
    left: Vec<f32>,
    right: Vec<f32>,
    out_left: Vec<f32>,
    out_right: Vec<f32>,
    side_left: Vec<f32>,
    side_right: Vec<f32>,
    controls: Vec<f32>,
    latency_at: Option<usize>,
    side_at: Vec<usize>,
    _library: Library,
}

unsafe impl Send for Effect {}

impl Effect {
    pub fn start(library: Library, index: usize, rate: f64, block: usize) -> Result<Self, String> {
        let wanted = library.described.get(index).ok_or("that bundle has no such plugin")?.clone();
        let ins: Vec<_> = wanted.ports.iter().filter(|port| port.audio && port.input && !port.sidechain).collect();
        let outs: Vec<_> = wanted.ports.iter().filter(|port| port.audio && !port.input).collect();
        if wanted.ports.is_empty() {
            return Err("Loupe could not read this plugin's ports".into());
        }
        if ins.len() > 2 || outs.is_empty() || outs.len() > 2 {
            return Err("Loupe only hosts LV2 plugins with one or two audio channels".into());
        }
        let uri = CString::new(wanted.uri.as_str()).map_err(|_| "that plugin's name cannot be passed on")?;
        unsafe {
            let find = library
                .lib
                .get::<unsafe extern "C" fn(u32) -> *const Descriptor>(b"lv2_descriptor\0")
                .map_err(|_| "this file is not an LV2 plugin".to_string())?;
            let mut descriptor = std::ptr::null();
            for at in 0..64u32 {
                let one = find(at);
                if one.is_null() {
                    break;
                }
                if CStr::from_ptr((*one).uri) == uri.as_c_str() {
                    descriptor = one;
                    break;
                }
            }
            if descriptor.is_null() {
                return Err("the plugin does not know that name".into());
            }
            let here = CString::new(library.bundle.to_string_lossy().as_bytes()).map_err(|_| "that path cannot be passed on")?;
            let none: *const Feature = std::ptr::null();
            let make = (*descriptor).instantiate.ok_or("the plugin cannot be made")?;
            let handle = make(descriptor, rate, here.as_ptr(), &none);
            if handle.is_null() {
                return Err("the plugin would not start up".into());
            }
            let mut effect = Self {
                handle,
                descriptor,
                left: vec![0.0; block],
                right: vec![0.0; block],
                out_left: vec![0.0; block],
                out_right: vec![0.0; block],
                side_left: vec![0.0; block],
                side_right: vec![0.0; block],
                controls: wanted.ports.iter().map(|port| port.default).collect(),
                latency_at: wanted.ports.iter().position(|port| port.latency && !port.audio),
                side_at: wanted
                    .ports
                    .iter()
                    .enumerate()
                    .filter(|(_, port)| port.audio && port.input && port.sidechain)
                    .map(|(at, _)| at)
                    .collect(),
                _library: library,
            };
            let connect = (*descriptor).connect_port.ok_or("the plugin has no ports to connect")?;
            for (slot, port) in wanted.ports.iter().enumerate() {
                let data: *mut c_void = if port.audio && port.input && port.sidechain {
                    let which = effect.side_at.iter().position(|other| *other == slot).unwrap_or(0);
                    if which == 0 { effect.side_left.as_mut_ptr() as *mut c_void } else { effect.side_right.as_mut_ptr() as *mut c_void }
                } else if port.audio && port.input {
                    let which = ins.iter().position(|other| other.index == port.index).unwrap_or(0);
                    if which == 0 { effect.left.as_mut_ptr() as *mut c_void } else { effect.right.as_mut_ptr() as *mut c_void }
                } else if port.audio {
                    let which = outs.iter().position(|other| other.index == port.index).unwrap_or(0);
                    if which == 0 { effect.out_left.as_mut_ptr() as *mut c_void } else { effect.out_right.as_mut_ptr() as *mut c_void }
                } else {
                    effect.controls[slot..].as_mut_ptr() as *mut c_void
                };
                connect(handle, port.index, data);
            }
            if let Some(wake) = (*descriptor).activate {
                wake(handle);
            }
            Ok(effect)
        }
    }

    pub fn process(&mut self, audio: &mut [[f32; 2]]) {
        self.process_with(audio, &[]);
    }

    pub fn process_with(&mut self, audio: &mut [[f32; 2]], side: &[[f32; 2]]) {
        let frames = audio.len().min(self.left.len());
        for (i, frame) in audio.iter().take(frames).enumerate() {
            self.left[i] = frame[0];
            self.right[i] = frame[1];
        }
        for i in 0..frames {
            let frame = side.get(i).copied().unwrap_or([0.0; 2]);
            self.side_left[i] = frame[0];
            self.side_right[i] = frame[1];
        }
        unsafe {
            if let Some(run) = (*self.descriptor).run {
                run(self.handle, frames as u32);
            }
        }
        for (i, frame) in audio.iter_mut().take(frames).enumerate() {
            frame[0] = self.out_left[i];
            frame[1] = self.out_right[i];
        }
    }

    pub fn latency(&self) -> usize {
        match self.latency_at {
            Some(slot) => self.controls.get(slot).copied().unwrap_or(0.0).max(0.0) as usize,
            None => 0,
        }
    }

    pub fn turn(&mut self, knob: usize, value: f32) {
        if let Some(slot) = self.controls.get_mut(knob) {
            *slot = value;
        }
    }

    pub fn save(&self) -> Result<Vec<u8>, String> {
        let mut out = Vec::with_capacity(self.controls.len() * 4);
        for value in &self.controls {
            out.extend_from_slice(&value.to_le_bytes());
        }
        Ok(out)
    }

    pub fn restore(&mut self, state: &[u8]) -> Result<(), String> {
        if state.is_empty() {
            return Ok(());
        }
        if state.len() != self.controls.len() * 4 {
            return Err("those settings are not for this plugin".into());
        }
        for (slot, four) in state.chunks_exact(4).enumerate() {
            self.controls[slot] = f32::from_le_bytes([four[0], four[1], four[2], four[3]]);
        }
        Ok(())
    }
}

impl Drop for Effect {
    fn drop(&mut self) {
        unsafe {
            if let Some(sleep) = (*self.descriptor).deactivate {
                sleep(self.handle);
            }
            if let Some(gone) = (*self.descriptor).cleanup {
                gone(self.handle);
            }
        }
    }
}
