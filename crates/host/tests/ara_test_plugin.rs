use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use loupe_plugins::ara_document::unpack;
use loupe_plugins::wire::{next_line, read_block, write_block, Ask, Region, Reply};

const RATE: u32 = 48_000;
const BLOCK: usize = 512;

struct Host {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl Host {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_loupe-host"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the plugin host starts");
        let input = child.stdin.take().expect("a way in");
        let output = BufReader::new(child.stdout.take().expect("a way out"));
        Self { child, input, output }
    }

    fn ask(&mut self, ask: Ask) -> Reply {
        ask.write(&mut self.input).expect("the host listens");
        let line = next_line(&mut self.output).expect("the host answers");
        Reply::read(&line).unwrap_or_else(|| panic!("the host said {line}"))
    }

    fn play(&mut self, at: i64) -> Vec<[f32; 2]> {
        Ask::ProcessAt(at).write(&mut self.input).expect("the host listens");
        write_block(&mut self.input, &[[0.25, 0.25]; BLOCK]).expect("the host takes audio");
        let mut audio = Vec::new();
        read_block(&mut self.output, &mut audio).expect("the host gives audio back");
        audio
    }

    fn finish(mut self) -> String {
        let _ = Ask::Quit.write(&mut self.input);
        let mut said = String::new();
        if let Some(mut errors) = self.child.stderr.take() {
            let _ = errors.read_to_string(&mut said);
        }
        let _ = self.child.wait();
        said
    }
}

fn plugin() -> PathBuf {
    PathBuf::from(std::env::var_os("LOUPE_ARA_TEST_PLUGIN").expect("set LOUPE_ARA_TEST_PLUGIN to ARATestPlugIn.vst3"))
}

fn sample_at(frame: usize, side: usize, seed: usize) -> i16 {
    let value = ((frame * 7 + seed * 1000) % 2000) as i16 - 1000;
    if side == 0 { value * 8 } else { -value * 8 }
}

fn write_wav(path: &Path, frames: usize, seed: usize) {
    let data = (frames * 4) as u32;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(RATE * 4).to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for frame in 0..frames {
        for side in 0..2 {
            bytes.extend_from_slice(&sample_at(frame, side, seed).to_le_bytes());
        }
    }
    std::fs::write(path, bytes).expect("the test file is written");
}

fn heard_from_file(audio: &[[f32; 2]], from: usize, seed: usize) -> bool {
    audio.iter().enumerate().all(|(at, frame)| {
        let left = sample_at(from + at, 0, seed) as f32 / 32_768.0;
        let right = sample_at(from + at, 1, seed) as f32 / 32_768.0;
        (frame[0] - left).abs() < 1e-4 && (frame[1] - right).abs() < 1e-4
    })
}

fn region(file: &Path, start: f64) -> Region {
    Region {
        file: file.to_string_lossy().into_owned(),
        name: "Vocal".into(),
        start,
        offset: 0.5,
        length: 1.0,
        stretch: 1.0,
        tempo: 120.0,
    }
}

fn load(host: &mut Host, wanted: Region) {
    assert_eq!(host.ask(Ask::Region(wanted)), Reply::Fine);
    let loaded = host.ask(Ask::Load { path: plugin().to_string_lossy().into_owned(), index: 0, rate: RATE, block: BLOCK });
    assert!(matches!(loaded, Reply::Loaded { ara: true, .. }), "the test plugin did not bind with ARA: {loaded:?}");
}

