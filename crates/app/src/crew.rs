//! A crew of worker threads that stays up between blocks, so a wave of independent tracks can
//! run its plugin chains side by side on the audio thread without spawning a thread per block.
//!
//! The caller hands over a batch of jobs and does not return until every one is done, which is
//! what lets the jobs borrow the racks and audio for the duration: the borrows outlive the run
//! the same way they would inside `std::thread::scope`, only the threads are already there.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// How long a worker keeps looking for the next wave before it sleeps: the waves of one block
/// come a few hundred microseconds apart, the next block comes milliseconds later.
const STAYS_KEEN_FOR: Duration = Duration::from_micros(800);

pub struct Job {
    run: *mut (dyn FnMut() + Send),
}

// A job is a pointer to a closure the caller still owns; the caller blocks until it has run.
unsafe impl Send for Job {}

struct Shared {
    queue: Mutex<Vec<Job>>,
    done: AtomicUsize,
    quit: AtomicBool,
    sleeping: Mutex<()>,
    wake: Condvar,
}

pub struct Crew {
    shared: Arc<Shared>,
}

impl Crew {
    pub fn of(hands: usize) -> Self {
        let hands = hands.max(1);
        let shared = Arc::new(Shared {
            queue: Mutex::new(Vec::with_capacity(64)),
            done: AtomicUsize::new(0),
            quit: AtomicBool::new(false),
            sleeping: Mutex::new(()),
            wake: Condvar::new(),
        });
        for which in 0..hands {
            let theirs = shared.clone();
            let _ = std::thread::Builder::new().name(format!("loupe-crew-{which}")).spawn(move || work(theirs));
        }
        Self { shared }
    }

    /// Runs every closure, on the crew and on this thread, and returns once all have run.
    pub fn run_all<'a>(&self, jobs: Vec<Box<dyn FnMut() + Send + 'a>>) {
        let count = jobs.len();
        if count == 0 {
            return;
        }
        let mut boxes: Vec<Box<dyn FnMut() + Send + 'a>> = jobs;
        self.shared.done.store(0, Ordering::Release);
        {
            let mut queue = lock(&self.shared.queue);
            for job in boxes.iter_mut() {
                let run: *mut (dyn FnMut() + Send + 'a) = &mut **job;
                // The lifetime is erased here and restored by the wait below: nothing in
                // `boxes` is dropped until `done` says every job has run.
                let run: *mut (dyn FnMut() + Send) = unsafe { std::mem::transmute(run) };
                queue.push(Job { run });
            }
        }
        self.shared.wake.notify_all();
        loop {
            let next = lock(&self.shared.queue).pop();
            let Some(job) = next else { break };
            run_counted(job, &self.shared);
        }
        while self.shared.done.load(Ordering::Acquire) < count {
            std::hint::spin_loop();
        }
        drop(boxes);
    }
}

impl Drop for Crew {
    fn drop(&mut self) {
        self.shared.quit.store(true, Ordering::Release);
        self.shared.wake.notify_all();
    }
}

/// A job that panics still counts as done, so the caller waiting on the batch never hangs.
fn run_counted(job: Job, shared: &Shared) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe { (*job.run)() }));
    shared.done.fetch_add(1, Ordering::AcqRel);
}

fn lock<T>(held: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    held.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn work(shared: Arc<Shared>) {
    hurry_this_thread();
    let mut keen_since = Instant::now();
    loop {
        if shared.quit.load(Ordering::Acquire) {
            return;
        }
        let next = lock(&shared.queue).pop();
        if let Some(job) = next {
            run_counted(job, &shared);
            keen_since = Instant::now();
            continue;
        }
        if keen_since.elapsed() < STAYS_KEEN_FOR {
            std::hint::spin_loop();
            continue;
        }
        let guard = lock(&shared.sleeping);
        if lock(&shared.queue).is_empty() && !shared.quit.load(Ordering::Acquire) {
            let _ = shared.wake.wait_timeout(guard, Duration::from_millis(2));
        }
        keen_since = Instant::now();
    }
}

#[cfg(windows)]
fn hurry_this_thread() {
    use std::ffi::c_void;
    extern "system" {
        fn GetCurrentThread() -> *mut c_void;
        fn SetThreadPriority(thread: *mut c_void, priority: i32) -> i32;
    }
    const TIME_CRITICAL: i32 = 15;
    unsafe {
        SetThreadPriority(GetCurrentThread(), TIME_CRITICAL);
    }
}

#[cfg(not(windows))]
fn hurry_this_thread() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_job_runs_once_and_the_call_waits_for_all_of_them() {
        let crew = Crew::of(3);
        let mut counts = vec![0u32; 20];
        for _ in 0..50 {
            let jobs: Vec<Box<dyn FnMut() + Send + '_>> = counts
                .iter_mut()
                .map(|count| {
                    Box::new(move || {
                        std::thread::sleep(Duration::from_micros(50));
                        *count += 1;
                    }) as Box<dyn FnMut() + Send + '_>
                })
                .collect();
            crew.run_all(jobs);
        }
        assert!(counts.iter().all(|count| *count == 50), "{counts:?}");
    }

    #[test]
    fn a_job_that_panics_does_not_hang_the_batch() {
        let crew = Crew::of(2);
        let mut ran = 0u32;
        let jobs: Vec<Box<dyn FnMut() + Send + '_>> = vec![
            Box::new(|| panic!("a plugin job blew up")) as Box<dyn FnMut() + Send + '_>,
            Box::new(|| ran += 1) as Box<dyn FnMut() + Send + '_>,
        ];
        crew.run_all(jobs);
        assert_eq!(ran, 1);
    }

    #[test]
    fn an_empty_batch_returns_at_once() {
        let crew = Crew::of(2);
        let began = Instant::now();
        crew.run_all(Vec::new());
        assert!(began.elapsed() < Duration::from_millis(50));
    }
}
