// SPDX-License-Identifier: GPL-2.0-only

//! Building-site apron grading and road/terrain tie-in validation.

use super::geometry::{SITE_POINT_EPS_M, point_in_polygon_slice, signed_polygon_area};
#[cfg(test)]
use super::model::BuildingSiteClient;
use super::model::BuildingSiteTerrainSnapshot;
use crate::config::SIDEWALK_WIDTH;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::roadbed::RoadbedSystem;
use crate::simulation::network::types::{TransitFlags, TransitType};
use crate::simulation::terrain::cdt::{
    MAX_TERRAIN_TIE_IN_SLOPE_RATIO, TerrainCdtTieInGuideSample, TerrainCdtVertex,
};
use crate::simulation::terrain::{
    TerrainSystem, TerrainVisualSource, terrain_cdt_local_sample_margin_m,
};
use godot::prelude::{Vector2, Vector3};
use std::collections::HashSet;

const BUILDING_SITE_GRADING_SAMPLE_KEY_SCALE: f64 = 1000.0;
const BUILDING_SITE_GRADING_RING_MULTIPLIERS: [f32; 5] = [0.5, 1.0, 2.0, 4.0, 8.0];
const BUILDING_SITE_SUPPORT_TIE_IN_SAMPLE_STEP_M: f32 = 2.0;
const BUILDING_SITE_SUPPORT_TIE_IN_EPS_M: f32 = 0.05;
pub(super) const BUILDING_SITE_NEAREST_ROAD_SURFACE_MIN_RADIUS_M: f32 = 3.0;
pub(super) const BUILDING_SITE_NEAREST_ROAD_SURFACE_MAX_RADIUS_M: f32 = 8.0;
const BUILDING_SITE_ROAD_SURFACE_PROBE_INSET_M: f32 = 0.05;
// How far a pad's tie-in rays reach (`docs/terrain.md` T12 (a)): the graded terrain's cap, the
// farthest a pad's cone grades the ground.
const BUILDING_SITE_SUPPORT_TIE_IN_REACH_M: f32 = crate::simulation::terrain::graded::CAP_M as f32;

/// Roads seen by site grading: the graph for nearest-edge probes, the roadbed for heights.
#[derive(Clone, Copy)]
pub(crate) struct SiteRoads<'a> {
    graph: &'a RegionGraph,
    roadbed: &'a RoadbedSystem,
}

impl<'a> SiteRoads<'a> {
    /// Binds the live graph and the roadbed built from it.
    pub(crate) fn new(graph: &'a RegionGraph, roadbed: &'a RoadbedSystem) -> Self {
        Self { graph, roadbed }
    }

    fn sample_visible_height(self, x: f32, z: f32) -> Option<f32> {
        self.roadbed.height_at(x, z)
    }

    // Visits R-tree candidates in ascending edge order for deterministic nearest-edge ties.
    // Cost is O(log(resident edges) + local candidates log(local candidates)).
    fn visit_edges_near_point(
        self,
        pos: Vector3,
        radius_m: f32,
        mut visit: impl FnMut(&RegionGraph, usize),
    ) {
        let mut edges = self.graph.get_edges_near_point(pos, radius_m);
        edges.sort_unstable();
        edges.dedup();
        for edge in edges {
            visit(self.graph, edge);
        }
    }
}

/// Immutable inputs for exporting building-site grading into one terrain-CDT window.
pub(crate) struct BuildingSiteGradingRequest<'a> {
    terrain: &'a dyn TerrainVisualSource,
    roads: SiteRoads<'a>,
    world_bounds: (f32, f32, f32, f32),
    render_step_m: f32,
}

impl<'a> BuildingSiteGradingRequest<'a> {
    /// Binds one terrain window to the road-surface state used for grading samples.
    pub(crate) fn new(
        terrain: &'a dyn TerrainVisualSource,
        roads: SiteRoads<'a>,
        world_bounds: (f32, f32, f32, f32),
        render_step_m: f32,
    ) -> Self {
        Self {
            terrain,
            roads,
            world_bounds,
            render_step_m,
        }
    }
}

pub(super) struct SiteGradingContext<'a> {
    terrain: &'a dyn TerrainVisualSource,
    roads: SiteRoads<'a>,
    safe_step_m: f32,
    max_distance_m: f32,
}

