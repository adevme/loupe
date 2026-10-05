use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};

use clap_sys::entry::clap_plugin_entry;
use clap_sys::events::{clap_event_header, clap_input_events, clap_output_events};
use clap_sys::ext::state::{clap_plugin_state, CLAP_EXT_STATE};
use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
use clap_sys::host::clap_host;
use clap_sys::plugin::clap_plugin;
use clap_sys::process::{clap_process, CLAP_PROCESS_ERROR};
use clap_sys::stream::{clap_istream, clap_ostream};
use clap_sys::version::CLAP_VERSION;

use crate::vst3::Class;

pub fn binary_in(bundle: &Path) -> PathBuf {
    if bundle.is_file() {
        return bundle.to_path_buf();
    }
    let machine = if cfg!(windows) {
        "x86_64-win"
    } else if cfg!(target_os = "macos") {
        "MacOS"
    } else {
        "x86_64-linux"
    };
    let inside = bundle.join("Contents").join(machine);
    std::fs::read_dir(&inside)
        .ok()
        .and_then(|entries| entries.flatten().map(|entry| entry.path()).find(|path| path.is_file()))
        .unwrap_or_else(|| bundle.to_path_buf())
}

pub struct Library {
    entry: *const clap_plugin_entry,
    factory: *const clap_plugin_factory,
    _lib: libloading::Library,
}

impl Library {
    pub fn open(bundle: &Path) -> Result<Self, String> {
        let binary = binary_in(bundle);
        let lib = unsafe { libloading::Library::new(&binary) }
            .map_err(|why| format!("{} could not be opened: {why}", binary.display()))?;
        unsafe {
            let symbol = lib
                .get::<*const clap_plugin_entry>(b"clap_entry\0")
                .map_err(|_| "this file is not a CLAP plugin".to_string())?;
            let entry = *symbol;
            if entry.is_null() {
                return Err("this CLAP plugin has no way in".into());
            }
            let here = CString::new(binary.to_string_lossy().as_bytes()).map_err(|_| "that path cannot be passed on")?;
            let started = (*entry).init.map(|init| init(here.as_ptr())).unwrap_or(true);
            if !started {
                return Err("the plugin refused to start".into());
            }
            let got = (*entry).get_factory.map(|get| get(CLAP_PLUGIN_FACTORY_ID.as_ptr())).unwrap_or(std::ptr::null());
            if got.is_null() {
                return Err("the plugin offers nothing to load".into());
            }
            Ok(Self { entry, factory: got as *const clap_plugin_factory, _lib: lib })
        }
    }

    pub fn classes(&self) -> Vec<Class> {
        let mut classes = Vec::new();
        unsafe {
            let count = (*self.factory).get_plugin_count.map(|count| count(self.factory)).unwrap_or(0);
            for index in 0..count {
                let Some(describe) = (*self.factory).get_plugin_descriptor else { continue };
                let about = describe(self.factory, index);
                if about.is_null() {
                    continue;
                }
                let text = |raw: *const c_char| {
                    if raw.is_null() {
                        String::new()
                    } else {
                        CStr::from_ptr(raw).to_string_lossy().into_owned()
                    }
                };
                classes.push(Class {
                    name: text((*about).name),
                    category: text((*about).vendor),
                    id: [0; 16],
                });
            }
        }
        classes
    }

    fn id_at(&self, index: usize) -> Option<CString> {
        unsafe {
            let describe = (*self.factory).get_plugin_descriptor?;
            let about = describe(self.factory, index as u32);
            if about.is_null() || (*about).id.is_null() {
                return None;
            }
            Some(CStr::from_ptr((*about).id).to_owned())
        }
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        unsafe {
            if let Some(stop) = (*self.entry).deinit {
                stop();
            }
        }
    }
}

unsafe extern "C" fn no_extension(_host: *const clap_host, _id: *const c_char) -> *const c_void {
    std::ptr::null()
}

unsafe extern "C" fn nothing_to_do(_host: *const clap_host) {}

unsafe extern "C" fn how_many(list: *const clap_input_events) -> u32 {
    let held = &*((*list).ctx as *const Vec<clap_sys::events::clap_event_param_value>);
    held.len() as u32
}

unsafe extern "C" fn one_event(list: *const clap_input_events, index: u32) -> *const clap_event_header {
    let held = &*((*list).ctx as *const Vec<clap_sys::events::clap_event_param_value>);
    match held.get(index as usize) {
        Some(found) => &found.header as *const clap_event_header,
        None => std::ptr::null(),
    }
}

