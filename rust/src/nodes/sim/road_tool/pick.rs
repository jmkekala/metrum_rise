// SPDX-License-Identifier: GPL-2.0-only

//! Road-tool picking (`ROAD-112` round B): the pointer ray to a surface point on the level it hit
//! (`ROAD-99` `StructureHit`), and today's whole cursor query as plain Rust.
//!
//! [`road_tool_cursor`] is the bridge's `get_road_tool_cursor_pos` (`network_api/road_tool/
//! cursor.rs`) copied operation for operation, returning a [`CursorResult`] in place of the Godot
//! dictionary; its helpers are `SimulationNode`'s `road_tool_*` statics. Round C turns the bridge
//! method into a thin wrapper over it. The session uses [`pick_surface`] and the level height
//! helpers only; its snapping is `snap.rs`.

use crate::nodes::sim::core::RoadToolQuerySnapshot;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::interaction::{
    NetworkSnap, NetworkSnapTarget, StructureHit, get_closest_network_snap_at_hit,
    retained_network_snap_xz, snap_stroke_angle,
};
use crate::simulation::network::roadbed::PieceId;
use godot::prelude::Vector3;

// A stroke within this many degrees of a snap angle is pulled onto it (today's magnet).
const ANGLE_SNAP_TOLERANCE_DEG: f32 = 4.0;
/// Today's network snap reach in metres (`interaction` acquisition radius).
pub const NETWORK_SNAP_REACH_M: f32 = 5.0;
/// Today's self-snap radius: a cursor this near the stroke's start or control point takes it.
pub const SELF_SNAP_M: f32 = 2.5;

/// What the bridge's cursor query takes besides the ray (its `get_road_tool_cursor_pos`
/// arguments, the previous result already decoded).
#[derive(Clone, Copy, Debug)]
pub struct CursorQuery {
    /// Ray origin in world space.
    pub ray_origin: Vector3,
    /// Ray direction (not necessarily unit).
    pub ray_dir: Vector3,
    /// The tool's keyboard height offset, added to a ground hit.
    pub altitude_offset_m: f32,
    /// Whether a stroke is being drawn.
    pub active: bool,
    /// Today's tool state: 0 idle, 1 placing the control point, 2 placing the end.
    pub current_state: i32,
    /// The stroke's start.
    pub start_pos: Vector3,
    /// The curve's control point (the start while none is placed).
    pub control_pos: Vector3,
    /// The "Snap to zoning grid" checkbox.
    pub zoning_snap_enabled: bool,
    /// Forward lanes of the drawn type.
    pub fwd_lanes: i32,
    /// Backward lanes of the drawn type.
    pub bkw_lanes: i32,
    /// Map-edge snap distance in metres (25 m today).
    pub border_snap_dist_m: f32,
    /// The previous result's network target and the generation it was taken at.
    pub(crate) previous_snap: Option<(NetworkSnapTarget, u64)>,
    /// Release radius of a retained network target (8 m today).
    pub sticky_release_dist_m: f32,
    /// Reference direction of the 45 degree magnet (unit XZ), zero for none.
    pub angle_snap: Vector3,
}

/// The cursor query's answer: the bridge dictionary's fields.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CursorResult {
    /// The resolved cursor position.
    pub position: Vector3,
    /// The network node or edge the cursor snapped to.
    pub(crate) target: Option<NetworkSnapTarget>,
    /// The road-tool surface generation the target belongs to.
    pub surface_generation: u64,
    /// Whether the 45 degree magnet moved the cursor.
    pub angle_snapped: bool,
}

/// Where a pointer ray landed, before any snapping.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SurfacePick {
    /// The ray hit a road piece or the graded ground. `position` carries the altitude offset on
    /// the ground, the road's own height on a road.
    Surface {
        /// Hit point (offset height on the ground).
        position: Vector3,
        /// Whether the nearer hit was a road piece.
        hit_road: bool,
        /// The structure the ray hit (`ROAD-99`), the only one it may snap to.
        structure: Structure,
    },
    /// No surface: the ray met the `y = 0` plane, clamped into the map and onto its nearest
    /// edge, at the height there (today's off-surface fallback).
    Plane(Vector3),
}

