// SPDX-License-Identifier: GPL-2.0-only

//! Compiled roadbed renderer coordinator.

use crate::config::ROAD_DECAL_RENDER_Z_BIAS_M;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::roadbed::{RoadVec3, RoadbedSystem, SurfaceChunkKey};
use crate::simulation::terrain::render_frame_local;
use crate::simulation::terrain::graded::{NoSites, SiteSource};
use crate::{debug, debug_log};
use godot::prelude::*;
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

use super::{NetworkMeshData, NetworkMeshOwner, RoadChunkMap, TransitRenderer};
use crate::simulation::network::roadbed::PieceId;
use standard_surface::CompiledSurfaceCoverage;

mod chunks;
/// Intersection crosswalk markings.
pub mod crosswalks;
/// Cosmetic Finnish crossing and speed-limit sign placement.
pub(crate) mod road_signs;
/// Roadbed rendering: piece surfaces, bridge structures and lane markings.
pub mod standard_surface;
pub(crate) mod street_lights;

pub(super) const MIN_SEGMENT_LEN: f32 = 0.01;
pub(super) const MARKING_RENDER_Z_BIAS_M: f32 = ROAD_DECAL_RENDER_Z_BIAS_M;

// Discriminants index `NetworkMeshData`'s seven layers.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MeshLayer {
    Skirt = 0,
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
        terrain: &crate::simulation::terrain::TerrainSystem,
        roadbed: &RoadbedSystem,
    ) -> NetworkMeshData {
        let compiled_surface = standard_surface::build_compiled_surface_coverage(graph, roadbed);
        let mut mesh = NetworkMeshData::new();
        emit_into(
            &mut mesh,
            graph,
            terrain,
            roadbed,
            &NoSites,
            &compiled_surface,
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
        sites: &dyn SiteSource,
        roadbed: &RoadbedSystem,
        target_chunks: &BTreeSet<SurfaceChunkKey>,
        lanes_built_after: Option<u64>,
        published: &RoadChunkMap,
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
        let emit = |pieces: &CompiledSurfaceCoverage| {
            emit_chunks(
                graph,
                terrain,
                roadbed,
                sites,
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

/// One render chunk of a road-tool preview, below the chunk (`ROAD-68-H` round 3, open question
/// 63 (b)): what Godot draws over the resident published chunk and which of its runs it
/// collapses meanwhile. The published chunk with those runs dropped and the overlay merged is the
/// chunk the commit publishes from the same pieces, lights and signs aside.
#[derive(Debug)]
pub(crate) struct PreviewChunk {
    /// The rebuilt pieces' triangles in the chunk's frame, sealed, without street lights or
    /// signs: the commit's own emission of exactly those pieces.
    pub(crate) overlay: Arc<NetworkMeshData>,
    /// `[layer, start, end]` vertex ranges of the published chunk's runs the overlay replaces,
    /// by layer, ascending; empty where nothing is published.
    pub(crate) hidden: Vec<[u32; 3]>,
    /// The published chunk's positions in `hidden`'s ranges, in their order: what Godot puts
    /// back when a run is shown again.
    pub(crate) hidden_positions: Vec<Vector3>,
}

impl RoadRenderer {
    /// The road-tool preview's sub-chunk payload (`ROAD-68-H` round 3): for each target chunk
    /// with anything to change, the rebuilt pieces' triangles (`closure`'s owners with a piece,
    /// emitted as the commit emits them) and the runs of the `published` chunk that `closure`'s
    /// owners hold, which are exactly what the commit's splice drops (test builds check both
    /// against the splice). `closure` holds every owner the preview's delta covers, with whether
    /// it built a piece. `None` if a searched run has no owner.
    /// `O(rebuilt pieces' triangles + closure * layers * log runs + hidden vertices)`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn generate_preview_overlay(
        &self,
        graph: &RegionGraph,
        terrain: &crate::simulation::terrain::TerrainSystem,
        sites: &dyn SiteSource,
        roadbed: &RoadbedSystem,
        target_chunks: &BTreeSet<SurfaceChunkKey>,
        closure: &[(PieceId, bool)],
        built_after: u64,
        published: &RoadChunkMap,
    ) -> Option<BTreeMap<SurfaceChunkKey, PreviewChunk>> {
        let rebuilt = standard_surface::coverage_of_pieces(
            graph,
            closure.iter().filter(|(_, built)| *built).map(|(id, _)| *id),
        );
        let mut overlay = emit_chunks(
            graph,
            terrain,
            roadbed,
            sites,
            target_chunks,
            &rebuilt,
            &standard_surface::coverage_of_pieces(graph, []),
        );
        let mut owners: Vec<_> = closure
            .iter()
            .map(|(id, _)| match *id {
                PieceId::Span(edge) => NetworkMeshOwner::Edge(edge),
                PieceId::Node(node) => NetworkMeshOwner::Node(node),
            })
            .collect();
        owners.sort_unstable_by_key(|&owner| super::ownership::owner_order(owner));
        owners.dedup();
        #[cfg(debug_assertions)]
        let splice = standard_surface::build_compiled_surface_coverage_for_chunks(
            graph,
            roadbed,
            target_chunks,
            Some(built_after),
        );
        #[cfg(debug_assertions)]
        assert!(
            splice.rebuilt_edges == rebuilt.edge_indices
                && splice.rebuilt_nodes == rebuilt.node_ids,
            "the overlay emits exactly the splice's fresh pieces"
        );
        #[cfg(not(debug_assertions))]
        let _ = built_after;
        let mut chunks = BTreeMap::new();
        for &chunk in target_chunks {
            let overlay = overlay.remove(&chunk).unwrap_or_default();
            let mut hidden_positions = Vec::new();
            let hidden = match published.get(&chunk) {
                Some(mesh) => {
                    let hidden = mesh.owner_vertex_ranges(&owners, &mut hidden_positions)?;
                    #[cfg(debug_assertions)]
                    assert_eq!(
                        hidden,
                        mesh.vertex_ranges_dropped(|owner| splice.keeps(owner)),
                        "chunk {chunk:?}: the hidden runs are the runs the splice drops"
                    );
                    hidden
                }
                None => Vec::new(),
            };
            if overlay.is_empty() && hidden.is_empty() {
                continue;
            }
            chunks.insert(
                chunk,
                PreviewChunk {
                    overlay: overlay.seal(),
                    hidden,
                    hidden_positions,
                },
            );
        }
        Some(chunks)
    }
}

// Emits `pieces` (surfaces, bridges, markings and the zebras they carry) and the street lights
// and signs of every `covered` edge into the target chunks.
fn emit_chunks(
    graph: &RegionGraph,
    terrain: &crate::simulation::terrain::TerrainSystem,
    roadbed: &RoadbedSystem,
    sites: &dyn SiteSource,
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
    emit_into(&mut mesh, graph, terrain, roadbed, sites, pieces, covered);
    mesh.into_partitioned_chunks()
}

// Emits `pieces` and the street lights and signs of every `covered` edge into `mesh`. It reads
// only the graph, the pieces and the terrain, never lanes, so a preview's pieces emit the chunks
// the commit publishes from the same pieces (`ROAD-68-F`).
fn emit_into(
    mesh: &mut NetworkMeshData,
    graph: &RegionGraph,
    terrain: &crate::simulation::terrain::TerrainSystem,
    roadbed: &RoadbedSystem,
    sites: &dyn SiteSource,
    pieces: &CompiledSurfaceCoverage,
    covered: &CompiledSurfaceCoverage,
) {
    // Lights and signs read only the graph, roadbed and terrain (signs keep clear of the
    // lights), so they are placed while the pieces are emitted, then pushed after them as the
    // serial passes pushed them: each chunk's payload is the same.
    let (lights, signs) = crate::thread_pools::install(|| {
        rayon::join(
            || {
                crosswalks::prepare_crosswalks(mesh, graph, roadbed, pieces);
                standard_surface::emit_piece_surfaces(mesh, graph, roadbed, pieces);
                standard_surface::emit_bridge_structures(
                    mesh, graph, roadbed, terrain, sites, pieces,
                );
                standard_surface::emit_compiled_lane_markings(mesh, graph, roadbed, pieces);
            },
            || {
                let lights = street_lights::place_street_lights(graph, roadbed, terrain, covered);
                let signs = road_signs::place_road_signs(
                    graph,
                    roadbed,
                    terrain,
                    covered,
                    &lights,
                );
                (lights, signs)
            },
        )
        .1
    });
    debug_assert!(mesh.street_lights.is_empty() && mesh.road_signs.is_empty());
    for light in lights {
        mesh.push_street_light(light);
    }
    for sign in signs {
        mesh.push_road_sign(sign);
    }
}

// The target chunks assembled from the `published` meshes and `fresh` (the rebuilt pieces'
// triangles with every covered edge's lights and signs), keeping the published triangles of
// the owners `keep` admits; `None` if a published chunk holds a run without an owner.
fn splice_chunks(
    target_chunks: &BTreeSet<SurfaceChunkKey>,
    published: &RoadChunkMap,
    fresh: &mut BTreeMap<SurfaceChunkKey, NetworkMeshData>,
    keep: impl Fn(NetworkMeshOwner) -> bool + Sync,
) -> Option<BTreeMap<SurfaceChunkKey, NetworkMeshData>> {
    // Each chunk is spliced from its own published and fresh meshes alone, so on a pool worker
    // (the commit's publication) the chunks are spliced in parallel and collected in chunk
    // order (`ROAD-68-G` part 7). Outside the pools (the road tool's preview, a few chunks)
    // they are spliced here: entering the pool cost the preview more than it saved. The chunks
    // are the same either way. O(chunks) setup.
    let chunks: Vec<_> = target_chunks
        .iter()
        .map(|&chunk| (chunk, fresh.remove(&chunk).unwrap_or_default()))
        .collect();
    let splice = |(chunk, fresh_chunk): (SurfaceChunkKey, NetworkMeshData)| {
        let mesh = match published.get(&chunk) {
            Some(old) => NetworkMeshData::splice_owners(old, &fresh_chunk, &keep)?,
            None => fresh_chunk,
        };
        Some((chunk, mesh))
    };
    let spliced: Option<Vec<_>> = if rayon::current_thread_index().is_some() {
        chunks.into_par_iter().map(splice).collect()
    } else {
        chunks.into_iter().map(splice).collect()
    };
    Some(
        spliced?
            .into_iter()
            .filter(|(_, mesh)| !mesh.is_empty())
            .collect(),
    )
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
        terrain: &crate::simulation::terrain::TerrainSystem,
    ) -> NetworkMeshData {
        let mut temporary_roadbed = RoadbedSystem::new(RegionGraph::CHUNK_SIZE);
        temporary_roadbed.compile_dirty(graph);
        self.generate_mesh_data_with_surface(graph, terrain, &temporary_roadbed)
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
    let vertices = vertices.map(world_point);
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

// World metres of an f32 render vertex, exactly.
pub(super) fn world_point(vertex: Vector3) -> RoadVec3 {
    RoadVec3::new(f64::from(vertex.x), f64::from(vertex.y), f64::from(vertex.z))
}

// The f32 world vertex of a world point, for winding, normals and paint frames.
pub(super) fn render_point(point: RoadVec3) -> Vector3 {
    Vector3::new(point.x as f32, point.y as f32, point.z as f32)
}

pub(super) fn push_triangle(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    mut vertices: [RoadVec3; 3],
    mut uvs: [Vector2; 3],
    color: Color,
) {
    let points = vertices.map(render_point);
    let projected_winding = (points[1].x - points[0].x) * (points[2].z - points[0].z)
        - (points[1].z - points[0].z) * (points[2].x - points[0].x);
    if projected_winding < 0.0 {
        vertices.swap(1, 2);
        uvs.swap(1, 2);
    }

    push_triangle_preserving_winding(mesh, layer, vertices, uvs, color);
}

pub(super) fn push_triangle_preserving_winding(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    vertices: [RoadVec3; 3],
    uvs: [Vector2; 3],
    color: Color,
) {
    let points = vertices.map(render_point);
    let mut normal = (points[1] - points[0]).cross(points[2] - points[0]);
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
    vertices: [RoadVec3; 3],
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
            let (centre_x, centre_z) = partition.frame_centre(chunk);
            sign.position.x = render_frame_local(f64::from(sign.position.x), centre_x);
            sign.position.z = render_frame_local(f64::from(sign.position.z), centre_z);
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

    // Routes a light to the chunk holding its pole, in that chunk's frame; lights outside the
    // target chunks belong to another rebuild and are dropped.
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
            let (centre_x, centre_z) = partition.frame_centre(chunk);
            light.position.x = render_frame_local(f64::from(light.position.x), centre_x);
            light.position.z = render_frame_local(f64::from(light.position.z), centre_z);
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
    vertices: [RoadVec3; 3],
    uvs: [Vector2; 3],
    color: Color,
    normal: Vector3,
) {
    if matches!(layer, MeshLayer::Road) {
        crosswalks::emit_triangle(mesh, vertices, uvs, color, normal);
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

// A partitioned mesh routes the triangle into its chunks' frames (`chunks.rs`); a plain mesh
// keeps it in f32 world space.
#[allow(clippy::too_many_arguments)]
fn push_triangle_to_layer_with_crossing(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    vertices: [RoadVec3; 3],
    uvs: [Vector2; 3],
    color: Color,
    normal: Vector3,
    crossing: [Vector4; 3],
) {
    if let Some(partition) = mesh.chunk_partition.as_mut() {
        let owner = mesh.current_owner;
        let triangle = chunks::WorldTriangle {
            points: vertices,
            uvs,
            paint: crossing,
        };
        partition.route(&triangle, |chunk_mesh, local, uvs, paint| {
            chunk_mesh.current_owner = owner;
            chunk_mesh.push_layer_triangle(layer, local, uvs, color, normal, paint);
        });
        return;
    }
    mesh.push_layer_triangle(layer, vertices.map(render_point), uvs, color, normal, crossing);
}

impl NetworkMeshData {
    // Appends a triangle in this mesh's own coordinates; one exactly degenerate in f32 is skipped.
    fn push_layer_triangle(
        &mut self,
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
        if matches!(layer, MeshLayer::Road) {
            self.road_crossings.extend_from_slice(&crossing);
        }
        let target = match layer {
            MeshLayer::Skirt => (
                &mut self.skirt_vertices,
                &mut self.skirt_normals,
                &mut self.skirt_uvs,
                &mut self.skirt_colors,
            ),
            MeshLayer::Curb => (
                &mut self.curb_vertices,
                &mut self.curb_normals,
                &mut self.curb_uvs,
                &mut self.curb_colors,
            ),
            MeshLayer::RaisedStep => (
                &mut self.raised_step_vertices,
                &mut self.raised_step_normals,
                &mut self.raised_step_uvs,
                &mut self.raised_step_colors,
            ),
            MeshLayer::Sidewalk => (
                &mut self.sidewalk_vertices,
                &mut self.sidewalk_normals,
                &mut self.sidewalk_uvs,
                &mut self.sidewalk_colors,
            ),
            MeshLayer::Road => (
                &mut self.road_vertices,
                &mut self.road_normals,
                &mut self.road_uvs,
                &mut self.road_colors,
            ),
            MeshLayer::Marking => (
                &mut self.marking_vertices,
                &mut self.marking_normals,
                &mut self.marking_uvs,
                &mut self.marking_colors,
            ),
            MeshLayer::Concrete => (
                &mut self.concrete_vertices,
                &mut self.concrete_normals,
                &mut self.concrete_uvs,
                &mut self.concrete_colors,
            ),
        };
        for index in 0..3 {
            target.0.push(vertices[index]);
            target.1.push(normal);
            target.2.push(uvs[index]);
            target.3.push(color);
        }
        let end = target.0.len();
        self.record_owner_triangle(layer as usize, end);
    }
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
