use std::time::Instant;

fn main() {
    let rate = 48_000.0;
    let seconds = 60.0;
    let frames = (rate * seconds) as usize;
    let mut audio: Vec<[f32; 2]> = (0..frames).map(|i| [((i * 7919) % 2000) as f32 / 1000.0 - 1.0; 2]).collect();
    for name in loupe_stock::NAMES {
        let mut effect = loupe_stock::make(name).unwrap();
        effect.prepare(rate);
        for index in 0..effect.params().len() {
            let wide = effect.params()[index];
            if wide.unit == loupe_stock::Unit::Switch {
                effect.set(index, 1.0);
            } else if wide.unit == loupe_stock::Unit::Decibels && wide.default == 0.0 {
                effect.set(index, 3.0);
            }
        }
        let began = Instant::now();
        for block in audio.chunks_mut(256) {
            effect.process(block);
        }
        let took = began.elapsed().as_secs_f64();
        println!("{name}: {:.0}x real time, {:.2}% of one core per instance", seconds as f64 / took, took / seconds as f64 * 100.0);
    }
}
