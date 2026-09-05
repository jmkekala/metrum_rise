//! Lane geometry, lane connectivity, and per-lane derived planning caches.

use godot::prelude::*;
use std::collections::{BTreeSet, HashMap};
use std::time::Instant;

use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::surface::{
    CURB_STEP_HEIGHT_M, RoadLaneSurfaceQuery, RoadSurfaceSystem,
};
use crate::simulation::network::types::TransitType;
use crate::simulation::terrain::TerrainSystem;

/// Lane geometry generation and offset calculations.
pub mod geometry;
/// Pedestrian sidewalk and crosswalk connection logic.
pub mod pedestrian_junctions;
/// Full and incremental lane system rebuild orchestration.
pub mod rebuild;
/// High-level logic for vehicle connections at junctions.
pub mod vehicle_junctions;

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
        }
    }
}

/// System for managing road and intersection lanes.
pub struct LaneSystem {
    /// All active lanes.
    pub lanes: Vec<Lane>,
    /// Mapping of edge IDs to their constituent lanes.
    pub edge_lanes: HashMap<usize, Vec<usize>>,
    /// Mapping of node IDs to their connection lane indices (crosswalks and vehicle turns).
    pub node_lanes: HashMap<usize, Vec<usize>>,
}

impl LaneSystem {
    /// Creates a new, empty lane system.
    pub fn new() -> Self {
        Self {
            lanes: Vec::new(),
            edge_lanes: HashMap::new(),
            node_lanes: HashMap::new(),
        }
    }

    /// Clears all lanes and structural mappings.
    pub fn clear(&mut self) {
        self.lanes.clear();
        self.edge_lanes.clear();
        self.node_lanes.clear();
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
        let road_debug = crate::debug::category_enabled("road");
        let mut edge_lane_ids = BTreeSet::new();
        for edge_idx in edge_indices {
            if let Some(ids) = self.edge_lanes.get(edge_idx) {
                edge_lane_ids.extend(ids.iter().copied());
            }
        }
        let edge_point_count = road_debug.then(|| {
            edge_lane_ids
                .iter()
                .filter_map(|&lane_id| self.lanes.get(lane_id))
                .map(|lane| lane.geometry.len())
                .sum::<usize>()
        });
        let edge_start = road_debug.then(Instant::now);
        for lane_id in edge_lane_ids.iter().copied() {
            let Some(lane) = self.lanes.get_mut(lane_id) else {
                continue;
            };
            sync_lane_height_to_visible_surface(lane, graph, terrain, road_surface);
        }
        let edge_ms = edge_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);

        let mut node_lane_ids = BTreeSet::new();
        for node_id in node_ids {
            if let Some(ids) = self.node_lanes.get(&(*node_id as usize)) {
                node_lane_ids.extend(ids.iter().copied());
            }
        }
        let node_point_count = road_debug.then(|| {
            node_lane_ids
                .iter()
                .filter_map(|&lane_id| self.lanes.get(lane_id))
                .map(|lane| lane.geometry.len())
                .sum::<usize>()
        });
        let node_start = road_debug.then(Instant::now);
        for lane_id in node_lane_ids.iter().copied() {
            let Some(lane) = self.lanes.get_mut(lane_id) else {
                continue;
            };
            sync_lane_height_to_visible_surface(lane, graph, terrain, road_surface);
        }
        if road_debug {
            crate::debug_log!(
                "road",
                "lane_surface_sync edge_lanes={} edge_points={} edge_ms={:.3} node_lanes={} node_points={} node_ms={:.3}",
                edge_lane_ids.len(),
                edge_point_count.unwrap_or(0),
                edge_ms,
                node_lane_ids.len(),
                node_point_count.unwrap_or(0),
                node_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
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
    let edge_id = lane.edge_id;
    let node_id = lane.node_id;
    let owner_query = road_surface.lane_owner_surface_query(
        graph,
        terrain,
        edge_id,
        node_id,
        lane_type == LaneType::Vehicle,
    );

    let mut changed = false;
    for point in &mut lane.geometry {
        let Some(surface_y) = lane_visible_surface_height(
            lane_type,
            graph,
            terrain,
            road_surface,
            &owner_query,
            point,
        ) else {
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
    owner_query: &RoadLaneSurfaceQuery<'_>,
    point: &Vector3,
) -> Option<f32> {
    let carriageway_only = lane_type == LaneType::Vehicle;
    owner_query.sample_height(point.x, point.z).or_else(|| {
        if carriageway_only {
            road_surface
                .sample_visible_carriageway_height(graph, terrain, point.x, point.z)
                .or_else(|| {
                    road_surface.sample_visible_surface_height(graph, terrain, point.x, point.z)
                })
        } else {
            road_surface.sample_visible_surface_height(graph, terrain, point.x, point.z)
        }
    })
}

/// Unit tests for the lane system.
#[cfg(test)]
pub mod tests;
