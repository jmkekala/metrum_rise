//! Building-site apron grading and road/terrain tie-in validation.

use super::geometry::{SITE_POINT_EPS_M, point_in_polygon_slice, signed_polygon_area};
use super::model::BuildingSiteClient;
use crate::config::SIDEWALK_WIDTH;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::surface::RoadSurfaceSystem;
use crate::simulation::network::types::{TransitFlags, TransitType};
use crate::simulation::terrain::cdt::{
    MAX_TERRAIN_TIE_IN_SLOPE_RATIO, TerrainCdtTieInGuideSample, TerrainCdtVertex,
};
use crate::simulation::terrain::{TerrainSystem, terrain_cdt_local_sample_margin_m};
use godot::prelude::{Vector2, Vector3};
use std::collections::BTreeMap;

const BUILDING_SITE_GRADING_SAMPLE_KEY_SCALE: f64 = 1000.0;
const BUILDING_SITE_GRADING_RING_MULTIPLIERS: [f32; 5] = [0.5, 1.0, 2.0, 4.0, 8.0];
const BUILDING_SITE_SUPPORT_TIE_IN_SAMPLE_STEP_M: f32 = 2.0;
const BUILDING_SITE_SUPPORT_TIE_IN_EPS_M: f32 = 0.05;
pub(super) const BUILDING_SITE_NEAREST_ROAD_SURFACE_MIN_RADIUS_M: f32 = 3.0;
pub(super) const BUILDING_SITE_NEAREST_ROAD_SURFACE_MAX_RADIUS_M: f32 = 8.0;
const BUILDING_SITE_ROAD_SURFACE_PROBE_INSET_M: f32 = 0.05;

/// Immutable inputs for exporting building-site grading into one terrain-CDT window.
pub(crate) struct BuildingSiteGradingRequest<'a> {
    terrain: &'a TerrainSystem,
    graph: &'a RegionGraph,
    road_surface: &'a RoadSurfaceSystem,
    world_bounds: (f32, f32, f32, f32),
    render_step_m: f32,
}

impl<'a> BuildingSiteGradingRequest<'a> {
    /// Binds one terrain window to the road-surface state used for grading samples.
    pub(crate) fn new(
        terrain: &'a TerrainSystem,
        graph: &'a RegionGraph,
        road_surface: &'a RoadSurfaceSystem,
        world_bounds: (f32, f32, f32, f32),
        render_step_m: f32,
    ) -> Self {
        Self {
            terrain,
            graph,
            road_surface,
            world_bounds,
            render_step_m,
        }
    }
}

pub(super) struct SiteGradingContext<'a> {
    terrain: &'a TerrainSystem,
    graph: &'a RegionGraph,
    road_surface: &'a RoadSurfaceSystem,
    safe_step_m: f32,
    max_distance_m: f32,
}

impl<'a> SiteGradingContext<'a> {
    pub(super) fn new(
        terrain: &'a TerrainSystem,
        graph: &'a RegionGraph,
        road_surface: &'a RoadSurfaceSystem,
        safe_step_m: f32,
        max_distance_m: f32,
    ) -> Self {
        Self {
            terrain,
            graph,
            road_surface,
            safe_step_m,
            max_distance_m,
        }
    }
}

pub(super) struct SiteGradingGuideSink<'a> {
    samples: &'a mut Vec<TerrainCdtTieInGuideSample>,
    sample_keys: &'a mut BTreeMap<(i64, i64), ()>,
}

