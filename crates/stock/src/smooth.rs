const GLIDE_SECONDS: f32 = 0.02;

#[derive(Clone, Copy)]
pub struct Smoothed {
    now: f32,
    target: f32,
    step: f32,
    left: u32,
    glide: u32,
}

impl Smoothed {
    pub fn new(value: f32) -> Self {
        Self { now: value, target: value, step: 0.0, left: 0, glide: 960 }
    }

    pub fn prepare(&mut self, rate: f32) {
        self.glide = ((GLIDE_SECONDS * rate) as u32).max(1);
        self.snap();
    }

    pub fn aim(&mut self, target: f32) {
        if target == self.target {
            return;
        }
        self.target = target;
        self.left = self.glide;
        self.step = (target - self.now) / self.glide as f32;
    }

    pub fn snap(&mut self) {
        self.now = self.target;
        self.left = 0;
    }

    pub fn next(&mut self) -> f32 {
        if self.left > 0 {
            self.left -= 1;
            self.now = if self.left == 0 { self.target } else { self.now + self.step };
        }
        self.now
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_glides_and_lands_exactly() {
        let mut level = Smoothed::new(0.0);
        level.prepare(1000.0);
        level.aim(1.0);
        let seen: Vec<f32> = (0..30).map(|_| level.next()).collect();
        assert!(seen.windows(2).all(|pair| pair[1] >= pair[0]));
        assert!(seen[0] > 0.0 && seen[0] < 0.1);
        assert_eq!(seen[19], 1.0);
        assert_eq!(seen[29], 1.0);
    }
}
