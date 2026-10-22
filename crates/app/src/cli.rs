use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use loupe_engine::{Chains, Fx, Project, SavedProject, Track};

use crate::racks;

const USAGE: &str = "\
loupe — a digital audio workstation

usage:
  loupe [project.lp]                            open the app
  loupe tracks <project.lp>                     print the track list
  loupe render <project.lp> <out.wav>           render the master bus to one wav
  loupe render <project.lp> <folder> --stems    render one wav per track into that folder
  loupe bench <project.lp> [frames] [seconds] [from]   time the live mix through the plugins, no window
  loupe --help                                  print this";

const OPENS_WITHIN: Duration = Duration::from_secs(90);
const ASKS_AGAIN_IN: Duration = Duration::from_millis(50);
const HEADINGS: [&str; 5] = ["#", "track", "into", "sends", "fx"];
const MASTER_ROW: &str = "M";
const NOTHING: &str = "-";
const GAP: &str = "  ";
const BETWEEN_SLOTS: &str = " > ";

#[derive(Clone, Debug, PartialEq)]
pub enum Asked {
    Gui,
    Usage,
    Wrong(String),
    Tracks(PathBuf),
    Render { project: PathBuf, out: PathBuf, stems: bool },
    Bench { project: PathBuf, frames: usize, seconds: f64, from: f64 },
}

pub fn asked_for(args: &[String]) -> Asked {
    let Some(first) = args.first() else {
        return Asked::Gui;
    };
    match first.as_str() {
        "tracks" => match &args[1..] {
            [project] => Asked::Tracks(PathBuf::from(project)),
            _ => Asked::Wrong("tracks takes one project file".into()),
        },
        "bench" => match &args[1..] {
            [project, rest @ ..] => {
                let mut got = [256.0, 10.0, 0.0];
                for (slot, text) in got.iter_mut().zip(rest) {
                    match text.parse::<f64>() {
                        Ok(number) if number >= 0.0 => *slot = number,
                        _ => return Asked::Wrong(format!("bench does not understand {text}")),
                    }
                }
                Asked::Bench { project: PathBuf::from(project), frames: got[0] as usize, seconds: got[1], from: got[2] }
            }
            _ => Asked::Wrong("bench takes a project file, then optionally frames a block, seconds to run and where to start in seconds".into()),
        },
        "render" => {
            let mut stems = false;
            let mut places: Vec<&String> = Vec::new();
            for arg in &args[1..] {
                match arg.as_str() {
                    "--stems" => stems = true,
                    flag if flag.starts_with('-') => return Asked::Wrong(format!("render does not know {flag}")),
                    _ => places.push(arg),
                }
            }
            match places.as_slice() {
                [project, out] => Asked::Render { project: PathBuf::from(*project), out: PathBuf::from(*out), stems },
                _ if stems => Asked::Wrong("render --stems takes a project file and a folder to write into".into()),
                _ => Asked::Wrong("render takes a project file and a wav to write".into()),
            }
        }
        "--help" | "-h" => Asked::Usage,
        _ => Asked::Gui,
    }
}

#[cfg(windows)]
fn speak_to_the_shell() {
    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(process: u32) -> i32;
        fn GetStdHandle(which: u32) -> isize;
        fn GetFileType(handle: isize) -> u32;
    }
    const PARENT: u32 = u32::MAX;
    const STD_OUTPUT: u32 = -11i32 as u32;
    const UNKNOWN: u32 = 0;
    unsafe {
        let already = GetStdHandle(STD_OUTPUT);
        if already != 0 && already != -1 && GetFileType(already) != UNKNOWN {
            return;
        }
        AttachConsole(PARENT);
    }
}

#[cfg(not(windows))]
fn speak_to_the_shell() {}

