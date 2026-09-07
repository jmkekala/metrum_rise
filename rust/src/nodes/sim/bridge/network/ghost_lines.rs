// SPDX-License-Identifier: GPL-2.0-only

//! Per-edge ghost geometry reuse against authoritative mesh and terrain publication inputs.

use super::*;
use crate::simulation::network::render::NetworkMeshData;
use crate::simulation::network::surface::SurfaceChunkKey;
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

const GHOST_HEIGHT_SAMPLE_STEP_M: f32 = 8.0;

/// Retained CPU guide lines; complete uploads preserve the historical outward/offset ordering.
#[derive(Default)]
pub(crate) struct RoadGhostLineCache {
    edges: Vec<Option<CachedEdge>>,
    terrain_source_generation: u64,
    terrain_global_generation: u64,
    /// Complete current guide positions, in road-edge order.
    pub(crate) vertices: Vec<Vector3>,
    /// Colors corresponding one-to-one to the current positions.
    pub(crate) colors: Vec<Color>,
    /// Number of edges resampled in the last refresh (zero for a warm unchanged cache).
    pub(crate) rebuilt_edges: usize,
}

#[derive(Default)]
struct CachedEdge {
    geometry: Vec<Vector3>,
    outward: Lines,
    offsets: Lines,
    offset_scratch: Vec<(Vector2, Vector2)>,
    road_chunks: Vec<(SurfaceChunkKey, Option<Arc<NetworkMeshData>>)>,
    terrain_patches: Vec<((usize, usize), u64)>,
}

#[derive(Default)]
struct Lines {
    vertices: Vec<Vector3>,
    colors: Vec<Color>,
}

struct Inputs<'a> {
    graph: &'a RegionGraph,
    surface: &'a RoadSurfaceSystem,
    terrain: &'a TerrainSystem,
    meshes: &'a BTreeMap<SurfaceChunkKey, Arc<NetworkMeshData>>,
    terrain_patches: &'a HashMap<(usize, usize), u64>,
}

impl SimCore {
    /// Refreshes changed guides using existing road-mesh identities and terrain patch revisions.
    pub(crate) fn refresh_road_ghost_lines(&mut self) {
        self.road_ghost_lines.refresh(
            Inputs {
                graph: &self.region_graph,
                surface: &self.transit_network.road_surface,
                terrain: &self.heightmap,
                meshes: &self.cached_road_mesh_chunks,
                terrain_patches: &self.terrain_payload_patch_generations,
            },
            self.terrain_payload_global_generation,
        );
    }
}

impl RoadGhostLineCache {
    fn refresh(&mut self, inputs: Inputs<'_>, terrain_global_generation: u64) {
        // Source brushes and whole-world replacement are conservative full invalidations.
        // Road grading changes visual terrain only, covered by persistent patch revisions.
        if self.terrain_source_generation != inputs.terrain.source_generation()
            || self.terrain_global_generation != terrain_global_generation
        {
            self.edges.clear();
            self.terrain_source_generation = inputs.terrain.source_generation();
            self.terrain_global_generation = terrain_global_generation;
        }
        self.edges.resize_with(inputs.graph.edges().len(), || None);
        // O(source points + C log M + changed samples + output vertices), C dependency chunks,
        // M existing mesh chunks. No second spatial index or full-network height resampling.
        // Independent rebuilds use Rayon; output assembly below remains deterministically ordered.
        self.rebuilt_edges = self
            .edges
            .par_iter_mut()
            .zip(inputs.graph.edges().par_iter())
            .map(|(cached, edge)| {
                if edge.deleted || edge.physical_geometry.len() < 2 {
                    *cached = None;
                    return 0;
                }
                let cached = cached.get_or_insert_with(CachedEdge::default);
                let geometry_changed = cached.geometry != edge.physical_geometry;
                if !geometry_changed && cached.matches(&inputs) {
                    return 0;
                }
                if geometry_changed {
                    cached.geometry.clone_from(&edge.physical_geometry);
                }
                cached.rebuild(&inputs);
                if geometry_changed {
                    cached.capture_dependency_keys(&inputs);
                }
                cached.capture_revisions(&inputs);
                1
            })
            .sum();

        self.vertices.clear();
        self.colors.clear();
        for offsets in [false, true] {
            for cached in self.edges.iter().flatten() {
                let lines = if offsets {
                    &cached.offsets
                } else {
                    &cached.outward
                };
                self.vertices.extend_from_slice(&lines.vertices);
                self.colors.extend_from_slice(&lines.colors);
            }
        }
    }
}

