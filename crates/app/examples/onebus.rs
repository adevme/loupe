use std::path::{Path, PathBuf};
use std::sync::Arc;

use loupe_engine::{Chains, Command, SavedProject, Source};

#[path = "../src/racks.rs"]
mod racks;

const BLOCK: usize = 512;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rate = 48_000u32;
    let text = std::fs::read_to_string(&args[1]).expect("read the project");
    let saved = SavedProject::parse(&text).expect("parse the project");
    let (mut project, _) = saved.build(&[], rate);
    let voice = Arc::new(Source::load(Path::new(&args[2]), rate).expect("read the audio"));
    let Some(track) = project.tracks.iter().find(|found| found.name == args[3]).map(|found| found.id) else {
        eprintln!("no track called {}", args[3]);
        std::process::exit(2);
    };
    let others: Vec<_> = project.tracks.iter().filter(|found| found.id != track).map(|found| found.id).collect();
    for gone in others {
        let _ = project.apply(Command::RemoveTrack(gone));
    }
    let _ = project.apply(Command::SetTrackParent { track, parent: None });
    project.apply(Command::AddSource(voice.clone())).expect("hold the audio");
    project.apply(Command::AddClip { track, source: voice.clone(), start: 0 }).expect("place the audio");
    let mut chains = racks::Racks::new(rate, BLOCK, racks::Peeks::default(), Default::default(), Default::default()).away_from_the_audio_thread();
    chains.follow(&project);
    let waited = std::time::Instant::now();
    while chains.still_opening() && waited.elapsed() < std::time::Duration::from_secs(120) {
        chains.nudge();
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    eprintln!("loupe reports chain latency: {} samples", chains.latency(track));
    let until = voice.frames.len() as u64;
    loupe_engine::render_to_wav_through(&project, &PathBuf::from(&args[4]), 0, until, Some(&mut chains)).expect("render");
    println!("wrote {}", args[4]);
}
