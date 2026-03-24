//! Global simulation constants shared across all modules.
//!
//! Import with `use crate::config;` or individual constants as needed.
//! All physical distances are in **metres**. All speeds are in **m/s**.

// Map and Global Simulation Limits

/// Width of the simulation map in zoning grid cells. Physical width = `MAP_WIDTH × GRID_CELL_SIZE` metres.
pub const MAP_WIDTH: usize = 1000;
/// Height of the simulation map in zoning grid cells. Physical height = `MAP_HEIGHT × GRID_CELL_SIZE` metres.
pub const MAP_HEIGHT: usize = 1000;
/// Size of the environmental grids (pollution, noise, desirability).
/// Reducing this from 2000/1000 to 500 provides a 16x/4x reduction in compute/memory.
pub const ENV_GRID_SIZE: usize = 500;
/// Vertical exaggeration applied to the terrain heightmap for rendering. Raw height values are multiplied by this.
pub const HEIGHT_SCALE: f32 = 20.0;
/// Maximum distance (metres) within which two road endpoint positions are snapped to the same node.
pub const SNAP_TOLERANCE: f32 = 1.0;
/// Maximum distance (metres) within which a new road endpoint is considered to intersect an existing edge.
pub const INTERSECTION_TOLERANCE: f32 = 1.0;

// Road Geometry and Rendering Constants

/// Elevation (metres) of the road mesh above the terrain surface, used to prevent Z-fighting.
pub const ROAD_H_OFFSET: f32 = 0.01;
/// Width of a single traffic lane in metres. A standard 2-lane road is `2 × LANE_WIDTH` asphalt.
pub const LANE_WIDTH: f32 = 3.5;
/// Width of a single sidewalk in metres, applied on each side of the road.
pub const SIDEWALK_WIDTH: f32 = 1.5;
/// Fine Z-bias (metres) applied to overlay meshes (e.g., zoning highlights) to prevent Z-fighting with the road mesh.
pub const Z_FIGHT_BIAS: f32 = 0.001;

// Zoning Simulation Parameters

/// How many zoning cells deep a zone extends away from the road sidewalk edge.
/// Total zoning depth = `ZONING_DEPTH × GRID_CELL_SIZE` metres (default: 10 × 10 m = 100 m).
pub const ZONING_DEPTH: usize = 10;
/// Physical size of one square zoning grid cell in metres. Buildings occupy a 3 × 3 cell (30 m × 30 m) footprint.
pub const GRID_CELL_SIZE: f32 = 10.0;
