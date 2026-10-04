use std::os::raw::{c_float, c_int, c_uchar, c_void};

const JOINT_STEREO: c_int = 1;
const CONSTANT_BITRATE: c_int = 0;
const NEAR_BEST: c_int = 2;
const SPARE_BYTES: usize = 7200;
const LONGEST_TAG: usize = 2880;
const CHANNELS: usize = 2;

extern "C" {
    fn lame_init() -> *mut c_void;
    fn lame_close(state: *mut c_void) -> c_int;
    fn lame_set_num_channels(state: *mut c_void, channels: c_int) -> c_int;
    fn lame_set_in_samplerate(state: *mut c_void, rate: c_int) -> c_int;
    fn lame_set_out_samplerate(state: *mut c_void, rate: c_int) -> c_int;
    fn lame_set_mode(state: *mut c_void, mode: c_int) -> c_int;
    fn lame_set_VBR(state: *mut c_void, mode: c_int) -> c_int;
    fn lame_set_brate(state: *mut c_void, kbps: c_int) -> c_int;
    fn lame_set_quality(state: *mut c_void, quality: c_int) -> c_int;
    fn lame_set_bWriteVbrTag(state: *mut c_void, write: c_int) -> c_int;
    fn lame_init_params(state: *mut c_void) -> c_int;
    fn lame_encode_buffer_interleaved_ieee_float(state: *mut c_void, pcm: *const c_float, frames: c_int, out: *mut c_uchar, room: c_int) -> c_int;
    fn lame_encode_flush(state: *mut c_void, out: *mut c_uchar, room: c_int) -> c_int;
    fn lame_get_lametag_frame(state: *const c_void, out: *mut c_uchar, room: usize) -> usize;
}

pub struct Encoder(*mut c_void);

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe {
            lame_close(self.0);
        }
    }
}

fn refused(code: c_int) -> String {
    format!("the MP3 encoder refused the audio (LAME error {code})")
}

impl Encoder {
    pub fn constant(rate_in: u32, rate_out: u32, kbps: u32) -> Result<Self, String> {
        let state = unsafe { lame_init() };
        if state.is_null() {
            return Err("the MP3 encoder could not start".into());
        }
        let encoder = Self(state);
        let settings = unsafe {
            [
                lame_set_num_channels(state, CHANNELS as c_int),
                lame_set_in_samplerate(state, rate_in as c_int),
                lame_set_out_samplerate(state, rate_out as c_int),
                lame_set_mode(state, JOINT_STEREO),
                lame_set_VBR(state, CONSTANT_BITRATE),
                lame_set_brate(state, kbps as c_int),
                lame_set_quality(state, NEAR_BEST),
                lame_set_bWriteVbrTag(state, 1),
                lame_init_params(state),
            ]
        };
        match settings.into_iter().find(|code| *code < 0) {
            Some(code) => Err(format!("the MP3 encoder would not take these settings (LAME error {code})")),
            None => Ok(encoder),
        }
    }

    pub fn encode(&mut self, interleaved: &[f32], out: &mut Vec<u8>) -> Result<(), String> {
        let frames = interleaved.len() / CHANNELS;
        let room = frames * 5 / 4 + SPARE_BYTES;
        let start = out.len();
        out.resize(start + room, 0);
        let wrote = unsafe {
            lame_encode_buffer_interleaved_ieee_float(self.0, interleaved.as_ptr(), frames as c_int, out[start..].as_mut_ptr(), room as c_int)
        };
        out.truncate(start + wrote.max(0) as usize);
        if wrote < 0 {
            return Err(refused(wrote));
        }
        Ok(())
    }

    pub fn flush(&mut self, out: &mut Vec<u8>) -> Result<(), String> {
        let start = out.len();
        out.resize(start + SPARE_BYTES, 0);
        let wrote = unsafe { lame_encode_flush(self.0, out[start..].as_mut_ptr(), SPARE_BYTES as c_int) };
        out.truncate(start + wrote.max(0) as usize);
        if wrote < 0 {
            return Err(refused(wrote));
        }
        Ok(())
    }

    pub fn tag(&self) -> Vec<u8> {
        let mut tag = vec![0u8; LONGEST_TAG];
        let size = unsafe { lame_get_lametag_frame(self.0, tag.as_mut_ptr(), tag.len()) };
        tag.truncate(if size <= LONGEST_TAG { size } else { 0 });
        tag
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_of_tone_encodes_to_frames_of_the_right_size() {
        let mut encoder = Encoder::constant(48_000, 48_000, 320).unwrap();
        let tone: Vec<f32> = (0..48_000).flat_map(|i| {
            let s = 0.5 * (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin();
            [s, s]
        }).collect();
        let mut out = Vec::new();
        encoder.encode(&tone, &mut out).unwrap();
        encoder.flush(&mut out).unwrap();
        assert_eq!(&out[..2], &[0xFF, 0xFB]);
        assert!((38_000..44_000).contains(&out.len()), "{} bytes", out.len());
        assert_eq!(encoder.tag().len(), 960);
    }
}
