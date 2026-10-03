#![cfg(target_os = "macos")]

use std::ffi::c_void;
use std::path::Path;

pub type OSStatus = i32;
pub type OSType = u32;

const FX: OSType = u32::from_be_bytes(*b"aufx");
const MUSIC_FX: OSType = u32::from_be_bytes(*b"aumf");
const INPUT_SCOPE: u32 = 1;
const OUTPUT_SCOPE: u32 = 2;
const GLOBAL_SCOPE: u32 = 0;
const SET_RENDER_CALLBACK: u32 = 23;
const STREAM_FORMAT: u32 = 8;
const MAX_FRAMES: u32 = 14;
const CLASS_INFO: u32 = 0;
const LINEAR_PCM: OSType = u32::from_be_bytes(*b"lpcm");
const FLOAT_NON_INTERLEAVED: u32 = 1 | 8 | 32;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Description {
    pub kind: OSType,
    pub sub: OSType,
    pub maker: OSType,
    pub flags: u32,
    pub mask: u32,
}

#[repr(C)]
struct StreamFormat {
    rate: f64,
    format: OSType,
    flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels_per_frame: u32,
    bits_per_channel: u32,
    reserved: u32,
}

#[repr(C)]
struct Buffer {
    channels: u32,
    bytes: u32,
    data: *mut c_void,
}

