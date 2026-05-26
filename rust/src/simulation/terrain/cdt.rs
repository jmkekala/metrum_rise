//! Deterministic constrained triangulation for road-touched terrain patches.
//!
//! This module owns the Rust-side CDT kernel used by terrain patches. It deliberately
//! does not depend on Godot types: callers adapt road-piece loops and terrain samples
//! into this small data model, then convert the returned indexed mesh to renderer
//! buffers at the boundary.

#![cfg_attr(not(test), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet, HashSet};

use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};

const CDT_EPSILON_M: f64 = 0.001;
const MAX_INVALID_CONSTRAINT_SAMPLES: usize = 8;
const MAX_ROAD_SEAM_FACE_SAMPLES: usize = 8;
const MAX_SEAM_QUALITY_SAMPLES: usize = 8;
const MAX_TIE_IN_SAMPLE_DIAGNOSTICS: usize = 8;
const MAX_TERRAIN_TIE_IN_SLOPE_RATIO: f32 = 0.5;
const MIN_TIE_IN_HEIGHT_DELTA_M: f32 = 0.01;
const MIN_RETAINING_WALL_TIE_IN_HEIGHT_DELTA_M: f32 = 0.5;
const MIN_SOURCE_OWNED_SEAM_EDGE_LENGTH_M: f64 = 0.05;

type SpadeCdt = ConstrainedDelaunayTriangulation<Point2<f64>>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainCdtVertex {
    pub(crate) x: f64,
    pub(crate) height_m: f32,
    pub(crate) z: f64,
}

impl TerrainCdtVertex {
    pub(crate) fn new(x: f64, height_m: f32, z: f64) -> Self {
        Self { x, height_m, z }
    }

