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
}

impl Resources {
    pub fn new() -> Self {
        let cores = std::thread::available_parallelism().map(|count| count.get()).unwrap_or(1) as f32;
        Self { system: System::new(), me: sysinfo::get_current_pid().ok(), cores, looked: None, cpu: None, memory: None }
    }

    pub fn look(&mut self) {
        if self.looked.is_some_and(|at| at.elapsed() < LOOK_EVERY) {
            return;
        }
        let first = self.looked.is_none();
        self.looked = Some(Instant::now());
        let Some(me) = self.me else { return };
        self.system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing().with_cpu().with_memory());
        let ours = self.system.processes().values().filter(|process| process.thread_kind().is_none() && (process.pid() == me || process.parent() == Some(me)));
        let (cpu, memory) = ours.fold((0.0f32, 0u64), |(cpu, memory), process| (cpu + process.cpu_usage(), memory + process.memory()));
        self.memory = Some(memory);
        self.cpu = (!first).then(|| (cpu / self.cores).clamp(0.0, 100.0));
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
        found.look();
        assert!(found.memory.is_some_and(|bytes| bytes > 0));
        assert_eq!(found.cpu, None, "the first look has nothing to compare with");
        std::thread::sleep(LOOK_EVERY + Duration::from_millis(50));
        found.look();
        assert!(found.cpu.is_some_and(|cpu| (0.0..=100.0).contains(&cpu)));
    }
}
