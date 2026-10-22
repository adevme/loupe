use std::path::{Path, PathBuf};

use loupe_plugins::ceiling::Ceiling;
use loupe_plugins::chain::{on_its_own, Joining};

const RATE: u32 = 48_000;
const PREPARED: usize = 512;
const CALF: &str = "/usr/lib/lv2/calf.lv2";
const CALF_PLUGINS: [usize; 3] = [24, 16, 37];

fn host() -> Option<PathBuf> {
    let beside = std::env::current_exe().ok()?.parent()?.parent()?.join(format!("loupe-host{}", std::env::consts::EXE_SUFFIX));
    beside.is_file().then_some(beside)
}

fn calf() -> Option<&'static Path> {
    let folder = Path::new(CALF);
    folder.exists().then_some(folder)
}

fn order(at: usize) -> Joining {
    Joining { name: format!("Calf {at}"), path: PathBuf::from(CALF), index: at, rate: RATE, block: PREPARED, region: None, state: Vec::new() }
}

fn tone(frames: usize) -> Vec<[f32; 2]> {
    (0..frames)
        .map(|at| {
            let left = 0.4 * (std::f32::consts::TAU * 220.0 * at as f32 / RATE as f32).sin();
            let right = 0.3 * (std::f32::consts::TAU * 330.0 * at as f32 / RATE as f32).sin();
            [left, right]
        })
        .collect()
}

#[test]
fn a_block_longer_than_the_plugin_was_prepared_for_is_processed_all_the_way_through() {
    let (Some(host), Some(_)) = (host(), calf()) else { return };
    let feed = tone(PREPARED * 4);
    for plugin in CALF_PLUGINS {
        let mut whole = on_its_own(&host, Ceiling::of(64).squeeze_in(), &order(plugin)).expect("the plugin opens").host;
        let mut pieces = on_its_own(&host, Ceiling::of(64).squeeze_in(), &order(plugin)).expect("the plugin opens again").host;
        let mut at_once = feed.clone();
        whole.run(&mut at_once).expect("one long block runs");
        let mut in_parts = Vec::new();
        for part in feed.chunks(PREPARED) {
            let mut piece = part.to_vec();
            pieces.run(&mut piece).expect("a prepared block runs");
            in_parts.extend_from_slice(&piece);
        }
        let differs = |frames: std::ops::Range<usize>| {
            frames.map(|at| (at_once[at][0] - in_parts[at][0]).abs().max((at_once[at][1] - in_parts[at][1]).abs())).fold(0.0f32, f32::max)
        };
        assert!(in_parts != feed, "Calf {plugin} changed nothing, so it proves nothing");
        assert!(differs(0..PREPARED) < 1.0e-5, "Calf {plugin}: the first prepared block already differs by {}", differs(0..PREPARED));
        let after = differs(PREPARED..feed.len());
        assert!(after < 1.0e-5, "Calf {plugin}: past frame {PREPARED} the long block differs by {after}, the plugin let it through untouched");
    }
}
