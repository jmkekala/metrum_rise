// SPDX-License-Identifier: GPL-2.0-only

//! Compiled roadbed renderer coordinator.

use crate::config::ROAD_DECAL_RENDER_Z_BIAS_M;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::roadbed::{RoadbedSystem, SurfaceChunkKey};
use crate::{debug, debug_log};
use godot::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

use super::{NetworkMeshData, NetworkMeshOwner, TransitRenderer};
use standard_surface::CompiledSurfaceCoverage;

/// Intersection crosswalk markings.
pub mod crosswalks;
/// Cosmetic Finnish crossing and speed-limit sign placement.
pub(crate) mod road_signs;
/// Roadbed rendering: piece surfaces, bridge structures and lane markings.
pub mod standard_surface;
pub(crate) mod street_lights;

pub(super) const MIN_SEGMENT_LEN: f32 = 0.01;
pub(super) const MARKING_RENDER_Z_BIAS_M: f32 = ROAD_DECAL_RENDER_Z_BIAS_M;

// Discriminants index `NetworkMeshData`'s seven layers. The earthwork layer (0) returns with
// road-owned terrain patches in `ROAD-68-D`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MeshLayer {
    Curb = 1,
    RaisedStep = 2,
    Sidewalk = 3,
    Road = 4,
    Marking = 5,
    Concrete = 6,
}

/// Top-surface road renderer built entirely from compiled roadbed ownership.
pub struct RoadRenderer;

impl RoadRenderer {
    /// Generates road mesh data for every published roadbed piece.
    pub fn generate_mesh_data_with_surface(
        &self,
        graph: &RegionGraph,
        lane_system: &crate::simulation::network::lanes::LaneSystem,
        terrain: &crate::simulation::terrain::TerrainSystem,
        roadbed: &RoadbedSystem,
    ) -> NetworkMeshData {
        let compiled_surface = standard_surface::build_compiled_surface_coverage(graph, roadbed);
        let mut mesh = NetworkMeshData::new();

        crosswalks::prepare_crosswalks(&mut mesh, graph, lane_system, &compiled_surface);
        standard_surface::emit_piece_surfaces(&mut mesh, graph, roadbed, &compiled_surface);
        standard_surface::emit_bridge_structures(
            &mut mesh,
            graph,
            roadbed,
            terrain,
            &compiled_surface,
        );
        standard_surface::emit_compiled_lane_markings(&mut mesh, graph, roadbed, &compiled_surface);
        street_lights::emit_street_lights(&mut mesh, graph, roadbed, terrain, &compiled_surface);
        road_signs::emit_road_signs(
            &mut mesh,
            graph,
            lane_system,
            roadbed,
            terrain,
            &compiled_surface,
        );

        mesh
    }

    /// Generates complete replacement meshes for a bounded set of render chunks.
    ///
    /// First lifts the lanes of the covered pieces a compile built after roadbed generation
    /// `lanes_built_after` onto their surface, or of every covered piece when `None`: a piece
    /// that did not change since the last publication left its lanes where they were, so a
    /// local publication re-lifts only the edit's owners, `O(their lane points)`.
    ///
    /// With `lanes_built_after`, `published` holds every chunk of that publication: a target
    /// chunk keeps its unchanged owners' published triangles and only the rebuilt pieces are
    /// emitted again; street lights and signs, placed along whole streets, are placed again for
    /// every covered edge. That is `O(rebuilt pieces + chunk vertices)` instead of
    /// `O(pieces in the chunks)`, and test builds check it equals emitting every piece.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn generate_mesh_chunks_with_surface(
        &self,
        graph: &RegionGraph,
        lane_system: &mut crate::simulation::network::lanes::LaneSystem,
        terrain: &crate::simulation::terrain::TerrainSystem,
        roadbed: &RoadbedSystem,
        target_chunks: &BTreeSet<SurfaceChunkKey>,
        lanes_built_after: Option<u64>,
        published: &BTreeMap<SurfaceChunkKey, Arc<NetworkMeshData>>,
    ) -> BTreeMap<SurfaceChunkKey, NetworkMeshData> {
        let road_debug = debug::Channel::Road.enabled();
        let total_start = road_debug.then(Instant::now);
        let compiled_surface = standard_surface::build_compiled_surface_coverage_for_chunks(
            graph,
            roadbed,
            target_chunks,
            lanes_built_after,
        );
        lane_system.sync_heights_to_visible_surface_for_owners(
            graph,
            roadbed,
            &compiled_surface.rebuilt_edges,
            &compiled_surface.rebuilt_nodes,
        );
        let lane_system = &*lane_system;
        let emit = |pieces: &CompiledSurfaceCoverage| {
            emit_chunks(
                graph,
                lane_system,
                terrain,
                roadbed,
                target_chunks,
                pieces,
                &compiled_surface,
            )
        };
        let spliced = lanes_built_after.and_then(|_| {
            let mut fresh = emit(&compiled_surface.rebuilt());
            splice_chunks(target_chunks, published, &mut fresh, |owner| {
                compiled_surface.keeps(owner)
            })
        });
        let was_spliced = spliced.is_some();
        let chunks = match spliced {
            Some(chunks) => {
                #[cfg(debug_assertions)]
                assert_same_chunks(&chunks, &emit(&compiled_surface));
                chunks
            }
            None => emit(&compiled_surface),
        };
        if road_debug {
            debug_log!(
                "road",
                "road_mesh_generate target_chunks={} covered_edges={} covered_nodes={} rebuilt_edges={} rebuilt_nodes={} spliced={} output_chunks={} vertices={} total_ms={:.3}",
                target_chunks.len(),
                compiled_surface.edge_indices.len(),
                compiled_surface.node_ids.len(),
                compiled_surface.rebuilt_edges.len(),
                compiled_surface.rebuilt_nodes.len(),
                was_spliced,
                chunks.len(),
                chunks
                    .values()
                    .map(NetworkMeshData::vertex_count)
                    .sum::<usize>(),
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }
        chunks
    }
}

