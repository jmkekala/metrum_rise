//! Global simulation constants shared across all modules.
//!
//! Import with `use crate::config;` or individual constants as needed.
//! All physical distances are in **metres**. All speeds are in **m/s**.

// Map and Global Simulation Limits

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

pub const ZONING_DEPTH: usize = 12;
/// Whether traffic drives on the left side of the road (`true` = UK/Japan style, `false` = continental/US style).
pub const DRIVE_ON_LEFT: bool = false;
/// Minimum vertical separation (metres) for a bridge/tunnel to NOT obstruct other systems.
pub const CLEARANCE_THRESHOLD: f32 = 5.0;

/// Maximum distance (metres) from the map edge within which a road endpoint is offered as
/// a candidate for an external border connection.
///
/// This should be small — the road tool snaps endpoints to exactly the map boundary, so a
/// node created there will be within [`SNAP_TOLERANCE`] of the edge. 3 m gives comfortable
/// headroom without triggering for roads that merely pass near the border.
pub const BORDER_DETECTION_THRESHOLD: f32 = 3.0;

/// The physical distance (metres) that a road is automatically extended off-screen
/// when marked as an external border connection. This ensures immigrants spawn cleanly off-map.
pub const BORDER_EXTENSION_M: f32 = 10.0;
