//! Pathfinding: cost calculation, A* state, and HPA* hierarchical routing.
//!
//! Use [`hpa::HpaGraph::find_path`] for all agent route queries.
//! Rebuild [`hpa::HpaGraph`] via [`crate::simulation::network::TransitNetwork::rebuild_pathing`]
//! after any structural road-network change.

pub mod cost;
pub mod astar;
pub mod hpa;

#[cfg(test)]
mod tests;
