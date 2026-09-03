// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: mod.rs
//  script_path: rust/src/simulation/network/lanes/mod.rs
//  module_name: mod
//  version: 0.1.0
//  description: Lane geometry, lane connectivity, and per-lane derived
//           planning caches.
//  kind: module
//  spec: none
//  internal_dependencies: []
//  external_dependencies: []
//  features: []
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-27
// ========================================================================

//! Lane geometry, lane connectivity, and per-lane derived planning caches.

use godot::prelude::*;
use std::collections::{BTreeSet, HashMap};

use crate::simulation::network::graph::{RegionGraph, TurnSet};
use crate::simulation::network::surface::{CURB_STEP_HEIGHT_M, RoadSurfaceSystem};
use crate::simulation::network::types::TransitType;
use crate::simulation::terrain::TerrainSystem;

/// Which connector movements through a junction cross each other.
pub mod conflicts;
/// Lane geometry generation and offset calculations.
pub mod geometry;
/// Pedestrian sidewalk and crosswalk connection logic.
pub mod pedestrian_junctions;
/// Full and incremental lane system rebuild orchestration.
pub mod rebuild;
/// High-level logic for vehicle connections at junctions.
pub mod vehicle_junctions;

// ========================================================================
// LANE TYPES
// ========================================================================

/// Types of travel lanes supported by the network.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LaneType {
    /// Lane for motorized vehicles.
    Vehicle,
    /// Lane for pedestrians.
    Foot,
}

/// Asphalt segment that owns one visible zebra crossing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrosswalkMarking {
    /// Road edge whose junction mouth contains the crossing.
    pub edge_id: usize,
    /// First asphalt-edge endpoint of the stripe corridor.
    pub start: Vector3,
    /// Opposite asphalt-edge endpoint of the stripe corridor.
    pub end: Vector3,
}

// ========================================================================
// ONE LANE
// ========================================================================

/// A single travel lane through a road or intersection.
#[derive(Clone)]
pub struct Lane {
    /// The parent road edge ID. `usize::MAX` for intersection connections.
    pub edge_id: usize,
    /// Direction relative to the edge geometry.
    pub is_fwd: bool,
    /// Lane index (0 is innermost).
    pub lane_idx: i8,
    /// The physical path of the lane.
    pub geometry: Vec<Vector3>,
    /// Total length in meters.
    pub length: f32,
    /// Low-frequency cached frontage congestion penalty in seconds used only by exact access planning.
    pub frontage_delay_penalty_s: f32,
    /// Cumulative distance at each geometry vertex: `cum_dist[i]` = distance from `geometry[0]` to `geometry[i]`.
    /// Used for O(log N) position interpolation via binary search.
    pub cum_dist: Vec<f32>,
    /// The travel type of this lane.
    pub lane_type: LaneType,
    /// Road edge crossed by this pedestrian connection in either travel direction.
    pub crosswalk_edge_id: Option<usize>,
    /// Exact asphalt-only segment used to render this crossing, when owned by this lane.
    pub crosswalk_marking: Option<CrosswalkMarking>,
    /// Reachable lanes from the end of this lane.
    pub next_lanes: Vec<usize>,
    /// The junction node this connection lane belongs to. `usize::MAX` for road lanes.
    pub node_id: usize,
    /// Which movements this lane permits at the node it ends on.
    ///
    /// Carried down from the authored `LaneSpec`. An ordinary travel lane
    /// permits everything and is left empty, which reads as no restriction; a
    /// turn pocket permits one movement, and that is what makes it a pocket
    /// rather than a lane that happens to be short.
    pub turns: TurnSet,
    /// The fraction of the edge this lane exists over, as `(start, end)`.
    ///
    /// `(0.0, 1.0)` for a lane running the whole edge. A turn pocket opens part
    /// way along, so a car has to be past its start before it can move into it.
    pub extent: (f32, f32),
}

