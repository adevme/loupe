use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

pub const NO_ROOM: &str =
    "This computer is nearly out of memory, so this plugin is not loaded. Press Load to start it anyway.";

const FEWEST_OPENING_AT_ONCE: usize = 2;
const MOST_OPENING_AT_ONCE: usize = 8;
const CEILING_VARIABLE: &str = "LOUPE_PLUGIN_CEILING";
const MEGABYTES_A_HOST_IS_PROMISED: u64 = 64;
const FEWEST_MEGABYTES_KEPT_FREE: u64 = 2_048;
const SHARE_OF_THE_LIMIT_KEPT_FREE: u64 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Memory {
    pub free_megabytes: u64,
    pub limit_megabytes: u64,
}

impl Memory {
    pub fn kept_free(&self) -> u64 {
        (self.limit_megabytes / SHARE_OF_THE_LIMIT_KEPT_FREE).max(FEWEST_MEGABYTES_KEPT_FREE)
    }

    fn has_room_for(&self, hosts: usize) -> bool {
        let wanted = self.kept_free().saturating_add(MEGABYTES_A_HOST_IS_PROMISED.saturating_mul(hosts as u64));
        self.free_megabytes >= wanted
    }
}

type Measure = fn() -> Option<Memory>;

pub struct Ceiling {
    live: AtomicUsize,
    unsettled: AtomicUsize,
    most: Option<usize>,
    memory: Option<Measure>,
    opening: AtomicUsize,
    most_opening: usize,
}

pub struct Seat {
    ceiling: Arc<Ceiling>,
    settled: bool,
}