impl CachedEdge {
    fn matches(&self, inputs: &Inputs<'_>) -> bool {
        self.road_chunks
            .iter()
            .all(|(key, old)| match (old, inputs.meshes.get(key)) {
                (None, None) => true,
                (Some(old), Some(current)) => Arc::ptr_eq(old, current),
                _ => false,
            })
            && self.terrain_patches.iter().all(|(key, generation)| {
                *generation == inputs.terrain_patches.get(key).copied().unwrap_or(0)
            })
    }

    fn capture_dependency_keys(&mut self, inputs: &Inputs<'_>) {
        self.road_chunks.clear();
        self.terrain_patches.clear();
        let mut points = self.outward.vertices.iter().chain(&self.offsets.vertices);
        let Some(first) = points.next() else {
            return;
        };
        let (mut min_x, mut min_z, mut max_x, mut max_z) = (first.x, first.z, first.x, first.z);
        for point in points {
            min_x = min_x.min(point.x);
            min_z = min_z.min(point.z);
            max_x = max_x.max(point.x);
            max_z = max_z.max(point.z);
        }
        // Derive coverage from actual sampled points, including endpoint ticks and offset bends.
        // A tiny pad includes both cells for samples exactly on mesh chunk boundaries.
        let (origin_x, origin_z) = inputs.surface.chunk_origin_m();
        let span = inputs.surface.chunk_span_m();
        for x in ((min_x - origin_x - 0.01) / span).floor() as i32
            ..=((max_x - origin_x + 0.01) / span).floor() as i32
        {
            for z in ((min_z - origin_z - 0.01) / span).floor() as i32
                ..=((max_z - origin_z + 0.01) / span).floor() as i32
            {
                self.road_chunks.push(((x, z), None));
            }
        }
        // Bilinear terrain samples can read the neighbouring cell across a patch boundary.
        let margin = inputs.terrain.cell_size_m();
        self.terrain_patches.extend(
            inputs
                .terrain
                .render_patch_keys_for_world_bounds(
                    min_x - margin,
                    min_z - margin,
                    max_x + margin,
                    max_z + margin,
                )
                .into_iter()
                .map(|key| (key, 0)),
        );
    }

    fn capture_revisions(&mut self, inputs: &Inputs<'_>) {
        for (key, mesh) in &mut self.road_chunks {
            *mesh = inputs.meshes.get(key).cloned();
        }
        for (key, generation) in &mut self.terrain_patches {
            *generation = inputs.terrain_patches.get(key).copied().unwrap_or(0);
        }
    }

    fn rebuild(&mut self, inputs: &Inputs<'_>) {
        self.outward.vertices.clear();
        self.outward.colors.clear();
        self.offsets.vertices.clear();
        self.offsets.colors.clear();
        let mut samples = 0;
        let geometry = &self.geometry;
        let end = geometry.len() - 1;
        for (anchor, adjacent) in [
            (geometry[0], geometry[1]),
            (geometry[end], geometry[end - 1]),
        ] {
            if let Some(tangent) = endpoint_tangent_xz(anchor, adjacent) {
                append_outward_ghost_guide(
                    anchor,
                    tangent,
                    inputs.graph,
                    inputs.surface,
                    inputs.terrain,
                    Color::from_rgba(1.0, 1.0, 1.0, 0.30),
                    &mut self.outward.vertices,
                    &mut self.outward.colors,
                    &mut samples,
                );
            }
        }
        for offset_index in 1..=GHOST_MAX_OFFSETS {
            let color = Color::from_rgba(1.0, 1.0, 1.0, GHOST_OFFSET_ALPHAS[offset_index - 1]);
            let offset = offset_index as f32 * GHOST_GRID_SPACING_M;
            for offset in [offset, -offset] {
                append_offset_ghost_curve(
                    geometry,
                    &mut self.offset_scratch,
                    offset,
                    inputs.graph,
                    inputs.surface,
                    inputs.terrain,
                    color,
                    &mut self.offsets.vertices,
                    &mut self.offsets.colors,
                    &mut samples,
                );
            }
        }
    }
}