/// A copyable wrapper of [`StructureHit`] so the pick can be public.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Structure(pub(crate) StructureHit);

/// `get_road_tool_cursor_pos` without the dictionary: pick, zoning snap, sticky network snap, the
/// 45 degree magnet, map-edge snap and self snap, in today's order. `None` where the bridge
/// returns `null` (the ray misses every surface and points up). Allocates where today's does
/// (the network snap's edge candidate list).
pub(crate) fn road_tool_cursor(
    query: &RoadToolQuerySnapshot,
    q: &CursorQuery,
) -> Option<CursorResult> {
    let (half_w, half_h) = half_extents(query);
    let border_snap_dist = q.border_snap_dist_m.min(half_w.min(half_h));
    let result = |position, target| CursorResult {
        position,
        target,
        surface_generation: query.surface_generation,
        angle_snapped: false,
    };

    let (mut pos, hit_road, structure_hit) =
        match pick_surface(query, q.ray_origin, q.ray_dir, q.altitude_offset_m)? {
            SurfacePick::Surface {
                position,
                hit_road,
                structure,
            } => (position, hit_road, structure.0),
            SurfacePick::Plane(hit) => return Some(result(hit, None)),
        };
    let hit_m = pos.y;
    let level_height_at =
        |x: f32, z: f32| level_height(query, x, z, hit_road, hit_m, q.altitude_offset_m);

    if q.zoning_snap_enabled {
        let raw_snap = cursor_network_snap(
            &query.region_graph,
            pos,
            structure_hit,
            q.previous_snap,
            query.surface_generation,
            q.sticky_release_dist_m,
        );
        // Existing junctions remain exact connection targets.
        if let Some(snap) =
            raw_snap.filter(|snap| matches!(snap.target, NetworkSnapTarget::Node(_)))
        {
            pos = snap.position;
        } else {
            let xz = |point: Vector3| glam::DVec2::new(f64::from(point.x), f64::from(point.z));
            let snapped = query.zoning_grid.snap_road_cursor(
                &query.region_graph,
                xz(pos),
                (q.active && q.current_state != 0).then_some(xz(q.start_pos)),
                f64::from(lane_width_m(q.fwd_lanes, q.bkw_lanes)),
            );
            pos.x = snapped.x as f32;
            pos.z = snapped.y as f32;
        }
        pos.y = level_height_at(pos.x, pos.z);
    }

    if let Some(snap) = cursor_network_snap(
        &query.region_graph,
        pos,
        structure_hit,
        q.previous_snap,
        query.surface_generation,
        q.sticky_release_dist_m,
    ) {
        // A snapped node or edge reports its graph height (`ROAD-99`).
        return Some(result(snap.position, Some(snap.target)));
    }

    if q.active
        && q.current_state == 2
        && !q.zoning_snap_enabled
        && let Some(mut snapped) = snap_stroke_angle(
            q.start_pos,
            pos,
            q.angle_snap,
            ANGLE_SNAP_TOLERANCE_DEG.to_radians(),
        )
    {
        snapped.y = level_height_at(snapped.x, snapped.z);
        return Some(CursorResult {
            angle_snapped: true,
            ..result(snapped, None)
        });
    }

    if is_near_border(pos, half_w, half_h, border_snap_dist) {
        pos = snap_to_border(pos, half_w, half_h);
        pos.y = level_height_at(pos.x, pos.z);
        return Some(result(pos, None));
    }

    if q.active {
        if pos.distance_to(q.start_pos) < SELF_SNAP_M {
            return Some(result(q.start_pos, None));
        }
        if q.current_state == 2 && pos.distance_to(q.control_pos) < SELF_SNAP_M {
            return Some(result(q.control_pos, None));
        }
    }

    Some(result(pos, None))
}

