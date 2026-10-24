// SPDX-License-Identifier: GPL-2.0-only

//! The grading envelope shared by roads and building sites (`ROAD-68-D`, `ROAD-68` open
//! question 3): how far from a footprint boundary its grade-limited tie-in reaches, and the
//! road tie-in guides placed inside that reach.
//!
//! The rule is [`earthworks.md`](../../../../docs/earthworks.md) "Geometry Decision": rays leave
//! every sourced boundary edge at the render step (2 m or finer) and every boundary vertex on
//! its outward bisector; a ray needs the distance at which the 1:2 tie-in from the seam height
//! meets the terrain. Guides are grade-limited samples on rings along those rays; rails join
//! consecutive rings only for a single clean convex outer loop. Sites take the same margin and
//! keep their own sample-only apron guides.

use super::cdt::{
    MAX_TERRAIN_TIE_IN_SLOPE_RATIO, TerrainCdtRoadBoundarySource, TerrainCdtRoadLoop,
    TerrainCdtRoadSupport, TerrainCdtTieInGuideConstraint, TerrainCdtTieInGuideSample,
    TerrainCdtVertex,
};
use super::{TerrainVisualSource, terrain_cdt_local_sample_margin_m};
use crate::simulation::core::round_f64_to_i64;
use std::collections::HashSet;

/// Upper bound of a grading envelope: the terrain CDT tile halo, beyond which no tile sees the
/// footprint whose tie-in it would carry (`ROAD-68` open question 18).
pub(crate) const GRADING_ENVELOPE_MAX_MARGIN_M: f32 = super::TERRAIN_CDT_TILE_HALO_M;
const GRADING_RING_MULTIPLIERS: [f32; 4] = [1.0, 2.0, 4.0, 8.0];
const GRADING_MAX_PROBES: usize = 8;
const GRADING_SAMPLE_KEY_SCALE: f64 = 1000.0;

// One boundary ray: seam point and height, unit outward direction.
#[derive(Clone, Copy)]
struct Ray {
    x: f64,
    z: f64,
    height_m: f32,
    outward: (f64, f64),
}

/// Grading envelope of one footprint loop: the larger of the base local sample margin and the
/// largest reach of the rays from its boundary (the distance each needs to tie in at the 1:2
/// limit), capped at [`GRADING_ENVELOPE_MAX_MARGIN_M`]. Loops without a clean grounded tie-in
/// (sites, structures) use their vertex heights against the terrain below them.
pub(crate) fn grading_envelope_margin_m(
    terrain: &impl TerrainVisualSource,
    road_loop: &TerrainCdtRoadLoop,
    render_step_m: f32,
) -> f32 {
    road_loop_grading(terrain, road_loop, render_step_m, None)
}

/// The guides one CDT tile takes from a road loop: `tile` is the tile's XZ rectangle
/// `[min_x, min_z, max_x, max_z]`, already widened by the tile's guide tolerance.
pub(crate) struct TileGuides<'a> {
    /// Rectangle a ray's reach must meet for its ring to be emitted.
    pub(crate) tile: [f64; 4],
    /// Guide samples, appended in ray order.
    pub(crate) samples: &'a mut Vec<TerrainCdtTieInGuideSample>,
    /// Rails between consecutive rings of one edge.
    pub(crate) constraints: &'a mut Vec<TerrainCdtTieInGuideConstraint>,
    /// Whether rails may join the rings: the loop is its tile's only road loop. They do only for
    /// a clean convex outer loop.
    pub(crate) rails: bool,
}

