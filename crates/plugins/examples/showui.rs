fn main() {
    let path = std::env::args().nth(1).expect("give a plugin path");
    let index: usize = std::env::args().nth(2).unwrap_or_else(|| "0".into()).parse().unwrap_or(0);
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
    match rack.show(0) {
        Ok(()) => println!("window open"),
        Err(why) => println!("trouble: {why}"),
    }
    let mut audio: Vec<[f32; 2]> = vec![[0.25, 0.25]; 512];
    for _ in 0..200 {
        let mut copy = audio.clone();
        rack.process(&mut copy);
        audio = vec![[0.25, 0.25]; 512];
        let _ = copy;
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    match rack.hide(0) {
        Ok(()) => println!("window hidden"),
        Err(why) => println!("trouble: {why}"),
    }
    println!("done");
}
