//! Mimics what Loupe does: reconcile a slot (which opens the plugin on a thread of
//! its own) and ask for its window straight away, before it has finished opening.
fn main() {
    let path = std::env::args().nth(1).expect("give a plugin path");
    let index: usize = std::env::args().nth(2).unwrap_or_else(|| "0".into()).parse().unwrap_or(0);
    let host = loupe_plugins::sandbox::host_beside_us();
    let mut rack = loupe_plugins::rack::Rack::new(host, 48_000, 512);
    let want = vec![loupe_plugins::rack::Wanted {
        path: std::path::PathBuf::from(&path),
        index,
        name: "test".into(),
        bypassed: false,
        state: Vec::new(),
        record: false,
    }];
    let troubles = rack.reconcile(&want);
    println!("reconcile said {troubles:?}, still opening: {}", rack.still_opening());
    match rack.show(0) {
        Ok(()) => println!("show accepted"),
        Err(why) => println!("show refused: {why}"),
    }
    let mut audio: Vec<[f32; 2]> = vec![[0.25, 0.25]; 512];
    for tick in 0..600 {
        rack.process(&mut audio);
        let troubles = rack.open_waiting();
        if !troubles.is_empty() {
            println!("tick {tick}: the window would not open: {troubles:?}");
        }
        if tick % 50 == 0 {
            println!("tick {tick}: still opening {}", rack.still_opening());
        }
        audio = vec![[0.25, 0.25]; 512];
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    println!("done, still opening: {}", rack.still_opening());
}
