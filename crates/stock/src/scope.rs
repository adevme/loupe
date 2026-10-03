use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

pub struct Scope {
    samples: Box<[AtomicU32]>,
    written: AtomicUsize,
}

impl Scope {
    pub fn new(length: usize) -> Self {
        let length = length.next_power_of_two();
        Self { samples: (0..length).map(|_| AtomicU32::new(0)).collect(), written: AtomicUsize::new(0) }
    }

    pub fn capacity(&self) -> usize {
        self.samples.len()
    }

    pub(crate) fn writer(&self) -> usize {
        self.written.load(Ordering::Relaxed)
    }

    pub(crate) fn put(&self, at: usize, value: f32) {
        self.samples[at & (self.samples.len() - 1)].store(value.to_bits(), Ordering::Relaxed);
    }

    pub(crate) fn publish(&self, written: usize) {
        self.written.store(written, Ordering::Release);
    }

    pub fn written(&self) -> usize {
        self.written.load(Ordering::Acquire)
    }

    pub fn latest(&self, out: &mut [f32]) {
        let mask = self.samples.len() - 1;
        let end = self.written();
        let start = end.wrapping_sub(out.len());
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = f32::from_bits(self.samples[start.wrapping_add(i) & mask].load(Ordering::Relaxed));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_latest_samples_come_back_in_order() {
        let scope = Scope::new(8);
        let mut at = scope.writer();
        for value in 1..=11 {
            scope.put(at, value as f32);
            at += 1;
        }
        scope.publish(at);
        let mut seen = [0.0; 4];
        scope.latest(&mut seen);
        assert_eq!(seen, [8.0, 9.0, 10.0, 11.0]);
    }
}
