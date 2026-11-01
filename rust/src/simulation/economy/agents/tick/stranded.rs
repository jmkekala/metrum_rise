// SPDX-License-Identifier: GPL-2.0-only

//! Stranded walkers (`ROAD-74`): the walkers waiting at the end of a lane with no route home
//! ([`ACCESS_WALK_STRANDED`]), and the report of where they wait for the player's notice.
//!
//! The set is incremental: a walker enters it with the trip-failure record that strands it,
//! read from the movement pass's records (O(new strandings) per tick), and leaves it when the
//! report finds it no longer stranded (walking home again, a plan clear by a road edit, gone).
//! The report is rebuilt from the set in the `gridlock check` slot, once a game minute:
//! O(stranded × log stranded). The set is derived: loading rebuilds it and the report from the
//! saved stranded bits (one O(agents) scan at load).

use super::super::data::AgentSystem;
use super::super::{ACCESS_WALK_STRANDED, MODE_WALK, TRANSIT_INTERSECTION, TRANSIT_NETWORK};
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

    /// Rebuilds the report from the set (the `gridlock check` slot), dropping the walkers that are
    /// no longer stranded. O(stranded × log stranded).
    pub(crate) fn refresh_stranded_report(
        &mut self,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        let agents = &self.agents;
        let handles = &self.handles;
        let stranded = &mut self.stranded;
        stranded.nodes.clear();
        stranded.members.retain(|&bits| {
            let Some(i) = handles.resolve(Handle::from_bits(bits)) else {
                return false;
            };
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
    }

    /// Rebuilds the set and the report from the agents' stranded bits, after a load. O(agents).
    pub(crate) fn rebuild_stranded_walkers(
        &mut self,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        self.handles.register_appended(self.agents.len());
        self.stranded.members.clear();
        for i in 0..self.agents.len() {
            if self.agents.access_flags[i] & ACCESS_WALK_STRANDED != 0 {
                self.stranded
                    .members
                    .insert(self.handles.handle(i).to_bits());
            }
        }
        self.refresh_stranded_report(network, graph);
    }
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
