// SPDX-License-Identifier: GPL-2.0-only

//! Bounded post-edit topology and profile products shared by preview and authoritative insertion.

use super::graph::{Edge, RegionGraph};
use super::roadbed::{PieceId, RoadbedSystem};
use super::types::NodeType;
use super::{TransitNetwork, topology};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::zoning::ZoningSystem;
use godot::prelude::Vector3;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::Instant;

mod input;
mod support;
#[cfg(test)]
mod tests;
mod validation;

pub(crate) use input::{PreparedRoadInput, prepare_road_input_for_tool};
pub(crate) use support::{RebuiltPieces, plan_road_edit};
pub(crate) use validation::{
    RoadPreviewValidation, validate_road_candidate_fast, validate_road_edit,
};

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
    /// The dirty edges whose geometry, clips, endpoints or deletion changed, as an adopted
    /// plan knows them; `None` when every dirty edge may have changed. Cell lots follow these.
    pub(crate) changed_edges: Option<BTreeSet<usize>>,
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
/// Source/roadbed generations are checked by RoadEditPlan before adoption. Existing runtime
/// metadata remains live; only topology, profiles and clip ownership are installed from this plan.
pub(crate) struct RoadTopologyPlan {
    nodes: Vec<PlannedNode>,
    edges: Vec<PlannedEdge>,
    splits: Vec<PlannedRoadSplit>,
    dirty_edges: HashSet<usize>,
    affected_nodes: HashSet<u32>,
    // Canonical local nodes at the stroke's first and last point.
    stroke_ends: [u32; 2],
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
    /// Captures the already solved validation excerpt in O(local records + profile points).
    /// Incomplete mutation frontiers cannot be adopted; interactive callers must rebuild the plan.
    pub(super) fn capture(
        local: &RegionGraph,
        source: &RegionGraph,
        source_nodes: &HashMap<u32, u32>,
        source_edges: &HashMap<usize, usize>,
        finalized: FinalizedRoadGeometry,
        splits: Vec<PlannedRoadSplit>,
        stroke_ends: [u32; 2],
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
            if let Some(id) = node.source
                && source
                    .node_adjacency(id)
                    .iter()
                    .any(|edge| !source.edge(*edge).deleted && !source_edges.contains_key(edge))
            {
                return None;
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
            stroke_ends: stroke_ends.map(|end| local.get_valid_node(end)),
        })
    }

    /// The live pieces this plan rebuilds, the ones [`AppliedRoadTopology::mark_pieces`] marks
    /// (`ROAD-68` open question 16): the spans of its dirty source edges and the pieces of its
    /// solved and merged source nodes, with the source edges it splits and the source nodes the
    /// stroke's ends join. O(local records).
    pub(crate) fn rebuilt_pieces(&self) -> RebuiltPieces {
        let spans = self
            .dirty_edges
            .iter()
            .filter_map(|&edge| self.edges[edge].source)
            .map(PieceId::Span);
        let nodes = self
            .nodes
            .iter()
            .enumerate()
            .filter(|&(id, node)| {
                node.canonical != id as u32 || self.affected_nodes.contains(&(id as u32))
            })
            .filter_map(|(_, node)| node.source)
            .map(PieceId::Node);
        // A split half appended by the edit names its parent; the root is the live edge.
        let source_root = |mut edge: usize| loop {
            let planned = &self.edges[edge];
            if planned.source.is_some() {
                return planned.source;
            }
            edge = planned.split_parent?;
        };
        let crossed = self
            .splits
            .iter()
            .filter_map(|split| source_root(split.edge_id))
            .collect();
        let joined_ends = self.stroke_ends.map(|end| {
            self.nodes
                .iter()
                .filter(|node| node.canonical == end)
                .find_map(|node| node.source)
        });
        RebuiltPieces::new(spans.chain(nodes).collect(), crossed, joined_ends)
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
    /// Solves the same local junction/profile/clip scope for preview and unplanned commits; the
    /// blends keep within the source band of `terrain`
    /// ([`RegionGraph::finalize_junction_endpoint_profiles`]).
    pub(crate) fn finalize_road_geometry(
        &mut self,
        graph: &mut RegionGraph,
        terrain: Option<&TerrainSystem>,
    ) -> FinalizedRoadGeometry {
        let mut dirty_edges = std::mem::take(&mut self.bulk_dirty_edges);
        let affected_nodes = self.bulk_surface_profile_nodes(graph, &dirty_edges);
        // XZ -> layout -> clips -> profile solve: the solve pins each arm up to its layout mouth.
        let start = Instant::now();
        graph.rebuild_intersection_clips_for_nodes(&affected_nodes);
        let clips_us = start.elapsed().as_micros();
        let start = Instant::now();
        graph.finalize_junction_endpoint_profiles(&affected_nodes, terrain, &mut dirty_edges);
        let profile_us = start.elapsed().as_micros();
        self.mark_surface_dirty_from_sets(&dirty_edges, &affected_nodes);
        FinalizedRoadGeometry {
            dirty_edges,
            affected_nodes,
            profile_us,
            clips_us,
            changed_edges: None,
        }
    }

    /// Installs a generation-checked solved delta without repeating intersections or profile solves.
    /// Work is O(local profile points + sum(local degree * log(degree))
    /// + changed edges * log(resident edges)),
    ///   plus the existing split-dependent migration cost. No full graph scan/copy is introduced.
    pub(crate) fn adopt_road_topology_plan(
        &mut self,
        graph: &mut RegionGraph,
        plan: &RoadTopologyPlan,
        zoning: &ZoningSystem,
        allocator: &mut BuildingAllocator,
    ) -> Option<FinalizedRoadGeometry> {
        let applied = plan.apply_to_graph(graph)?;
        self.cch_dirty_chunks
            .extend(applied.cch_chunks.iter().copied());
        let edge_ids = &applied.edge_ids;
        for split in &plan.splits {
            topology::migrate_split_dependents(
                graph,
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
        let changed_edges = plan
            .edges
            .iter()
            .zip(edge_ids)
            .filter(|(planned, _)| planned.geometry.is_some())
            .map(|(_, &id)| id)
            .collect();
        applied.mark_pieces(&mut self.roadbed);
        self.bulk_dirty_edges.clear();
        Some(FinalizedRoadGeometry {
            dirty_edges: applied.dirty_edges,
            affected_nodes: applied.affected_nodes,
            changed_edges: Some(changed_edges),
            ..Default::default()
        })
    }
}

/// Where a plan's records went in a graph, and what the edit dirtied there.
pub(crate) struct AppliedRoadTopology {
    // Live id of each local edge.
    edge_ids: Vec<usize>,
    /// Edges whose geometry changed, in live ids.
    pub(crate) dirty_edges: HashSet<usize>,
    /// Nodes the junction solve used, in live ids.
    pub(crate) affected_nodes: HashSet<u32>,
    // Nodes merged into another; their pieces drop.
    merged_nodes: Vec<u32>,
    /// Routing chunks under the moved nodes and the replaced edges, before and after.
    pub(crate) cch_chunks: Vec<(i32, i32)>,
}

impl AppliedRoadTopology {
    /// Marks the pieces the edit rebuilds: its changed edges, the solved nodes and the merged
    /// nodes, whose pieces drop. The commit and the preview mark through this one function.
    pub(crate) fn mark_pieces(&self, roadbed: &mut RoadbedSystem) {
        for &edge_idx in &self.dirty_edges {
            roadbed.mark_edge_dirty(edge_idx);
        }
        for &node in self.affected_nodes.iter().chain(&self.merged_nodes) {
            roadbed.mark_node_dirty(node);
        }
    }
}

impl RoadTopologyPlan {
    /// Installs the plan's topology, profiles and clips into `graph`: the one implementation the
    /// commit's live graph and the preview's scratch replica share (`ROAD-68-E`), so both end
    /// with the same records and dirty the same pieces. Runtime fields of replaced edges stay the
    /// graph's own. `None`, with nothing changed, when a source record no longer exists.
    /// O(local profile points + sum(local degree * log(degree)) + changed edges * log(edges)).
    pub(crate) fn apply_to_graph(&self, graph: &mut RegionGraph) -> Option<AppliedRoadTopology> {
        if !self.source_identities_match(graph) {
            return None;
        }
        let mut cch_chunks = Vec::new();
        let mut node_ids = Vec::with_capacity(self.nodes.len());
        for node in &self.nodes {
            let id = node
                .source
                .unwrap_or_else(|| graph.add_node(node.position, node.kind));
            node_ids.push(id);
            if node.source.is_some() && node.changed {
                let old_pos = graph.node(id).pos;
                cch_chunks.push(RegionGraph::get_chunk_coords(old_pos));
                graph.remove_node_from_spatial_index(id, old_pos);
                graph.nodes[id as usize].pos = node.position;
                graph.nodes[id as usize].node_type = node.kind;
                graph.nodes[id as usize].lane_connections.clear();
                graph.add_node_to_spatial_index(id);
            }
        }
        let mut merged_nodes = Vec::new();
        for (local, node) in self.nodes.iter().enumerate() {
            if node.canonical != local as u32 {
                graph
                    .node_aliases
                    .insert(node_ids[local], node_ids[node.canonical as usize]);
                // The final solve scope contains canonical IDs only. Explicitly evict the
                // removed node's old ownership/coverage before publishing its survivor.
                merged_nodes.push(node_ids[local]);
            }
        }
        let mut edge_ids = Vec::with_capacity(self.edges.len());
        // Allocate exact local-to-live mappings before applying endpoint incidence.
        for edge in &self.edges {
            let id = edge.source.unwrap_or_else(|| {
                let id = graph.edges.len();
                graph.edges.push(Edge::default());
                id
            });
            edge_ids.push(id);
        }
        let replaced_edges = self
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
        for (local_id, planned) in self.edges.iter().enumerate() {
            let Some(geometry) = &planned.geometry else {
                continue;
            };
            let id = edge_ids[local_id];
            if planned.source.is_some() {
                cch_chunks.extend(graph.get_edge_chunks(id));
                graph.remove_from_spatial_index(id);
            }
            let runtime_source = planned
                .source
                .or_else(|| planned.split_parent.map(|parent| edge_ids[parent]));
            let mut edge = geometry.clone();
            if let Some(source) = runtime_source {
                let live = graph.edge(source);
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
                cch_chunks.extend(graph.get_edge_chunks(id));
            }
        }
        // Keep incidence order deterministic, including unmoved edges beyond the excerpt.
        for id in &node_ids {
            graph.adjacency[*id as usize].sort_unstable();
            graph.adjacency[*id as usize].dedup();
        }
        let dirty_edges = self.dirty_edges.iter().map(|id| edge_ids[*id]).collect();
        let affected_nodes = self
            .affected_nodes
            .iter()
            .map(|id| node_ids[*id as usize])
            .collect();
        Some(AppliedRoadTopology {
            edge_ids,
            dirty_edges,
            affected_nodes,
            merged_nodes,
            cch_chunks,
        })
    }
}
