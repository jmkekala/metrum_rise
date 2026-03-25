//! Decision-making logic for agents: transit mode selection, path planning triggers.

use crate::simulation::network::graph::RegionGraph;
use crate::simulation::pathing::hpa::HpaGraph;
use crate::simulation::network::types::TransitFlags;
use super::data::AgentSystem;
use super::{MODE_CAR, MODE_WALK};

impl AgentSystem {
    /// Selects the most appropriate transit mode (Walk vs Car) for the agent based on distance and car ownership.
    /// Returns the target node and the chosen `MODE_*` constant.
    pub fn decide_transit_mode(
        &mut self,
        i: usize,
        target_node: u32,
        graph: &RegionGraph,
        hpa: &HpaGraph,
    ) -> (u32, u8) {
        self.pathfind_count += 1;
        let current_node = self.current_node[i];
        let mut pedestrian_dist = 10000.0;
        if let Some((_cost, _dist, _path)) = hpa.find_path(current_node, target_node, usize::MAX, graph, TransitFlags::FOOT) {
            pedestrian_dist = _dist;
        }


        if pedestrian_dist > 500.0 && self.has_car[i] {
            // Far target and has car, but ONLY drive if a driving path actually exists!
            self.pathfind_count += 1;
            if hpa.find_path(current_node, target_node, usize::MAX, graph, TransitFlags::CAR).is_some() {
                return (target_node, MODE_CAR);
            }
        }
        
        // Close target, no car, OR car path disconnected -> Walk
        return (target_node, MODE_WALK);
    }
}