// Emits `pieces` (surfaces, bridges, markings and the zebras they carry) and the street lights
// and signs of every `covered` edge into the target chunks.
fn emit_chunks(
    graph: &RegionGraph,
    lane_system: &crate::simulation::network::lanes::LaneSystem,
    terrain: &crate::simulation::terrain::TerrainSystem,
    roadbed: &RoadbedSystem,
    target_chunks: &BTreeSet<SurfaceChunkKey>,
    pieces: &CompiledSurfaceCoverage,
    covered: &CompiledSurfaceCoverage,
) -> BTreeMap<SurfaceChunkKey, NetworkMeshData> {
    let (chunk_origin_x_m, chunk_origin_z_m) = roadbed.chunk_origin_m();
    let mut mesh = NetworkMeshData::new_chunk_partitioned(
        roadbed.chunk_span_m(),
        chunk_origin_x_m,
        chunk_origin_z_m,
        target_chunks.clone(),
    );
    crosswalks::prepare_crosswalks(&mut mesh, graph, lane_system, pieces);
    standard_surface::emit_piece_surfaces(&mut mesh, graph, roadbed, pieces);
    standard_surface::emit_bridge_structures(&mut mesh, graph, roadbed, terrain, pieces);
    standard_surface::emit_compiled_lane_markings(&mut mesh, graph, roadbed, pieces);
    street_lights::emit_street_lights(&mut mesh, graph, roadbed, terrain, covered);
    road_signs::emit_road_signs(&mut mesh, graph, lane_system, roadbed, terrain, covered);
    mesh.into_partitioned_chunks()
}

// The target chunks assembled from the `published` meshes and `fresh` (the rebuilt pieces'
// triangles with every covered edge's lights and signs), keeping the published triangles of
// the owners `keep` admits; `None` if a published chunk holds a run without an owner.
fn splice_chunks(
    target_chunks: &BTreeSet<SurfaceChunkKey>,
    published: &BTreeMap<SurfaceChunkKey, Arc<NetworkMeshData>>,
    fresh: &mut BTreeMap<SurfaceChunkKey, NetworkMeshData>,
    keep: impl Fn(NetworkMeshOwner) -> bool,
) -> Option<BTreeMap<SurfaceChunkKey, NetworkMeshData>> {
    let mut chunks = BTreeMap::new();
    for chunk in target_chunks {
        let fresh_chunk = fresh.remove(chunk).unwrap_or_default();
        let mesh = match published.get(chunk) {
            Some(old) => NetworkMeshData::splice_owners(old, &fresh_chunk, &keep)?,
            None => fresh_chunk,
        };
        if !mesh.is_empty() {
            chunks.insert(*chunk, mesh);
        }
    }
    Some(chunks)
}

