use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};

use loupe_plugins::wire::FELL;

static SEAT_AND_ONE: AtomicUsize = AtomicUsize::new(0);
static TELLING: AtomicI32 = AtomicI32::new(-1);

pub fn working_on(seat: usize) {
    SEAT_AND_ONE.store(seat + 1, Ordering::Relaxed);
}

pub fn finished() {
    SEAT_AND_ONE.store(0, Ordering::Relaxed);
}

pub fn tell_on_a_crash(fd: i32) {
    TELLING.store(fd, Ordering::Relaxed);
    catch_the_fall();
}

fn name_the_seat() {
    let seat = SEAT_AND_ONE.load(Ordering::Relaxed);
    let fd = TELLING.load(Ordering::Relaxed);
    if seat == 0 || fd < 0 {
        return;
    }
    let mut line = [0u8; 32];
    let mut filled = 0;
    for byte in FELL.as_bytes().iter().chain(b"\t") {
        line[filled] = *byte;
        filled += 1;
    }
    let mut digits = [0u8; 20];
    let mut counted = 0;
    let mut left = seat - 1;
    loop {
        digits[counted] = b'0' + (left % 10) as u8;
        counted += 1;
        left /= 10;
        if left == 0 {
            break;
        }
    }
    while counted > 0 {
        counted -= 1;
        line[filled] = digits[counted];
        filled += 1;
    }
    line[filled] = b'\n';
    filled += 1;
    straight_out(fd, &line[..filled]);
}

const WHAT_A_CRASH_EXITS_WITH: i32 = 134;

#[cfg(unix)]
fn straight_out(fd: i32, bytes: &[u8]) {
    extern "C" {
        fn write(fd: i32, from: *const u8, how_many: usize) -> isize;
    }
    unsafe {
        write(fd, bytes.as_ptr(), bytes.len());
    }
}

#[cfg(unix)]
extern "C" fn fell(_signal: i32) {
    extern "C" {
        fn _exit(code: i32) -> !;
    }
    name_the_seat();
    unsafe { _exit(WHAT_A_CRASH_EXITS_WITH) }
}

#[cfg(unix)]
fn catch_the_fall() {
    extern "C" {
        fn signal(which: i32, handler: usize) -> usize;
    }
    const ILLEGAL: i32 = 4;
    const ABORT: i32 = 6;
    const ARITHMETIC: i32 = 8;
    const SEGMENT: i32 = 11;
    #[cfg(target_os = "macos")]
    const BUS: i32 = 10;
    #[cfg(not(target_os = "macos"))]
    const BUS: i32 = 7;
    for which in [ILLEGAL, ABORT, ARITHMETIC, SEGMENT, BUS] {
        unsafe {
            signal(which, fell as *const () as usize);
        }
    }
}

#[cfg(windows)]
fn straight_out(fd: i32, bytes: &[u8]) {
    extern "C" {
        #[link_name = "_write"]
        fn write(fd: i32, from: *const u8, how_many: u32) -> i32;
    }
    unsafe {
        write(fd, bytes.as_ptr(), bytes.len() as u32);
    }
}

#[cfg(windows)]
unsafe extern "system" fn fell(_what: *mut core::ffi::c_void) -> i32 {
    const RUN_THE_HANDLER: i32 = 1;
    name_the_seat();
    RUN_THE_HANDLER
}

#[cfg(windows)]
fn catch_the_fall() {
    #[link(name = "kernel32")]
    extern "system" {
        fn SetUnhandledExceptionFilter(filter: usize) -> usize;
    }
    unsafe {
        SetUnhandledExceptionFilter(fell as *const () as usize);
    }
}

#[cfg(not(any(unix, windows)))]
fn straight_out(_fd: i32, _bytes: &[u8]) {}

#[cfg(not(any(unix, windows)))]
fn catch_the_fall() {}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    const CRASH_AT: &str = "LOUPE_BLAME_TEST_SEAT";
    const THIS_TEST: &str = "blame::tests::a_plugin_that_falls_over_mid_block_names_its_seat";
    const STDOUT: i32 = 1;

    #[test]
    fn a_plugin_that_falls_over_mid_block_names_its_seat() {
        if let Ok(said) = std::env::var(CRASH_AT) {
            extern "C" {
                fn raise(what: i32) -> i32;
            }
            tell_on_a_crash(STDOUT);
            working_on(said.parse().expect("a seat number"));
            unsafe { raise(11) };
            panic!("the crash was not caught");
        }
        let again = std::process::Command::new(std::env::current_exe().expect("the test binary"))
            .args(["--exact", THIS_TEST, "--nocapture"])
            .env(CRASH_AT, "4")
            .output()
            .expect("a second run of the test binary");
        let said = String::from_utf8_lossy(&again.stdout);
        assert!(said.contains("fell\t4\n"), "a crash at seat 4 said {said:?}");
    }

    #[test]
    fn nothing_is_said_when_no_plugin_was_running() {
        finished();
        tell_on_a_crash(-1);
        name_the_seat();
    }
}