#[test]
#[ignore = "needs Celemony's ARATestPlugIn.vst3 built for this computer, named by LOUPE_ARA_TEST_PLUGIN"]
fn the_ara_test_plugin_plays_the_clip_where_it_sits_and_keeps_its_state() {
    let folder = std::env::temp_dir().join(format!("loupe-ara-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let first = folder.join("first take.wav");
    let second = folder.join("second take.wav");
    write_wav(&first, 2 * RATE as usize, 1);
    write_wav(&second, 2 * RATE as usize, 2);
    let half = RATE as usize / 2;

    let mut host = Host::start();
    load(&mut host, region(&first, 1.0));
    assert!(host.play(0).iter().all(|frame| *frame == [0.0, 0.0]), "it played before the clip starts");
    assert!(heard_from_file(&host.play(RATE as i64), half, 1), "the clip did not start from its offset");
    assert!(heard_from_file(&host.play(RATE as i64 + 1024), half + 1024, 1));

    assert_eq!(host.ask(Ask::Region(region(&first, 2.0))), Reply::Fine);
    assert!(host.play(RATE as i64).iter().all(|frame| *frame == [0.0, 0.0]), "the old place still plays");
    assert!(heard_from_file(&host.play(2 * RATE as i64), half, 1), "the moved clip does not play");

    let state = match host.ask(Ask::Save) {
        Reply::State(state) => state,
        other => panic!("no state came back: {other:?}"),
    };
    let unpacked = unpack(&state).expect("an ARA plugin's state carries its archive");
    assert!(!unpacked.archive_id.is_empty());
    assert!(unpacked.archive.windows(18).any(|part| part == b"loupe audio source"), "the archive does not name our audio source");

    assert_eq!(host.ask(Ask::Region(region(&second, 2.0))), Reply::Fine);
    assert!(heard_from_file(&host.play(2 * RATE as i64), half, 2), "the new take does not play");
    let said = host.finish();
    assert!(!said.contains("ARA assert"), "the plugin complained: {said}");

    let mut again = Host::start();
    load(&mut again, region(&first, 2.0));
    assert_eq!(again.ask(Ask::Restore(state.clone())), Reply::Fine);
    assert!(heard_from_file(&again.play(2 * RATE as i64), half, 1), "the reopened clip does not play");
    let saved_again = match again.ask(Ask::Save) {
        Reply::State(state) => state,
        other => panic!("no state came back: {other:?}"),
    };
    assert_eq!(unpack(&saved_again).map(|kept| kept.archive.to_vec()), Some(unpacked.archive.to_vec()));
    let said = again.finish();
    assert!(!said.contains("ARA assert"), "the plugin complained: {said}");

    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
#[ignore = "needs Celemony's ARATestPlugIn.vst3 built for this computer, named by LOUPE_ARA_TEST_PLUGIN"]
fn a_clip_rack_opens_the_ara_test_plugin_on_its_clip_and_follows_it() {
    use loupe_plugins::rack::{Rack, Wanted};
    let folder = std::env::temp_dir().join(format!("loupe-ara-rack-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let take = folder.join("take.wav");
    write_wav(&take, 2 * RATE as usize, 3);
    let half = RATE as usize / 2;
    let mut rack = Rack::new(PathBuf::from(env!("CARGO_BIN_EXE_loupe-host")), RATE, BLOCK);
    assert!(rack.follow_region(Some(region(&take, 1.0))).is_empty());
    let wanted = Wanted { path: plugin(), index: 0, name: "ARATestPlugIn".into(), bypassed: false, state: Vec::new(), record: false, mix: 1.0 };
    assert!(rack.reconcile(&[wanted]).is_empty());
    let began = std::time::Instant::now();
    while !rack.ready() {
        assert!(began.elapsed() < std::time::Duration::from_secs(20), "the plugin never opened");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(rack.slots()[0].ara, "the rack did not see an ARA plugin");
    assert!(rack.slots()[0].trouble.is_none(), "{:?}", rack.slots()[0].trouble);
    let mut audio = vec![[0.25, 0.25]; BLOCK];
    rack.process_at(&mut audio, RATE as i64);
    assert!(heard_from_file(&audio, half, 3), "the rack did not play the clip from its offset");
    assert!(rack.follow_region(Some(region(&take, 3.0))).is_empty());
    let mut audio = vec![[0.25, 0.25]; BLOCK];
    rack.process_at(&mut audio, 3 * RATE as i64);
    assert!(heard_from_file(&audio, half, 3), "the rack did not follow the moved clip");
    let state = rack.save(0).expect("the rack saves the plugin");
    assert!(unpack(&state).is_some());
    let _ = std::fs::remove_dir_all(&folder);
}
