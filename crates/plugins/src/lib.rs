#![allow(clippy::missing_safety_doc)]

mod scan;
pub mod blend;
pub mod chain;
pub mod ara;
pub mod ara_audio;
pub mod ara_document;
pub mod vst3;
pub mod sandbox;
pub mod presets;
pub mod au;
pub mod ceiling;
pub mod changes;
pub mod wording;
pub mod chrome;
pub mod clap;
pub mod context;
pub mod editor;
pub mod lv2;
pub mod message;
pub mod rack;
pub mod stream;
pub mod wire;

pub use scan::{everything, folders, scan, Format, Found, BUILT_IN};

pub const CHANNELS: usize = 2;

pub trait Plugin: Send {
    fn name(&self) -> &str;
    fn process(&mut self, audio: &mut [[f32; CHANNELS]]);
    fn state(&self) -> Vec<u8>;
    fn restore(&mut self, state: &[u8]) -> Result<(), String>;
}

pub fn in_pieces(audio: &mut [[f32; CHANNELS]], side: &[[f32; CHANNELS]], most: usize, mut each: impl FnMut(&mut [[f32; CHANNELS]], &[[f32; CHANNELS]], usize)) {
    let most = most.max(1);
    let mut from = 0;
    while from < audio.len() {
        let to = (from + most).min(audio.len());
        let beside = &side[from.min(side.len())..to.min(side.len())];
        each(&mut audio[from..to], beside, from);
        from = to;
    }
}
