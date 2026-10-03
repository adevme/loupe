pub const HEADER_BYTES: u32 = 58;
const BYTES_PER_SAMPLE: u16 = 4;
const IEEE_FLOAT: u16 = 3;

pub fn most_frames(channels: u16) -> u64 {
    ((u32::MAX - HEADER_BYTES) / (channels * BYTES_PER_SAMPLE) as u32) as u64
}

pub fn float_header(channels: u16, rate: u32, frames: u32) -> Vec<u8> {
    let block_align = channels * BYTES_PER_SAMPLE;
    let data_bytes = frames * block_align as u32;
    let mut header = Vec::with_capacity(HEADER_BYTES as usize);
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&(HEADER_BYTES - 8 + data_bytes).to_le_bytes());
    header.extend_from_slice(b"WAVEfmt ");
    header.extend_from_slice(&18u32.to_le_bytes());
    header.extend_from_slice(&IEEE_FLOAT.to_le_bytes());
    header.extend_from_slice(&channels.to_le_bytes());
    header.extend_from_slice(&rate.to_le_bytes());
    header.extend_from_slice(&(rate * block_align as u32).to_le_bytes());
    header.extend_from_slice(&block_align.to_le_bytes());
    header.extend_from_slice(&(BYTES_PER_SAMPLE * 8).to_le_bytes());
    header.extend_from_slice(&0u16.to_le_bytes());
    header.extend_from_slice(b"fact");
    header.extend_from_slice(&4u32.to_le_bytes());
    header.extend_from_slice(&frames.to_le_bytes());
    header.extend_from_slice(b"data");
    header.extend_from_slice(&data_bytes.to_le_bytes());
    header
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_is_as_long_as_it_says() {
        assert_eq!(float_header(2, 48_000, 10).len() as u32, HEADER_BYTES);
    }

    #[test]
    fn the_longest_file_still_fits_its_size_field() {
        let frames = most_frames(2);
        assert!(frames * 8 + HEADER_BYTES as u64 <= u32::MAX as u64);
    }
}