unsafe extern "C" fn drop_event(_list: *const clap_output_events, _event: *const clap_event_header) -> bool {
    true
}

pub struct Effect {
    plugin: *const clap_plugin,
    host: Box<clap_host>,
    left: Vec<f32>,
    right: Vec<f32>,
    side_left: Vec<f32>,
    side_right: Vec<f32>,
    ids: Vec<u32>,
    ranges: Vec<(f64, f64)>,
    steps: Vec<u32>,
    waiting: Vec<clap_sys::events::clap_event_param_value>,
    _library: Library,
}

unsafe impl Send for Effect {}

impl Effect {
    pub fn start(library: Library, index: usize, rate: f64, block: usize) -> Result<Self, String> {
        let id = library.id_at(index).ok_or("that plugin has no such part")?;
        let name = CString::new("Loupe").unwrap();
        let empty = CString::new("").unwrap();
        let mut host = Box::new(clap_host {
            clap_version: CLAP_VERSION,
            host_data: std::ptr::null_mut(),
            name: name.as_ptr(),
            vendor: empty.as_ptr(),
            url: empty.as_ptr(),
            version: empty.as_ptr(),
            get_extension: Some(no_extension),
            request_restart: Some(nothing_to_do),
            request_process: Some(nothing_to_do),
            request_callback: Some(nothing_to_do),
        });
        std::mem::forget(name);
        std::mem::forget(empty);
        unsafe {
            let make = (*library.factory).create_plugin.ok_or("this plugin cannot be made")?;
            let plugin = make(library.factory, host.as_mut() as *mut clap_host, id.as_ptr());
            if plugin.is_null() {
                return Err("the plugin would not be made".into());
            }
            if !(*plugin).init.map(|init| init(plugin)).unwrap_or(false) {
                return Err("the plugin would not start up".into());
            }
            let awake = (*plugin).activate.map(|go| go(plugin, rate, 1, block as u32)).unwrap_or(false);
            if !awake {
                return Err("the plugin refused this sample rate or block size".into());
            }
            if let Some(go) = (*plugin).start_processing {
                go(plugin);
            }
            Ok(Self {
                plugin,
                host,
                left: vec![0.0; block],
                right: vec![0.0; block],
                side_left: vec![0.0; block],
                side_right: vec![0.0; block],
                ids: Vec::new(),
                ranges: Vec::new(),
                steps: Vec::new(),
                waiting: Vec::new(),
                _library: library,
            })
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
            let mut channels = [self.left.as_mut_ptr(), self.right.as_mut_ptr()];
            let mut side_channels = [self.side_left.as_mut_ptr(), self.side_right.as_mut_ptr()];
            let mut bus = clap_sys::audio_buffer::clap_audio_buffer {
                data32: channels.as_mut_ptr(),
                data64: std::ptr::null_mut(),
                channel_count: 2,
                latency: 0,
                constant_mask: 0,
            };
            let side_bus = clap_sys::audio_buffer::clap_audio_buffer {
                data32: side_channels.as_mut_ptr(),
                data64: std::ptr::null_mut(),
                channel_count: 2,
                latency: 0,
                constant_mask: 0,
            };
            let ins = [bus, side_bus];
            let coming = clap_input_events {
                ctx: &self.waiting as *const Vec<clap_sys::events::clap_event_param_value> as *mut c_void,
                size: Some(how_many),
                get: Some(one_event),
            };
            let going = clap_output_events { ctx: std::ptr::null_mut(), try_push: Some(drop_event) };
            let data = clap_process {
                steady_time: -1,
                frames_count: frames as u32,
                transport: std::ptr::null(),
                audio_inputs: ins.as_ptr(),
                audio_outputs: &mut bus,
                audio_inputs_count: if side.is_empty() { 1 } else { 2 },
                audio_outputs_count: 1,
                in_events: &coming,
                out_events: &going,
            };
            if let Some(run) = (*self.plugin).process {
                if run(self.plugin, &data) == CLAP_PROCESS_ERROR {
                    return;
                }
            }
        }
        for (i, frame) in audio.iter_mut().take(frames).enumerate() {
            frame[0] = self.left[i];
            frame[1] = self.right[i];
        }
    }