impl<'a> SiteGradingContext<'a> {
    pub(super) fn new(
        terrain: &'a dyn TerrainVisualSource,
        graph: &'a RegionGraph,
        road_surface: &'a RoadbedSystem,
        safe_step_m: f32,
        max_distance_m: f32,
    ) -> Self {
        Self {
            terrain,
            roads: SiteRoads::new(graph, road_surface),
            safe_step_m,
            max_distance_m,
        }
    }
}

pub(super) struct SiteGradingGuideSink<'a> {
    samples: &'a mut Vec<TerrainCdtTieInGuideSample>,
    sample_keys: &'a mut HashSet<(i64, i64)>,
}

impl<'a> SiteGradingGuideSink<'a> {
    pub(super) fn new(
        samples: &'a mut Vec<TerrainCdtTieInGuideSample>,
        sample_keys: &'a mut HashSet<(i64, i64)>,
    ) -> Self {
        Self {
            samples,
            sample_keys,
        }
    }
}

#[derive(Clone, Copy)]
struct FootprintEdge {
    start: Vector2,
    end: Vector2,
    outward: Vector2,
    sample_count: u32,
}

impl BuildingSiteTerrainSnapshot {
    pub(crate) fn append_terrain_cdt_site_grading_guides_for_world_bounds(
        &self,
        request: BuildingSiteGradingRequest<'_>,
        tie_in_guide_samples: &mut Vec<TerrainCdtTieInGuideSample>,
        sample_keys: &mut HashSet<(i64, i64)>,
    ) {
        let (min_x, min_z, max_x, max_z) = request.world_bounds;
        let safe_step_m = request.render_step_m.max(f32::EPSILON);
        let max_distance_m =
            terrain_cdt_local_sample_margin_m(request.terrain.terrain(), safe_step_m);
        let context = SiteGradingContext {
            terrain: request.terrain,
            roads: request.roads,
            safe_step_m,
            max_distance_m,
        };
        let mut sink = SiteGradingGuideSink::new(tie_in_guide_samples, sample_keys);
        for site in &self.sites {
            let (site_min_x, site_min_z, site_max_x, site_max_z) =
                super::geometry::polygon_slice_bounds(&site.footprint_world);
            if site_min_x - max_distance_m > max_x
                || site_max_x + max_distance_m < min_x
                || site_min_z - max_distance_m > max_z
                || site_max_z + max_distance_m < min_z
            {
                continue;
            }
            append_building_site_grading_guides_from_parts(
                &site.footprint_world,
                &context,
                &mut sink,
            );
        }
    }
}

pub(crate) fn building_site_support_tie_in_is_valid(
    footprint_world: &[Vector2],
    support_height_m: f32,
    terrain: &TerrainSystem,
    graph: &RegionGraph,
    road_surface: &RoadbedSystem,
) -> bool {
    if !support_height_m.is_finite() {
        return false;
    }
    let safe_step_m = BUILDING_SITE_SUPPORT_TIE_IN_SAMPLE_STEP_M.max(f32::EPSILON);
    let context = SiteGradingContext::new(
        terrain,
        graph,
        road_surface,
        safe_step_m,
        BUILDING_SITE_SUPPORT_TIE_IN_REACH_M,
    );
    visit_footprint_grading_rays(footprint_world, safe_step_m, |seam, outward| {
        building_site_support_tie_in_ray_is_valid(support_height_m, seam, outward, &context)
    })
}

