use std::os::raw::{c_double, c_float, c_int, c_uint, c_void};

const PROCESS_OFFLINE: c_int = 0x0000_0000;
const PITCH_HIGH_QUALITY: c_int = 0x0200_0000;
const CHANNELS_TOGETHER: c_int = 0x1000_0000;
const ENGINE_FINER: c_int = 0x2000_0000;
const BLOCK: usize = 8192;
pub const SHORTEST: f64 = 0.05;
pub const LONGEST: f64 = 20.0;

extern "C" {
    fn rubberband_new(rate: c_uint, channels: c_uint, options: c_int, time_ratio: c_double, pitch_scale: c_double) -> *mut c_void;
    fn rubberband_delete(state: *mut c_void);
    fn rubberband_set_expected_input_duration(state: *mut c_void, samples: c_uint);
    fn rubberband_set_max_process_size(state: *mut c_void, samples: c_uint);
    fn rubberband_study(state: *mut c_void, input: *const *const c_float, samples: c_uint, last: c_int);
    fn rubberband_process(state: *mut c_void, input: *const *const c_float, samples: c_uint, last: c_int);
    fn rubberband_available(state: *const c_void) -> c_int;
    fn rubberband_retrieve(state: *const c_void, output: *const *mut c_float, samples: c_uint) -> c_uint;
}

struct Stretcher(*mut c_void);

impl Drop for Stretcher {
    fn drop(&mut self) {
        unsafe { rubberband_delete(self.0) }
    }
}

pub fn stretched_len(len: usize, ratio: f64) -> usize {
    (len as f64 * ratio).round() as usize
}

pub fn stretch(frames: &[[f32; 2]], rate: u32, ratio: f64) -> Vec<[f32; 2]> {
    let wanted = stretched_len(frames.len(), ratio);
    if frames.is_empty() || !ratio.is_finite() || (ratio - 1.0).abs() < 1e-9 {
        return frames.to_vec();
    }
    let ratio = ratio.clamp(SHORTEST, LONGEST);
    let left: Vec<f32> = frames.iter().map(|f| f[0]).collect();
    let right: Vec<f32> = frames.iter().map(|f| f[1]).collect();
    let options = PROCESS_OFFLINE | ENGINE_FINER | CHANNELS_TOGETHER | PITCH_HIGH_QUALITY;
    let stretcher = Stretcher(unsafe { rubberband_new(rate, 2, options, ratio, 1.0) });
    let mut out_left = Vec::with_capacity(wanted + BLOCK);
    let mut out_right = Vec::with_capacity(wanted + BLOCK);
    let mut buffers = [vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]];
    unsafe {
        rubberband_set_expected_input_duration(stretcher.0, frames.len() as c_uint);
        rubberband_set_max_process_size(stretcher.0, BLOCK as c_uint);
        let mut at = 0;
        while at < frames.len() {
            let end = (at + BLOCK).min(frames.len());
            let input = [left[at..].as_ptr(), right[at..].as_ptr()];
            rubberband_study(stretcher.0, input.as_ptr(), (end - at) as c_uint, (end == frames.len()) as c_int);
            at = end;
        }
        let mut drain = |out_left: &mut Vec<f32>, out_right: &mut Vec<f32>| loop {
            let ready = rubberband_available(stretcher.0);
            if ready <= 0 {
                return ready;
            }
            let take = (ready as usize).min(BLOCK);
            let [a, b] = &mut buffers;
            let output = [a.as_mut_ptr(), b.as_mut_ptr()];
            let got = rubberband_retrieve(stretcher.0, output.as_ptr(), take as c_uint) as usize;
            out_left.extend_from_slice(&a[..got]);
            out_right.extend_from_slice(&b[..got]);
        };
        let mut at = 0;
        while at < frames.len() {
            let end = (at + BLOCK).min(frames.len());
            let input = [left[at..].as_ptr(), right[at..].as_ptr()];
            rubberband_process(stretcher.0, input.as_ptr(), (end - at) as c_uint, (end == frames.len()) as c_int);
            drain(&mut out_left, &mut out_right);
            at = end;
        }
        for _ in 0..1000 {
            if drain(&mut out_left, &mut out_right) < 0 {
                break;
            }
            std::thread::yield_now();
        }
    }
    out_left.resize(wanted, 0.0);
    out_right.resize(wanted, 0.0);
    out_left.into_iter().zip(out_right).map(|(l, r)| [l, r]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn tone(hertz: f32, seconds: f32) -> Vec<[f32; 2]> {
        (0..(RATE as f32 * seconds) as usize)
            .map(|i| {
                let v = (std::f32::consts::TAU * hertz * i as f32 / RATE as f32).sin() * 0.5;
                [v, v]
            })
            .collect()
    }

    fn crossings(frames: &[[f32; 2]]) -> usize {
        frames.windows(2).filter(|pair| pair[0][0] < 0.0 && pair[1][0] >= 0.0).count()
    }

    #[test]
    fn longer_and_shorter_keep_the_pitch() {
        let input = tone(440.0, 1.0);
        for ratio in [2.0, 0.5, 1.37] {
            let out = stretch(&input, RATE, ratio);
            assert_eq!(out.len(), stretched_len(input.len(), ratio));
            let middle = &out[out.len() / 4..out.len() * 3 / 4];
            let hertz = crossings(middle) as f64 / (middle.len() as f64 / RATE as f64);
            assert!((hertz - 440.0).abs() < 6.0, "ratio {ratio} sounds at {hertz} Hz");
            let loud = middle.iter().map(|f| f[0].abs()).fold(0.0, f32::max);
            assert!(loud > 0.3 && loud < 0.8, "ratio {ratio} level {loud}");
        }
    }

    #[test]
    fn no_stretch_and_silence_come_back_untouched() {
        let input = tone(220.0, 0.2);
        assert_eq!(stretch(&input, RATE, 1.0), input);
        assert!(stretch(&[], RATE, 2.0).is_empty());
    }
}
