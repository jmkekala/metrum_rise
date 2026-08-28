// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: planning.rs
//  script_path: rust/src/simulation/economy/agents/tick/planning.rs
//  module_name: planning
//  version: 0.1.0
//  description: Trip planning and network replanning helpers for agent
//           movement.
//  kind: module
//  spec: none
//  internal_dependencies: []
//  external_dependencies: []
//  features: []
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-27
// ========================================================================

//! Trip planning and network replanning helpers for agent movement.

// ========================================================================
// SUBMODULES
// ========================================================================

mod candidate;
mod immigration;
mod network;
mod reroute;
mod trip;
mod types;

// ========================================================================
// RE-EXPORTS
// ========================================================================

pub(crate) use immigration::plan_immigration_trip;
pub(super) use network::{plan_border_network_replan, plan_network_replan};
pub(super) use reroute::{REROUTE_INTERVAL_S, price_node_path, reroute_is_worthwhile};
pub(crate) use trip::{building_origin_trip_is_feasible, estimate_building_origin_trip_minutes};
pub(crate) use trip::{plan_building_origin_trip, plan_building_to_border_trip};
pub(super) use types::BuiltNetworkReplan;
pub(crate) use types::BuiltTripPlan;