impl Seat {
    pub fn settle(&mut self) {
        if !self.settled {
            self.settled = true;
            self.ceiling.unsettled.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

impl Drop for Seat {
    fn drop(&mut self) {
        self.settle();
        self.ceiling.live.fetch_sub(1, Ordering::AcqRel);
    }
}

pub struct Opening(Arc<Ceiling>);

impl Drop for Opening {
    fn drop(&mut self) {
        self.0.opening.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Ceiling {
    fn made(most: Option<usize>, memory: Option<Measure>) -> Arc<Self> {
        Arc::new(Self {
            live: AtomicUsize::new(0),
            unsettled: AtomicUsize::new(0),
            most: most.map(|most| most.max(1)),
            memory,
            opening: AtomicUsize::new(0),
            most_opening: opening_at_once(),
        })
    }

    pub fn of(most: usize) -> Arc<Self> {
        Self::made(Some(most), None)
    }

    pub fn measuring(memory: Measure) -> Arc<Self> {
        Self::made(None, Some(memory))
    }

    pub fn fresh_for_this_computer() -> Arc<Self> {
        Self::made(asked_for(), Some(memory_of_this_computer))
    }

    pub fn for_this_computer() -> Arc<Self> {
        static SHARED: OnceLock<Arc<Ceiling>> = OnceLock::new();
        Arc::clone(SHARED.get_or_init(Ceiling::fresh_for_this_computer))
    }

    pub fn most(&self) -> Option<usize> {
        self.most
    }

    pub fn running(&self) -> usize {
        self.live.load(Ordering::Acquire)
    }

    pub fn why_full(&self) -> String {
        match self.most {
            Some(most) if self.running() >= most => format!("{CEILING_VARIABLE} allows only {most}"),
            _ => match self.memory.and_then(|measure| measure()) {
                Some(memory) => format!(
                    "this computer has only {:.1} GB of memory left to give",
                    memory.free_megabytes.saturating_sub(memory.kept_free()) as f64 / 1024.0
                ),
                None => "this computer is nearly out of memory".into(),
            },
        }
    }

    fn there_is_room(&self, taken: usize) -> bool {
        if self.most.is_some_and(|most| taken >= most) {
            return false;
        }
        let Some(memory) = self.memory.and_then(|measure| measure()) else { return true };
        memory.has_room_for(self.unsettled.load(Ordering::Acquire) + 1)
    }

    fn seat(self: &Arc<Self>) -> Seat {
        self.unsettled.fetch_add(1, Ordering::AcqRel);
        Seat { ceiling: Arc::clone(self), settled: false }
    }

    pub fn take_a_seat(self: &Arc<Self>) -> Option<Seat> {
        let mut taken = self.live.load(Ordering::Acquire);
        loop {
            if !self.there_is_room(taken) {
                return None;
            }
            match self.live.compare_exchange_weak(taken, taken + 1, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Some(self.seat()),
                Err(now) => taken = now,
            }
        }
    }

    pub fn squeeze_in(self: &Arc<Self>) -> Seat {
        self.live.fetch_add(1, Ordering::AcqRel);
        self.seat()
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

fn asked_for() -> Option<usize> {
    let said = std::env::var_os(CEILING_VARIABLE)?;
    let text = said.to_str()?.trim().to_string();
    text.parse::<usize>().ok().map(|most| most.max(1))
}

#[cfg(not(windows))]
fn memory_of_this_computer() -> Option<Memory> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let read = |name: &str| -> Option<u64> {
        let line = text.lines().find(|line| line.starts_with(name))?;
        line[name.len()..].split_whitespace().next()?.parse::<u64>().ok()
    };
    let available = read("MemAvailable:")?;
    let total = read("MemTotal:")?;
    let swap_free = read("SwapFree:").unwrap_or(0);
    let swap_total = read("SwapTotal:").unwrap_or(0);
    Some(Memory { free_megabytes: (available + swap_free) / 1024, limit_megabytes: (total + swap_total) / 1024 })
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
fn memory_of_this_computer() -> Option<Memory> {
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
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        return None;
    }
    let megabytes = |bytes: u64| bytes / (1024 * 1024);
    Some(Memory { free_megabytes: megabytes(status.available_page_file), limit_megabytes: megabytes(status.total_page_file) })
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

    fn his_pc() -> Option<Memory> {
        Some(Memory { free_megabytes: 90_807, limit_megabytes: 136_790 })
    }

    #[test]
    fn his_pc_takes_every_plugin_of_his_song_and_more() {
        let ceiling = Ceiling::measuring(his_pc);
        let mut seats: Vec<Seat> = (0..538).map(|_| ceiling.take_a_seat().expect("538 hosts fit in 90 GB of free commit")).collect();
        assert_eq!(ceiling.running(), 538);
        for seat in seats.iter_mut() {
            seat.settle();
        }
        assert!(ceiling.take_a_seat().is_some(), "settled hosts are already counted in what the computer says is free");
    }

    #[test]
    fn hosts_still_opening_are_counted_against_the_free_memory() {
        let ceiling = Ceiling::measuring(his_pc);
        let room = his_pc().map(|memory| (memory.free_megabytes - memory.kept_free()) / MEGABYTES_A_HOST_IS_PROMISED).unwrap() as usize;
        let seats: Vec<Seat> = (0..room).map(|_| ceiling.take_a_seat().expect("there is memory for it")).collect();
        assert!(ceiling.take_a_seat().is_none(), "one more would eat the memory kept free for the computer");
        assert!(ceiling.why_full().contains("GB of memory"));
        drop(seats);
        assert!(ceiling.take_a_seat().is_some());
    }

    #[test]
    fn a_nearly_full_computer_says_no() {
        fn nearly_full() -> Option<Memory> {
            Some(Memory { free_megabytes: 2_000, limit_megabytes: 16_384 })
        }
        let ceiling = Ceiling::measuring(nearly_full);
        assert!(ceiling.take_a_seat().is_none());
        assert!(ceiling.squeeze_in().settled == false, "the user can still load one by hand");
    }

    #[test]
    fn the_variable_still_sets_a_count() {
        let ceiling = Ceiling::made(Some(1), Some(his_pc));
        let _first = ceiling.take_a_seat().expect("there is room");
        assert!(ceiling.take_a_seat().is_none());
        assert!(ceiling.why_full().contains(CEILING_VARIABLE));
    }

    #[test]
    fn this_computer_reads_its_memory() {
        let memory = memory_of_this_computer().expect("the computer says how much memory it has");
        assert!(memory.limit_megabytes > 0);
        assert!(memory.free_megabytes <= memory.limit_megabytes);
    }
}
