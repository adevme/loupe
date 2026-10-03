use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::model::{Frames, Project, Track};
use crate::render::render;

const BLOCK: usize = 16_384;
const CHANNELS: u16 = 2;
const BYTES_PER_SAMPLE: u16 = 4;
const IEEE_FLOAT: u16 = 3;
const STEMS_FOLDER: &str = "Stems";
const NOT_IN_FILE_NAMES: [char; 9] = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

pub struct ExportPlan {
    pub folder: PathBuf,
    pub name: String,
    pub split: bool,
    pub range: Option<(Frames, Frames)>,
    pub project_file: String,
}

pub fn export(project: &Project, plan: &ExportPlan, progress: &dyn Fn(f32)) -> Result<(), String> {
    let (from, to) = plan.range.unwrap_or((0, project.length()));
    if to <= from {
        return Err("there is nothing to export".into());
    }
    let failed = |what: &Path, why: io::Error| format!("{}: {why}", what.display());
    fs::create_dir_all(&plan.folder).map_err(|why| failed(&plan.folder, why))?;

    let stems: Vec<&Track> =
        if plan.split { project.tracks.iter().filter(|track| !track.muted).collect() } else { Vec::new() };
    let all_frames = (to - from) * (1 + stems.len() as Frames);
    let mut written = 0;
    let mut count = |frames: Frames| {
        written += frames;
        progress(written as f32 / all_frames as f32);
    };

    let mix = plan.folder.join(format!("{}.wav", plan.name));
    write_wav(&mix, project, from, to, &mut count).map_err(|why| failed(&mix, why))?;
    let copy = plan.folder.join(format!("{}.lp", plan.name));
    fs::write(&copy, &plan.project_file).map_err(|why| failed(&copy, why))?;

    if plan.split {
        let stems_folder = plan.folder.join(STEMS_FOLDER);
        fs::create_dir_all(&stems_folder).map_err(|why| failed(&stems_folder, why))?;
        let mut used: Vec<String> = Vec::new();
        for track in stems {
            let mut alone = project.clone();
            alone.tracks.retain(|other| other.id == track.id);
            alone.master = 1.0;
            let file = stems_folder.join(format!("{}.wav", unused_name(&track.name, &mut used)));
            write_wav(&file, &alone, from, to, &mut count).map_err(|why| failed(&file, why))?;
        }
    }
    Ok(())
}

pub fn next_version_folder(exports: &Path) -> PathBuf {
    let taken = fs::read_dir(exports)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().to_string_lossy().strip_prefix('V')?.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    exports.join(format!("V{}", taken + 1))
}

fn unused_name(wanted: &str, used: &mut Vec<String>) -> String {
    let safe: String = wanted.trim().chars().filter(|c| !NOT_IN_FILE_NAMES.contains(c)).collect();
    let base = if safe.is_empty() { "Track".to_string() } else { safe };
    let mut name = base.clone();
    let mut copy = 2;
    while used.iter().any(|taken| taken.eq_ignore_ascii_case(&name)) {
        name = format!("{base} {copy}");
        copy += 1;
    }
    used.push(name.clone());
    name
}

