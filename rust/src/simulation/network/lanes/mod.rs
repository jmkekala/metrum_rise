use godot::prelude::*;
use std::collections::HashMap;

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
    /// Cumulative distance at each geometry vertex: `cum_dist[i]` = distance from `geometry[0]` to `geometry[i]`.
    /// Used for O(log N) position interpolation via binary search.
    pub cum_dist: Vec<f32>,
    /// The travel type of this lane.
    pub lane_type: LaneType,
    /// Whether this is a visual crosswalk.
    pub is_crosswalk: bool,
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
            cum_dist: Vec::new(),
            lane_type: LaneType::Vehicle,
            is_crosswalk: false,
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
}

/// Unit tests for the lane system.
#[cfg(test)]
pub mod tests;