    fn point2(self) -> Point2<f64> {
        Point2::new(self.x, self.z)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainCdtPatch {
    pub(crate) min_x: f64,
    pub(crate) min_z: f64,
    pub(crate) max_x: f64,
    pub(crate) max_z: f64,
    pub(crate) corner_heights_m: [f32; 4],
}

impl TerrainCdtPatch {
    pub(crate) fn new(
        min_x: f64,
        min_z: f64,
        max_x: f64,
        max_z: f64,
        corner_heights_m: [f32; 4],
    ) -> Self {
        Self {
            min_x,
            min_z,
            max_x,
            max_z,
            corner_heights_m,
        }
    }

    fn is_valid(self) -> bool {
        self.max_x > self.min_x + CDT_EPSILON_M && self.max_z > self.min_z + CDT_EPSILON_M
    }

    fn corners_cw(self) -> [TerrainCdtVertex; 4] {
        [
            TerrainCdtVertex::new(self.min_x, self.corner_heights_m[0], self.min_z),
            TerrainCdtVertex::new(self.min_x, self.corner_heights_m[1], self.max_z),
            TerrainCdtVertex::new(self.max_x, self.corner_heights_m[2], self.max_z),
            TerrainCdtVertex::new(self.max_x, self.corner_heights_m[3], self.min_z),
        ]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TerrainCdtRoadLoop {
    pub(crate) stable_piece_id: u64,
    pub(crate) footprint_group_id: u64,
    pub(crate) local_loop_index: u32,
    pub(crate) is_hole: bool,
    pub(crate) vertices: Vec<TerrainCdtVertex>,
    pub(crate) source_edges: Vec<TerrainCdtRoadLoopSourceEdge>,
}

impl TerrainCdtRoadLoop {
    pub(crate) fn new(
        stable_piece_id: u64,
        local_loop_index: u32,
        vertices: Vec<TerrainCdtVertex>,
    ) -> Self {
        let source_edges = if vertices.is_empty() {
            Vec::new()
        } else {
            vertices
                .iter()
                .copied()
                .enumerate()
                .map(|(index, start)| TerrainCdtRoadLoopSourceEdge {
                    start,
                    end: vertices[(index + 1) % vertices.len()],
                    source: TerrainCdtRoadBoundarySource::SyntheticTestBoundary {
                        stable_piece_id,
                        local_loop_index,
                        local_edge_index: u32::try_from(index).unwrap_or(u32::MAX),
                    },
                })
                .collect()
        };
        Self {
            stable_piece_id,
            footprint_group_id: stable_piece_id,
            local_loop_index,
            is_hole: false,
            vertices,
            source_edges,
        }
    }

    pub(crate) fn new_with_source_edges(
        stable_piece_id: u64,
        local_loop_index: u32,
        vertices: Vec<TerrainCdtVertex>,
        source_edges: Vec<TerrainCdtRoadLoopSourceEdge>,
    ) -> Self {
        Self {
            stable_piece_id,
            footprint_group_id: stable_piece_id,
            local_loop_index,
            is_hole: false,
            vertices,
            source_edges,
        }
    }

    pub(crate) fn new_with_source_edges_and_topology(
        stable_piece_id: u64,
        footprint_group_id: u64,
        local_loop_index: u32,
        is_hole: bool,
        vertices: Vec<TerrainCdtVertex>,
        source_edges: Vec<TerrainCdtRoadLoopSourceEdge>,
    ) -> Self {
        Self {
            stable_piece_id,
            footprint_group_id,
            local_loop_index,
            is_hole,
            vertices,
            source_edges,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainCdtRoadLoopSourceEdge {
    pub(crate) start: TerrainCdtVertex,
    pub(crate) end: TerrainCdtVertex,
    pub(crate) source: TerrainCdtRoadBoundarySource,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerrainCdtEdgeClass {
    Standard,
    Bridge,
    Tunnel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerrainCdtRoadBandKind {
    Carriageway,
    CurbOrShoulder,
    Sidewalk,
    Footpath,
    Median,
    Parking,
    CycleTrack,
    TramReservation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerrainCdtSpanRegionRole {
    Asphalt,
    CurbOrShoulder,
    NonRoad,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerrainCdtEarthworkSupportPolicy {
    StandardFullGroundedSpan,
    BridgeEndpointAbutments,
    TunnelVisiblePortals,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerrainCdtNodePieceKind {
    Terminal,
    Bend,
    JunctionN,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct TerrainCdtNodeFootprintBoundaryDirectSource {
    pub(crate) top_surface_source_index: u64,
    pub(crate) grade_authority_index: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum TerrainCdtNodeFootprintBoundaryVertexSource {
    Direct(TerrainCdtNodeFootprintBoundaryDirectSource),
    CanonicalBoundaryPoint {
        x_key: i64,
        z_key: i64,
        y_mm: i64,
    },
    BoundaryInterpolation {
        owning_segment_start: TerrainCdtNodeFootprintBoundaryDirectSource,
        owning_segment_end: TerrainCdtNodeFootprintBoundaryDirectSource,
        height_mm: i64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct TerrainCdtNodeFootprintBoundarySegmentSource {
    pub(crate) start: TerrainCdtNodeFootprintBoundaryVertexSource,
    pub(crate) end: TerrainCdtNodeFootprintBoundaryVertexSource,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TerrainCdtRoadBoundarySource {
    SpanSupportBoundary {
        edge_idx: u64,
        edge_class: TerrainCdtEdgeClass,
        support_policy: TerrainCdtEarthworkSupportPolicy,
        source_band_index: u32,
        band_kind: TerrainCdtRoadBandKind,
        role: TerrainCdtSpanRegionRole,
        start_section_index: u32,
        end_section_index: u32,
        start_s_m: f32,
        end_s_m: f32,
    },
    NodeFootprintBoundary {
        node_id: u32,
        node_kind: TerrainCdtNodePieceKind,
        owner_kind: TerrainCdtRoadBandKind,
        owner_index: u32,
        boundary_source: Option<TerrainCdtNodeFootprintBoundarySegmentSource>,
    },
    SyntheticTestBoundary {
        stable_piece_id: u64,
        local_loop_index: u32,
        local_edge_index: u32,
    },
}

impl TerrainCdtRoadBoundarySource {
    pub(crate) fn source_kind_code(self) -> i32 {
        match self {
            Self::SpanSupportBoundary { .. } => 0,
            Self::NodeFootprintBoundary { .. } => 1,
            Self::SyntheticTestBoundary { .. } => 2,
        }
    }

    pub(crate) fn primary_id_code(self) -> i32 {
        match self {
            Self::SpanSupportBoundary { edge_idx, .. } => clamp_u64_to_i32(edge_idx),
            Self::NodeFootprintBoundary { node_id, .. } => clamp_u32_to_i32(node_id),
            Self::SyntheticTestBoundary {
                stable_piece_id, ..
            } => clamp_u64_to_i32(stable_piece_id),
        }
    }

    pub(crate) fn node_kind_code(self) -> i32 {
        match self {
            Self::NodeFootprintBoundary { node_kind, .. } => {
                i32::from(terrain_cdt_node_kind_sort_key(node_kind))
            }
            _ => -1,
        }
    }

    pub(crate) fn edge_class_code(self) -> i32 {
        match self {
            Self::SpanSupportBoundary { edge_class, .. } => {
                i32::from(terrain_cdt_edge_class_sort_key(edge_class))
            }
            _ => -1,
        }
    }

    pub(crate) fn support_policy_code(self) -> i32 {
        match self {
            Self::SpanSupportBoundary { support_policy, .. } => {
                i32::from(terrain_cdt_support_policy_sort_key(support_policy))
            }
            _ => -1,
        }
    }

    pub(crate) fn owner_kind_code(self) -> i32 {
        match self {
            Self::SpanSupportBoundary { band_kind, .. } => {
                i32::from(terrain_cdt_band_kind_sort_key(band_kind))
            }
            Self::NodeFootprintBoundary { owner_kind, .. } => {
                i32::from(terrain_cdt_band_kind_sort_key(owner_kind))
            }
            Self::SyntheticTestBoundary { .. } => -1,
        }
    }

    pub(crate) fn owner_index_code(self) -> i32 {
        match self {
            Self::SpanSupportBoundary {
                source_band_index, ..
            } => clamp_u32_to_i32(source_band_index),
            Self::NodeFootprintBoundary { owner_index, .. } => clamp_u32_to_i32(owner_index),
            Self::SyntheticTestBoundary { .. } => -1,
        }
    }

    pub(crate) fn role_code(self) -> i32 {
        match self {
            Self::SpanSupportBoundary { role, .. } => {
                i32::from(terrain_cdt_span_role_sort_key(role))
            }
            _ => -1,
        }
    }

    pub(crate) fn section_range_codes(self) -> [i32; 2] {
        match self {
            Self::SpanSupportBoundary {
                start_section_index,
                end_section_index,
                ..
            } => [
                clamp_u32_to_i32(start_section_index),
                clamp_u32_to_i32(end_section_index),
            ],
            _ => [-1, -1],
        }
    }

    pub(crate) fn s_range_values(self) -> [f32; 2] {
        match self {
            Self::SpanSupportBoundary {
                start_s_m, end_s_m, ..
            } => [start_s_m, end_s_m],
            _ => [-1.0, -1.0],
        }
    }

    pub(crate) fn debug_label(self) -> String {
        match self {
            Self::SpanSupportBoundary {
                edge_idx,
                edge_class,
                support_policy,
                source_band_index,
                band_kind,
                role,
                start_section_index,
                end_section_index,
                start_s_m,
                end_s_m,
            } => format!(
                "span edge={} class={} policy={} band={} kind={} role={} sections={}..{} s={:.3}..{:.3}",
                edge_idx,
                terrain_cdt_edge_class_label(edge_class),
                terrain_cdt_support_policy_label(support_policy),
                source_band_index,
                terrain_cdt_band_kind_label(band_kind),
                terrain_cdt_span_role_label(role),
                start_section_index,
                end_section_index,
                start_s_m,
                end_s_m
            ),
            Self::NodeFootprintBoundary {
                node_id,
                node_kind,
                owner_kind,
                owner_index,
                boundary_source,
            } => format!(
                "node id={} kind={} owner_kind={} owner_index={} boundary_source={:?}",
                node_id,
                terrain_cdt_node_kind_label(node_kind),
                terrain_cdt_band_kind_label(owner_kind),
                owner_index,
                boundary_source
            ),
            Self::SyntheticTestBoundary {
                stable_piece_id,
                local_loop_index,
                local_edge_index,
            } => format!(
                "synthetic_test piece={} loop={} edge={}",
                stable_piece_id, local_loop_index, local_edge_index
            ),
        }
    }
}

fn clamp_u64_to_i32(value: u64) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

fn clamp_u32_to_i32(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TerrainCdtInput {
    pub(crate) patch: TerrainCdtPatch,
    pub(crate) road_loops: Vec<TerrainCdtRoadLoop>,
    pub(crate) source_samples: Vec<TerrainCdtVertex>,
}

impl TerrainCdtInput {
    pub(crate) fn new(
        patch: TerrainCdtPatch,
        road_loops: Vec<TerrainCdtRoadLoop>,
        source_samples: Vec<TerrainCdtVertex>,
    ) -> Self {
        Self {
            patch,
            road_loops,
            source_samples,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TerrainCdtMesh {
    pub(crate) vertices: Vec<TerrainCdtVertex>,
    pub(crate) emitted_faces: Vec<TerrainCdtEmittedFace>,
    pub(crate) triangles: Vec<[usize; 3]>,
    pub(crate) terrain_triangle_sources: Vec<Vec<TerrainCdtRoadBoundarySource>>,
    pub(crate) retaining_wall_triangles: Vec<[usize; 3]>,
    pub(crate) retaining_wall_triangle_sources: Vec<Vec<TerrainCdtRoadBoundarySource>>,
    pub(crate) stats: TerrainCdtStats,
    pub(crate) invalid_constraint_samples: Vec<TerrainCdtInvalidConstraintSample>,
    pub(crate) road_seam_face_samples: Vec<TerrainCdtFaceSample>,
    pub(crate) retaining_wall_face_samples: Vec<TerrainCdtFaceSample>,
    pub(crate) tie_in_widened_samples: Vec<TerrainCdtTieInSample>,
    pub(crate) seam_quality_samples: Vec<TerrainCdtSeamQualitySample>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TerrainCdtEmittedFace {
    pub(crate) triangle: [usize; 3],
    pub(crate) kind: TerrainCdtTieInKind,
    pub(crate) sources: Vec<TerrainCdtRoadBoundarySource>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainCdtStats {
    pub(crate) input_vertices: usize,
    pub(crate) constraint_edges: usize,
    pub(crate) road_constraint_edges: usize,
    pub(crate) accepted_faces: usize,
    pub(crate) rejected_road_faces: usize,
    pub(crate) preserved_road_constraint_edges: usize,
    pub(crate) invalid_constraint_edges: usize,
    pub(crate) max_face_y_delta_m: f32,
    pub(crate) max_face_slope_ratio: f32,
    pub(crate) road_seam_faces: usize,
    pub(crate) road_seam_max_y_delta_m: f32,
    pub(crate) road_seam_max_slope_ratio: f32,
    pub(crate) retaining_wall_faces: usize,
    pub(crate) retaining_wall_max_y_delta_m: f32,
    pub(crate) retaining_wall_max_slope_ratio: f32,
    pub(crate) accepted_seam_edges: usize,
    pub(crate) merged_subbudget_seam_edges: usize,
    pub(crate) omitted_near_seam_source_samples: usize,
    pub(crate) retaining_wall_required_seam_edges: usize,
    pub(crate) retaining_wall_required_seam_faces: usize,
    pub(crate) blocking_degenerate_seam_edges: usize,
    pub(crate) tie_in_widened_source_samples: usize,
    pub(crate) tie_in_widened_max_y_delta_m: f32,
    pub(crate) tie_in_widened_max_slope_ratio: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerrainCdtTieInKind {
    OrdinaryTerrain,
    RetainingWall,
}

impl TerrainCdtTieInKind {
    pub(crate) fn debug_code(self) -> i32 {
        match self {
            Self::OrdinaryTerrain => 0,
            Self::RetainingWall => 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TerrainCdtFaceSample {
    pub(crate) kind: TerrainCdtTieInKind,
    pub(crate) vertices: [TerrainCdtVertex; 3],
    pub(crate) centroid: TerrainCdtVertex,
    pub(crate) sources: Vec<TerrainCdtRoadBoundarySource>,
    pub(crate) min_x: f64,
    pub(crate) min_z: f64,
    pub(crate) max_x: f64,
    pub(crate) max_z: f64,
    pub(crate) min_y_m: f32,
    pub(crate) max_y_m: f32,
    pub(crate) max_y_delta_m: f32,
    pub(crate) max_slope_ratio: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainCdtInvalidConstraintSample {
    pub(crate) start: TerrainCdtVertex,
    pub(crate) end: TerrainCdtVertex,
    pub(crate) road_owned: bool,
    pub(crate) stable_piece_id: u64,
    pub(crate) local_loop_index: u32,
    pub(crate) local_edge_index: u32,
    pub(crate) source: Option<TerrainCdtRoadBoundarySource>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainCdtTieInSample {
    pub(crate) source_sample: TerrainCdtVertex,
    pub(crate) seam_point: TerrainCdtVertex,
    pub(crate) seam_source: TerrainCdtRoadBoundarySource,
    pub(crate) distance_m: f32,
    pub(crate) required_distance_m: f32,
    pub(crate) height_delta_m: f32,
    pub(crate) slope_ratio: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerrainCdtSeamQualityKind {
    MergedSubbudgetSeamEdge,
    RetainingWallRequired,
    BlockingDegenerateSeam,
}

impl TerrainCdtSeamQualityKind {
    pub(crate) fn debug_code(self) -> i32 {
        match self {
            Self::MergedSubbudgetSeamEdge => 0,
            Self::RetainingWallRequired => 1,
            Self::BlockingDegenerateSeam => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainCdtSeamQualitySample {
    pub(crate) kind: TerrainCdtSeamQualityKind,
    pub(crate) start: TerrainCdtVertex,
    pub(crate) end: TerrainCdtVertex,
    pub(crate) source: TerrainCdtRoadBoundarySource,
    pub(crate) length_m: f32,
    pub(crate) height_delta_m: f32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TerrainCdtError {
    InvalidPatch,
    MissingRoadBoundarySource,
    TriangulationFailed,
}

pub(crate) fn build_road_touched_terrain_patch(
    input: TerrainCdtInput,
) -> Result<TerrainCdtMesh, TerrainCdtError> {
    if !input.patch.is_valid() {
        return Err(TerrainCdtError::InvalidPatch);
    }

    let canonical = canonicalize_input(input)?;
    let spade_vertices = canonical
        .vertices
        .iter()
        .map(|vertex| vertex.point2())
        .collect::<Vec<_>>();
    let mut invalid_constraint_edges = 0usize;
    let mut invalid_constraint_samples = Vec::new();
    let cdt = SpadeCdt::try_bulk_load_cdt(spade_vertices, canonical.constraints.clone(), |edge| {
        invalid_constraint_edges += 1;
        insert_invalid_constraint_sample(
            &mut invalid_constraint_samples,
            normalize_edge_array(edge[0], edge[1]),
            &canonical.vertices,
            &canonical.road_constraint_sources,
        );
    })
    .map_err(|_| TerrainCdtError::TriangulationFailed)?;

    let mut triangles = Vec::new();
    let mut rejected_road_faces = 0usize;
    for face in cdt.inner_faces() {
        let [a, b, c] = face.vertices();
        let triangle = [a.fix().index(), b.fix().index(), c.fix().index()];
        let center = centroid([
            canonical.vertices[triangle[0]],
            canonical.vertices[triangle[1]],
            canonical.vertices[triangle[2]],
        ]);
        if point_inside_any_road_footprint(center, &canonical.road_loops) {
            rejected_road_faces += 1;
            continue;
        }
        triangles.push(triangle);
    }

    let accepted_edges = emitted_triangle_edges(&triangles);
    let preserved_road_constraint_edges = canonical
        .road_constraint_edges
        .iter()
        .filter(|edge| accepted_edges.contains(&normalize_edge(edge[0], edge[1])))
        .count();
    let diagnostics = terrain_face_diagnostics(
        &canonical.vertices,
        &triangles,
        &canonical.road_constraint_sources,
        &canonical.retaining_wall_required_sources,
    );
    let mut terrain_triangles = Vec::new();
    let mut terrain_triangle_sources = Vec::new();
    let mut retaining_wall_triangles = Vec::new();
    let mut retaining_wall_triangle_sources = Vec::new();
    for face in &diagnostics.emitted_faces {
        match face.kind {
            TerrainCdtTieInKind::OrdinaryTerrain => {
                terrain_triangles.push(face.triangle);
                terrain_triangle_sources.push(face.sources.clone());
            }
            TerrainCdtTieInKind::RetainingWall => {
                retaining_wall_triangles.push(face.triangle);
                retaining_wall_triangle_sources.push(face.sources.clone());
            }
        }
    }

    Ok(TerrainCdtMesh {
        stats: TerrainCdtStats {
            input_vertices: canonical.vertices.len(),
            constraint_edges: canonical.constraints.len(),
            road_constraint_edges: canonical.road_constraint_edges.len(),
            accepted_faces: triangles.len(),
            rejected_road_faces,
            preserved_road_constraint_edges,
            invalid_constraint_edges,
            max_face_y_delta_m: diagnostics.max_face_y_delta_m,
            max_face_slope_ratio: diagnostics.max_face_slope_ratio,
            road_seam_faces: diagnostics.road_seam_faces,
            road_seam_max_y_delta_m: diagnostics.road_seam_max_y_delta_m,
            road_seam_max_slope_ratio: diagnostics.road_seam_max_slope_ratio,
            retaining_wall_faces: diagnostics.retaining_wall_faces,
            retaining_wall_max_y_delta_m: diagnostics.retaining_wall_max_y_delta_m,
            retaining_wall_max_slope_ratio: diagnostics.retaining_wall_max_slope_ratio,
            accepted_seam_edges: canonical.accepted_seam_edges,
            merged_subbudget_seam_edges: canonical.merged_subbudget_seam_edges,
            omitted_near_seam_source_samples: canonical.tie_in_widened_source_samples,
            retaining_wall_required_seam_edges: canonical.retaining_wall_required_seam_edges,
            retaining_wall_required_seam_faces: diagnostics.retaining_wall_faces,
            blocking_degenerate_seam_edges: canonical.blocking_degenerate_seam_edges,
            tie_in_widened_source_samples: canonical.tie_in_widened_source_samples,
            tie_in_widened_max_y_delta_m: canonical.tie_in_widened_max_y_delta_m,
            tie_in_widened_max_slope_ratio: canonical.tie_in_widened_max_slope_ratio,
        },
        vertices: canonical.vertices,
        emitted_faces: diagnostics.emitted_faces,
        triangles: terrain_triangles,
        terrain_triangle_sources,
        retaining_wall_triangles,
        retaining_wall_triangle_sources,
        invalid_constraint_samples,
        road_seam_face_samples: diagnostics.road_seam_face_samples,
        retaining_wall_face_samples: diagnostics.retaining_wall_face_samples,
        tie_in_widened_samples: canonical.tie_in_widened_samples,
        seam_quality_samples: canonical.seam_quality_samples,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TerrainCdtRoadConstraintSource {
    stable_piece_id: u64,
    local_loop_index: u32,
    local_edge_index: u32,
    boundary_source: TerrainCdtRoadBoundarySource,
}

struct TerrainCdtDiagnostics {
    emitted_faces: Vec<TerrainCdtEmittedFace>,
    max_face_y_delta_m: f32,
    max_face_slope_ratio: f32,
    road_seam_faces: usize,
    road_seam_max_y_delta_m: f32,
    road_seam_max_slope_ratio: f32,
    retaining_wall_faces: usize,
    retaining_wall_max_y_delta_m: f32,
    retaining_wall_max_slope_ratio: f32,
    road_seam_face_samples: Vec<TerrainCdtFaceSample>,
    retaining_wall_face_samples: Vec<TerrainCdtFaceSample>,
}

struct CanonicalTerrainCdtInput {
    vertices: Vec<TerrainCdtVertex>,
    constraints: Vec<[usize; 2]>,
    road_constraint_edges: Vec<[usize; 2]>,
    road_constraint_sources: BTreeMap<[usize; 2], TerrainCdtRoadConstraintSource>,
    road_loops: Vec<CanonicalTerrainCdtRoadLoop>,
    accepted_seam_edges: usize,
    merged_subbudget_seam_edges: usize,
    retaining_wall_required_seam_edges: usize,
    retaining_wall_required_sources: Vec<TerrainCdtRoadBoundarySource>,
    blocking_degenerate_seam_edges: usize,
    seam_quality_samples: Vec<TerrainCdtSeamQualitySample>,
    tie_in_widened_source_samples: usize,
    tie_in_widened_max_y_delta_m: f32,
    tie_in_widened_max_slope_ratio: f32,
    tie_in_widened_samples: Vec<TerrainCdtTieInSample>,
}

#[derive(Clone, Debug, PartialEq)]
struct CanonicalTerrainCdtRoadLoop {
    footprint_group_id: u64,
    is_hole: bool,
    vertices: Vec<TerrainCdtVertex>,
    edge_sources: Vec<Option<TerrainCdtRoadBoundarySource>>,
}

#[derive(Clone, Debug, PartialEq)]
struct TerrainCdtLoopSeamQuality {
    points: Vec<TerrainCdtVertex>,
    edge_sources: Vec<Option<TerrainCdtRoadBoundarySource>>,
    accepted_seam_edges: usize,
    merged_subbudget_seam_edges: usize,
    retaining_wall_required_seam_edges: usize,
    blocking_degenerate_seam_edges: usize,
    samples: Vec<TerrainCdtSeamQualitySample>,
}

fn canonicalize_input(
    mut input: TerrainCdtInput,
) -> Result<CanonicalTerrainCdtInput, TerrainCdtError> {
    let mut vertices = Vec::new();
    let mut vertex_lookup = BTreeMap::new();
    let mut constraint_set = BTreeSet::new();
    let mut road_constraint_edges = Vec::new();
    let mut road_constraint_sources = BTreeMap::new();
    let mut road_loops = Vec::new();
    let mut source_sample_vertex_indices = Vec::new();
    let mut accepted_seam_edges = 0usize;
    let mut merged_subbudget_seam_edges = 0usize;
    let mut retaining_wall_required_seam_edges = 0usize;
    let mut retaining_wall_required_sources = Vec::new();
    let mut blocking_degenerate_seam_edges = 0usize;
    let mut seam_quality_samples = Vec::new();
    let mut tie_in_widened_source_samples = 0usize;
    let mut tie_in_widened_max_y_delta_m = 0.0_f32;
    let mut tie_in_widened_max_slope_ratio = 0.0_f32;
    let mut tie_in_widened_samples = Vec::new();

    let patch_corners = input.patch.corners_cw();
    for &vertex in &patch_corners {
        insert_vertex(vertex, &mut vertices, &mut vertex_lookup);
    }

    input.road_loops.sort_by_key(|road_loop| {
        (
            road_loop.footprint_group_id,
            road_loop.is_hole,
            road_loop.stable_piece_id,
            road_loop.local_loop_index,
        )
    });
    for road_loop in input.road_loops {
        let original_source_edges = normalized_road_loop_source_edges(&road_loop);
        let original_points = simplified_loop(road_loop.vertices);
        if original_points.len() < 3
            || signed_area(&original_points).abs() <= CDT_EPSILON_M * CDT_EPSILON_M
        {
            continue;
        }
        let points = simplified_loop(clip_loop_to_patch(original_points, input.patch));
        if points.len() < 3 {
            continue;
        }
        if signed_area(&points).abs() <= CDT_EPSILON_M * CDT_EPSILON_M {
            continue;
        }
        let points = ensure_ccw(points);
        let points = split_road_loop_segments_at_source_vertices(points, &original_source_edges);
        if points.len() < 3 {
            continue;
        }
        if signed_area(&points).abs() <= CDT_EPSILON_M * CDT_EPSILON_M {
            continue;
        }
        let seam_quality =
            harden_terrain_cdt_road_loop_seams(points, &original_source_edges, input.patch);
        let points = seam_quality.points;
        let edge_sources = seam_quality.edge_sources;
        accepted_seam_edges += seam_quality.accepted_seam_edges;
        merged_subbudget_seam_edges += seam_quality.merged_subbudget_seam_edges;
        retaining_wall_required_seam_edges += seam_quality.retaining_wall_required_seam_edges;
        blocking_degenerate_seam_edges += seam_quality.blocking_degenerate_seam_edges;
        append_seam_quality_samples(&mut seam_quality_samples, seam_quality.samples);
        if points.len() < 3 || signed_area(&points).abs() <= CDT_EPSILON_M * CDT_EPSILON_M {
            continue;
        }
        let loop_indices = points
            .iter()
            .map(|&vertex| insert_vertex(vertex, &mut vertices, &mut vertex_lookup))
            .collect::<Vec<_>>();
        let missing_road_boundary_sources = push_road_loop_constraints(
            &loop_indices,
            &vertices,
            input.patch,
            road_loop.stable_piece_id,
            road_loop.local_loop_index,
            &edge_sources,
            &mut road_constraint_edges,
            &mut road_constraint_sources,
        );
        if missing_road_boundary_sources > 0 {
            return Err(TerrainCdtError::MissingRoadBoundarySource);
        }
        road_loops.push(CanonicalTerrainCdtRoadLoop {
            footprint_group_id: road_loop.footprint_group_id,
            is_hole: road_loop.is_hole,
            vertices: points,
            edge_sources,
        });
    }

    input.source_samples.sort_by_key(|sample| {
        (
            quantized_coord(sample.x),
            quantized_coord(sample.z),
            quantized_coord(f64::from(sample.height_m)),
        )
    });
    for sample in input.source_samples {
        if !patch_contains(sample, input.patch) {
            continue;
        }
        if point_inside_any_road_footprint(sample, &road_loops) {
            continue;
        }
        if let Some(tie_in_sample) =
            widening_tie_in_sample_against_any_road_loop(sample, &road_loops)
        {
            tie_in_widened_source_samples += 1;
            tie_in_widened_max_y_delta_m =
                tie_in_widened_max_y_delta_m.max(tie_in_sample.height_delta_m);
            tie_in_widened_max_slope_ratio =
                tie_in_widened_max_slope_ratio.max(tie_in_sample.slope_ratio);
            if tie_in_sample.height_delta_m >= MIN_RETAINING_WALL_TIE_IN_HEIGHT_DELTA_M {
                retaining_wall_required_sources.push(tie_in_sample.seam_source);
            }
            insert_tie_in_widened_sample(&mut tie_in_widened_samples, tie_in_sample);
            continue;
        }
        let previous_vertex_count = vertices.len();
        let vertex_index = insert_vertex(sample, &mut vertices, &mut vertex_lookup);
        if vertices.len() > previous_vertex_count {
            source_sample_vertex_indices.push(vertex_index);
        }
    }

    node_road_constraint_edges(
        &mut vertices,
        &mut vertex_lookup,
        input.patch,
        &source_sample_vertex_indices,
        &mut road_constraint_edges,
        &mut road_constraint_sources,
    );
    push_patch_boundary_constraints(input.patch, &vertices, &mut constraint_set);
    for edge in &road_constraint_edges {
        insert_constraint(*edge, &mut constraint_set);
    }
    sort_dedup_terrain_cdt_boundary_sources(&mut retaining_wall_required_sources);

    Ok(CanonicalTerrainCdtInput {
        vertices,
        constraints: constraint_set.into_iter().collect(),
        road_constraint_edges,
        road_constraint_sources,
        road_loops,
        accepted_seam_edges,
        merged_subbudget_seam_edges,
        retaining_wall_required_seam_edges,
        retaining_wall_required_sources,
        blocking_degenerate_seam_edges,
        seam_quality_samples,
        tie_in_widened_source_samples,
        tie_in_widened_max_y_delta_m,
        tie_in_widened_max_slope_ratio,
        tie_in_widened_samples,
    })
}

fn insert_vertex(
    vertex: TerrainCdtVertex,
    vertices: &mut Vec<TerrainCdtVertex>,
    vertex_lookup: &mut BTreeMap<(i64, i64), usize>,
) -> usize {
    let key = (quantized_coord(vertex.x), quantized_coord(vertex.z));
    if let Some(index) = vertex_lookup.get(&key) {
        return *index;
    }
    let index = vertices.len();
    vertices.push(vertex);
    vertex_lookup.insert(key, index);
    index
}

fn push_road_loop_constraints(
    indices: &[usize],
    vertices: &[TerrainCdtVertex],
    patch: TerrainCdtPatch,
    stable_piece_id: u64,
    local_loop_index: u32,
    edge_sources: &[Option<TerrainCdtRoadBoundarySource>],
    road_constraint_edges: &mut Vec<[usize; 2]>,
    road_constraint_sources: &mut BTreeMap<[usize; 2], TerrainCdtRoadConstraintSource>,
) -> usize {
    let mut missing_road_boundary_sources = 0usize;
    for index in 0..indices.len() {
        let edge = normalize_edge_array(indices[index], indices[(index + 1) % indices.len()]);
        if edge[0] == edge[1] {
            continue;
        }
        if !edge_lies_on_patch_boundary(vertices[edge[0]], vertices[edge[1]], patch) {
            let Some(boundary_source) = edge_sources.get(index).copied().flatten() else {
                missing_road_boundary_sources += 1;
                continue;
            };
            road_constraint_edges.push(edge);
            road_constraint_sources
                .entry(edge)
                .or_insert(TerrainCdtRoadConstraintSource {
                    stable_piece_id,
                    local_loop_index,
                    local_edge_index: u32::try_from(index).unwrap_or(u32::MAX),
                    boundary_source,
                });
        }
    }
    missing_road_boundary_sources
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TerrainCdtSourceVertexSplit {
    t: f64,
    vertex: TerrainCdtVertex,
}

fn split_road_loop_segments_at_source_vertices(
    points: Vec<TerrainCdtVertex>,
    source_edges: &[TerrainCdtRoadLoopSourceEdge],
) -> Vec<TerrainCdtVertex> {
    if points.len() < 2 || source_edges.is_empty() {
        return points;
    }

    let mut split_points = Vec::with_capacity(points.len() + source_edges.len());
    for index in 0..points.len() {
        let start = points[index];
        let end = points[(index + 1) % points.len()];
        if split_points
            .last()
            .is_none_or(|last: &TerrainCdtVertex| !same_xz(*last, start))
        {
            split_points.push(start);
        }

        let mut splits = source_edges
            .iter()
            .flat_map(|edge| [edge.start, edge.end])
            .filter(|candidate| !same_xz(*candidate, start) && !same_xz(*candidate, end))
            .filter_map(|candidate| {
                source_sample_parameter_on_road_constraint(start, end, candidate).and_then(|t| {
                    (t > CDT_EPSILON_M && t < 1.0 - CDT_EPSILON_M).then_some(
                        TerrainCdtSourceVertexSplit {
                            t,
                            vertex: candidate,
                        },
                    )
                })
            })
            .collect::<Vec<_>>();
        sort_dedup_source_vertex_splits(&mut splits);
        for split in splits {
            if split_points
                .last()
                .is_some_and(|last: &TerrainCdtVertex| same_xz(*last, split.vertex))
            {
                continue;
            }
            split_points.push(split.vertex);
        }
    }

    simplified_loop(split_points)
}

fn sort_dedup_source_vertex_splits(splits: &mut Vec<TerrainCdtSourceVertexSplit>) {
    splits.sort_by(|a, b| {
        a.t.total_cmp(&b.t)
            .then_with(|| quantized_coord(a.vertex.x).cmp(&quantized_coord(b.vertex.x)))
            .then_with(|| quantized_coord(a.vertex.z).cmp(&quantized_coord(b.vertex.z)))
            .then_with(|| {
                quantized_coord(f64::from(a.vertex.height_m))
                    .cmp(&quantized_coord(f64::from(b.vertex.height_m)))
            })
    });

    let mut deduped = Vec::with_capacity(splits.len());
    for split in splits.iter().copied() {
        if let Some(last) = deduped.last_mut() {
            let last: &mut TerrainCdtSourceVertexSplit = last;
            if same_xz(split.vertex, last.vertex) {
                if split.vertex.height_m > last.vertex.height_m {
                    last.vertex.height_m = split.vertex.height_m;
                }
                continue;
            }
        }
        deduped.push(split);
    }
    *splits = deduped;
}

fn harden_terrain_cdt_road_loop_seams(
    mut points: Vec<TerrainCdtVertex>,
    source_edges: &[TerrainCdtRoadLoopSourceEdge],
    patch: TerrainCdtPatch,
) -> TerrainCdtLoopSeamQuality {
    let mut edge_sources = terrain_cdt_loop_edge_sources(&points, source_edges);
    let mut merged_subbudget_seam_edges = 0usize;
    let mut samples = Vec::new();

    loop {
        if points.len() < 3 || edge_sources.len() != points.len() {
            break;
        }
        let Some(merge) = next_mergeable_subbudget_seam_vertex(&points, &edge_sources) else {
            break;
        };
        let start = points[merge.previous_index];
        let end = points[merge.next_index];
        let length_m = edge_length_xz_m(start, end) as f32;
        let height_delta_m = (end.height_m - start.height_m).abs();
        insert_seam_quality_sample(
            &mut samples,
            TerrainCdtSeamQualitySample {
                kind: TerrainCdtSeamQualityKind::MergedSubbudgetSeamEdge,
                start,
                end,
                source: merge.source,
                length_m,
                height_delta_m,
            },
        );
        remove_loop_vertex_and_merge_source(
            &mut points,
            &mut edge_sources,
            merge.vertex_index,
            merge.source,
        );
        merged_subbudget_seam_edges += 1;
    }

    let mut accepted_seam_edges = 0usize;
    let mut retaining_wall_required_seam_edges = 0usize;
    let mut blocking_degenerate_seam_edges = 0usize;
    for index in 0..points.len() {
        let Some(source) = edge_sources.get(index).copied().flatten() else {
            continue;
        };
        let start = points[index];
        let end = points[(index + 1) % points.len()];
        if edge_lies_on_patch_boundary(start, end, patch) {
            continue;
        }
        let length_m = edge_length_xz_m(start, end);
        let height_delta_m = (end.height_m - start.height_m).abs();
        if length_m + CDT_EPSILON_M < MIN_SOURCE_OWNED_SEAM_EDGE_LENGTH_M {
            if height_delta_m > MIN_TIE_IN_HEIGHT_DELTA_M
                && (length_m <= CDT_EPSILON_M
                    || height_delta_m / length_m as f32 > MAX_TERRAIN_TIE_IN_SLOPE_RATIO)
            {
                retaining_wall_required_seam_edges += 1;
                insert_seam_quality_sample(
                    &mut samples,
                    TerrainCdtSeamQualitySample {
                        kind: TerrainCdtSeamQualityKind::RetainingWallRequired,
                        start,
                        end,
                        source,
                        length_m: length_m as f32,
                        height_delta_m,
                    },
                );
            } else if length_m <= CDT_EPSILON_M {
                blocking_degenerate_seam_edges += 1;
                insert_seam_quality_sample(
                    &mut samples,
                    TerrainCdtSeamQualitySample {
                        kind: TerrainCdtSeamQualityKind::BlockingDegenerateSeam,
                        start,
                        end,
                        source,
                        length_m: length_m as f32,
                        height_delta_m,
                    },
                );
            } else {
                accepted_seam_edges += 1;
            }
        } else {
            accepted_seam_edges += 1;
        }
    }

    TerrainCdtLoopSeamQuality {
        points,
        edge_sources,
        accepted_seam_edges,
        merged_subbudget_seam_edges,
        retaining_wall_required_seam_edges,
        blocking_degenerate_seam_edges,
        samples,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TerrainCdtSeamMerge {
    previous_index: usize,
    vertex_index: usize,
    next_index: usize,
    source: TerrainCdtRoadBoundarySource,
}

fn next_mergeable_subbudget_seam_vertex(
    points: &[TerrainCdtVertex],
    edge_sources: &[Option<TerrainCdtRoadBoundarySource>],
) -> Option<TerrainCdtSeamMerge> {
    for vertex_index in 0..points.len() {
        let previous_index = if vertex_index == 0 {
            points.len() - 1
        } else {
            vertex_index - 1
        };
        let next_index = (vertex_index + 1) % points.len();
        let previous_len_m = edge_length_xz_m(points[previous_index], points[vertex_index]);
        let next_len_m = edge_length_xz_m(points[vertex_index], points[next_index]);
        if previous_len_m >= MIN_SOURCE_OWNED_SEAM_EDGE_LENGTH_M
            && next_len_m >= MIN_SOURCE_OWNED_SEAM_EDGE_LENGTH_M
        {
            continue;
        }
        let Some(previous_source) = edge_sources.get(previous_index).copied().flatten() else {
            continue;
        };
        let Some(next_source) = edge_sources.get(vertex_index).copied().flatten() else {
            continue;
        };
        let Some(source) = mergeable_terrain_cdt_seam_source(previous_source, next_source) else {
            continue;
        };
        if !source_owned_vertex_can_be_removed(
            points[previous_index],
            points[vertex_index],
            points[next_index],
        ) {
            continue;
        }
        return Some(TerrainCdtSeamMerge {
            previous_index,
            vertex_index,
            next_index,
            source,
        });
    }
    None
}

fn remove_loop_vertex_and_merge_source(
    points: &mut Vec<TerrainCdtVertex>,
    edge_sources: &mut Vec<Option<TerrainCdtRoadBoundarySource>>,
    vertex_index: usize,
    source: TerrainCdtRoadBoundarySource,
) {
    if points.len() <= 3 || points.len() != edge_sources.len() {
        return;
    }
    points.remove(vertex_index);
    if vertex_index == 0 {
        let last = edge_sources.len() - 1;
        edge_sources[last] = Some(source);
        edge_sources.remove(0);
    } else {
        edge_sources[vertex_index - 1] = Some(source);
        edge_sources.remove(vertex_index);
    }
}

fn source_owned_vertex_can_be_removed(
    previous: TerrainCdtVertex,
    vertex: TerrainCdtVertex,
    next: TerrainCdtVertex,
) -> bool {
    let merged_len_m = edge_length_xz_m(previous, next);
    if merged_len_m <= CDT_EPSILON_M {
        return false;
    }
    let cross = cross_xz(
        vertex.x - previous.x,
        vertex.z - previous.z,
        next.x - previous.x,
        next.z - previous.z,
    )
    .abs();
    if cross > CDT_EPSILON_M * merged_len_m.max(1.0) {
        return false;
    }
    if !point_bounds_overlap_segment(vertex, previous, next) {
        return false;
    }
    let t = clamp_unit(segment_parameter(previous, next, vertex.x, vertex.z));
    same_height(
        interpolated_segment_height(previous, next, t),
        vertex.height_m,
    )
}

fn mergeable_terrain_cdt_seam_source(
    first: TerrainCdtRoadBoundarySource,
    second: TerrainCdtRoadBoundarySource,
) -> Option<TerrainCdtRoadBoundarySource> {
    match (first, second) {
        (
            TerrainCdtRoadBoundarySource::SpanSupportBoundary {
                edge_idx: edge_idx_a,
                edge_class: edge_class_a,
                support_policy: support_policy_a,
                source_band_index: source_band_index_a,
                band_kind: band_kind_a,
                role: role_a,
                start_section_index: start_section_index_a,
                end_section_index: end_section_index_a,
                start_s_m: start_s_m_a,
                end_s_m: end_s_m_a,
            },
            TerrainCdtRoadBoundarySource::SpanSupportBoundary {
                edge_idx: edge_idx_b,
                edge_class: edge_class_b,
                support_policy: support_policy_b,
                source_band_index: source_band_index_b,
                band_kind: band_kind_b,
                role: role_b,
                start_section_index: start_section_index_b,
                end_section_index: end_section_index_b,
                start_s_m: start_s_m_b,
                end_s_m: end_s_m_b,
            },
        ) if edge_idx_a == edge_idx_b
            && edge_class_a == edge_class_b
            && support_policy_a == support_policy_b
            && source_band_index_a == source_band_index_b
            && band_kind_a == band_kind_b
            && role_a == role_b =>
        {
            Some(TerrainCdtRoadBoundarySource::SpanSupportBoundary {
                edge_idx: edge_idx_a,
                edge_class: edge_class_a,
                support_policy: support_policy_a,
                source_band_index: source_band_index_a,
                band_kind: band_kind_a,
                role: role_a,
                start_section_index: start_section_index_a.min(start_section_index_b),
                end_section_index: end_section_index_a.max(end_section_index_b),
                start_s_m: start_s_m_a.min(start_s_m_b),
                end_s_m: end_s_m_a.max(end_s_m_b),
            })
        }
        (
            TerrainCdtRoadBoundarySource::NodeFootprintBoundary {
                node_id: node_id_a,
                node_kind: node_kind_a,
                owner_kind: owner_kind_a,
                owner_index: owner_index_a,
                boundary_source: boundary_source_a,
            },
            TerrainCdtRoadBoundarySource::NodeFootprintBoundary {
                node_id: node_id_b,
                node_kind: node_kind_b,
                owner_kind: owner_kind_b,
                owner_index: owner_index_b,
                boundary_source: boundary_source_b,
            },
        ) if node_id_a == node_id_b
            && node_kind_a == node_kind_b
            && owner_kind_a == owner_kind_b
            && owner_index_a == owner_index_b
            && boundary_source_a == boundary_source_b =>
        {
            Some(first)
        }
        (
            TerrainCdtRoadBoundarySource::SyntheticTestBoundary {
                stable_piece_id: stable_piece_id_a,
                local_loop_index: local_loop_index_a,
                local_edge_index: local_edge_index_a,
            },
            TerrainCdtRoadBoundarySource::SyntheticTestBoundary {
                stable_piece_id: stable_piece_id_b,
                local_loop_index: local_loop_index_b,
                local_edge_index: local_edge_index_b,
            },
        ) if stable_piece_id_a == stable_piece_id_b && local_loop_index_a == local_loop_index_b => {
            Some(TerrainCdtRoadBoundarySource::SyntheticTestBoundary {
                stable_piece_id: stable_piece_id_a,
                local_loop_index: local_loop_index_a,
                local_edge_index: local_edge_index_a.min(local_edge_index_b),
            })
        }
        _ => None,
    }
}

fn insert_seam_quality_sample(
    samples: &mut Vec<TerrainCdtSeamQualitySample>,
    sample: TerrainCdtSeamQualitySample,
) {
    if samples.len() >= MAX_SEAM_QUALITY_SAMPLES {
        return;
    }
    samples.push(sample);
}

fn append_seam_quality_samples(
    target: &mut Vec<TerrainCdtSeamQualitySample>,
    samples: Vec<TerrainCdtSeamQualitySample>,
) {
    for sample in samples {
        insert_seam_quality_sample(target, sample);
    }
}

fn normalized_road_loop_source_edges(
    road_loop: &TerrainCdtRoadLoop,
) -> Vec<TerrainCdtRoadLoopSourceEdge> {
    let mut source_edges = road_loop.source_edges.clone();
    if source_edges.is_empty() && !road_loop.vertices.is_empty() {
        source_edges = road_loop
            .vertices
            .iter()
            .copied()
            .enumerate()
            .map(|(index, start)| TerrainCdtRoadLoopSourceEdge {
                start,
                end: road_loop.vertices[(index + 1) % road_loop.vertices.len()],
                source: TerrainCdtRoadBoundarySource::SyntheticTestBoundary {
                    stable_piece_id: road_loop.stable_piece_id,
                    local_loop_index: road_loop.local_loop_index,
                    local_edge_index: u32::try_from(index).unwrap_or(u32::MAX),
                },
            })
            .collect();
    }
    source_edges
        .into_iter()
        .filter(|edge| !same_xz(edge.start, edge.end))
        .collect()
}

fn terrain_cdt_loop_edge_sources(
    points: &[TerrainCdtVertex],
    source_edges: &[TerrainCdtRoadLoopSourceEdge],
) -> Vec<Option<TerrainCdtRoadBoundarySource>> {
    if points.is_empty() {
        return Vec::new();
    }
    (0..points.len())
        .map(|index| {
            terrain_cdt_loop_segment_source(
                points[index],
                points[(index + 1) % points.len()],
                source_edges,
            )
        })
        .collect()
}

fn terrain_cdt_loop_segment_source(
    start: TerrainCdtVertex,
    end: TerrainCdtVertex,
    source_edges: &[TerrainCdtRoadLoopSourceEdge],
) -> Option<TerrainCdtRoadBoundarySource> {
    if same_xz(start, end) {
        return None;
    }
    let mut source = None;
    for &source_edge in source_edges {
        if !terrain_cdt_segment_lies_on_source_edge(start, end, source_edge) {
            continue;
        }
        merge_terrain_cdt_boundary_source(&mut source, source_edge.source);
    }
    source
}

fn terrain_cdt_segment_lies_on_source_edge(
    start: TerrainCdtVertex,
    end: TerrainCdtVertex,
    source_edge: TerrainCdtRoadLoopSourceEdge,
) -> bool {
    if !segment_bounds_overlap(start, end, source_edge.start, source_edge.end) {
        return false;
    }
    let Some(start_t) =
        source_sample_parameter_on_road_constraint(source_edge.start, source_edge.end, start)
    else {
        return false;
    };
    let Some(end_t) =
        source_sample_parameter_on_road_constraint(source_edge.start, source_edge.end, end)
    else {
        return false;
    };
    (start_t - end_t).abs() > CDT_EPSILON_M
}

fn merge_terrain_cdt_boundary_source(
    target: &mut Option<TerrainCdtRoadBoundarySource>,
    candidate: TerrainCdtRoadBoundarySource,
) {
    if target.is_none_or(|current| terrain_cdt_boundary_source_cmp(candidate, current).is_lt()) {
        *target = Some(candidate);
    }
}

fn push_patch_boundary_constraints(
    patch: TerrainCdtPatch,
    vertices: &[TerrainCdtVertex],
    constraint_set: &mut BTreeSet<[usize; 2]>,
) {
    let mut left = Vec::new();
    let mut top = Vec::new();
    let mut right = Vec::new();
    let mut bottom = Vec::new();

    for (index, vertex) in vertices.iter().copied().enumerate() {
        if same_coord(vertex.x, patch.min_x)
            && vertex.z >= patch.min_z - CDT_EPSILON_M
            && vertex.z <= patch.max_z + CDT_EPSILON_M
        {
            left.push((quantized_coord(vertex.z), index));
        }
        if same_coord(vertex.z, patch.max_z)
            && vertex.x >= patch.min_x - CDT_EPSILON_M
            && vertex.x <= patch.max_x + CDT_EPSILON_M
        {
            top.push((quantized_coord(vertex.x), index));
        }
        if same_coord(vertex.x, patch.max_x)
            && vertex.z >= patch.min_z - CDT_EPSILON_M
            && vertex.z <= patch.max_z + CDT_EPSILON_M
        {
            right.push((-quantized_coord(vertex.z), index));
        }
        if same_coord(vertex.z, patch.min_z)
            && vertex.x >= patch.min_x - CDT_EPSILON_M
            && vertex.x <= patch.max_x + CDT_EPSILON_M
        {
            bottom.push((-quantized_coord(vertex.x), index));
        }
    }

    push_sorted_boundary_side(&mut left, constraint_set);
    push_sorted_boundary_side(&mut top, constraint_set);
    push_sorted_boundary_side(&mut right, constraint_set);
    push_sorted_boundary_side(&mut bottom, constraint_set);
}

fn push_sorted_boundary_side(
    side: &mut Vec<(i64, usize)>,
    constraint_set: &mut BTreeSet<[usize; 2]>,
) {
    side.sort_unstable();
    side.dedup_by_key(|(_, index)| *index);
    for pair in side.windows(2) {
        insert_constraint([pair[0].1, pair[1].1], constraint_set);
    }
}

fn insert_constraint(edge: [usize; 2], constraint_set: &mut BTreeSet<[usize; 2]>) {
    let edge = normalize_edge_array(edge[0], edge[1]);
    if edge[0] != edge[1] {
        constraint_set.insert(edge);
    }
}

// Spade accepts a constrained graph but does not node crossing or T-touching
// constraints for us. i_overlay owns roadbed area union; this patch-local pass
// only canonicalizes the final CDT constraint graph. Determinism comes from
// sorted original road loops, quantized XZ vertex lookup, and BTreeSet edge
// emission. Complexity is O(E^2 + E*S) with bbox rejection over one dirty
// terrain patch's roadbed constraints and source samples, outside the per-tick
// simulation hot path.
#[derive(Clone, Copy, Debug, PartialEq)]
struct TerrainCdtRoadConstraintSplit {
    t: f64,
    vertex_index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TerrainCdtSourceSampleConstraintHit {
    edge_index: usize,
    t: f64,
    height_m: f32,
}

fn node_road_constraint_edges(
    vertices: &mut Vec<TerrainCdtVertex>,
    vertex_lookup: &mut BTreeMap<(i64, i64), usize>,
    patch: TerrainCdtPatch,
    source_sample_vertex_indices: &[usize],
    road_constraint_edges: &mut Vec<[usize; 2]>,
    road_constraint_sources: &mut BTreeMap<[usize; 2], TerrainCdtRoadConstraintSource>,
) {
    if road_constraint_edges.len() < 2 {
        return;
    }

    let original_edges = road_constraint_edges.clone();
    let mut split_points = original_edges
        .iter()
        .map(|edge| {
            vec![
                TerrainCdtRoadConstraintSplit {
                    t: 0.0,
                    vertex_index: edge[0],
                },
                TerrainCdtRoadConstraintSplit {
                    t: 1.0,
                    vertex_index: edge[1],
                },
            ]
        })
        .collect::<Vec<_>>();

    for first_index in 0..original_edges.len() {
        for second_index in first_index + 1..original_edges.len() {
            let first_edge = original_edges[first_index];
            let second_edge = original_edges[second_index];
            if first_edge == second_edge {
                continue;
            }
            let first_start = vertices[first_edge[0]];
            let first_end = vertices[first_edge[1]];
            let second_start = vertices[second_edge[0]];
            let second_end = vertices[second_edge[1]];
            if !segment_bounds_overlap(first_start, first_end, second_start, second_end) {
                continue;
            }

            for intersection in
                segment_intersections(first_start, first_end, second_start, second_end)
            {
                let first_t =
                    segment_parameter(first_start, first_end, intersection.x, intersection.z);
                let second_t =
                    segment_parameter(second_start, second_end, intersection.x, intersection.z);
                if !unit_interval_contains(first_t) || !unit_interval_contains(second_t) {
                    continue;
                }
                let first_height =
                    interpolated_segment_height(first_start, first_end, clamp_unit(first_t));
                let second_height =
                    interpolated_segment_height(second_start, second_end, clamp_unit(second_t));
                let Some(intersection_height) =
                    shared_road_constraint_height(first_height, second_height)
                else {
                    continue;
                };
                let Some(vertex_index) = insert_road_constraint_vertex(
                    TerrainCdtVertex::new(intersection.x, intersection_height, intersection.z),
                    vertices,
                    vertex_lookup,
                ) else {
                    continue;
                };
                split_points[first_index].push(TerrainCdtRoadConstraintSplit {
                    t: clamp_unit(first_t),
                    vertex_index,
                });
                split_points[second_index].push(TerrainCdtRoadConstraintSplit {
                    t: clamp_unit(second_t),
                    vertex_index,
                });
            }
        }
    }

    split_road_constraints_at_source_samples(
        &original_edges,
        vertices,
        source_sample_vertex_indices,
        &mut split_points,
    );

    let original_sources = road_constraint_sources.clone();
    let mut noded_edges = BTreeSet::new();
    road_constraint_sources.clear();

    for (edge, splits) in original_edges.iter().copied().zip(split_points.iter_mut()) {
        sort_dedup_constraint_splits(splits);
        let source = original_sources.get(&edge).copied();
        for pair in splits.windows(2) {
            let noded_edge = normalize_edge_array(pair[0].vertex_index, pair[1].vertex_index);
            if noded_edge[0] == noded_edge[1]
                || edge_lies_on_patch_boundary(
                    vertices[noded_edge[0]],
                    vertices[noded_edge[1]],
                    patch,
                )
            {
                continue;
            }
            noded_edges.insert(noded_edge);
            if let Some(source) = source {
                road_constraint_sources
                    .entry(noded_edge)
                    .and_modify(|existing| {
                        if terrain_cdt_road_constraint_source_cmp(source, *existing).is_lt() {
                            *existing = source;
                        }
                    })
                    .or_insert(source);
            }
        }
    }

    *road_constraint_edges = noded_edges.into_iter().collect();
}

fn split_road_constraints_at_source_samples(
    original_edges: &[[usize; 2]],
    vertices: &mut [TerrainCdtVertex],
    source_sample_vertex_indices: &[usize],
    split_points: &mut [Vec<TerrainCdtRoadConstraintSplit>],
) {
    for &vertex_index in source_sample_vertex_indices {
        let Some(vertex) = vertices.get(vertex_index).copied() else {
            continue;
        };
        let mut hits = Vec::new();
        for (edge_index, edge) in original_edges.iter().copied().enumerate() {
            if vertex_index == edge[0] || vertex_index == edge[1] {
                continue;
            }
            let start = vertices[edge[0]];
            let end = vertices[edge[1]];
            if !point_bounds_overlap_segment(vertex, start, end) {
                continue;
            }
            let Some(t) = source_sample_parameter_on_road_constraint(start, end, vertex) else {
                continue;
            };
            hits.push(TerrainCdtSourceSampleConstraintHit {
                edge_index,
                t,
                height_m: interpolated_segment_height(start, end, t),
            });
        }

        if hits.is_empty() {
            continue;
        }
        let height_m = hits[0].height_m;
        if !hits.iter().all(|hit| same_height(hit.height_m, height_m)) {
            continue;
        }
        vertices[vertex_index].height_m = height_m;
        for hit in hits {
            split_points[hit.edge_index].push(TerrainCdtRoadConstraintSplit {
                t: hit.t,
                vertex_index,
            });
        }
    }
}

fn source_sample_parameter_on_road_constraint(
    start: TerrainCdtVertex,
    end: TerrainCdtVertex,
    sample: TerrainCdtVertex,
) -> Option<f64> {
    if same_xz(sample, start) {
        return Some(0.0);
    }
    if same_xz(sample, end) {
        return Some(1.0);
    }
    let t = segment_parameter(start, end, sample.x, sample.z);
    if !unit_interval_contains(t) {
        return None;
    }
    let t = clamp_unit(t);
    let closest = interpolate_vertex(start, end, t);
    same_xz(closest, sample).then_some(t)
}

fn point_bounds_overlap_segment(
    point: TerrainCdtVertex,
    start: TerrainCdtVertex,
    end: TerrainCdtVertex,
) -> bool {
    point.x >= start.x.min(end.x) - CDT_EPSILON_M
        && point.x <= start.x.max(end.x) + CDT_EPSILON_M
        && point.z >= start.z.min(end.z) - CDT_EPSILON_M
        && point.z <= start.z.max(end.z) + CDT_EPSILON_M
}

fn insert_road_constraint_vertex(
    vertex: TerrainCdtVertex,
    vertices: &mut Vec<TerrainCdtVertex>,
    vertex_lookup: &mut BTreeMap<(i64, i64), usize>,
) -> Option<usize> {
    let key = (quantized_coord(vertex.x), quantized_coord(vertex.z));
    if let Some(index) = vertex_lookup.get(&key) {
        let Some(height_m) =
            shared_road_constraint_height(vertices[*index].height_m, vertex.height_m)
        else {
            return None;
        };
        vertices[*index].height_m = height_m;
        return Some(*index);
    }
    let index = vertices.len();
    vertices.push(vertex);
    vertex_lookup.insert(key, index);
    Some(index)
}

fn sort_dedup_constraint_splits(splits: &mut Vec<TerrainCdtRoadConstraintSplit>) {
    splits.sort_by(|a, b| {
        a.t.total_cmp(&b.t)
            .then_with(|| a.vertex_index.cmp(&b.vertex_index))
    });
    let mut deduped = Vec::with_capacity(splits.len());
    for split in splits.iter().copied() {
        if let Some(last) = deduped.last_mut() {
            let last: &mut TerrainCdtRoadConstraintSplit = last;
            if (split.t - last.t).abs() <= CDT_EPSILON_M || split.vertex_index == last.vertex_index
            {
                if split.vertex_index < last.vertex_index {
                    *last = split;
                }
                continue;
            }
        }
        deduped.push(split);
    }
    *splits = deduped;
}

fn segment_intersections(
    first_start: TerrainCdtVertex,
    first_end: TerrainCdtVertex,
    second_start: TerrainCdtVertex,
    second_end: TerrainCdtVertex,
) -> Vec<TerrainCdtVertex> {
    let first_dx = first_end.x - first_start.x;
    let first_dz = first_end.z - first_start.z;
    let second_dx = second_end.x - second_start.x;
    let second_dz = second_end.z - second_start.z;
    let first_len_sq = first_dx * first_dx + first_dz * first_dz;
    let second_len_sq = second_dx * second_dx + second_dz * second_dz;
    if first_len_sq <= CDT_EPSILON_M * CDT_EPSILON_M
        || second_len_sq <= CDT_EPSILON_M * CDT_EPSILON_M
    {
        return Vec::new();
    }

    let cross = cross_xz(first_dx, first_dz, second_dx, second_dz);
    let start_delta_x = second_start.x - first_start.x;
    let start_delta_z = second_start.z - first_start.z;
    if cross.abs() > CDT_EPSILON_M * first_len_sq.sqrt().max(second_len_sq.sqrt()) {
        let first_t = cross_xz(start_delta_x, start_delta_z, second_dx, second_dz) / cross;
        let second_t = cross_xz(start_delta_x, start_delta_z, first_dx, first_dz) / cross;
        if unit_interval_contains(first_t) && unit_interval_contains(second_t) {
            return vec![TerrainCdtVertex::new(
                first_start.x + first_dx * clamp_unit(first_t),
                0.0,
                first_start.z + first_dz * clamp_unit(first_t),
            )];
        }
        return Vec::new();
    }

    if cross_xz(start_delta_x, start_delta_z, first_dx, first_dz).abs()
        > CDT_EPSILON_M * first_len_sq.sqrt()
    {
        return Vec::new();
    }

    let first_t0 = segment_parameter(first_start, first_end, second_start.x, second_start.z);
    let first_t1 = segment_parameter(first_start, first_end, second_end.x, second_end.z);
    let overlap_start = first_t0.min(first_t1).max(0.0);
    let overlap_end = first_t0.max(first_t1).min(1.0);
    if overlap_start > overlap_end + CDT_EPSILON_M {
        return Vec::new();
    }

    let mut intersections = vec![TerrainCdtVertex::new(
        first_start.x + first_dx * clamp_unit(overlap_start),
        0.0,
        first_start.z + first_dz * clamp_unit(overlap_start),
    )];
    if (overlap_end - overlap_start).abs() > CDT_EPSILON_M {
        intersections.push(TerrainCdtVertex::new(
            first_start.x + first_dx * clamp_unit(overlap_end),
            0.0,
            first_start.z + first_dz * clamp_unit(overlap_end),
        ));
    }
    intersections
}

fn segment_bounds_overlap(
    first_start: TerrainCdtVertex,
    first_end: TerrainCdtVertex,
    second_start: TerrainCdtVertex,
    second_end: TerrainCdtVertex,
) -> bool {
    first_start.x.min(first_end.x) <= second_start.x.max(second_end.x) + CDT_EPSILON_M
        && second_start.x.min(second_end.x) <= first_start.x.max(first_end.x) + CDT_EPSILON_M
        && first_start.z.min(first_end.z) <= second_start.z.max(second_end.z) + CDT_EPSILON_M
        && second_start.z.min(second_end.z) <= first_start.z.max(first_end.z) + CDT_EPSILON_M
}

fn segment_parameter(start: TerrainCdtVertex, end: TerrainCdtVertex, x: f64, z: f64) -> f64 {
    let dx = end.x - start.x;
    let dz = end.z - start.z;
    let length_squared = dx * dx + dz * dz;
    if length_squared <= CDT_EPSILON_M * CDT_EPSILON_M {
        return 0.0;
    }
    ((x - start.x) * dx + (z - start.z) * dz) / length_squared
}

fn interpolated_segment_height(start: TerrainCdtVertex, end: TerrainCdtVertex, t: f64) -> f32 {
    (f64::from(start.height_m) + f64::from(end.height_m - start.height_m) * t) as f32
}

fn unit_interval_contains(value: f64) -> bool {
    value >= -CDT_EPSILON_M && value <= 1.0 + CDT_EPSILON_M
}

fn clamp_unit(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

fn cross_xz(ax: f64, az: f64, bx: f64, bz: f64) -> f64 {
    ax * bz - az * bx
}

fn edge_lies_on_patch_boundary(
    a: TerrainCdtVertex,
    b: TerrainCdtVertex,
    patch: TerrainCdtPatch,
) -> bool {
    (same_coord(a.x, patch.min_x) && same_coord(b.x, patch.min_x))
        || (same_coord(a.x, patch.max_x) && same_coord(b.x, patch.max_x))
        || (same_coord(a.z, patch.min_z) && same_coord(b.z, patch.min_z))
        || (same_coord(a.z, patch.max_z) && same_coord(b.z, patch.max_z))
}

fn edge_length_xz_m(a: TerrainCdtVertex, b: TerrainCdtVertex) -> f64 {
    let dx = b.x - a.x;
    let dz = b.z - a.z;
    (dx * dx + dz * dz).sqrt()
}

fn simplified_loop(points: Vec<TerrainCdtVertex>) -> Vec<TerrainCdtVertex> {
    let mut deduplicated = Vec::with_capacity(points.len());
    for point in points {
        if deduplicated
            .last()
            .is_some_and(|last: &TerrainCdtVertex| same_xz(*last, point))
        {
            continue;
        }
        deduplicated.push(point);
    }
    if deduplicated.len() > 1 && same_xz(deduplicated[0], *deduplicated.last().unwrap()) {
        deduplicated.pop();
    }
    deduplicated
}

fn clip_loop_to_patch(
    points: Vec<TerrainCdtVertex>,
    patch: TerrainCdtPatch,
) -> Vec<TerrainCdtVertex> {
    let points = clip_loop_to_boundary(
        points,
        |point| point.x >= patch.min_x - CDT_EPSILON_M,
        |a, b| intersect_at_x(a, b, patch.min_x),
    );
    let points = clip_loop_to_boundary(
        points,
        |point| point.x <= patch.max_x + CDT_EPSILON_M,
        |a, b| intersect_at_x(a, b, patch.max_x),
    );
    let points = clip_loop_to_boundary(
        points,
        |point| point.z >= patch.min_z - CDT_EPSILON_M,
        |a, b| intersect_at_z(a, b, patch.min_z),
    );
    let points = clip_loop_to_boundary(
        points,
        |point| point.z <= patch.max_z + CDT_EPSILON_M,
        |a, b| intersect_at_z(a, b, patch.max_z),
    );
    points
        .into_iter()
        .map(|point| clamp_to_patch(point, patch))
        .collect()
}

fn clip_loop_to_boundary(
    points: Vec<TerrainCdtVertex>,
    inside: impl Fn(TerrainCdtVertex) -> bool,
    intersection: impl Fn(TerrainCdtVertex, TerrainCdtVertex) -> TerrainCdtVertex,
) -> Vec<TerrainCdtVertex> {
    if points.is_empty() {
        return points;
    }

    let mut clipped = Vec::new();
    let mut previous = *points.last().unwrap();
    let mut previous_inside = inside(previous);
    for current in points {
        let current_inside = inside(current);
        if current_inside {
            if !previous_inside {
                clipped.push(intersection(previous, current));
            }
            clipped.push(current);
        } else if previous_inside {
            clipped.push(intersection(previous, current));
        }
        previous = current;
        previous_inside = current_inside;
    }
    clipped
}

fn intersect_at_x(a: TerrainCdtVertex, b: TerrainCdtVertex, x: f64) -> TerrainCdtVertex {
    let denominator = b.x - a.x;
    if denominator.abs() <= CDT_EPSILON_M {
        return TerrainCdtVertex::new(x, a.height_m, a.z);
    }
    interpolate_vertex(a, b, (x - a.x) / denominator)
}

fn intersect_at_z(a: TerrainCdtVertex, b: TerrainCdtVertex, z: f64) -> TerrainCdtVertex {
    let denominator = b.z - a.z;
    if denominator.abs() <= CDT_EPSILON_M {
        return TerrainCdtVertex::new(a.x, a.height_m, z);
    }
    interpolate_vertex(a, b, (z - a.z) / denominator)
}

fn interpolate_vertex(a: TerrainCdtVertex, b: TerrainCdtVertex, t: f64) -> TerrainCdtVertex {
    let t = t.clamp(0.0, 1.0);
    TerrainCdtVertex::new(
        a.x + (b.x - a.x) * t,
        (f64::from(a.height_m) + f64::from(b.height_m - a.height_m) * t) as f32,
        a.z + (b.z - a.z) * t,
    )
}

fn clamp_to_patch(vertex: TerrainCdtVertex, patch: TerrainCdtPatch) -> TerrainCdtVertex {
    TerrainCdtVertex::new(
        vertex.x.clamp(patch.min_x, patch.max_x),
        vertex.height_m,
        vertex.z.clamp(patch.min_z, patch.max_z),
    )
}

fn patch_contains(vertex: TerrainCdtVertex, patch: TerrainCdtPatch) -> bool {
    vertex.x >= patch.min_x - CDT_EPSILON_M
        && vertex.x <= patch.max_x + CDT_EPSILON_M
        && vertex.z >= patch.min_z - CDT_EPSILON_M
        && vertex.z <= patch.max_z + CDT_EPSILON_M
}

fn ensure_ccw(mut points: Vec<TerrainCdtVertex>) -> Vec<TerrainCdtVertex> {
    if signed_area(&points) < 0.0 {
        points.reverse();
    }
    points
}

fn same_xz(a: TerrainCdtVertex, b: TerrainCdtVertex) -> bool {
    quantized_coord(a.x) == quantized_coord(b.x) && quantized_coord(a.z) == quantized_coord(b.z)
}

fn same_coord(a: f64, b: f64) -> bool {
    quantized_coord(a) == quantized_coord(b)
}

fn same_height(a: f32, b: f32) -> bool {
    quantized_coord(f64::from(a)) == quantized_coord(f64::from(b))
}

fn shared_road_constraint_height(a: f32, b: f32) -> Option<f32> {
    same_height(a, b).then_some(a)
}

fn quantized_coord(value: f64) -> i64 {
    (value / CDT_EPSILON_M).round() as i64
}

fn signed_area(points: &[TerrainCdtVertex]) -> f64 {
    let mut area = 0.0;
    for index in 0..points.len() {
        let next = (index + 1) % points.len();
        area += points[index].x * points[next].z - points[next].x * points[index].z;
    }
    area * 0.5
}

fn point_in_polygon(point: TerrainCdtVertex, polygon: &[TerrainCdtVertex]) -> bool {
    let mut inside = false;
    let mut previous = polygon.len() - 1;
    for current in 0..polygon.len() {
        let a = polygon[current];
        let b = polygon[previous];
        if (a.z > point.z) != (b.z > point.z) {
            let intersection_x = (b.x - a.x) * (point.z - a.z) / (b.z - a.z) + a.x;
            if point.x < intersection_x {
                inside = !inside;
            }
        }
        previous = current;
    }
    inside
}

fn point_inside_any_road_footprint(
    point: TerrainCdtVertex,
    road_loops: &[CanonicalTerrainCdtRoadLoop],
) -> bool {
    road_loops
        .iter()
        .filter(|road_loop| !road_loop.is_hole)
        .filter(|road_loop| point_in_polygon(point, &road_loop.vertices))
        .any(|outer_loop| {
            !road_loops.iter().any(|hole_loop| {
                hole_loop.is_hole
                    && hole_loop.footprint_group_id == outer_loop.footprint_group_id
                    && point_in_polygon(point, &hole_loop.vertices)
            })
        })
}

fn centroid(points: [TerrainCdtVertex; 3]) -> TerrainCdtVertex {
    TerrainCdtVertex::new(
        (points[0].x + points[1].x + points[2].x) / 3.0,
        (points[0].height_m + points[1].height_m + points[2].height_m) / 3.0,
        (points[0].z + points[1].z + points[2].z) / 3.0,
    )
}

fn emitted_triangle_edges(triangles: &[[usize; 3]]) -> HashSet<(usize, usize)> {
    let mut edges = HashSet::new();
    for [a, b, c] in triangles {
        edges.insert(normalize_edge(*a, *b));
        edges.insert(normalize_edge(*b, *c));
        edges.insert(normalize_edge(*c, *a));
    }
    edges
}

fn terrain_face_diagnostics(
    vertices: &[TerrainCdtVertex],
    triangles: &[[usize; 3]],
    road_constraint_sources: &BTreeMap<[usize; 2], TerrainCdtRoadConstraintSource>,
    retaining_wall_required_sources: &[TerrainCdtRoadBoundarySource],
) -> TerrainCdtDiagnostics {
    let mut diagnostics = TerrainCdtDiagnostics {
        emitted_faces: Vec::new(),
        max_face_y_delta_m: 0.0,
        max_face_slope_ratio: 0.0,
        road_seam_faces: 0,
        road_seam_max_y_delta_m: 0.0,
        road_seam_max_slope_ratio: 0.0,
        retaining_wall_faces: 0,
        retaining_wall_max_y_delta_m: 0.0,
        retaining_wall_max_slope_ratio: 0.0,
        road_seam_face_samples: Vec::new(),
        retaining_wall_face_samples: Vec::new(),
    };

    for triangle in triangles {
        let points = [
            vertices[triangle[0]],
            vertices[triangle[1]],
            vertices[triangle[2]],
        ];
        let sources = terrain_triangle_road_sources(triangle, road_constraint_sources);
        let touches_road_seam = !sources.is_empty();
        let kind = classify_terrain_tie_in_face(points, &sources, retaining_wall_required_sources);
        let metrics = terrain_face_sample(points, kind, sources);
        diagnostics.emitted_faces.push(TerrainCdtEmittedFace {
            triangle: *triangle,
            kind,
            sources: metrics.sources.clone(),
        });
        diagnostics.max_face_y_delta_m = diagnostics.max_face_y_delta_m.max(metrics.max_y_delta_m);
        diagnostics.max_face_slope_ratio = diagnostics
            .max_face_slope_ratio
            .max(metrics.max_slope_ratio);

        match kind {
            TerrainCdtTieInKind::OrdinaryTerrain => {}
            TerrainCdtTieInKind::RetainingWall => {
                diagnostics.retaining_wall_faces += 1;
                diagnostics.retaining_wall_max_y_delta_m = diagnostics
                    .retaining_wall_max_y_delta_m
                    .max(metrics.max_y_delta_m);
                diagnostics.retaining_wall_max_slope_ratio = diagnostics
                    .retaining_wall_max_slope_ratio
                    .max(metrics.max_slope_ratio);
                insert_road_seam_face_sample(
                    &mut diagnostics.retaining_wall_face_samples,
                    metrics.clone(),
                );
            }
        }

        if !touches_road_seam {
            continue;
        }

        diagnostics.road_seam_faces += 1;
        diagnostics.road_seam_max_y_delta_m = diagnostics
            .road_seam_max_y_delta_m
            .max(metrics.max_y_delta_m);
        diagnostics.road_seam_max_slope_ratio = diagnostics
            .road_seam_max_slope_ratio
            .max(metrics.max_slope_ratio);
        insert_road_seam_face_sample(&mut diagnostics.road_seam_face_samples, metrics);
    }

    diagnostics
}

fn terrain_triangle_road_sources(
    triangle: &[usize; 3],
    road_constraint_sources: &BTreeMap<[usize; 2], TerrainCdtRoadConstraintSource>,
) -> Vec<TerrainCdtRoadBoundarySource> {
    let mut sources = triangle_edges(triangle)
        .iter()
        .filter_map(|edge| {
            road_constraint_sources
                .get(&[edge.0, edge.1])
                .map(|source| source.boundary_source)
        })
        .collect::<Vec<_>>();
    sort_dedup_terrain_cdt_boundary_sources(&mut sources);
    sources
}

fn classify_terrain_tie_in_face(
    points: [TerrainCdtVertex; 3],
    sources: &[TerrainCdtRoadBoundarySource],
    retaining_wall_required_sources: &[TerrainCdtRoadBoundarySource],
) -> TerrainCdtTieInKind {
    if sources.is_empty() {
        return TerrainCdtTieInKind::OrdinaryTerrain;
    }
    if terrain_sources_include_retaining_wall_required_source(
        sources,
        retaining_wall_required_sources,
    ) {
        return TerrainCdtTieInKind::RetainingWall;
    }
    let metrics = terrain_face_sample(points, TerrainCdtTieInKind::OrdinaryTerrain, Vec::new());
    if metrics.max_slope_ratio > MAX_TERRAIN_TIE_IN_SLOPE_RATIO {
        TerrainCdtTieInKind::RetainingWall
    } else {
        TerrainCdtTieInKind::OrdinaryTerrain
    }
}

fn terrain_sources_include_retaining_wall_required_source(
    sources: &[TerrainCdtRoadBoundarySource],
    retaining_wall_required_sources: &[TerrainCdtRoadBoundarySource],
) -> bool {
    sources.iter().copied().any(|source| {
        retaining_wall_required_sources
            .binary_search_by(|required_source| {
                terrain_cdt_boundary_source_cmp(*required_source, source)
            })
            .is_ok()
    })
}

fn widening_tie_in_sample_against_any_road_loop(
    sample: TerrainCdtVertex,
    road_loops: &[CanonicalTerrainCdtRoadLoop],
) -> Option<TerrainCdtTieInSample> {
    road_loops
        .iter()
        .filter_map(|road_loop| widening_tie_in_sample(sample, road_loop))
        .max_by(|a, b| {
            a.slope_ratio
                .total_cmp(&b.slope_ratio)
                .then_with(|| a.height_delta_m.total_cmp(&b.height_delta_m))
                .then_with(|| b.distance_m.total_cmp(&a.distance_m))
        })
}

fn widening_tie_in_sample(
    sample: TerrainCdtVertex,
    road_loop: &CanonicalTerrainCdtRoadLoop,
) -> Option<TerrainCdtTieInSample> {
    let (distance_m, seam_point, seam_source) =
        closest_sourced_loop_edge_distance_point_and_source(sample, road_loop)?;
    let height_delta_m = (sample.height_m - seam_point.height_m).abs();
    if height_delta_m <= MIN_TIE_IN_HEIGHT_DELTA_M {
        return None;
    }
    if distance_m <= CDT_EPSILON_M {
        return Some(TerrainCdtTieInSample {
            source_sample: sample,
            seam_point,
            seam_source,
            distance_m: 0.0,
            required_distance_m: height_delta_m / MAX_TERRAIN_TIE_IN_SLOPE_RATIO,
            height_delta_m,
            slope_ratio: f32::INFINITY,
        });
    }
    let distance_m_f32 = distance_m as f32;
    let slope_ratio = height_delta_m / distance_m_f32;
    let required_distance_m = height_delta_m / MAX_TERRAIN_TIE_IN_SLOPE_RATIO;
    (distance_m < f64::from(required_distance_m) - CDT_EPSILON_M).then_some(TerrainCdtTieInSample {
        source_sample: sample,
        seam_point,
        seam_source,
        distance_m: distance_m_f32,
        required_distance_m,
        height_delta_m,
        slope_ratio,
    })
}

fn closest_sourced_loop_edge_distance_point_and_source(
    point: TerrainCdtVertex,
    road_loop: &CanonicalTerrainCdtRoadLoop,
) -> Option<(f64, TerrainCdtVertex, TerrainCdtRoadBoundarySource)> {
    if road_loop.vertices.len() < 2 {
        return None;
    }

    let mut closest_distance_m = f64::INFINITY;
    let mut closest_point = TerrainCdtVertex::new(0.0, 0.0, 0.0);
    let mut closest_source = None;
    for index in 0..road_loop.vertices.len() {
        let Some(source) = road_loop.edge_sources.get(index).copied().flatten() else {
            continue;
        };
        let start = road_loop.vertices[index];
        let end = road_loop.vertices[(index + 1) % road_loop.vertices.len()];
        let segment_x = end.x - start.x;
        let segment_z = end.z - start.z;
        let segment_len_sq = segment_x * segment_x + segment_z * segment_z;
        let t = if segment_len_sq <= CDT_EPSILON_M * CDT_EPSILON_M {
            0.0
        } else {
            (((point.x - start.x) * segment_x + (point.z - start.z) * segment_z) / segment_len_sq)
                .clamp(0.0, 1.0)
        };
        let closest_x = start.x + segment_x * t;
        let closest_z = start.z + segment_z * t;
        let dx = point.x - closest_x;
        let dz = point.z - closest_z;
        let distance_m = (dx * dx + dz * dz).sqrt();
        let height_m =
            (f64::from(start.height_m) + f64::from(end.height_m - start.height_m) * t) as f32;
        let candidate_point = TerrainCdtVertex::new(closest_x, height_m, closest_z);
        if terrain_cdt_closer_loop_point(
            distance_m,
            candidate_point,
            source,
            closest_distance_m,
            closest_point,
            closest_source,
        ) {
            closest_distance_m = distance_m;
            closest_point = candidate_point;
            closest_source = Some(source);
        }
    }

    closest_distance_m
        .is_finite()
        .then_some((closest_distance_m, closest_point, closest_source?))
}

fn terrain_cdt_closer_loop_point(
    candidate_distance_m: f64,
    candidate_point: TerrainCdtVertex,
    candidate_source: TerrainCdtRoadBoundarySource,
    current_distance_m: f64,
    current_point: TerrainCdtVertex,
    current_source: Option<TerrainCdtRoadBoundarySource>,
) -> bool {
    if candidate_distance_m < current_distance_m - CDT_EPSILON_M {
        return true;
    }
    if (candidate_distance_m - current_distance_m).abs() > CDT_EPSILON_M {
        return false;
    }
    let geometry_ordering = terrain_cdt_vertex_geometry_cmp(candidate_point, current_point);
    if !geometry_ordering.is_eq() {
        return geometry_ordering.is_lt();
    }
    current_source
        .is_none_or(|source| terrain_cdt_boundary_source_cmp(candidate_source, source).is_lt())
}

fn terrain_cdt_vertex_geometry_cmp(a: TerrainCdtVertex, b: TerrainCdtVertex) -> std::cmp::Ordering {
    (
        quantized_coord(a.x),
        quantized_coord(a.z),
        quantized_coord(f64::from(a.height_m)),
    )
        .cmp(&(
            quantized_coord(b.x),
            quantized_coord(b.z),
            quantized_coord(f64::from(b.height_m)),
        ))
}

fn triangle_edges(triangle: &[usize; 3]) -> [(usize, usize); 3] {
    [
        normalize_edge(triangle[0], triangle[1]),
        normalize_edge(triangle[1], triangle[2]),
        normalize_edge(triangle[2], triangle[0]),
    ]
}

fn terrain_face_sample(
    points: [TerrainCdtVertex; 3],
    kind: TerrainCdtTieInKind,
    sources: Vec<TerrainCdtRoadBoundarySource>,
) -> TerrainCdtFaceSample {
    let mut min_x = points[0].x;
    let mut min_z = points[0].z;
    let mut max_x = points[0].x;
    let mut max_z = points[0].z;
    let mut min_y_m = points[0].height_m;
    let mut max_y_m = points[0].height_m;
    let mut max_y_delta_m = 0.0_f32;
    let max_slope_ratio = terrain_face_plane_slope_ratio(points);

    for point in points {
        min_x = min_x.min(point.x);
        min_z = min_z.min(point.z);
        max_x = max_x.max(point.x);
        max_z = max_z.max(point.z);
        min_y_m = min_y_m.min(point.height_m);
        max_y_m = max_y_m.max(point.height_m);
    }

    for edge_index in 0..3 {
        let start = points[edge_index];
        let end = points[(edge_index + 1) % 3];
        let y_delta_m = (end.height_m - start.height_m).abs();
        max_y_delta_m = max_y_delta_m.max(y_delta_m);
    }

    TerrainCdtFaceSample {
        kind,
        vertices: points,
        centroid: centroid(points),
        sources,
        min_x,
        min_z,
        max_x,
        max_z,
        min_y_m,
        max_y_m,
        max_y_delta_m,
        max_slope_ratio,
    }
}

fn terrain_face_plane_slope_ratio(points: [TerrainCdtVertex; 3]) -> f32 {
    let ax = points[1].x - points[0].x;
    let ay = f64::from(points[1].height_m - points[0].height_m);
    let az = points[1].z - points[0].z;
    let bx = points[2].x - points[0].x;
    let by = f64::from(points[2].height_m - points[0].height_m);
    let bz = points[2].z - points[0].z;

    let normal_x = ay * bz - az * by;
    let normal_y = az * bx - ax * bz;
    let normal_z = ax * by - ay * bx;
    let horizontal_normal = (normal_x * normal_x + normal_z * normal_z).sqrt();
    if horizontal_normal <= CDT_EPSILON_M * CDT_EPSILON_M {
        return 0.0;
    }
    if normal_y.abs() <= CDT_EPSILON_M * CDT_EPSILON_M {
        return 1_000_000.0;
    }
    (horizontal_normal / normal_y.abs()) as f32
}

fn insert_road_seam_face_sample(
    samples: &mut Vec<TerrainCdtFaceSample>,
    sample: TerrainCdtFaceSample,
) {
    samples.push(sample);
    samples.sort_by(|a, b| {
        b.max_slope_ratio
            .total_cmp(&a.max_slope_ratio)
            .then_with(|| b.max_y_delta_m.total_cmp(&a.max_y_delta_m))
            .then_with(|| terrain_cdt_boundary_sources_cmp(&a.sources, &b.sources))
            .then_with(|| a.centroid.x.total_cmp(&b.centroid.x))
            .then_with(|| a.centroid.z.total_cmp(&b.centroid.z))
    });
    samples.truncate(MAX_ROAD_SEAM_FACE_SAMPLES);
}

fn insert_tie_in_widened_sample(
    samples: &mut Vec<TerrainCdtTieInSample>,
    sample: TerrainCdtTieInSample,
) {
    samples.push(sample);
    samples.sort_by(|a, b| {
        b.slope_ratio
            .total_cmp(&a.slope_ratio)
            .then_with(|| b.height_delta_m.total_cmp(&a.height_delta_m))
            .then_with(|| terrain_cdt_boundary_source_cmp(a.seam_source, b.seam_source))
            .then_with(|| a.source_sample.x.total_cmp(&b.source_sample.x))
            .then_with(|| a.source_sample.z.total_cmp(&b.source_sample.z))
    });
    samples.truncate(MAX_TIE_IN_SAMPLE_DIAGNOSTICS);
}

fn insert_invalid_constraint_sample(
    samples: &mut Vec<TerrainCdtInvalidConstraintSample>,
    edge: [usize; 2],
    vertices: &[TerrainCdtVertex],
    road_constraint_sources: &BTreeMap<[usize; 2], TerrainCdtRoadConstraintSource>,
) {
    let Some(&start) = vertices.get(edge[0]) else {
        return;
    };
    let Some(&end) = vertices.get(edge[1]) else {
        return;
    };
    let source = road_constraint_sources.get(&edge);
    samples.push(TerrainCdtInvalidConstraintSample {
        start,
        end,
        road_owned: source.is_some(),
        stable_piece_id: source.map_or(0, |source| source.stable_piece_id),
        local_loop_index: source.map_or(u32::MAX, |source| source.local_loop_index),
        local_edge_index: source.map_or(u32::MAX, |source| source.local_edge_index),
        source: source.map(|source| source.boundary_source),
    });
    samples.sort_by(|a, b| {
        b.road_owned
            .cmp(&a.road_owned)
            .then_with(|| a.stable_piece_id.cmp(&b.stable_piece_id))
            .then_with(|| a.local_loop_index.cmp(&b.local_loop_index))
            .then_with(|| a.local_edge_index.cmp(&b.local_edge_index))
            .then_with(|| terrain_cdt_optional_boundary_source_cmp(a.source, b.source))
            .then_with(|| a.start.x.total_cmp(&b.start.x))
            .then_with(|| a.start.z.total_cmp(&b.start.z))
            .then_with(|| a.end.x.total_cmp(&b.end.x))
            .then_with(|| a.end.z.total_cmp(&b.end.z))
    });
    samples.truncate(MAX_INVALID_CONSTRAINT_SAMPLES);
}

fn sort_dedup_terrain_cdt_boundary_sources(sources: &mut Vec<TerrainCdtRoadBoundarySource>) {
    sources.sort_by(|a, b| terrain_cdt_boundary_source_cmp(*a, *b));
    sources.dedup_by(|a, b| terrain_cdt_boundary_source_cmp(*a, *b).is_eq());
}

fn terrain_cdt_road_constraint_source_cmp(
    a: TerrainCdtRoadConstraintSource,
    b: TerrainCdtRoadConstraintSource,
) -> std::cmp::Ordering {
    a.stable_piece_id
        .cmp(&b.stable_piece_id)
        .then(a.local_loop_index.cmp(&b.local_loop_index))
        .then(a.local_edge_index.cmp(&b.local_edge_index))
        .then_with(|| terrain_cdt_boundary_source_cmp(a.boundary_source, b.boundary_source))
}

fn terrain_cdt_optional_boundary_source_cmp(
    a: Option<TerrainCdtRoadBoundarySource>,
    b: Option<TerrainCdtRoadBoundarySource>,
) -> std::cmp::Ordering {
    match (a, b) {
        (Some(a), Some(b)) => terrain_cdt_boundary_source_cmp(a, b),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

fn terrain_cdt_boundary_sources_cmp(
    a: &[TerrainCdtRoadBoundarySource],
    b: &[TerrainCdtRoadBoundarySource],
) -> std::cmp::Ordering {
    for (source_a, source_b) in a.iter().copied().zip(b.iter().copied()) {
        let ordering = terrain_cdt_boundary_source_cmp(source_a, source_b);
        if !ordering.is_eq() {
            return ordering;
        }
    }
    a.len().cmp(&b.len())
}

fn terrain_cdt_boundary_source_cmp(
    a: TerrainCdtRoadBoundarySource,
    b: TerrainCdtRoadBoundarySource,
) -> std::cmp::Ordering {
    match (a, b) {
        (
            TerrainCdtRoadBoundarySource::SpanSupportBoundary {
                edge_idx: edge_idx_a,
                edge_class: edge_class_a,
                support_policy: support_policy_a,
                source_band_index: source_band_index_a,
                band_kind: band_kind_a,
                role: role_a,
                start_section_index: start_section_index_a,
                end_section_index: end_section_index_a,
                start_s_m: start_s_m_a,
                end_s_m: end_s_m_a,
            },
            TerrainCdtRoadBoundarySource::SpanSupportBoundary {
                edge_idx: edge_idx_b,
                edge_class: edge_class_b,
                support_policy: support_policy_b,
                source_band_index: source_band_index_b,
                band_kind: band_kind_b,
                role: role_b,
                start_section_index: start_section_index_b,
                end_section_index: end_section_index_b,
                start_s_m: start_s_m_b,
                end_s_m: end_s_m_b,
            },
        ) => edge_idx_a
            .cmp(&edge_idx_b)
            .then(
                terrain_cdt_edge_class_sort_key(edge_class_a)
                    .cmp(&terrain_cdt_edge_class_sort_key(edge_class_b)),
            )
            .then(
                terrain_cdt_support_policy_sort_key(support_policy_a)
                    .cmp(&terrain_cdt_support_policy_sort_key(support_policy_b)),
            )
            .then(source_band_index_a.cmp(&source_band_index_b))
            .then(
                terrain_cdt_band_kind_sort_key(band_kind_a)
                    .cmp(&terrain_cdt_band_kind_sort_key(band_kind_b)),
            )
            .then(
                terrain_cdt_span_role_sort_key(role_a).cmp(&terrain_cdt_span_role_sort_key(role_b)),
            )
            .then(start_section_index_a.cmp(&start_section_index_b))
            .then(end_section_index_a.cmp(&end_section_index_b))
            .then(start_s_m_a.total_cmp(&start_s_m_b))
            .then(end_s_m_a.total_cmp(&end_s_m_b)),
        (
            TerrainCdtRoadBoundarySource::NodeFootprintBoundary {
                node_id: node_id_a,
                node_kind: node_kind_a,
                owner_kind: owner_kind_a,
                owner_index: owner_index_a,
                boundary_source: boundary_source_a,
            },
            TerrainCdtRoadBoundarySource::NodeFootprintBoundary {
                node_id: node_id_b,
                node_kind: node_kind_b,
                owner_kind: owner_kind_b,
                owner_index: owner_index_b,
                boundary_source: boundary_source_b,
            },
        ) => node_id_a
            .cmp(&node_id_b)
            .then(
                terrain_cdt_node_kind_sort_key(node_kind_a)
                    .cmp(&terrain_cdt_node_kind_sort_key(node_kind_b)),
            )
            .then(
                terrain_cdt_band_kind_sort_key(owner_kind_a)
                    .cmp(&terrain_cdt_band_kind_sort_key(owner_kind_b)),
            )
            .then(owner_index_a.cmp(&owner_index_b))
            .then(boundary_source_a.cmp(&boundary_source_b)),
        (
            TerrainCdtRoadBoundarySource::SyntheticTestBoundary {
                stable_piece_id: stable_piece_id_a,
                local_loop_index: local_loop_index_a,
                local_edge_index: local_edge_index_a,
            },
            TerrainCdtRoadBoundarySource::SyntheticTestBoundary {
                stable_piece_id: stable_piece_id_b,
                local_loop_index: local_loop_index_b,
                local_edge_index: local_edge_index_b,
            },
        ) => stable_piece_id_a
            .cmp(&stable_piece_id_b)
            .then(local_loop_index_a.cmp(&local_loop_index_b))
            .then(local_edge_index_a.cmp(&local_edge_index_b)),
        (TerrainCdtRoadBoundarySource::SpanSupportBoundary { .. }, _) => std::cmp::Ordering::Less,
        (
            TerrainCdtRoadBoundarySource::NodeFootprintBoundary { .. },
            TerrainCdtRoadBoundarySource::SpanSupportBoundary { .. },
        ) => std::cmp::Ordering::Greater,
        (
            TerrainCdtRoadBoundarySource::NodeFootprintBoundary { .. },
            TerrainCdtRoadBoundarySource::SyntheticTestBoundary { .. },
        ) => std::cmp::Ordering::Less,
        (TerrainCdtRoadBoundarySource::SyntheticTestBoundary { .. }, _) => {
            std::cmp::Ordering::Greater
        }
    }
}

fn terrain_cdt_edge_class_sort_key(edge_class: TerrainCdtEdgeClass) -> u8 {
    match edge_class {
        TerrainCdtEdgeClass::Standard => 0,
        TerrainCdtEdgeClass::Bridge => 1,
        TerrainCdtEdgeClass::Tunnel => 2,
    }
}

fn terrain_cdt_support_policy_sort_key(policy: TerrainCdtEarthworkSupportPolicy) -> u8 {
    match policy {
        TerrainCdtEarthworkSupportPolicy::StandardFullGroundedSpan => 0,
        TerrainCdtEarthworkSupportPolicy::BridgeEndpointAbutments => 1,
        TerrainCdtEarthworkSupportPolicy::TunnelVisiblePortals => 2,
    }
}

fn terrain_cdt_band_kind_sort_key(kind: TerrainCdtRoadBandKind) -> u8 {
    match kind {
        TerrainCdtRoadBandKind::Carriageway => 0,
        TerrainCdtRoadBandKind::CurbOrShoulder => 1,
        TerrainCdtRoadBandKind::Sidewalk => 2,
        TerrainCdtRoadBandKind::Footpath => 3,
        TerrainCdtRoadBandKind::Median => 4,
        TerrainCdtRoadBandKind::Parking => 5,
        TerrainCdtRoadBandKind::CycleTrack => 6,
        TerrainCdtRoadBandKind::TramReservation => 7,
    }
}

fn terrain_cdt_span_role_sort_key(role: TerrainCdtSpanRegionRole) -> u8 {
    match role {
        TerrainCdtSpanRegionRole::Asphalt => 0,
        TerrainCdtSpanRegionRole::CurbOrShoulder => 1,
        TerrainCdtSpanRegionRole::NonRoad => 2,
    }
}

fn terrain_cdt_node_kind_sort_key(kind: TerrainCdtNodePieceKind) -> u8 {
    match kind {
        TerrainCdtNodePieceKind::Terminal => 0,
        TerrainCdtNodePieceKind::Bend => 1,
        TerrainCdtNodePieceKind::JunctionN => 2,
    }
}

fn terrain_cdt_edge_class_label(edge_class: TerrainCdtEdgeClass) -> &'static str {
    match edge_class {
        TerrainCdtEdgeClass::Standard => "standard",
        TerrainCdtEdgeClass::Bridge => "bridge",
        TerrainCdtEdgeClass::Tunnel => "tunnel",
    }
}

fn terrain_cdt_support_policy_label(policy: TerrainCdtEarthworkSupportPolicy) -> &'static str {
    match policy {
        TerrainCdtEarthworkSupportPolicy::StandardFullGroundedSpan => "standard_full_grounded_span",
        TerrainCdtEarthworkSupportPolicy::BridgeEndpointAbutments => "bridge_endpoint_abutments",
        TerrainCdtEarthworkSupportPolicy::TunnelVisiblePortals => "tunnel_visible_portals",
    }
}

fn terrain_cdt_band_kind_label(kind: TerrainCdtRoadBandKind) -> &'static str {
    match kind {
        TerrainCdtRoadBandKind::Carriageway => "carriageway",
        TerrainCdtRoadBandKind::CurbOrShoulder => "curb_or_shoulder",
        TerrainCdtRoadBandKind::Sidewalk => "sidewalk",
        TerrainCdtRoadBandKind::Footpath => "footpath",
        TerrainCdtRoadBandKind::Median => "median",
        TerrainCdtRoadBandKind::Parking => "parking",
        TerrainCdtRoadBandKind::CycleTrack => "cycle_track",
        TerrainCdtRoadBandKind::TramReservation => "tram_reservation",
    }
}

fn terrain_cdt_span_role_label(role: TerrainCdtSpanRegionRole) -> &'static str {
    match role {
        TerrainCdtSpanRegionRole::Asphalt => "asphalt",
        TerrainCdtSpanRegionRole::CurbOrShoulder => "curb_or_shoulder",
        TerrainCdtSpanRegionRole::NonRoad => "non_road",
    }
}

fn terrain_cdt_node_kind_label(kind: TerrainCdtNodePieceKind) -> &'static str {
    match kind {
        TerrainCdtNodePieceKind::Terminal => "terminal",
        TerrainCdtNodePieceKind::Bend => "bend",
        TerrainCdtNodePieceKind::JunctionN => "junction_n",
    }
}

fn normalize_edge(a: usize, b: usize) -> (usize, usize) {
    if a < b { (a, b) } else { (b, a) }
}

fn normalize_edge_array(a: usize, b: usize) -> [usize; 2] {
    if a < b { [a, b] } else { [b, a] }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spade_cdt_preserves_road_constraints_and_omits_road_faces() {
        let road = diagonal_road_loop();
        let input = TerrainCdtInput::new(
            TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]),
            vec![TerrainCdtRoadLoop::new(7, 0, road.clone())],
            vec![
                TerrainCdtVertex::new(5.0, 0.0, 5.0),
                TerrainCdtVertex::new(6.0, 0.0, 30.0),
                TerrainCdtVertex::new(18.0, 0.0, 34.0),
                TerrainCdtVertex::new(20.0, 0.0, 6.0),
                TerrainCdtVertex::new(34.0, 0.0, 10.0),
                TerrainCdtVertex::new(35.0, 0.0, 35.0),
            ],
        );

        let mesh = build_road_touched_terrain_patch(input)
            .expect("Spade should triangulate a road-touched terrain patch");

        assert!(!mesh.triangles.is_empty());
        assert_eq!(mesh.stats.road_constraint_edges, 4);
        assert_eq!(mesh.stats.invalid_constraint_edges, 0);
        assert_eq!(
            mesh.stats.preserved_road_constraint_edges,
            mesh.stats.road_constraint_edges
        );
        assert!(mesh.stats.rejected_road_faces > 0);
        for triangle in &mesh.triangles {
            let center = centroid([
                mesh.vertices[triangle[0]],
                mesh.vertices[triangle[1]],
                mesh.vertices[triangle[2]],
            ]);
            assert!(
                !point_in_polygon(center, &road),
                "accepted terrain triangle leaked into the road footprint"
            );
        }
    }

    #[test]
    fn spade_cdt_face_set_is_deterministic_for_canonical_input() {
        let input = TerrainCdtInput::new(
            TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]),
            vec![TerrainCdtRoadLoop::new(7, 0, diagonal_road_loop())],
            vec![
                TerrainCdtVertex::new(35.0, 0.0, 35.0),
                TerrainCdtVertex::new(5.0, 0.0, 5.0),
                TerrainCdtVertex::new(34.0, 0.0, 10.0),
                TerrainCdtVertex::new(20.0, 0.0, 6.0),
                TerrainCdtVertex::new(18.0, 0.0, 34.0),
                TerrainCdtVertex::new(6.0, 0.0, 30.0),
            ],
        );

        let first = build_road_touched_terrain_patch(input.clone()).unwrap();
        let second = build_road_touched_terrain_patch(input).unwrap();

        assert_eq!(
            canonical_triangle_set(&first.triangles),
            canonical_triangle_set(&second.triangles)
        );
        assert_eq!(first.stats, second.stats);
    }

    #[test]
    fn cdt_reports_source_samples_that_widen_road_tie_ins() {
        let road = vec![
            TerrainCdtVertex::new(3.0, 0.12, 3.0),
            TerrainCdtVertex::new(7.0, 0.12, 3.0),
            TerrainCdtVertex::new(7.0, 0.12, 7.0),
            TerrainCdtVertex::new(3.0, 0.12, 7.0),
        ];
        let input = TerrainCdtInput::new(
            TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]),
            vec![TerrainCdtRoadLoop::new(3, 0, road)],
            vec![
                TerrainCdtVertex::new(5.0, 0.0, 2.99),
                TerrainCdtVertex::new(2.99, 0.0, 5.0),
                TerrainCdtVertex::new(7.01, 0.0, 5.0),
                TerrainCdtVertex::new(5.0, 0.0, 7.01),
            ],
        );

        let mesh = build_road_touched_terrain_patch(input)
            .expect("Spade should triangulate a raised road seam");

        assert_eq!(
            mesh.stats.input_vertices, 8,
            "near-road source samples should be omitted from the tie-in input"
        );
        assert_eq!(mesh.stats.tie_in_widened_source_samples, 4);
        assert!(mesh.stats.tie_in_widened_max_y_delta_m >= 0.12);
        assert!(mesh.stats.tie_in_widened_max_slope_ratio > MAX_TERRAIN_TIE_IN_SLOPE_RATIO);
        assert_eq!(mesh.tie_in_widened_samples.len(), 4);
        assert!(
            mesh.tie_in_widened_samples
                .iter()
                .all(|sample| sample.required_distance_m > sample.distance_m)
        );
        assert!(mesh.stats.road_seam_faces > 0);
        assert_eq!(mesh.stats.retaining_wall_faces, 0);
        assert!(mesh.retaining_wall_triangles.is_empty());
        assert!(mesh.stats.road_seam_max_y_delta_m >= 0.12);
        assert!(
            mesh.stats.road_seam_max_slope_ratio <= MAX_TERRAIN_TIE_IN_SLOPE_RATIO + 0.0001,
            "terrain tie-in should not exceed the configured slope budget; stats={:?}",
            mesh.stats
        );
        assert!(!mesh.road_seam_face_samples.is_empty());
        assert!(
            mesh.road_seam_face_samples[0].max_slope_ratio
                >= mesh.stats.road_seam_max_slope_ratio - 0.0001
        );
    }

    #[test]
    fn cdt_diagnostics_preserve_explicit_boundary_sources() {
        let source = test_node_boundary_source(42, TerrainCdtRoadBandKind::Sidewalk, 3);
        let road = vec![
            TerrainCdtVertex::new(4.0, 4.0, 4.0),
            TerrainCdtVertex::new(6.0, 4.0, 4.0),
            TerrainCdtVertex::new(6.0, 4.0, 6.0),
            TerrainCdtVertex::new(4.0, 4.0, 6.0),
        ];
        let input = TerrainCdtInput::new(
            TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]),
            vec![sourced_road_loop(42, 0, road, source)],
            Vec::new(),
        );

        let mesh =
            build_road_touched_terrain_patch(input).expect("sourced road loop should triangulate");

        assert!(!mesh.road_seam_face_samples.is_empty());
        assert!(
            mesh.road_seam_face_samples
                .iter()
                .all(|sample| sample.sources.contains(&source)),
            "road seam diagnostics must name the explicit road boundary source"
        );
        assert!(!mesh.retaining_wall_face_samples.is_empty());
        assert!(
            mesh.retaining_wall_face_samples
                .iter()
                .all(|sample| sample.sources.contains(&source)),
            "retaining wall diagnostics must preserve the same boundary source"
        );
    }

    #[test]
    fn node_boundary_sources_keep_endpoint_provenance_in_ordering_and_merge() {
        let source_a = test_node_boundary_source_with_direct_provenance(
            42,
            TerrainCdtRoadBandKind::Sidewalk,
            3,
            30,
            31,
        );
        let source_b = test_node_boundary_source_with_direct_provenance(
            42,
            TerrainCdtRoadBandKind::Sidewalk,
            3,
            30,
            32,
        );

        assert!(terrain_cdt_boundary_source_cmp(source_a, source_b).is_lt());
        assert_eq!(
            mergeable_terrain_cdt_seam_source(source_a, source_a),
            Some(source_a)
        );
        assert_eq!(
            mergeable_terrain_cdt_seam_source(source_a, source_b),
            None,
            "node seam merging must not collapse distinct endpoint provenance"
        );
    }

    #[test]
    fn cdt_emitted_retaining_wall_faces_preserve_boundary_sources() {
        let source = test_node_boundary_source(43, TerrainCdtRoadBandKind::Sidewalk, 4);
        let road = vec![
            TerrainCdtVertex::new(4.0, 4.0, 4.0),
            TerrainCdtVertex::new(6.0, 4.0, 4.0),
            TerrainCdtVertex::new(6.0, 4.0, 6.0),
            TerrainCdtVertex::new(4.0, 4.0, 6.0),
        ];
        let input = TerrainCdtInput::new(
            TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]),
            vec![sourced_road_loop(43, 0, road, source)],
            Vec::new(),
        );

        let mesh =
            build_road_touched_terrain_patch(input).expect("sourced road loop should triangulate");

        assert_eq!(mesh.emitted_faces.len(), mesh.stats.accepted_faces);
        assert_eq!(
            mesh.retaining_wall_triangle_sources.len(),
            mesh.retaining_wall_triangles.len()
        );
        assert!(
            !mesh.retaining_wall_triangles.is_empty(),
            "raised seam should emit explicit retaining-wall tie-in faces"
        );
        assert!(
            mesh.retaining_wall_triangle_sources
                .iter()
                .all(|sources| sources.contains(&source)),
            "emitted retaining-wall faces must carry their road boundary source"
        );
        assert!(
            mesh.emitted_faces
                .iter()
                .filter(|face| face.kind == TerrainCdtTieInKind::RetainingWall)
                .all(|face| face.sources.contains(&source)),
            "the first-class emitted-face model must preserve retaining-wall provenance"
        );
    }

    #[test]
    fn cdt_emitted_road_seam_terrain_faces_preserve_boundary_sources() {
        let source = test_node_boundary_source(44, TerrainCdtRoadBandKind::CurbOrShoulder, 5);
        let road = vec![
            TerrainCdtVertex::new(3.0, 0.12, 3.0),
            TerrainCdtVertex::new(7.0, 0.12, 3.0),
            TerrainCdtVertex::new(7.0, 0.12, 7.0),
            TerrainCdtVertex::new(3.0, 0.12, 7.0),
        ];
        let input = TerrainCdtInput::new(
            TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]),
            vec![sourced_road_loop(44, 0, road, source)],
            vec![
                TerrainCdtVertex::new(5.0, 0.0, 2.99),
                TerrainCdtVertex::new(2.99, 0.0, 5.0),
                TerrainCdtVertex::new(7.01, 0.0, 5.0),
                TerrainCdtVertex::new(5.0, 0.0, 7.01),
            ],
        );

        let mesh =
            build_road_touched_terrain_patch(input).expect("sourced road loop should triangulate");

        assert_eq!(mesh.terrain_triangle_sources.len(), mesh.triangles.len());
        assert!(mesh.retaining_wall_triangles.is_empty());
        assert!(
            mesh.terrain_triangle_sources
                .iter()
                .any(|sources| sources.contains(&source)),
            "accepted road-seam terrain faces must carry their road boundary source"
        );
        assert!(
            mesh.emitted_faces.iter().any(|face| {
                face.kind == TerrainCdtTieInKind::OrdinaryTerrain && face.sources.contains(&source)
            }),
            "the first-class emitted-face model must preserve ordinary seam provenance"
        );
    }

    #[test]
    fn cdt_emitted_non_road_terrain_faces_may_be_source_empty() {
        let input = TerrainCdtInput::new(
            TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]),
            Vec::new(),
            Vec::new(),
        );

        let mesh = build_road_touched_terrain_patch(input)
            .expect("plain terrain patch should triangulate without road sources");

        assert!(!mesh.triangles.is_empty());
        assert_eq!(mesh.terrain_triangle_sources.len(), mesh.triangles.len());
        assert!(mesh.terrain_triangle_sources.iter().all(Vec::is_empty));
        assert!(
            mesh.emitted_faces
                .iter()
                .all(|face| face.sources.is_empty())
        );
    }

    #[test]
    fn cdt_tie_in_widening_preserves_closest_seam_source() {
        let source = test_node_boundary_source(77, TerrainCdtRoadBandKind::CurbOrShoulder, 1);
        let road = vec![
            TerrainCdtVertex::new(3.0, 0.12, 3.0),
            TerrainCdtVertex::new(7.0, 0.12, 3.0),
            TerrainCdtVertex::new(7.0, 0.12, 7.0),
            TerrainCdtVertex::new(3.0, 0.12, 7.0),
        ];
        let input = TerrainCdtInput::new(
            TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]),
            vec![sourced_road_loop(77, 0, road, source)],
            vec![TerrainCdtVertex::new(5.0, 0.0, 2.99)],
        );

        let mesh = build_road_touched_terrain_patch(input)
            .expect("sourced tie-in widening case should triangulate");

        assert_eq!(mesh.tie_in_widened_samples.len(), 1);
        assert_eq!(mesh.tie_in_widened_samples[0].seam_source, source);
    }

    #[test]
    fn cdt_tie_in_widening_ties_choose_seam_geometry_before_source_identity() {
        let patch = TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]);
        let horizontal_source = test_node_boundary_source(88, TerrainCdtRoadBandKind::Sidewalk, 1);
        let vertical_source = test_node_boundary_source(88, TerrainCdtRoadBandKind::Sidewalk, 2);
        let source_samples = vec![TerrainCdtVertex::new(5.0, 0.0, 5.0)];

        let first = build_road_touched_terrain_patch(TerrainCdtInput::new(
            patch,
            vec![sourced_l_road_loop_with_notch_sources(
                horizontal_source,
                vertical_source,
            )],
            source_samples.clone(),
        ))
        .expect("first sourced L road loop should triangulate");
        let second = build_road_touched_terrain_patch(TerrainCdtInput::new(
            patch,
            vec![sourced_l_road_loop_with_notch_sources(
                vertical_source,
                horizontal_source,
            )],
            source_samples,
        ))
        .expect("reordered sourced L road loop should triangulate");

        assert_eq!(
            first.stats, second.stats,
            "source identity order must not change tie-in widening diagnostics"
        );
        assert_eq!(first.tie_in_widened_samples.len(), 1);
        assert_eq!(second.tie_in_widened_samples.len(), 1);
        assert_eq!(
            first.tie_in_widened_samples[0].seam_point, second.tie_in_widened_samples[0].seam_point,
            "equidistant seam candidates must choose by geometry before provenance"
        );
        assert!(same_coord(
            first.tie_in_widened_samples[0].seam_point.x,
            4.0
        ));
        assert!(same_coord(
            first.tie_in_widened_samples[0].seam_point.z,
            5.0
        ));
    }

    #[test]
    fn cdt_promotes_high_delta_omitted_tie_in_source_to_retaining_wall() {
        let source = TerrainCdtRoadBoundarySource::NodeFootprintBoundary {
            node_id: 1,
            node_kind: TerrainCdtNodePieceKind::Terminal,
            owner_kind: TerrainCdtRoadBandKind::Sidewalk,
            owner_index: 0,
            boundary_source: None,
        };
        let road = vec![
            TerrainCdtVertex::new(48.0, 1.2, 48.0),
            TerrainCdtVertex::new(52.0, 1.2, 48.0),
            TerrainCdtVertex::new(52.0, 1.2, 52.0),
            TerrainCdtVertex::new(48.0, 1.2, 52.0),
        ];
        let input = TerrainCdtInput::new(
            TerrainCdtPatch::new(0.0, 0.0, 100.0, 100.0, [0.0; 4]),
            vec![sourced_road_loop(1, 0, road, source)],
            vec![TerrainCdtVertex::new(50.0, 0.0, 47.99)],
        );

        let mesh = build_road_touched_terrain_patch(input)
            .expect("high-delta sourced tie-in widening should triangulate");

        assert_eq!(mesh.stats.tie_in_widened_source_samples, 1);
        assert!(mesh.stats.retaining_wall_faces > 0);
        assert!(
            mesh.retaining_wall_triangle_sources
                .iter()
                .any(|sources| sources.contains(&source)),
            "source-required retaining walls must preserve the omitted terminal sidewalk source"
        );
        assert!(
            mesh.emitted_faces.iter().any(|face| {
                face.kind == TerrainCdtTieInKind::RetainingWall && face.sources.contains(&source)
            }),
            "first-class emitted retaining-wall faces must carry the required seam source"
        );
    }

    #[test]
    fn cdt_merges_subbudget_same_authority_seam_fragments_before_triangulation() {
        let source_a = test_span_boundary_source_range(
            78,
            TerrainCdtRoadBandKind::Sidewalk,
            5,
            15,
            16,
            10.0,
            10.004,
        );
        let source_b = test_span_boundary_source_range(
            78,
            TerrainCdtRoadBandKind::Sidewalk,
            5,
            16,
            17,
            10.004,
            12.0,
        );
        let source_c = test_span_boundary_source_range(
            78,
            TerrainCdtRoadBandKind::Sidewalk,
            5,
            17,
            18,
            12.0,
            14.0,
        );
        let road = vec![
            TerrainCdtVertex::new(3.0, 0.12, 3.0),
            TerrainCdtVertex::new(5.0, 0.12, 3.0),
            TerrainCdtVertex::new(5.004, 0.12, 3.0),
            TerrainCdtVertex::new(7.0, 0.12, 3.0),
            TerrainCdtVertex::new(7.0, 0.12, 7.0),
            TerrainCdtVertex::new(3.0, 0.12, 7.0),
        ];
        let source_edges = vec![
            TerrainCdtRoadLoopSourceEdge {
                start: road[0],
                end: road[1],
                source: source_a,
            },
            TerrainCdtRoadLoopSourceEdge {
                start: road[1],
                end: road[2],
                source: source_b,
            },
            TerrainCdtRoadLoopSourceEdge {
                start: road[2],
                end: road[3],
                source: source_c,
            },
            TerrainCdtRoadLoopSourceEdge {
                start: road[3],
                end: road[4],
                source: source_c,
            },
            TerrainCdtRoadLoopSourceEdge {
                start: road[4],
                end: road[5],
                source: source_c,
            },
            TerrainCdtRoadLoopSourceEdge {
                start: road[5],
                end: road[0],
                source: source_c,
            },
        ];
        let input = TerrainCdtInput::new(
            TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]),
            vec![TerrainCdtRoadLoop::new_with_source_edges(
                78,
                0,
                road,
                source_edges,
            )],
            Vec::new(),
        );

        let mesh = build_road_touched_terrain_patch(input)
            .expect("source-compatible seam fragments should merge before Spade input");

        assert_eq!(mesh.stats.invalid_constraint_edges, 0);
        assert_eq!(mesh.stats.merged_subbudget_seam_edges, 1);
        assert_eq!(mesh.stats.blocking_degenerate_seam_edges, 0);
        assert_eq!(mesh.seam_quality_samples.len(), 1);
        let sample = mesh.seam_quality_samples[0];
        assert_eq!(
            sample.kind,
            TerrainCdtSeamQualityKind::MergedSubbudgetSeamEdge
        );
        assert!(sample.length_m > MIN_SOURCE_OWNED_SEAM_EDGE_LENGTH_M as f32);
        match sample.source {
            TerrainCdtRoadBoundarySource::SpanSupportBoundary {
                edge_idx,
                source_band_index,
                start_section_index,
                end_section_index,
                start_s_m,
                end_s_m,
                ..
            } => {
                assert_eq!(edge_idx, 78);
                assert_eq!(source_band_index, 5);
                assert_eq!(start_section_index, 15);
                assert_eq!(end_section_index, 17);
                assert_eq!(start_s_m, 10.0);
                assert_eq!(end_s_m, 12.0);
            }
            other => panic!("merged seam must preserve span authority, got {other:?}"),
        }
    }

    #[test]
    fn cdt_splits_loop_segments_through_source_vertices_before_source_mapping() {
        let source_a = test_span_boundary_source_range(
            92,
            TerrainCdtRoadBandKind::Sidewalk,
            5,
            15,
            16,
            10.0,
            11.0,
        );
        let source_b = test_span_boundary_source_range(
            92,
            TerrainCdtRoadBandKind::Sidewalk,
            5,
            16,
            17,
            11.0,
            12.0,
        );
        let source_c = test_span_boundary_source_range(
            92,
            TerrainCdtRoadBandKind::Sidewalk,
            5,
            17,
            18,
            12.0,
            13.0,
        );
        let p0 = TerrainCdtVertex::new(3.0, 1.0, 3.0);
        let p1 = TerrainCdtVertex::new(5.0, 1.0, 3.0);
        let p2 = TerrainCdtVertex::new(7.0, 1.0, 3.0);
        let p3 = TerrainCdtVertex::new(7.0, 1.0, 7.0);
        let p4 = TerrainCdtVertex::new(3.0, 1.0, 7.0);
        let input = TerrainCdtInput::new(
            TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]),
            vec![TerrainCdtRoadLoop::new_with_source_edges(
                92,
                0,
                vec![p0, p2, p3, p4],
                vec![
                    TerrainCdtRoadLoopSourceEdge {
                        start: p0,
                        end: p1,
                        source: source_a,
                    },
                    TerrainCdtRoadLoopSourceEdge {
                        start: p1,
                        end: p2,
                        source: source_b,
                    },
                    TerrainCdtRoadLoopSourceEdge {
                        start: p2,
                        end: p3,
                        source: source_c,
                    },
                    TerrainCdtRoadLoopSourceEdge {
                        start: p3,
                        end: p4,
                        source: source_c,
                    },
                    TerrainCdtRoadLoopSourceEdge {
                        start: p4,
                        end: p0,
                        source: source_c,
                    },
                ],
            )],
            Vec::new(),
        );

        let mesh = build_road_touched_terrain_patch(input)
            .expect("CDT road loops must split through source vertices before mapping sources");

        assert_eq!(mesh.stats.invalid_constraint_edges, 0);
        assert!(
            mesh.stats.road_constraint_edges >= 5,
            "the p0..p2 boundary segment must be split through p1"
        );
        assert!(
            mesh.emitted_faces
                .iter()
                .flat_map(|face| face.sources.iter())
                .any(|source| *source == source_a),
            "first split road boundary source must survive CDT output"
        );
        assert!(
            mesh.emitted_faces
                .iter()
                .flat_map(|face| face.sources.iter())
                .any(|source| *source == source_b),
            "second split road boundary source must survive CDT output"
        );
    }

    #[test]
    fn cdt_rejects_unsourced_road_boundary_constraints() {
        let source = test_node_boundary_source(91, TerrainCdtRoadBandKind::Sidewalk, 2);
        let road = vec![
            TerrainCdtVertex::new(3.0, 0.0, 3.0),
            TerrainCdtVertex::new(7.0, 0.0, 3.0),
            TerrainCdtVertex::new(7.0, 0.0, 7.0),
            TerrainCdtVertex::new(3.0, 0.0, 7.0),
        ];
        let input = TerrainCdtInput::new(
            TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]),
            vec![TerrainCdtRoadLoop::new_with_source_edges(
                91,
                0,
                road.clone(),
                vec![TerrainCdtRoadLoopSourceEdge {
                    start: road[0],
                    end: road[1],
                    source,
                }],
            )],
            Vec::new(),
        );

        assert_eq!(
            build_road_touched_terrain_patch(input),
            Err(TerrainCdtError::MissingRoadBoundarySource)
        );
    }

    #[test]
    fn cdt_classifies_overbudget_road_seam_faces_as_retaining_walls() {
        let road = vec![
            TerrainCdtVertex::new(4.0, 4.0, 4.0),
            TerrainCdtVertex::new(6.0, 4.0, 4.0),
            TerrainCdtVertex::new(6.0, 4.0, 6.0),
            TerrainCdtVertex::new(4.0, 4.0, 6.0),
        ];
        let input = TerrainCdtInput::new(
            TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]),
            vec![TerrainCdtRoadLoop::new(5, 0, road.clone())],
            Vec::new(),
        );

        let mesh = build_road_touched_terrain_patch(input)
            .expect("Spade should classify over-budget road seam tie-ins");

        assert_eq!(mesh.stats.invalid_constraint_edges, 0);
        assert!(mesh.stats.road_seam_faces > 0);
        assert!(mesh.stats.retaining_wall_faces > 0);
        assert_eq!(
            mesh.stats.accepted_faces,
            mesh.triangles.len() + mesh.retaining_wall_triangles.len()
        );
        assert_eq!(
            mesh.stats.retaining_wall_faces,
            mesh.retaining_wall_triangles.len()
        );
        assert!(
            mesh.stats.retaining_wall_max_slope_ratio > MAX_TERRAIN_TIE_IN_SLOPE_RATIO,
            "retaining wall classification must be driven by the documented slope budget"
        );
        assert!(
            mesh.road_seam_face_samples
                .iter()
                .any(|sample| sample.kind == TerrainCdtTieInKind::RetainingWall)
        );
        assert!(
            mesh.retaining_wall_face_samples
                .iter()
                .all(|sample| sample.kind == TerrainCdtTieInKind::RetainingWall)
        );
        assert!(
            mesh.retaining_wall_triangles.iter().all(|triangle| {
                let center = centroid([
                    mesh.vertices[triangle[0]],
                    mesh.vertices[triangle[1]],
                    mesh.vertices[triangle[2]],
                ]);
                !point_in_polygon(center, &road)
            }),
            "retaining walls are explicit terrain tie-ins, not emitted road-footprint faces"
        );
    }

    #[test]
    fn road_touched_dem_validation_matrix_covers_retaining_wall_tie_ins() {
        assert_road_touched_dem_tie_in_case(
            "ordinary raised road on supportive DEM",
            square_road_loop(3.0, 7.0, 0.20),
            Vec::new(),
            0,
            false,
        );
        assert_road_touched_dem_tie_in_case(
            "near-road DEM samples widen ordinary cut fill",
            square_road_loop(3.0, 7.0, 0.12),
            vec![
                TerrainCdtVertex::new(5.0, 0.0, 2.99),
                TerrainCdtVertex::new(2.99, 0.0, 5.0),
                TerrainCdtVertex::new(7.01, 0.0, 5.0),
                TerrainCdtVertex::new(5.0, 0.0, 7.01),
            ],
            4,
            false,
        );
        assert_road_touched_dem_tie_in_case(
            "raised road above unavoidable cliff DEM",
            square_road_loop(4.0, 6.0, 4.0),
            Vec::new(),
            0,
            true,
        );
        assert_road_touched_dem_tie_in_case(
            "lowered road below unavoidable cliff DEM",
            square_road_loop(4.0, 6.0, -4.0),
            Vec::new(),
            0,
            true,
        );
        assert_road_touched_dem_tie_in_case(
            "near-road DEM widening still leaves explicit retaining wall",
            square_road_loop(4.0, 6.0, 4.0),
            vec![
                TerrainCdtVertex::new(5.0, 0.0, 3.99),
                TerrainCdtVertex::new(3.99, 0.0, 5.0),
                TerrainCdtVertex::new(6.01, 0.0, 5.0),
                TerrainCdtVertex::new(5.0, 0.0, 6.01),
            ],
            4,
            true,
        );
    }

    #[test]
    fn authored_steep_dem_matrix_preserves_sourced_road_touched_contract() {
        let patch = TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0, 0.0, 0.0, 0.0]);
        let cases = vec![
            (
                "road crossing a steep hillside",
                road_loop_from_centerline_with_heights(
                    TerrainCdtVertex::new(5.0, authored_cross_slope_height(5.0, 20.0), 20.0),
                    TerrainCdtVertex::new(35.0, authored_cross_slope_height(35.0, 20.0), 20.0),
                    6.0,
                ),
                authored_dem_samples(patch, 4.0, authored_cross_slope_height),
                false,
            ),
            (
                "road running along a cross-slope",
                road_loop_from_centerline_with_heights(
                    TerrainCdtVertex::new(20.0, authored_along_slope_height(20.0, 5.0), 5.0),
                    TerrainCdtVertex::new(20.0, authored_along_slope_height(20.0, 35.0), 35.0),
                    6.0,
                ),
                authored_dem_samples(patch, 4.0, authored_along_slope_height),
                false,
            ),
            (
                "road crossing an authored ridge and valley",
                road_loop_from_centerline_with_heights(
                    TerrainCdtVertex::new(6.0, 0.0, 10.0),
                    TerrainCdtVertex::new(34.0, 0.0, 30.0),
                    6.0,
                ),
                authored_dem_samples(patch, 4.0, authored_ridge_valley_height),
                false,
            ),
        ];

        for (case_name, road, source_samples, expect_retaining_wall) in cases {
            let source = test_span_boundary_source(200, TerrainCdtRoadBandKind::Sidewalk, 4);
            let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
                patch,
                vec![sourced_road_loop(200, 0, road.clone(), source)],
                source_samples,
            ))
            .unwrap_or_else(|_| panic!("{case_name}: terrain CDT should build"));

            assert_sourced_road_touched_mesh_contract(case_name, &mesh, patch, &[road], source);
            if expect_retaining_wall {
                assert!(
                    mesh.stats.retaining_wall_faces > 0,
                    "{case_name}: authored extreme DEM should expose retaining-wall tie-in faces"
                );
            }
        }
    }

    #[test]
    fn sourced_patch_edge_matrix_preserves_sources_after_clipping() {
        let patch = TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]);
        let source = test_span_boundary_source(201, TerrainCdtRoadBandKind::CurbOrShoulder, 2);
        let cases = [
            (
                "road footprint crossing one patch edge",
                road_loop_from_centerline(
                    TerrainCdtVertex::new(-10.0, 0.0, 20.0),
                    TerrainCdtVertex::new(20.0, 0.0, 20.0),
                    6.0,
                ),
            ),
            (
                "road footprint crossing two patch edges",
                road_loop_from_centerline(
                    TerrainCdtVertex::new(-10.0, 0.0, 20.0),
                    TerrainCdtVertex::new(50.0, 0.0, 20.0),
                    6.0,
                ),
            ),
            (
                "road footprint crossing a patch corner",
                road_loop_from_centerline(
                    TerrainCdtVertex::new(-10.0, 0.0, -10.0),
                    TerrainCdtVertex::new(20.0, 0.0, 20.0),
                    6.0,
                ),
            ),
        ];

        for (case_name, road) in cases {
            let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
                patch,
                vec![sourced_road_loop(201, 0, road.clone(), source)],
                piece_source_samples(),
            ))
            .unwrap_or_else(|_| panic!("{case_name}: clipped terrain CDT should build"));

            assert_sourced_road_touched_mesh_contract(case_name, &mesh, patch, &[road], source);
        }
    }

    #[test]
    fn road_loop_crossing_one_patch_edge_is_clipped_to_shared_boundary_vertices() {
        let patch = TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]);
        let road = road_loop_from_centerline(
            TerrainCdtVertex::new(-10.0, 0.0, 20.0),
            TerrainCdtVertex::new(20.0, 0.0, 20.0),
            6.0,
        );

        let mesh = build_crossing_patch(patch, road.clone());
        assert_valid_clipped_mesh(&mesh, patch, &road);
        assert!(
            mesh.vertices
                .iter()
                .any(|vertex| same_coord(vertex.x, patch.min_x))
        );
    }

    #[test]
    fn road_loop_patch_clipping_preserves_boundary_sources() {
        let patch = TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]);
        let road = road_loop_from_centerline(
            TerrainCdtVertex::new(-10.0, 0.0, 20.0),
            TerrainCdtVertex::new(20.0, 0.0, 20.0),
            6.0,
        );
        let source = test_node_boundary_source(88, TerrainCdtRoadBandKind::Sidewalk, 5);

        let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
            patch,
            vec![sourced_road_loop(88, 0, road.clone(), source)],
            Vec::new(),
        ))
        .expect("source-preserving clipped road loop should triangulate");

