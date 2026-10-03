fn main() {
    let beside = loupe_plugins::sandbox::host_beside_us();
    let host = if beside.exists() {
        beside
    } else {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/loupe-host")
    };
    println!("host at {}", host.display());
    let found = loupe_plugins::everything();
    let mut opened = 0;
    let mut refused = 0;
    for one in &found {
        if one.format == loupe_plugins::Format::Stock {
            continue;
        }
        let mut rack = loupe_plugins::rack::Rack::new(host.clone(), 48_000, 512);
        match rack.add(&one.path, one.index, &one.name) {
            Ok(_) => {
                let mut audio = vec![[0.25f32, 0.25]; 512];
                rack.process(&mut audio);
                let top = audio.iter().fold(0.0f32, |most, frame| most.max(frame[0].abs()));
                let alive = rack.slots()[0].trouble.is_none();
                if alive {
                    opened += 1;
                    println!("OK   {:<34} {:>5} out {top:.4}", one.name, one.format.label());
                } else {
                    refused += 1;
                    println!("FAIL {:<34} {:>5} {}", one.name, one.format.label(), rack.slots()[0].trouble.clone().unwrap_or_default());
                }
            }
            Err(why) => {
                refused += 1;
                println!("FAIL {:<34} {:>5} {why}", one.name, one.format.label());
            }
        }
    }
    println!("scanned {}, opened {opened}, refused {refused}", found.len());
}
