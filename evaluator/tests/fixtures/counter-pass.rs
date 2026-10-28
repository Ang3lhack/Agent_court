#[derive(Default)]
pub struct Counter(u64);

impl Counter {
    pub fn increment(&mut self) { self.0 += 1; }
    pub fn value(&self) -> u64 { self.0 }
    pub fn reset(&mut self) { self.0 = 0; }
}
