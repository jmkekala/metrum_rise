// SPDX-License-Identifier: GPL-3.0-or-later

//! Road-tool picking (`ROAD-112` round B): the pointer ray to a surface point on the level it hit
//! (`ROAD-99` `StructureHit`), and the level height and map-edge helpers of the session's cursor.
//! Its snapping is `snap.rs`.

use crate::nodes::sim::core::RoadToolQuerySnapshot;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::interaction::StructureHit;
use crate::simulation::network::roadbed::PieceId;
use godot::prelude::Vector3;

/// The network snap's reach in metres (`interaction` acquisition radius).
pub const NETWORK_SNAP_REACH_M: f32 = 5.0;
/// The self-snap radius: a cursor this near the stroke's start or control point takes it.
pub const SELF_SNAP_M: f32 = 2.5;

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
    /// edge, at the height there (the off-surface fallback).
    Plane(Vector3),
}

/// A copyable wrapper of [`StructureHit`] so the pick can be public.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Structure(pub(crate) StructureHit);

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

// The lane-width proxy for the zoning snap: lanes times the lane width, at least 2 m.
pub(crate) fn lane_width_m(fwd_lanes: i32, bkw_lanes: i32) -> f32 {
    ((fwd_lanes.clamp(0, 4) + bkw_lanes.clamp(0, 4)) as f32 * crate::config::LANE_WIDTH).max(2.0)
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
