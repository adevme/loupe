use std::io::BufReader;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use loupe_plugins::wire::{next_line, Ask, Reply};

#[test]
fn the_window_questions_are_answered_at_once_without_waiting_on_the_window() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_loupe-host"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("the plugin host starts");
    let mut input = child.stdin.take().expect("a way in");
    let mut output = BufReader::new(child.stdout.take().expect("a way out"));
    let began = Instant::now();
    for ask in [Ask::OnTrack("Vox".into(), 3), Ask::Called("Auto-Tune".into()), Ask::Show] {
        ask.write(&mut input).expect("the host listens");
    }
    let answers: Vec<Option<Reply>> = (0..3).map(|_| next_line(&mut output).and_then(|line| Reply::read(&line))).collect();
    let took = began.elapsed();
    let _ = Ask::Quit.write(&mut input);
    let _ = child.wait();
    assert_eq!(
        answers,
        vec![Some(Reply::Fine), Some(Reply::Fine), Some(Reply::Trouble("only VST3 plugins have a window so far".into()))]
    );
    assert!(took < Duration::from_secs(2), "the window questions took {took:?}");
}
