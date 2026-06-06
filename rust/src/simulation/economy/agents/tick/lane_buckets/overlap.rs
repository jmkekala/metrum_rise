//! Static overlap correction for cars in retained lane buckets.

use crate::config::{CAR_LENGTH, IDM_S_MIN};
use crate::simulation::economy::agents::data::AgentSystem;

impl AgentSystem {
    pub(super) fn correct_lane_overlaps(&mut self) {
        let min_sep = CAR_LENGTH + IDM_S_MIN;
        for &lid in &self.dirty_lanes {
            let bucket = &mut self.lane_buckets[lid];
            for j in (0..bucket.len().saturating_sub(1)).rev() {
                let max_rear = (bucket[j + 1].0 - min_sep).max(0.0);
                if bucket[j].0 > max_rear {
                    bucket[j].0 = max_rear;
                    self.agents.lane_distance[bucket[j].1] = max_rear;
                }
            }
        }
    }
}
