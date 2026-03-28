//! Decision-making logic for agents: transit mode selection, path planning triggers.

use super::data::AgentSystem;
use super::{MODE_CAR, MODE_WALK};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use crate::simulation::pathing::cch::CchGraph;
use crate::simulation::pathing::pedestrian::{
    PedestrianEndpoint, find_path as find_pedestrian_path,
};

impl AgentSystem {
    /// Selects the most appropriate transit mode (Walk vs Car) for the agent based on distance and car ownership.
    /// Returns the target node and the chosen `MODE_*` constant.
    pub fn decide_transit_mode(
        &mut self,
        i: usize,
        target_node: u32,
        graph: &RegionGraph,
        cch: &CchGraph,
    ) -> (u32, u8) {
        self.decide_transit_mode_with_endpoints(
            i,
            PedestrianEndpoint {
                node: self.current_node[i],
                edge_idx: None,
                side: 0,
            },
            PedestrianEndpoint {
                node: target_node,
                edge_idx: None,
                side: 0,
            },
            target_node,
            graph,
            cch,
        )
    }

    /// Selects the most appropriate transit mode using concrete pedestrian start/end anchors.
    pub fn decide_transit_mode_with_endpoints(
        &mut self,
        i: usize,
        start_ped: PedestrianEndpoint,
        target_ped: PedestrianEndpoint,
        target_node: u32,
        graph: &RegionGraph,
        cch: &CchGraph,
    ) -> (u32, u8) {
        self.pathfind_count += 1;
        let mut pedestrian_dist = 10000.0;
        if let Some((_cost, dist, _path)) = find_pedestrian_path(graph, start_ped, target_ped) {
            pedestrian_dist = dist;
        }

        if pedestrian_dist > 500.0 && self.has_car[i] {
            // Far target and has car, but ONLY drive if a driving path actually exists!
            self.pathfind_count += 1;
            if cch
                .find_path(
                    self.current_node[i],
                    target_node,
                    usize::MAX,
                    graph,
                    TransitFlags::CAR,
                )
                .is_some()
            {
                return (target_node, MODE_CAR);
            }
        }

        // Close target, no car, OR car path disconnected -> Walk
        return (target_node, MODE_WALK);
    }
}
