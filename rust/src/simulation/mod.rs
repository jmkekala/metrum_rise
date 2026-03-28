//! All simulation subsystems. Driven by [`crate::nodes::simulation_node::SimulationNode`].
//!
//! Subsystems communicate by passing references to each other in `simulate_tick` —
//! there is no shared global state. The canonical tick order is:
//! time → terrain (passive) → water → network (road edits) → pathing → grid (env) → buildings → agents.

pub mod buildings;
pub mod core;
pub mod economy;
pub mod grid;
pub mod network;
pub mod pathing;
pub(crate) mod save;
pub mod terrain;
pub mod water;
