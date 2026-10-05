use std::collections::HashMap;
use std::time::{Duration, Instant};

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

const LOOK_EVERY: Duration = Duration::from_secs(1);
const MEGABYTE: u64 = 1024 * 1024;

pub struct Resources {
    system: System,
    me: Option<Pid>,
    cores: f32,
    looked: Option<Instant>,
    pub cpu: Option<f32>,
    pub memory: Option<u64>,
    each: HashMap<u32, Usage>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    pub cpu: Option<f32>,
    pub memory: u64,
}

impl Resources {
    pub fn new() -> Self {
        let cores = std::thread::available_parallelism().map(|count| count.get()).unwrap_or(1) as f32;
        Self { system: System::new(), me: sysinfo::get_current_pid().ok(), cores, looked: None, cpu: None, memory: None, each: HashMap::new() }
    }

    pub fn look(&mut self, hosts: &[u32]) {
        if self.looked.is_some_and(|at| at.elapsed() < LOOK_EVERY) {
            return;
        }
        let first = self.looked.is_none();
        self.looked = Some(Instant::now());
        let Some(me) = self.me else { return };
        let mine: Vec<Pid> = std::iter::once(me).chain(hosts.iter().map(|pid| Pid::from_u32(*pid))).collect();
        self.system.refresh_processes_specifics(ProcessesToUpdate::Some(&mine), true, ProcessRefreshKind::nothing().with_cpu().with_memory());
        let cores = self.cores;
        self.each = self
            .system
            .processes()
            .values()
            .filter(|process| process.thread_kind().is_none())
            .map(|process| (process.pid().as_u32(), Usage { cpu: (!first).then(|| (process.cpu_usage() / cores).clamp(0.0, 100.0)), memory: process.memory() }))
            .collect();
        let (cpu, memory) = self.each.values().fold((0.0f32, 0u64), |(cpu, memory), usage| (cpu + usage.cpu.unwrap_or(0.0), memory + usage.memory));
        self.memory = Some(memory);
        self.cpu = (!first).then(|| cpu.clamp(0.0, 100.0));
    }

    pub fn of(&self, pid: u32) -> Option<Usage> {
        self.each.get(&pid).copied()
    }

    pub fn own(&self) -> Option<Usage> {
        self.of(self.me?.as_u32())
    }

    pub fn summary(&self) -> String {
        let cpu = self.cpu.map_or("CPU ...".to_string(), |cpu| format!("CPU {cpu:.0}%"));
        let memory = self.memory.map_or("RAM ...".to_string(), memory_text);
        format!("{cpu}   {memory}")
    }
}

pub fn memory_text(bytes: u64) -> String {
    let megabytes = bytes / MEGABYTE;
    if megabytes >= 1024 {
        format!("RAM {:.1} GB", megabytes as f64 / 1024.0)
    } else {
        format!("RAM {megabytes} MB")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_reads_in_megabytes_then_gigabytes() {
        assert_eq!(memory_text(412 * MEGABYTE), "RAM 412 MB");
        assert_eq!(memory_text(1536 * MEGABYTE), "RAM 1.5 GB");
    }

    #[test]
    fn this_process_is_measured_after_a_second_look() {
        let mut found = Resources::new();
        found.look(&[]);
        assert!(found.memory.is_some_and(|bytes| bytes > 0));
        assert_eq!(found.cpu, None, "the first look has nothing to compare with");
        std::thread::sleep(LOOK_EVERY + Duration::from_millis(50));
        found.look(&[]);
        assert!(found.cpu.is_some_and(|cpu| (0.0..=100.0).contains(&cpu)));
    }
}
