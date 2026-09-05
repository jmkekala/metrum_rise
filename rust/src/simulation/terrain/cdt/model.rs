// SPDX-License-Identifier: GPL-2.0-only

//! CDT input, output, provenance, and diagnostic data contracts.

use spade::Point2;

mod ordering;

pub(super) use ordering::*;

pub(super) const CDT_EPSILON_M: f64 = 0.001;
pub(super) const MAX_INVALID_CONSTRAINT_SAMPLES: usize = 8;
pub(super) const MAX_ROAD_SEAM_FACE_SAMPLES: usize = 8;
pub(super) const MAX_SEAM_QUALITY_SAMPLES: usize = 8;
pub(super) const MAX_TIE_IN_SAMPLE_DIAGNOSTICS: usize = 8;
pub(crate) const MAX_TERRAIN_TIE_IN_SLOPE_RATIO: f32 = 0.5;
pub(super) const MIN_TIE_IN_HEIGHT_DELTA_M: f32 = 0.01;
pub(super) const MIN_RETAINING_WALL_TIE_IN_HEIGHT_DELTA_M: f32 = 0.5;
pub(super) const MIN_SOURCE_OWNED_SEAM_EDGE_LENGTH_M: f64 = 0.05;

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

    pub(super) fn point2(self) -> Point2<f64> {
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

    pub(super) fn is_valid(self) -> bool {
        self.max_x > self.min_x + CDT_EPSILON_M && self.max_z > self.min_z + CDT_EPSILON_M
    }

    pub(super) fn corners_cw(self) -> [TerrainCdtVertex; 4] {
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
    #[cfg(test)]
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainCdtTieInGuideSample {
    pub(crate) vertex: TerrainCdtVertex,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainCdtTieInGuideConstraint {
    pub(crate) start: TerrainCdtVertex,
    pub(crate) end: TerrainCdtVertex,
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
    NodeSameMaterialBoundaryHandoff {
        node_id: u32,
        node_kind: TerrainCdtNodePieceKind,
        owner_kind: TerrainCdtRoadBandKind,
        owner_index_a: u32,
        owner_index_b: u32,
        boundary_source: Option<TerrainCdtNodeFootprintBoundarySegmentSource>,
    },
    BuildingSiteBoundary {
        building_idx: u64,
        local_loop_index: u32,
        local_edge_index: u32,
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
            Self::NodeSameMaterialBoundaryHandoff { .. } => 2,
            Self::BuildingSiteBoundary { .. } => 3,
            Self::SyntheticTestBoundary { .. } => 4,
        }
    }

    pub(crate) fn primary_id_code(self) -> i32 {
        match self {
            Self::SpanSupportBoundary { edge_idx, .. } => clamp_u64_to_i32(edge_idx),
            Self::NodeFootprintBoundary { node_id, .. } => clamp_u32_to_i32(node_id),
            Self::NodeSameMaterialBoundaryHandoff { node_id, .. } => clamp_u32_to_i32(node_id),
            Self::BuildingSiteBoundary { building_idx, .. } => clamp_u64_to_i32(building_idx),
            Self::SyntheticTestBoundary {
                stable_piece_id, ..
            } => clamp_u64_to_i32(stable_piece_id),
        }
    }

    pub(crate) fn node_kind_code(self) -> i32 {
        match self {
            Self::NodeFootprintBoundary { node_kind, .. }
            | Self::NodeSameMaterialBoundaryHandoff { node_kind, .. } => {
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
            Self::NodeSameMaterialBoundaryHandoff { owner_kind, .. } => {
                i32::from(terrain_cdt_band_kind_sort_key(owner_kind))
            }
            Self::BuildingSiteBoundary { .. } => -1,
            Self::SyntheticTestBoundary { .. } => -1,
        }
    }

    pub(crate) fn owner_index_code(self) -> i32 {
        match self {
            Self::SpanSupportBoundary {
                source_band_index, ..
            } => clamp_u32_to_i32(source_band_index),
            Self::NodeFootprintBoundary { owner_index, .. } => clamp_u32_to_i32(owner_index),
            Self::NodeSameMaterialBoundaryHandoff { owner_index_a, .. } => {
                clamp_u32_to_i32(owner_index_a)
            }
            Self::BuildingSiteBoundary {
                local_loop_index, ..
            } => clamp_u32_to_i32(local_loop_index),
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
            Self::NodeSameMaterialBoundaryHandoff {
                node_id,
                node_kind,
                owner_kind,
                owner_index_a,
                owner_index_b,
                boundary_source,
            } => format!(
                "node_same_material_handoff id={} kind={} owner_kind={} owner_indices={}..{} boundary_source={:?}",
                node_id,
                terrain_cdt_node_kind_label(node_kind),
                terrain_cdt_band_kind_label(owner_kind),
                owner_index_a,
                owner_index_b,
                boundary_source
            ),
            Self::BuildingSiteBoundary {
                building_idx,
                local_loop_index,
                local_edge_index,
            } => format!(
                "building_site building={} loop={} edge={}",
                building_idx, local_loop_index, local_edge_index
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
    pub(crate) tie_in_guide_samples: Vec<TerrainCdtTieInGuideSample>,
    pub(crate) tie_in_guide_constraints: Vec<TerrainCdtTieInGuideConstraint>,
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
            tie_in_guide_samples: Vec::new(),
            tie_in_guide_constraints: Vec::new(),
        }
    }

    pub(crate) fn with_tie_in_guide_samples(
        mut self,
        tie_in_guide_samples: Vec<TerrainCdtTieInGuideSample>,
    ) -> Self {
        self.tie_in_guide_samples = tie_in_guide_samples;
        self
    }

    pub(crate) fn with_tie_in_guide_constraints(
        mut self,
        tie_in_guide_constraints: Vec<TerrainCdtTieInGuideConstraint>,
    ) -> Self {
        self.tie_in_guide_constraints = tie_in_guide_constraints;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TerrainCdtMesh {
    pub(crate) vertices: Vec<TerrainCdtVertex>,
    #[cfg(test)]
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
    pub(crate) unpreserved_road_constraint_samples: Vec<TerrainCdtInvalidConstraintSample>,
}

#[cfg(test)]
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
    pub(crate) building_site_constraint_edges: usize,
    pub(crate) accepted_faces: usize,
    pub(crate) rejected_road_faces: usize,
    pub(crate) preserved_road_constraint_edges: usize,
    pub(crate) preserved_building_site_constraint_edges: usize,
    pub(crate) spade_missing_road_constraint_edges: usize,
    pub(crate) rejected_road_constraint_edges: usize,
    pub(crate) internal_road_constraint_edges: usize,
    pub(crate) invalid_constraint_edges: usize,
    pub(crate) max_face_y_delta_m: f32,
    pub(crate) max_face_slope_ratio: f32,
    pub(crate) longest_triangle_edge_m: f32,
    pub(crate) road_seam_faces: usize,
    pub(crate) road_seam_max_y_delta_m: f32,
    pub(crate) road_seam_max_slope_ratio: f32,
    pub(crate) retaining_wall_faces: usize,
    pub(crate) retaining_wall_max_y_delta_m: f32,
    pub(crate) retaining_wall_max_slope_ratio: f32,
    pub(crate) accepted_seam_edges: usize,
    pub(crate) merged_subbudget_seam_edges: usize,
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

#[cfg(test)]
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
    ConflictingRoadBoundaryHeight,
    TriangulationFailed,
}