impl<'a> SiteGradingGuideSink<'a> {
    pub(super) fn new(
        samples: &'a mut Vec<TerrainCdtTieInGuideSample>,
        sample_keys: &'a mut BTreeMap<(i64, i64), ()>,
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

impl BuildingAllocator {
    pub(crate) fn append_terrain_cdt_site_grading_guides_for_world_bounds(
        &self,
        request: BuildingSiteGradingRequest<'_>,
        tie_in_guide_samples: &mut Vec<TerrainCdtTieInGuideSample>,
        sample_keys: &mut BTreeMap<(i64, i64), ()>,
    ) {
        let (min_x, min_z, max_x, max_z) = request.world_bounds;
        let safe_step_m = request.render_step_m.max(f32::EPSILON);
        let max_distance_m = terrain_cdt_local_sample_margin_m(request.terrain, safe_step_m);
        for building_idx in self.site_candidate_indices_for_bounds(min_x, min_z, max_x, max_z) {
            let Some(site) = self.building_sites.get(building_idx) else {
                continue;
            };
            if !site.overlaps_bounds(min_x, min_z, max_x, max_z) {
                continue;
            }
            let context = SiteGradingContext::new(
                request.terrain,
                request.graph,
                request.road_surface,
                safe_step_m,
                max_distance_m,
            );
            let mut sink = SiteGradingGuideSink::new(tie_in_guide_samples, sample_keys);
            append_building_site_grading_guides(site, &context, &mut sink);
        }
    }
}

pub(crate) fn building_site_support_tie_in_is_valid(
    footprint_world: &[Vector2],
    support_height_m: f32,
    terrain: &TerrainSystem,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
) -> bool {
    if !support_height_m.is_finite() {
        return false;
    }
    let safe_step_m = BUILDING_SITE_SUPPORT_TIE_IN_SAMPLE_STEP_M.max(f32::EPSILON);
    let max_distance_m = terrain_cdt_local_sample_margin_m(terrain, safe_step_m);
    let context =
        SiteGradingContext::new(terrain, graph, road_surface, safe_step_m, max_distance_m);
    visit_footprint_grading_rays(footprint_world, safe_step_m, |seam, outward| {
        building_site_support_tie_in_ray_is_valid(support_height_m, seam, outward, &context)
    })
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

pub(super) fn append_building_site_grading_guides(
    site: &BuildingSiteClient,
    context: &SiteGradingContext<'_>,
    sink: &mut SiteGradingGuideSink<'_>,
) {
    visit_footprint_grading_rays(
        &site.footprint_world,
        context.safe_step_m,
        |seam, outward| {
            append_building_site_grading_ray(site.support_height_m, seam, outward, context, sink);
            true
        },
    );
}

fn append_building_site_grading_ray(
    seam_height_m: f32,
    seam: Vector2,
    outward: Vector2,
    context: &SiteGradingContext<'_>,
    sink: &mut SiteGradingGuideSink<'_>,
) {
    for distance_m in grading_ring_distances(context.safe_step_m, context.max_distance_m) {
        let pos = seam + outward * distance_m;
        let height_m = building_site_grading_target_height(
            seam_height_m,
            pos,
            distance_m,
            context.terrain,
            context.graph,
            context.road_surface,
        );
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
            context.terrain,
            context.graph,
            context.road_surface,
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
        .filter_map(move |multiplier| {
            let distance_m = (safe_step_m * multiplier).min(max_distance_m);
            if distance_m <= previous_distance_m + f32::EPSILON {
                return None;
            }
            previous_distance_m = distance_m;
            Some(distance_m)
        })
}

pub(super) fn building_site_grading_target_height(
    seam_height_m: f32,
    pos: Vector2,
    distance_m: f32,
    terrain: &TerrainSystem,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
) -> f32 {
    let raw_height_m =
        building_site_raw_tie_in_target_height(pos, distance_m, terrain, graph, road_surface);
    grade_limited_site_tie_in_height(seam_height_m, raw_height_m, distance_m)
}

fn building_site_raw_tie_in_target_height(
    pos: Vector2,
    distance_m: f32,
    terrain: &TerrainSystem,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
) -> f32 {
    let nearest_radius_m = distance_m.clamp(
        BUILDING_SITE_NEAREST_ROAD_SURFACE_MIN_RADIUS_M,
        BUILDING_SITE_NEAREST_ROAD_SURFACE_MAX_RADIUS_M,
    );
    if let Some(road_height_m) =
        building_site_visible_road_height(terrain, graph, road_surface, pos, nearest_radius_m)
    {
        return road_height_m;
    }
    terrain.sample_visual_height_world(pos.x, pos.y) * crate::config::HEIGHT_SCALE
}

fn building_site_visible_road_height(
    terrain: &TerrainSystem,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    pos: Vector2,
    nearest_radius_m: f32,
) -> Option<f32> {
    if let Some(height_m) = road_surface.sample_visible_surface_height(graph, terrain, pos.x, pos.y)
    {
        return Some(height_m);
    }
    nearest_building_site_road_surface_sample(terrain, graph, road_surface, pos, nearest_radius_m)
        .map(|(_, height_m)| height_m)
}

pub(super) fn nearest_building_site_road_surface_sample(
    terrain: &TerrainSystem,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    pos: Vector2,
    nearest_radius_m: f32,
) -> Option<(Vector2, f32)> {
    let radius_m = nearest_radius_m.max(0.0);
    if radius_m <= f32::EPSILON {
        return None;
    }
    let mut candidates = graph.get_edges_near_point(Vector3::new(pos.x, 0.0, pos.y), radius_m);
    candidates.sort_unstable();
    candidates.dedup();

    let mut best: Option<(f32, usize, Vector2, f32)> = None;
    for edge_idx in candidates {
        let Some(edge) = graph.get_edge(edge_idx) else {
            continue;
        };
        if edge.deleted || edge.physical_geometry.len() < 2 || edge.physical_length <= 1e-6 {
            continue;
        }
        let Some(projection) =
            BuildingAllocator::project_point_to_edge_centerline(edge_idx, edge, pos)
        else {
            continue;
        };
        let center = BuildingAllocator::sample_pos_on_edge(graph, edge_idx, projection.t);
        let tangent = BuildingAllocator::sample_tangent_on_edge(graph, edge_idx, projection.t);
        if tangent.length_squared() <= 1e-12 {
            continue;
        }
        let normal = Vector2::new(tangent.y, -tangent.x) * projection.side as f32;
        let probe = center + normal * building_site_road_connection_lateral_offset_m(edge);
        let dist_sq = probe.distance_squared_to(pos);
        if dist_sq > radius_m * radius_m {
            continue;
        }
        let Some(height_m) =
            road_surface.sample_visible_surface_height(graph, terrain, probe.x, probe.y)
        else {
            continue;
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
    }
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

fn grade_limited_site_tie_in_height(
    seam_height_m: f32,
    terrain_height_m: f32,
    distance_m: f32,
) -> f32 {
    let max_delta_m = distance_m.max(0.0) * MAX_TERRAIN_TIE_IN_SLOPE_RATIO;
    let delta_m = terrain_height_m - seam_height_m;
    if delta_m.abs() <= max_delta_m {
        terrain_height_m
    } else {
        seam_height_m + delta_m.signum() * max_delta_m
    }
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
    sample_keys: &mut BTreeMap<(i64, i64), ()>,
) {
    if !vertex.x.is_finite() || !vertex.z.is_finite() || !vertex.height_m.is_finite() {
        return;
    }
    let key = building_site_grading_sample_key(vertex.x, vertex.z);
    if sample_keys.insert(key, ()).is_some() {
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
