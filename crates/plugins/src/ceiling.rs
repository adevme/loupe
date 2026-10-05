use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

pub const NO_ROOM: &str =
    "Loupe is already running as many plugins as this computer can manage, so this one is not loaded. Press Load to start it anyway.";

const FEWEST_HOSTS: usize = 32;
const MOST_HOSTS: usize = 256;
const HOSTS_A_CORE_CARRIES: usize = 12;
const MEGABYTES_A_HOST_TAKES: usize = 64;
const FEWEST_OPENING_AT_ONCE: usize = 2;
const MOST_OPENING_AT_ONCE: usize = 8;
const CEILING_VARIABLE: &str = "LOUPE_PLUGIN_CEILING";

pub struct Ceiling {
    live: AtomicUsize,
    most: usize,
    opening: AtomicUsize,
    most_opening: usize,
}

pub struct Seat(Arc<Ceiling>);

impl Drop for Seat {
    fn drop(&mut self) {
        self.0.live.fetch_sub(1, Ordering::AcqRel);
    }
}

pub struct Opening(Arc<Ceiling>);

impl Drop for Opening {
    fn drop(&mut self) {
        self.0.opening.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Ceiling {
    pub fn of(most: usize) -> Arc<Self> {
        Arc::new(Self {
            live: AtomicUsize::new(0),
            most: most.max(1),
            opening: AtomicUsize::new(0),
            most_opening: opening_at_once(),
        })
    }

    pub fn for_this_computer() -> Arc<Self> {
        static SHARED: OnceLock<Arc<Ceiling>> = OnceLock::new();
        Arc::clone(SHARED.get_or_init(|| Ceiling::of(what_this_computer_can_carry())))
    }

    pub fn most(&self) -> usize {
        self.most
    }

    pub fn running(&self) -> usize {
        self.live.load(Ordering::Acquire)
    }

    pub fn take_a_seat(self: &Arc<Self>) -> Option<Seat> {
        let mut taken = self.live.load(Ordering::Acquire);
        loop {
            if taken >= self.most {
                return None;
            }
            match self.live.compare_exchange_weak(taken, taken + 1, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Some(Seat(Arc::clone(self))),
                Err(now) => taken = now,
            }
        }
    }

    pub fn squeeze_in(self: &Arc<Self>) -> Seat {
        self.live.fetch_add(1, Ordering::AcqRel);
        Seat(Arc::clone(self))
    }

    pub fn may_start_opening(self: &Arc<Self>) -> Option<Opening> {
        let mut busy = self.opening.load(Ordering::Acquire);
        loop {
            if busy >= self.most_opening {
                return None;
            }
            match self.opening.compare_exchange_weak(busy, busy + 1, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Some(Opening(Arc::clone(self))),
                Err(now) => busy = now,
            }
        }
    }
}

fn opening_at_once() -> usize {
    std::thread::available_parallelism()
        .map_or(FEWEST_OPENING_AT_ONCE, |cores| cores.get())
        .clamp(FEWEST_OPENING_AT_ONCE, MOST_OPENING_AT_ONCE)
}

fn what_this_computer_can_carry() -> usize {
    if let Some(said) = asked_for() {
        return said;
    }
    let cores = std::thread::available_parallelism().map_or(4, |how_many| how_many.get());
    let by_cores = cores.saturating_mul(HOSTS_A_CORE_CARRIES);
    let by_memory = (spare_megabytes() / MEGABYTES_A_HOST_TAKES).max(1);
    by_cores.min(by_memory).clamp(FEWEST_HOSTS, MOST_HOSTS)
}

fn asked_for() -> Option<usize> {
    let said = std::env::var_os(CEILING_VARIABLE)?;
    let text = said.to_str()?.trim().to_string();
    text.parse::<usize>().ok().map(|most| most.max(1))
}

#[cfg(not(windows))]
fn spare_megabytes() -> usize {
    let enough_for_the_floor = FEWEST_HOSTS * MEGABYTES_A_HOST_TAKES;
    let Ok(text) = std::fs::read_to_string("/proc/meminfo") else {
        return enough_for_the_floor;
    };
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("MemAvailable:") else { continue };
        if let Some(kilobytes) = rest.split_whitespace().next().and_then(|number| number.parse::<usize>().ok()) {
            return kilobytes / 1024;
        }
    }
    enough_for_the_floor
}

#[cfg(windows)]
#[repr(C)]
struct MemoryStatus {
    length: u32,
    load: u32,
    total_physical: u64,
    available_physical: u64,
    total_page_file: u64,
    available_page_file: u64,
    total_virtual: u64,
    available_virtual: u64,
    available_extended_virtual: u64,
}

#[cfg(windows)]
fn spare_megabytes() -> usize {
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalMemoryStatusEx(status: *mut MemoryStatus) -> i32;
    }
    let mut status = MemoryStatus {
        length: std::mem::size_of::<MemoryStatus>() as u32,
        load: 0,
        total_physical: 0,
        available_physical: 0,
        total_page_file: 0,
        available_page_file: 0,
        total_virtual: 0,
        available_virtual: 0,
        available_extended_virtual: 0,
    };
    let told = unsafe { GlobalMemoryStatusEx(&mut status) };
    if told == 0 {
        return FEWEST_HOSTS * MEGABYTES_A_HOST_TAKES;
    }
    (status.available_physical / (1024 * 1024)) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seats_run_out_and_come_back() {
        let ceiling = Ceiling::of(2);
        let first = ceiling.take_a_seat().expect("there is room");
        let second = ceiling.take_a_seat().expect("there is room");
        assert!(ceiling.take_a_seat().is_none(), "the third one does not fit");
        assert_eq!(ceiling.running(), 2);
        drop(first);
        assert!(ceiling.take_a_seat().is_some(), "a seat came back");
        drop(second);
        assert_eq!(ceiling.running(), 0);
    }

    #[test]
    fn a_plugin_the_user_asks_for_is_let_in_past_the_ceiling() {
        let ceiling = Ceiling::of(1);
        let _first = ceiling.take_a_seat().expect("there is room");
        assert!(ceiling.take_a_seat().is_none());
        let squeezed = ceiling.squeeze_in();
        assert_eq!(ceiling.running(), 2);
        drop(squeezed);
        assert_eq!(ceiling.running(), 1);
    }

    #[test]
    fn this_computer_has_a_sensible_ceiling() {
        let ceiling = Ceiling::for_this_computer();
        assert!(ceiling.most() >= FEWEST_HOSTS);
        assert!(ceiling.most() <= MOST_HOSTS);
    }
}
