use loupe_stock::{Effect, Param};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Change(pub usize, pub f32);

pub fn change(index: usize, value: f32) -> Change {
    Change(index, value)
}

pub struct Knobs {
    pub params: &'static [Param],
    pub values: Vec<f32>,
}

impl Knobs {
    pub fn of(effect: &dyn Effect) -> Self {
        let params = effect.params();
        Self { params, values: (0..params.len()).map(|index| effect.value(index)).collect() }
    }

    pub fn apply(&mut self, Change(index, value): Change) -> Vec<(usize, f32)> {
        let Some(param) = self.params.get(index) else {
            return Vec::new();
        };
        let value = param.clamp(value);
        if self.values[index] == value {
            return Vec::new();
        }
        self.values[index] = value;
        vec![(index, value)]
    }

    pub fn on(&self, index: usize) -> bool {
        self.values[index] > 0.5
    }
}