impl Default for Lane {
    fn default() -> Self {
        Self {
            edge_id: usize::MAX,
            is_fwd: true,
            lane_idx: 0,
            geometry: Vec::new(),
            length: 0.0,
            frontage_delay_penalty_s: 0.0,
            cum_dist: Vec::new(),
            lane_type: LaneType::Vehicle,
            crosswalk_edge_id: None,
            crosswalk_marking: None,
            next_lanes: Vec::new(),
            node_id: usize::MAX,
            // Empty permits everything: an ordinary lane restricts nothing, and
            // a default that restricted movements would strand every car built
            // before pockets existed.
            turns: TurnSet(0),
            extent: (0.0, 1.0),
        }
    }
}

// ========================================================================
// THE LANE SYSTEM
// ========================================================================

/// System for managing road and intersection lanes.
pub struct LaneSystem {
    /// All active lanes.
    pub lanes: Vec<Lane>,
    /// Mapping of edge IDs to their constituent lanes.
    pub edge_lanes: HashMap<usize, Vec<usize>>,
    /// Mapping of node IDs to their connection lane indices (crosswalks and vehicle turns).
    pub node_lanes: HashMap<usize, Vec<usize>>,
    /// Crossing movements per node, so a turning car can yield to what it cuts across.
    ///
    /// Computed when lanes are rebuilt rather than per tick, because it is pure
    /// geometry over connectors that only change when the junction does.
    pub node_conflicts: HashMap<usize, conflicts::JunctionConflicts>,
}

impl LaneSystem {
    /// Creates a new, empty lane system.
    pub fn new() -> Self {
        Self {
            lanes: Vec::new(),
            edge_lanes: HashMap::new(),
            node_lanes: HashMap::new(),
            node_conflicts: HashMap::new(),
        }
    }

    /// Clears all lanes and structural mappings.
    pub fn clear(&mut self) {
        self.lanes.clear();
        self.edge_lanes.clear();
        self.node_lanes.clear();
        self.node_conflicts.clear();
    }

    /// Recomputes the crossing-movement table for `node_id`.
    ///
    /// Call after the connectors at a node are built or rebuilt. Only vehicle
    /// connectors participate: a crosswalk is governed by its own rules and a
    /// road lane is not inside the junction box.
    pub fn rebuild_node_conflicts(&mut self, node_id: usize) {
        let Some(lane_ids) = self.node_lanes.get(&node_id) else {
            self.node_conflicts.remove(&node_id);
            return;
        };
        let vehicle: Vec<usize> = lane_ids
            .iter()
            .copied()
            .filter(|&id| {
                self.lanes
                    .get(id)
                    .is_some_and(|l| l.lane_type == LaneType::Vehicle)
            })
            .collect();

        let table = conflicts::build_junction_conflicts(&vehicle, &self.lanes);
        crate::traffic_log!(
            "[JUNCTION_CONFLICTS] node={} vehicle_connectors={} lanes_with_conflicts={}",
            node_id,
            vehicle.len(),
            table.len(),
        );
        if table.is_empty() {
            self.node_conflicts.remove(&node_id);
        } else {
            self.node_conflicts.insert(node_id, table);
        }
    }

    /// Crossing movements for `lane_id` at `node_id`, empty when there are none.
    #[inline]
    pub fn conflicting_lanes(&self, node_id: usize, lane_id: usize) -> &[usize] {
        self.node_conflicts
            .get(&node_id)
            .map(|c| c.conflicting(lane_id))
            .unwrap_or(&[])
    }

    /// Movements sharing a start point with `lane_id` at `node_id`.
    #[inline]
    pub fn co_entrant_lanes(&self, node_id: usize, lane_id: usize) -> &[usize] {
        self.node_conflicts
            .get(&node_id)
            .map(|c| c.co_entrants(lane_id))
            .unwrap_or(&[])
    }