// Test builds: a spliced publication equals emitting every covered piece, chunk for chunk.
#[cfg(debug_assertions)]
fn assert_same_chunks(
    spliced: &BTreeMap<SurfaceChunkKey, NetworkMeshData>,
    full: &BTreeMap<SurfaceChunkKey, NetworkMeshData>,
) {
    let filled = |chunks: &BTreeMap<SurfaceChunkKey, NetworkMeshData>| {
        chunks
            .iter()
            .filter(|(_, mesh)| !mesh.is_empty())
            .map(|(&chunk, _)| chunk)
            .collect::<Vec<_>>()
    };
    assert_eq!(filled(spliced), filled(full), "spliced road chunks differ");
    for (chunk, mesh) in spliced {
        assert!(
            mesh.same_payload(&full[chunk]),
            "spliced road chunk {chunk:?} differs from emitting every piece"
        );
    }
}

impl TransitRenderer for RoadRenderer {
    fn generate_mesh_data(
        &self,
        graph: &RegionGraph,
        lane_system: &crate::simulation::network::lanes::LaneSystem,
        terrain: &crate::simulation::terrain::TerrainSystem,
    ) -> NetworkMeshData {
        let mut temporary_roadbed = RoadbedSystem::new(RegionGraph::CHUNK_SIZE);
        temporary_roadbed.compile_dirty(graph);
        self.generate_mesh_data_with_surface(graph, lane_system, terrain, &temporary_roadbed)
    }
}

#[cfg(test)]
pub(super) fn push_quad(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    vertices: [Vector3; 4],
    uvs: [Vector2; 4],
    color: Color,
) {
    push_triangle(
        mesh,
        layer,
        [vertices[0], vertices[1], vertices[2]],
        [uvs[0], uvs[1], uvs[2]],
        color,
    );
    push_triangle(
        mesh,
        layer,
        [vertices[0], vertices[2], vertices[3]],
        [uvs[0], uvs[2], uvs[3]],
        color,
    );
}

pub(super) fn push_triangle(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    mut vertices: [Vector3; 3],
    mut uvs: [Vector2; 3],
    color: Color,
) {
    let projected_winding = (vertices[1].x - vertices[0].x) * (vertices[2].z - vertices[0].z)
        - (vertices[1].z - vertices[0].z) * (vertices[2].x - vertices[0].x);
    if projected_winding < 0.0 {
        vertices.swap(1, 2);
        uvs.swap(1, 2);
    }

    push_triangle_preserving_winding(mesh, layer, vertices, uvs, color);
}

pub(super) fn push_triangle_preserving_winding(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    vertices: [Vector3; 3],
    uvs: [Vector2; 3],
    color: Color,
) {
    let mut normal = (vertices[1] - vertices[0]).cross(vertices[2] - vertices[0]);
    if normal.length_squared() <= 1e-8 {
        normal = Vector3::UP;
    } else {
        if normal.y < 0.0 {
            normal = -normal;
        }
        normal = normal.normalized();
    }

    push_triangle_to_layer(mesh, layer, vertices, uvs, color, normal);
}

pub(super) fn push_triangle_preserving_winding_with_exact_normal(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    vertices: [Vector3; 3],
    uvs: [Vector2; 3],
    color: Color,
    mut normal: Vector3,
) {
    if normal.length_squared() <= 1e-8 {
        normal = Vector3::UP;
    } else {
        normal = normal.normalized();
    }

    push_triangle_to_layer(mesh, layer, vertices, uvs, color, normal);
}

impl NetworkMeshData {
    // Signs use the same chunk ownership and local-coordinate contract as pole instances.
    fn push_road_sign(&mut self, mut sign: road_signs::RoadSign) {
        if let Some(partition) = self.chunk_partition.as_mut() {
            let span = f64::from(partition.chunk_span_m);
            let chunk = (
                ((f64::from(sign.position.x) - f64::from(partition.chunk_origin_x_m)) / span)
                    .floor() as i32,
                ((f64::from(sign.position.z) - f64::from(partition.chunk_origin_z_m)) / span)
                    .floor() as i32,
            );
            if !partition.target_chunks.contains(&chunk) {
                return;
            }
            sign.position.x -= partition.chunk_origin_x_m + chunk.0 as f32 * partition.chunk_span_m;
            sign.position.z -= partition.chunk_origin_z_m + chunk.1 as f32 * partition.chunk_span_m;
            partition
                .chunks
                .entry(chunk)
                .or_insert_with(NetworkMeshData::new)
                .road_signs
                .push(sign);
            return;
        }
        self.road_signs.push(sign);
    }