/// Chooses the closest feasible level pad to the preferred road-connection height.
/// Intersects exact unions of the existing ray-height intervals, plus driveway grade intervals;
/// it does not raise the slope limit or move the road. Ties choose the lower height. The rays
/// read the base terrain and the roads out to the graded terrain's cap (`docs/terrain.md` T12
/// (a)).
pub(crate) fn solve_building_site_support_height(
    footprint: &[Vector2],
    preferred: f32,
    connections: &[(Vector2, f32)],
    terrain: &TerrainSystem,
    graph: &RegionGraph,
    surface: &RoadbedSystem,
) -> Option<f32> {
    if footprint.len() < 3 || !preferred.is_finite() || connections.is_empty() {
        return None;
    }
    let mut low = f32::NEG_INFINITY;
    let mut high = f32::INFINITY;
    for &(point, height) in connections {
        let distance = if point_in_polygon_slice(point, footprint) {
            0.0
        } else {
            (0..footprint.len())
                .map(|i| {
                    let a = footprint[i];
                    let delta = footprint[(i + 1) % footprint.len()] - a;
                    let t = ((point - a).dot(delta) / delta.length_squared()).clamp(0.0, 1.0);
                    point.distance_to(a + delta * t)
                })
                .fold(f32::INFINITY, f32::min)
        };
        let budget = distance * MAX_TERRAIN_TIE_IN_SLOPE_RATIO;
        low = low.max(height - budget);
        high = high.min(height + budget);
    }
    if low > high {
        return None;
    }
    let context = SiteGradingContext::new(
        terrain,
        graph,
        surface,
        BUILDING_SITE_SUPPORT_TIE_IN_SAMPLE_STEP_M,
        BUILDING_SITE_SUPPORT_TIE_IN_REACH_M,
    );
    let mut intervals = vec![(low, high)];
    let mut next = Vec::new();
    let valid = visit_footprint_grading_rays(footprint, context.safe_step_m, |seam, outward| {
        next.clear();
        for d in grading_ring_distances(context.safe_step_m, context.max_distance_m) {
            let target =
                building_site_raw_tie_in_target_height(seam + outward * d, d, terrain, context.roads);
            let budget = d * MAX_TERRAIN_TIE_IN_SLOPE_RATIO;
            for &(lo, hi) in &intervals {
                let a = lo.max(target - budget);
                let b = hi.min(target + budget);
                if a <= b {
                    next.push((a, b));
                }
            }
        }
        next.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        intervals.clear();
        for &(lo, hi) in &next {
            if let Some(last) = intervals.last_mut().filter(|last| lo <= last.1) {
                last.1 = last.1.max(hi);
            } else {
                intervals.push((lo, hi));
            }
        }
        !intervals.is_empty()
    });
    if !valid {
        return None;
    }
    intervals
        .into_iter()
        .map(|(lo, hi)| preferred.clamp(lo, hi))
        .min_by(|a, b| {
            (a - preferred)
                .abs()
                .total_cmp(&(b - preferred).abs())
                .then(a.total_cmp(b))
        })
}

/// Padding needed to capture every terrain and nearest-road dependency of site feasibility:
/// the tie-in reach (the graded terrain's cap), the nearest-road probe radius and one terrain
/// cell of bilinear support.
pub(crate) fn site_feasibility_dependency_margin_m(terrain: &TerrainSystem) -> f32 {
    BUILDING_SITE_SUPPORT_TIE_IN_REACH_M
        + BUILDING_SITE_NEAREST_ROAD_SURFACE_MAX_RADIUS_M
        + terrain.cell_size_m()
}

fn visit_footprint_grading_rays(
    footprint_world: &[Vector2],
    safe_step_m: f32,
    mut visit: impl FnMut(Vector2, Vector2) -> bool,
) -> bool {
    let signed_area = signed_polygon_area(footprint_world);
    if signed_area.abs() <= f32::EPSILON {
        return false;
    }
    let loop_is_ccw = signed_area > 0.0;
    let mut edges = Vec::with_capacity(footprint_world.len());

    for edge_idx in 0..footprint_world.len() {
        let start = footprint_world[edge_idx];
        let end = footprint_world[(edge_idx + 1) % footprint_world.len()];
        let delta = end - start;
        let length_m = delta.length();
        if length_m <= f32::EPSILON {
            return false;
        }
        let outward = if loop_is_ccw {
            Vector2::new(delta.y, -delta.x)
        } else {
            Vector2::new(-delta.y, delta.x)
        } / length_m;
        edges.push(FootprintEdge {
            start,
            end,
            outward: corrected_footprint_outward(footprint_world, (start + end) * 0.5, outward),
            sample_count: ((length_m / safe_step_m).ceil() as u32).max(1),
        });
    }

    for edge in &edges {
        for sample_idx in 0..=edge.sample_count {
            let t = sample_idx as f32 / edge.sample_count as f32;
            if !visit(edge.start.lerp(edge.end, t), edge.outward) {
                return false;
            }
        }
    }

    for vertex_idx in 0..footprint_world.len() {
        let previous = edges[(vertex_idx + edges.len() - 1) % edges.len()].outward;
        let next = edges[vertex_idx].outward;
        let bisector = previous + next;
        if bisector.length_squared() <= f32::EPSILON {
            continue;
        }
        let vertex = footprint_world[vertex_idx];
        let outward = corrected_footprint_outward(footprint_world, vertex, bisector.normalized());
        if !visit(vertex, outward) {
            return false;
        }
    }

    true
}

