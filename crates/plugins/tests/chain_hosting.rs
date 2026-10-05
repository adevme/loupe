use std::path::{Path, PathBuf};
use std::time::Instant;

use loupe_plugins::blend::Dry;
use loupe_plugins::ceiling::Ceiling;
use loupe_plugins::chain::{into_the_chain, on_its_own, out_of_the_chain, Alone, Chain, Joining};
use loupe_plugins::sandbox::Sandbox;
use loupe_plugins::wire::{Ask, Link, Reply};

const RATE: u32 = 48_000;
const BLOCK: usize = 512;
const CALF: &str = "/usr/lib/lv2/calf.lv2";

fn host() -> Option<PathBuf> {
    let beside = std::env::current_exe().ok()?.parent()?.parent()?.join(format!("loupe-host{}", std::env::consts::EXE_SUFFIX));
    beside.is_file().then_some(beside)
}

fn calf() -> Option<&'static Path> {
    let folder = Path::new(CALF);
    folder.exists().then_some(folder)
}

fn plugins() -> Vec<(usize, f32)> {
    vec![(24, 1.0), (16, 1.0), (37, 1.0), (12, 1.0), (13, 0.35)]
}

fn order(at: usize) -> Joining {
    Joining { name: format!("Calf {at}"), path: PathBuf::from(CALF), index: at, rate: RATE, block: BLOCK, region: None, state: Vec::new() }
}

fn room() -> loupe_plugins::ceiling::Seat {
    Ceiling::of(4_096).squeeze_in()
}

fn noise(frames: usize) -> Vec<[f32; 2]> {
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    (0..frames)
        .map(|_| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            let one = ((seed >> 40) as i32 - 8_388_608 / 32) as f32 / 8_388_608.0;
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            let two = ((seed >> 40) as i32 - 8_388_608 / 32) as f32 / 8_388_608.0;
            [one.clamp(-1.0, 1.0) * 0.5, two.clamp(-1.0, 1.0) * 0.5]
        })
        .collect()
}

struct OnePerPlugin {
    hosts: Vec<(Sandbox, f32, usize, Dry)>,
}

impl OnePerPlugin {
    fn start(host: &Path) -> Option<Self> {
        let mut hosts = Vec::new();
        for (at, mix) in plugins() {
            let Alone { host: sandbox, latency, .. } = on_its_own(host, room(), &order(at)).ok()?;
            hosts.push((sandbox, mix, latency, Dry::empty()));
        }
        Some(Self { hosts })
    }

    fn run(&mut self, audio: &mut Vec<[f32; 2]>) -> Result<(), String> {
        for (sandbox, mix, latency, dry) in self.hosts.iter_mut() {
            let blending = *mix < 1.0;
            if blending {
                dry.room_for(*latency, audio.len());
                dry.remember(audio);
            }
            sandbox.run(audio)?;
            if blending {
                dry.blend(audio, *mix);
            }
        }
        Ok(())
    }
}

type Chained = (Chain, Vec<Link>);

fn whole_chain(host: &Path) -> Option<Chained> {
    let mut chain = Chain::start(host, room()).ok()?;
    let mut order_of = Vec::new();
    for (at, mix) in plugins() {
        let joined = chain.join(&order(at)).ok()?;
        order_of.push(Link { seat: joined.seat, mix });
    }
    Some((chain, order_of))
}

#[test]
fn a_whole_chain_in_one_process_sounds_like_one_process_each() {
    let (Some(host), Some(_)) = (host(), calf()) else { return };
    let Some(mut apart) = OnePerPlugin::start(&host) else { panic!("the plugins would not open on their own") };
    let Some((mut together, order)) = whole_chain(&host) else { panic!("the chain host would not load the plugins") };
    assert_eq!(together.running(), plugins().len(), "every plugin took a seat");
    let feed = noise(BLOCK * 8);
    let mut one = Vec::new();
    let mut other = Vec::new();
    for lot in feed.chunks(BLOCK) {
        let mut through_many = lot.to_vec();
        apart.run(&mut through_many).expect("the separate hosts ran");
        let mut through_one = lot.to_vec();
        together.run(&order, &mut through_one, &[]).expect("the chain host ran");
        one.extend_from_slice(&through_many);
        other.extend_from_slice(&through_one);
    }
    assert_eq!(one.len(), other.len());
    let worst = one.iter().zip(&other).fold(0.0f32, |most, (a, b)| most.max((a[0] - b[0]).abs()).max((a[1] - b[1]).abs()));
    assert!(worst < 1.0e-5, "the chain host differed by {worst}");
    let loudest = other.iter().fold(0.0f32, |most, frame| most.max(frame[0].abs()));
    assert!(loudest > 0.0, "the chain gave back silence");
}

