// SPDX-License-Identifier: GPL-2.0-only

//! Bounded post-edit topology and profile products shared by preview and authoritative insertion.

use super::graph::{Edge, RegionGraph};
use super::surface::{RoadEarthworkPlan, RoadSurfaceSystem};
use super::types::NodeType;
use super::{TransitNetwork, topology};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::zoning::ZoningSystem;
use godot::prelude::Vector3;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

#[cfg(test)]
mod tests;

/// Local dependency scope and timings from the shared junction/profile finalizer.
#[derive(Clone, Debug, Default)]
pub(crate) struct FinalizedRoadGeometry {
    /// Edges whose geometry or dependent lane data must be refreshed.
    pub(crate) dirty_edges: HashSet<usize>,
    /// Nodes used by the junction solve and clip rebuild.
    pub(crate) affected_nodes: HashSet<u32>,
    /// Time spent solving endpoint profile planes; zero when adopting a solved plan.
    pub(crate) profile_us: u128,
    /// Time spent rebuilding clips; zero when adopting a solved plan.
    pub(crate) clips_us: u128,
}

/// Ordered split provenance and pre-finalization lengths used by dependent-record migration.
#[derive(Clone, Debug)]
pub(super) struct PlannedRoadSplit {
    /// Local edge slot retained by the first split half.
    pub(super) edge_id: usize,
    /// Local edge slot appended for the second split half.
    pub(super) new_edge_id: usize,
    /// First-half length at the split, before junction finalization.
    pub(super) first_length_m: f32,
    /// Second-half length at the split, before junction finalization.
    pub(super) second_length_m: f32,
}

struct PlannedNode {
    source: Option<u32>,
    position: Vector3,
    kind: NodeType,
    canonical: u32,
    changed: bool,
}

struct PlannedEdge {
    source: Option<usize>,
    split_parent: Option<usize>,
    // Unchanged frontier edges need only an ID mapping, not another profile allocation.
    geometry: Option<Edge>,
}

/// Final local graph delta in deterministic preview-local ID order, never a whole-city snapshot.
///
/// Source/surface generations are checked by RoadEditPlan before adoption. Existing runtime
/// metadata remains live; only topology, profiles and clip ownership are installed from this plan.
pub(crate) struct RoadTopologyPlan {
    nodes: Vec<PlannedNode>,
    edges: Vec<PlannedEdge>,
    splits: Vec<PlannedRoadSplit>,
    dirty_edges: HashSet<usize>,
    affected_nodes: HashSet<u32>,
    earthworks: Option<Arc<RoadEarthworkPlan>>,
}

impl std::fmt::Debug for RoadTopologyPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoadTopologyPlan")
            .field("nodes", &self.nodes.len())
            .field("edges", &self.edges.len())
            .field("splits", &self.splits.len())
            .finish()
    }
}

impl RoadTopologyPlan {
    /// Read-only local terrain inputs produced from the same final road geometry as the preview.
    pub(crate) fn earthworks(&self) -> Option<&Arc<RoadEarthworkPlan>> {
        self.earthworks.as_ref()
    }

