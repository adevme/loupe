use std::sync::OnceLock;
use std::time::{Duration, Instant};

static START: OnceLock<Instant> = OnceLock::new();

pub fn start() -> Instant {
    *START.get_or_init(Instant::now)
}

pub fn nanos(at: Instant) -> u64 {
    at.saturating_duration_since(start()).as_nanos() as u64
}

pub fn instant(nanos: u64) -> Instant {
    start() + Duration::from_nanos(nanos)
}