#[test]
fn a_plugin_keeps_its_settings_when_it_moves_out_of_the_chain_and_back() {
    let (Some(host), Some(_)) = (host(), calf()) else { return };
    let Some((mut chain, _)) = whole_chain(&host) else { panic!("the chain host would not load the plugins") };
    let moving = 1;
    let knobs = match chain.ask_of(moving, Ask::Knobs) {
        Ok(Reply::Knobs(names)) => names,
        other => panic!("the chain host would not list the knobs: {other:?}"),
    };
    let knob = knobs.len() / 2;
    assert!(chain.ask_of(moving, Ask::Turn { knob, value: 0.25 }).is_ok(), "the knob turns in the chain");
    let before = chain.save(moving).expect("the chain host saves its settings");
    let alone = out_of_the_chain(&mut chain, moving, &host, room(), &order(16)).expect("it moved into its own process");
    assert_eq!(chain.running(), plugins().len() - 1, "the chain gave up its seat");
    let mut solo = alone;
    let on_its_own_state = match solo.host.ask(Ask::Save) {
        Ok(Reply::State(state)) => state,
        other => panic!("the lone host would not save: {other:?}"),
    };
    assert_eq!(on_its_own_state, before, "moving out of the chain kept the settings");
    let joined = into_the_chain(&mut chain, solo, &order(16)).expect("it moved back into the chain");
    assert_eq!(chain.running(), plugins().len(), "it took a seat again");
    assert_eq!(chain.save(joined.seat).expect("it saves again"), before, "moving back kept the settings");
}

#[test]
fn a_bypassed_plugin_is_simply_left_out_of_the_order() {
    let (Some(host), Some(_)) = (host(), calf()) else { return };
    let Some((mut chain, order)) = whole_chain(&host) else { panic!("the chain host would not load the plugins") };
    let feed = noise(BLOCK);
    let mut everything = feed.clone();
    chain.run(&order, &mut everything, &[]).expect("the chain ran");
    let fewer: Vec<Link> = order.iter().copied().filter(|link| link.seat != 0).collect();
    let mut without = feed.clone();
    chain.run(&fewer, &mut without, &[]).expect("the shorter chain ran");
    assert_ne!(everything, without, "leaving a plugin out changed the sound");
    let mut none = feed.clone();
    chain.run(&[], &mut none, &[]).expect("an empty order is no trouble");
    assert_eq!(none, feed, "an empty order leaves the audio alone");
}

const HOW_MANY_TRACKS: &str = "LOUPE_CHAIN_TRACKS";
const HOW_LONG: &str = "LOUPE_CHAIN_SECONDS";

fn asked(name: &str, instead: usize) -> usize {
    std::env::var(name).ok().and_then(|said| said.trim().parse().ok()).unwrap_or(instead)
}

fn hands() -> usize {
    std::thread::available_parallelism().map_or(4, |cores| cores.get() * 2)
}

fn spread<Work: Fn(usize) + Send + Sync>(how_many: usize, work: Work) {
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..hands().min(how_many) {
            let next = &next;
            let work = &work;
            scope.spawn(move || loop {
                let mine = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if mine >= how_many {
                    return;
                }
                work(mine);
            });
        }
    });
}

#[test]
#[ignore]
fn how_much_quicker_a_chain_host_is() {
    let (Some(host), Some(_)) = (host(), calf()) else {
        println!("no loupe-host or no Calf plugins here, so nothing was measured");
        return;
    };
    let tracks = asked(HOW_MANY_TRACKS, 50);
    let seconds = asked(HOW_LONG, 10);
    let blocks = RATE as usize * seconds / BLOCK;
    let feed = noise(BLOCK);
    println!("{tracks} tracks of {} plugins, {blocks} blocks of {BLOCK} frames each", plugins().len());

    let began = Instant::now();
    let apart: Vec<std::sync::Mutex<Option<OnePerPlugin>>> = (0..tracks).map(|_| std::sync::Mutex::new(None)).collect();
    spread(tracks, |which| {
        let made = OnePerPlugin::start(&host);
        *apart[which].lock().expect("the slot is free") = made;
    });
    let opened = began.elapsed();
    let running = apart.iter().filter(|held| held.lock().is_ok_and(|held| held.is_some())).count();
    println!("one host a plugin: {running} tracks opened in {opened:?}, {} processes", running * plugins().len());
    let began = Instant::now();
    spread(tracks, |which| {
        let Ok(mut held) = apart[which].lock() else { return };
        let Some(one) = held.as_mut() else { return };
        let mut audio = feed.clone();
        for _ in 0..blocks {
            if one.run(&mut audio).is_err() {
                return;
            }
        }
    });
    let one_each = began.elapsed();
    println!("one host a plugin: {one_each:?}");
    drop(apart);

    let began = Instant::now();
    let shared: Vec<std::sync::Mutex<Option<Chained>>> = (0..tracks).map(|_| std::sync::Mutex::new(None)).collect();
    spread(tracks, |which| {
        let made = whole_chain(&host);
        *shared[which].lock().expect("the slot is free") = made;
    });
    let opened = began.elapsed();
    let running = shared.iter().filter(|held| held.lock().is_ok_and(|held| held.is_some())).count();
    println!("one host a chain: {running} tracks opened in {opened:?}, {running} processes");
    let began = Instant::now();
    spread(tracks, |which| {
        let Ok(mut held) = shared[which].lock() else { return };
        let Some((chain, order)) = held.as_mut() else { return };
        let mut audio = feed.clone();
        for _ in 0..blocks {
            if chain.run(order, &mut audio, &[]).is_err() {
                return;
            }
        }
    });
    let together = began.elapsed();
    println!("one host a chain: {together:?}");
    println!("the chain host took {:.1}% of the time", together.as_secs_f64() / one_each.as_secs_f64() * 100.0);
}
