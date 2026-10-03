use std::time::Instant;

use loupe_plugins::sandbox::{host_beside_us, Sandbox};
use loupe_plugins::wire::{Ask, Reply};

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        println!("give me a plugin path");
        return;
    };
    let mut box_ = match Sandbox::start(&host_beside_us()) {
        Ok(it) => it,
        Err(why) => {
            println!("host would not start: {why}");
            return;
        }
    };
    match box_.ask(Ask::Load { path, index: 0, rate: 48_000, block: 512 }) {
        Ok(Reply::Loaded { .. }) => println!("loaded"),
        Ok(Reply::Trouble(why)) | Err(why) => {
            println!("could not load: {why}");
            return;
        }
        Ok(other) => println!("odd reply {other:?}"),
    }
    let mut audio: Vec<[f32; 2]> = (0..512)
        .map(|i| {
            let t = i as f32 / 48_000.0;
            let v = (t * 440.0 * std::f32::consts::TAU).sin() * 0.5;
            [v, v]
        })
        .collect();
    let before = audio.iter().fold(0.0f32, |top, f| top.max(f[0].abs()));
    let began = Instant::now();
    let rounds = 200;
    for _ in 0..rounds {
        if let Err(why) = box_.run(&mut audio) {
            println!("stopped: {why}");
            return;
        }
    }
    let each = began.elapsed().as_secs_f64() / rounds as f64 * 1000.0;
    let after = audio.iter().fold(0.0f32, |top, f| top.max(f[0].abs()));
    println!("peak in {before:.4} out {after:.4}");
    println!("{each:.3} ms per block of 512 frames (10.7 ms of audio)");
}