    /// Movements `lane_id` must give way to at `node_id`.
    #[inline]
    pub fn yielding_lanes(&self, node_id: usize, lane_id: usize) -> &[usize] {
        self.node_conflicts
            .get(&node_id)
            .map(|c| c.yields_to(lane_id))
            .unwrap_or(&[])
    }

    /// Retrieve the global `lane_id` given an `edge_idx` and a local `lane_idx`.
    pub fn get_lane_id(&self, edge_idx: usize, lane_idx: usize) -> Option<usize> {
        self.edge_lanes.get(&edge_idx).and_then(|lanes| {
            lanes
                .iter()
                .find(|&&id| self.lanes[id].lane_idx == lane_idx as i8)
                .copied()
        })
    }

    pub(crate) fn sync_heights_to_visible_surface(
        &mut self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        road_surface: &RoadSurfaceSystem,
    ) {
        for lane in &mut self.lanes {
            sync_lane_height_to_visible_surface(lane, graph, terrain, road_surface);
        }
    }

    pub(crate) fn sync_heights_to_visible_surface_for_owners(
        &mut self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        road_surface: &RoadSurfaceSystem,
        edge_indices: &[usize],
        node_ids: &[u32],
    ) {
        let mut lane_ids = BTreeSet::new();
        for edge_idx in edge_indices {
            if let Some(ids) = self.edge_lanes.get(edge_idx) {
                lane_ids.extend(ids.iter().copied());
            }
        }
        for node_id in node_ids {
            if let Some(ids) = self.node_lanes.get(&(*node_id as usize)) {
                lane_ids.extend(ids.iter().copied());
            }
        }
        for lane_id in lane_ids {
            let Some(lane) = self.lanes.get_mut(lane_id) else {
                continue;
            };
            sync_lane_height_to_visible_surface(lane, graph, terrain, road_surface);
        }
    }
}

fn sync_lane_height_to_visible_surface(
    lane: &mut Lane,
    graph: &RegionGraph,
    terrain: &TerrainSystem,
    road_surface: &RoadSurfaceSystem,
) {
    if lane.geometry.is_empty() {
        return;
    }

    let sidewalk_base_offset = if lane_is_road_sidewalk(lane, graph) {
        CURB_STEP_HEIGHT_M
    } else {
        0.0
    };
    let lane_type = lane.lane_type;

    let mut changed = false;
    for point in &mut lane.geometry {
        let Some(surface_y) =
            lane_visible_surface_height(lane_type, graph, terrain, road_surface, point)
        else {
            continue;
        };
        let target_y = surface_y - sidewalk_base_offset;
        if (point.y - target_y).abs() > 1e-4 {
            point.y = target_y;
            changed = true;
        }
    }

    if changed {
        lane.cum_dist = geometry::build_cum_dist(&lane.geometry);
        lane.length = lane.cum_dist.last().copied().unwrap_or(0.0);
    }
}

fn lane_is_road_sidewalk(lane: &Lane, graph: &RegionGraph) -> bool {
    if lane.edge_id == usize::MAX
        || lane.lane_type != LaneType::Foot
        || lane.lane_idx.unsigned_abs() != 100
    {
        return false;
    }
    graph
        .get_edge(lane.edge_id)
        .is_some_and(|edge| edge.primary_type == TransitType::Road)
}

fn lane_visible_surface_height(
    lane_type: LaneType,
    graph: &RegionGraph,
    terrain: &TerrainSystem,
    road_surface: &RoadSurfaceSystem,
    point: &Vector3,
) -> Option<f32> {
    if lane_type == LaneType::Vehicle {
        road_surface
            .sample_visible_carriageway_height(graph, terrain, point.x, point.z)
            .or_else(|| {
                road_surface.sample_visible_surface_height(graph, terrain, point.x, point.z)
            })
    } else {
        road_surface.sample_visible_surface_height(graph, terrain, point.x, point.z)
    }
}

/// Unit tests for the lane system.
#[cfg(test)]
pub mod tests;