fn write_wav(
    path: &Path,
    project: &Project,
    from: Frames,
    to: Frames,
    wrote: &mut dyn FnMut(Frames),
) -> io::Result<()> {
    let frames = to - from;
    let block_align = CHANNELS * BYTES_PER_SAMPLE;
    let data_bytes = u32::try_from(frames * block_align as u64)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "the song is too long for one WAV file"))?;
    let mut out = BufWriter::new(File::create(path)?);
    out.write_all(b"RIFF")?;
    out.write_all(&(50 + data_bytes).to_le_bytes())?;
    out.write_all(b"WAVEfmt ")?;
    out.write_all(&18u32.to_le_bytes())?;
    out.write_all(&IEEE_FLOAT.to_le_bytes())?;
    out.write_all(&CHANNELS.to_le_bytes())?;
    out.write_all(&project.rate.to_le_bytes())?;
    out.write_all(&(project.rate * block_align as u32).to_le_bytes())?;
    out.write_all(&block_align.to_le_bytes())?;
    out.write_all(&(BYTES_PER_SAMPLE * 8).to_le_bytes())?;
    out.write_all(&0u16.to_le_bytes())?;
    out.write_all(b"fact")?;
    out.write_all(&4u32.to_le_bytes())?;
    out.write_all(&(frames as u32).to_le_bytes())?;
    out.write_all(b"data")?;
    out.write_all(&data_bytes.to_le_bytes())?;

    let mut block = vec![[0.0f32; 2]; BLOCK];
    let mut pos = from;
    while pos < to {
        let count = BLOCK.min((to - pos) as usize);
        render(project, pos, &mut block[..count]);
        for frame in &block[..count] {
            out.write_all(&frame[0].to_le_bytes())?;
            out.write_all(&frame[1].to_le_bytes())?;
        }
        pos += count as Frames;
        wrote(count as Frames);
    }
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Command, Edge, Fade, Outcome};
    use crate::source::Source;
    use std::sync::Arc;

    fn song() -> Project {
        let mut p = Project::new(48_000);
        for (name, level) in [("Lead: vox", 0.25f32), ("Beat", 0.5), ("Lead: vox", 0.125)] {
            let Ok(Outcome::Track(track)) = p.apply(Command::AddTrack { name: name.into() }) else {
                panic!("no track")
            };
            let source = Arc::new(Source::from_frames(name, vec![[level, -level]; 40_000]));
            let Ok(Outcome::Clip(clip)) = p.apply(Command::AddClip { track, source, start: 1000 }) else {
                panic!("no clip")
            };
            p.apply(Command::SetClipFade { clip, edge: Edge::In, fade: Fade { len: 500, curve: 0.0 } }).unwrap();
        }
        p.apply(Command::SetTrackMuted { track: p.tracks[2].id, muted: true }).unwrap();
        p.apply(Command::SetMasterGain(0.5)).unwrap();
        p
    }

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("loupe-export-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        folder
    }

    fn heard(project: &Project) -> Vec<[f32; 2]> {
        let mut out = vec![[0.0; 2]; project.length() as usize];
        render(project, 0, &mut out);
        out
    }

    #[test]
    fn the_exported_mix_is_exactly_what_plays() {
        let project = song();
        let folder = scratch("mix");
        let plan = ExportPlan {
            folder: folder.clone(),
            name: "Song".into(),
            split: false,
            range: None,
            project_file: "saved".into(),
        };
        export(&project, &plan, &|_| {}).unwrap();
        let read = Source::load(&folder.join("Song.wav"), 48_000).unwrap();
        assert_eq!(read.frames, heard(&project));
        assert_eq!(fs::read_to_string(folder.join("Song.lp")).unwrap(), "saved");
        assert!(!folder.join(STEMS_FOLDER).exists());
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn stems_line_up_from_the_start_and_add_up_to_the_mix_before_the_master() {
        let project = song();
        let folder = scratch("split");
        let plan =
            ExportPlan { folder: folder.clone(), name: "Song".into(), split: true, range: None, project_file: String::new() };
        export(&project, &plan, &|_| {}).unwrap();
        let stems = folder.join(STEMS_FOLDER);
        let vox = Source::load(&stems.join("Lead vox.wav"), 48_000).unwrap();
        let beat = Source::load(&stems.join("Beat.wav"), 48_000).unwrap();
        assert!(!stems.join("Lead vox 2.wav").exists(), "a muted track is left out");
        assert_eq!(vox.frames.len() as u64, project.length());
        assert_eq!(beat.frames.len() as u64, project.length());
        assert_eq!(vox.frames[999], [0.0, 0.0]);
        assert_eq!(vox.frames[1500], [0.25, -0.25]);
        let mut unmastered = project.clone();
        unmastered.apply(Command::SetMasterGain(1.0)).unwrap();
        let summed: Vec<[f32; 2]> =
            vox.frames.iter().zip(&beat.frames).map(|(a, b)| [a[0] + b[0], a[1] + b[1]]).collect();
        assert_eq!(summed, heard(&unmastered));
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn progress_only_climbs_and_ends_complete() {
        let project = song();
        let folder = scratch("progress");
        let plan =
            ExportPlan { folder: folder.clone(), name: "Song".into(), split: true, range: None, project_file: String::new() };
        let seen = std::cell::RefCell::new(Vec::new());
        export(&project, &plan, &|fraction| seen.borrow_mut().push(fraction)).unwrap();
        let seen = seen.into_inner();
        assert!(seen.len() >= 3, "one report per file at least");
        assert!(seen.windows(2).all(|pair| pair[1] >= pair[0]));
        assert_eq!(seen.last(), Some(&1.0));
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_range_exports_only_that_part() {
        let project = song();
        let folder = scratch("range");
        let plan = ExportPlan {
            folder: folder.clone(),
            name: "Song".into(),
            split: false,
            range: Some((1500, 2500)),
            project_file: String::new(),
        };
        export(&project, &plan, &|_| {}).unwrap();
        let read = Source::load(&folder.join("Song.wav"), 48_000).unwrap();
        assert_eq!(read.frames, heard(&project)[1500..2500]);
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn each_export_gets_the_next_version_folder() {
        let exports = scratch("versions");
        assert_eq!(next_version_folder(&exports), exports.join("V1"));
        fs::create_dir_all(exports.join("V1")).unwrap();
        fs::create_dir_all(exports.join("V7")).unwrap();
        fs::create_dir_all(exports.join("Vocals")).unwrap();
        assert_eq!(next_version_folder(&exports), exports.join("V8"));
        fs::remove_dir_all(exports).unwrap();
    }
}