#[cfg(test)]
pub(super) fn append_building_site_grading_guides(
    site: &BuildingSiteClient,
    context: &SiteGradingContext<'_>,
    sink: &mut SiteGradingGuideSink<'_>,
) {
    append_building_site_grading_guides_from_parts(&site.footprint_world, context, sink);
}

fn append_building_site_grading_guides_from_parts(
    footprint_world: &[Vector2],
    context: &SiteGradingContext<'_>,
    sink: &mut SiteGradingGuideSink<'_>,
) {
    visit_footprint_grading_rays(footprint_world, context.safe_step_m, |seam, outward| {
        append_building_site_grading_ray(seam, outward, context, sink);
        true
    });
}

fn append_building_site_grading_ray(
    seam: Vector2,
    outward: Vector2,
    context: &SiteGradingContext<'_>,
    sink: &mut SiteGradingGuideSink<'_>,
) {
    for distance_m in grading_ring_distances(context.safe_step_m, context.max_distance_m) {
        let pos = seam + outward * distance_m;
        // Road and site guides sample the same ground. Only the footprint boundary
        // owns the pad elevation; CDT derives its tie-in from that constraint.
        let height_m =
            context.terrain.sample_visual_height_world(pos.x, pos.y) * crate::config::HEIGHT_SCALE;
        push_building_site_grading_sample(
            TerrainCdtVertex::new(pos.x as f64, height_m, pos.y as f64),
            &mut *sink.samples,
            &mut *sink.sample_keys,
        );
    }
}

fn building_site_support_tie_in_ray_is_valid(
    seam_height_m: f32,
    seam: Vector2,
    outward: Vector2,
    context: &SiteGradingContext<'_>,
) -> bool {
    for distance_m in grading_ring_distances(context.safe_step_m, context.max_distance_m) {
        let pos = seam + outward * distance_m;
        let target_height_m = building_site_raw_tie_in_target_height(
            pos,
            distance_m,
            context.terrain.terrain(),
            context.roads,
        );
        let max_delta_m = distance_m.max(0.0) * MAX_TERRAIN_TIE_IN_SLOPE_RATIO
            + BUILDING_SITE_SUPPORT_TIE_IN_EPS_M;
        if (target_height_m - seam_height_m).abs() <= max_delta_m {
            return true;
        }
    }
    false
}

fn grading_ring_distances(safe_step_m: f32, max_distance_m: f32) -> impl Iterator<Item = f32> {
    let mut previous_distance_m = 0.0_f32;
    BUILDING_SITE_GRADING_RING_MULTIPLIERS
        .into_iter()
        .map(move |multiplier| (safe_step_m * multiplier).min(max_distance_m))
        // The envelope need not be a power-of-two multiple of the render step (Kuopio: 20 m).
        // Validation and guide generation must both reach its actual outer boundary.
        .chain(std::iter::once(max_distance_m))
        .filter(move |&distance_m| {
            if distance_m <= previous_distance_m + f32::EPSILON {
                return false;
            }
            previous_distance_m = distance_m;
            true
        })
}

/// Samples the road or terrain target used to validate a proposed pad's support height: the
/// visible road, else the base terrain (`docs/terrain.md` T12 (a)), which the graded terrain
/// grades from.
pub(super) fn building_site_raw_tie_in_target_height(
    pos: Vector2,
    distance_m: f32,
    terrain: &TerrainSystem,
    roads: SiteRoads<'_>,
) -> f32 {
    let nearest_radius_m = distance_m.clamp(
        BUILDING_SITE_NEAREST_ROAD_SURFACE_MIN_RADIUS_M,
        BUILDING_SITE_NEAREST_ROAD_SURFACE_MAX_RADIUS_M,
    );
    if let Some(road_height_m) = building_site_visible_road_height(roads, pos, nearest_radius_m) {
        return road_height_m;
    }
    terrain.sample_height_world(pos.x, pos.y) * crate::config::HEIGHT_SCALE
}

