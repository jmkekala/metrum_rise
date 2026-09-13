// SPDX-License-Identifier: GPL-2.0-only

//! Pathfinding costs, exact CCH queries and optional destination flow fields.
//!
//! Use [`cch::CchGraph::find_path`] for exact routing; flow fields provide a validated fast path.
//! Rebuild [`cch::CchGraph`] via [`crate::simulation::network::TransitNetwork::rebuild_pathing`]
//! after any structural road-network change or metric update.

pub mod cch;
pub mod cost;
pub mod flow_field;

#[cfg(test)]
mod tests;
