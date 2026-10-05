use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::ceiling::{Ceiling, Seat};
use crate::wire::{next_line, read_block, write_block, Ask, Reply};

const PATIENCE: Duration = Duration::from_secs(20);
const BLOCK_PATIENCE: Duration = Duration::from_millis(2_000);
const CATCHING_UP: Duration = Duration::from_millis(1);
const LATE_BLOCKS_ALLOWED: u16 = 200;
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
    late: u16,
    owed: bool,
    seat: Option<Seat>,
}

impl Sandbox {
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn start(host: &Path) -> Result<Self, String> {
        Self::start_in_a_seat(host, Ceiling::for_this_computer().squeeze_in())
    }

    pub fn start_in_a_seat(host: &Path, seat: Seat) -> Result<Self, String> {
        if !host.is_file() {
            return Err(format!("Loupe cannot find its plugin host at {}. Install Loupe again to put it back.", host.display()));
        }
        let mut command = Command::new(host);
        if let Some(root) = crate::presets::root() {
            command.env(crate::presets::FOLDER_VARIABLE, root);
        }
        if let Some(chrome) = crate::chrome::worn() {
            command.env(crate::chrome::CHROME_VARIABLE, chrome.to_text());
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(NO_WINDOW);
        }
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(if std::env::var_os("LOUPE_HOST_NOISE").is_some() { Stdio::inherit() } else { Stdio::null() });
        let mut child = start_so_it_dies_with_us(command).map_err(|why| format!("the plugin host would not start: {why}"))?;
        let writing = child.stdin.take().ok_or("the plugin host has no way in")?;
        let out = child.stdout.take().ok_or("the plugin host has no way out")?;
        let (wants, asked) = channel::<Want>();
        let (sends, gets) = channel::<Got>();
        let reader = std::thread::spawn(move || read_for(out, asked, sends));
        Ok(Self { child, writing, wants: Some(wants), gets, reader: Some(reader), lost: false, ran: false, late: 0, owed: false, seat: Some(seat) })
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
        let ask = if side.is_empty() { Ask::Process } else { Ask::ProcessWithSide };
        self.exchange(ask, audio, side)
    }

    pub fn run_at(&mut self, audio: &mut Vec<[f32; 2]>, at: i64) -> Result<(), String> {
        self.exchange(Ask::ProcessAt(at), audio, &[])
    }

    fn exchange(&mut self, ask: Ask, audio: &mut Vec<[f32; 2]>, side: &[[f32; 2]]) -> Result<(), String> {
        if self.lost {
            return Err("the plugin host is gone".into());
        }
        if self.owed {
            return self.catch_up();
        }
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
                self.late = 0;
                audio.clear();
                audio.extend_from_slice(&came);
                Ok(())
            }
            Ok(Got::Line(line)) => Err(self.give_up(&format!("the plugin host said {}", line.trim()))),
            Ok(Got::Gone) => Err(self.give_up("the plugin crashed")),
            Err(RecvTimeoutError::Timeout) => {
                self.owed = true;
                self.fell_behind()
            }
            Err(RecvTimeoutError::Disconnected) => Err(self.give_up("the plugin crashed")),
        }
    }

    fn catch_up(&mut self) -> Result<(), String> {
        match self.gets.recv_timeout(CATCHING_UP) {
            Ok(Got::Block(_)) => {
                self.owed = false;
                self.late = 0;
                Ok(())
            }
            Ok(Got::Line(line)) => Err(self.give_up(&format!("the plugin host said {}", line.trim()))),
            Ok(Got::Gone) => Err(self.give_up("the plugin crashed")),
            Err(RecvTimeoutError::Timeout) => self.fell_behind(),
            Err(RecvTimeoutError::Disconnected) => Err(self.give_up("the plugin crashed")),
        }
    }

    fn fell_behind(&mut self) -> Result<(), String> {
        self.late = self.late.saturating_add(1);
        if self.late >= LATE_BLOCKS_ALLOWED {
            return Err(self.give_up("the plugin stopped answering, so the song carried on without it"));
        }
        Ok(())
    }

    pub fn gone(&self) -> bool {
        self.lost
    }

    pub fn may_come_forward(&self) {
        #[cfg(windows)]
        {
            #[link(name = "user32")]
            extern "system" {
                fn AllowSetForegroundWindow(process: u32) -> i32;
            }
            unsafe {
                AllowSetForegroundWindow(self.child.id());
            }
        }
    }

    pub fn alive(&mut self) -> bool {
        !self.lost && matches!(self.child.try_wait(), Ok(None))
    }

    fn give_up(&mut self, why: &str) -> String {
        self.lost = true;
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.seat.take();
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

#[cfg(unix)]
struct Hatch {
    command: Command,
    back: Sender<std::io::Result<Child>>,
}

#[cfg(unix)]
fn the_only_thread_that_lives_as_long_as_loupe() -> &'static std::sync::Mutex<Sender<Hatch>> {
    static HATCHERY: std::sync::OnceLock<std::sync::Mutex<Sender<Hatch>>> = std::sync::OnceLock::new();
    HATCHERY.get_or_init(|| {
        let (asks, taken) = channel::<Hatch>();
        std::thread::spawn(move || {
            while let Ok(mut hatch) = taken.recv() {
                let started = hatch.command.spawn();
                let _ = hatch.back.send(started);
            }
        });
        std::sync::Mutex::new(asks)
    })
}

