use std::path::Path;
use std::sync::Arc;

use loupe_engine::{Chains, Command, Mixdown, SavedProject, Source};

#[path = "../src/racks.rs"]
mod racks;

const BLOCK: usize = 16_384;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("stemtap <project.lp> <audio.wav> <track name> [second track] [second audio]");
        std::process::exit(2);
    }
    let rate = 48_000u32;
    let text = std::fs::read_to_string(&args[1]).expect("read the project");
    let saved = SavedProject::parse(&text).expect("parse the project");
    let (mut project, _) = saved.build(&[], rate);
    let voice = Arc::new(Source::load(Path::new(&args[2]), rate).expect("read the audio"));
    let Some(track) = project.tracks.iter().find(|found| found.name == args[3]).map(|found| found.id) else {
        eprintln!("no track called {}", args[3]);
        std::process::exit(2);
    };
    project.apply(Command::AddSource(voice.clone())).expect("hold the audio");
    project.apply(Command::AddClip { track, source: voice.clone(), start: 0 }).expect("place the audio");
    let mut chains = racks::Racks::new(rate, BLOCK, racks::Peeks::default(), Default::default(), Default::default()).away_from_the_audio_thread();
    chains.follow(&project);
    let waited = std::time::Instant::now();
    while chains.still_opening() && waited.elapsed() < std::time::Duration::from_secs(120) {
        chains.nudge();
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let until = voice.frames.len() as u64 + rate as u64 * 2;
    let mut spare = Mixdown::default();
    let mut block = vec![[0.0f32; 2]; BLOCK];
    let mut loudest = vec![0.0f32; project.tracks.len()];
    let mut pos = 0u64;
    while pos < until {
        let count = BLOCK.min((until - pos) as usize);
        loupe_engine::render_through(&project, pos, &mut block[..count], &mut spare, Some(&mut chains));
        for (at, top) in loudest.iter_mut().enumerate() {
            if let Some(tapped) = spare.tap(at, count) {
                for frame in tapped {
                    *top = top.max(frame[0].abs()).max(frame[1].abs());
                }
            }
        }
        pos += count as u64;
    }
    for (at, track) in project.tracks.iter().enumerate() {
        let db = 20.0 * loudest[at].max(1e-9).log10();
        println!("{:<16} peak {db:>7.1} dBFS", track.name);
    }
}
