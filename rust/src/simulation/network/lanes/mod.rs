// SPDX-License-Identifier: GPL-2.0-only

//! Lane geometry, lane connectivity, and per-lane derived planning caches.

use godot::prelude::*;
use std::collections::{BTreeSet, HashMap};
use std::time::Instant;

use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::roadbed::{
    ArmKey, CURB_STEP_HEIGHT_M, EdgeEnd, LaneSurfaceQuery, RoadbedSystem,
};
use crate::simulation::network::types::TransitType;

mod bounds;
/// Lane geometry generation and offset calculations.
pub mod geometry;
mod identity;
/// Pedestrian sidewalk and crosswalk connection logic.
pub mod pedestrian_junctions;
/// Full and incremental lane system rebuild orchestration.
pub mod rebuild;
mod successors;
/// High-level logic for vehicle connections at junctions.
pub mod vehicle_junctions;

pub(crate) use bounds::LanePlanarBox;
pub(crate) use identity::LaneIdentity;
pub use successors::LaneSuccessors;

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

impl CrosswalkMarking {
    /// The marking of arm `key`'s crossing from its kerb points, right side first (as
    /// `NodeWalkways::crossings` and `RoadbedSystem::for_each_crossing` give them): it runs from
    /// the kerb on the positive lateral side of the edge's forward frame (driving on the right,
    /// where the sidewalk with the lower lane index ends) to the other; the connector from that
    /// kerb owns it. The one function pedestrian lanes and the renderer's zebras and crossing
    /// signs orient a crossing with.
    pub(crate) fn of_arm(key: ArmKey, [right, left]: [Vector3; 2]) -> Self {
        let (start, end) = if starts_left(key) {
            (left, right)
        } else {
            (right, left)
        };
        Self {
            edge_id: key.edge,
            start,
            end,
        }
    }
}

/// Whether arm `key`'s crossing marking starts at its left kerb, seen walking away from the node:
/// the left is the positive lateral side of the edge's forward frame at the edge's start.
pub(crate) fn starts_left(key: ArmKey) -> bool {
    key.end == EdgeEnd::Start
}

/// Street crossing walked by a pedestrian junction connector that is a single crosswalk.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LaneCrosswalk {
    /// Road edge crossed, in either travel direction.
    pub edge_id: usize,
    /// Visible zebra; only one of the two directions of a crossing owns it.
    pub marking: Option<CrosswalkMarking>,
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
    /// Frontage congestion penalty in seconds that economy prices read: freight, customer and
    /// construction quotes. [`TransitNetwork::publish_frontage_prices`] copies it from
    /// `live_frontage_delay_s` once per operational hour, so quotes stay fixed within the hour.
    ///
    /// [`TransitNetwork::publish_frontage_prices`]: crate::simulation::network::TransitNetwork::publish_frontage_prices
    pub frontage_delay_penalty_s: f32,
    /// Smoothed frontage congestion penalty in seconds, updated once per simulated second from
    /// live lane speeds. Agent trip planning reads it directly.
    pub live_frontage_delay_s: f32,
    /// Cumulative distance at each geometry vertex: `cum_dist[i]` = distance from `geometry[0]` to `geometry[i]`.
    /// Used for O(log N) position interpolation via binary search.
    pub cum_dist: Vec<f32>,
    /// The travel type of this lane.
    pub lane_type: LaneType,
    /// Reachable lanes from the end of this lane.
    pub next_lanes: LaneSuccessors,
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
            live_frontage_delay_s: 0.0,
            cum_dist: Vec::new(),
            lane_type: LaneType::Vehicle,
            next_lanes: LaneSuccessors::default(),
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
    // Crossings by connector lane id. Few lanes cross a street, so they are kept out of `Lane`.
    crosswalks: HashMap<usize, LaneCrosswalk>,
    revision: u64,
    // Render-cull boxes derived from `lanes`, keyed by `revision` (`SIM-02-K`).
    planar_bounds: bounds::LanePlanarBounds,
    // Lanes below this id already sit on the visible road surface. Rebuilds only append lanes
    // (old ids become tombstones), so every lane at or above it still has flat build heights.
    surface_synced_len: usize,
    // `(first, generation)`: every lane from `first` up to `surface_synced_len` was lifted onto
    // roadbed generation `generation`, and only lifts and tombstoning write lane geometry, so
    // lifting such a lane again while the roadbed is at that generation changes nothing.
    lifted_since: Option<(usize, u64)>,
}

impl Default for LaneSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl LaneSystem {
    /// Creates a new, empty lane system.
    pub fn new() -> Self {
        Self {
            lanes: Vec::new(),
            edge_lanes: HashMap::new(),
            node_lanes: HashMap::new(),
            crosswalks: HashMap::new(),
            revision: 0,
            planar_bounds: Default::default(),
            surface_synced_len: 0,
            lifted_since: None,
        }
    }

