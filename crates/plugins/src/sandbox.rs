use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::wire::{next_line, read_block, write_block, Ask, Reply};

const PATIENCE: Duration = Duration::from_secs(20);
const BLOCK_PATIENCE: Duration = Duration::from_millis(500);
const FIRST_BLOCK_PATIENCE: Duration = Duration::from_secs(10);

enum Want {
    Line,
    Block,
}

enum Got {
    Line(String),
    Block(Vec<[f32; 2]>),
    Gone,
}

pub struct Sandbox {
    child: Child,
    writing: ChildStdin,
    wants: Option<Sender<Want>>,
    gets: Receiver<Got>,
    reader: Option<JoinHandle<()>>,
    lost: bool,
    ran: bool,
}

impl Sandbox {
    pub fn start(host: &Path) -> Result<Self, String> {
        if !host.is_file() {
            return Err(format!("Loupe cannot find its plugin host at {}. Install Loupe again to put it back.", host.display()));
        }
        let mut command = Command::new(host);
        if let Some(root) = crate::presets::root() {
            command.env(crate::presets::FOLDER_VARIABLE, root);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(if std::env::var_os("LOUPE_HOST_NOISE").is_some() { Stdio::inherit() } else { Stdio::null() })
            .spawn()
            .map_err(|why| format!("the plugin host would not start: {why}"))?;
        let writing = child.stdin.take().ok_or("the plugin host has no way in")?;
        let out = child.stdout.take().ok_or("the plugin host has no way out")?;
        let (wants, asked) = channel::<Want>();
        let (sends, gets) = channel::<Got>();
        let reader = std::thread::spawn(move || read_for(out, asked, sends));
        Ok(Self { child, writing, wants: Some(wants), gets, reader: Some(reader), lost: false, ran: false })
    }

    pub fn ask(&mut self, ask: Ask) -> Result<Reply, String> {
        if self.lost {
            return Err("the plugin host is gone".into());
        }
        if ask.write(&mut self.writing).is_err() {
            return Err(self.give_up("the plugin host stopped listening"));
        }
        if self.wants.as_ref().is_none_or(|w| w.send(Want::Line).is_err()) {
            return Err(self.give_up("the plugin host stopped listening"));
        }
        match self.gets.recv_timeout(PATIENCE) {
            Ok(Got::Line(line)) => Reply::read(&line).ok_or_else(|| format!("the plugin host said {}", line.trim())),
            Ok(Got::Block(_)) => Err(self.give_up("the plugin host answered out of turn")),
            Ok(Got::Gone) => Err(self.give_up("the plugin crashed")),
            Err(RecvTimeoutError::Timeout) => Err(self.give_up("the plugin host went quiet")),
            Err(RecvTimeoutError::Disconnected) => Err(self.give_up("the plugin crashed")),
        }
    }

    pub fn run(&mut self, audio: &mut Vec<[f32; 2]>) -> Result<(), String> {
        self.run_with(audio, &[])
    }

    pub fn run_with(&mut self, audio: &mut Vec<[f32; 2]>, side: &[[f32; 2]]) -> Result<(), String> {
        if self.lost {
            return Err("the plugin host is gone".into());
        }
        let ask = if side.is_empty() { Ask::Process } else { Ask::ProcessWithSide };
        if ask.write(&mut self.writing).is_err() || write_block(&mut self.writing, audio).is_err() {
            return Err(self.give_up("the plugin host stopped listening"));
        }
        if !side.is_empty() && write_block(&mut self.writing, side).is_err() {
            return Err(self.give_up("the plugin host stopped listening"));
        }
        if self.wants.as_ref().is_none_or(|w| w.send(Want::Block).is_err()) {
            return Err(self.give_up("the plugin host stopped listening"));
        }
        let waiting = if self.ran { BLOCK_PATIENCE } else { FIRST_BLOCK_PATIENCE };
        self.ran = true;
        match self.gets.recv_timeout(waiting) {
            Ok(Got::Block(came)) => {
                audio.clear();
                audio.extend_from_slice(&came);
                Ok(())
            }
            Ok(Got::Line(line)) => Err(self.give_up(&format!("the plugin host said {}", line.trim()))),
            Ok(Got::Gone) => Err(self.give_up("the plugin crashed")),
            Err(RecvTimeoutError::Timeout) => Err(self.give_up("the plugin took too long on a block")),
            Err(RecvTimeoutError::Disconnected) => Err(self.give_up("the plugin crashed")),
        }
    }

    pub fn alive(&mut self) -> bool {
        !self.lost && matches!(self.child.try_wait(), Ok(None))
    }

    fn give_up(&mut self, why: &str) -> String {
        self.lost = true;
        let _ = self.child.kill();
        let _ = self.child.wait();
        why.to_string()
    }
}

fn read_for(out: ChildStdout, asked: Receiver<Want>, sends: Sender<Got>) {
    let mut reading = BufReader::new(out);
    let mut audio = Vec::new();
    while let Ok(want) = asked.recv() {
        let got = match want {
            Want::Line => match next_line(&mut reading) {
                Some(line) => Got::Line(line),
                None => Got::Gone,
            },
            Want::Block => match read_block(&mut reading, &mut audio) {
                Ok(()) => Got::Block(std::mem::take(&mut audio)),
                Err(_) => Got::Gone,
            },
        };
        let gone = matches!(got, Got::Gone);
        if sends.send(got).is_err() || gone {
            return;
        }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        if !self.lost {
            let _ = Ask::Quit.write(&mut self.writing);
            let since = Instant::now();
            loop {
                if matches!(self.child.try_wait(), Ok(Some(_))) {
                    break;
                }
                if since.elapsed() >= Duration::from_millis(500) {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        self.wants.take();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

pub fn host_beside_us() -> PathBuf {
    let name = if cfg!(windows) { "loupe-host.exe" } else { "loupe-host" };
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|folder| folder.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
}
