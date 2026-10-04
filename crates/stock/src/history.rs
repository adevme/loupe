use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

pub const MOMENTS_PER_SECOND: f32 = 200.0;
const KEPT: usize = 2048;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Moment {
    pub input: f32,
    pub output: f32,
    pub reduction: f32,
}

pub struct History {
    slots: Box<[[AtomicU32; 3]]>,
    written: AtomicUsize,
}

impl History {
    pub fn new() -> Self {
        Self {
            slots: (0..KEPT).map(|_| [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)]).collect(),
            written: AtomicUsize::new(0),
        }
    }

    pub fn capacity(&self) -> usize {
        KEPT
    }

    pub fn written(&self) -> usize {
        self.written.load(Ordering::Acquire)
    }

    pub(crate) fn push(&self, moment: Moment) {
        let at = self.written.load(Ordering::Relaxed);
        let slot = &self.slots[at % KEPT];
        slot[0].store(moment.input.to_bits(), Ordering::Relaxed);
        slot[1].store(moment.output.to_bits(), Ordering::Relaxed);
        slot[2].store(moment.reduction.to_bits(), Ordering::Relaxed);
        self.written.store(at.wrapping_add(1), Ordering::Release);
    }

    pub fn latest(&self, out: &mut [Moment]) {
        let end = self.written();
        let start = end.wrapping_sub(out.len());
        for (i, moment) in out.iter_mut().enumerate() {
            let at = start.wrapping_add(i);
            if end.wrapping_sub(at) > end.min(KEPT) {
                *moment = Moment::default();
                continue;
            }
            let slot = &self.slots[at % KEPT];
            let read = |n: usize| f32::from_bits(slot[n].load(Ordering::Relaxed));
            *moment = Moment { input: read(0), output: read(1), reduction: read(2) };
        }
    }
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) struct Gatherer {
    every: usize,
    count: usize,
    loudest_in: f32,
    loudest_out: f32,
    lowest_gain: f32,
    widest_db: f32,
}

impl Gatherer {
    fn start_again(&mut self) {
        self.count = 0;
        self.loudest_in = 0.0;
        self.loudest_out = 0.0;
        self.lowest_gain = 1.0;
        self.widest_db = 0.0;
    }

    pub(crate) fn new(rate: f32) -> Self {
        Self { every: ((rate / MOMENTS_PER_SECOND) as usize).max(1), count: 0, loudest_in: 0.0, loudest_out: 0.0, lowest_gain: 1.0, widest_db: 0.0 }
    }

    #[inline]
    pub(crate) fn hear_change(&mut self, history: &History, input: f32, output: f32, change_db: f32) {
        self.loudest_in = self.loudest_in.max(input);
        self.loudest_out = self.loudest_out.max(output);
        if change_db.abs() > self.widest_db.abs() {
            self.widest_db = change_db;
        }
        self.count += 1;
        if self.count >= self.every {
            history.push(Moment { input: self.loudest_in, output: self.loudest_out, reduction: self.widest_db });
            self.start_again();
        }
    }

    #[inline]
    pub(crate) fn hear(&mut self, history: &History, input: f32, output: f32, gain: f32) {
        self.loudest_in = self.loudest_in.max(input);
        self.loudest_out = self.loudest_out.max(output);
        self.lowest_gain = self.lowest_gain.min(gain);
        self.count += 1;
        if self.count >= self.every {
            history.push(Moment {
                input: self.loudest_in,
                output: self.loudest_out,
                reduction: crate::db_of(self.lowest_gain),
            });
            self.start_again();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moments_gather_the_loudest_and_the_deepest() {
        let history = History::new();
        let mut gatherer = Gatherer::new(800.0);
        for i in 0..8 {
            gatherer.hear(&history, i as f32 * 0.1, 0.2, 1.0 - i as f32 * 0.1);
        }
        let mut seen = [Moment::default(); 3];
        history.latest(&mut seen);
        assert_eq!(seen[0], Moment::default(), "nothing was written that long ago");
        assert!((seen[1].input - 0.3).abs() < 1e-6 && (seen[2].input - 0.7).abs() < 1e-6);
        assert!((seen[2].reduction - crate::db_of(0.3)).abs() < 1e-4);
    }
}
