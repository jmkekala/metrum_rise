// SPDX-License-Identifier: GPL-2.0-only

//! Shared trip-plan result types.

/// A fully built trip from a current building to a target building.
#[derive(Clone)]
pub(crate) struct BuiltTripPlan {
    /// Travel mode chosen for the trip.
    pub(crate) mode: u8,
    /// Destination building index.
    pub(crate) target_building: usize,
    /// Activity to start after the trip completes.
    pub(crate) activity: u8,
    /// Road node where the access-egress leg attaches to the network.
    pub(crate) planned_attach_node: u32,
    /// Road node where the network leg detaches toward the destination.
    pub(crate) planned_detach_node: u32,
    /// Lane used for the origin access handoff.
    pub(crate) planned_attach_lane_id: usize,
    /// Lane used for the destination access handoff.
    pub(crate) planned_detach_lane_id: usize,
    /// Distance along the attach lane for the origin handoff.
    pub(crate) planned_attach_lane_d: f32,
    /// Distance along the detach lane for the destination handoff.
    pub(crate) planned_detach_lane_d: f32,
    /// Planned network node path between attach and detach nodes.
    pub(crate) current_path: Vec<u32>,
    /// Access-plan flags describing path provenance and special cases.
    pub(crate) access_flags: u8,
}

/// A rebuilt destination-side network plan for an agent already outside a building.
#[derive(Clone)]
pub(in crate::simulation::economy::agents::tick) struct BuiltNetworkReplan {
    /// Road node where the network leg detaches toward the destination.
    pub(in crate::simulation::economy::agents::tick) planned_detach_node: u32,
    /// Lane used for the destination access handoff.
    pub(in crate::simulation::economy::agents::tick) planned_detach_lane_id: usize,
    /// Distance along the detach lane for the destination handoff.
    pub(in crate::simulation::economy::agents::tick) planned_detach_lane_d: f32,
    /// Planned network node path from current location to detach node.
    pub(in crate::simulation::economy::agents::tick) current_path: Vec<u32>,
    /// Access-plan flags describing path provenance and special cases.
    pub(in crate::simulation::economy::agents::tick) access_flags: u8,
}
