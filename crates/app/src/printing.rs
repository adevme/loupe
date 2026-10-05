use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use loupe_engine::{Fx, Source};
use loupe_plugins::rack::{Rack, Wanted};

const BLOCK: usize = 512;
const OPENS_WITHIN: Duration = Duration::from_secs(30);
const LOOKS_EVERY: Duration = Duration::from_millis(20);

pub fn wanted(fx: &[Fx]) -> Vec<Wanted> {
    fx.iter()
        .filter(|fx| fx.record)
        .map(|fx| Wanted {
            mix: 1.0,
            path: fx.path.clone(),
            index: fx.index,
            name: fx.name.clone(),
            bypassed: fx.bypassed,
            state: fx.state.clone(),
            record: true,
        })
        .collect()
}

pub fn printed_name(take: &Path) -> PathBuf {
    let stem = take.file_stem().map(|stem| stem.to_string_lossy().to_string()).unwrap_or_default();
    take.with_file_name(format!("{stem} with Rec plugins.wav"))
}

pub fn print_take(dry: &Source, take: &Path, want: &[Wanted], rate: u32, host: &Path) -> Result<Source, String> {
    let mut rack = Rack::with_room_for(host.to_path_buf(), rate, BLOCK, want.len().max(1));
    let troubles = rack.reconcile(want);
    if !troubles.is_empty() {
        return Err(troubles.join(", "));
    }
    let until = Instant::now() + OPENS_WITHIN;
    while !rack.ready() {
        if Instant::now() > until {
            return Err("the Rec plugins took too long to open".into());
        }
        std::thread::sleep(LOOKS_EVERY);
    }
    if let Some(slot) = rack.slots().iter().find(|slot| slot.trouble.is_some()) {
        return Err(format!("{}: {}", slot.name, slot.trouble.clone().unwrap_or_default()));
    }
    let delay = rack.takes_latency();
    let total = dry.frames.len() + delay;
    let mut out = Vec::with_capacity(total);
    let mut block = Vec::with_capacity(BLOCK);
    for start in (0..total).step_by(BLOCK) {
        block.clear();
        block.extend((start..(start + BLOCK).min(total)).map(|at| dry.frames.get(at).copied().unwrap_or([0.0; 2])));
        rack.process_takes(&mut block);
        out.extend_from_slice(&block);
    }
    out.drain(..delay.min(out.len()));
    out.truncate(dry.frames.len());
    let path = printed_name(take);
    loupe_engine::write_frames(&path, &out, rate).map_err(|why| format!("{}: {why}", path.display()))?;
    Source::load(&path, rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_take_is_printed_through_its_rec_plugins_beside_the_dry_one() {
        let folder = std::env::temp_dir().join(format!("loupe-print-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let take = folder.join("Vocal (take 1).wav");
        let loud = vec![[0.9, 0.9]; 48_000];
        loupe_engine::write_frames(&take, &loud, 48_000).unwrap();
        let dry = Source::load(&take, 48_000).unwrap();
        let fx = [
            Fx { path: PathBuf::from(loupe_plugins::BUILT_IN), index: 1, name: "Loupe Compressor".into(), bypassed: false, state: Vec::new(), record: true, mix: 1.0 },
            Fx { path: PathBuf::from(loupe_plugins::BUILT_IN), index: 2, name: "Loupe Limiter".into(), bypassed: false, state: Vec::new(), record: false, mix: 1.0 },
        ];
        let want = wanted(&fx);
        assert_eq!(want.len(), 1, "only the Rec plugins print");
        let printed = print_take(&dry, &take, &want, 48_000, Path::new("no-host-here")).unwrap();
        assert_eq!(printed.frames.len(), dry.frames.len());
        let loudest = printed.frames[24_000..].iter().fold(0.0f32, |top, frame| top.max(frame[0].abs()));
        assert!(loudest < 0.85, "the compressor left it at {loudest}");
        assert!(printed_name(&take).exists());
        assert!(take.exists(), "the dry take stays");
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