    // Routes a light to its chunk like a triangle centroid, in chunk-local coordinates; lights
    // outside the target chunks belong to another rebuild and are dropped.
    fn push_street_light(&mut self, mut light: street_lights::StreetLight) {
        if let Some(partition) = self.chunk_partition.as_mut() {
            // Keep world-space poles for sign clearance, including poles outside target chunks.
            self.street_lights.push(light);
            let chunk_span_m = f64::from(partition.chunk_span_m);
            let chunk = (
                ((f64::from(light.position.x) - f64::from(partition.chunk_origin_x_m))
                    / chunk_span_m)
                    .floor() as i32,
                ((f64::from(light.position.z) - f64::from(partition.chunk_origin_z_m))
                    / chunk_span_m)
                    .floor() as i32,
            );
            if !partition.target_chunks.contains(&chunk) {
                return;
            }
            light.position.x -=
                partition.chunk_origin_x_m + chunk.0 as f32 * partition.chunk_span_m;
            light.position.z -=
                partition.chunk_origin_z_m + chunk.1 as f32 * partition.chunk_span_m;
            partition
                .chunks
                .entry(chunk)
                .or_insert_with(NetworkMeshData::new)
                .street_lights
                .push(light);
            return;
        }
        self.street_lights.push(light);
    }
}

