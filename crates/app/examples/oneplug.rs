use std::path::{Path, PathBuf};
use std::sync::Arc;

use loupe_engine::{Chains, Command, Fx, Project, Source};

#[path = "../src/crew.rs"]
mod crew;
#[path = "../src/racks.rs"]
mod racks;

const BLOCK: usize = 16_384;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 {
        eprintln!("oneplug <plugin.vst3> <index> <audio.wav> <out.wav>");
        std::process::exit(2);
    }
    let rate = 48_000u32;
    let mut project = Project::new(rate);
    let voice = Arc::new(Source::load(Path::new(&args[3]), rate).expect("read the audio"));
    let loupe_engine::Outcome::Track(track) = project.apply(Command::AddTrack { name: "One".into() }).expect("add") else { return };
    project.apply(Command::AddSource(voice.clone())).expect("hold");
    project.apply(Command::AddClip { track, source: voice.clone(), start: 0 }).expect("place");
    project
        .apply(Command::AddFx {
            track,
            fx: Fx { path: PathBuf::from(&args[1]), index: args[2].parse().unwrap_or(0), name: "plug".into(), bypassed: false, state: Vec::new(), record: false, mix: 1.0 },
        })
        .expect("add fx");
    let mut chains = racks::Racks::new(rate, BLOCK, racks::Peeks::default(), Default::default(), Default::default()).away_from_the_audio_thread();
    chains.follow(&project);
    let waited = std::time::Instant::now();
    while chains.still_opening() && waited.elapsed() < std::time::Duration::from_secs(120) {
        chains.nudge();
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    eprintln!("latency {}", chains.latency(track));
    let until = voice.frames.len() as u64;
    loupe_engine::render_to_wav_through(&project, &PathBuf::from(&args[4]), 0, until, Some(&mut chains)).expect("render");
    println!("wrote {}", args[4]);
}
