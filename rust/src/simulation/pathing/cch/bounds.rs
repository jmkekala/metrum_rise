// SPDX-License-Identifier: GPL-2.0-only

//! Integer coordinate potentials whose power-of-two costs bound rounded CCH car queries.

use super::CchGraph;
use crate::simulation::network::{graph::RegionGraph, types::TransitFlags};
use rayon::prelude::*;

/// A metric-scoped lower bound, independent of route availability or turn restrictions.
/// Preparation/storage are O(nodes + edges)/O(nodes); queries are allocation-free O(1).
#[derive(Clone, Debug, Default)]
pub(crate) struct CchCarCostBounds {
    revision: Option<(u32, u64)>,
    coordinates: Vec<[i32; 2]>,
    unit_cost: [f32; 2],
    valid: bool,
}

// Every integer through 2^24 times a power of two is exactly representable in f32
// (or overflows to infinity). Capping here preserves that property under any addition tree.
const EXACT_UNITS: u32 = 1 << 24;

impl CchCarCostBounds {
    /// Rebuilds potentials only after CCH topology/customization changes, using its frozen
    /// base weights rather than potentially newer, not-yet-customized graph prices.
    pub(crate) fn prepare(&mut self, cch: &CchGraph, graph: &RegionGraph) {
        let revision = Some((cch.build_generation, cch.metric_revision));
        if self.revision == revision { return; }
        self.revision = revision;
        self.valid = false;
        self.coordinates.resize(graph.node_count(), [0; 2]);
        let coordinates_valid = self.coordinates.par_iter_mut().enumerate().all(|(index, target)| {
            let pos = graph.node(index as u32).pos;
            let point = [f64::from(pos.x).floor(), f64::from(pos.z).floor()];
            if point.iter().any(|&value| !value.is_finite()
                || value < f64::from(i32::MIN) || value > f64::from(i32::MAX)) { return false; }
            *target = point.map(|value| value as i32);
            true
        });
        if !coordinates_valid || cch.base_metric.len() != graph.edge_count()
            || !cch.base_metric.par_iter().all(|&(cost, _)| cost >= 0.0) { return; }
        self.unit_cost = graph.edges().par_iter().zip(&cch.base_metric)
            .map(|(edge, &(cost, _))| {
                if edge.deleted || edge.allowed_types & TransitFlags::CAR == 0 || cost == f32::INFINITY {
                    return [f32::INFINITY; 2];
                }
                let Some(from) = self.coordinates.get(edge.start_node as usize) else { return [0.0; 2]; };
                let Some(to) = self.coordinates.get(edge.end_node as usize) else { return [0.0; 2]; };
                std::array::from_fn(|axis| unit_cost(cost, from[axis].abs_diff(to[axis])))
            })
            .reduce(|| [f32::INFINITY; 2], |a, b| [a[0].min(b[0]), a[1].min(b[1])])
            .map(|cost| if cost.is_finite() { cost } else { 0.0 });
        self.valid = true;
    }

    /// Bounds any finite car query, including its exact f32 shortcut/meeting additions.
    /// Each edge dominates its integer coordinate displacement times a power-of-two unit.
    /// Monotone addition preserves capped sums of those exactly representable units; the
    /// triangle inequality then bounds endpoint displacement. Taking either axis or a minimum
    /// over possible entrance endpoints therefore cannot reject a cheaper rounded route.
    pub(crate) fn cost(&self, from: u32, to: u32, cch: &CchGraph) -> Option<f32> {
        if !self.valid || self.revision != Some((cch.build_generation, cch.metric_revision)) { return None; }
        let from = self.coordinates.get(from as usize)?;
        let to = self.coordinates.get(to as usize)?;
        Some((0..2).map(|axis| {
            from[axis].abs_diff(to[axis]).min(EXACT_UNITS) as f32 * self.unit_cost[axis]
        }).fold(0.0, f32::max))
    }
}

fn unit_cost(cost: f32, displacement: u32) -> f32 {
    if displacement == 0 { return f32::INFINITY; }
    let ratio = (f64::from(cost) / f64::from(displacement)) as f32;
    let bits = ratio.to_bits();
    let power = if bits < 0x0080_0000 {
        if bits == 0 { return 0.0; }
        1 << (31 - bits.leading_zeros())
    } else { bits & 0x7f80_0000 };
    let mut unit = f32::from_bits(power);
    // The initial quotient may round upward. A 32-bit integer times a power of two is
    // exact in f64, so this comparison repairs that boundary without an arbitrary epsilon.
    if f64::from(unit) * f64::from(displacement) > f64::from(cost) { unit *= 0.5; }
    unit
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::{graph::Edge, types::NodeType};
    use godot::prelude::Vector3;

    #[test]
    fn potentials_follow_frozen_metrics_and_disable_invalid_inputs() {
        let mut graph = RegionGraph::new();
        graph.add_node(Vector3::ZERO, NodeType::Junction);
        graph.add_node(Vector3::new(100.0, 0.0, -100.0), NodeType::Junction);
        graph.add_edge(Edge { start_node: 0, end_node: 1, base_cost: 100.0,
            allowed_types: TransitFlags::CAR, fwd_lanes: 1, bkw_lanes: 1, ..Default::default() });
        graph.rebuild_adjacency_list();
        let mut cch = CchGraph::build(&graph);
        let mut bounds = CchCarCostBounds::default();
        bounds.prepare(&cch, &graph);
        assert_eq!(bounds.cost(0, 1, &cch), Some(100.0));
        graph.edge_mut(0).base_cost = 0.0;
        let mut freshly_prepared = CchCarCostBounds::default();
        freshly_prepared.prepare(&cch, &graph);
        assert_eq!(freshly_prepared.cost(0, 1, &cch), Some(100.0));
        cch.customize(&graph);
        assert_eq!(bounds.cost(0, 1, &cch), None);
        bounds.prepare(&cch, &graph);
        assert_eq!(bounds.cost(0, 1, &cch), Some(0.0));
        assert_eq!(bounds.cost(0, 2, &cch), None);
        for invalid in [-1.0, f32::NAN] {
            graph.edge_mut(0).base_cost = invalid;
            cch.customize(&graph);
            bounds.prepare(&cch, &graph);
            assert_eq!(bounds.cost(0, 1, &cch), None);
        }
        let mut invalid_coordinates = RegionGraph::new();
        invalid_coordinates.add_node(Vector3::new(f32::INFINITY, 0.0, 0.0), NodeType::Junction);
        let invalid_cch = CchGraph::build(&invalid_coordinates);
        bounds.prepare(&invalid_cch, &invalid_coordinates);
        assert_eq!(bounds.cost(0, 0, &invalid_cch), None);
    }

    #[test]
    fn potential_units_are_exact_lower_bounds_at_normal_and_subnormal_boundaries() {
        for bits in [1, 3, 0x007f_ffff, 0x0080_0000, 0x3f7f_ffff, 0x3f80_0000,
            0x3f80_0001, 0x4180_0000, 0x7f7f_ffff] {
            let cost = f32::from_bits(bits);
            for displacement in [1, 2, 3, 100, EXACT_UNITS - 1, EXACT_UNITS, u32::MAX] {
                let unit = unit_cost(cost, displacement);
                assert!(f64::from(unit) * f64::from(displacement) <= f64::from(cost));
                assert!(displacement.min(EXACT_UNITS) as f32 * unit <= cost);
            }
        }
        assert_eq!(unit_cost(0.0, 10), 0.0);
    }
}
