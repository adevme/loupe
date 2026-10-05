pub struct Dry {
    kept: Vec<[f32; 2]>,
    at: usize,
    held: usize,
    spare: Vec<[f32; 2]>,
}

impl Dry {
    pub fn empty() -> Self {
        Self { kept: Vec::new(), at: 0, held: 0, spare: Vec::new() }
    }

    pub fn room_for(&mut self, latency: usize, block: usize) {
        let want = latency + block.max(1);
        if self.held != latency || self.kept.len() < want {
            self.kept.clear();
            self.kept.resize(want.max(1), [0.0; 2]);
            self.spare.clear();
            self.spare.resize(block.max(1), [0.0; 2]);
            self.at = 0;
            self.held = latency;
        }
    }

    pub fn blend(&mut self, audio: &mut [[f32; 2]], mix: f32) {
        if self.kept.is_empty() {
            return;
        }
        let wet = mix.clamp(0.0, 1.0);
        let dry = 1.0 - wet;
        let room = self.kept.len();
        for (at, frame) in audio.iter_mut().enumerate() {
            let was = self.spare.get(at).copied().unwrap_or([0.0; 2]);
            self.kept[self.at] = was;
            let behind = (self.at + room - self.held.min(room)) % room;
            let older = self.kept[behind];
            self.at = (self.at + 1) % room;
            frame[0] = frame[0] * wet + older[0] * dry;
            frame[1] = frame[1] * wet + older[1] * dry;
        }
    }

    pub fn remember(&mut self, audio: &[[f32; 2]]) {
        if self.spare.len() < audio.len() {
            return;
        }
        self.spare[..audio.len()].copy_from_slice(audio);
    }
}
