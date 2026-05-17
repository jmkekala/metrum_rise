//! Core node-height records, keys, authority types, and scalar quantization.

use super::*;

pub(super) const HEIGHT_SOURCE_KEY_SCALE: f64 = SURFACE_XZ_KEY_SCALE;
// Source-edge handoff may absorb only project point-dedup drift, not a general near-edge search.
pub(super) const HEIGHT_SOURCE_EDGE_DEDUP_DRIFT_UNITS: i128 =
    (WORLD_POINT_DEDUP_DISTANCE_M as f64 * HEIGHT_SOURCE_KEY_SCALE + 0.5) as i128;
pub(super) type NodeHeightedContour = Vec<NodeHeightedVertex>;
pub(super) type NodeHeightedShape = Vec<NodeHeightedContour>;
pub(super) type NodeHeightSourcePointKey = (i64, i64);

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeHeightSolution {
    pub(crate) node_id: u32,
    pub(crate) piece_kind: RoadSurfaceVisualNodePieceKind,
    pub(crate) regions: Vec<NodeHeightedRegion>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeHeightedRegion {
    pub(crate) kind: RoadSurfaceBandKind,
    pub(crate) owner: NodeBandOwner,
    pub(crate) height_field_id: NodeBandHeightFieldId,
    pub(crate) shape: NodeHeightedShape,
    pub(crate) area_m2: f32,
    pub(crate) seam_constraints: Vec<NodeRegionSeamConstraint>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeHeightedVertex {
    pub(crate) point_xz: RoadVec2,
    pub(crate) height_m: f64,
    pub(crate) height_field_id: NodeBandHeightFieldId,
    pub(crate) height_authority: Option<NodeHeightAuthoritySource>,
    pub(crate) grade_authority: Option<NodeGradeVertexAuthority>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NodeHeightFieldError {
    InputOwnershipMismatch {
        input_node_id: u32,
        ownership_node_id: u32,
        input_piece_kind: RoadSurfaceVisualNodePieceKind,
        ownership_piece_kind: RoadSurfaceVisualNodePieceKind,
    },
    DuplicateSourceBand {
        mouth_order_index: usize,
        band_index: usize,
    },
    MissingRegionBandIndex {
        mouth_order_index: usize,
        kind: RoadSurfaceBandKind,
    },
    MissingSourceBand {
        mouth_order_index: usize,
        band_index: usize,
    },
    SourceBandKindMismatch {
        mouth_order_index: usize,
        band_index: usize,
        region_kind: RoadSurfaceBandKind,
        source_kind: RoadSurfaceBandKind,
    },
    InvalidSourceBandHeightCarrier {
        mouth_order_index: usize,
        band_index: usize,
        source_kind: RoadSurfaceBandKind,
        height_field_id: NodeBandHeightFieldId,
        reason: &'static str,
    },
    MissingGeneratedContourHeightPoints {
        mouth_order_index: usize,
        band_index: usize,
        source_kind: RoadSurfaceBandKind,
        height_field_id: NodeBandHeightFieldId,
        purpose: NodeGeneratedContourPurpose,
        claim_priority: NodeGeneratedContourClaimPriority,
    },
    InvalidHeightCarrierContour {
        mouth_order_index: usize,
        band_index: usize,
        source_kind: RoadSurfaceBandKind,
        height_field_id: NodeBandHeightFieldId,
        authority: NodeHeightAuthoritySource,
        reason: &'static str,
    },
    VertexOutsideHeightField {
        mouth_order_index: usize,
        band_index: usize,
        source_kind: RoadSurfaceBandKind,
        height_field_id: NodeBandHeightFieldId,
        owner: Option<NodeBandOwner>,
        point_x_mm: i64,
        point_z_mm: i64,
        axis: &'static str,
        raw_parameter: f64,
    },
    SourceHeightFieldConflict {
        mouth_order_index: usize,
        band_index: usize,
        source_kind: RoadSurfaceBandKind,
        height_field_id: NodeBandHeightFieldId,
        owner: Option<NodeBandOwner>,
        existing_authority: NodeHeightAuthoritySource,
        incoming_authority: NodeHeightAuthoritySource,
        point_x_mm: i64,
        point_z_mm: i64,
        existing_height_mm: i64,
        incoming_height_mm: i64,
    },
    SharedSourceHeightConflict {
        point_x_mm: i64,
        point_z_mm: i64,
        kind: RoadSurfaceBandKind,
        owner: NodeBandOwner,
        opposite_owner: Option<NodeBandOwner>,
        height_field_id: Option<NodeBandHeightFieldId>,
        incoming_owner: NodeBandOwner,
        incoming_height_field_id: Option<NodeBandHeightFieldId>,
        constraint_index: Option<usize>,
        existing_authority: Option<NodeHeightAuthoritySource>,
        incoming_authority: Option<NodeHeightAuthoritySource>,
        existing_height_mm: i64,
        incoming_height_mm: i64,
    },
    TerminalCapGeneration {
        error: TerminalCapGenerationError,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum NodeHeightAuthoritySource {
    SourceInterval,
    TerminalCap,
    GeneratedContour {
        purpose: NodeGeneratedContourPurpose,
        claim_priority: NodeGeneratedContourClaimPriority,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct NodeHeightPointKey {
    pub(super) x_key: i64,
    pub(super) z_key: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct NodeSourceBandKey {
    pub(super) mouth_order_index: usize,
    pub(super) band_index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct NodeHeightVertexContextKey {
    pub(super) point: NodeHeightPointKey,
    pub(super) owner: NodeBandOwner,
    pub(super) height_field_id: NodeBandHeightFieldId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct NodeResolvedHeightAuthorityKey {
    pub(super) point: NodeHeightPointKey,
    pub(super) owner: NodeBandOwner,
    pub(super) height_field_id: NodeBandHeightFieldId,
    pub(super) claim_priority: NodeGeneratedContourClaimPriority,
}

pub(super) struct NodeResolvedHeightAuthorityMap {
    pub(super) heights_by_key:
        BTreeMap<NodeResolvedHeightAuthorityKey, NodeResolvedHeightAuthority>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct NodeResolvedHeightAuthority {
    pub(super) point_xz: RoadVec2,
    pub(super) height_m: f64,
    pub(super) authority: NodeHeightAuthoritySource,
}

pub(super) struct NodeBandHeightField {
    pub(super) id: NodeBandHeightFieldId,
    pub(super) kind: RoadSurfaceBandKind,
    pub(super) patches: Vec<NodeBandHeightPatch>,
}

pub(super) struct NodeBandHeightPatch {
    pub(super) authority: NodeHeightPatchAuthority,
    pub(super) triangles: Option<Vec<NodeBandHeightTriangle>>,
    pub(super) contour_edges: Option<Vec<NodeBandHeightEdge>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct NodeHeightPatchAuthority {
    pub(super) owner: Option<NodeBandOwner>,
    pub(super) role: NodeHeightPatchAuthorityRole,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NodeHeightPatchAuthorityRole {
    SourceInterval,
    TerminalCap,
    GeneratedContour {
        purpose: NodeGeneratedContourPurpose,
        claim_priority: NodeGeneratedContourClaimPriority,
    },
}

pub(super) struct NodeBandHeightTriangle {
    pub(super) a_xz: RoadVec2,
    pub(super) b_xz: RoadVec2,
    pub(super) c_xz: RoadVec2,
    pub(super) a_height_m: f64,
    pub(super) b_height_m: f64,
    pub(super) c_height_m: f64,
}

pub(super) struct NodeBandHeightEdge {
    pub(super) start_xz: RoadVec2,
    pub(super) end_xz: RoadVec2,
    pub(super) start_height_m: f64,
    pub(super) end_height_m: f64,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct NodeAuthorizedHeightCandidate {
    pub(super) authority_rank: u8,
    pub(super) authority: NodeHeightAuthoritySource,
    pub(super) height_m: f64,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct NodeEvaluatedHeight {
    pub(super) height_m: f64,
    pub(super) authority: NodeHeightAuthoritySource,
}

pub(super) enum NodeHeightPatchEvaluation {
    Inside(f64),
    Outside(NodeHeightFieldError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HeightCarrierContourError {
    TooFewVertices,
    DegenerateContour,
    CdtBuildFailed,
    InvalidConstraint,
    EmptyInteriorTriangulation,
}

impl HeightCarrierContourError {
    pub(super) fn diagnostic_reason(self) -> &'static str {
        match self {
            Self::TooFewVertices => "height_carrier_too_few_vertices",
            Self::DegenerateContour => "height_carrier_degenerate_contour",
            Self::CdtBuildFailed => "height_carrier_cdt_build_failed",
            Self::InvalidConstraint => "height_carrier_invalid_constraint",
            Self::EmptyInteriorTriangulation => "height_carrier_empty_interior_triangulation",
        }
    }
}

pub(super) fn quantize_m(value: f64) -> i64 {
    SurfaceHeightMmKey::from_m_f64(value).as_i64()
}

impl NodeHeightPointKey {
    pub(super) fn from_point(point: RoadVec2) -> Self {
        let key = SurfaceXzKey::from_road_xz(point);
        Self {
            x_key: key.x_key(),
            z_key: key.z_key(),
        }
    }

    pub(super) fn x_mm(self) -> i64 {
        SurfaceXzKey::from_raw_keys(self.x_key, self.z_key).x_mm()
    }

    pub(super) fn z_mm(self) -> i64 {
        SurfaceXzKey::from_raw_keys(self.x_key, self.z_key).z_mm()
    }
}