/// The pointer ray's surface point on the level it hit: the nearer of the roadbed piece and the
/// graded ground (the ground with `altitude_offset_m`), else the `y = 0` plane clamped into the
/// map, snapped to its nearest edge and lifted to the height there. `None` for a ray that misses
/// every surface and does not point down. Cost: one roadbed raycast and one graded raycast.
pub(crate) fn pick_surface(
    query: &RoadToolQuerySnapshot,
    ray_origin: Vector3,
    ray_dir: Vector3,
    altitude_offset_m: f32,
) -> Option<SurfacePick> {
    let road_hit = query.roadbed.raycast_piece(ray_origin, ray_dir);
    let terrain_hit = query.with_graded_surface(|surface, scratch| {
        surface
            .raycast_surface(
                [ray_origin.x, ray_origin.y, ray_origin.z].map(f64::from),
                [ray_dir.x, ray_dir.y, ray_dir.z].map(f64::from),
                scratch,
            )
            .map(|[x, y, z]| Vector3::new(x as f32, y as f32, z as f32))
    });
    // The level the ray hit (`ROAD-99`): a road's surface, or the ground. Under a bridge the
    // cursor stays on the road or the ground it hit, not on the deck over it.
    let structure = structure_hit(&query.region_graph, ray_origin, road_hit, terrain_hit);
    let road_hit = road_hit.map(|(hit, _)| hit);
    match closest_cursor_hit(ray_origin, road_hit, terrain_hit) {
        Some((position, true)) => Some(SurfacePick::Surface {
            position,
            hit_road: true,
            structure: Structure(structure),
        }),
        Some((mut position, false)) => {
            position.y += altitude_offset_m;
            Some(SurfacePick::Surface {
                position,
                hit_road: false,
                structure: Structure(structure),
            })
        }
        None => {
            if ray_dir.y >= -0.001 {
                return None;
            }
            let (half_w, half_h) = half_extents(query);
            let t_plane = -ray_origin.y / ray_dir.y;
            let mut hit = ray_origin + ray_dir * t_plane;
            hit.x = hit.x.clamp(-half_w, half_w);
            hit.z = hit.z.clamp(-half_h, half_h);
            hit = snap_to_border(hit, half_w, half_h);
            hit.y = height_at_xz(query, hit.x, hit.z, altitude_offset_m);
            Some(SurfacePick::Plane(hit))
        }
    }
}

/// Half the world's width and depth in metres.
pub(crate) fn half_extents(query: &RoadToolQuerySnapshot) -> (f32, f32) {
    let (world_w, world_h) = query.terrain.world_size();
    (world_w * 0.5, world_h * 0.5)
}

// Today's lane-width proxy for the zoning snap: lanes times the lane width, at least 2 m.
pub(crate) fn lane_width_m(fwd_lanes: i32, bkw_lanes: i32) -> f32 {
    ((fwd_lanes.clamp(0, 4) + bkw_lanes.clamp(0, 4)) as f32 * crate::config::LANE_WIDTH).max(2.0)
}

/// The network snap of a cursor at `cursor_pos` whose ray hit `structure_hit`: the previous
/// target while the cursor stays within its release radius and the ray still admits it, else a
/// fresh acquisition among the targets the ray admits (`ROAD-99` follow-up B1). Copy of
/// `SimulationNode::road_tool_cursor_network_snap`; allocates in the acquisition.
pub(crate) fn cursor_network_snap(
    graph: &RegionGraph,
    cursor_pos: Vector3,
    structure_hit: StructureHit,
    previous: Option<(NetworkSnapTarget, u64)>,
    surface_generation: u64,
    release_dist_m: f32,
) -> Option<NetworkSnap> {
    // IDs are snapshot-local: never reuse a target after topology/generation replacement.
    let target = previous
        .filter(|&(_, generation)| generation == surface_generation)
        .map(|(target, _)| target);
    if let Some(snap) = target
        .filter(|&target| structure_hit.admits(graph, target))
        .and_then(|target| {
            retained_network_snap_xz(
                graph,
                cursor_pos,
                target,
                NETWORK_SNAP_REACH_M,
                release_dist_m,
            )
        })
        .filter(|snap| structure_hit.admits(graph, snap.target))
    {
        return Some(snap);
    }
    get_closest_network_snap_at_hit(graph, cursor_pos, NETWORK_SNAP_REACH_M, structure_hit)
}

