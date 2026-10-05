use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::Instant;

use loupe_stock::{Effect, Meter, SILENT_LUFS};

use crate::encode::{Format, Writer};
use crate::model::{Frames, Project, Track, TrackId};
use crate::wav;

const BLOCK: usize = 16_384;
const STEMS_FOLDER: &str = "Stems";
const NOT_IN_FILE_NAMES: [char; 9] = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];
const MEASURING: &str = "measuring.wav";
const METER_STEPS_PER_SECOND: u32 = 10;
pub const TRUE_PEAK_CEILING: f32 = -1.0;
const LOWEST_TARGET: f32 = -60.0;
const MOST_HELD_FRAMES: Frames = 1 << 26;
const METER_NANOSECONDS_A_FRAME: f64 = 18.0;
const READING_BACK_NANOSECONDS_A_FRAME: f64 = 3.0;
const RENDER_NANOSECONDS_A_UNIT: f64 = 1.0;
const PLUGIN_SLOT_UNITS: f32 = 8.0;
const FRAMES_BEFORE_TIMING_THE_RENDER: Frames = BLOCK as Frames * 4;

pub struct ExportPlan {
    pub folder: PathBuf,
    pub name: String,
    pub split: bool,
    pub range: Option<(Frames, Frames)>,
    pub project_file: String,
    pub format: Format,
    pub dither: bool,
    pub normalise: Normalise,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Normalise {
    Off,
    Peak(f32),
    Loudness(f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Levels {
    pub loudness: f32,
    pub true_peak: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gain {
    pub decibels: f32,
    pub note: Option<String>,
}

impl Gain {
    fn unchanged(note: Option<&str>) -> Self {
        Self { decibels: 0.0, note: note.map(str::to_string) }
    }

    fn linear(&self) -> f32 {
        10f32.powf(self.decibels / 20.0)
    }
}

fn signed(decibels: f32) -> String {
    format!("{decibels:+.1} dB")
}

impl Normalise {
    pub fn key(self) -> String {
        match self {
            Normalise::Off => "off".to_string(),
            Normalise::Peak(decibels) => format!("peak {decibels}"),
            Normalise::Loudness(lufs) => format!("lufs {lufs}"),
        }
    }

    pub fn from_key(text: &str) -> Option<Self> {
        let text = text.trim();
        if text == "off" {
            return Some(Normalise::Off);
        }
        let (kind, number) = text.split_once(' ')?;
        let target = number.trim().parse::<f32>().ok().filter(|target| (LOWEST_TARGET..=0.0).contains(target))?;
        match kind {
            "peak" => Some(Normalise::Peak(target)),
            "lufs" => Some(Normalise::Loudness(target)),
            _ => None,
        }
    }

    pub fn gain(self, levels: Levels) -> Gain {
        if self == Normalise::Off {
            return Gain::unchanged(None);
        }
        if levels.true_peak <= SILENT_LUFS {
            return Gain::unchanged(Some("Not normalised: the export is silent."));
        }
        match self {
            Normalise::Off => Gain::unchanged(None),
            Normalise::Peak(target) => {
                let decibels = target - levels.true_peak;
                Gain { decibels, note: Some(format!("Normalised to a true peak of {target} dBTP ({}).", signed(decibels))) }
            }
            Normalise::Loudness(target) => {
                if levels.loudness <= SILENT_LUFS {
                    return Gain::unchanged(Some("Not normalised: too quiet to measure its loudness."));
                }
                let wanted = target - levels.loudness;
                let room = TRUE_PEAK_CEILING - levels.true_peak;
                if wanted <= room {
                    let note = format!("Normalised to {target} LUFS ({}), true peak {:.1} dBTP.", signed(wanted), levels.true_peak + wanted);
                    Gain { decibels: wanted, note: Some(note) }
                } else {
                    let note = format!(
                        "Reached {:.1} LUFS, not {target}: getting there would take the true peak over {TRUE_PEAK_CEILING} dBTP, so the gain stopped at {}.",
                        levels.loudness + room,
                        signed(room)
                    );
                    Gain { decibels: room, note: Some(note) }
                }
            }
        }
    }
}

impl fmt::Display for Normalise {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Normalise::Off => f.write_str("Off"),
            Normalise::Peak(decibels) => write!(f, "True peak at {decibels} dBTP"),
            Normalise::Loudness(lufs) => write!(f, "Loudness at {lufs} LUFS integrated"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Stage {
    pub frames: Frames,
    pub render_units: f32,
    pub other_nanoseconds_a_frame: f64,
}

impl Stage {
    fn nanoseconds(&self, frames: Frames, nanoseconds_a_unit: f64) -> f64 {
        frames as f64 * (self.other_nanoseconds_a_frame + self.render_units as f64 * nanoseconds_a_unit)
    }
}

pub(crate) fn render_units(project: &Project, only: Option<TrackId>) -> f32 {
    let mut units = 1.0 + PLUGIN_SLOT_UNITS * project.master_fx.len() as f32;
    for track in &project.tracks {
        if only.is_some_and(|wanted| wanted != track.id) {
            continue;
        }
        units += 1.0 + PLUGIN_SLOT_UNITS * track.fx.len() as f32;
        for clip in &track.clips {
            units += PLUGIN_SLOT_UNITS * clip.fx.len() as f32;
        }
    }
    units
}

pub(crate) fn stages_of(project: &Project, plan: &ExportPlan, span: Frames, stems: &[&Track], held_in_memory: bool) -> Vec<Stage> {
    let encoding = plan.format.nanoseconds_a_frame();
    let mut stages = Vec::with_capacity(2 + stems.len());
    if plan.normalise == Normalise::Off {
        stages.push(Stage { frames: span, render_units: render_units(project, None), other_nanoseconds_a_frame: encoding });
    } else {
        let spilling = if held_in_memory { 0.0 } else { Format::WavFloat.nanoseconds_a_frame() };
        stages.push(Stage {
            frames: span,
            render_units: render_units(project, None),
            other_nanoseconds_a_frame: METER_NANOSECONDS_A_FRAME + spilling,
        });
        let reading = if held_in_memory { 0.0 } else { READING_BACK_NANOSECONDS_A_FRAME };
        stages.push(Stage { frames: span, render_units: 0.0, other_nanoseconds_a_frame: reading + encoding });
    }
    for track in stems {
        stages.push(Stage {
            frames: span,
            render_units: render_units(project, Some(track.id)),
            other_nanoseconds_a_frame: encoding,
        });
    }
    stages
}

pub const STOPPED: &str = "the export was stopped";

pub(crate) struct Pacer<'a> {
    report: &'a dyn Fn(f32) -> bool,
    stages: Vec<Stage>,
    at: usize,
    done: Frames,
    stages_before: f64,
    stage_begun: Instant,
    nanoseconds_a_unit: f64,
    highest: f32,
    stopped: bool,
}

impl<'a> Pacer<'a> {
    pub fn new(report: &'a dyn Fn(f32) -> bool, stages: Vec<Stage>) -> Self {
        Self {
            report,
            stages,
            at: 0,
            done: 0,
            stages_before: 0.0,
            stage_begun: Instant::now(),
            nanoseconds_a_unit: RENDER_NANOSECONDS_A_UNIT,
            highest: 0.0,
            stopped: false,
        }
    }

    fn left(&self) -> f64 {
        let mut left = 0.0;
        if let Some(stage) = self.stages.get(self.at) {
            left += stage.nanoseconds(stage.frames.saturating_sub(self.done), self.nanoseconds_a_unit);
        }
        for stage in self.stages.iter().skip(self.at + 1) {
            left += stage.nanoseconds(stage.frames, self.nanoseconds_a_unit);
        }
        left
    }

    fn spent(&self) -> f64 {
        self.stages_before + self.stage_begun.elapsed().as_nanos() as f64
    }

    fn time_the_render(&mut self) {
        let Some(stage) = self.stages.get(self.at) else { return };
        if stage.render_units <= 0.0 || self.done < FRAMES_BEFORE_TIMING_THE_RENDER {
            return;
        }
        let apart_from_the_render = self.done as f64 * stage.other_nanoseconds_a_frame;
        let seen = (self.stage_begun.elapsed().as_nanos() as f64 - apart_from_the_render)
            / (self.done as f64 * stage.render_units as f64);
        if seen.is_finite() && seen > 0.0 {
            self.nanoseconds_a_unit = seen;
        }
    }

    fn tell(&mut self) {
        let spent = self.spent();
        let whole = spent + self.left();
        let fraction = if whole > 0.0 { (spent / whole) as f32 } else { 0.0 };
        self.highest = self.highest.max(fraction.clamp(0.0, 1.0));
        self.stopped |= !(self.report)(self.highest);
    }

    pub fn wrote(&mut self, frames: Frames) {
        self.done += frames;
        self.time_the_render();
        self.tell();
    }

    pub fn stopped(&self) -> bool {
        self.stopped
    }

    pub fn finished_a_stage(&mut self) {
        self.stages_before += self.stage_begun.elapsed().as_nanos() as f64;
        self.stage_begun = Instant::now();
        self.done = 0;
        self.at += 1;
        self.tell();
    }
}

pub fn export(project: &Project, plan: &ExportPlan, progress: &dyn Fn(f32) -> bool) -> Result<Option<String>, String> {
    export_through(project, plan, progress, None)
}

pub fn export_through(
    project: &Project,
    plan: &ExportPlan,
    progress: &dyn Fn(f32) -> bool,
    mut chains: Option<&mut (dyn crate::render::Chains + '_)>,
) -> Result<Option<String>, String> {
    let (from, to) = plan.range.unwrap_or((0, project.length()));
    if to <= from {
        return Err("there is nothing to export".into());
    }
    let failed = |what: &Path, why: io::Error| format!("{}: {why}", what.display());
    fs::create_dir_all(&plan.folder).map_err(|why| failed(&plan.folder, why))?;

    let stems: Vec<&Track> =
        if plan.split { project.tracks.iter().filter(|track| !track.muted).collect() } else { Vec::new() };
    let measuring = plan.normalise != Normalise::Off;
    let span = to - from;
    let in_memory = span <= MOST_HELD_FRAMES;
    let mut pacer = Pacer::new(progress, stages_of(project, plan, span, &stems, in_memory));

    let extension = plan.format.extension();
    let mix = plan.folder.join(format!("{}.{extension}", plan.name));
    let gain = if measuring {
        let spilt = plan.folder.join(format!("{}.{MEASURING}", plan.name));
        let mut mixdown = if in_memory { Mixed::Memory(Vec::new()) } else { Mixed::Spilt(spilt.clone()) };
        let made = measure_into(&mut mixdown, project, from, to, &mut pacer, chains.as_deref_mut()).and_then(|levels| {
            pacer.finished_a_stage();
            let gain = plan.normalise.gain(levels);
            write_mixdown(&mixdown, &mix, plan, project.rate, span, gain.linear(), &mut pacer)?;
            pacer.finished_a_stage();
            Ok(gain)
        });
        let _ = fs::remove_file(&spilt);
        made.map_err(|why| failed(&mix, why))?
    } else {
        write_file(&mix, plan.format, plan.dither, project, from, to, 1.0, &mut pacer, chains.as_deref_mut())
            .map_err(|why| failed(&mix, why))?;
        pacer.finished_a_stage();
        Gain::unchanged(None)
    };
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
            let file = stems_folder.join(format!("{}.{extension}", unused_name(&track.name, &mut used)));
            write_file(&file, plan.format, plan.dither, &alone, from, to, gain.linear(), &mut pacer, chains.as_deref_mut())
                .map_err(|why| failed(&file, why))?;
            pacer.finished_a_stage();
        }
    }
    let mut notes: Vec<String> = gain.note.into_iter().collect();
    let written_rate = Format::mp3_rate(project.rate);
    if plan.format == Format::Mp3Cbr320 && written_rate != project.rate {
        notes.push(format!("MP3 stops at 48 kHz, so it was written at {written_rate} Hz."));
    }
    Ok((!notes.is_empty()).then(|| notes.join(" ")))
}

pub fn render_to_wav(project: &Project, path: &Path, from: Frames, to: Frames) -> Result<(), String> {
    render_to_wav_through(project, path, from, to, None)
}

pub fn render_to_wav_through(
    project: &Project,
    path: &Path,
    from: Frames,
    to: Frames,
    chains: Option<&mut (dyn crate::render::Chains + '_)>,
) -> Result<(), String> {
    if to <= from {
        return Err("there is nothing to write".into());
    }
    let quiet = |_: f32| true;
    let span = to - from;
    let mut pacer = Pacer::new(&quiet, vec![Stage {
        frames: span,
        render_units: render_units(project, None),
        other_nanoseconds_a_frame: Format::WavFloat.nanoseconds_a_frame(),
    }]);
    write_file(path, Format::WavFloat, false, project, from, to, 1.0, &mut pacer, chains)
        .map_err(|why| format!("{}: {why}", path.display()))
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

fn amplify(block: &mut [[f32; 2]], gain: f32) {
    if gain != 1.0 {
        for frame in block {
            frame[0] *= gain;
            frame[1] *= gain;
        }
    }
}

fn render_into(
    project: &Project,
    from: Frames,
    to: Frames,
    pacer: &mut Pacer<'_>,
    mut chains: Option<&mut (dyn crate::render::Chains + '_)>,
    take: &mut dyn FnMut(&mut [[f32; 2]]) -> io::Result<()>,
) -> io::Result<()> {
    let mut block = vec![[0.0f32; 2]; BLOCK];
    let mut spare = crate::render::Mixdown::default();
    let mut pos = from;
    while pos < to {
        let count = BLOCK.min((to - pos) as usize);
        crate::render::render_through(project, pos, &mut block[..count], &mut spare, chains.as_deref_mut());
        take(&mut block[..count])?;
        pos += count as Frames;
        pacer.wrote(count as Frames);
        if pacer.stopped() {
            return Err(io::Error::other(STOPPED));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_file(
    path: &Path,
    format: Format,
    dither: bool,
    project: &Project,
    from: Frames,
    to: Frames,
    gain: f32,
    pacer: &mut Pacer<'_>,
    chains: Option<&mut (dyn crate::render::Chains + '_)>,
) -> io::Result<()> {
    let mut writer = Writer::create(path, format, project.rate, to - from, dither)?;
    render_into(project, from, to, pacer, chains, &mut |block| {
        amplify(block, gain);
        writer.push(block)
    })?;
    writer.finish()
}


fn levels_of(meter: &mut Meter, rate: u32) -> Levels {
    let mut last_step = vec![[0.0f32; 2]; (rate / METER_STEPS_PER_SECOND) as usize];
    meter.process(&mut last_step);
    let readings = meter.readings();
    Levels { loudness: readings.integrated(), true_peak: readings.peak() }
}

enum Mixed {
    Memory(Vec<[f32; 2]>),
    Spilt(PathBuf),
}

fn measure_into(
    mixdown: &mut Mixed,
    project: &Project,
    from: Frames,
    to: Frames,
    pacer: &mut Pacer<'_>,
    chains: Option<&mut (dyn crate::render::Chains + '_)>,
) -> io::Result<Levels> {
    let mut meter = Meter::new();
    meter.prepare(project.rate as f32);
    match mixdown {
        Mixed::Memory(kept) => {
            kept.reserve((to - from) as usize);
            render_into(project, from, to, pacer, chains, &mut |block| {
                meter.process(block);
                kept.extend_from_slice(block);
                Ok(())
            })?;
        }
        Mixed::Spilt(path) => {
            let mut writer = Writer::create(path, Format::WavFloat, project.rate, to - from, false)?;
            render_into(project, from, to, pacer, chains, &mut |block| {
                meter.process(block);
                writer.push(block)
            })?;
            writer.finish()?;
        }
    }
    Ok(levels_of(&mut meter, project.rate))
}

fn write_mixdown(
    mixdown: &Mixed,
    path: &Path,
    plan: &ExportPlan,
    rate: u32,
    frames: Frames,
    gain: f32,
    pacer: &mut Pacer<'_>,
) -> io::Result<()> {
    let mut writer = Writer::create(path, plan.format, rate, frames, plan.dither)?;
    let mut block = vec![[0.0f32; 2]; BLOCK];
    match mixdown {
        Mixed::Memory(kept) => {
            for part in kept.chunks(BLOCK) {
                block[..part.len()].copy_from_slice(part);
                amplify(&mut block[..part.len()], gain);
                writer.push(&block[..part.len()])?;
                pacer.wrote(part.len() as Frames);
                if pacer.stopped() {
                    return Err(io::Error::other(STOPPED));
                }
            }
        }
        Mixed::Spilt(held) => {
            let mut input = BufReader::new(File::open(held)?);
            let mut header = [0u8; wav::HEADER_BYTES as usize];
            input.read_exact(&mut header)?;
            let mut bytes = vec![0u8; BLOCK * 8];
            let mut left = frames;
            while left > 0 {
                let count = BLOCK.min(left as usize);
                input.read_exact(&mut bytes[..count * 8])?;
                for (frame, raw) in block.iter_mut().zip(bytes[..count * 8].chunks_exact(8)) {
                    *frame = [
                        f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]),
                        f32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]),
                    ];
                }
                amplify(&mut block[..count], gain);
                writer.push(&block[..count])?;
                left -= count as Frames;
                pacer.wrote(count as Frames);
                if pacer.stopped() {
                    return Err(io::Error::other(STOPPED));
                }
            }
        }
    }
    writer.finish()
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
        crate::render::render(project, 0, &mut out);
        out
    }

    fn plan_for(folder: &Path) -> ExportPlan {
        ExportPlan {
            folder: folder.to_path_buf(),
            name: "Song".into(),
            split: false,
            range: None,
            project_file: String::new(),
            format: Format::WavFloat,
            dither: false,
            normalise: Normalise::Off,
        }
    }

    fn tone_song(level: f32, seconds: u32) -> Project {
        let mut p = Project::new(48_000);
        for (name, hz) in [("Low", 220.0f32), ("High", 1_000.0)] {
            let Ok(Outcome::Track(track)) = p.apply(Command::AddTrack { name: name.into() }) else {
                panic!("no track")
            };
            let frames = (0..48_000 * seconds)
                .map(|i| {
                    let s = level * (std::f32::consts::TAU * hz * i as f32 / 48_000.0).sin();
                    [s, s]
                })
                .collect();
            let source = Arc::new(Source::from_frames(name, frames));
            p.apply(Command::AddClip { track, source, start: 0 }).unwrap();
        }
        p
    }

    fn measured(path: &Path) -> Levels {
        let read = Source::load(path, 48_000).unwrap();
        let mut meter = Meter::new();
        meter.prepare(48_000.0);
        let mut audio = read.frames;
        meter.process(&mut audio);
        levels_of(&mut meter, 48_000)
    }

    #[test]
    fn the_exported_mix_is_exactly_what_plays() {
        let project = song();
        let folder = scratch("mix");
        let plan = ExportPlan { project_file: "saved".into(), ..plan_for(&folder) };
        export(&project, &plan, &|_| true).unwrap();
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
            ExportPlan { split: true, ..plan_for(&folder) };
        export(&project, &plan, &|_| true).unwrap();
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

    fn shares(stages: &[Stage], nanoseconds_a_unit: f64) -> Vec<f64> {
        let each: Vec<f64> = stages.iter().map(|stage| stage.nanoseconds(stage.frames, nanoseconds_a_unit)).collect();
        let whole: f64 = each.iter().sum();
        each.into_iter().map(|part| part / whole).collect()
    }

    #[test]
    fn each_stage_of_an_export_is_weighted_by_what_it_costs() {
        let project = song();
        let span = project.length();
        let folder = scratch("weights");
        let plain_stages = stages_of(&project, &ExportPlan { format: Format::Mp3Cbr320, ..plan_for(&folder) }, span, &[], true);
        assert_eq!(plain_stages.len(), 1);
        assert_eq!(plain_stages[0].other_nanoseconds_a_frame, Format::Mp3Cbr320.nanoseconds_a_frame());
        assert_eq!(plain_stages[0].render_units, render_units(&project, None));

        let normalising = ExportPlan { format: Format::Mp3Cbr320, normalise: Normalise::Loudness(-14.0), ..plan_for(&folder) };
        let two = stages_of(&project, &normalising, span, &[], true);
        assert_eq!(two.len(), 2, "measuring renders once and writes once");
        assert_eq!(two[0].other_nanoseconds_a_frame, METER_NANOSECONDS_A_FRAME);
        assert_eq!(two[1].render_units, 0.0, "the second pass renders nothing");
        assert_eq!(two[1].other_nanoseconds_a_frame, Format::Mp3Cbr320.nanoseconds_a_frame());
        let measured = shares(&two, RENDER_NANOSECONDS_A_UNIT)[0];
        assert!(measured < 0.3, "metering an MP3 export is cheap next to encoding it, got {measured}");

        let spilt = stages_of(&project, &normalising, span, &[], false);
        assert!(spilt[0].other_nanoseconds_a_frame > two[0].other_nanoseconds_a_frame, "a spilt mixdown costs a write");
        assert!(spilt[1].other_nanoseconds_a_frame > two[1].other_nanoseconds_a_frame, "and a read back");

        let stems: Vec<&Track> = project.tracks.iter().filter(|track| !track.muted).collect();
        let split = stages_of(&project, &ExportPlan { split: true, ..normalising }, span, &stems, true);
        assert_eq!(split.len(), 2 + stems.len());
        for stage in &split[2..] {
            assert!(stage.render_units < split[0].render_units, "one track is less work than the whole mix");
        }
        let whole: f64 = shares(&split, RENDER_NANOSECONDS_A_UNIT).iter().sum();
        assert!((whole - 1.0).abs() < 1e-9);
        let _ = fs::remove_dir_all(folder);
    }

    #[test]
    fn plugins_count_towards_what_a_render_costs() {
        let mut project = song();
        let bare = render_units(&project, None);
        project.tracks[0].fx.push(crate::model::Fx {
            mix: 1.0,
            path: "a.vst3".into(),
            index: 0,
            name: "A".into(),
            bypassed: false,
            state: Vec::new(),
            record: false,
        });
        assert!(render_units(&project, None) > bare, "a plugin is work the bar should know about");
        assert!(render_units(&project, Some(project.tracks[1].id)) < render_units(&project, None));
    }

    #[test]
    fn the_bar_does_not_leap_through_the_cheap_half_of_a_normalised_export() {
        let project = tone_song(0.05, 4);
        let folder = scratch("pace");
        let plan = ExportPlan { format: Format::Mp3Cbr320, normalise: Normalise::Loudness(-14.0), ..plan_for(&folder) };
        let seen = std::cell::RefCell::new(Vec::new());
        export(&project, &plan, &|fraction| { seen.borrow_mut().push(fraction); true }).unwrap();
        let seen = seen.into_inner();
        let rendered = project.length() as usize / BLOCK;
        let after_the_render = seen[rendered.saturating_sub(1)];
        assert!(after_the_render < 0.45, "rendering is not half the work of an MP3 export, got {after_the_render}");
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn progress_only_climbs_and_ends_complete() {
        let project = song();
        let folder = scratch("progress");
        let plan =
            ExportPlan { split: true, ..plan_for(&folder) };
        let seen = std::cell::RefCell::new(Vec::new());
        export(&project, &plan, &|fraction| { seen.borrow_mut().push(fraction); true }).unwrap();
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
        let plan = ExportPlan { range: Some((1500, 2500)), ..plan_for(&folder) };
        export(&project, &plan, &|_| true).unwrap();
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

    #[test]
    fn peak_normalising_moves_the_true_peak_to_the_target() {
        let gain = Normalise::Peak(-1.0).gain(Levels { loudness: -20.0, true_peak: -6.0 });
        assert_eq!(gain.decibels, 5.0);
        let gain = Normalise::Peak(-0.1).gain(Levels { loudness: -8.0, true_peak: 1.5 });
        assert!((gain.decibels + 1.6).abs() < 1e-6);
    }

    #[test]
    fn loudness_normalising_stops_at_the_true_peak_ceiling_and_says_so() {
        let fits = Normalise::Loudness(-14.0).gain(Levels { loudness: -20.0, true_peak: -10.0 });
        assert_eq!(fits.decibels, 6.0);
        assert!(fits.note.as_deref().unwrap().starts_with("Normalised to -14 LUFS (+6.0 dB)"), "{:?}", fits.note);
        let capped = Normalise::Loudness(-14.0).gain(Levels { loudness: -20.0, true_peak: -3.0 });
        assert_eq!(capped.decibels, 2.0);
        let note = capped.note.unwrap();
        assert!(note.starts_with("Reached -18.0 LUFS, not -14"), "{note}");
        assert!(note.contains("-1 dBTP"), "{note}");
        let quieter = Normalise::Loudness(-14.0).gain(Levels { loudness: -9.0, true_peak: -0.2 });
        assert_eq!(quieter.decibels, -5.0);
    }

    #[test]
    fn silence_and_off_are_left_alone() {
        assert_eq!(Normalise::Off.gain(Levels { loudness: -20.0, true_peak: -3.0 }), Gain { decibels: 0.0, note: None });
        let silent = Normalise::Loudness(-14.0).gain(Levels { loudness: SILENT_LUFS, true_peak: SILENT_LUFS });
        assert_eq!(silent.decibels, 0.0);
        assert!(silent.note.unwrap().contains("silent"));
    }

    #[test]
    fn normalise_settings_round_trip_and_refuse_nonsense() {
        for choice in [Normalise::Off, Normalise::Peak(-0.1), Normalise::Peak(-1.0), Normalise::Loudness(-14.0), Normalise::Loudness(-23.0)] {
            assert_eq!(Normalise::from_key(&choice.key()), Some(choice));
        }
        for bad in ["", "peak", "peak 3", "lufs -100", "lufs x", "loud -14", "peak NaN"] {
            assert_eq!(Normalise::from_key(bad), None, "{bad} was accepted");
        }
    }

    #[test]
    fn a_loudness_target_is_met_in_the_written_file() {
        let project = tone_song(0.05, 6);
        let folder = scratch("lufs");
        let plan = ExportPlan { normalise: Normalise::Loudness(-14.0), format: Format::Wav24, dither: true, ..plan_for(&folder) };
        let note = export(&project, &plan, &|_| true).unwrap().unwrap();
        assert!(note.starts_with("Normalised to -14 LUFS"), "{note}");
        let levels = measured(&folder.join("Song.wav"));
        assert!((levels.loudness + 14.0).abs() < 0.1, "{levels:?}");
        assert!(!folder.join(format!("Song.{MEASURING}")).exists(), "the measuring file is cleaned up");
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_peak_target_is_met_in_the_written_file() {
        let project = tone_song(0.05, 3);
        let folder = scratch("peak");
        let plan = ExportPlan { normalise: Normalise::Peak(-1.0), format: Format::Flac16, dither: true, ..plan_for(&folder) };
        export(&project, &plan, &|_| true).unwrap();
        let levels = measured(&folder.join("Song.flac"));
        assert!((levels.true_peak + 1.0).abs() < 0.1, "{levels:?}");
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_loudness_target_beyond_the_peak_ceiling_keeps_the_peak_safe() {
        let project = tone_song(0.05, 3);
        let folder = scratch("ceiling");
        let plan = ExportPlan { normalise: Normalise::Loudness(0.0), ..plan_for(&folder) };
        let note = export(&project, &plan, &|_| true).unwrap().unwrap();
        assert!(note.starts_with("Reached "), "{note}");
        let levels = measured(&folder.join("Song.wav"));
        assert!((levels.true_peak - TRUE_PEAK_CEILING).abs() < 0.1, "{levels:?}");
        assert!(levels.loudness < -1.0, "{levels:?}");
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn normalised_stems_take_the_same_gain_so_they_still_add_up_to_the_mix() {
        let project = tone_song(0.05, 2);
        let folder = scratch("stems");
        let plan = ExportPlan { split: true, normalise: Normalise::Peak(-3.0), ..plan_for(&folder) };
        export(&project, &plan, &|_| true).unwrap();
        let mix = Source::load(&folder.join("Song.wav"), 48_000).unwrap();
        let low = Source::load(&folder.join(STEMS_FOLDER).join("Low.wav"), 48_000).unwrap();
        let high = Source::load(&folder.join(STEMS_FOLDER).join("High.wav"), 48_000).unwrap();
        let worst = mix
            .frames
            .iter()
            .zip(low.frames.iter().zip(&high.frames))
            .map(|(m, (a, b))| (m[0] - a[0] - b[0]).abs())
            .fold(0.0, f32::max);
        assert!(worst < 1e-5, "stems are {worst} away from the mix");
        assert!(rms_of(&mix.frames) > 0.1, "the mix was turned up");
        fs::remove_dir_all(folder).unwrap();
    }

    fn rms_of(audio: &[[f32; 2]]) -> f32 {
        (audio.iter().map(|f| f[0] * f[0]).sum::<f32>() / audio.len() as f32).sqrt()
    }

    #[test]
    fn stems_follow_the_chosen_format() {
        let project = song();
        let folder = scratch("stem-format");
        let plan = ExportPlan { split: true, format: Format::Mp3Cbr320, ..plan_for(&folder) };
        export(&project, &plan, &|_| true).unwrap();
        assert!(folder.join("Song.mp3").exists());
        assert!(folder.join(STEMS_FOLDER).join("Beat.mp3").exists());
        assert!(folder.join(STEMS_FOLDER).join("Lead vox.mp3").exists());
        assert!(!folder.join("Song.wav").exists());
        fs::remove_dir_all(folder).unwrap();
    }
}