        assert_valid_clipped_mesh(&mesh, patch, &road);
        assert!(
            !mesh.road_seam_face_samples.is_empty(),
            "clipped sourced road loop should still report seam diagnostics"
        );
        assert!(
            mesh.road_seam_face_samples
                .iter()
                .all(|sample| sample.sources.contains(&source)),
            "patch-clipped road seam constraints must inherit their original source edge"
        );
    }

    #[test]
    fn road_loop_crossing_two_patch_edges_splits_both_patch_boundary_constraints() {
        let patch = TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]);
        let road = road_loop_from_centerline(
            TerrainCdtVertex::new(-10.0, 0.0, 20.0),
            TerrainCdtVertex::new(50.0, 0.0, 20.0),
            6.0,
        );

        let mesh = build_crossing_patch(patch, road.clone());
        assert_valid_clipped_mesh(&mesh, patch, &road);
        assert!(
            mesh.vertices
                .iter()
                .any(|vertex| same_coord(vertex.x, patch.min_x))
        );
        assert!(
            mesh.vertices
                .iter()
                .any(|vertex| same_coord(vertex.x, patch.max_x))
        );
    }

    #[test]
    fn road_loop_crossing_patch_corner_uses_corner_as_constraint_endpoint() {
        let patch = TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]);
        let road = road_loop_from_centerline(
            TerrainCdtVertex::new(-10.0, 0.0, -10.0),
            TerrainCdtVertex::new(20.0, 0.0, 20.0),
            6.0,
        );

        let mesh = build_crossing_patch(patch, road.clone());
        assert_valid_clipped_mesh(&mesh, patch, &road);
        assert!(
            mesh.vertices.iter().any(
                |vertex| same_coord(vertex.x, patch.min_x) && same_coord(vertex.z, patch.min_z)
            )
        );
    }

    #[test]
    fn multiple_road_loops_in_one_patch_preserve_all_seam_constraints_deterministically() {
        let patch = TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]);
        let road_a = road_loop_from_centerline(
            TerrainCdtVertex::new(8.0, 0.0, 10.0),
            TerrainCdtVertex::new(18.0, 0.0, 18.0),
            4.0,
        );
        let road_b = road_loop_from_centerline(
            TerrainCdtVertex::new(22.0, 0.0, 28.0),
            TerrainCdtVertex::new(34.0, 0.0, 28.0),
            4.0,
        );
        let input = TerrainCdtInput::new(
            patch,
            vec![
                TerrainCdtRoadLoop::new(99, 0, road_b.clone()),
                TerrainCdtRoadLoop::new(7, 0, road_a.clone()),
            ],
            vec![
                TerrainCdtVertex::new(5.0, 0.0, 5.0),
                TerrainCdtVertex::new(5.0, 0.0, 35.0),
                TerrainCdtVertex::new(20.0, 0.0, 5.0),
                TerrainCdtVertex::new(20.0, 0.0, 35.0),
                TerrainCdtVertex::new(35.0, 0.0, 5.0),
                TerrainCdtVertex::new(35.0, 0.0, 35.0),
            ],
        );

        let first = build_road_touched_terrain_patch(input.clone())
            .expect("Spade should triangulate multiple road loops");
        let second = build_road_touched_terrain_patch(input)
            .expect("Spade should deterministically triangulate multiple road loops");

        assert_eq!(first.stats.road_constraint_edges, 8);
        assert_eq!(first.stats.invalid_constraint_edges, 0);
        assert_eq!(
            first.stats.preserved_road_constraint_edges,
            first.stats.road_constraint_edges
        );
        assert_eq!(
            canonical_triangle_set(&first.triangles),
            canonical_triangle_set(&second.triangles)
        );
        for triangle in &first.triangles {
            let center = centroid([
                first.vertices[triangle[0]],
                first.vertices[triangle[1]],
                first.vertices[triangle[2]],
            ]);
            assert!(!point_in_polygon(center, &road_a));
            assert!(!point_in_polygon(center, &road_b));
        }
    }

    #[test]
    fn bend_footprint_loop_preserves_piece_owned_constraints() {
        let patch = piece_test_patch();
        let road = vec![
            test_vertex(10.0, 10.0),
            test_vertex(26.0, 10.0),
            test_vertex(26.0, 20.0),
            test_vertex(42.0, 20.0),
            test_vertex(42.0, 34.0),
            test_vertex(10.0, 34.0),
        ];

        let mesh = build_piece_patch(patch, 11, road.clone());

        assert_valid_piece_footprint_mesh(&mesh, patch, &road);
    }

    #[test]
    fn terminal_footprint_loop_preserves_piece_owned_constraints() {
        let patch = piece_test_patch();
        let road = vec![
            test_vertex(22.0, 8.0),
            test_vertex(38.0, 8.0),
            test_vertex(38.0, 36.0),
            test_vertex(44.0, 40.0),
            test_vertex(38.0, 44.0),
            test_vertex(22.0, 44.0),
            test_vertex(16.0, 40.0),
            test_vertex(22.0, 36.0),
        ];

        let mesh = build_piece_patch(patch, 12, road.clone());

        assert_valid_piece_footprint_mesh(&mesh, patch, &road);
    }

    #[test]
    fn junction_n_footprint_loop_preserves_piece_owned_constraints() {
        let patch = piece_test_patch();
        let road = vec![
            test_vertex(24.0, 4.0),
            test_vertex(36.0, 4.0),
            test_vertex(36.0, 24.0),
            test_vertex(56.0, 24.0),
            test_vertex(56.0, 36.0),
            test_vertex(36.0, 36.0),
            test_vertex(36.0, 56.0),
            test_vertex(24.0, 56.0),
            test_vertex(24.0, 36.0),
            test_vertex(4.0, 36.0),
            test_vertex(4.0, 24.0),
            test_vertex(24.0, 24.0),
        ];

        let first = build_piece_patch(patch, 13, road.clone());
        let second = build_piece_patch(patch, 13, road.clone());

        assert_valid_piece_footprint_mesh(&first, patch, &road);
        assert_eq!(
            canonical_triangle_set(&first.triangles),
            canonical_triangle_set(&second.triangles)
        );
        assert_eq!(first.stats, second.stats);
    }

    #[test]
    fn crossing_road_constraints_are_noded_before_triangulation() {
        let patch = TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]);
        let road_a = road_loop_from_centerline(
            TerrainCdtVertex::new(4.0, 0.0, 20.0),
            TerrainCdtVertex::new(36.0, 0.0, 20.0),
            5.0,
        );
        let road_b = road_loop_from_centerline(
            TerrainCdtVertex::new(20.0, 0.0, 4.0),
            TerrainCdtVertex::new(20.0, 0.0, 36.0),
            5.0,
        );

        let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
            patch,
            vec![
                TerrainCdtRoadLoop::new(21, 0, road_a),
                TerrainCdtRoadLoop::new(22, 0, road_b),
            ],
            piece_source_samples(),
        ))
        .expect("crossing road loops must not panic the terrain bridge");

        assert_eq!(
            mesh.stats.invalid_constraint_edges, 0,
            "road constraints must be split at deterministic intersections before Spade sees them"
        );
        assert!(
            mesh.stats.road_constraint_edges > 8,
            "crossing road loops should gain noded roadbed constraints"
        );
        for vertex in &mesh.vertices {
            assert!(patch_contains(*vertex, patch));
        }
    }

    #[test]
    fn source_sample_on_road_seam_splits_the_road_constraint() {
        let patch = TerrainCdtPatch::new(0.0, 0.0, 20.0, 20.0, [0.0; 4]);
        let road = vec![
            TerrainCdtVertex::new(4.0, 1.0, 4.0),
            TerrainCdtVertex::new(16.0, 1.0, 4.0),
            TerrainCdtVertex::new(16.0, 1.0, 10.0),
            TerrainCdtVertex::new(4.0, 1.0, 10.0),
        ];

        let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
            patch,
            vec![TerrainCdtRoadLoop::new(41, 0, road)],
            vec![TerrainCdtVertex::new(16.0, 1.0, 7.0)],
        ))
        .expect("terrain source samples on a road seam must not invalidate the CDT");

        assert_eq!(mesh.stats.invalid_constraint_edges, 0);
        assert!(
            mesh.stats.road_constraint_edges > 4,
            "the road seam constraint must be split at the existing source sample vertex"
        );
        assert_eq!(
            mesh.stats.preserved_road_constraint_edges,
            mesh.stats.road_constraint_edges
        );
        assert!(mesh.vertices.iter().any(|vertex| {
            same_coord(vertex.x, 16.0)
                && same_coord(vertex.z, 7.0)
                && same_height(vertex.height_m, 1.0)
        }));
    }

    #[test]
    fn conflicting_height_road_constraints_are_not_welded_by_height_max() {
        let patch = TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0; 4]);
        let road_a = road_loop_from_centerline(
            TerrainCdtVertex::new(4.0, 0.0, 20.0),
            TerrainCdtVertex::new(36.0, 0.0, 20.0),
            8.0,
        );
        let mut road_b = road_loop_from_centerline(
            TerrainCdtVertex::new(20.0, 0.0, 4.0),
            TerrainCdtVertex::new(20.0, 0.0, 36.0),
            8.0,
        );
        for vertex in &mut road_b {
            vertex.height_m = 1.0;
        }

        let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
            patch,
            vec![
                TerrainCdtRoadLoop::new(31, 0, road_a),
                TerrainCdtRoadLoop::new(32, 0, road_b),
            ],
            piece_source_samples(),
        ))
        .expect("conflicting road constraints should report invalid constraints without panicking");

        assert!(
            mesh.stats.invalid_constraint_edges > 0,
            "conflicting road seam heights must stay visible as CDT diagnostics instead of being welded by max-height"
        );
        assert!(
            !mesh.vertices.iter().any(|vertex| {
                vertex.height_m > 0.9
                    && vertex.x > 15.0
                    && vertex.x < 25.0
                    && vertex.z > 15.0
                    && vertex.z < 25.0
            }),
            "conflicting road constraints must not create synthesized max-height intersection vertices"
        );
    }

    #[test]
    fn road_loop_endpoint_on_another_loop_edge_splits_the_roadbed_constraint() {
        let patch = TerrainCdtPatch::new(-96.0, -32.0, 64.0, 64.0, [0.0; 4]);
        let horizontal = vec![
            TerrainCdtVertex::new(-83.390, 0.12, -18.916),
            TerrainCdtVertex::new(49.610, 0.12, -18.916),
            TerrainCdtVertex::new(49.610, 0.12, -8.916),
            TerrainCdtVertex::new(-83.390, 0.12, -8.916),
        ];
        let incoming = vec![
            TerrainCdtVertex::new(-16.818, 0.12, -8.916),
            TerrainCdtVertex::new(-9.747, 0.12, -1.845),
            TerrainCdtVertex::new(-16.818, 0.12, 5.226),
            TerrainCdtVertex::new(-23.889, 0.12, -1.845),
        ];

        let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
            patch,
            vec![
                TerrainCdtRoadLoop::new(0, 0, horizontal),
                TerrainCdtRoadLoop::new(1, 0, incoming),
            ],
            Vec::new(),
        ))
        .expect("T-touching terrain roadbed constraints must be accepted");

        assert_eq!(mesh.stats.invalid_constraint_edges, 0);
        assert!(
            mesh.stats.road_constraint_edges > 8,
            "the horizontal roadbed edge must be split at the incoming mouth vertex"
        );
        assert_eq!(
            mesh.stats.preserved_road_constraint_edges,
            mesh.stats.road_constraint_edges
        );
        assert!(mesh.vertices.iter().any(|vertex| {
            same_coord(vertex.x, -16.818)
                && same_coord(vertex.z, -8.916)
                && (vertex.height_m - 0.12).abs() <= 0.0001
        }));
    }

    fn sourced_road_loop(
        stable_piece_id: u64,
        local_loop_index: u32,
        vertices: Vec<TerrainCdtVertex>,
        source: TerrainCdtRoadBoundarySource,
    ) -> TerrainCdtRoadLoop {
        let source_edges = vertices
            .iter()
            .copied()
            .enumerate()
            .map(|(index, start)| TerrainCdtRoadLoopSourceEdge {
                start,
                end: vertices[(index + 1) % vertices.len()],
                source,
            })
            .collect();
        TerrainCdtRoadLoop::new_with_source_edges(
            stable_piece_id,
            local_loop_index,
            vertices,
            source_edges,
        )
    }

    fn sourced_l_road_loop_with_notch_sources(
        notch_horizontal_source: TerrainCdtRoadBoundarySource,
        notch_vertical_source: TerrainCdtRoadBoundarySource,
    ) -> TerrainCdtRoadLoop {
        let fallback_source = test_node_boundary_source(88, TerrainCdtRoadBandKind::Sidewalk, 3);
        let vertices = vec![
            TerrainCdtVertex::new(2.0, 0.0, 2.0),
            TerrainCdtVertex::new(8.0, 0.0, 2.0),
            TerrainCdtVertex::new(8.0, 4.0, 4.0),
            TerrainCdtVertex::new(4.0, 2.0, 4.0),
            TerrainCdtVertex::new(4.0, 0.0, 8.0),
            TerrainCdtVertex::new(2.0, 0.0, 8.0),
        ];
        let sources = [
            fallback_source,
            fallback_source,
            notch_horizontal_source,
            notch_vertical_source,
            fallback_source,
            fallback_source,
        ];
        let source_edges = vertices
            .iter()
            .copied()
            .enumerate()
            .map(|(index, start)| TerrainCdtRoadLoopSourceEdge {
                start,
                end: vertices[(index + 1) % vertices.len()],
                source: sources[index],
            })
            .collect();
        TerrainCdtRoadLoop::new_with_source_edges(88, 0, vertices, source_edges)
    }

    fn test_node_boundary_source(
        node_id: u32,
        owner_kind: TerrainCdtRoadBandKind,
        owner_index: u32,
    ) -> TerrainCdtRoadBoundarySource {
        TerrainCdtRoadBoundarySource::NodeFootprintBoundary {
            node_id,
            node_kind: TerrainCdtNodePieceKind::JunctionN,
            owner_kind,
            owner_index,
            boundary_source: None,
        }
    }

    fn test_node_boundary_source_with_direct_provenance(
        node_id: u32,
        owner_kind: TerrainCdtRoadBandKind,
        owner_index: u32,
        start_grade_authority_index: u64,
        end_grade_authority_index: u64,
    ) -> TerrainCdtRoadBoundarySource {
        TerrainCdtRoadBoundarySource::NodeFootprintBoundary {
            node_id,
            node_kind: TerrainCdtNodePieceKind::JunctionN,
            owner_kind,
            owner_index,
            boundary_source: Some(TerrainCdtNodeFootprintBoundarySegmentSource {
                start: TerrainCdtNodeFootprintBoundaryVertexSource::Direct(
                    TerrainCdtNodeFootprintBoundaryDirectSource {
                        top_surface_source_index: 7,
                        grade_authority_index: start_grade_authority_index,
                    },
                ),
                end: TerrainCdtNodeFootprintBoundaryVertexSource::Direct(
                    TerrainCdtNodeFootprintBoundaryDirectSource {
                        top_surface_source_index: 7,
                        grade_authority_index: end_grade_authority_index,
                    },
                ),
            }),
        }
    }

    fn test_span_boundary_source(
        edge_idx: u64,
        band_kind: TerrainCdtRoadBandKind,
        source_band_index: u32,
    ) -> TerrainCdtRoadBoundarySource {
        test_span_boundary_source_range(edge_idx, band_kind, source_band_index, 3, 4, 12.0, 16.0)
    }

    fn test_span_boundary_source_range(
        edge_idx: u64,
        band_kind: TerrainCdtRoadBandKind,
        source_band_index: u32,
        start_section_index: u32,
        end_section_index: u32,
        start_s_m: f32,
        end_s_m: f32,
    ) -> TerrainCdtRoadBoundarySource {
        TerrainCdtRoadBoundarySource::SpanSupportBoundary {
            edge_idx,
            edge_class: TerrainCdtEdgeClass::Standard,
            support_policy: TerrainCdtEarthworkSupportPolicy::StandardFullGroundedSpan,
            source_band_index,
            band_kind,
            role: match band_kind {
                TerrainCdtRoadBandKind::Carriageway => TerrainCdtSpanRegionRole::Asphalt,
                TerrainCdtRoadBandKind::CurbOrShoulder => TerrainCdtSpanRegionRole::CurbOrShoulder,
                _ => TerrainCdtSpanRegionRole::NonRoad,
            },
            start_section_index,
            end_section_index,
            start_s_m,
            end_s_m,
        }
    }

    fn diagonal_road_loop() -> Vec<TerrainCdtVertex> {
        road_loop_from_centerline(
            TerrainCdtVertex::new(8.0, 0.0, 12.0),
            TerrainCdtVertex::new(32.0, 0.0, 28.0),
            6.0,
        )
    }

    fn road_loop_from_centerline(
        start: TerrainCdtVertex,
        end: TerrainCdtVertex,
        width: f64,
    ) -> Vec<TerrainCdtVertex> {
        let dx = end.x - start.x;
        let dz = end.z - start.z;
        let length = (dx * dx + dz * dz).sqrt();
        let normal_x = -dz / length;
        let normal_z = dx / length;
        let half_width = width * 0.5;
        let mut road = vec![
            TerrainCdtVertex::new(
                start.x + normal_x * half_width,
                0.0,
                start.z + normal_z * half_width,
            ),
            TerrainCdtVertex::new(
                end.x + normal_x * half_width,
                0.0,
                end.z + normal_z * half_width,
            ),
            TerrainCdtVertex::new(
                end.x - normal_x * half_width,
                0.0,
                end.z - normal_z * half_width,
            ),
            TerrainCdtVertex::new(
                start.x - normal_x * half_width,
                0.0,
                start.z - normal_z * half_width,
            ),
        ];
        if signed_area(&road) < 0.0 {
            road.reverse();
        }
        road
    }

    fn road_loop_from_centerline_with_heights(
        start: TerrainCdtVertex,
        end: TerrainCdtVertex,
        width: f64,
    ) -> Vec<TerrainCdtVertex> {
        let dx = end.x - start.x;
        let dz = end.z - start.z;
        let length = (dx * dx + dz * dz).sqrt();
        let normal_x = -dz / length;
        let normal_z = dx / length;
        let half_width = width * 0.5;
        let mut road = vec![
            TerrainCdtVertex::new(
                start.x + normal_x * half_width,
                start.height_m,
                start.z + normal_z * half_width,
            ),
            TerrainCdtVertex::new(
                end.x + normal_x * half_width,
                end.height_m,
                end.z + normal_z * half_width,
            ),
            TerrainCdtVertex::new(
                end.x - normal_x * half_width,
                end.height_m,
                end.z - normal_z * half_width,
            ),
            TerrainCdtVertex::new(
                start.x - normal_x * half_width,
                start.height_m,
                start.z - normal_z * half_width,
            ),
        ];
        if signed_area(&road) < 0.0 {
            road.reverse();
        }
        road
    }

    fn piece_test_patch() -> TerrainCdtPatch {
        TerrainCdtPatch::new(0.0, 0.0, 60.0, 60.0, [0.0; 4])
    }

    fn test_vertex(x: f64, z: f64) -> TerrainCdtVertex {
        TerrainCdtVertex::new(x, 0.0, z)
    }

    fn build_piece_patch(
        patch: TerrainCdtPatch,
        stable_piece_id: u64,
        road: Vec<TerrainCdtVertex>,
    ) -> TerrainCdtMesh {
        build_road_touched_terrain_patch(TerrainCdtInput::new(
            patch,
            vec![TerrainCdtRoadLoop::new(stable_piece_id, 0, road)],
            piece_source_samples(),
        ))
        .expect("Spade should triangulate a piece-owned road footprint")
    }

    fn piece_source_samples() -> Vec<TerrainCdtVertex> {
        vec![
            test_vertex(6.0, 6.0),
            test_vertex(6.0, 20.0),
            test_vertex(6.0, 40.0),
            test_vertex(6.0, 54.0),
            test_vertex(20.0, 6.0),
            test_vertex(20.0, 54.0),
            test_vertex(40.0, 6.0),
            test_vertex(40.0, 54.0),
            test_vertex(54.0, 6.0),
            test_vertex(54.0, 20.0),
            test_vertex(54.0, 40.0),
            test_vertex(54.0, 54.0),
        ]
    }

    fn assert_road_touched_dem_tie_in_case(
        case_name: &str,
        road: Vec<TerrainCdtVertex>,
        source_samples: Vec<TerrainCdtVertex>,
        expected_widened_source_samples: usize,
        expect_retaining_wall: bool,
    ) {
        let patch = TerrainCdtPatch::new(0.0, 0.0, 10.0, 10.0, [0.0; 4]);
        let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
            patch,
            vec![TerrainCdtRoadLoop::new(17, 0, road.clone())],
            source_samples.clone(),
        ))
        .unwrap_or_else(|_| panic!("{case_name}: terrain CDT should build"));

        let mut reversed_samples = source_samples;
        reversed_samples.reverse();
        let reordered_mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
            patch,
            vec![TerrainCdtRoadLoop::new(17, 0, road.clone())],
            reversed_samples,
        ))
        .unwrap_or_else(|_| panic!("{case_name}: reordered terrain CDT should build"));

        assert_eq!(
            mesh.stats, reordered_mesh.stats,
            "{case_name}: source sample order must not change CDT diagnostics"
        );
        assert_eq!(
            canonical_triangle_set(&mesh.triangles),
            canonical_triangle_set(&reordered_mesh.triangles),
            "{case_name}: ordinary terrain triangles must be deterministic"
        );
        assert_eq!(
            canonical_triangle_set(&mesh.retaining_wall_triangles),
            canonical_triangle_set(&reordered_mesh.retaining_wall_triangles),
            "{case_name}: retaining wall triangles must be deterministic"
        );
        assert_eq!(
            mesh.stats.invalid_constraint_edges, 0,
            "{case_name}: DEM tie-in must not invalidate exact road seam constraints"
        );
        assert_eq!(
            mesh.stats.preserved_road_constraint_edges, mesh.stats.road_constraint_edges,
            "{case_name}: every road seam constraint must survive Spade insertion"
        );
        assert_eq!(
            mesh.stats.accepted_faces,
            mesh.triangles.len() + mesh.retaining_wall_triangles.len(),
            "{case_name}: accepted faces must be fully classified"
        );
        assert_eq!(
            mesh.stats.tie_in_widened_source_samples, expected_widened_source_samples,
            "{case_name}: widened DEM source sample count changed"
        );
        if expected_widened_source_samples == 0 {
            assert!(
                mesh.tie_in_widened_samples.is_empty(),
                "{case_name}: unexpected widened tie-in diagnostics"
            );
        } else {
            assert_eq!(
                mesh.tie_in_widened_samples.len(),
                expected_widened_source_samples.min(MAX_TIE_IN_SAMPLE_DIAGNOSTICS),
                "{case_name}: widened tie-in diagnostics should be capped deterministically"
            );
            assert!(
                mesh.tie_in_widened_samples
                    .iter()
                    .all(|sample| sample.required_distance_m > sample.distance_m),
                "{case_name}: widened samples must prove the ordinary tie-in would exceed budget"
            );
        }

        if expect_retaining_wall {
            assert!(
                mesh.stats.retaining_wall_faces > 0,
                "{case_name}: expected explicit retaining-wall faces"
            );
            assert_eq!(
                mesh.stats.retaining_wall_faces,
                mesh.retaining_wall_triangles.len(),
                "{case_name}: retaining-wall face count must match emitted wall topology"
            );
            assert!(
                mesh.stats.retaining_wall_max_slope_ratio > MAX_TERRAIN_TIE_IN_SLOPE_RATIO,
                "{case_name}: retaining walls must be driven by the documented slope budget"
            );
            assert!(
                mesh.retaining_wall_face_samples
                    .iter()
                    .all(|sample| sample.kind == TerrainCdtTieInKind::RetainingWall),
                "{case_name}: retaining diagnostics must not be ordinary terrain samples"
            );
        } else {
            assert_eq!(
                mesh.stats.retaining_wall_faces, 0,
                "{case_name}: ordinary DEM tie-ins must not emit retaining-wall faces"
            );
            assert!(
                mesh.retaining_wall_triangles.is_empty(),
                "{case_name}: ordinary DEM tie-ins must not emit retaining-wall topology"
            );
            assert!(
                mesh.stats.road_seam_max_slope_ratio <= MAX_TERRAIN_TIE_IN_SLOPE_RATIO + 0.0001,
                "{case_name}: ordinary road seam faces exceeded the slope budget: {:?}",
                mesh.stats
            );
        }

        let road = ensure_ccw(simplified_loop(road));
        for triangle in mesh
            .triangles
            .iter()
            .chain(mesh.retaining_wall_triangles.iter())
        {
            let center = centroid([
                mesh.vertices[triangle[0]],
                mesh.vertices[triangle[1]],
                mesh.vertices[triangle[2]],
            ]);
            assert!(
                !point_in_polygon(center, &road),
                "{case_name}: emitted terrain tie-in leaked into the road-owned footprint"
            );
        }
    }

    fn assert_sourced_road_touched_mesh_contract(
        case_name: &str,
        mesh: &TerrainCdtMesh,
        patch: TerrainCdtPatch,
        road_loops: &[Vec<TerrainCdtVertex>],
        expected_source: TerrainCdtRoadBoundarySource,
    ) {
        assert!(
            !mesh.emitted_faces.is_empty(),
            "{case_name}: terrain CDT should emit accepted terrain topology"
        );
        assert_eq!(
            mesh.stats.invalid_constraint_edges, 0,
            "{case_name}: authored DEM must not create invalid road constraints"
        );
        assert_eq!(
            mesh.stats.preserved_road_constraint_edges, mesh.stats.road_constraint_edges,
            "{case_name}: road seam constraints must survive triangulation"
        );
        assert_eq!(
            mesh.stats.accepted_faces,
            mesh.triangles.len() + mesh.retaining_wall_triangles.len(),
            "{case_name}: every accepted face must be projected into one emitted bucket"
        );
        assert_eq!(
            mesh.emitted_faces.len(),
            mesh.stats.accepted_faces,
            "{case_name}: first-class emitted faces must cover every accepted face"
        );
        assert_eq!(
            mesh.terrain_triangle_sources.len(),
            mesh.triangles.len(),
            "{case_name}: terrain triangle source sidecars must match terrain triangles"
        );
        assert_eq!(
            mesh.retaining_wall_triangle_sources.len(),
            mesh.retaining_wall_triangles.len(),
            "{case_name}: retaining-wall source sidecars must match retaining-wall triangles"
        );
        assert!(
            mesh.stats.road_seam_faces > 0,
            "{case_name}: road-touched terrain should report sourced road-seam faces"
        );
        assert!(
            mesh.road_seam_face_samples
                .iter()
                .all(|sample| sample.sources.contains(&expected_source)),
            "{case_name}: road-seam diagnostics must name the source owner"
        );
        assert!(
            mesh.retaining_wall_face_samples
                .iter()
                .all(|sample| sample.kind == TerrainCdtTieInKind::RetainingWall
                    && sample.sources.contains(&expected_source)),
            "{case_name}: retaining-wall diagnostics must name the source owner"
        );
        assert!(
            mesh.retaining_wall_triangle_sources
                .iter()
                .all(|sources| sources.contains(&expected_source)),
            "{case_name}: emitted retaining-wall faces must carry structured source provenance"
        );
        assert!(
            mesh.emitted_faces.iter().all(|face| {
                if face.kind == TerrainCdtTieInKind::RetainingWall {
                    face.sources.contains(&expected_source)
                } else {
                    true
                }
            }),
            "{case_name}: first-class retaining-wall faces must not be anonymous"
        );

        for source in mesh
            .emitted_faces
            .iter()
            .flat_map(|face| face.sources.iter().copied())
            .chain(
                mesh.road_seam_face_samples
                    .iter()
                    .flat_map(|sample| sample.sources.iter().copied()),
            )
            .chain(
                mesh.retaining_wall_face_samples
                    .iter()
                    .flat_map(|sample| sample.sources.iter().copied()),
            )
        {
            assert_source_exports_structured_provenance(case_name, source);
        }

        let clipped_roads = road_loops
            .iter()
            .filter_map(|road| {
                let clipped = ensure_ccw(simplified_loop(clip_loop_to_patch(road.clone(), patch)));
                (clipped.len() >= 3).then_some(clipped)
            })
            .collect::<Vec<_>>();
        for triangle in mesh
            .triangles
            .iter()
            .chain(mesh.retaining_wall_triangles.iter())
        {
            let center = centroid([
                mesh.vertices[triangle[0]],
                mesh.vertices[triangle[1]],
                mesh.vertices[triangle[2]],
            ]);
            assert!(
                clipped_roads
                    .iter()
                    .all(|road| !point_in_polygon(center, road)),
                "{case_name}: emitted terrain tie-in leaked into a road-owned footprint"
            );
        }
    }

    fn assert_source_exports_structured_provenance(
        case_name: &str,
        source: TerrainCdtRoadBoundarySource,
    ) {
        assert!(
            !source.debug_label().is_empty(),
            "{case_name}: source should retain a human debug label"
        );
        match source {
            TerrainCdtRoadBoundarySource::SpanSupportBoundary {
                support_policy,
                start_section_index,
                end_section_index,
                start_s_m,
                end_s_m,
                ..
            } => {
                assert_eq!(
                    source.source_kind_code(),
                    0,
                    "{case_name}: span support source kind code changed"
                );
                assert!(source.primary_id_code() >= 0);
                assert!(source.edge_class_code() >= 0);
                assert!(source.owner_kind_code() >= 0);
                assert!(source.owner_index_code() >= 0);
                assert!(source.role_code() >= 0);
                assert!(source.support_policy_code() >= 0);
                assert_eq!(
                    support_policy,
                    TerrainCdtEarthworkSupportPolicy::StandardFullGroundedSpan
                );
                assert!(end_section_index >= start_section_index);
                assert!(end_s_m >= start_s_m);
                assert_eq!(
                    source.section_range_codes(),
                    [
                        i32::try_from(start_section_index).unwrap(),
                        i32::try_from(end_section_index).unwrap()
                    ]
                );
                assert_eq!(source.s_range_values(), [start_s_m, end_s_m]);
            }
            TerrainCdtRoadBoundarySource::NodeFootprintBoundary { owner_index, .. } => {
                assert_eq!(
                    source.source_kind_code(),
                    1,
                    "{case_name}: node footprint source kind code changed"
                );
                assert!(source.primary_id_code() >= 0);
                assert!(source.node_kind_code() >= 0);
                assert!(source.owner_kind_code() >= 0);
                assert_eq!(
                    source.owner_index_code(),
                    i32::try_from(owner_index).unwrap()
                );
                assert_eq!(source.support_policy_code(), -1);
                assert_eq!(source.role_code(), -1);
                assert_eq!(source.section_range_codes(), [-1, -1]);
                assert_eq!(source.s_range_values(), [-1.0, -1.0]);
            }
            TerrainCdtRoadBoundarySource::SyntheticTestBoundary { .. } => {
                panic!("{case_name}: source-preserving validation must not use synthetic sources")
            }
        }
    }

    fn authored_dem_samples(
        patch: TerrainCdtPatch,
        step_m: f64,
        height_at: fn(f64, f64) -> f32,
    ) -> Vec<TerrainCdtVertex> {
        let mut samples = Vec::new();
        let mut z = patch.min_z;
        while z <= patch.max_z + CDT_EPSILON_M {
            let mut x = patch.min_x;
            while x <= patch.max_x + CDT_EPSILON_M {
                samples.push(TerrainCdtVertex::new(x, height_at(x, z), z));
                x += step_m;
            }
            z += step_m;
        }
        samples
    }

    fn authored_cross_slope_height(x: f64, z: f64) -> f32 {
        (x * 0.16 + z * 0.02 - 3.6) as f32
    }

    fn authored_along_slope_height(x: f64, _z: f64) -> f32 {
        ((x - 20.0) * 0.22) as f32
    }

    fn authored_ridge_valley_height(x: f64, z: f64) -> f32 {
        let ridge_dx = x - 20.0;
        let valley_dz = z - 25.0;
        let ridge = 3.5 * (-(ridge_dx * ridge_dx) / (2.0 * 5.0 * 5.0)).exp();
        let valley = -2.2 * (-(valley_dz * valley_dz) / (2.0 * 7.0 * 7.0)).exp();
        (ridge + valley) as f32
    }

    fn square_road_loop(min: f64, max: f64, height_m: f32) -> Vec<TerrainCdtVertex> {
        vec![
            TerrainCdtVertex::new(min, height_m, min),
            TerrainCdtVertex::new(max, height_m, min),
            TerrainCdtVertex::new(max, height_m, max),
            TerrainCdtVertex::new(min, height_m, max),
        ]
    }

    fn build_crossing_patch(patch: TerrainCdtPatch, road: Vec<TerrainCdtVertex>) -> TerrainCdtMesh {
        build_road_touched_terrain_patch(TerrainCdtInput::new(
            patch,
            vec![TerrainCdtRoadLoop::new(7, 0, road)],
            vec![
                TerrainCdtVertex::new(5.0, 0.0, 5.0),
                TerrainCdtVertex::new(5.0, 0.0, 35.0),
                TerrainCdtVertex::new(20.0, 0.0, 5.0),
                TerrainCdtVertex::new(20.0, 0.0, 35.0),
                TerrainCdtVertex::new(35.0, 0.0, 5.0),
                TerrainCdtVertex::new(35.0, 0.0, 35.0),
            ],
        ))
        .expect("Spade should triangulate a clipped road footprint")
    }

    fn assert_valid_clipped_mesh(
        mesh: &TerrainCdtMesh,
        patch: TerrainCdtPatch,
        original_road: &[TerrainCdtVertex],
    ) {
        let clipped_road = ensure_ccw(simplified_loop(clip_loop_to_patch(
            original_road.to_vec(),
            patch,
        )));
        assert!(clipped_road.len() >= 3);
        assert!(!mesh.triangles.is_empty());
        assert!(mesh.stats.rejected_road_faces > 0);
        assert_eq!(mesh.stats.invalid_constraint_edges, 0);
        assert_eq!(
            mesh.stats.preserved_road_constraint_edges,
            mesh.stats.road_constraint_edges
        );
        for vertex in &mesh.vertices {
            assert!(patch_contains(*vertex, patch));
        }
        for triangle in &mesh.triangles {
            let center = centroid([
                mesh.vertices[triangle[0]],
                mesh.vertices[triangle[1]],
                mesh.vertices[triangle[2]],
            ]);
            assert!(
                !point_in_polygon(center, &clipped_road),
                "accepted terrain triangle leaked into the clipped road footprint"
            );
        }
    }

    fn assert_valid_piece_footprint_mesh(
        mesh: &TerrainCdtMesh,
        patch: TerrainCdtPatch,
        road: &[TerrainCdtVertex],
    ) {
        let road = ensure_ccw(simplified_loop(road.to_vec()));
        assert!(road.len() >= 3);
        assert!(!mesh.triangles.is_empty());
        assert_eq!(mesh.stats.road_constraint_edges, road.len());
        assert_eq!(mesh.stats.invalid_constraint_edges, 0);
        assert!(mesh.stats.rejected_road_faces > 0);
        assert_eq!(
            mesh.stats.preserved_road_constraint_edges,
            mesh.stats.road_constraint_edges
        );
        for vertex in &mesh.vertices {
            assert!(patch_contains(*vertex, patch));
        }
        for triangle in &mesh.triangles {
            let center = centroid([
                mesh.vertices[triangle[0]],
                mesh.vertices[triangle[1]],
                mesh.vertices[triangle[2]],
            ]);
            assert!(
                !point_in_polygon(center, &road),
                "accepted terrain triangle leaked into a piece-owned road footprint"
            );
        }
    }

    fn canonical_triangle_set(triangles: &[[usize; 3]]) -> Vec<[usize; 3]> {
        let mut canonical = triangles
            .iter()
            .map(|triangle| {
                let mut sorted = *triangle;
                sorted.sort_unstable();
                sorted
            })
            .collect::<Vec<_>>();
        canonical.sort_unstable();
        canonical
    }
}