    /// Captures the already solved validation excerpt in O(local records + profile points).
    /// Incomplete mutation frontiers cannot be adopted; interactive callers must rebuild the plan.
    pub(super) fn capture(
        local: &RegionGraph,
        source: &RegionGraph,
        source_nodes: &HashMap<u32, u32>,
        source_edges: &HashMap<usize, usize>,
        finalized: FinalizedRoadGeometry,
        splits: Vec<PlannedRoadSplit>,
    ) -> Option<Self> {
        let mut nodes = local
            .nodes()
            .iter()
            .enumerate()
            .map(|(id, node)| PlannedNode {
                source: None,
                position: node.pos,
                kind: node.node_type,
                canonical: local.get_valid_node(id as u32),
                changed: true,
            })
            .collect::<Vec<_>>();
        for (&source_id, &local_id) in source_nodes {
            let node = &mut nodes[local_id as usize];
            node.source = Some(source_id);
            node.changed = node.canonical != local_id
                || node.position != source.node(source_id).pos
                || node.kind != source.node(source_id).node_type;
        }
        // A merge also clears restrictions on the surviving junction even if its pose/type
        // did not change. Retain that mutation, including its complete-incidence requirement.
        for local_id in 0..nodes.len() {
            let canonical = nodes[local_id].canonical as usize;
            if canonical != local_id {
                nodes[canonical].changed = true;
            }
        }
        // Solved junctions as well as moved/merged nodes need their complete live incidence.
        // An unchanged junction position alone does not prove a complete profile constraint set.
        // Inspect adjacency only, never the resident graph or a partial frontier solution.
        for (_, node) in nodes
            .iter()
            .enumerate()
            .filter(|(id, node)| node.changed || finalized.affected_nodes.contains(&(*id as u32)))
        {
            if let Some(id) = node.source {
                if source
                    .node_adjacency(id)
                    .iter()
                    .any(|edge| !source.edge(*edge).deleted && !source_edges.contains_key(edge))
                {
                    return None;
                }
            }
        }
        let mut local_sources = vec![None; local.edge_count()];
        for (&source_id, &local_id) in source_edges {
            local_sources[local_id] = Some(source_id);
        }
        let mut parents = vec![None; local.edge_count()];
        for split in &splits {
            parents[split.new_edge_id] = Some(split.edge_id);
        }
        let mut dirty_edges = finalized.dirty_edges;
        let edges = local
            .edges()
            .iter()
            .enumerate()
            .map(|(id, edge)| {
                let source_id = local_sources[id];
                let changed = source_id.is_none_or(|source_id| {
                    let before = source.edge(source_id);
                    nodes[edge.start_node as usize].source != Some(before.start_node)
                        || nodes[edge.end_node as usize].source != Some(before.end_node)
                        || edge.geometry != before.geometry
                        || edge.physical_geometry != before.physical_geometry
                        || edge.start_clip != before.start_clip
                        || edge.end_clip != before.end_clip
                        || edge.deleted != before.deleted
                });
                if changed {
                    dirty_edges.insert(id);
                }
                PlannedEdge {
                    source: source_id,
                    split_parent: parents[id],
                    geometry: changed.then(|| edge.clone()),
                }
            })
            .collect();
        Some(Self {
            nodes,
            edges,
            splits,
            dirty_edges,
            affected_nodes: finalized.affected_nodes,
            earthworks: None,
        })
    }

    /// Retains the existing stamper's local results without changing source or visual terrain.
    pub(super) fn compile_earthworks(
        &mut self,
        local: &RegionGraph,
        surface: &RoadSurfaceSystem,
        source: &RegionGraph,
        source_surface: &RoadSurfaceSystem,
        terrain: &TerrainSystem,
    ) {
        // Predict exactly the append order used by adopt_road_topology_plan, not local IDs.
        let mut next_edge = source.edge_count();
        let edge_ids = self
            .edges
            .iter()
            .map(|edge| {
                edge.source.unwrap_or_else(|| {
                    let id = next_edge;
                    next_edge += 1;
                    id
                })
            })
            .collect::<Vec<_>>();
        let mut next_node = source.node_count() as u32;
        let node_ids = self
            .nodes
            .iter()
            .map(|node| {
                node.source.unwrap_or_else(|| {
                    let id = next_node;
                    next_node += 1;
                    id
                })
            })
            .collect::<Vec<_>>();
        let replaced_edges = self
            .edges
            .iter()
            .enumerate()
            .filter_map(|(local_id, edge)| {
                (edge.geometry.is_some()
                    || surface.compiled_visual_span_pieces.contains_key(&local_id))
                .then_some(edge.source)
                .flatten()
            })
            .collect();
        let replaced_nodes = self
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(local_id, node)| {
                (node.changed
                    || self.affected_nodes.contains(&(local_id as u32))
                    || surface
                        .compiled_visual_node_pieces
                        .contains_key(&(local_id as u32)))
                .then_some(node.source)
                .flatten()
            })
            .collect();
        self.earthworks = surface
            .plan_earthwork_overlay(
                local,
                source_surface,
                source,
                terrain,
                &edge_ids,
                &node_ids,
                &replaced_edges,
                &replaced_nodes,
            )
            .map(Arc::new);
    }

    /// Returns the bounded source records required by the existing undo journal.
    pub(crate) fn source_topology_ids(&self) -> (HashSet<usize>, HashSet<u32>) {
        (
            self.edges.iter().filter_map(|edge| edge.source).collect(),
            self.nodes.iter().filter_map(|node| node.source).collect(),
        )
    }

    /// Checks mapped source identities before any authoritative mutation; revisions are checked
    /// by the owning RoadEditPlan. Dynamic congestion is intentionally not a geometry dependency.
    pub(crate) fn source_identities_match(&self, graph: &RegionGraph) -> bool {
        self.nodes
            .iter()
            .filter_map(|node| node.source)
            .all(|id| (id as usize) < graph.node_count() && graph.get_valid_node(id) == id)
            && self
                .edges
                .iter()
                .filter_map(|edge| edge.source)
                .all(|id| id < graph.edge_count() && !graph.edge(id).deleted)
    }
}

