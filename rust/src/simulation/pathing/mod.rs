//! Pathfinding: cost calculation, A* state, and CCH/CRP hierarchical routing.
//!
//! Use [`cch::CchGraph::find_path`] for all agent route queries.
//! Rebuild [`cch::CchGraph`] via [`crate::simulation::network::TransitNetwork::rebuild_pathing`]
//! after any structural road-network change or metric update.

pub mod astar;
pub mod cch;
pub mod cost;
pub mod pedestrian;

#[cfg(test)]
mod tests;