pub fn run(asked: Asked) -> i32 {
    speak_to_the_shell();
    match asked {
        Asked::Gui => 0,
        Asked::Usage => {
            println!("{USAGE}");
            0
        }
        Asked::Wrong(why) => {
            eprintln!("loupe: {why}\n\n{USAGE}");
            2
        }
        Asked::Tracks(project) => match listing(&project) {
            Ok(told) => {
                print!("{told}");
                0
            }
            Err(why) => {
                eprintln!("loupe: {why}");
                1
            }
        },
        Asked::Bench { project, frames, seconds, from } => match benching(&project, frames, seconds, from) {
            Ok(told) => {
                print!("{told}");
                0
            }
            Err(why) => {
                eprintln!("loupe: {why}");
                1
            }
        },
        Asked::Render { project, out, stems } => match rendering(&project, &out, stems) {
            Ok(written) => {
                for file in written {
                    println!("{}", file.display());
                }
                0
            }
            Err(why) => {
                eprintln!("loupe: {why}");
                1
            }
        },
    }
}

fn listing(file: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(file).map_err(|why| format!("{}: {why}", file.display()))?;
    let saved = SavedProject::parse(&text)?;
    let (project, _) = saved.build(&[], saved.rate);
    Ok(listed(&project))
}

fn listed(project: &Project) -> String {
    let mut rows: Vec<[String; 5]> = project
        .tracks
        .iter()
        .enumerate()
        .map(|(at, track)| {
            [
                (at + 1).to_string(),
                track.name.clone(),
                into_what(project, track),
                sends_of(project, track),
                chain_of(&track.fx),
            ]
        })
        .collect();
    rows.push([
        MASTER_ROW.to_string(),
        "Master".to_string(),
        NOTHING.to_string(),
        NOTHING.to_string(),
        chain_of(&project.master_fx),
    ]);
    let mut widths: Vec<usize> = HEADINGS.iter().map(|heading| heading.chars().count()).collect();
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let mut out = String::new();
    let headings: [String; 5] = HEADINGS.map(str::to_string);
    for row in std::iter::once(&headings).chain(rows.iter()) {
        out.push_str(&laid_out(row, &widths));
        out.push('\n');
    }
    out
}

fn laid_out(row: &[String; 5], widths: &[usize]) -> String {
    let mut line = right(&row[0], widths[0]);
    for (cell, width) in row[1..].iter().zip(&widths[1..]) {
        line.push_str(GAP);
        line.push_str(&left(cell, *width));
    }
    line.trim_end().to_string()
}

fn left(cell: &str, width: usize) -> String {
    format!("{cell}{}", " ".repeat(width.saturating_sub(cell.chars().count())))
}

fn right(cell: &str, width: usize) -> String {
    format!("{}{cell}", " ".repeat(width.saturating_sub(cell.chars().count())))
}

fn into_what(project: &Project, track: &Track) -> String {
    match track.parent.and_then(|parent| project.track(parent)) {
        Some(found) => found.name.clone(),
        None => "Master".to_string(),
    }
}

fn sends_of(project: &Project, track: &Track) -> String {
    let told: Vec<String> = track
        .sends
        .iter()
        .map(|send| {
            let name = project.track(send.to).map_or_else(|| "gone".to_string(), |found| found.name.clone());
            format!("{name} {} dB", crate::mixer::level_text(send.gain))
        })
        .collect();
    joined(told)
}

fn chain_of(chain: &[Fx]) -> String {
    match chain.is_empty() {
        true => NOTHING.to_string(),
        false => chain.iter().map(slot_text).collect::<Vec<String>>().join(BETWEEN_SLOTS),
    }
}

fn slot_text(fx: &Fx) -> String {
    let mut notes: Vec<String> = Vec::new();
    if fx.record {
        notes.push("rec".to_string());
    }
    if fx.mix != 1.0 {
        notes.push(format!("mix {:.0}%", fx.mix * 100.0));
    }
    match notes.is_empty() {
        true => fx.name.clone(),
        false => format!("{} ({})", fx.name, notes.join(", ")),
    }
}

fn joined(told: Vec<String>) -> String {
    match told.is_empty() {
        true => NOTHING.to_string(),
        false => told.join(", "),
    }
}

