// SPDX-License-Identifier: GPL-2.0-only

//! One-to-all costs using an upward query followed by a descending state sweep.

use super::*;

/// Costs from one source road node, including all legal arrival states at each target.
/// Summation proceeds outward from the source, so f32 rounding can differ from point queries.
#[derive(Clone, Debug)]
pub struct CchCostsFrom {
    cost: Vec<f32>,
}
impl CchCostsFrom {
    /// Returns a finite reachable cost, or None for an invalid or unreachable node.
    pub fn cost_to(&self, node: u32) -> Option<f32> {
        self.cost
            .get(node as usize)
            .copied()
            .filter(|&cost| cost < f32::MAX)
    }
}
impl CchGraph {
    /// Computes all target costs per source in O(states + filled arcs), plus the upward query.
    /// Sources execute independently with Rayon; each source owns its output and scratch.
    pub fn costs_from_each(
        &self,
        starts: &[u32],
        graph: &RegionGraph,
        mask: u8,
    ) -> Vec<CchCostsFrom> {
        starts
            .par_iter()
            .map(|&start| CchCostsFrom {
                cost: self.all_costs(start, graph, mask),
            })
            .collect()
    }
}