    /// Clears all lanes and structural mappings.
    pub fn clear(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.lanes.clear();
        self.edge_lanes.clear();
        self.node_lanes.clear();
        self.crosswalks.clear();
        self.surface_synced_len = 0;
        self.lifted_since = None;
    }

    /// Changes whenever physical lane IDs or connector topology are rebuilt, even when
    /// terrain authoring leaves the CCH hierarchy unchanged. Building edits do not change it.
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// Street crossing walked by connector `lane_id`, when the connector is a single crosswalk.
    pub fn crosswalk(&self, lane_id: usize) -> Option<LaneCrosswalk> {
        self.crosswalks.get(&lane_id).copied()
    }

    /// Visible zebra owned by connector `lane_id`, if any.
    pub fn crosswalk_marking(&self, lane_id: usize) -> Option<CrosswalkMarking> {
        self.crosswalk(lane_id)?.marking
    }

    pub(crate) fn sync_heights_to_visible_surface(
        &mut self,
        graph: &RegionGraph,
        roadbed: &RoadbedSystem,
    ) {
        for (lane_id, lane) in self.lanes.iter_mut().enumerate() {
            let crossing = self.crosswalks.get(&lane_id).copied();
            sync_lane_height_to_visible_surface(lane, crossing, graph, roadbed);
        }
        self.surface_synced_len = self.lanes.len();
        self.lifted_since = Some((0, roadbed.generation()));
    }

    /// Live lanes (an edge's or a node's) whose points a full surface sync would still move:
    /// zero when every lane follows the published roadbed (`ROAD-68-G`). Retired lanes keep
    /// their last geometry and are not counted. O(lane points).
    #[cfg(test)]
    pub(crate) fn lanes_off_surface(&self, graph: &RegionGraph, roadbed: &RoadbedSystem) -> usize {
        self.edge_lanes
            .values()
            .chain(self.node_lanes.values())
            .flatten()
            .filter(|&&lane_id| {
                let lane = &self.lanes[lane_id];
                let mut synced = lane.clone();
                let crossing = self.crosswalks.get(&lane_id).copied();
                sync_lane_height_to_visible_surface(&mut synced, crossing, graph, roadbed);
                synced.geometry != lane.geometry
            })
            .count()
    }

    /// True when a rebuild appended lanes that still carry flat build heights.
    pub(crate) fn has_unsynced_lanes(&self) -> bool {
        self.surface_synced_len < self.lanes.len()
    }

    /// Lifts every lane appended since the last sync onto the visible road surface.
    ///
    /// Lane length feeds agent motion and route costs, so this runs at fixed points of the
    /// simulation (frame start and road commits) rather than only when a chunk is drawn. Each
    /// lane reads only the roadbed and writes only itself, so the shared pool lifts them.
    /// O(new lane points / P).
    pub(crate) fn sync_new_lanes_to_visible_surface(
        &mut self,
        graph: &RegionGraph,
        roadbed: &RoadbedSystem,
    ) {
        use rayon::prelude::*;
        let start = self.surface_synced_len.min(self.lanes.len());
        let new_lanes = &mut self.lanes[start..];
        let crosswalks = &self.crosswalks;
        // The frame and the commit call this inside the shared pool; other callers are not.
        crate::thread_pools::install(|| {
            new_lanes
                .par_iter_mut()
                .enumerate()
                .for_each(|(offset, lane)| {
                    let crossing = crosswalks.get(&(start + offset)).copied();
                    sync_lane_height_to_visible_surface(lane, crossing, graph, roadbed);
                });
        });
        self.surface_synced_len = self.lanes.len();
        let generation = roadbed.generation();
        self.lifted_since = Some(match self.lifted_since {
            Some((first, lifted)) if lifted == generation => (first, generation),
            _ => (start, generation),
        });
    }

    // Whether `lane_id` was lifted onto the roadbed generation it is at now
    // (`sync_new_lanes_to_visible_surface`), so lifting it again would change nothing.
    fn lifted_onto(&self, lane_id: usize, roadbed: &RoadbedSystem) -> bool {
        self.lifted_since.is_some_and(|(first, generation)| {
            generation == roadbed.generation() && (first..self.surface_synced_len).contains(&lane_id)
        })
    }

