// SPDX-License-Identifier: GPL-2.0-only

//! Read-only live/planned road queries over existing spatial and owner-local indices.

use super::super::RoadSurfaceSystem;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::Vector3;
use std::collections::HashSet;

/// Bounded finalized neighborhood and its source-owner replacement contract.
pub(crate) struct PlannedRoadSurfaceQuery {
    graph: RegionGraph,
    surface: RoadSurfaceSystem,
    edge_ids: Vec<usize>,
    node_ids: Vec<u32>,
    replaced_edges: HashSet<usize>,
    replaced_nodes: HashSet<u32>,
}

impl std::fmt::Debug for PlannedRoadSurfaceQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlannedRoadSurfaceQuery")
            .field("edges", &self.edge_ids.len())
            .field("replaced_nodes", &self.replaced_nodes.len())
            .finish()
    }
}

impl PlannedRoadSurfaceQuery {
    /// Retains only the validation excerpt; compiled owner products remain shared by Arc.
    pub(in crate::simulation::network::surface) fn capture(
        graph: &RegionGraph,
        surface: &RoadSurfaceSystem,
        edge_ids: &[usize],
        node_ids: &[u32],
        replaced_edges: &HashSet<usize>,
        replaced_nodes: &HashSet<u32>,
    ) -> Self {
        Self {
            graph: graph.clone(),
            surface: surface.clone(),
            edge_ids: edge_ids.to_vec(),
            node_ids: node_ids.to_vec(),
            replaced_edges: replaced_edges.clone(),
            replaced_nodes: replaced_nodes.clone(),
        }
    }

    /// Verifies the published local products using the compiler's existing identity remappers.
    /// No geometric solve occurs; work and temporary copies are bounded to the planned owners.
    pub(crate) fn matches_published(
        &self,
        graph: &RegionGraph,
        surface: &RoadSurfaceSystem,
    ) -> bool {
        for (local, &live) in self.edge_ids.iter().enumerate() {
            if let Some(expected) = self.surface.compiled_visual_span_pieces.get(&local) {
                let Some(actual) = surface.compiled_visual_span_pieces.get(&live) else {
                    return false;
                };
                if **actual != expected.clone_with_edge_identity(live) {
                    return false;
                }
            }
        }
        for (local, &live) in self.node_ids.iter().enumerate() {
            let local = local as u32;
            if let Some(piece) = self.surface.compiled_visual_node_pieces.get(&local) {
                let Some(input) = surface.compiled_visual_node_inputs.get(&live) else {
                    return false;
                };
                if !self.surface.compiled_visual_node_inputs[&local]
                    .topology_eq_ignoring_edge_identity(input)
                {
                    return false;
                }
                let expected = surface.replay_exact_preview_node_piece(
                    graph,
                    live,
                    input,
                    std::sync::Arc::clone(piece),
                    self.surface
                        .compiled_visual_node_earthwork_boundaries
                        .get(&local)
                        .cloned()
                        .unwrap_or_default(),
                    std::sync::Arc::clone(&self.surface.compiled_visual_node_topologies[&local]),
                    false,
                );
                if surface.compiled_visual_node_pieces.get(&live) != Some(&expected.piece) {
                    return false;
                }
            }
        }
        true
    }

    /// Borrows the immutable resident world without copying it or recompiling its neighbors.
    pub(crate) fn view<'a>(
        &'a self,
        graph: &'a RegionGraph,
        surface: &'a RoadSurfaceSystem,
    ) -> RoadSurfaceView<'a> {
        RoadSurfaceView {
            graph,
            surface,
            planned: Some(self),
        }
    }
}

/// Sampling view that replaces local owners while retaining all other live contributions.
#[derive(Clone, Copy)]
pub(crate) struct RoadSurfaceView<'a> {
    graph: &'a RegionGraph,
    surface: &'a RoadSurfaceSystem,
    planned: Option<&'a PlannedRoadSurfaceQuery>,
}

impl<'a> RoadSurfaceView<'a> {
    /// Uses the authoritative world directly for ordinary terrain compilation.
    pub(crate) fn new(graph: &'a RegionGraph, surface: &'a RoadSurfaceSystem) -> Self {
        Self {
            graph,
            surface,
            planned: None,
        }
    }

    /// Samples indexed road tops before earthworks, preserving the live visibility precedence.
    pub(crate) fn sample_visible_height(
        self,
        terrain: &TerrainSystem,
        x: f32,
        z: f32,
    ) -> Option<f32> {
        let Some(planned) = self.planned else {
            return self
                .surface
                .sample_visible_surface_height(self.graph, terrain, x, z);
        };
        let (old_top, old_earthwork) = self.surface.sample_visible_surface_layers_filtered(
            self.graph,
            terrain,
            x,
            z,
            |edge| !planned.replaced_edges.contains(&edge),
            |node| !planned.replaced_nodes.contains(&node),
        );
        let (new_top, new_earthwork) = planned.surface.sample_visible_surface_layers_filtered(
            &planned.graph,
            terrain,
            x,
            z,
            |_| true,
            |_| true,
        );
        max_height(old_top, new_top).or_else(|| max_height(old_earthwork, new_earthwork))
    }

    /// Visits R-tree candidates with prospective live IDs for deterministic nearest-edge ties.
    /// Cost is O(log(resident edges) + local candidates log(local candidates)); no city traversal.
    pub(crate) fn visit_edges_near_point(
        self,
        pos: Vector3,
        radius_m: f32,
        mut visit: impl FnMut(&RegionGraph, usize, usize),
    ) {
        let mut source = self.graph.get_edges_near_point(pos, radius_m);
        source.sort_unstable();
        source.dedup();
        for edge in source {
            if self
                .planned
                .is_none_or(|plan| !plan.replaced_edges.contains(&edge))
            {
                visit(self.graph, edge, edge);
            }
        }
        if let Some(plan) = self.planned {
            let mut local = plan.graph.get_edges_near_point(pos, radius_m);
            local.sort_unstable();
            local.dedup();
            for edge in local {
                let live = plan.edge_ids[edge];
                if live >= self.graph.edge_count() || plan.replaced_edges.contains(&live) {
                    visit(&plan.graph, edge, live);
                }
            }
        }
    }
}

fn max_height(left: Option<f32>, right: Option<f32>) -> Option<f32> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        _ => left.or(right),
    }
}
