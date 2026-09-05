// SPDX-License-Identifier: GPL-2.0-only

//! Pathfinding: cost calculation, A* state, and CCH/CRP hierarchical routing.
//!
//! Use [`cch::CchGraph::find_path`] for all agent route queries.
//! Rebuild [`cch::CchGraph`] via [`crate::simulation::network::TransitNetwork::rebuild_pathing`]
//! after any structural road-network change or metric update.

pub mod astar;
pub mod cch;
pub mod cost;
pub mod flow_field;

#[cfg(test)]
mod tests;
