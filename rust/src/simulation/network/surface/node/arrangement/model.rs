//! Core arrangement identifiers, keys, records, and accessors.

use super::super::backend::RoadVec2;
use super::super::height::{NodeGradeVertexAuthority, NodeHeightCarrierProvenanceKey};
use super::super::keys::SurfaceXzKey;
use super::super::segments::arrangement_key_lies_on_segment;
use super::super::{RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind};
use super::seams::{NodeRegionSeamConstraint, NodeSeamSource};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct NodeArrangementVertexId(pub(super) usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct NodeArrangementEdgeId(pub(super) usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct NodeOwnedRegionId(pub(super) usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct NodeArrangementFaceId(pub(super) usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct NodeArrangementKey {
    pub(super) x_key: i64,
    pub(super) z_key: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(super) struct NodeArrangementHeightKey(pub(super) i64);

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct NodeArrangementVertexContextKey {
    pub(super) position: NodeArrangementKey,
    pub(super) owners: Vec<NodeBandOwner>,
    pub(super) height_field_id: NodeBandHeightFieldId,
    pub(super) source_provenance: Option<NodeHeightCarrierProvenanceKey>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct NodeBandOwner {
    pub(super) kind: RoadSurfaceBandKind,
    pub(super) owner_index: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct NodeBandHeightFieldId {
    pub(super) mouth_order_index: usize,
    pub(super) band_index: usize,
    pub(super) kind: RoadSurfaceBandKind,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeArrangementVertex {
    pub(super) id: NodeArrangementVertexId,
    pub(super) key: NodeArrangementKey,
    pub(super) point_xz: RoadVec2,
    pub(super) height_m: f64,
    pub(super) height_key: NodeArrangementHeightKey,
    pub(super) owners: Vec<NodeBandOwner>,
    pub(super) height_field_id: NodeBandHeightFieldId,
    pub(super) seam_sources: Vec<NodeSeamSource>,
    pub(super) grade_authority: NodeGradeVertexAuthority,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeArrangementEdge {
    pub(super) id: NodeArrangementEdgeId,
    pub(super) start: NodeArrangementVertexId,
    pub(super) end: NodeArrangementVertexId,
    pub(super) owner: NodeBandOwner,
    pub(super) height_field_id: NodeBandHeightFieldId,
    pub(super) opposite_owner: Option<NodeBandOwner>,
    pub(super) opposite_height_field_id: Option<NodeBandHeightFieldId>,
    pub(super) exposed_boundary: bool,
    pub(super) constrains_shared_height: bool,
    pub(super) is_material_transition: bool,
    pub(super) has_applicable_material_source_constraint: bool,
    pub(super) seam_source: NodeSeamSource,
    pub(super) source_constraint_indices: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeOwnedRegion {
    pub(super) id: NodeOwnedRegionId,
    pub(super) owner: NodeBandOwner,
    pub(super) height_field_id: NodeBandHeightFieldId,
    pub(super) outer_loop: Vec<NodeArrangementVertexId>,
    pub(super) holes: Vec<Vec<NodeArrangementVertexId>>,
    pub(super) boundary_edges: Vec<NodeArrangementEdgeId>,
    pub(super) area_m2: f32,
    pub(super) seam_constraints: Vec<NodeRegionSeamConstraint>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeArrangementFace {
    pub(super) id: NodeArrangementFaceId,
    pub(super) region: NodeOwnedRegionId,
    pub(super) owner: NodeBandOwner,
    pub(super) vertices: [NodeArrangementVertexId; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeArrangement {
    pub(super) node_id: u32,
    pub(super) piece_kind: RoadSurfaceVisualNodePieceKind,
    pub(super) vertices: Vec<NodeArrangementVertex>,
    pub(super) edges: Vec<NodeArrangementEdge>,
    pub(super) regions: Vec<NodeOwnedRegion>,
    pub(super) faces: Vec<NodeArrangementFace>,
    pub(super) diagnostics: Vec<NodeArrangementDiagnostic>,
    pub(super) vertex_by_context_key:
        BTreeMap<NodeArrangementVertexContextKey, NodeArrangementVertexId>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct NodeArrangementBuildProfile {
    pub(crate) total_ms: f64,
    pub(crate) pending_regions_ms: f64,
    pub(crate) noding_ms: f64,
    pub(crate) edge_support_ms: f64,
    pub(crate) boundary_edges_ms: f64,
    pub(crate) push_regions_ms: f64,
    pub(crate) conflict_ms: f64,
    pub(crate) height_regions: usize,
    pub(crate) pending_edges_before: usize,
    pub(crate) pending_edges_after: usize,
    pub(crate) vertices: usize,
    pub(crate) edges: usize,
    pub(crate) regions: usize,
    pub(crate) seam_constraints: usize,
    pub(crate) diagnostics: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct NodeArrangementAttachProfile {
    pub(crate) total_ms: f64,
    pub(crate) validation_ms: f64,
    pub(crate) insert_vertices_ms: f64,
    pub(crate) push_faces_ms: f64,
    pub(crate) conflict_ms: f64,
    pub(crate) regions: usize,
    pub(crate) source_vertices: usize,
    pub(crate) source_triangles: usize,
    pub(crate) vertex_insert_attempts: usize,
    pub(crate) arrangement_vertices_before: usize,
    pub(crate) arrangement_vertices_after: usize,
    pub(crate) vertices_inserted: usize,
    pub(crate) vertices_reused: usize,
    pub(crate) faces_pushed: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum NodeArrangementDiagnostic {
    MissingSeamConstraint {
        region_index: usize,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
        start: NodeArrangementKey,
        end: NodeArrangementKey,
    },
    AmbiguousSeamConstraint {
        region_index: usize,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
        start: NodeArrangementKey,
        end: NodeArrangementKey,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum NodeArrangementError {
    InputSolutionMismatch {
        height_node_id: u32,
        triangulation_node_id: u32,
        height_piece_kind: RoadSurfaceVisualNodePieceKind,
        triangulation_piece_kind: RoadSurfaceVisualNodePieceKind,
    },
    MissingHeightRegion {
        region_index: usize,
    },
    TriangulationRegionCountMismatch {
        arrangement_region_count: usize,
        triangulation_region_count: usize,
    },
    RegionOwnerMismatch {
        region_index: usize,
    },
    DegenerateRegionContour {
        region_index: usize,
        contour_index: usize,
    },
    MissingTriangulatedVertex {
        region_index: usize,
        vertex_index: usize,
    },
    EmptyOwnerSet {
        key: NodeArrangementKey,
    },
    DuplicateVertexHeightConflict {
        key: NodeArrangementKey,
        existing_height_mm: i64,
        incoming_height_mm: i64,
    },
    MissingGradeAuthority {
        region_index: usize,
        contour_index: usize,
        key: NodeArrangementKey,
        owner: NodeBandOwner,
        height_field_id: NodeBandHeightFieldId,
        height_mm: i64,
    },
}

impl NodeArrangementKey {
    pub(crate) fn from_point(point: RoadVec2) -> Self {
        Self::from_surface_key(SurfaceXzKey::from_road_xz(point))
    }

    pub(in crate::simulation::network::surface::node) fn from_surface_key(
        key: SurfaceXzKey,
    ) -> Self {
        Self {
            x_key: key.x_key(),
            z_key: key.z_key(),
        }
    }

    pub(crate) fn x_key(self) -> i64 {
        self.x_key
    }

    pub(crate) fn z_key(self) -> i64 {
        self.z_key
    }

    pub(crate) fn x_mm(self) -> i64 {
        SurfaceXzKey::from_raw_keys(self.x_key, self.z_key).x_mm()
    }

    pub(crate) fn z_mm(self) -> i64 {
        SurfaceXzKey::from_raw_keys(self.x_key, self.z_key).z_mm()
    }

    pub(super) fn surface_key(self) -> SurfaceXzKey {
        SurfaceXzKey::from_raw_keys(self.x_key, self.z_key)
    }

    pub(crate) fn lies_on_segment(self, start: Self, end: Self) -> bool {
        arrangement_key_lies_on_segment(self, start, end)
    }
}

impl NodeBandOwner {
    pub(crate) fn new(kind: RoadSurfaceBandKind, owner_index: usize) -> Self {
        Self { kind, owner_index }
    }

    pub(crate) fn kind(self) -> RoadSurfaceBandKind {
        self.kind
    }

    pub(crate) fn owner_index(self) -> usize {
        self.owner_index
    }
}

impl NodeBandHeightFieldId {
    pub(crate) fn new(
        mouth_order_index: usize,
        band_index: usize,
        kind: RoadSurfaceBandKind,
    ) -> Self {
        Self {
            mouth_order_index,
            band_index,
            kind,
        }
    }

    pub(crate) fn mouth_order_index(self) -> usize {
        self.mouth_order_index
    }

    pub(crate) fn band_index(self) -> usize {
        self.band_index
    }

    pub(crate) fn kind(self) -> RoadSurfaceBandKind {
        self.kind
    }
}

impl NodeArrangement {
    pub(crate) fn new(node_id: u32, piece_kind: RoadSurfaceVisualNodePieceKind) -> Self {
        Self {
            node_id,
            piece_kind,
            vertices: Vec::new(),
            edges: Vec::new(),
            regions: Vec::new(),
            faces: Vec::new(),
            diagnostics: Vec::new(),
            vertex_by_context_key: BTreeMap::new(),
        }
    }

    pub(crate) fn node_id(&self) -> u32 {
        self.node_id
    }

    pub(crate) fn piece_kind(&self) -> RoadSurfaceVisualNodePieceKind {
        self.piece_kind
    }

    pub(crate) fn vertices(&self) -> &[NodeArrangementVertex] {
        &self.vertices
    }

    pub(crate) fn edges(&self) -> &[NodeArrangementEdge] {
        &self.edges
    }

    pub(crate) fn regions(&self) -> &[NodeOwnedRegion] {
        &self.regions
    }

    pub(crate) fn faces(&self) -> &[NodeArrangementFace] {
        &self.faces
    }

    pub(crate) fn diagnostics(&self) -> &[NodeArrangementDiagnostic] {
        &self.diagnostics
    }
}

impl NodeArrangementVertexId {
    pub(crate) fn index(self) -> usize {
        self.0
    }
}

impl NodeArrangementEdgeId {
    #[cfg(test)]
    pub(crate) fn index(self) -> usize {
        self.0
    }
}

impl NodeOwnedRegionId {
    pub(crate) fn index(self) -> usize {
        self.0
    }
}

impl NodeArrangementVertex {
    pub(crate) fn key(&self) -> NodeArrangementKey {
        self.key
    }

    pub(crate) fn point_xz(&self) -> RoadVec2 {
        self.point_xz
    }

    pub(crate) fn height_m(&self) -> f64 {
        self.height_m
    }

    pub(crate) fn height_mm(&self) -> i64 {
        self.height_key.0
    }

    pub(crate) fn height_field_id(&self) -> NodeBandHeightFieldId {
        self.height_field_id
    }

    pub(crate) fn owners(&self) -> &[NodeBandOwner] {
        &self.owners
    }

    pub(crate) fn grade_authority(&self) -> NodeGradeVertexAuthority {
        self.grade_authority
    }
}

impl NodeArrangementEdge {
    pub(crate) fn start(&self) -> NodeArrangementVertexId {
        self.start
    }

    pub(crate) fn end(&self) -> NodeArrangementVertexId {
        self.end
    }

    pub(crate) fn exposed_boundary(&self) -> bool {
        self.exposed_boundary
    }

    pub(crate) fn owner(&self) -> NodeBandOwner {
        self.owner
    }

    pub(crate) fn height_field_id(&self) -> NodeBandHeightFieldId {
        self.height_field_id
    }
}

impl NodeOwnedRegion {
    pub(crate) fn owner(&self) -> NodeBandOwner {
        self.owner
    }

    pub(crate) fn height_field_id(&self) -> NodeBandHeightFieldId {
        self.height_field_id
    }

    pub(crate) fn outer_loop(&self) -> &[NodeArrangementVertexId] {
        &self.outer_loop
    }

    pub(crate) fn holes(&self) -> &[Vec<NodeArrangementVertexId>] {
        &self.holes
    }

    #[cfg(test)]
    pub(crate) fn boundary_edges(&self) -> &[NodeArrangementEdgeId] {
        &self.boundary_edges
    }

    pub(crate) fn area_m2(&self) -> f32 {
        self.area_m2
    }
}

impl NodeArrangementFace {
    pub(crate) fn region(&self) -> NodeOwnedRegionId {
        self.region
    }

    pub(crate) fn owner(&self) -> NodeBandOwner {
        self.owner
    }

    pub(crate) fn vertices(&self) -> [NodeArrangementVertexId; 3] {
        self.vertices
    }
}
