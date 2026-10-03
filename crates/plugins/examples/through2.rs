fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("give a plugin path");
    let index: usize = args.next().unwrap_or_else(|| "0".into()).parse().unwrap_or(0);
    let beside = loupe_plugins::sandbox::host_beside_us();
    let host = if beside.exists() {
        beside
    } else {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/loupe-host")
    };
    let mut rack = loupe_plugins::rack::Rack::new(host, 48_000, 512);
    match rack.add(std::path::Path::new(&path), index, "test") {
        Ok(_) => println!("loaded"),
        Err(why) => {
            println!("trouble: {why}");
            return;
        }
    }
    let mut audio: Vec<[f32; 2]> = (0..512).map(|i| {
        let x = (i as f32 / 48_000.0 * 440.0 * std::f32::consts::TAU).sin() * 0.5;
        [x, x]
    }).collect();
    let before = audio.iter().fold(0.0f32, |top, f| top.max(f[0].abs()));
    let began = std::time::Instant::now();
    let source = audio.clone();
    let mut after = 0.0f32;
    for _ in 0..20 {
        audio = source.clone();
        rack.process(&mut audio);
        after = audio.iter().fold(0.0f32, |top, f| top.max(f[0].abs()));
    }
    println!("peak in {before:.4} out {after:.4}");
    println!("{:.3} ms per block", began.elapsed().as_secs_f64() * 1000.0 / 20.0);
    for slot in rack.slots() {
        if let Some(why) = &slot.trouble {
            println!("trouble: {why}");
        }
    }
}
