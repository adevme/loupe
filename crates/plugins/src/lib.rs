mod scan;
pub mod vst3;
pub mod sandbox;
pub mod stream;
pub mod rack;
pub mod clap;
pub mod wire;

pub use scan::{folders, scan, Format, Found};

pub const CHANNELS: usize = 2;

pub trait Plugin: Send {
    fn name(&self) -> &str;
    fn process(&mut self, audio: &mut [[f32; CHANNELS]]);
    fn state(&self) -> Vec<u8>;
    fn restore(&mut self, state: &[u8]) -> Result<(), String>;
}