impl TransitNetwork {
    /// Records profile intent, separately from topology dirtiness, in the local edit ledger.
    pub(crate) fn mark_road_profile_authored(&mut self, edge_idx: usize) {
        self.profile_authored_edges.insert(edge_idx);
    }

    /// Solves the same local junction/profile/clip scope for preview and unplanned commits.
    pub(crate) fn finalize_road_geometry(
        &mut self,
        graph: &mut RegionGraph,
    ) -> FinalizedRoadGeometry {
        let mut dirty_edges = std::mem::take(&mut self.bulk_dirty_edges);
        let authored_edges = std::mem::take(&mut self.profile_authored_edges);
        let affected_nodes = self.bulk_surface_profile_nodes(graph, &dirty_edges);
        let start = Instant::now();
        dirty_edges.extend(graph.finalize_junction_endpoint_profiles_for_edges(
            &affected_nodes,
            &dirty_edges,
            &authored_edges,
        ));
        let profile_us = start.elapsed().as_micros();
        self.mark_surface_dirty_from_sets(graph, &dirty_edges, &affected_nodes);
        let start = Instant::now();
        graph.rebuild_intersection_clips_for_nodes(&affected_nodes);
        FinalizedRoadGeometry {
            dirty_edges,
            affected_nodes,
            profile_us,
            clips_us: start.elapsed().as_micros(),
        }
    }

