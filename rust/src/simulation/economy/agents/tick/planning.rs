// SPDX-License-Identifier: GPL-2.0-only

//! Trip planning and network replanning helpers for agent movement.

mod candidate;
mod immigration;
mod network;
mod reachability;
mod trip;
mod types;

pub(crate) use candidate::{SharedNetworkQuotes, TripQueryScratch};
pub(crate) use immigration::plan_immigration_trip;
pub(super) use network::{
    ReplanCandidateDiagnostic, ReplanFailure, plan_border_network_replan, plan_network_replan,
};
pub(crate) use reachability::{BuildingWalkingAccess, PedestrianReachability};
#[cfg(test)]
pub(crate) use trip::estimate_trip_seconds_between;
pub(crate) use trip::{
    BuildingTripEnds, TripEstimate, building_origin_trip_is_feasible,
    estimate_building_origin_trip_seconds, estimate_building_origin_trip_seconds_with_scratch,
    estimate_trip_seconds_with_scratch,
};
pub(crate) use trip::{plan_building_origin_trip, plan_building_to_border_trip};
pub(super) use types::BuiltNetworkReplan;
pub(crate) use types::BuiltTripPlan;