fn building_site_visible_road_height(
    roads: SiteRoads<'_>,
    pos: Vector2,
    nearest_radius_m: f32,
) -> Option<f32> {
    if let Some(height_m) = roads.sample_visible_height(pos.x, pos.y) {
        return Some(height_m);
    }
    nearest_building_site_road_surface_sample(roads, pos, nearest_radius_m)
        .map(|(_, height_m)| height_m)
}

pub(super) fn nearest_building_site_road_surface_sample(
    roads: SiteRoads<'_>,
    pos: Vector2,
    nearest_radius_m: f32,
) -> Option<(Vector2, f32)> {
    let radius_m = nearest_radius_m.max(0.0);
    if radius_m <= f32::EPSILON {
        return None;
    }
    let mut best: Option<(f32, usize, Vector2, f32)> = None;
    roads.visit_edges_near_point(
        Vector3::new(pos.x, 0.0, pos.y),
        radius_m,
        |graph, edge_idx| {
            let Some(edge) = graph.get_edge(edge_idx) else {
                return;
            };
            if edge.deleted || edge.physical_geometry.len() < 2 || edge.physical_length <= 1e-6 {
                return;
            }
            let Some(projection) =
                BuildingAllocator::project_point_to_edge_centerline(edge_idx, edge, pos)
            else {
                return;
            };
            let center = BuildingAllocator::sample_pos_on_edge(graph, edge_idx, projection.t);
            let tangent = BuildingAllocator::sample_tangent_on_edge(graph, edge_idx, projection.t);
            if tangent.length_squared() <= 1e-12 {
                return;
            }
            let normal = Vector2::new(tangent.y, -tangent.x) * projection.side as f32;
            let probe = center + normal * building_site_road_connection_lateral_offset_m(edge);
            let dist_sq = probe.distance_squared_to(pos);
            if dist_sq > radius_m * radius_m {
                return;
            }
            let Some(height_m) = roads.sample_visible_height(probe.x, probe.y) else {
                return;
            };
            let replace = best
                .as_ref()
                .is_none_or(|(best_dist_sq, best_edge_idx, _, _)| {
                    dist_sq
                        .total_cmp(best_dist_sq)
                        .then(edge_idx.cmp(best_edge_idx))
                        .is_lt()
                });
            if replace {
                best = Some((dist_sq, edge_idx, probe, height_m));
            }
        },
    );
    best.map(|(_, _, probe, height_m)| (probe, height_m))
}

pub(super) fn building_site_road_connection_lateral_offset_m(
    edge: &crate::simulation::network::graph::Edge,
) -> f32 {
    let sidewalk_m = if edge.primary_type == TransitType::Foot
        || (edge.allowed_types & TransitFlags::FOOT) == 0
    {
        0.0
    } else {
        SIDEWALK_WIDTH
    };
    (edge.width * 0.5 + sidewalk_m - BUILDING_SITE_ROAD_SURFACE_PROBE_INSET_M).max(0.0)
}

fn corrected_footprint_outward(
    footprint_world: &[Vector2],
    seam: Vector2,
    outward: Vector2,
) -> Vector2 {
    if point_in_polygon_slice(seam + outward * SITE_POINT_EPS_M * 8.0, footprint_world) {
        -outward
    } else {
        outward
    }
}

fn push_building_site_grading_sample(
    vertex: TerrainCdtVertex,
    tie_in_guide_samples: &mut Vec<TerrainCdtTieInGuideSample>,
    sample_keys: &mut HashSet<(i64, i64)>,
) {
    if !vertex.x.is_finite() || !vertex.z.is_finite() || !vertex.height_m.is_finite() {
        return;
    }
    let key = building_site_grading_sample_key(vertex.x, vertex.z);
    if !sample_keys.insert(key) {
        return;
    }
    tie_in_guide_samples.push(TerrainCdtTieInGuideSample { vertex });
}

fn building_site_grading_sample_key(x: f64, z: f64) -> (i64, i64) {
    (
        (x * BUILDING_SITE_GRADING_SAMPLE_KEY_SCALE).round() as i64,
        (z * BUILDING_SITE_GRADING_SAMPLE_KEY_SCALE).round() as i64,
    )
}
