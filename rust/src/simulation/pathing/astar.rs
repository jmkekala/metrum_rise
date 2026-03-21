use std::cmp::Ordering;

#[derive(Copy, Clone, PartialEq)]
pub struct State {
    pub priority: f32,
    pub cost: f32,
    pub dist: f32,
    pub node: u32,
    pub incoming_edge: usize,
}

impl Eq for State {}

impl Ord for State {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering so BinaryHeap acts as a min-heap based on priority
        other.priority.partial_cmp(&self.priority).unwrap_or(Ordering::Equal)
    }
}

impl PartialOrd for State {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