fn append_outward_ghost_guide(
    anchor: Vector3,
    tangent: Vector2,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    terrain: &TerrainSystem,
    color: Color,
    vertices: &mut Vec<Vector3>,
    colors: &mut Vec<Color>,
    height_samples: &mut usize,
) {
    let perp = Vector2::new(-tangent.y, tangent.x);
    let anchor_xz = Vector2::new(anchor.x, anchor.z);
    let end_xz = anchor_xz + tangent * GHOST_OUTWARD_EXTEND_M;
    push_ghost_surface_line(
        anchor_xz,
        end_xz,
        GHOST_LINE_LIFT_M,
        graph,
        road_surface,
        terrain,
        color,
        vertices,
        colors,
        height_samples,
    );

    let mut dist = GHOST_TICK_INTERVAL_M;
    while dist <= GHOST_OUTWARD_EXTEND_M {
        let tick_center = anchor_xz + tangent * dist;
        push_ghost_surface_line(
            Vector2::new(
                tick_center.x - perp.x * GHOST_TICK_HALF_M,
                tick_center.y - perp.y * GHOST_TICK_HALF_M,
            ),
            Vector2::new(
                tick_center.x + perp.x * GHOST_TICK_HALF_M,
                tick_center.y + perp.y * GHOST_TICK_HALF_M,
            ),
            GHOST_TICK_LIFT_M,
            graph,
            road_surface,
            terrain,
            color,
            vertices,
            colors,
            height_samples,
        );
        dist += GHOST_TICK_INTERVAL_M;
    }
}

fn append_offset_ghost_curve(
    points: &[Vector3],
    offset_segments: &mut Vec<(Vector2, Vector2)>,
    offset_m: f32,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    terrain: &TerrainSystem,
    color: Color,
    vertices: &mut Vec<Vector3>,
    colors: &mut Vec<Color>,
    height_samples: &mut usize,
) {
    if points.len() < 2 {
        return;
    }

    offset_segments.clear();
    for segment in points.windows(2) {
        let a = Vector2::new(segment[0].x, segment[0].z);
        let b = Vector2::new(segment[1].x, segment[1].z);
        let seg = b - a;
        if seg.length_squared() < 0.01 {
            continue;
        }
        let seg_norm = seg.normalized();
        let perp = Vector2::new(-seg_norm.y, seg_norm.x);
        let offset_a = a + perp * offset_m;
        let offset_b = b + perp * offset_m;
        if (offset_b - offset_a).dot(seg_norm) < 0.0 {
            continue;
        }
        offset_segments.push((offset_a, offset_b));
    }

    let mut skip_next = false;
    for index in 0..offset_segments.len() {
        if skip_next {
            skip_next = false;
            continue;
        }
        let (a, b) = offset_segments[index];
        if let Some((next_a, next_b)) = offset_segments.get(index + 1).copied() {
            if segments_cross_2d(a, b, next_a, next_b) {
                skip_next = true;
                continue;
            }
        }
        push_ghost_surface_line(
            a,
            b,
            GHOST_LINE_LIFT_M,
            graph,
            road_surface,
            terrain,
            color,
            vertices,
            colors,
            height_samples,
        );
    }
}

fn push_ghost_surface_line(
    start: Vector2,
    end: Vector2,
    lift_m: f32,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    terrain: &TerrainSystem,
    color: Color,
    vertices: &mut Vec<Vector3>,
    colors: &mut Vec<Color>,
    height_samples: &mut usize,
) {
    let delta = end - start;
    let length = delta.length();
    if length <= 0.01 {
        return;
    }

    let segment_count = (length / GHOST_HEIGHT_SAMPLE_STEP_M).ceil().max(1.0) as usize;
    let mut prev = ghost_surface_point_m(graph, road_surface, terrain, start, lift_m);
    *height_samples += 1;
    for step in 1..=segment_count {
        let t = step as f32 / segment_count as f32;
        let pos = start + delta * t;
        let next = ghost_surface_point_m(graph, road_surface, terrain, pos, lift_m);
        *height_samples += 1;
        vertices.push(prev);
        vertices.push(next);
        colors.push(color);
        colors.push(color);
        prev = next;
    }
}

fn ghost_surface_point_m(
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    terrain: &TerrainSystem,
    pos: Vector2,
    lift_m: f32,
) -> Vector3 {
    Vector3::new(
        pos.x,
        ghost_surface_height_m(graph, road_surface, terrain, pos) + lift_m,
        pos.y,
    )
}

fn ghost_surface_height_m(
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    terrain: &TerrainSystem,
    pos: Vector2,
) -> f32 {
    road_surface
        .sample_visible_surface_height(graph, terrain, pos.x, pos.y)
        .unwrap_or_else(|| terrain.sample_visual_height_world(pos.x, pos.y) * HEIGHT_SCALE)
}

fn segments_cross_2d(a1: Vector2, b1: Vector2, a2: Vector2, b2: Vector2) -> bool {
    let d1 = b1 - a1;
    let d2 = b2 - a2;
    let denom = d1.x * d2.y - d1.y * d2.x;
    if denom.abs() < 1e-6 {
        return false;
    }
    let t = ((a2.x - a1.x) * d2.y - (a2.y - a1.y) * d2.x) / denom;
    let u = ((a2.x - a1.x) * d1.y - (a2.y - a1.y) * d1.x) / denom;
    t > 0.0 && t < 1.0 && u > 0.0 && u < 1.0
}
