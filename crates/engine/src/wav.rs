use std::fs::{self, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

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

pub fn write_frames(path: &Path, frames: &[[f32; 2]], rate: u32) -> io::Result<()> {
    let mut out = io::BufWriter::new(fs::File::create(path)?);
    out.write_all(&float_header(2, rate, frames.len() as u32))?;
    for frame in frames {
        out.write_all(&frame[0].to_le_bytes())?;
        out.write_all(&frame[1].to_le_bytes())?;
    }
    out.flush()
}

fn word(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn long(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

pub fn repair(path: &Path) -> io::Result<bool> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let mut head = [0u8; HEADER_BYTES as usize];
    if file.read_exact(&mut head).is_err() {
        return Ok(false);
    }
    let ours = &head[0..4] == b"RIFF" && &head[8..16] == b"WAVEfmt " && word(&head, 20) == IEEE_FLOAT && &head[38..42] == b"fact" && &head[50..54] == b"data";
    let channels = word(&head, 22);
    if !ours || channels == 0 {
        return Ok(false);
    }
    let block = channels as u64 * BYTES_PER_SAMPLE as u64;
    let frames = (file.metadata()?.len().saturating_sub(HEADER_BYTES as u64) / block).min(most_frames(channels));
    if long(&head, 54) as u64 == frames * block && long(&head, 46) as u64 == frames {
        return Ok(false);
    }
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&float_header(channels, long(&head, 24), frames as u32))?;
    file.set_len(HEADER_BYTES as u64 + frames * block)?;
    file.flush()?;
    Ok(true)
}

pub fn repair_takes(folder: &Path) -> usize {
    fs::read_dir(folder)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("wav")))
        .filter(|path| repair(path).unwrap_or(false))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_take_cut_off_by_a_crash_is_given_its_real_length() {
        let folder = std::env::temp_dir().join(format!("loupe-repair-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        let take = folder.join("Vocal (take 1).wav");
        let mut bytes = float_header(1, 48_000, 0);
        for i in 0..1000u32 {
            bytes.extend_from_slice(&(i as f32 / 1000.0).to_le_bytes());
        }
        bytes.extend_from_slice(&[1, 2]);
        fs::write(&take, &bytes).unwrap();
        fs::write(folder.join("notes.wav"), b"not a wave").unwrap();
        assert_eq!(repair_takes(&folder), 1);
        let fixed = fs::read(&take).unwrap();
        assert_eq!(&fixed[..HEADER_BYTES as usize], &float_header(1, 48_000, 1000)[..]);
        assert_eq!(fixed.len(), HEADER_BYTES as usize + 4000);
        assert_eq!(repair_takes(&folder), 0, "a sound file is left alone");
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_stereo_take_cut_off_mid_frame_keeps_its_whole_frames() {
        let folder = std::env::temp_dir().join(format!("loupe-repair-stereo-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        let take = folder.join("Keys (take 1).wav");
        let mut bytes = float_header(2, 44_100, 0);
        for i in 0..1001u32 {
            bytes.extend_from_slice(&(i as f32).to_le_bytes());
        }
        fs::write(&take, &bytes).unwrap();
        assert_eq!(repair_takes(&folder), 1);
        let fixed = fs::read(&take).unwrap();
        assert_eq!(&fixed[..HEADER_BYTES as usize], &float_header(2, 44_100, 500)[..]);
        assert_eq!(fixed.len(), HEADER_BYTES as usize + 500 * 8);
        fs::remove_dir_all(folder).unwrap();
    }

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
