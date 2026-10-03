use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use crate::wire::{next_line, Ask, Reply};

const PATIENCE: Duration = Duration::from_secs(20);

pub struct Sandbox {
    child: Child,
    writing: ChildStdin,
    reading: BufReader<ChildStdout>,
}

impl Sandbox {
    pub fn start(host: &Path) -> Result<Self, String> {
        let mut child = Command::new(host)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|why| format!("the plugin host would not start: {why}"))?;
        let writing = child.stdin.take().ok_or("the plugin host has no way in")?;
        let reading = BufReader::new(child.stdout.take().ok_or("the plugin host has no way out")?);
        Ok(Self { child, writing, reading })
    }

    pub fn ask(&mut self, ask: Ask) -> Result<Reply, String> {
        ask.write(&mut self.writing).map_err(|_| "the plugin host stopped listening".to_string())?;
        let began = Instant::now();
        match next_line(&mut self.reading) {
            Some(line) => Reply::read(&line).ok_or_else(|| format!("the plugin host said {}", line.trim())),
            None if began.elapsed() >= PATIENCE => Err("the plugin host went quiet".into()),
            None => Err("the plugin crashed".into()),
        }
    }

    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = Ask::Quit.write(&mut self.writing);
        let _ = self.child.wait();
    }
}

pub fn host_beside_us() -> PathBuf {
    let name = if cfg!(windows) { "loupe-host.exe" } else { "loupe-host" };
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|folder| folder.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
}