fn rendering(file: &Path, out: &Path, stems: bool) -> Result<Vec<PathBuf>, String> {
    let text = std::fs::read_to_string(file).map_err(|why| format!("{}: {why}", file.display()))?;
    let rate = SavedProject::parse(&text)?.rate;
    let opened = crate::files::read_text(&text, rate)?;
    for audio in opened.missing() {
        eprintln!("loupe: audio not found: {audio}");
    }
    let (project, _) = opened.build(rate);
    let until = project.length();
    if until == 0 {
        return Err("there is nothing on this project's timeline".into());
    }
    if !stems {
        if let Some(folder) = out.parent().filter(|folder| !folder.as_os_str().is_empty()) {
            std::fs::create_dir_all(folder).map_err(|why| format!("{}: {why}", folder.display()))?;
        }
    }
    let mut racks: Box<dyn Chains> = Box::new(
        racks::Racks::new(rate, loupe_engine::EXPORT_BLOCK, racks::Peeks::default(), racks::PluginsOff::default(), racks::Falls::default())
            .away_from_the_audio_thread(),
    );
    racks.follow(&project);
    let went = opened_in_time(racks.as_mut()).and_then(|()| {
        for (_, name, why) in racks.troubles() {
            eprintln!("loupe: {name}: {why}");
        }
        match stems {
            true => loupe_engine::render_stems_through(&project, out, 0, until, Some(racks.as_mut())),
            false => loupe_engine::render_to_wav_through(&project, out, 0, until, Some(racks.as_mut()))
                .map(|()| vec![out.to_path_buf()]),
        }
    });
    drop(racks);
    went
}