fn push_triangle_to_layer(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    vertices: [Vector3; 3],
    uvs: [Vector2; 3],
    color: Color,
    normal: Vector3,
) {
    if matches!(layer, MeshLayer::Road) {
        let triangle = std::array::from_fn(|i| super::ownership::Vertex {
            p: vertices[i],
            n: normal,
            uv: uvs[i],
            c: color,
            crossing: Vector4::ZERO,
        });
        crosswalks::emit_triangle(mesh, triangle, 0);
    } else {
        push_triangle_to_layer_with_crossing(
            mesh,
            layer,
            vertices,
            uvs,
            color,
            normal,
            [Vector4::ZERO; 3],
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn push_triangle_to_layer_with_crossing(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    vertices: [Vector3; 3],
    uvs: [Vector2; 3],
    color: Color,
    normal: Vector3,
    crossing: [Vector4; 3],
) {
    if (vertices[1] - vertices[0])
        .cross(vertices[2] - vertices[0])
        .length_squared()
        == 0.0
    {
        return;
    }
    if let Some(partition) = mesh.chunk_partition.as_mut() {
        let centroid_x =
            (f64::from(vertices[0].x) + f64::from(vertices[1].x) + f64::from(vertices[2].x)) / 3.0;
        let centroid_z =
            (f64::from(vertices[0].z) + f64::from(vertices[1].z) + f64::from(vertices[2].z)) / 3.0;
        let chunk_span_m = f64::from(partition.chunk_span_m);
        let chunk = (
            ((centroid_x - f64::from(partition.chunk_origin_x_m)) / chunk_span_m).floor() as i32,
            ((centroid_z - f64::from(partition.chunk_origin_z_m)) / chunk_span_m).floor() as i32,
        );
        if !partition.target_chunks.contains(&chunk) {
            return;
        }
        let origin_x = partition.chunk_origin_x_m + chunk.0 as f32 * partition.chunk_span_m;
        let origin_z = partition.chunk_origin_z_m + chunk.1 as f32 * partition.chunk_span_m;
        let local_vertices =
            vertices.map(|vertex| Vector3::new(vertex.x - origin_x, vertex.y, vertex.z - origin_z));
        let chunk_mesh = partition
            .chunks
            .entry(chunk)
            .or_insert_with(NetworkMeshData::new);
        chunk_mesh.current_owner = mesh.current_owner;
        push_triangle_to_layer_with_crossing(
            chunk_mesh,
            layer,
            local_vertices,
            uvs,
            color,
            normal,
            crossing,
        );
        return;
    }

    if matches!(layer, MeshLayer::Road) {
        mesh.road_crossings.extend_from_slice(&crossing);
    }
    let target = match layer {
        MeshLayer::Curb => (
            &mut mesh.curb_vertices,
            &mut mesh.curb_normals,
            &mut mesh.curb_uvs,
            &mut mesh.curb_colors,
        ),
        MeshLayer::RaisedStep => (
            &mut mesh.raised_step_vertices,
            &mut mesh.raised_step_normals,
            &mut mesh.raised_step_uvs,
            &mut mesh.raised_step_colors,
        ),
        MeshLayer::Sidewalk => (
            &mut mesh.sidewalk_vertices,
            &mut mesh.sidewalk_normals,
            &mut mesh.sidewalk_uvs,
            &mut mesh.sidewalk_colors,
        ),
        MeshLayer::Road => (
            &mut mesh.road_vertices,
            &mut mesh.road_normals,
            &mut mesh.road_uvs,
            &mut mesh.road_colors,
        ),
        MeshLayer::Marking => (
            &mut mesh.marking_vertices,
            &mut mesh.marking_normals,
            &mut mesh.marking_uvs,
            &mut mesh.marking_colors,
        ),
        MeshLayer::Concrete => (
            &mut mesh.concrete_vertices,
            &mut mesh.concrete_normals,
            &mut mesh.concrete_uvs,
            &mut mesh.concrete_colors,
        ),
    };
    for index in 0..3 {
        target.0.push(vertices[index]);
        target.1.push(normal);
        target.2.push(uvs[index]);
        target.3.push(color);
    }
    let end = target.0.len();
    mesh.record_owner_triangle(layer as usize, end);
}

// Per-vertex road description for the asphalt shader (road_asphalt.gdshader), in 8-bit
// channels: forward and backward lane counts, the speed limit in km/h, and a cosmetic age in
// the upper half of alpha. Junction asphalt keeps a zero alpha: it has no road space.
pub(super) fn road_space_color(edge: &crate::simulation::network::graph::Edge) -> Color {
    let channel = |value: f32| value.round().clamp(0.0, 255.0) / 255.0;
    Color::from_rgba(
        channel(f32::from(edge.fwd_lanes)),
        channel(f32::from(edge.bkw_lanes)),
        channel(edge.speed_limit * 3.6),
        channel(128.0 + 127.0 * cosmetic_road_age(edge)),
    )
}

// Per-vertex footpath description for road_path.gdshader: half-width in decimetres, a full green
// channel marking the vertex as footpath (roadside sidewalks keep `sidewalk_color`'s zero), the
// surface kind (0 asphalt), and the cosmetic age as in `road_space_color`.
pub(super) fn footpath_color(edge: &crate::simulation::network::graph::Edge) -> Color {
    let channel = |value: f32| value.round().clamp(0.0, 255.0) / 255.0;
    Color::from_rgba(
        channel(edge.width.max(2.0) * 0.5 * 10.0),
        1.0,
        0.0,
        channel(128.0 + 127.0 * cosmetic_road_age(edge)),
    )
}

// Junction asphalt: zero alpha, so the asphalt shader draws it without road space.
pub(super) fn road_color() -> Color {
    Color::from_rgba(0.0, 0.0, 0.0, 0.0)
}

pub(super) fn curb_color() -> Color {
    Color::from_rgba(0.56, 0.55, 0.53, 1.0)
}

pub(super) fn sidewalk_color() -> Color {
    Color::from_rgba(0.0, 0.0, 0.0, 1.0)
}

// A stable 0..1 look age from where the edge lies, so a road keeps its wear across saves, splits
// of other roads and edge-id compaction. Cosmetic only: nothing in the simulation reads it.
fn cosmetic_road_age(edge: &crate::simulation::network::graph::Edge) -> f32 {
    let (Some(first), Some(last)) = (edge.geometry.first(), edge.geometry.last()) else {
        return 0.5;
    };
    // Direction-independent: the same road drawn either way ages alike.
    let key = |p: &Vector3| ((p.x * 2.0).round() as i64, (p.z * 2.0).round() as i64);
    let (a, b) = (key(first).min(key(last)), key(first).max(key(last)));
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for value in [a.0, a.1, b.0, b.1] {
        hash = (hash ^ value as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    (hash >> 40) as f32 / (1u64 << 24) as f32
}

pub(super) fn concrete_color() -> Color {
    Color::from_rgba(0.75, 0.75, 0.75, 1.0)
}

pub(super) fn marking_dash_color() -> Color {
    Color::from_rgba(1.0, 1.0, 1.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::marking_dash_color;

    #[test]
    fn lane_marking_colors_are_visible() {
        assert!(marking_dash_color().a > 0.0);
    }
}
