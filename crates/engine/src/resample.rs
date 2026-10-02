use std::f64::consts::PI;

const ZEROS: usize = 16;
const RESOLUTION: usize = 512;
const ROLLOFF: f64 = 0.95;
const KAISER_BETA: f64 = 9.0;

pub fn resample(input: &[[f32; 2]], from: u32, to: u32) -> Vec<[f32; 2]> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = to as f64 / from as f64;
    let cutoff = ratio.min(1.0) * ROLLOFF;
    let table = kernel_table();
    let half_width = ZEROS as f64 / cutoff;
    let len = (input.len() as u64 * to as u64).div_ceil(from as u64) as usize;
    let mut output = vec![[0.0f32; 2]; len];

    let workers = std::thread::available_parallelism().map_or(1, |n| n.get());
    let chunk = len.div_ceil(workers).max(1);
    std::thread::scope(|scope| {
        for (c, out) in output.chunks_mut(chunk).enumerate() {
            let table = &table;
            scope.spawn(move || {
                for (n, frame) in out.iter_mut().enumerate() {
                    let centre = (c * chunk + n) as f64 / ratio;
                    let first = (centre - half_width).ceil().max(0.0) as usize;
                    let last = ((centre + half_width).floor() as usize).min(input.len() - 1);
                    let (mut left, mut right) = (0.0f64, 0.0f64);
                    for (i, sample) in input.iter().enumerate().take(last + 1).skip(first) {
                        let at = ((centre - i as f64) * cutoff).abs() * RESOLUTION as f64;
                        let slot = at as usize;
                        let blend = at - slot as f64;
                        let weight = table[slot] + (table[slot + 1] - table[slot]) * blend;
                        left += sample[0] as f64 * weight;
                        right += sample[1] as f64 * weight;
                    }
                    *frame = [(left * cutoff) as f32, (right * cutoff) as f32];
                }
            });
        }
    });
    output
}

fn kernel_table() -> Vec<f64> {
    let size = ZEROS * RESOLUTION;
    let mut table: Vec<f64> = (0..=size)
        .map(|i| {
            let x = i as f64 / RESOLUTION as f64;
            let sinc = if i == 0 { 1.0 } else { (PI * x).sin() / (PI * x) };
            let edge = x / ZEROS as f64;
            sinc * bessel_i0(KAISER_BETA * (1.0 - edge * edge).max(0.0).sqrt()) / bessel_i0(KAISER_BETA)
        })
        .collect();
    table.push(0.0);
    table
}

fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term) = (1.0, 1.0);
    for k in 1..40 {
        term *= (x / (2.0 * k as f64)).powi(2);
        sum += term;
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, hz: f64, frames: usize) -> Vec<[f32; 2]> {
        (0..frames)
            .map(|i| {
                let s = (i as f64 * hz * 2.0 * PI / rate as f64).sin() as f32 * 0.8;
                [s, -s]
            })
            .collect()
    }

    fn worst_error(from: u32, to: u32, hz: f64) -> f32 {
        let output = resample(&sine(from, hz, from as usize), from, to);
        assert_eq!(output.len(), to as usize);
        let ideal = sine(to, hz, to as usize);
        let edge = to as usize / 100;
        output[edge..output.len() - edge]
            .iter()
            .zip(&ideal[edge..])
            .map(|(a, b)| (a[0] - b[0]).abs().max((a[1] - b[1]).abs()))
            .fold(0.0, f32::max)
    }

    #[test]
    fn a_tone_survives_conversion_up_and_down() {
        assert!(worst_error(44_100, 48_000, 1000.0) < 1e-3);
        assert!(worst_error(48_000, 44_100, 1000.0) < 1e-3);
        assert!(worst_error(44_100, 48_000, 15_000.0) < 1e-3);
        assert!(worst_error(96_000, 48_000, 10_000.0) < 1e-3);
    }

    #[test]
    fn the_same_rate_is_left_untouched() {
        let input = sine(48_000, 440.0, 1000);
        assert_eq!(resample(&input, 48_000, 48_000), input);
    }
}
