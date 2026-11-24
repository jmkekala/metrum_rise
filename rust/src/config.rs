// SPDX-License-Identifier: GPL-3.0-or-later

//! Global simulation constants shared across all modules.
//!
//! Import with `use crate::config;` or individual constants as needed.
//! All physical distances are in **metres**. All speeds are in **m/s**.

// Map and Global Simulation Limits

/// Maximum distance (metres) within which two road endpoint positions are snapped to the same node.
pub const SNAP_TOLERANCE: f32 = 2.0;
/// Maximum distance (metres) within which a new road endpoint is considered to intersect an existing edge.
pub const INTERSECTION_TOLERANCE: f32 = 2.0;

// Road Geometry and Rendering Constants

/// Render-only Z-bias (metres) for road decals such as lane markings and crosswalk stripes.
pub const ROAD_DECAL_RENDER_Z_BIAS_M: f32 = 0.04;
/// Width of a single traffic lane in metres. A standard 2-lane road is `2 × LANE_WIDTH` asphalt.
pub const LANE_WIDTH: f32 = 3.5;
/// Width of a single sidewalk in metres, applied on each side of the road.
pub const SIDEWALK_WIDTH: f32 = 1.5;
/// How far beyond the junction clip road sidewalks stop at a node without a junction layout
/// (pass-through pairs, footpath nodes, structure ends), leaving room for the walk round the
/// node. Sidewalks at layout nodes end where the layout says (`roadbed::NodeWalkways`).
pub const SIDEWALK_NODE_INSET: f32 = 4.0;
/// Fine Z-bias (metres) applied to overlay meshes (e.g., zoning highlights) to prevent Z-fighting with the road mesh.
pub const Z_FIGHT_BIAS: f32 = 0.001;

// Zoning Simulation Parameters

/// Reference depth used by tooling and heuristics (e.g. camera hit distance, depth-alpha fade).
/// Not a hard cap on zoning — each edge side grows dynamically from painted cells.
pub const DEFAULT_ZONING_DEPTH: usize = 12;
/// Whether traffic drives on the left side of the road (`true` = UK/Japan style, `false` = continental/US style).
pub const DRIVE_ON_LEFT: bool = false;
/// Whether cars are left-hand drive: the driver sits on the left and steps out of the left front
/// door (`PARK-35`). The game setting for which door a moving-in household's driver uses; the
/// other members take the remaining doors from it (`simulation::parking::doors`).
pub const LEFT_HAND_DRIVE: bool = true;

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

/// Distance (metres) from the camera at which walkers start to fade out (`RENDER-28`, chosen by
/// the user 2026-11-12). Within it a walker is drawn whole.
pub const WALKER_FADE_START_M: f32 = 800.0;

/// Walkers' draw distance (metres) from the camera: they are dithered out over the band from
/// [`WALKER_FADE_START_M`] and draw nothing beyond. The render snapshot stops posing them
/// [`AGENT_CULL_PADDING_M`] further out, for the camera's movement in the snapshot's latency.
pub const WALKER_DRAW_DISTANCE_M: f32 = 1000.0;

/// Distance (metres) from the camera at which cars, moving and parked, start to fade out
/// (`RENDER-28`, `RENDER-48`).
pub const CAR_FADE_START_M: f32 = 2000.0;

/// Cars' draw distance (metres) from the camera, as [`WALKER_DRAW_DISTANCE_M`] for walkers.
/// Moving cars are cut by the render snapshot; parked cars fade with the same band and each
/// parked bucket's batch ends its visibility range where none of its cars is within this
/// distance (`RENDER-48`, `stream/layers/parked.rs`).
pub const CAR_DRAW_DISTANCE_M: f32 = 2500.0;

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
/// Body length of the `car` class in metres (`TRAFFIC-38`): what a household car takes in a
/// lane, and the car lot proof's body (`simulation/parking/validation.rs`).
pub const CAR_BODY_LENGTH_M: f32 = 4.5;
/// Body length of the `truck` class in metres (`TRAFFIC-38`): what a freight carrier takes in a
/// lane, and the freight proof's body its docks, gates and turn squares are proven for.
pub const TRUCK_BODY_LENGTH_M: f32 = 8.0;
/// Length a walker takes in a foot lane's bucket, in metres (`TRAFFIC-38`): a standing person's
/// body depth front to back. Walkers queue single file at this plus [`IDM_S_MIN`].
pub const WALKER_BODY_LENGTH_M: f32 = 0.5;