    pub(crate) fn sync_heights_to_visible_surface_for_owners(
        &mut self,
        graph: &RegionGraph,
        roadbed: &RoadbedSystem,
        edge_indices: &[usize],
        node_ids: &[u32],
    ) {
        let road_debug = crate::debug::Channel::Road.enabled();
        // A rebuilt piece's new lanes were lifted onto it already (`ROAD-68-G` part 4).
        let lifted = |lane_id: usize| {
            let skip = self.lifted_onto(lane_id, roadbed);
            #[cfg(debug_assertions)]
            if skip {
                let mut lifted = self.lanes[lane_id].clone();
                let crossing = self.crosswalks.get(&lane_id).copied();
                sync_lane_height_to_visible_surface(&mut lifted, crossing, graph, roadbed);
                assert_eq!(lifted.geometry, self.lanes[lane_id].geometry, "lane {lane_id}");
            }
            skip
        };
        let mut edge_lane_ids = BTreeSet::new();
        for edge_idx in edge_indices {
            if let Some(ids) = self.edge_lanes.get(edge_idx) {
                edge_lane_ids.extend(ids.iter().copied().filter(|&id| !lifted(id)));
            }
        }
        let mut node_lane_ids = BTreeSet::new();
        for node_id in node_ids {
            if let Some(ids) = self.node_lanes.get(&(*node_id as usize)) {
                node_lane_ids.extend(ids.iter().copied().filter(|&id| !lifted(id)));
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
            let crossing = self.crosswalks.get(&lane_id).copied();
            sync_lane_height_to_visible_surface(lane, crossing, graph, roadbed);
        }
        let edge_ms = edge_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);

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
            let crossing = self.crosswalks.get(&lane_id).copied();
            sync_lane_height_to_visible_surface(lane, crossing, graph, roadbed);
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

// Lifts one lane onto the visible road surface; `crossing` is the street crossing it walks, when
// it is a crossing connector.
fn sync_lane_height_to_visible_surface(
    lane: &mut Lane,
    crossing: Option<LaneCrosswalk>,
    graph: &RegionGraph,
    roadbed: &RoadbedSystem,
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
    let owner_query = roadbed.lane_query(edge_id, node_id, lane_type == LaneType::Vehicle);

    let mut changed = false;
    for (index, point) in lane.geometry.iter_mut().enumerate() {
        // A crossing connector's second and third points are its kerb ends, the marking's kerb
        // segment. They stand on the crossing, on the carriageway (D8): their height is the
        // published crossing kerb's at the same point, never a coverage lookup, which would put
        // a point on the curb line on the curb's top or the carriageway by rounding.
        let surface_y = match crossing {
            Some(crossing) if index == 1 || index == 2 => {
                published_kerb_height(graph, roadbed, node_id, crossing.edge_id, point)
            }
            _ => lane_visible_surface_height(lane_type, roadbed, &owner_query, point),
        };
        let Some(surface_y) = surface_y else {
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

// The height of the kerb of edge `edge_id`'s crossing at node `node_id` that stands at `point` in
// XZ, bit for bit, in the published pieces (`RoadbedSystem::for_each_crossing`: a node piece's
// layout kerbs on its plane, or a pass-through crossing's span-row carriageway-edge vertices).
// The lanes build their kerb ends with the same functions, so a crossing the pieces publish has
// the lane's kerb ends; `None` (the end keeps its height) where the pieces hold no such kerb.
// `O(degree)`, allocation-free.
fn published_kerb_height(
    graph: &RegionGraph,
    roadbed: &RoadbedSystem,
    node_id: usize,
    edge_id: usize,
    point: &Vector3,
) -> Option<f32> {
    let node = u32::try_from(node_id).ok()?;
    let at = |kerb: &Vector3| {
        kerb.x.to_bits() == point.x.to_bits() && kerb.z.to_bits() == point.z.to_bits()
    };
    let mut height = None;
    roadbed.for_each_crossing(graph, node, |arm, kerbs| {
        if arm.edge == edge_id
            && let Some(kerb) = kerbs.iter().find(|kerb| at(kerb))
        {
            height = Some(kerb.y);
        }
    });
    height
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
    roadbed: &RoadbedSystem,
    owner_query: &LaneSurfaceQuery<'_>,
    point: &Vector3,
) -> Option<f32> {
    let carriageway_only = lane_type == LaneType::Vehicle;
    owner_query.height_at(point.x, point.z).or_else(|| {
        if carriageway_only {
            roadbed
                .carriageway_height_at(point.x, point.z)
                .or_else(|| roadbed.height_at(point.x, point.z))
        } else {
            roadbed.height_at(point.x, point.z)
        }
    })
}

/// Unit tests for the lane system.
#[cfg(test)]
pub mod tests;

/// Junction connector trajectory invariants and the measurement the save sweep shares.
#[cfg(test)]
pub(crate) mod trajectory_tests;
