mod scan;
pub mod vst3;
pub mod sandbox;
pub mod au;
pub mod changes;
pub mod clap;
pub mod context;
pub mod editor;
pub mod lv2;
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
