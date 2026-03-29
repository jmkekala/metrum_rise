//! Decision-making logic for agents: transit mode selection, path planning triggers.

use super::data::AgentSystem;
use super::{MODE_CAR, MODE_WALK};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use crate::simulation::pathing::cch::CchGraph;

impl AgentSystem {
    /// Selects the most appropriate transit mode (Walk vs Car) for agent `i` based on path distance
    /// and car ownership. Returns the chosen `MODE_*` constant and the route as a node list.
    ///
    /// `self.current_node[i]` must be set to the departure node before calling.
    /// Returns an empty path if no route exists.
    pub fn decide_transit_mode(
        &mut self,
        i: usize,
        target_node: u32,
        graph: &RegionGraph,
        cch: &CchGraph,
    ) -> (u8, Vec<u32>) {
        self.pathfind_count += 1;
        let walk_result = cch.find_path(
            self.current_node[i],
            target_node,
            usize::MAX,
            graph,
            TransitFlags::FOOT,
        );
        let pedestrian_dist = walk_result.as_ref().map_or(10000.0, |r| r.1);

        if pedestrian_dist > 500.0 && self.has_car[i] {
            self.pathfind_count += 1;
            if let Some((_, _, car_path)) = cch.find_path(
                self.current_node[i],
                target_node,
                usize::MAX,
                graph,
                TransitFlags::CAR,
            ) {
                return (MODE_CAR, car_path);
            }
        }

        let path = walk_result.map(|(_, _, p)| p).unwrap_or_default();
        (MODE_WALK, path)
    }
}