/// One pass over a loop's rays: returns [`grading_envelope_margin_m`] and, with `guides`, appends
/// the grade-limited guide rings of every ray whose reach meets the tile (every ray when rails
/// join them). Each ray's reach is probed once and serves both the margin and its ring, so a
/// guide depends only on the boundary near it. `O(R)` terrain probes for `R` rays.
pub(crate) fn road_loop_grading(
    terrain: &impl TerrainVisualSource,
    road_loop: &TerrainCdtRoadLoop,
    render_step_m: f32,
    mut guides: Option<TileGuides<'_>>,
) -> f32 {
    let step_m = render_step_m.max(f32::EPSILON);
    let base_m = terrain_cdt_local_sample_margin_m(terrain.terrain(), step_m);
    let cap_m = GRADING_ENVELOPE_MAX_MARGIN_M.max(base_m);
    let mut margin_m = base_m;
    if !clean_grounded(road_loop) {
        for vertex in &road_loop.vertices {
            let ground_m = terrain.sample_visual_height_world(vertex.x as f32, vertex.z as f32)
                * crate::config::HEIGHT_SCALE;
            let required_m = (vertex.height_m - ground_m).abs() / MAX_TERRAIN_TIE_IN_SLOPE_RATIO;
            margin_m = margin_m.max(required_m + step_m * 2.0);
        }
        return margin_m.min(cap_m);
    }
    let rails = guides.as_ref().is_some_and(|guides| guides.rails)
        && !road_loop.is_hole
        && convex(road_loop)
        && fully_sourced(road_loop);
    // The previous edge ray's ring, as a range of `samples`, for its rails.
    let mut previous: Option<std::ops::Range<usize>> = None;
    let mut visit = |ray: Ray, edge_ray: Option<bool>| {
        let reach_m = ray_margin_m(terrain, ray, step_m, base_m).min(cap_m);
        margin_m = margin_m.max(reach_m);
        let Some(guides) = guides.as_mut() else {
            return;
        };
        let (end_x, end_z) = along(ray, reach_m);
        let [min_x, min_z, max_x, max_z] = guides.tile;
        let meets = ray.x.max(end_x) >= min_x
            && ray.x.min(end_x) <= max_x
            && ray.z.max(end_z) >= min_z
            && ray.z.min(end_z) <= max_z;
        if !meets && !rails {
            previous = None;
            return;
        }
        let start = guides.samples.len();
        ring_vertices(terrain, ray, step_m, reach_m, guides.samples);
        let ring = start..guides.samples.len();
        if let (Some(false), Some(before)) = (edge_ray, previous.clone()) {
            for (a, b) in before.zip(ring.clone()) {
                let (start, end) = (guides.samples[a].vertex, guides.samples[b].vertex);
                if sample_key(start) != sample_key(end) {
                    guides
                        .constraints
                        .push(TerrainCdtTieInGuideConstraint { start, end });
                }
            }
        }
        previous = edge_ray.map(|_| ring).filter(|_| rails);
    };
    visit_edge_rays(road_loop, step_m, |ray, first_of_edge| {
        visit(ray, Some(first_of_edge));
    });
    visit_vertex_rays(road_loop, |ray| visit(ray, None));
    margin_m.min(cap_m)
}

/// Grade-limited tie-in guides of road loops over their whole reach, keyed once per millimetre
/// cell ([`road_loop_grading`] without a tile).
pub(crate) fn append_road_grading_guides(
    terrain: &impl TerrainVisualSource,
    road_loops: &[TerrainCdtRoadLoop],
    render_step_m: f32,
    samples: &mut Vec<TerrainCdtTieInGuideSample>,
    constraints: &mut Vec<TerrainCdtTieInGuideConstraint>,
    sample_keys: &mut HashSet<(i64, i64)>,
) {
    let everything = [
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::INFINITY,
    ];
    for road_loop in road_loops {
        let start = samples.len();
        let guides = TileGuides {
            tile: everything,
            samples,
            constraints,
            rails: road_loops.len() == 1,
        };
        road_loop_grading(terrain, road_loop, render_step_m, Some(guides));
        // Keep the first sample of each millimetre cell, in order.
        let mut kept = start;
        for index in start..samples.len() {
            if sample_keys.insert(sample_key(samples[index].vertex)) {
                samples.swap(kept, index);
                kept += 1;
            }
        }
        samples.truncate(kept);
    }
}

// Every source edge, as (start, end), in loop order; a bare test loop uses its vertex edges.
fn edges(
    road_loop: &TerrainCdtRoadLoop,
) -> impl Iterator<Item = (TerrainCdtVertex, TerrainCdtVertex)> + '_ {
    let bare = road_loop.source_edges.is_empty();
    let count = road_loop.vertices.len();
    let vertex_edges = (0..if bare { count } else { 0 }).map(move |index| {
        (
            road_loop.vertices[index],
            road_loop.vertices[(index + 1) % count],
        )
    });
    road_loop
        .source_edges
        .iter()
        .map(|edge| (edge.start, edge.end))
        .chain(vertex_edges)
}