// The nearer of the road and terrain hits, and whether it is the road's.
pub(crate) fn closest_cursor_hit(
    ray_origin: Vector3,
    road_hit: Option<Vector3>,
    terrain_hit: Option<Vector3>,
) -> Option<(Vector3, bool)> {
    match (road_hit, terrain_hit) {
        (Some(road), Some(terrain)) => {
            let road_dist_sq = (road - ray_origin).length_squared();
            let terrain_dist_sq = (terrain - ray_origin).length_squared();
            if road_dist_sq <= terrain_dist_sq {
                Some((road, true))
            } else {
                Some((terrain, false))
            }
        }
        (Some(road), None) => Some((road, true)),
        (None, Some(terrain)) => Some((terrain, false)),
        (None, None) => None,
    }
}

// The structure the cursor's ray hit, the only one it may snap to (`ROAD-99` follow-up B1):
// the bridge deck or deck node piece the road hit belongs to when that hit is the nearer
// one; the ground when the ray hit the ground first, or a grounded piece, or a buried bore.
pub(crate) fn structure_hit(
    graph: &RegionGraph,
    ray_origin: Vector3,
    road_hit: Option<(Vector3, PieceId)>,
    terrain_hit: Option<Vector3>,
) -> StructureHit {
    let piece = road_hit
        .filter(|&(hit, _)| {
            closest_cursor_hit(ray_origin, Some(hit), terrain_hit).is_some_and(|(_, road)| road)
        })
        .map(|(_, piece)| piece);
    StructureHit::of_piece(graph, piece)
}

/// The cursor's height at an XZ point near its hit, on the level the ray hit (`ROAD-99`): on a
/// road, the exposed surface there nearest the hit's height `hit_m`; on the ground, a grounded
/// road's surface there, else the ground with the tool's altitude offset.
pub(crate) fn level_height(
    query: &RoadToolQuerySnapshot,
    x: f32,
    z: f32,
    hit_road: bool,
    hit_m: f32,
    altitude_offset_m: f32,
) -> f32 {
    let road_m = if hit_road {
        query.roadbed.exposed_height_nearest_at(x, z, hit_m)
    } else {
        query.roadbed.grounded_height_at(x, z)
    };
    road_m.unwrap_or_else(|| ground_height(query, x, z) + altitude_offset_m)
}

// The exposed road surface at a point, else the ground plus the offset.
fn height_at_xz(query: &RoadToolQuerySnapshot, x: f32, z: f32, altitude_offset_m: f32) -> f32 {
    query
        .roadbed
        .exposed_height_at(x, z)
        .unwrap_or_else(|| ground_height(query, x, z) + altitude_offset_m)
}

// The graded ground's height at a point.
fn ground_height(query: &RoadToolQuerySnapshot, x: f32, z: f32) -> f32 {
    query.with_graded_surface(|surface, scratch| {
        surface.ground_height_at(f64::from(x), f64::from(z), scratch) as f32
    })
}

/// Whether `pos` lies within `threshold` of the map's edge.
pub(crate) fn is_near_border(pos: Vector3, half_w: f32, half_h: f32, threshold: f32) -> bool {
    pos.x < -half_w + threshold
        || pos.x > half_w - threshold
        || pos.z < -half_h + threshold
        || pos.z > half_h - threshold
}

/// `pos` moved onto the map's nearest edge.
pub(crate) fn snap_to_border(mut pos: Vector3, half_w: f32, half_h: f32) -> Vector3 {
    let d_left = pos.x + half_w;
    let d_right = half_w - pos.x;
    let d_top = pos.z + half_h;
    let d_bottom = half_h - pos.z;
    let min_d = d_left.min(d_right).min(d_top).min(d_bottom);
    if min_d == d_left {
        pos.x = -half_w;
    } else if min_d == d_right {
        pos.x = half_w;
    } else if min_d == d_top {
        pos.z = -half_h;
    } else {
        pos.z = half_h;
    }
    pos
}
