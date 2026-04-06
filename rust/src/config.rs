//! Global simulation constants shared across all modules.
//!
//! Import with `use crate::config;` or individual constants as needed.
//! All physical distances are in **metres**. All speeds are in **m/s**.

// Rendering Limits

/// Target render frame rate cap. The Godot engine's `max_fps` is set to this value at startup.
/// 60 is sufficient — the sim thread ticks at ~60 Hz so rendering faster wastes GPU.
/// Set to 0 to uncap (not recommended; causes unnecessary GPU load).
pub const TARGET_FPS: u32 = 60;

// Map and Global Simulation Limits

/// Vertical exaggeration applied to the terrain heightmap for rendering. Raw height values are multiplied by this.
pub const HEIGHT_SCALE: f32 = 20.0;
/// Maximum distance (metres) within which two road endpoint positions are snapped to the same node.
pub const SNAP_TOLERANCE: f32 = 2.0;
/// Maximum distance (metres) within which a new road endpoint is considered to intersect an existing edge.
pub const INTERSECTION_TOLERANCE: f32 = 2.0;

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

/// Reference depth used by tooling and heuristics (e.g. camera hit distance, depth-alpha fade).
/// Not a hard cap on zoning — each edge side grows dynamically from painted cells.
pub const DEFAULT_ZONING_DEPTH: usize = 12;
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

/// Horizontal scaling factor (Sx, Sy, Sz) applied to building MultiMesh instances.
/// Standard assets use 1 unit = 10m, so we scale by 10.0 for a meter-based simulation.
pub const BUILDING_VISUAL_SCALE: f32 = 10.0;

/// Maximum far-plane distance (metres) used when computing the camera frustum AABB for agent
/// culling. Far-plane rays are clamped to this distance before projecting onto XZ, preventing
/// a near-horizontal camera from producing an AABB that covers the entire map and defeating culling.
/// Increase if agents visibly pop in at the horizon; decrease to tighten the cull radius.
pub const AGENT_CULL_FAR_M: f32 = 4000.0;

/// Padding (metres) added to each side of the camera frustum AABB before sending it to the
/// sim thread. Prevents agents from popping in at the exact viewport edge due to AABB latency.
pub const AGENT_CULL_PADDING_M: f32 = 200.0;

// IDM (Intelligent Driver Model) — Car Physics

/// Maximum acceleration in m/s². Governs how quickly cars reach free-flow speed.
/// Increase for more aggressive acceleration from standstill.
pub const IDM_A_MAX: f32 = 6.0;
/// Comfortable braking deceleration in m/s². Reserved for the full IDM approach-speed term.
pub const IDM_B: f32 = 6.0;
/// Desired time headway in seconds. Smaller values produce tighter following distances.
pub const IDM_T_HEAD: f32 = 0.5;
/// Minimum bumper-to-bumper gap at standstill in metres.
pub const IDM_S_MIN: f32 = 0.1;
/// Effective vehicle length used for IDM gap calculation in metres.
/// Measured from .glb meshes: sedan/sports 2.55 m, SUV 2.85 m. Using 2.6 m as a
/// representative average; adjust if packing feels too tight or loose.
pub const CAR_LENGTH: f32 = 2.6;

/// Minimum distance (metres) between a building's frontage node and a junction.
/// If a building is closer, its frontage will snap to the nearest junction node.
/// This prevents road segments that are too short for stable junction clipping.
pub const MIN_FRONTAGE_DISTANCE: f32 = 8.0;
