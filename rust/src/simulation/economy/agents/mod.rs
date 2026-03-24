//! Agent simulation: data layout, activity states, and lifecycle management.

pub mod data;
pub mod decisions;
pub mod tick;

pub use data::AgentSystem;

/// Agent is inside a building — not moving. Waiting for the next activity trigger.
pub const TRANSIT_IDLE: u8 = 0;
/// Agent is walking from building interior to the road edge (departure animation phase).
pub const TRANSIT_DEPARTING: u8 = 1;
/// Agent is actively moving along road edges toward `target_node`.
pub const TRANSIT_ON_ROAD: u8 = 2;
/// Agent has reached the road node nearest their destination and is walking the final distance to the building.
pub const TRANSIT_ARRIVING: u8 = 3;
/// Newly spawned agent travelling from a highway border node to their first home — has no current building.
pub const TRANSIT_IMMIGRATING: u8 = 4;
/// Agent is traversing a bezier curve through a road intersection (lane-change phase).
pub const TRANSIT_INTERSECTION: u8 = 5;
