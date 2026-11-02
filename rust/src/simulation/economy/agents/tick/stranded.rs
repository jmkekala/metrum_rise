// SPDX-License-Identifier: GPL-2.0-only

//! Stranded walkers (`ROAD-74`): the walkers waiting at the end of a lane with no route home
//! ([`ACCESS_WALK_STRANDED`]), and the report of where they wait for the player's notice. With
//! them, the walkers waiting for a home (`TRAFFIC-19`): residents whose household was evicted
//! while they walked, so their trip home was cancelled. The economy rehouses the household (they
//! then walk to the new home) or, after its days without a home, it leaves the city; until then
//! they wait on the road and are counted here, never silently.
//!
//! The set is incremental: a walker enters it with the trip-failure record that strands it,
//! read from the movement pass's records (O(new strandings) per tick), or with its household's
//! eviction, and leaves it when the report finds it neither stranded nor waiting for a home
//! (walking home again, rehoused, a plan clear by a road edit, gone).
//! The report is rebuilt from the set in the `gridlock check` slot, once a game minute:
//! O(stranded × log stranded). The set is derived: loading rebuilds it and the report from the
//! saved stranded bits (one O(agents) scan at load).

use super::super::data::AgentSystem;
use super::super::{
    ACCESS_WALK_STRANDED, AgentVec, MODE_WALK, TRANSIT_INTERSECTION, TRANSIT_NETWORK,
};
use super::lane_nav::lane_terminal_node;
use super::trip_failures::FailureOutcome;
use crate::simulation::core::handles::Handle;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use std::collections::BTreeSet;

/// Where stranded walkers wait, as of the last report.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct StrandedReport {
    /// `(node, walkers waiting there)`, ascending by node.
    pub(crate) junctions: Vec<(u32, u32)>,
    /// Stranded walkers in all.
    pub(crate) walkers: u32,
    /// Walkers waiting on the road for their household's next home (`TRAFFIC-19`).
    pub(crate) waiting_for_home: u32,
}

/// The stranded walkers by agent handle, and their last report.
#[derive(Default)]
pub(crate) struct StrandedWalkers {
    members: BTreeSet<u64>,
    report: StrandedReport,
    // The members' nodes, reused by each report.
    nodes: Vec<u32>,
}

impl AgentSystem {
    /// The last stranded-walker report.
    pub(crate) fn stranded_report(&self) -> &StrandedReport {
        &self.stranded.report
    }

    // Adds the walkers the last movement pass stranded, from its trip-failure records.
    pub(super) fn note_stranded_walkers(&mut self) {
        let buffer = &self.trip_failures.buffer;
        if !buffer
            .iter()
            .any(|failure| matches!(failure.outcome, FailureOutcome::Stranded { .. }))
        {
            return;
        }
        self.handles.register_appended(self.agents.len());
        for failure in buffer {
            if matches!(failure.outcome, FailureOutcome::Stranded { .. }) {
                let handle = self.handles.handle(failure.agent).to_bits();
                self.stranded.members.insert(handle);
            }
        }
    }

    /// Adds walker `i` to the set when it waits on the road for a home: its household lost the
    /// home while it walked (`evict_household_member_home`). O(log stranded).
    pub(crate) fn note_waiting_for_home(&mut self, i: usize) {
        if waiting_for_home(&self.agents, i) {
            self.handles.register_appended(self.agents.len());
            let handle = self.handles.handle(i).to_bits();
            self.stranded.members.insert(handle);
        }
    }

    /// Rebuilds the report from the set (the `gridlock check` slot), dropping the walkers that are
    /// neither stranded nor waiting for a home. O(stranded × log stranded).
    pub(crate) fn refresh_stranded_report(
        &mut self,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        let agents = &self.agents;
        let handles = &self.handles;
        let stranded = &mut self.stranded;
        stranded.nodes.clear();
        let mut waiting_for_home_count = 0;
        stranded.members.retain(|&bits| {
            let Some(i) = handles.resolve(Handle::from_bits(bits)) else {
                return false;
            };
            if waiting_for_home(agents, i) {
                waiting_for_home_count += 1;
                return true;
            }
            let waiting = agents.transit_mode[i] == MODE_WALK
                && agents.access_flags[i] & ACCESS_WALK_STRANDED != 0
                && matches!(agents.transit[i], TRANSIT_NETWORK | TRANSIT_INTERSECTION);
            if waiting {
                stranded.nodes.push(waiting_node(
                    agents.current_lane_id[i],
                    agents.current_node[i],
                    network,
                    graph,
                ));
            }
            waiting
        });
        stranded.nodes.sort_unstable();
        let report = &mut stranded.report;
        report.junctions.clear();
        for &node in &stranded.nodes {
            match report.junctions.last_mut() {
                Some((last, count)) if *last == node => *count += 1,
                _ => report.junctions.push((node, 1)),
            }
        }
        report.walkers = stranded.nodes.len() as u32;
        report.waiting_for_home = waiting_for_home_count;
    }

    /// Rebuilds the set and the report from the agents' stranded bits and homeless walkers, after a
    /// load. O(agents).
    pub(crate) fn rebuild_stranded_walkers(
        &mut self,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        self.handles.register_appended(self.agents.len());
        self.stranded.members.clear();
        for i in 0..self.agents.len() {
            if self.agents.access_flags[i] & ACCESS_WALK_STRANDED != 0
                || waiting_for_home(&self.agents, i)
            {
                self.stranded
                    .members
                    .insert(self.handles.handle(i).to_bits());
            }
        }
        self.refresh_stranded_report(network, graph);
    }
}

// Whether walker `i` waits on the road for a home: no home, no trip and no next one, not
// stranded (a stranded walker has a trip home or to its destination).
fn waiting_for_home(agents: &AgentVec, i: usize) -> bool {
    agents.transit_mode[i] == MODE_WALK
        && matches!(agents.transit[i], TRANSIT_NETWORK | TRANSIT_INTERSECTION)
        && agents.home_building[i] == usize::MAX
        && agents.target_building[i] == usize::MAX
        && agents.planned_target_building[i] == usize::MAX
        && agents.access_flags[i] & ACCESS_WALK_STRANDED == 0
}

// The node a stranded walker waits at: the end of its road lane, its connector's junction, or
// the node it stands at.
fn waiting_node(lane: usize, node: u32, network: &TransitNetwork, graph: &RegionGraph) -> u32 {
    match network.lane_system.lanes.get(lane) {
        Some(value) if value.edge_id == usize::MAX => value.node_id as u32,
        Some(_) => lane_terminal_node(lane, network, graph).unwrap_or(node),
        None => node,
    }
}
