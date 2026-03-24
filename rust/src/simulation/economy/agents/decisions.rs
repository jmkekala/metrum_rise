//! Decision-making logic for agents: transit mode selection, path planning triggers.

use crate::simulation::network::graph::TransitGraph;
use crate::simulation::pathing::hpa::HpaGraph;
use super::data::AgentSystem;

impl AgentSystem {
    pub fn decide_transit_mode(
        &mut self,
        i: usize,
        target_node: u32,
        graph: &TransitGraph,
        hpa: &HpaGraph,
    ) -> (u32, bool) {
        self.pathfind_count += 1;
        let current_node = self.current_node[i];
        let mut pedestrian_dist = 10000.0;
        if let Some((_cost, _dist, _path)) = hpa.find_path(current_node, target_node, usize::MAX, graph, true) {
            pedestrian_dist = _dist;
        }


        if pedestrian_dist > 500.0 && self.has_car[i] {
            // Far target and has car, but ONLY drive if a driving path actually exists!
            self.pathfind_count += 1;
            if hpa.find_path(current_node, target_node, usize::MAX, graph, false).is_some() {
                return (target_node, true);
            }
        }
        
        // Close target, no car, OR car path disconnected -> Walk
        return (target_node, false);
    }
}
