fn main() {
    let path = std::env::args().nth(1).expect("give a plugin path");
    let beside = loupe_plugins::sandbox::host_beside_us();
    let host = if beside.exists() { beside } else { std::path::PathBuf::from("target/debug/loupe-host") };
    let mut rack = loupe_plugins::rack::Rack::new(host, 48_000, 512);
    if let Err(why) = rack.add(std::path::Path::new(&path), 0, "test") {
        println!("trouble: {why}");
        return;
    }
    let names = rack.knobs(0);
    println!("{} knobs", names.len());
    for (at, name) in names.iter().take(8).enumerate() {
        println!("  {at} {name}");
    }
    let mut audio: Vec<[f32; 2]> = vec![[0.5, 0.5]; 512];
    rack.process(&mut audio);
    let before = audio.iter().fold(0.0f32, |top, f| top.max(f[0].abs()));
    for turn in [0.0f32, 1.0] {
        rack.automate(0, 0, turn);
        let mut copy: Vec<[f32; 2]> = vec![[0.5, 0.5]; 512];
        rack.process(&mut copy);
        let after = copy.iter().fold(0.0f32, |top, f| top.max(f[0].abs()));
        println!("knob 0 at {turn}: out {after:.4}");
    }
    println!("before {before:.4}");
}