// Whether the loop keeps the road on its left: road chains do; a loop wound against that
// convention (test input) does not. `O(V)`, once per loop.
fn road_on_left(road_loop: &TerrainCdtRoadLoop) -> bool {
    (signed_area(&road_loop.vertices) > 0.0) != road_loop.is_hole
}

// The outward (non-road) side of an edge: the right when the road is on the left.
fn outward(road_left: bool, start: TerrainCdtVertex, end: TerrainCdtVertex) -> Option<(f64, f64)> {
    let (dx, dz) = (end.x - start.x, end.z - start.z);
    let length = dx.hypot(dz);
    if length <= f64::EPSILON {
        return None;
    }
    let sign = if road_left { 1.0 } else { -1.0 };
    Some((sign * dz / length, -sign * dx / length))
}

// Rays at `step_m` or finer along each edge, both ends included; the flag marks an edge's first.
fn visit_edge_rays(road_loop: &TerrainCdtRoadLoop, step_m: f32, mut visit: impl FnMut(Ray, bool)) {
    let road_left = road_on_left(road_loop);
    for (start, end) in edges(road_loop) {
        let Some(outward) = outward(road_left, start, end) else {
            continue;
        };
        let length = (end.x - start.x).hypot(end.z - start.z);
        let count = ((length as f32 / step_m).ceil() as u32).max(1);
        for index in 0..=count {
            let t = f64::from(index) / f64::from(count);
            visit(
                Ray {
                    x: start.x + (end.x - start.x) * t,
                    z: start.z + (end.z - start.z) * t,
                    height_m: start.height_m + (end.height_m - start.height_m) * t as f32,
                    outward,
                },
                index == 0,
            );
        }
    }
}

// Rays on the outward bisector where one edge ends and the next starts.
fn visit_vertex_rays(road_loop: &TerrainCdtRoadLoop, mut visit: impl FnMut(Ray)) {
    let mut previous: Option<(TerrainCdtVertex, (f64, f64))> = None;
    let mut first: Option<(TerrainCdtVertex, (f64, f64))> = None;
    let mut joint = |end_of_previous: Option<(TerrainCdtVertex, (f64, f64))>,
                     start: TerrainCdtVertex,
                     normal: (f64, f64)| {
        let Some((end, previous_normal)) = end_of_previous else {
            return;
        };
        if end.x != start.x || end.z != start.z {
            return;
        }
        let bisector = (previous_normal.0 + normal.0, previous_normal.1 + normal.1);
        let length = bisector.0.hypot(bisector.1);
        if length > f64::EPSILON {
            visit(Ray {
                x: start.x,
                z: start.z,
                height_m: start.height_m,
                outward: (bisector.0 / length, bisector.1 / length),
            });
        }
    };
    let road_left = road_on_left(road_loop);
    for (start, end) in edges(road_loop) {
        let Some(normal) = outward(road_left, start, end) else {
            continue;
        };
        joint(previous, start, normal);
        first.get_or_insert((start, normal));
        previous = Some((end, normal));
    }
    if let Some((start, normal)) = first {
        joint(previous, start, normal);
    }
}

// Distance one ray needs: probes double out from one step until the 1:2 tie-in from the seam
// reaches the probed terrain height.
fn ray_margin_m(terrain: &impl TerrainVisualSource, ray: Ray, step_m: f32, base_m: f32) -> f32 {
    let mut margin_m = base_m;
    let mut probe_m = step_m;
    for _ in 0..GRADING_MAX_PROBES {
        let ground_m = ground_at(terrain, ray, probe_m);
        let required_m =
            (ray.height_m - ground_m).abs() / MAX_TERRAIN_TIE_IN_SLOPE_RATIO + step_m * 2.0;
        margin_m = margin_m.max(required_m);
        if probe_m + f32::EPSILON >= margin_m || probe_m >= GRADING_ENVELOPE_MAX_MARGIN_M {
            break;
        }
        probe_m = margin_m
            .max(probe_m * 2.0)
            .min(GRADING_ENVELOPE_MAX_MARGIN_M);
    }
    margin_m
}

