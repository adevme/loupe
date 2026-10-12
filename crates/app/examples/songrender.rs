use std::path::{Path, PathBuf};
use std::sync::Arc;

use loupe_engine::{Chains, SavedProject, Source};

#[path = "../src/racks.rs"]
mod racks;

const BLOCK: usize = 16_384;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("songrender <project.lp> <out.wav>");
        std::process::exit(2);
    }
    let rate = 48_000u32;
    let text = std::fs::read_to_string(&args[1]).expect("read the project");
    let saved = SavedProject::parse(&text).expect("parse the project");
    let here = Path::new(&args[1]).parent().unwrap_or(Path::new("."));
    let sources: Vec<Arc<Source>> = saved
        .sources
        .iter()
        .map(|path| {
            let whole = if path.is_absolute() { path.clone() } else { here.join(path) };
            Arc::new(Source::load(&whole, rate).unwrap_or_else(|why| panic!("{}: {why}", whole.display())))
        })
        .collect();
    let (project, _) = saved.build(&sources, rate);
    let mut chains = racks::Racks::new(rate, BLOCK, racks::Peeks::default(), Default::default(), Default::default()).away_from_the_audio_thread();
    chains.follow(&project);
    let waited = std::time::Instant::now();
    while chains.still_opening() && waited.elapsed() < std::time::Duration::from_secs(180) {
        chains.nudge();
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let until = project.tracks.iter().flat_map(|track| track.clips.iter()).map(|clip| clip.start + clip.len).max().unwrap_or(0);
    loupe_engine::render_to_wav_through(&project, &PathBuf::from(&args[2]), 0, until, Some(&mut chains)).expect("render");
    println!("wrote {} ({until} frames)", args[2]);
}