    /// Installs a generation-checked solved delta without repeating intersections or profile solves.
    /// Work is O(local profile points + sum(local degree * log(degree))
    /// + changed edges * log(resident edges)),
    /// plus the existing split-dependent migration cost. No full graph scan/copy is introduced.
    pub(crate) fn adopt_road_topology_plan(
        &mut self,
        graph: &mut RegionGraph,
        plan: &RoadTopologyPlan,
        zoning: &ZoningSystem,
        allocator: &mut BuildingAllocator,
    ) -> Option<FinalizedRoadGeometry> {
        if !plan.source_identities_match(graph) {
            return None;
        }
        let mut node_ids = Vec::with_capacity(plan.nodes.len());
        for node in &plan.nodes {
            let id = node
                .source
                .unwrap_or_else(|| graph.add_node(node.position, node.kind));
            node_ids.push(id);
            if node.source.is_some() && node.changed {
                let old_pos = graph.node(id).pos;
                self.mark_point_dirty(old_pos);
                self.mark_surface_point_dirty(old_pos);
                graph.remove_node_from_spatial_index(id, old_pos);
                graph.nodes[id as usize].pos = node.position;
                graph.nodes[id as usize].node_type = node.kind;
                graph.nodes[id as usize].lane_connections.clear();
                graph.add_node_to_spatial_index(id);
            }
        }
        for (local, node) in plan.nodes.iter().enumerate() {
            if node.canonical != local as u32 {
                graph
                    .node_aliases
                    .insert(node_ids[local], node_ids[node.canonical as usize]);
                // The final solve scope contains canonical IDs only. Explicitly evict the
                // removed node's old ownership/coverage before publishing its survivor.
                self.road_surface.mark_node_dirty(graph, node_ids[local]);
            }
        }
        let mut edge_ids = Vec::with_capacity(plan.edges.len());
        // Allocate exact local-to-live mappings before applying endpoint incidence.
        for edge in &plan.edges {
            let id = edge.source.unwrap_or_else(|| {
                let id = graph.edges.len();
                graph.edges.push(Edge::default());
                id
            });
            edge_ids.push(id);
        }
        let replaced_edges = plan
            .edges
            .iter()
            .filter(|edge| edge.geometry.is_some())
            .filter_map(|edge| edge.source)
            .collect::<HashSet<_>>();
        // Remove old incidence once per local node, not once per changed incident edge.
        // This avoids quadratic adjacency walks at high-degree junctions.
        for id in &node_ids {
            graph.adjacency[*id as usize].retain(|edge| !replaced_edges.contains(edge));
        }
        for (local_id, planned) in plan.edges.iter().enumerate() {
            let Some(geometry) = &planned.geometry else {
                continue;
            };
            let id = edge_ids[local_id];
            if planned.source.is_some() {
                self.cch_dirty_chunks.extend(graph.get_edge_chunks(id));
                graph.remove_from_spatial_index(id);
            }
            let runtime_source = planned
                .source
                .or_else(|| planned.split_parent.map(|parent| edge_ids[parent]));
            let mut edge = geometry.clone();
            if let Some(source) = runtime_source {
                let live = graph.edge(source);
                edge.current_congestion = live.current_congestion;
                edge.no_building_spawn = live.no_building_spawn;
                edge.vehicle_frontage_access = live.vehicle_frontage_access;
                let speed_changed = edge.speed_limit != live.speed_limit;
                edge.speed_limit = live.speed_limit;
                if speed_changed {
                    edge.base_cost =
                        crate::simulation::pathing::cost::CostCalculator::calculate_costs(&edge).0;
                }
            }
            edge.start_node = node_ids[geometry.start_node as usize];
            edge.end_node = node_ids[geometry.end_node as usize];
            graph.edges[id] = edge;
            if !graph.edge(id).deleted {
                let edge = &graph.edges[id];
                graph.adjacency[edge.start_node as usize].push(id);
                graph.adjacency[edge.end_node as usize].push(id);
                graph.add_to_spatial_index(id);
                self.cch_dirty_chunks.extend(graph.get_edge_chunks(id));
            }
        }
        // Keep incidence order deterministic, including unmoved edges beyond the excerpt.
        for id in &node_ids {
            graph.adjacency[*id as usize].sort_unstable();
            graph.adjacency[*id as usize].dedup();
        }
        for split in &plan.splits {
            topology::migrate_split_dependents(
                edge_ids[split.edge_id],
                edge_ids[split.new_edge_id],
                (split.first_length_m / zoning.config.zone_cell_m).floor() as usize,
                split.first_length_m,
                split.second_length_m,
                zoning,
                allocator,
                &mut self.road_edit_split_undo,
            );
        }
        let dirty_edges = plan.dirty_edges.iter().map(|id| edge_ids[*id]).collect();
        let affected_nodes = plan
            .affected_nodes
            .iter()
            .map(|id| node_ids[*id as usize])
            .collect();
        self.mark_surface_dirty_from_sets(graph, &dirty_edges, &affected_nodes);
        self.bulk_dirty_edges.clear();
        self.profile_authored_edges.clear();
        self.road_surface
            .enqueue_planned_earthworks(plan.earthworks.clone());
        Some(FinalizedRoadGeometry {
            dirty_edges,
            affected_nodes,
            ..Default::default()
        })
    }
}
