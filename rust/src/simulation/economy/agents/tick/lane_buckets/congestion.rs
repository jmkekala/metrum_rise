// SPDX-License-Identifier: GPL-2.0-only

//! Per-edge congestion aggregation from live network agents.

use crate::simulation::economy::agents::data::AgentSystem;
use crate::simulation::network::graph::RegionGraph;

impl AgentSystem {
    pub(super) fn ensure_edge_congestion_buffers(&mut self, edge_count: usize) {
        if self.edge_speed_sum.len() < edge_count {
            self.edge_speed_sum.resize(edge_count, 0.0_f32);
            self.edge_agent_cnt.resize(edge_count, 0_u32);
            self.edge_is_dirty.resize(edge_count, false);
        }
    }

    pub(super) fn clear_dirty_edge_congestion(&mut self) {
        self.stale_dirty_edges.clear();
        for &eid in &self.dirty_edges {
            if eid < self.edge_speed_sum.len() {
                self.edge_speed_sum[eid] = 0.0;
                self.edge_agent_cnt[eid] = 0;
                self.edge_is_dirty[eid] = false;
                self.stale_dirty_edges.push(eid);
            }
        }
        self.dirty_edges.clear();
    }

    pub(super) fn push_dirty_edge_speed(&mut self, eid: usize, speed: f32) {
        if !self.edge_is_dirty[eid] {
            self.edge_is_dirty[eid] = true;
            self.dirty_edges.push(eid);
        }
        self.edge_speed_sum[eid] += speed;
        self.edge_agent_cnt[eid] += 1;
    }

    pub(super) fn commit_edge_congestion(&mut self, graph: &mut RegionGraph, edge_count: usize) {
        for &eid in &self.stale_dirty_edges {
            if eid < edge_count && !self.edge_is_dirty[eid] && !graph.edge(eid).deleted {
                graph.set_edge_congestion(eid, 0.0);
            }
        }
        for &eid in &self.dirty_edges {
            if eid >= edge_count {
                continue;
            }
            if !graph.edge(eid).deleted && self.edge_agent_cnt[eid] > 0 {
                let avg = self.edge_speed_sum[eid] / self.edge_agent_cnt[eid] as f32;
                let limit = graph.edge(eid).speed_limit.max(1.0);
                graph.set_edge_congestion(eid, (1.0 - avg / limit).max(0.0));
            }
        }
    }
}