// Ring vertices at one, two, four and eight steps, then the margin, each at the terrain height
// limited to the 1:2 grade from the seam.
fn ring_vertices(
    terrain: &impl TerrainVisualSource,
    ray: Ray,
    step_m: f32,
    margin_m: f32,
    out: &mut Vec<TerrainCdtTieInGuideSample>,
) {
    let mut previous_m = 0.0_f32;
    let distances = GRADING_RING_MULTIPLIERS
        .into_iter()
        .map(|multiplier| (step_m * multiplier).min(margin_m))
        .chain(std::iter::once(margin_m));
    for distance_m in distances {
        if distance_m <= previous_m + f32::EPSILON {
            continue;
        }
        previous_m = distance_m;
        let ground_m = ground_at(terrain, ray, distance_m);
        let limit_m = distance_m * MAX_TERRAIN_TIE_IN_SLOPE_RATIO;
        let delta_m = ground_m - ray.height_m;
        let height_m = if delta_m.abs() <= limit_m {
            ground_m
        } else {
            ray.height_m + delta_m.signum() * limit_m
        };
        let (x, z) = along(ray, distance_m);
        out.push(TerrainCdtTieInGuideSample {
            vertex: TerrainCdtVertex::new(x, height_m, z),
        });
    }
}

fn along(ray: Ray, distance_m: f32) -> (f64, f64) {
    let distance = f64::from(distance_m);
    (
        ray.x + ray.outward.0 * distance,
        ray.z + ray.outward.1 * distance,
    )
}

fn ground_at(terrain: &impl TerrainVisualSource, ray: Ray, distance_m: f32) -> f32 {
    let (x, z) = along(ray, distance_m);
    terrain.sample_visual_height_world(x as f32, z as f32) * crate::config::HEIGHT_SCALE
}

// Grounded road edges tie in without walls; sites and structures do not take road guides.
fn clean_grounded(road_loop: &TerrainCdtRoadLoop) -> bool {
    road_loop.source_edges.iter().all(|edge| match edge.source {
        TerrainCdtRoadBoundarySource::RoadBoundary { support, .. } => {
            support == TerrainCdtRoadSupport::Grounded
        }
        TerrainCdtRoadBoundarySource::BuildingSiteBoundary { .. } => false,
        #[cfg(test)]
        TerrainCdtRoadBoundarySource::SyntheticTestBoundary { .. } => true,
    })
}

// Every loop edge carries a source: the loop is a whole footprint boundary, not cut by a
// query rectangle.
fn fully_sourced(road_loop: &TerrainCdtRoadLoop) -> bool {
    road_loop.source_edges.is_empty() || road_loop.source_edges.len() == road_loop.vertices.len()
}

fn convex(road_loop: &TerrainCdtRoadLoop) -> bool {
    let vertices = &road_loop.vertices;
    let area = signed_area(vertices);
    if vertices.len() < 3 || area.abs() <= f64::EPSILON {
        return false;
    }
    (0..vertices.len()).all(|index| {
        let previous = vertices[(index + vertices.len() - 1) % vertices.len()];
        let current = vertices[index];
        let next = vertices[(index + 1) % vertices.len()];
        let cross = (current.x - previous.x) * (next.z - current.z)
            - (current.z - previous.z) * (next.x - current.x);
        cross.abs() <= f64::EPSILON || cross.signum() == area.signum()
    })
}

fn signed_area(vertices: &[TerrainCdtVertex]) -> f64 {
    let mut twice = 0.0;
    for (index, a) in vertices.iter().enumerate() {
        let b = vertices[(index + 1) % vertices.len()];
        twice += a.x * b.z - b.x * a.z;
    }
    twice * 0.5
}

fn sample_key(vertex: TerrainCdtVertex) -> (i64, i64) {
    (
        round_f64_to_i64(vertex.x * GRADING_SAMPLE_KEY_SCALE),
        round_f64_to_i64(vertex.z * GRADING_SAMPLE_KEY_SCALE),
    )
}
