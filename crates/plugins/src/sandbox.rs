use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::collections::VecDeque;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::ceiling::{Ceiling, Seat};
use crate::wire::{read_block_or_line, write_block, Ask, Came, Link, Reply};

pub static EXCHANGES: AtomicU64 = AtomicU64::new(0);
pub static EXCHANGE_NANOS: AtomicU64 = AtomicU64::new(0);
pub static EXCHANGE_SLOWEST_NANOS: AtomicU64 = AtomicU64::new(0);
pub static EXCHANGES_LATE: AtomicU64 = AtomicU64::new(0);

pub fn exchanges() -> (u64, u64, u64, u64) {
    (
        EXCHANGES.load(Ordering::Relaxed),
        EXCHANGE_NANOS.load(Ordering::Relaxed),
        EXCHANGE_SLOWEST_NANOS.load(Ordering::Relaxed),
        EXCHANGES_LATE.load(Ordering::Relaxed),
    )
}

const SPIN_FOR_A_REPLY: Duration = Duration::from_millis(2);
const PATIENCE: Duration = Duration::from_secs(20);
const BLOCK_PATIENCE: Duration = Duration::from_millis(2_000);
const SILENT_FOR_TOO_LONG: Duration = Duration::from_secs(20);
const FIRST_BLOCK_PATIENCE: Duration = Duration::from_secs(10);
pub const GOODBYE_PATIENCE: Duration = Duration::from_millis(500);
pub const BETWEEN_GOODBYE_LOOKS: Duration = Duration::from_millis(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ran {
    InTime,
    Late,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Want {
    Line,
    Block,
}

static A_SAVE_WAS_ASKED_FOR: AtomicBool = AtomicBool::new(false);
static AN_UNDO_WAS_ASKED_FOR: AtomicBool = AtomicBool::new(false);
static A_REDO_WAS_ASKED_FOR: AtomicBool = AtomicBool::new(false);

fn flag_for(chord: &Reply) -> Option<&'static AtomicBool> {
    match chord {
        Reply::SaveTheSong => Some(&A_SAVE_WAS_ASKED_FOR),
        Reply::UndoAnEdit => Some(&AN_UNDO_WAS_ASKED_FOR),
        Reply::RedoAnEdit => Some(&A_REDO_WAS_ASKED_FOR),
        _ => None,
    }
}

fn remember_the_chord(chord: &Reply) -> bool {
    match flag_for(chord) {
        Some(flag) => {
            flag.store(true, Ordering::Relaxed);
            true
        }
        None => false,
    }
}

pub fn asked_to_save_the_song() -> bool {
    A_SAVE_WAS_ASKED_FOR.swap(false, Ordering::Relaxed)
}

pub fn asked_to_undo_an_edit() -> bool {
    AN_UNDO_WAS_ASKED_FOR.swap(false, Ordering::Relaxed)
}

pub fn asked_to_redo_an_edit() -> bool {
    A_REDO_WAS_ASKED_FOR.swap(false, Ordering::Relaxed)
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
    outgoing: Vec<u8>,
    pending: Vec<u8>,
    gets: Receiver<Got>,
    moved: Receiver<(usize, usize)>,
    reader: Option<JoinHandle<()>>,
    lost: bool,
    ran: bool,
    owed: VecDeque<Want>,
    late_since: Option<Instant>,
    seat: Option<Seat>,
    fell_at: Option<usize>,
    said_bye: bool,
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
        let (tells, moved) = channel::<(usize, usize)>();
        let reader = std::thread::spawn(move || read_for(out, asked, sends, tells));
        Ok(Self {
            child,
            writing,
            wants: Some(wants),
            outgoing: Vec::with_capacity(1 << 16),
            pending: Vec::new(),
            gets,
            moved,
            reader: Some(reader),
            lost: false,
            ran: false,
            owed: VecDeque::new(),
            late_since: None,
            seat: Some(seat),
            fell_at: None,
            said_bye: false,
        })
    }

    pub fn tell(&mut self, ask: Ask) -> Result<(), String> {
        if self.lost {
            return Err("the plugin host is gone".into());
        }
        let _ = ask.write(&mut self.pending);
        Ok(())
    }

    fn write_with_pending(&mut self, ask: &Ask) -> std::io::Result<()> {
        self.outgoing.clear();
        self.outgoing.append(&mut self.pending);
        ask.write(&mut self.outgoing)?;
        self.writing.write_all(&self.outgoing)?;
        self.writing.flush()
    }

    pub fn ask(&mut self, ask: Ask) -> Result<Reply, String> {
        if self.lost {
            return Err("the plugin host is gone".into());
        }
        self.settle_what_is_owed()?;
        self.send_the_ask(&ask, Want::Line)?;
        match self.gets.recv_timeout(PATIENCE) {
            Ok(got) => {
                self.late_since = None;
                self.reply_from(got)
            }
            Err(RecvTimeoutError::Timeout) => Err(self.give_up("the plugin host went quiet")),
            Err(RecvTimeoutError::Disconnected) => Err(self.give_up("the plugin crashed")),
        }
    }

    pub fn ask_within(&mut self, ask: Ask, within: Duration) -> Result<Option<Reply>, String> {
        if self.lost {
            return Err("the plugin host is gone".into());
        }
        self.take_what_came_late()?;
        self.send_the_ask(&ask, Want::Line)?;
        if !self.owed.is_empty() {
            self.owed.push_back(Want::Line);
            self.fell_behind()?;
            return Ok(None);
        }
        match self.gets.recv_timeout(within) {
            Ok(got) => {
                self.late_since = None;
                self.reply_from(got).map(Some)
            }
            Err(RecvTimeoutError::Timeout) => {
                self.owed.push_back(Want::Line);
                self.fell_behind()?;
                Ok(None)
            }
            Err(RecvTimeoutError::Disconnected) => Err(self.give_up("the plugin crashed")),
        }
    }

    fn send_the_ask(&mut self, ask: &Ask, want: Want) -> Result<(), String> {
        if self.write_with_pending(ask).is_err() {
            return Err(self.give_up("the plugin host stopped listening"));
        }
        if self.wants.as_ref().is_none_or(|w| w.send(want).is_err()) {
            return Err(self.give_up("the plugin host stopped listening"));
        }
        Ok(())
    }

    fn reply_from(&mut self, got: Got) -> Result<Reply, String> {
        match got {
            Got::Line(line) => match Reply::read(&line) {
                Some(Reply::Fell(seat)) => Err(self.name_the_fall(seat)),
                Some(reply) => Ok(reply),
                None => Err(format!("the plugin host said {}", line.trim())),
            },
            Got::Block(_) => Err(self.give_up("the plugin host answered out of turn")),
            Got::Gone => Err(self.give_up("the plugin crashed")),
        }
    }

    pub fn patience(&self) -> Duration {
        if self.ran {
            BLOCK_PATIENCE
        } else {
            FIRST_BLOCK_PATIENCE
        }
    }

    pub fn run(&mut self, audio: &mut Vec<[f32; 2]>) -> Result<(), String> {
        self.run_with(audio, &[])
    }

    pub fn run_with(&mut self, audio: &mut Vec<[f32; 2]>, side: &[[f32; 2]]) -> Result<(), String> {
        let within = self.patience();
        self.run_within(audio, side, None, within).map(|_| ())
    }

    pub fn run_at(&mut self, audio: &mut Vec<[f32; 2]>, at: i64) -> Result<(), String> {
        let within = self.patience();
        self.run_within(audio, &[], Some(at), within).map(|_| ())
    }

    pub fn run_within(&mut self, audio: &mut Vec<[f32; 2]>, side: &[[f32; 2]], at: Option<i64>, within: Duration) -> Result<Ran, String> {
        let ask = match at {
            Some(at) => Ask::ProcessAt(at),
            None if side.is_empty() => Ask::Process,
            None => Ask::ProcessWithSide,
        };
        let side = if matches!(ask, Ask::ProcessAt(_)) { &[] } else { side };
        self.exchange(ask, audio, side, within)
    }

    pub fn run_chain(&mut self, links: &[Link], audio: &mut Vec<[f32; 2]>, side: &[[f32; 2]]) -> Result<(), String> {
        if links.is_empty() {
            return Ok(());
        }
        let ask = Ask::Chain { side: !side.is_empty(), links: links.to_vec() };
        let within = self.patience();
        self.exchange(ask, audio, side, within).map(|_| ())
    }

    fn exchange(&mut self, ask: Ask, audio: &mut Vec<[f32; 2]>, side: &[[f32; 2]], within: Duration) -> Result<Ran, String> {
        let clock = Instant::now();
        let went = self.exchange_now(ask, audio, side, within);
        let took = clock.elapsed().as_nanos() as u64;
        EXCHANGES.fetch_add(1, Ordering::Relaxed);
        EXCHANGE_NANOS.fetch_add(took, Ordering::Relaxed);
        EXCHANGE_SLOWEST_NANOS.fetch_max(took, Ordering::Relaxed);
        if matches!(went, Ok(Ran::Late)) {
            EXCHANGES_LATE.fetch_add(1, Ordering::Relaxed);
        }
        went
    }

    fn exchange_now(&mut self, ask: Ask, audio: &mut Vec<[f32; 2]>, side: &[[f32; 2]], within: Duration) -> Result<Ran, String> {
        if self.lost {
            return Err("the plugin host is gone".into());
        }
        self.take_what_came_late()?;
        if !self.owed.is_empty() {
            self.fell_behind()?;
            return Ok(Ran::Late);
        }
        self.outgoing.clear();
        self.outgoing.append(&mut self.pending);
        let _ = ask.write(&mut self.outgoing);
        let _ = write_block(&mut self.outgoing, audio);
        if !side.is_empty() {
            let _ = write_block(&mut self.outgoing, side);
        }
        if self.writing.write_all(&self.outgoing).is_err() || self.writing.flush().is_err() {
            return Err(self.give_up("the plugin host stopped listening"));
        }
        if self.wants.as_ref().is_none_or(|w| w.send(Want::Block).is_err()) {
            return Err(self.give_up("the plugin host stopped listening"));
        }
        self.ran = true;
        match self.reply_within(within) {
            Ok(Got::Block(came)) => {
                self.late_since = None;
                audio.clear();
                audio.extend_from_slice(&came);
                Ok(Ran::InTime)
            }
            Ok(Got::Line(line)) => Err(self.took_it_badly(&line)),
            Ok(Got::Gone) => Err(self.give_up("the plugin crashed")),
            Err(RecvTimeoutError::Timeout) => {
                self.owed.push_back(Want::Block);
                self.fell_behind()?;
                Ok(Ran::Late)
            }
            Err(RecvTimeoutError::Disconnected) => Err(self.give_up("the plugin crashed")),
        }
    }

    fn reply_within(&mut self, within: Duration) -> Result<Got, RecvTimeoutError> {
        let began = Instant::now();
        while began.elapsed() < SPIN_FOR_A_REPLY.min(within) {
            match self.gets.try_recv() {
                Ok(got) => return Ok(got),
                Err(TryRecvError::Disconnected) => return Err(RecvTimeoutError::Disconnected),
                Err(TryRecvError::Empty) => std::hint::spin_loop(),
            }
        }
        self.gets.recv_timeout(within.saturating_sub(began.elapsed()))
    }

    fn take_what_came_late(&mut self) -> Result<(), String> {
        while let Some(want) = self.owed.front().copied() {
            match self.gets.try_recv() {
                Ok(got) => {
                    self.owed.pop_front();
                    self.late_since = None;
                    self.check_what_came_late(want, got)?;
                }
                Err(TryRecvError::Empty) => return Ok(()),
                Err(TryRecvError::Disconnected) => return Err(self.give_up("the plugin crashed")),
            }
        }
        Ok(())
    }

    fn settle_what_is_owed(&mut self) -> Result<(), String> {
        while let Some(want) = self.owed.pop_front() {
            match self.gets.recv_timeout(PATIENCE) {
                Ok(got) => {
                    self.late_since = None;
                    self.check_what_came_late(want, got)?;
                }
                Err(RecvTimeoutError::Timeout) => return Err(self.give_up("the plugin host went quiet")),
                Err(RecvTimeoutError::Disconnected) => return Err(self.give_up("the plugin crashed")),
            }
        }
        Ok(())
    }

    fn check_what_came_late(&mut self, want: Want, got: Got) -> Result<(), String> {
        match (want, got) {
            (Want::Block, Got::Block(_)) => Ok(()),
            (Want::Block, Got::Line(line)) => Err(self.took_it_badly(&line)),
            (Want::Line, got @ (Got::Line(_) | Got::Block(_))) => self.reply_from(got).map(|_| ()),
            (_, Got::Gone) => Err(self.give_up("the plugin crashed")),
        }
    }

    pub fn owes(&self) -> usize {
        self.owed.len()
    }

    fn fell_behind(&mut self) -> Result<(), String> {
        let since = *self.late_since.get_or_insert_with(Instant::now);
        if since.elapsed() >= SILENT_FOR_TOO_LONG {
            return Err(self.give_up("the plugin stopped answering, so the song carried on without it"));
        }
        Ok(())
    }

    fn took_it_badly(&mut self, line: &str) -> String {
        match Reply::read(line) {
            Some(Reply::Fell(seat)) => self.name_the_fall(seat),
            _ => self.give_up(&format!("the plugin host said {}", line.trim())),
        }
    }

    fn name_the_fall(&mut self, seat: usize) -> String {
        self.fell_at = Some(seat);
        self.give_up("the plugin crashed")
    }

    pub fn fell_at(&self) -> Option<usize> {
        self.fell_at
    }

    pub fn latency_moved(&self) -> Option<(usize, usize)> {
        self.moved.try_recv().ok()
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

fn read_for(out: impl Read, _asked: Receiver<Want>, sends: Sender<Got>, tells: Sender<(usize, usize)>) {
    let mut reading = BufReader::new(out);
    let mut audio = Vec::new();
    loop {
        let got = loop {
            let came = match read_block_or_line(&mut reading, &mut audio) {
                Ok(Came::Block) => Got::Block(std::mem::take(&mut audio)),
                Ok(Came::Line(line)) => Got::Line(line),
                Err(_) => Got::Gone,
            };
            if let Got::Line(line) = &came {
                match Reply::read(line) {
                    Some(Reply::LatencyMoved { seat, frames }) => {
                        let _ = tells.send((seat, frames));
                        continue;
                    }
                    Some(said) if remember_the_chord(&said) => continue,
                    _ => {}
                }
            }
            break came;
        };
        let gone = matches!(got, Got::Gone);
        if sends.send(got).is_err() || gone {
            return;
        }
    }
}

impl Sandbox {
    pub fn say_bye(&mut self) {
        if self.lost || self.said_bye {
            return;
        }
        let _ = self.write_with_pending(&Ask::Quit);
        self.said_bye = true;
        self.seat.take();
    }

    pub fn has_gone(&mut self) -> bool {
        self.lost || matches!(self.child.try_wait(), Ok(Some(_)))
    }

    pub fn make_it_go(&mut self) {
        if !self.has_gone() {
            let _ = self.give_up("the plugin host would not quit when it was asked");
        }
    }
}

pub fn see_them_out(mut hosts: Vec<&mut Sandbox>) {
    for host in hosts.iter_mut() {
        host.say_bye();
    }
    let since = Instant::now();
    while !hosts.iter_mut().all(|host| host.has_gone()) {
        if since.elapsed() >= GOODBYE_PATIENCE {
            break;
        }
        std::thread::sleep(BETWEEN_GOODBYE_LOOKS);
    }
    for host in hosts.iter_mut() {
        host.make_it_go();
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        if !self.lost {
            self.say_bye();
            let since = Instant::now();
            while !self.has_gone() {
                if since.elapsed() >= GOODBYE_PATIENCE {
                    self.make_it_go();
                    break;
                }
                std::thread::sleep(BETWEEN_GOODBYE_LOOKS);
            }
        }
        self.wants.take();
        self.reader.take();
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

#[cfg(test)]
mod chord_tests {
    use super::*;

    const CHORDS: [Reply; 3] = [Reply::SaveTheSong, Reply::UndoAnEdit, Reply::RedoAnEdit];

    fn one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
        static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let taken = TURN.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        nothing_is_waiting();
        taken
    }

    fn nothing_is_waiting() {
        let _ = asked_to_save_the_song();
        let _ = asked_to_undo_an_edit();
        let _ = asked_to_redo_an_edit();
    }

    fn waiting_now() -> (bool, bool, bool) {
        (asked_to_save_the_song(), asked_to_undo_an_edit(), asked_to_redo_an_edit())
    }

    #[test]
    fn each_chord_lands_on_its_own_and_nothing_else() {
        let _turn = one_at_a_time();
        for chord in CHORDS {
            let wanted = match chord {
                Reply::SaveTheSong => (true, false, false),
                Reply::UndoAnEdit => (false, true, false),
                _ => (false, false, true),
            };
            assert!(remember_the_chord(&chord));
            assert_eq!(waiting_now(), wanted, "{chord:?} woke the wrong one");
        }
    }

    #[test]
    fn two_hosts_asking_at_once_only_save_the_song_once() {
        let _turn = one_at_a_time();
        let both: Vec<_> =
            (0..2).map(|_| std::thread::spawn(|| remember_the_chord(&Reply::SaveTheSong))).collect();
        for one in both {
            assert!(one.join().expect("both hosts asked"));
        }
        assert!(asked_to_save_the_song(), "the first look should take the ask");
        assert!(!asked_to_save_the_song(), "the second look should find nothing left");
    }

    #[test]
    fn two_hosts_asking_at_once_only_undo_one_edit() {
        let _turn = one_at_a_time();
        let both: Vec<_> =
            (0..2).map(|_| std::thread::spawn(|| remember_the_chord(&Reply::UndoAnEdit))).collect();
        for one in both {
            assert!(one.join().expect("both hosts asked"));
        }
        assert!(asked_to_undo_an_edit(), "the first look should take the ask");
        assert!(!asked_to_undo_an_edit(), "the second look should find nothing left");
    }

    #[test]
    fn an_ordinary_reply_is_not_a_chord() {
        let _turn = one_at_a_time();
        for reply in [Reply::Fine, Reply::Edited(true), Reply::Fell(2), Reply::Value(0.5)] {
            assert!(!remember_the_chord(&reply), "{reply:?} was taken for a chord");
        }
        assert_eq!(waiting_now(), (false, false, false));
    }

    #[test]
    fn nothing_is_waiting_until_a_host_asks() {
        let _turn = one_at_a_time();
        assert_eq!(waiting_now(), (false, false, false));
        assert!(remember_the_chord(&Reply::RedoAnEdit));
        assert_eq!(waiting_now(), (false, false, true));
    }

    #[test]
    fn a_chord_does_not_stand_in_for_the_answer() {
        let _turn = one_at_a_time();
        let said = b"savesong\nundoedit\nsavesong\nedited\t1\n".to_vec();
        let (wants, asked) = channel::<Want>();
        let (sends, gets) = channel::<Got>();
        let (tells, _moved) = channel::<(usize, usize)>();
        let reader = std::thread::spawn(move || read_for(&said[..], asked, sends, tells));
        wants.send(Want::Line).expect("the reader is listening");
        let Ok(Got::Line(line)) = gets.recv_timeout(PATIENCE) else { panic!("the answer never came back") };
        assert_eq!(Reply::read(&line), Some(Reply::Edited(true)));
        assert_eq!(waiting_now(), (true, true, false));
        drop(wants);
        reader.join().expect("the reader finished");
    }

    #[test]
    fn a_chord_does_not_stand_in_for_a_block() {
        let _turn = one_at_a_time();
        let mut said = Vec::new();
        Reply::UndoAnEdit.write(&mut said).expect("the chord is written");
        write_block(&mut said, &[[0.25f32, -0.25]; 4]).expect("the block is written");
        let (wants, asked) = channel::<Want>();
        let (sends, gets) = channel::<Got>();
        let (tells, _moved) = channel::<(usize, usize)>();
        let reader = std::thread::spawn(move || read_for(&said[..], asked, sends, tells));
        wants.send(Want::Block).expect("the reader is listening");
        let Ok(Got::Block(came)) = gets.recv_timeout(PATIENCE) else { panic!("the block never came back") };
        assert_eq!(came, vec![[0.25f32, -0.25]; 4]);
        assert_eq!(waiting_now(), (false, true, false));
        drop(wants);
        reader.join().expect("the reader finished");
    }
}

#[cfg(test)]
mod latency_tests {
    use super::*;

    #[test]
    fn a_plugin_that_moved_its_latency_does_not_stand_in_for_a_block() {
        let mut said = Vec::new();
        Reply::LatencyMoved { seat: 0, frames: 1_024 }.write(&mut said).expect("the word is written");
        write_block(&mut said, &[[0.5f32, -0.5]; 4]).expect("the block is written");
        let (wants, asked) = channel::<Want>();
        let (sends, gets) = channel::<Got>();
        let (tells, moved) = channel::<(usize, usize)>();
        let reader = std::thread::spawn(move || read_for(&said[..], asked, sends, tells));
        wants.send(Want::Block).expect("the reader is listening");
        let Ok(Got::Block(came)) = gets.recv_timeout(PATIENCE) else { panic!("the block never came back") };
        assert_eq!(came, vec![[0.5f32, -0.5]; 4]);
        assert_eq!(moved.try_recv(), Ok((0, 1_024)));
        drop(wants);
        reader.join().expect("the reader finished");
    }

    #[test]
    fn a_plugin_that_moved_its_latency_does_not_stand_in_for_the_answer() {
        let said = b"latencymoved\t2\t64\nlatencymoved\t2\t512\nfine\n".to_vec();
        let (wants, asked) = channel::<Want>();
        let (sends, gets) = channel::<Got>();
        let (tells, moved) = channel::<(usize, usize)>();
        let reader = std::thread::spawn(move || read_for(&said[..], asked, sends, tells));
        wants.send(Want::Line).expect("the reader is listening");
        let Ok(Got::Line(line)) = gets.recv_timeout(PATIENCE) else { panic!("the answer never came back") };
        assert_eq!(Reply::read(&line), Some(Reply::Fine));
        assert_eq!(moved.try_recv(), Ok((2, 64)));
        assert_eq!(moved.try_recv(), Ok((2, 512)));
        assert!(moved.try_recv().is_err(), "nothing else was said");
        drop(wants);
        reader.join().expect("the reader finished");
    }
}