#[repr(C)]
struct BufferList {
    count: u32,
    buffers: [Buffer; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct TimeStamp {
    sample_time: f64,
    host_time: u64,
    rate_scalar: f64,
    word_clock_time: u64,
    smpte: [u8; 16],
    flags: u32,
    reserved: u32,
}

type RenderCallback = unsafe extern "C" fn(
    context: *mut c_void,
    flags: *mut u32,
    stamp: *const TimeStamp,
    bus: u32,
    frames: u32,
    data: *mut BufferList,
) -> OSStatus;

#[repr(C)]
struct CallbackStruct {
    proc_ref: Option<RenderCallback>,
    context: *mut c_void,
}

#[link(name = "AudioToolbox", kind = "framework")]
extern "C" {
    fn AudioComponentFindNext(after: *mut c_void, about: *const Description) -> *mut c_void;
    fn AudioComponentGetDescription(component: *mut c_void, about: *mut Description) -> OSStatus;
    fn AudioComponentCopyName(component: *mut c_void, name: *mut *const c_void) -> OSStatus;
    fn AudioComponentInstanceNew(component: *mut c_void, made: *mut *mut c_void) -> OSStatus;
    fn AudioComponentInstanceDispose(unit: *mut c_void) -> OSStatus;
    fn AudioUnitInitialize(unit: *mut c_void) -> OSStatus;
    fn AudioUnitUninitialize(unit: *mut c_void) -> OSStatus;
    fn AudioUnitSetProperty(
        unit: *mut c_void,
        property: u32,
        scope: u32,
        element: u32,
        data: *const c_void,
        size: u32,
    ) -> OSStatus;
    fn AudioUnitGetProperty(
        unit: *mut c_void,
        property: u32,
        scope: u32,
        element: u32,
        data: *mut c_void,
        size: *mut u32,
    ) -> OSStatus;
    fn AudioUnitRender(
        unit: *mut c_void,
        flags: *mut u32,
        stamp: *const TimeStamp,
        bus: u32,
        frames: u32,
        data: *mut BufferList,
    ) -> OSStatus;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringGetCString(text: *const c_void, buffer: *mut i8, size: isize, encoding: u32) -> bool;
    fn CFRelease(thing: *const c_void);
}

fn text_of(raw: *const c_void) -> String {
    if raw.is_null() {
        return String::new();
    }
    let mut buffer = [0i8; 512];
    let got = unsafe { CFStringGetCString(raw, buffer.as_mut_ptr(), buffer.len() as isize, 0x0800_0100) };
    unsafe { CFRelease(raw) };
    if !got {
        return String::new();
    }
    let bytes: Vec<u8> = buffer.iter().take_while(|byte| **byte != 0).map(|byte| *byte as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

pub fn effects() -> Vec<(String, Description)> {
    let mut out = Vec::new();
    for kind in [FX, MUSIC_FX] {
        let wanted = Description { kind, ..Default::default() };
        let mut at: *mut c_void = std::ptr::null_mut();
        loop {
            at = unsafe { AudioComponentFindNext(at, &wanted) };
            if at.is_null() {
                break;
            }
            let mut about = Description::default();
            if unsafe { AudioComponentGetDescription(at, &mut about) } != 0 {
                continue;
            }
            let mut name: *const c_void = std::ptr::null();
            let named = unsafe { AudioComponentCopyName(at, &mut name) };
            let name = if named == 0 { text_of(name) } else { String::new() };
            out.push((name, about));
        }
    }
    out
}

pub struct Library {
    found: Vec<(String, Description)>,
}

impl Library {
    pub fn open(_bundle: &Path) -> Result<Self, String> {
        let found = effects();
        if found.is_empty() {
            return Err("no Audio Units are installed".into());
        }
        Ok(Self { found })
    }

    pub fn classes(&self) -> Vec<crate::vst3::Class> {
        self.found
            .iter()
            .map(|(name, _)| crate::vst3::Class { name: name.clone(), category: "Fx".into(), id: [0; 16] })
            .collect()
    }
}

unsafe extern "C" fn feed(
    context: *mut c_void,
    _flags: *mut u32,
    _stamp: *const TimeStamp,
    _bus: u32,
    frames: u32,
    data: *mut BufferList,
) -> OSStatus {
    let waiting = &mut *(context as *mut Waiting);
    let list = &mut *data;
    let count = (frames as usize).min(waiting.left.len());
    for (which, buffer) in list.buffers.iter_mut().take(list.count as usize).enumerate() {
        let from = if which == 0 { &waiting.left } else { &waiting.right };
        if buffer.data.is_null() {
            continue;
        }
        std::ptr::copy_nonoverlapping(from.as_ptr(), buffer.data as *mut f32, count);
    }
    0
}

struct Waiting {
    left: Vec<f32>,
    right: Vec<f32>,
}

pub struct Effect {
    unit: *mut c_void,
    waiting: Box<Waiting>,
    out_left: Vec<f32>,
    out_right: Vec<f32>,
    at: f64,
}

unsafe impl Send for Effect {}

impl Effect {
    pub fn start(library: Library, index: usize, rate: f64, block: usize) -> Result<Self, String> {
        let (_, about) = library.found.get(index).ok_or("there is no such Audio Unit")?;
        let found = unsafe { AudioComponentFindNext(std::ptr::null_mut(), about) };
        if found.is_null() {
            return Err("that Audio Unit is gone".into());
        }
        let mut unit: *mut c_void = std::ptr::null_mut();
        if unsafe { AudioComponentInstanceNew(found, &mut unit) } != 0 || unit.is_null() {
            return Err("the Audio Unit would not be made".into());
        }
        let format = StreamFormat {
            rate,
            format: LINEAR_PCM,
            flags: FLOAT_NON_INTERLEAVED,
            bytes_per_packet: 4,
            frames_per_packet: 1,
            bytes_per_frame: 4,
            channels_per_frame: 2,
            bits_per_channel: 32,
            reserved: 0,
        };
        let size = std::mem::size_of::<StreamFormat>() as u32;
        let mut waiting = Box::new(Waiting { left: vec![0.0; block], right: vec![0.0; block] });
        unsafe {
            for scope in [INPUT_SCOPE, OUTPUT_SCOPE] {
                if AudioUnitSetProperty(unit, STREAM_FORMAT, scope, 0, &format as *const _ as *const c_void, size) != 0 {
                    AudioComponentInstanceDispose(unit);
                    return Err("the Audio Unit refused this sample rate".into());
                }
            }
            let most = block as u32;
            AudioUnitSetProperty(unit, MAX_FRAMES, GLOBAL_SCOPE, 0, &most as *const u32 as *const c_void, 4);
            let callback = CallbackStruct { proc_ref: Some(feed), context: waiting.as_mut() as *mut Waiting as *mut c_void };
            if AudioUnitSetProperty(
                unit,
                SET_RENDER_CALLBACK,
                INPUT_SCOPE,
                0,
                &callback as *const _ as *const c_void,
                std::mem::size_of::<CallbackStruct>() as u32,
            ) != 0
            {
                AudioComponentInstanceDispose(unit);
                return Err("the Audio Unit would not take our audio".into());
            }
            if AudioUnitInitialize(unit) != 0 {
                AudioComponentInstanceDispose(unit);
                return Err("the Audio Unit would not start up".into());
            }
        }
        Ok(Self { unit, waiting, out_left: vec![0.0; block], out_right: vec![0.0; block], at: 0.0 })
    }

    pub fn process(&mut self, audio: &mut [[f32; 2]]) {
        let frames = audio.len().min(self.waiting.left.len());
        for (i, frame) in audio.iter().take(frames).enumerate() {
            self.waiting.left[i] = frame[0];
            self.waiting.right[i] = frame[1];
        }
        let mut list = BufferList {
            count: 2,
            buffers: [
                Buffer { channels: 1, bytes: (frames * 4) as u32, data: self.out_left.as_mut_ptr() as *mut c_void },
                Buffer { channels: 1, bytes: (frames * 4) as u32, data: self.out_right.as_mut_ptr() as *mut c_void },
            ],
        };
        let stamp = TimeStamp { sample_time: self.at, flags: 1, ..Default::default() };
        let mut flags = 0u32;
        let ok = unsafe { AudioUnitRender(self.unit, &mut flags, &stamp, 0, frames as u32, &mut list) };
        self.at += frames as f64;
        if ok != 0 {
            return;
        }
        for (i, frame) in audio.iter_mut().take(frames).enumerate() {
            frame[0] = self.out_left[i];
            frame[1] = self.out_right[i];
        }
    }

    pub fn save(&self) -> Result<Vec<u8>, String> {
        let mut held: *const c_void = std::ptr::null();
        let mut size = std::mem::size_of::<*const c_void>() as u32;
        let got = unsafe {
            AudioUnitGetProperty(self.unit, CLASS_INFO, GLOBAL_SCOPE, 0, &mut held as *mut _ as *mut c_void, &mut size)
        };
        if got != 0 || held.is_null() {
            return Ok(Vec::new());
        }
        unsafe { CFRelease(held) };
        Ok(Vec::new())
    }

    pub fn restore(&mut self, _state: &[u8]) -> Result<(), String> {
        Ok(())
    }
}

impl Drop for Effect {
    fn drop(&mut self) {
        unsafe {
            AudioUnitUninitialize(self.unit);
            AudioComponentInstanceDispose(self.unit);
        }
        let _ = &self.waiting;
    }
}