    pub fn knobs(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen = Vec::new();
        let mut spans = Vec::new();
        let mut stepped = Vec::new();
        unsafe {
            let Some(get) = (*self.plugin).get_extension else { return out };
            let found = get(self.plugin, clap_sys::ext::params::CLAP_EXT_PARAMS.as_ptr());
            if found.is_null() {
                return out;
            }
            let part = found as *const clap_sys::ext::params::clap_plugin_params;
            let count = (*part).count.map(|count| count(self.plugin)).unwrap_or(0);
            let Some(about) = (*part).get_info else { return out };
            for index in 0..count.min(crate::wording::MOST_KNOBS as _) {
                let mut info: clap_sys::ext::params::clap_param_info = std::mem::zeroed();
                if !about(self.plugin, index, &mut info) {
                    continue;
                }
                let raw: Vec<u8> = info.name.iter().take_while(|byte| **byte != 0).map(|byte| *byte as u8).collect();
                out.push(String::from_utf8_lossy(&raw).into_owned());
                seen.push(info.id);
                spans.push((info.min_value, info.max_value));
                stepped.push(if info.flags & clap_sys::ext::params::CLAP_PARAM_IS_STEPPED != 0 { (info.max_value - info.min_value).round().max(0.0) as u32 } else { 0 });
            }
        }
        self.ids = seen;
        self.ranges = spans;
        self.steps = stepped;
        out
    }

    pub fn turn(&mut self, knob: usize, value: f32) {
        if self.ids.is_empty() {
            let _ = self.knobs();
        }
        let Some(id) = self.ids.get(knob).copied() else { return };
        let (low, high) = self.ranges.get(knob).copied().unwrap_or((0.0, 1.0));
        let value = low + (high - low) * value.clamp(0.0, 1.0) as f64;
        let event = clap_sys::events::clap_event_param_value {
            header: clap_sys::events::clap_event_header {
                size: std::mem::size_of::<clap_sys::events::clap_event_param_value>() as u32,
                time: 0,
                space_id: clap_sys::events::CLAP_CORE_EVENT_SPACE_ID,
                type_: clap_sys::events::CLAP_EVENT_PARAM_VALUE,
                flags: 0,
            },
            param_id: id,
            cookie: std::ptr::null_mut(),
            note_id: -1,
            port_index: -1,
            channel: -1,
            key: -1,
            value,
        };
        self.waiting.retain(|kept| kept.param_id != id);
        self.waiting.push(event);
    }

    fn params(&self) -> Option<*const clap_sys::ext::params::clap_plugin_params> {
        unsafe {
            let get = (*self.plugin).get_extension?;
            let found = get(self.plugin, clap_sys::ext::params::CLAP_EXT_PARAMS.as_ptr());
            (!found.is_null()).then_some(found as *const clap_sys::ext::params::clap_plugin_params)
        }
    }

    fn spread(&self, knob: usize, plain: f64) -> f32 {
        let (low, high) = self.ranges.get(knob).copied().unwrap_or((0.0, 1.0));
        if high > low { ((plain - low) / (high - low)).clamp(0.0, 1.0) as f32 } else { 0.0 }
    }

    pub fn readings(&mut self) -> Vec<crate::wire::Reading> {
        let names = self.knobs();
        let Some(part) = self.params() else { return Vec::new() };
        let mut out = Vec::new();
        for (knob, (name, id)) in names.into_iter().zip(self.ids.clone()).enumerate() {
            let waiting = self.waiting.iter().find(|kept| kept.param_id == id).map(|kept| kept.value);
            let mut plain = 0.0f64;
            let known = waiting.is_some() || unsafe { (*part).get_value.map(|get| get(self.plugin, id, &mut plain)).unwrap_or(false) };
            let plain = waiting.unwrap_or(plain);
            let mut words = [0 as std::ffi::c_char; 128];
            let text = unsafe {
                match (*part).value_to_text {
                    Some(say) if known && say(self.plugin, id, plain, words.as_mut_ptr(), words.len() as u32) => {
                        std::ffi::CStr::from_ptr(words.as_ptr()).to_string_lossy().trim().to_string()
                    }
                    _ => String::new(),
                }
            };
            out.push(crate::wire::Reading { name, value: self.spread(knob, plain), text });
        }
        out
    }

