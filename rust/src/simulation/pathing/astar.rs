//! A* priority-queue state used by the bidirectional Dijkstra query in [`super::cch`].
//!
//! State is keyed by `(node_id, incoming_edge)` rather than just `node_id` so that
//! turn restrictions stored in [`crate::simulation::network::graph::Node::lane_connections`]
//! can be correctly enforced during expansion.

use std::cmp::Ordering;

/// A single entry in the A* / Dijkstra priority queue.
#[derive(Copy, Clone, PartialEq)]
pub struct State {
    /// `cost + heuristic` — the value the heap orders by (min-heap via reversed `Ord`).
    pub priority: f32,
    /// Accumulated true cost (time, not distance) from the source to this node via `incoming_edge`.
    pub cost: f32,
    /// Accumulated physical distance (metres) from the source, tracked separately from cost
    /// so callers receive both time-cost and distance on path completion.
    pub dist: f32,
    /// The graph node this state represents.
    pub node: u32,
    /// The edge index through which `node` was reached. `usize::MAX` for the start state.
    /// Required for turn-restriction checks at junctions.
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