#[cfg(unix)]
fn start_so_it_dies_with_us(mut command: Command) -> std::io::Result<Child> {
    let gone = || std::io::Error::other("Loupe's plugin starter has stopped");
    tie_to_this_thread(&mut command);
    let (back, answer) = channel();
    {
        let asks = the_only_thread_that_lives_as_long_as_loupe().lock().map_err(|_| gone())?;
        asks.send(Hatch { command, back }).map_err(|_| gone())?;
    }
    answer.recv().unwrap_or_else(|_| Err(gone()))
}

#[cfg(all(unix, target_os = "linux"))]
fn tie_to_this_thread(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    const SET_PARENT_DEATH_SIGNAL: i32 = 1;
    const KILL: u64 = 9;
    extern "C" {
        fn prctl(option: i32, second: u64, third: u64, fourth: u64, fifth: u64) -> i32;
        fn getppid() -> i32;
        fn _exit(code: i32) -> !;
    }
    let loupe = std::process::id() as i32;
    unsafe {
        command.pre_exec(move || {
            prctl(SET_PARENT_DEATH_SIGNAL, KILL, 0, 0, 0);
            if getppid() != loupe {
                _exit(0);
            }
            Ok(())
        });
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
fn tie_to_this_thread(command: &mut Command) {
    let _ = command;
}

#[cfg(windows)]
#[repr(C)]
struct JobBasicLimits {
    per_process_user_time: i64,
    per_job_user_time: i64,
    limit_flags: u32,
    minimum_working_set: usize,
    maximum_working_set: usize,
    most_processes: u32,
    affinity: usize,
    priority_class: u32,
    scheduling_class: u32,
}

#[cfg(windows)]
#[repr(C)]
struct JobCounters {
    reads: u64,
    writes: u64,
    others: u64,
    bytes_read: u64,
    bytes_written: u64,
    bytes_other: u64,
}

#[cfg(windows)]
#[repr(C)]
struct JobLimits {
    basic: JobBasicLimits,
    counters: JobCounters,
    process_memory_limit: usize,
    job_memory_limit: usize,
    peak_process_memory: usize,
    peak_job_memory: usize,
}

#[cfg(windows)]
fn the_job_loupe_closes_when_it_goes() -> Option<usize> {
    use std::ffi::c_void;
    const KILL_ON_JOB_CLOSE: u32 = 0x2000;
    const EXTENDED_LIMITS: i32 = 9;
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(attributes: *mut c_void, name: *const u16) -> *mut c_void;
        fn SetInformationJobObject(job: *mut c_void, class: i32, info: *mut c_void, length: u32) -> i32;
    }
    static JOB: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    *JOB.get_or_init(|| unsafe {
        let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
        if job.is_null() {
            return None;
        }
        let mut limits: JobLimits = std::mem::zeroed();
        limits.basic.limit_flags = KILL_ON_JOB_CLOSE;
        let told = SetInformationJobObject(
            job,
            EXTENDED_LIMITS,
            &mut limits as *mut JobLimits as *mut c_void,
            std::mem::size_of::<JobLimits>() as u32,
        );
        if told == 0 {
            return None;
        }
        Some(job as usize)
    })
}

#[cfg(windows)]
fn start_so_it_dies_with_us(mut command: Command) -> std::io::Result<Child> {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    extern "system" {
        fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
    }
    let child = command.spawn()?;
    if let Some(job) = the_job_loupe_closes_when_it_goes() {
        unsafe {
            AssignProcessToJobObject(job as *mut c_void, child.as_raw_handle() as *mut c_void);
        }
    }
    Ok(child)
}

#[cfg(not(any(unix, windows)))]
fn start_so_it_dies_with_us(mut command: Command) -> std::io::Result<Child> {
    command.spawn()
}

pub fn host_beside_us() -> PathBuf {
    let name = if cfg!(windows) { "loupe-host.exe" } else { "loupe-host" };
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|folder| folder.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
}

#[cfg(all(test, target_os = "linux"))]
mod orphan_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const SCRIPT_VARIABLE: &str = "LOUPE_ORPHAN_TEST_HOST";
    const PID_FILE_VARIABLE: &str = "LOUPE_ORPHAN_TEST_PID_FILE";
    const WAITING_TEST: &str = "sandbox::orphan_tests::a_host_waits_to_be_orphaned";

    fn still_about(pid: u32) -> bool {
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        let after_the_name = stat.rsplit(')').next().unwrap_or_default();
        after_the_name.split_whitespace().next() != Some("Z")
    }

    #[test]
    fn a_host_waits_to_be_orphaned() {
        let Some(script) = std::env::var_os(SCRIPT_VARIABLE) else { return };
        let kept = Sandbox::start(Path::new(&script)).expect("the fake host starts");
        std::thread::sleep(Duration::from_secs(120));
        drop(kept);
    }

    #[test]
    fn a_host_does_not_outlive_loupe_being_killed() {
        let folder = std::env::temp_dir().join(format!("loupe-orphan-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("a folder to work in");
        let script = folder.join("loupe-host");
        let pid_file = folder.join("host.pid");
        std::fs::write(&script, "#!/bin/sh\nprintf '%s\\n' \"$$\" > \"$LOUPE_ORPHAN_TEST_PID_FILE\"\nwhile : ; do sleep 1 ; done\n")
            .expect("the fake host is written");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("the fake host can run");
        let mut loupe = Command::new(std::env::current_exe().expect("the test binary"))
            .args(["--exact", WAITING_TEST, "--nocapture"])
            .env(SCRIPT_VARIABLE, &script)
            .env(PID_FILE_VARIABLE, &pid_file)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("a second Loupe starts");
        let gave_up = Instant::now();
        let host = loop {
            let said = std::fs::read_to_string(&pid_file).unwrap_or_default();
            if let Ok(pid) = said.trim().parse::<u32>() {
                break pid;
            }
            if gave_up.elapsed() > Duration::from_secs(30) {
                let _ = loupe.kill();
                let _ = std::fs::remove_dir_all(&folder);
                panic!("the fake host never said what it was");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(still_about(host), "the fake host should be running before Loupe is killed");
        loupe.kill().expect("Loupe is killed outright");
        loupe.wait().expect("Loupe is gone");
        let gave_up = Instant::now();
        while still_about(host) {
            if gave_up.elapsed() > Duration::from_secs(15) {
                let _ = std::fs::remove_dir_all(&folder);
                panic!("the plugin host outlived Loupe as an orphan");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = std::fs::remove_dir_all(&folder);
    }
}