/// Multiplier for converting kilometres per hour into the simulation's internal m/s units.
pub const KMH_TO_MPS: f32 = 1.0 / 3.6;

/// Default design speed for the currently supported urban road presets: 50 km/h.
pub const DEFAULT_URBAN_ROAD_SPEED_MS: f32 = 50.0 * KMH_TO_MPS;

/// Speed threshold where a road is treated as high-speed frontage-hostile infrastructure: 80 km/h.
/// Advisory only (`ROAD-101`): the road builder suggests zoning off at it; an edge's own
/// `no_building_spawn` decides, never its speed.
pub const HIGH_SPEED_ROAD_THRESHOLD_MS: f32 = 80.0 * KMH_TO_MPS;

/// Speed threshold where road traffic emits higher environmental noise: 60 km/h.
pub const HIGH_NOISE_ROAD_THRESHOLD_MS: f32 = 60.0 * KMH_TO_MPS;

/// Maximum car speed while traversing a junction connector lane, about 22 km/h.
pub const CAR_JUNCTION_SPEED_MS: f32 = 6.0;

/// Minimum crawl speed once a car is admitted into a junction connector lane.
pub const CAR_JUNCTION_MIN_SPEED_MS: f32 = 2.0;

/// Comfortable lateral acceleration target for cars turning through junction connectors.
/// Used with the connector's approximate turn radius to slow tight turns more than gentle bends.
pub const CAR_JUNCTION_LATERAL_ACCEL_MS2: f32 = 2.2;

/// Small denominator floor for route-cost calculations on malformed or very slow edges.
pub const MIN_ROUTE_SPEED_MS: f32 = 1.0;

/// Mean walking speed (m/s): a gameplay speed, not a pedestrian's pace to the eye (`TRAFFIC-41`,
/// the owner's decision). Speeds are metres per real second at 1x and one game minute is one
/// real second, so 4 m/s is 240 m a game hour, where a 5 km/h walk would be 84 m. Each walker
/// walks it times its own pace (`walker_gait::walker_speed`) on foot lanes, on crossings and off
/// the lanes alike, and every walking plan, quote and limit is priced at it: the routing metric,
/// access and frontage times, the parking walk, the mode choice.
pub const AGENT_WALK_SPEED_MS: f32 = 4.0;

/// Speed of a vehicle off the lanes (m/s): a car on a door's access path, a bay's manoeuvre, a
/// drop-off's kerb stop and way out, a carrier driving to a door, and the speed any vehicle joins
/// its lane with. 5 m/s so that a car is never slower than a walker beside it (the fastest walks
/// `AGENT_WALK_SPEED_MS` times 1.18, 4.72 m/s) and stays under a junction connector's
/// `CAR_JUNCTION_SPEED_MS`. A gameplay speed like the walkers' (`TRAFFIC-41`, `TRAFFIC-43`).
/// Every plan and quote of an off-lane drive is priced at it.
pub const AGENT_DRIVEWAY_SPEED_MS: f32 = 5.0;

/// Speed of a freight carrier on its dock's route in a yard (m/s), driving in and backing or
/// driving out (`PARK-20`, `PARK-21`): a truck reverses the whole route out of a dock, which at
/// [`AGENT_DRIVEWAY_SPEED_MS`] would read as 18 km/h in reverse. No plan prices a dock's route.
pub const FREIGHT_YARD_SPEED_MS: f32 = 3.0;

/// Minimum distance (metres) between a building's frontage node and a junction.
/// If a building is closer, its frontage will snap to the nearest junction node.
/// This prevents road segments that are too short for stable junction clipping.
pub const MIN_FRONTAGE_DISTANCE: f32 = 8.0;