    pub fn from_text(&mut self, knob: usize, text: &str) -> Option<f32> {
        if self.ids.is_empty() {
            let _ = self.knobs();
        }
        let id = self.ids.get(knob).copied()?;
        let part = self.params()?;
        let words = std::ffi::CString::new(text).ok()?;
        let mut plain = 0.0f64;
        let read = unsafe { (*part).text_to_value.is_some_and(|read| read(self.plugin, id, words.as_ptr(), &mut plain)) };
        if read && plain.is_finite() {
            return Some(self.spread(knob, plain));
        }
        let (low, high) = self.ranges.get(knob).copied().unwrap_or((0.0, 1.0));
        let say_text = unsafe { (*part).value_to_text? };
        let plugin = self.plugin;
        let say = |value: f64| {
            let mut words = [0 as std::ffi::c_char; 128];
            unsafe { say_text(plugin, id, low + (high - low) * value, words.as_mut_ptr(), words.len() as u32) }
                .then(|| unsafe { std::ffi::CStr::from_ptr(words.as_ptr()) }.to_string_lossy().into_owned())
        };
        let steps = self.steps.get(knob).copied().unwrap_or(0);
        crate::wording::find_value(steps, text, say).map(|value| value as f32)
    }

    pub fn latency(&self) -> usize {
        unsafe {
            let Some(get) = (*self.plugin).get_extension else { return 0 };
            let found = get(self.plugin, clap_sys::ext::latency::CLAP_EXT_LATENCY.as_ptr());
            if found.is_null() {
                return 0;
            }
            let part = found as *const clap_sys::ext::latency::clap_plugin_latency;
            (*part).get.map(|get| get(self.plugin) as usize).unwrap_or(0)
        }
    }

    fn state_part(&self) -> Option<*const clap_plugin_state> {
        unsafe {
            let get = (*self.plugin).get_extension?;
            let found = get(self.plugin, CLAP_EXT_STATE.as_ptr());
            (!found.is_null()).then_some(found as *const clap_plugin_state)
        }
    }

    pub fn save(&self) -> Result<Vec<u8>, String> {
        let Some(part) = self.state_part() else { return Ok(Vec::new()) };
        let mut held: Vec<u8> = Vec::new();
        unsafe {
            let Some(save) = (*part).save else { return Ok(Vec::new()) };
            let stream = clap_ostream { ctx: &mut held as *mut Vec<u8> as *mut c_void, write: Some(push_bytes) };
            if !save(self.plugin, &stream) {
                return Err("the plugin would not hand over its settings".into());
            }
        }
        Ok(held)
    }

    pub fn restore(&mut self, state: &[u8]) -> Result<(), String> {
        if state.is_empty() {
            return Ok(());
        }
        let Some(part) = self.state_part() else { return Ok(()) };
        let mut reading = Reading { held: state, at: 0 };
        unsafe {
            let Some(load) = (*part).load else { return Ok(()) };
            let stream = clap_istream { ctx: &mut reading as *mut Reading as *mut c_void, read: Some(pull_bytes) };
            if !load(self.plugin, &stream) {
                return Err("the plugin would not take those settings".into());
            }
        }
        Ok(())
    }
}

struct Reading<'a> {
    held: &'a [u8],
    at: usize,
}

unsafe extern "C" fn push_bytes(stream: *const clap_ostream, buffer: *const c_void, size: u64) -> i64 {
    let held = &mut *((*stream).ctx as *mut Vec<u8>);
    let count = size as usize;
    held.extend_from_slice(std::slice::from_raw_parts(buffer as *const u8, count));
    count as i64
}

unsafe extern "C" fn pull_bytes(stream: *const clap_istream, buffer: *mut c_void, size: u64) -> i64 {
    let reading = &mut *((*stream).ctx as *mut Reading);
    let left = reading.held.len() - reading.at;
    let take = left.min(size as usize);
    if take > 0 {
        std::ptr::copy_nonoverlapping(reading.held[reading.at..].as_ptr(), buffer as *mut u8, take);
        reading.at += take;
    }
    take as i64
}

impl Drop for Effect {
    fn drop(&mut self) {
        unsafe {
            if let Some(stop) = (*self.plugin).stop_processing {
                stop(self.plugin);
            }
            if let Some(sleep) = (*self.plugin).deactivate {
                sleep(self.plugin);
            }
            if let Some(gone) = (*self.plugin).destroy {
                gone(self.plugin);
            }
        }
        let _ = &self.host;
    }
}