/// Runs the live mix the way the audio thread does, block after block, through the real
/// plugin hosts, and reports where the time goes. No window, no sound card.
fn benching(file: &Path, frames: usize, seconds: f64, from: f64) -> Result<String, String> {
    let text = std::fs::read_to_string(file).map_err(|why| format!("{}: {why}", file.display()))?;
    let rate = SavedProject::parse(&text)?.rate;
    let opened = crate::files::read_text(&text, rate)?;
    for audio in opened.missing() {
        eprintln!("loupe: audio not found: {audio}");
    }
    let (project, _) = opened.build(rate);
    let frames = frames.clamp(16, 4096);
    let mut racks: Box<dyn Chains> =
        Box::new(racks::Racks::new(rate, racks::CHAIN_BLOCK, racks::Peeks::default(), racks::PluginsOff::default(), racks::Falls::default()));
    racks.follow(&project);
    let opening = Instant::now();
    opened_in_time(racks.as_mut())?;
    let opening = opening.elapsed();
    for (_, name, why) in racks.troubles() {
        eprintln!("loupe: {name}: {why}");
    }
    let start = (from * rate as f64) as loupe_engine::Frames;
    let mut scratch = loupe_engine::Mixdown::default();
    racks.takes_its_time(true);
    let priming = Instant::now();
    scratch.get_ready(&project, start, None, None, frames, Some(racks.as_mut()), None);
    let priming = priming.elapsed();
    racks.takes_its_time(false);
    scratch.hurries();
    let blocks = ((seconds * rate as f64) / frames as f64).max(1.0) as usize;
    let budget = Duration::from_secs_f64(frames as f64 / rate as f64);
    let mut out = vec![[0.0f32; 2]; frames];
    let mut spent: Vec<u64> = Vec::with_capacity(blocks);
    let (count_before, nanos_before, _, late_before) = loupe_plugins::sandbox::exchanges();
    let (waves_before, wave_nanos_before) = racks::wave_timing();
    let began = Instant::now();
    let mut pos = start;
    for _ in 0..blocks {
        racks.song_goes(loupe_engine::Song::where_it_is(&project, pos, true, false));
        let one = Instant::now();
        loupe_engine::render_through(&project, pos, &mut out, &mut scratch, Some(racks.as_mut()));
        spent.push(one.elapsed().as_micros() as u64);
        pos += frames as loupe_engine::Frames;
    }
    let whole = began.elapsed();
    let (count, nanos, slowest, late) = loupe_plugins::sandbox::exchanges();
    let (waves, wave_nanos) = racks::wave_timing();
    let spending = racks.plugin_spending();
    let mut sorted = spent.clone();
    sorted.sort_unstable();
    let at = |share: f64| sorted[((sorted.len() - 1) as f64 * share) as usize];
    let over = spent.iter().filter(|us| Duration::from_micros(**us) > budget).count();
    let exchanges = count - count_before;
    let exchange_us = (nanos - nanos_before) as f64 / 1000.0;
    let scoped = waves - waves_before;
    let scope_us = (wave_nanos - wave_nanos_before) as f64 / 1000.0;
    let mut told = String::new();
    told.push_str(&format!("Loupe bench: {} blocks of {frames} frames at {rate} Hz from {from:.1} s

", spent.len()));
    told.push_str(&format!("plugins    {} running, {} held back, opened in {:.1} s, primed in {:.3} s
", spending.running, spending.held_back, opening.as_secs_f64(), priming.as_secs_f64()));
    told.push_str(&format!("budget     {:>8.0} us a block
", budget.as_secs_f64() * 1e6));
    told.push_str(&format!("block      {:>8} us median  {:>8} us p90  {:>8} us p99  {:>8} us worst
", at(0.5), at(0.9), at(0.99), at(1.0)));
    told.push_str(&format!("over       {over} of {} blocks took longer than the budget
", spent.len()));
    told.push_str(&format!("whole      {:.3} s for {:.3} s of song ({:.2}x real time)

", whole.as_secs_f64(), spent.len() as f64 * frames as f64 / rate as f64, (spent.len() as f64 * frames as f64 / rate as f64) / whole.as_secs_f64().max(1e-9)));
    told.push_str(&format!("exchanges  {exchanges} round trips, {:.1} a block, {:.0} us each on average, {:.0} us slowest, {} late
", exchanges as f64 / spent.len().max(1) as f64, exchange_us / exchanges.max(1) as f64, slowest as f64 / 1000.0, late - late_before));
    told.push_str(&format!("           {:.0} us a block inside the hosts' round trips, {} late replies the racks counted
", exchange_us / spent.len().max(1) as f64, spending.late_replies));
    told.push_str(&format!("waves      {} waves, {} on scoped threads, {:.0} us a block inside those scopes, up to {} tracks at once
", spending.waves, scoped, scope_us / spent.len().max(1) as f64, spending.at_once));
    told.push_str(&format!("plugins    {:.0} us a block in process_wave/master/monitor overall
", spending.nanoseconds as f64 / 1000.0 / spent.len().max(1) as f64));
    drop(racks);
    Ok(told)
}

fn opened_in_time(racks: &mut dyn Chains) -> Result<(), String> {
    let waited = Instant::now();
    while racks.still_opening() && waited.elapsed() < OPENS_WITHIN {
        racks.nudge();
        std::thread::sleep(ASKS_AGAIN_IN);
    }
    if racks.still_opening() {
        let slow: Vec<String> = racks.troubles().into_iter().map(|(_, name, why)| format!("{name}: {why}")).collect();
        let named = match slow.is_empty() {
            true => String::new(),
            false => format!(" ({})", slow.join(", ")),
        };
        return Err(format!("a plugin never opened after {} seconds{named}", OPENS_WITHIN.as_secs()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use loupe_engine::{Command, Outcome};

    fn args(typed: &str) -> Vec<String> {
        typed.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn no_arguments_and_a_dropped_song_both_open_the_app() {
        assert_eq!(asked_for(&[]), Asked::Gui);
        assert_eq!(asked_for(&args("/songs/Get High/Get High.lp")), Asked::Gui);
        assert_eq!(asked_for(&args("lead.wav")), Asked::Gui);
        assert_eq!(asked_for(&args("mixdown nonsense")), Asked::Gui);
    }

    #[test]
    fn the_track_list_is_asked_for_by_name() {
        assert_eq!(asked_for(&args("tracks song.lp")), Asked::Tracks(PathBuf::from("song.lp")));
        assert!(matches!(asked_for(&args("tracks")), Asked::Wrong(_)));
        assert!(matches!(asked_for(&args("tracks one.lp two.lp")), Asked::Wrong(_)));
    }

    #[test]
    fn render_reads_its_two_places_and_the_stems_flag() {
        assert_eq!(
            asked_for(&args("render song.lp out.wav")),
            Asked::Render { project: PathBuf::from("song.lp"), out: PathBuf::from("out.wav"), stems: false }
        );
        let folder = Asked::Render { project: PathBuf::from("song.lp"), out: PathBuf::from("Stems"), stems: true };
        assert_eq!(asked_for(&args("render song.lp Stems --stems")), folder);
        assert_eq!(asked_for(&args("render --stems song.lp Stems")), folder);
        assert!(matches!(asked_for(&args("render song.lp")), Asked::Wrong(_)));
        assert!(matches!(asked_for(&args("render song.lp out.wav extra.wav")), Asked::Wrong(_)));
        assert!(matches!(asked_for(&args("render song.lp out.wav --split")), Asked::Wrong(_)));
    }

    #[test]
    fn help_is_asked_for_both_ways() {
        assert_eq!(asked_for(&args("--help")), Asked::Usage);
        assert_eq!(asked_for(&args("-h")), Asked::Usage);
    }

    fn plugin(name: &str, record: bool, mix: f32) -> Fx {
        Fx { path: PathBuf::from("C:\\VST3\\plug.vst3"), index: 0, name: name.into(), bypassed: false, state: Vec::new(), record, mix }
    }

    #[test]
    fn the_track_list_lines_up_parents_sends_and_slots() {
        let mut project = Project::new(48_000);
        let Ok(Outcome::Track(bus)) = project.apply(Command::AddTrack { name: "Vox Bus".into() }) else { panic!() };
        let Ok(Outcome::Track(lead)) = project.apply(Command::AddTrack { name: "Lead".into() }) else { panic!() };
        let Ok(Outcome::Track(verb)) = project.apply(Command::AddTrack { name: "Reverb".into() }) else { panic!() };
        project.apply(Command::SetTrackParent { track: lead, parent: Some(bus) }).unwrap();
        project.apply(Command::AddSend { from: lead, to: verb }).unwrap();
        project.apply(Command::SetSendGain { from: lead, to: verb, gain: 0.5 }).unwrap();
        project.apply(Command::AddFx { track: lead, fx: plugin("Pro-Q 3", false, 1.0) }).unwrap();
        project.apply(Command::AddFx { track: lead, fx: plugin("Pro-DS", true, 1.0) }).unwrap();
        project.apply(Command::AddFx { track: lead, fx: plugin("CLA-76", false, 0.4) }).unwrap();
        project.apply(Command::AddMasterFx(plugin("Pro-L 2", false, 1.0))).unwrap();
        let told = listed(&project);
        let lines: Vec<&str> = told.lines().collect();
        assert_eq!(lines.len(), 5);
        assert!(lines[0].starts_with("#  track"));
        assert!(lines[1].contains("Vox Bus") && lines[1].contains("Master"));
        assert!(lines[2].contains("Reverb -6.0 dB"), "{}", lines[2]);
        assert!(lines[2].contains("Pro-Q 3 > Pro-DS (rec) > CLA-76 (mix 40%)"), "{}", lines[2]);
        assert!(lines[3].contains("Reverb") && lines[3].ends_with(NOTHING));
        assert!(lines[4].starts_with("M  Master") && lines[4].ends_with("Pro-L 2"), "{}", lines[4]);
        let into_at = lines[0].find("into").unwrap();
        assert!(lines[1][into_at..].starts_with("Master"), "{}", lines[1]);
        assert!(lines[2][into_at..].starts_with("Vox Bus"), "{}", lines[2]);
        assert!(lines[4][into_at..].starts_with(NOTHING), "{}", lines[4]);
    }

    #[test]
    fn a_bare_project_still_lists_its_master() {
        let project = Project::new(48_000);
        let told = listed(&project);
        assert_eq!(told.lines().count(), 2);
        assert!(told.lines().nth(1).unwrap().contains("Master"));
    }
}
