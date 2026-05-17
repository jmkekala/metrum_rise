//! Canonical node-arrangement identity and ownership data model.

mod build;
mod seams;
mod steps;

use super::backend::RoadVec2;
use super::grade::NodeGradeVertexAuthority;
use super::keys::SurfaceXzKey;
use super::segments::arrangement_key_lies_on_segment;
use super::{RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind};
use std::collections::BTreeMap;

pub(crate) use seams::{NodeRegionSeamConstraint, NodeSeamSource, seam_constraints_are_ambiguous};
pub(crate) use steps::{NodeExplicitVerticalStepSegment, owners_form_explicit_vertical_step_pair};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct NodeArrangementVertexId(usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct NodeArrangementEdgeId(usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct NodeOwnedRegionId(usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct NodeArrangementFaceId(usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct NodeArrangementKey {
    x_key: i64,
    z_key: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct NodeArrangementHeightKey(i64);

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct NodeArrangementVertexContextKey {
    position: NodeArrangementKey,
    owners: Vec<NodeBandOwner>,
    height_field_id: NodeBandHeightFieldId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct NodeBandOwner {
    kind: RoadSurfaceBandKind,
    owner_index: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct NodeBandHeightFieldId {
    mouth_order_index: usize,
    band_index: usize,
    kind: RoadSurfaceBandKind,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeArrangementVertex {
    id: NodeArrangementVertexId,
    key: NodeArrangementKey,
    point_xz: RoadVec2,
    height_m: f64,
    height_key: NodeArrangementHeightKey,
    owners: Vec<NodeBandOwner>,
    height_field_id: NodeBandHeightFieldId,
    seam_sources: Vec<NodeSeamSource>,
    grade_authority: NodeGradeVertexAuthority,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeArrangementEdge {
    id: NodeArrangementEdgeId,
    start: NodeArrangementVertexId,
    end: NodeArrangementVertexId,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
    opposite_owner: Option<NodeBandOwner>,
    opposite_height_field_id: Option<NodeBandHeightFieldId>,
    exposed_boundary: bool,
    constrains_shared_height: bool,
    is_material_transition: bool,
    seam_source: NodeSeamSource,
    source_constraint_indices: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeOwnedRegion {
    id: NodeOwnedRegionId,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
    outer_loop: Vec<NodeArrangementVertexId>,
    holes: Vec<Vec<NodeArrangementVertexId>>,
    boundary_edges: Vec<NodeArrangementEdgeId>,
    area_m2: f32,
    seam_constraints: Vec<NodeRegionSeamConstraint>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeArrangementFace {
    id: NodeArrangementFaceId,
    region: NodeOwnedRegionId,
    owner: NodeBandOwner,
    vertices: [NodeArrangementVertexId; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeArrangement {
    node_id: u32,
    piece_kind: RoadSurfaceVisualNodePieceKind,
    vertices: Vec<NodeArrangementVertex>,
    edges: Vec<NodeArrangementEdge>,
    regions: Vec<NodeOwnedRegion>,
    faces: Vec<NodeArrangementFace>,
    diagnostics: Vec<NodeArrangementDiagnostic>,
    vertex_by_context_key: BTreeMap<NodeArrangementVertexContextKey, NodeArrangementVertexId>,
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

    fn from_surface_key(key: SurfaceXzKey) -> Self {
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

    fn surface_key(self) -> SurfaceXzKey {
        SurfaceXzKey::from_raw_keys(self.x_key, self.z_key)
    }

    fn lies_on_segment(self, start: Self, end: Self) -> bool {
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

    pub(crate) fn grade_authority(&self) -> NodeGradeVertexAuthority {
        self.grade_authority
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

#[cfg(test)]
mod tests {
    use super::super::grade::NodeGradeCarrierDecision;
    use super::super::height::{NodeHeightSolution, NodeHeightedRegion, NodeHeightedVertex};
    use super::*;

    fn owner(kind: RoadSurfaceBandKind, owner_index: usize) -> NodeBandOwner {
        NodeBandOwner::new(kind, owner_index)
    }

    fn height_field_id(kind: RoadSurfaceBandKind, band_index: usize) -> NodeBandHeightFieldId {
        NodeBandHeightFieldId::new(0, band_index, kind)
    }

    fn seam_source(owner_index: usize) -> NodeSeamSource {
        NodeSeamSource::FootprintBoundary { owner_index }
    }

    #[test]
    fn duplicate_arrangement_vertex_key_merges_matching_owner_source_context() {
        let mut arrangement = NodeArrangement::new(42, RoadSurfaceVisualNodePieceKind::JunctionN);
        let point = RoadVec2::new(12.345, -6.789);

        let first = arrangement
            .insert_vertex(
                point,
                3.25,
                [owner(RoadSurfaceBandKind::Sidewalk, 2)],
                height_field_id(RoadSurfaceBandKind::Sidewalk, 2),
                [seam_source(2)],
            )
            .expect("first vertex should insert");
        let second = arrangement
            .insert_vertex(
                point,
                3.25,
                [owner(RoadSurfaceBandKind::Sidewalk, 2)],
                height_field_id(RoadSurfaceBandKind::Sidewalk, 2),
                [seam_source(2)],
            )
            .expect("matching context vertex should merge");

        assert_eq!(first, second);
        let vertex = &arrangement.vertices()[first.0];
        assert_eq!(vertex.owners, vec![owner(RoadSurfaceBandKind::Sidewalk, 2)]);
        assert_eq!(vertex.seam_sources, vec![seam_source(2)]);
    }

    #[test]
    fn duplicate_arrangement_vertex_key_merges_same_height_field_and_quantized_height() {
        let mut arrangement = NodeArrangement::new(42, RoadSurfaceVisualNodePieceKind::JunctionN);
        let point = RoadVec2::new(12.345, -6.789);
        let field_id = height_field_id(RoadSurfaceBandKind::Sidewalk, 2);

        let first = arrangement
            .insert_vertex(
                point,
                3.25,
                [owner(RoadSurfaceBandKind::Sidewalk, 2)],
                field_id,
                [seam_source(2)],
            )
            .expect("first vertex should insert");
        let second = arrangement
            .insert_vertex(
                point,
                3.25,
                [owner(RoadSurfaceBandKind::Sidewalk, 2)],
                field_id,
                [NodeSeamSource::SidewalkOuter { owner_index: 2 }],
            )
            .expect("same height-field and solved height should share the canonical vertex");

        assert_eq!(first, second);
        let vertex = &arrangement.vertices()[first.0];
        assert_eq!(vertex.height_field_id(), field_id);
        assert_eq!(
            vertex.seam_sources,
            vec![
                NodeSeamSource::SidewalkOuter { owner_index: 2 },
                seam_source(2)
            ]
        );
    }

    #[test]
    fn duplicate_arrangement_vertex_key_keeps_distinct_material_height_field_contexts() {
        let mut arrangement = NodeArrangement::new(42, RoadSurfaceVisualNodePieceKind::JunctionN);
        let point = RoadVec2::new(12.345, -6.789);

        let first = arrangement
            .insert_vertex(
                point,
                3.25,
                [owner(RoadSurfaceBandKind::Sidewalk, 2)],
                height_field_id(RoadSurfaceBandKind::Sidewalk, 2),
                [seam_source(2)],
            )
            .expect("first vertex should insert");
        let second = arrangement
            .insert_vertex(
                point,
                3.25,
                [owner(RoadSurfaceBandKind::CurbOrShoulder, 1)],
                height_field_id(RoadSurfaceBandKind::CurbOrShoulder, 1),
                [seam_source(1)],
            )
            .expect("distinct height source context should keep its own vertex");

        assert_ne!(first, second);
        assert_eq!(arrangement.vertices().len(), 2);
    }

    #[test]
    fn junctionn_arrangement_vertices_preserve_node_grade_authority() {
        let carriageway = owner(RoadSurfaceBandKind::Carriageway, 0);
        let sidewalk = owner(RoadSurfaceBandKind::Sidewalk, 1);
        let heights = two_region_height_solution(carriageway, sidewalk, Vec::new(), Vec::new());
        let arrangement = NodeArrangement::from_height_solution(&heights)
            .expect("heighted JunctionN should arrange with grade authority");

        assert!(!arrangement.vertices().is_empty());
        assert!(arrangement.vertices().iter().all(|vertex| {
            matches!(
                vertex.grade_authority().decision,
                NodeGradeCarrierDecision::SourceCarrier { .. }
            )
        }));
    }

    #[test]
    fn arrangement_rejects_heighted_vertex_without_node_grade_authority() {
        let owner = owner(RoadSurfaceBandKind::Sidewalk, 4);
        let field = NodeBandHeightFieldId::new(0, 4, RoadSurfaceBandKind::Sidewalk);
        let mut shape = vec![
            height_vertex(0.0, 0.0, 1.0),
            height_vertex(1.0, 0.0, 1.0),
            height_vertex(0.0, 1.0, 1.0),
        ];
        for vertex in &mut shape {
            vertex.height_field_id = field;
        }
        let heights = NodeHeightSolution {
            node_id: 81,
            piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
            regions: vec![NodeHeightedRegion {
                kind: RoadSurfaceBandKind::Sidewalk,
                owner,
                height_field_id: field,
                shape: vec![shape],
                area_m2: 0.5,
                seam_constraints: Vec::new(),
            }],
        };

        assert!(matches!(
            NodeArrangement::from_height_solution(&heights),
            Err(NodeArrangementError::MissingGradeAuthority {
                owner: missing_owner,
                height_field_id,
                ..
            }) if missing_owner == owner && height_field_id == field
        ));
    }

    #[test]
    fn arrangement_exports_explicit_material_seam_grade_decision() {
        let owner = owner(RoadSurfaceBandKind::Carriageway, 6);
        let field = NodeBandHeightFieldId::new(0, 6, RoadSurfaceBandKind::Carriageway);
        let mut heights = NodeHeightSolution {
            node_id: 82,
            piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
            regions: vec![NodeHeightedRegion {
                kind: RoadSurfaceBandKind::Carriageway,
                owner,
                height_field_id: field,
                shape: vec![vec![
                    height_vertex(0.0, 0.0, 2.0),
                    height_vertex(1.0, 0.0, 2.0),
                    height_vertex(0.0, 1.0, 2.0),
                ]],
                area_m2: 0.5,
                seam_constraints: Vec::new(),
            }],
        };
        for vertex in heights.regions[0].shape[0].iter_mut() {
            vertex.height_field_id = field;
            vertex.grade_authority = Some(NodeGradeVertexAuthority::new(
                vertex.point_xz,
                vertex.height_m,
                owner,
                field,
                NodeGradeCarrierDecision::ExplicitMaterialSeam,
            ));
        }

        let arrangement = NodeArrangement::from_height_solution(&heights)
            .expect("grade-authorized explicit seam should arrange");
        assert!(arrangement.vertices().iter().all(|vertex| {
            vertex.grade_authority().decision == NodeGradeCarrierDecision::ExplicitMaterialSeam
        }));
    }

    #[test]
    fn duplicate_arrangement_vertex_key_rejects_same_context_height_conflict() {
        let mut arrangement = NodeArrangement::new(7, RoadSurfaceVisualNodePieceKind::JunctionN);
        let point = RoadVec2::new(0.0, 0.0);

        arrangement
            .insert_vertex(
                point,
                1.0,
                [owner(RoadSurfaceBandKind::Sidewalk, 0)],
                height_field_id(RoadSurfaceBandKind::Sidewalk, 0),
                [seam_source(0)],
            )
            .expect("first vertex should insert");

        let result = arrangement.insert_vertex(
            point,
            1.01,
            [owner(RoadSurfaceBandKind::Sidewalk, 0)],
            height_field_id(RoadSurfaceBandKind::Sidewalk, 0),
            [seam_source(0)],
        );

        assert!(matches!(
            result,
            Err(NodeArrangementError::DuplicateVertexHeightConflict { .. })
        ));
    }

    #[test]
    fn duplicate_arrangement_vertex_key_keeps_distinct_same_material_owner_contexts() {
        let mut arrangement = NodeArrangement::new(7, RoadSurfaceVisualNodePieceKind::JunctionN);
        let point = RoadVec2::new(0.0, 0.0);

        arrangement
            .insert_vertex(
                point,
                1.0,
                [owner(RoadSurfaceBandKind::Sidewalk, 0)],
                height_field_id(RoadSurfaceBandKind::Sidewalk, 0),
                [seam_source(0)],
            )
            .expect("first sidewalk vertex should insert");

        let second = arrangement
            .insert_vertex(
                point,
                2.0,
                [owner(RoadSurfaceBandKind::Sidewalk, 1)],
                height_field_id(RoadSurfaceBandKind::Sidewalk, 1),
                [seam_source(1)],
            )
            .expect("same material point contact keeps distinct owner-height context");

        assert_ne!(second, NodeArrangementVertexId(0));
        assert_eq!(arrangement.vertices().len(), 2);
    }

    #[test]
    fn duplicate_arrangement_vertex_key_keeps_distinct_curb_rail_height_contexts() {
        let mut arrangement = NodeArrangement::new(7, RoadSurfaceVisualNodePieceKind::JunctionN);
        let point = RoadVec2::new(0.0, 0.0);

        let lower = arrangement
            .insert_vertex(
                point,
                0.0,
                [owner(RoadSurfaceBandKind::CurbOrShoulder, 0)],
                height_field_id(RoadSurfaceBandKind::CurbOrShoulder, 0),
                [NodeSeamSource::RaisedStepContact { owner_index: 0 }],
            )
            .expect("lower curb rail vertex should insert");
        let raised = arrangement
            .insert_vertex(
                point,
                0.12,
                [owner(RoadSurfaceBandKind::CurbOrShoulder, 1)],
                height_field_id(RoadSurfaceBandKind::CurbOrShoulder, 1),
                [NodeSeamSource::RaisedStepContact { owner_index: 1 }],
            )
            .expect("raised curb rail vertex should keep separate owner-height context");

        assert_ne!(lower, raised);
        assert_eq!(arrangement.vertices().len(), 2);
    }

    #[test]
    fn arrangement_keeps_height_distinct_explicit_seam_contexts() {
        let mut arrangement = NodeArrangement::new(7, RoadSurfaceVisualNodePieceKind::Bend);
        let point = RoadVec2::new(0.0, 0.0);

        let low = arrangement
            .insert_vertex(
                point,
                1.0,
                [owner(RoadSurfaceBandKind::Carriageway, 0)],
                height_field_id(RoadSurfaceBandKind::Carriageway, 0),
                [seam_source(0)],
            )
            .expect("first vertex should insert");
        let high = arrangement
            .insert_vertex(
                point,
                2.0,
                [owner(RoadSurfaceBandKind::Sidewalk, 1)],
                height_field_id(RoadSurfaceBandKind::Sidewalk, 1),
                [seam_source(1)],
            )
            .expect("explicit owner context keeps steep endpoint-height duplicates deterministic");

        assert_ne!(low, high);
        assert_eq!(arrangement.vertices().len(), 2);
    }

    #[test]
    fn arrangement_rejects_different_material_height_context_without_explicit_seam() {
        let carriageway = owner(RoadSurfaceBandKind::Carriageway, 0);
        let sidewalk = owner(RoadSurfaceBandKind::Sidewalk, 1);
        let heights = two_region_height_solution_with_material_heights(
            carriageway,
            sidewalk,
            0.0,
            1.0,
            Vec::new(),
            Vec::new(),
        );

        assert!(matches!(
            NodeArrangement::from_height_solution(&heights),
            Err(NodeArrangementError::DuplicateVertexHeightConflict { .. })
        ));
    }

    #[test]
    fn arrangement_accepts_different_material_height_context_at_explicit_seam_endpoint() {
        let carriageway = owner(RoadSurfaceBandKind::Carriageway, 0);
        let sidewalk = owner(RoadSurfaceBandKind::Sidewalk, 1);
        let seam = NodeRegionSeamConstraint {
            constraint_index: 31,
            seam_source: NodeSeamSource::AsphaltBoundary { owner_index: 0 },
            owner: None,
            opposite_owner: None,
            constrains_shared_height: false,
            is_material_transition: true,
            start_xz: RoadVec2::new(1.0, 0.0),
            end_xz: RoadVec2::new(1.0, 1.0),
        };
        let heights = two_region_height_solution_with_material_heights(
            carriageway,
            sidewalk,
            0.0,
            1.0,
            vec![seam.clone()],
            vec![seam],
        );

        let arrangement = NodeArrangement::from_height_solution(&heights)
            .expect("explicit material seam endpoints may carry distinct field heights");

        assert_eq!(arrangement.vertices().len(), 8);
        assert!(arrangement.edges().iter().any(|edge| {
            edge.owner == carriageway
                && edge.opposite_owner == Some(sidewalk)
                && edge.is_material_transition
                && edge.source_constraint_indices == vec![31]
        }));
    }

    #[test]
    fn arrangement_accepts_different_material_height_context_at_explicit_point_seam() {
        let carriageway = owner(RoadSurfaceBandKind::Carriageway, 0);
        let sidewalk = owner(RoadSurfaceBandKind::Sidewalk, 1);
        let seam = NodeRegionSeamConstraint {
            constraint_index: 32,
            seam_source: NodeSeamSource::AsphaltBoundary { owner_index: 0 },
            owner: None,
            opposite_owner: None,
            constrains_shared_height: false,
            is_material_transition: true,
            start_xz: RoadVec2::new(1.0, 1.0),
            end_xz: RoadVec2::new(1.0, 1.0),
        };
        let heights = NodeHeightSolution {
            node_id: 11,
            piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
            regions: vec![
                test_height_region_with_seams(
                    RoadSurfaceBandKind::Carriageway,
                    carriageway,
                    vec![
                        height_vertex(0.0, 0.0, 0.0),
                        height_vertex(1.0, 0.0, 0.0),
                        height_vertex(1.0, 1.0, 0.0),
                        height_vertex(0.0, 1.0, 0.0),
                    ],
                    vec![seam.clone()],
                ),
                test_height_region_with_seams(
                    RoadSurfaceBandKind::Sidewalk,
                    sidewalk,
                    vec![
                        height_vertex(1.0, 1.0, 1.0),
                        height_vertex(2.0, 1.0, 1.0),
                        height_vertex(2.0, 2.0, 1.0),
                        height_vertex(1.0, 2.0, 1.0),
                    ],
                    vec![seam],
                ),
            ],
        };

        let arrangement = NodeArrangement::from_height_solution(&heights)
            .expect("explicit material point seam may carry distinct field heights");

        assert_eq!(arrangement.vertices().len(), 8);
        assert!(arrangement.edges().iter().all(|edge| {
            edge.opposite_owner != Some(sidewalk) || edge.source_constraint_indices != vec![32]
        }));
    }

    #[test]
    fn arrangement_edges_match_opposite_owners_by_canonical_xz_segment() {
        let carriageway = owner(RoadSurfaceBandKind::Carriageway, 0);
        let sidewalk = owner(RoadSurfaceBandKind::Sidewalk, 1);
        let generic_seam = NodeRegionSeamConstraint {
            constraint_index: 2,
            seam_source: NodeSeamSource::FootprintBoundary { owner_index: 0 },
            owner: None,
            opposite_owner: None,
            constrains_shared_height: true,
            is_material_transition: false,
            start_xz: RoadVec2::new(1.0, 0.0),
            end_xz: RoadVec2::new(1.0, 1.0),
        };
        let shared_seam = NodeRegionSeamConstraint {
            constraint_index: 17,
            seam_source: NodeSeamSource::AsphaltBoundary { owner_index: 0 },
            owner: None,
            opposite_owner: None,
            constrains_shared_height: true,
            is_material_transition: true,
            start_xz: RoadVec2::new(1.0, 0.0),
            end_xz: RoadVec2::new(1.0, 1.0),
        };
        let heights = NodeHeightSolution {
            node_id: 11,
            piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
            regions: vec![
                test_height_region_with_seams(
                    RoadSurfaceBandKind::Carriageway,
                    carriageway,
                    vec![
                        height_vertex(0.0, 0.0, 0.0),
                        height_vertex(1.0, 0.0, 0.0),
                        height_vertex(1.0, 1.0, 0.0),
                        height_vertex(0.0, 1.0, 0.0),
                    ],
                    vec![generic_seam.clone(), shared_seam.clone()],
                ),
                test_height_region_with_seams(
                    RoadSurfaceBandKind::Sidewalk,
                    sidewalk,
                    vec![
                        height_vertex(1.0, 0.0, 0.0),
                        height_vertex(2.0, 0.0, 0.0),
                        height_vertex(2.0, 1.0, 0.0),
                        height_vertex(1.0, 1.0, 0.0),
                    ],
                    vec![generic_seam, shared_seam],
                ),
            ],
        };
        let arrangement = NodeArrangement::from_height_solution(&heights)
            .expect("height-owned regions should produce canonical arrangement");

        assert_eq!(arrangement.vertices().len(), 8);
        assert!(arrangement.edges().iter().any(|edge| {
            edge.owner == carriageway
                && edge.opposite_owner == Some(sidewalk)
                && matches!(edge.seam_source, NodeSeamSource::AsphaltBoundary { .. })
                && edge.source_constraint_indices == vec![2, 17]
        }));
        assert!(arrangement.edges().iter().any(|edge| {
            edge.owner == sidewalk
                && edge.opposite_owner == Some(carriageway)
                && matches!(edge.seam_source, NodeSeamSource::AsphaltBoundary { .. })
                && edge.source_constraint_indices == vec![2, 17]
        }));
    }

    #[test]
    fn explicit_vertical_step_segments_use_canonical_edge_owner_pair() {
        let carriageway = owner(RoadSurfaceBandKind::Carriageway, 2);
        let curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let start = RoadVec2::new(1.0, 0.0);
        let end = RoadVec2::new(1.0, 1.0);
        let seam = NodeRegionSeamConstraint {
            constraint_index: 91,
            seam_source: NodeSeamSource::RaisedStepContact { owner_index: 1 },
            owner: Some(carriageway),
            opposite_owner: Some(curb),
            constrains_shared_height: false,
            is_material_transition: true,
            start_xz: start,
            end_xz: end,
        };
        let heights = NodeHeightSolution {
            node_id: 11,
            piece_kind: RoadSurfaceVisualNodePieceKind::Bend,
            regions: vec![
                test_height_region_with_seams(
                    RoadSurfaceBandKind::Carriageway,
                    carriageway,
                    vec![
                        height_vertex(0.0, 0.0, 0.0),
                        height_vertex(1.0, 0.0, 0.0),
                        height_vertex(1.0, 1.0, 0.0),
                        height_vertex(0.0, 1.0, 0.0),
                    ],
                    vec![seam.clone()],
                ),
                test_height_region_with_seams(
                    RoadSurfaceBandKind::CurbOrShoulder,
                    curb,
                    vec![
                        height_vertex(1.0, 0.0, 0.12),
                        height_vertex(2.0, 0.0, 0.12),
                        height_vertex(2.0, 1.0, 0.12),
                        height_vertex(1.0, 1.0, 0.12),
                    ],
                    vec![seam],
                ),
            ],
        };
        let arrangement = NodeArrangement::from_height_solution(&heights)
            .expect("explicit curb step seam should produce a canonical arrangement");

        let segments = arrangement.explicit_vertical_step_segments();
        let expected = NodeExplicitVerticalStepSegment::new(
            NodeArrangementKey::from_point(start),
            NodeArrangementKey::from_point(end),
            carriageway,
            curb,
        )
        .expect("test segment is non-degenerate");

        assert!(segments.contains(&expected));
    }

    #[test]
    fn explicit_vertical_step_segments_do_not_derive_steps_from_face_overlap() {
        let carriageway = owner(RoadSurfaceBandKind::Carriageway, 2);
        let curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let mut arrangement = NodeArrangement::new(11, RoadSurfaceVisualNodePieceKind::Bend);
        let carriageway_height = height_field_id(RoadSurfaceBandKind::Carriageway, 2);
        let curb_height = height_field_id(RoadSurfaceBandKind::CurbOrShoulder, 1);

        let carriageway_region = arrangement.push_region(
            carriageway,
            carriageway_height,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            1.0,
            Vec::new(),
        );
        let curb_region = arrangement.push_region(
            curb,
            curb_height,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            1.0,
            Vec::new(),
        );

        let carriageway_start = arrangement
            .insert_vertex(
                RoadVec2::new(0.0, 0.0),
                0.0,
                [carriageway],
                carriageway_height,
                [],
            )
            .expect("test vertex is legal");
        let carriageway_end = arrangement
            .insert_vertex(
                RoadVec2::new(4.0, 0.0),
                0.0,
                [carriageway],
                carriageway_height,
                [],
            )
            .expect("test vertex is legal");
        let carriageway_apex = arrangement
            .insert_vertex(
                RoadVec2::new(0.0, 1.0),
                0.0,
                [carriageway],
                carriageway_height,
                [],
            )
            .expect("test vertex is legal");
        arrangement.push_face(
            carriageway_region,
            carriageway,
            [carriageway_start, carriageway_end, carriageway_apex],
        );

        let curb_start = arrangement
            .insert_vertex(RoadVec2::new(1.0, 0.0), 0.12, [curb], curb_height, [])
            .expect("test vertex is legal");
        let curb_end = arrangement
            .insert_vertex(RoadVec2::new(3.0, 0.0), 0.12, [curb], curb_height, [])
            .expect("test vertex is legal");
        let curb_apex = arrangement
            .insert_vertex(RoadVec2::new(1.0, -1.0), 0.12, [curb], curb_height, [])
            .expect("test vertex is legal");
        arrangement.push_face(curb_region, curb, [curb_start, curb_end, curb_apex]);

        let segments = arrangement.explicit_vertical_step_segments();
        let expected = NodeExplicitVerticalStepSegment::new(
            NodeArrangementKey::from_point(RoadVec2::new(1.0, 0.0)),
            NodeArrangementKey::from_point(RoadVec2::new(3.0, 0.0)),
            carriageway,
            curb,
        )
        .expect("test segment is non-degenerate");
        let stale_full_edge = NodeExplicitVerticalStepSegment::new(
            NodeArrangementKey::from_point(RoadVec2::new(0.0, 0.0)),
            NodeArrangementKey::from_point(RoadVec2::new(4.0, 0.0)),
            carriageway,
            curb,
        )
        .expect("test segment is non-degenerate");

        assert!(!segments.contains(&expected));
        assert!(!segments.contains(&stale_full_edge));
    }

    #[test]
    fn explicit_vertical_step_segments_include_direct_sidewalk_contacts() {
        let carriageway = owner(RoadSurfaceBandKind::Carriageway, 2);
        let sidewalk = owner(RoadSurfaceBandKind::Sidewalk, 1);
        let seam = NodeRegionSeamConstraint {
            constraint_index: 92,
            seam_source: NodeSeamSource::AsphaltBoundary { owner_index: 2 },
            owner: Some(carriageway),
            opposite_owner: Some(sidewalk),
            constrains_shared_height: false,
            is_material_transition: true,
            start_xz: RoadVec2::new(1.0, 0.0),
            end_xz: RoadVec2::new(1.0, 1.0),
        };
        let heights = two_region_height_solution_with_material_heights(
            carriageway,
            sidewalk,
            0.0,
            0.12,
            vec![seam.clone()],
            vec![seam],
        );
        let arrangement = NodeArrangement::from_height_solution(&heights)
            .expect("explicit non-road step seam should produce a canonical arrangement");

        let segments = arrangement.explicit_vertical_step_segments();
        let expected = NodeExplicitVerticalStepSegment::new(
            NodeArrangementKey::from_point(RoadVec2::new(1.0, 0.0)),
            NodeArrangementKey::from_point(RoadVec2::new(1.0, 1.0)),
            carriageway,
            sidewalk,
        )
        .expect("test segment is non-degenerate");

        assert!(segments.contains(&expected));
    }

    #[test]
    fn explicit_vertical_step_segments_use_authorized_source_pair_on_boundary_edge() {
        let carriageway = owner(RoadSurfaceBandKind::Carriageway, 8);
        let curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 7);
        let start = RoadVec2::new(0.0, 0.0);
        let end = RoadVec2::new(2.0, 0.0);
        let seam = NodeRegionSeamConstraint {
            constraint_index: 95,
            seam_source: NodeSeamSource::RaisedStepContact { owner_index: 7 },
            owner: Some(carriageway),
            opposite_owner: Some(curb),
            constrains_shared_height: false,
            is_material_transition: true,
            start_xz: start,
            end_xz: end,
        };
        let heights = NodeHeightSolution {
            node_id: 11,
            piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
            regions: vec![test_height_region_with_seams(
                RoadSurfaceBandKind::Carriageway,
                carriageway,
                vec![
                    height_vertex(0.0, 0.0, 0.0),
                    height_vertex(2.0, 0.0, 0.0),
                    height_vertex(0.0, 1.0, 0.0),
                ],
                vec![seam],
            )],
        };
        let arrangement = NodeArrangement::from_height_solution(&heights)
            .expect("source-authorized boundary edge should produce a canonical arrangement");

        let segments = arrangement.explicit_vertical_step_segments();
        let expected = NodeExplicitVerticalStepSegment::new(
            NodeArrangementKey::from_point(start),
            NodeArrangementKey::from_point(end),
            carriageway,
            curb,
        )
        .expect("test segment is non-degenerate");

        assert!(segments.contains(&expected));
    }

    #[test]
    fn explicit_vertical_step_segments_use_selected_source_pair_on_exposed_boundary() {
        let carriageway = owner(RoadSurfaceBandKind::Carriageway, 9);
        let selected_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 10);
        let stale_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 13);
        let start = RoadVec2::new(0.0, 0.0);
        let end = RoadVec2::new(2.0, 0.0);
        let selected_seam = NodeRegionSeamConstraint {
            constraint_index: 103,
            seam_source: NodeSeamSource::RaisedStepContact { owner_index: 9 },
            owner: Some(carriageway),
            opposite_owner: Some(selected_curb),
            constrains_shared_height: false,
            is_material_transition: true,
            start_xz: start,
            end_xz: end,
        };
        let stale_seam = NodeRegionSeamConstraint {
            constraint_index: 416,
            seam_source: NodeSeamSource::RaisedStepContact { owner_index: 9 },
            owner: Some(carriageway),
            opposite_owner: Some(stale_curb),
            constrains_shared_height: false,
            is_material_transition: true,
            start_xz: start,
            end_xz: end,
        };
        let heights = NodeHeightSolution {
            node_id: 11,
            piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
            regions: vec![test_height_region_with_seams(
                RoadSurfaceBandKind::Carriageway,
                carriageway,
                vec![
                    height_vertex(0.0, 0.0, 0.0),
                    height_vertex(2.0, 0.0, 0.0),
                    height_vertex(0.0, 1.0, 0.0),
                ],
                vec![selected_seam, stale_seam],
            )],
        };
        let arrangement = NodeArrangement::from_height_solution(&heights)
            .expect("selected exposed boundary source should produce a canonical arrangement");

        let segments = arrangement.explicit_vertical_step_segments();
        let expected = NodeExplicitVerticalStepSegment::new(
            NodeArrangementKey::from_point(start),
            NodeArrangementKey::from_point(end),
            carriageway,
            selected_curb,
        )
        .expect("test segment is non-degenerate");
        let stale = NodeExplicitVerticalStepSegment::new(
            NodeArrangementKey::from_point(start),
            NodeArrangementKey::from_point(end),
            carriageway,
            stale_curb,
        )
        .expect("test segment is non-degenerate");

        assert!(segments.contains(&expected));
        assert!(!segments.contains(&stale));
    }

    #[test]
    fn explicit_vertical_step_segments_prefer_final_edge_owner_pair_source() {
        let carriageway = owner(RoadSurfaceBandKind::Carriageway, 9);
        let actual_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let stale_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 10);
        let start = RoadVec2::new(1.0, 0.0);
        let end = RoadVec2::new(1.0, 1.0);
        let actual_seam = NodeRegionSeamConstraint {
            constraint_index: 572,
            seam_source: NodeSeamSource::RaisedStepContact { owner_index: 1 },
            owner: Some(carriageway),
            opposite_owner: Some(actual_curb),
            constrains_shared_height: false,
            is_material_transition: true,
            start_xz: start,
            end_xz: end,
        };
        let stale_overlapping_seam = NodeRegionSeamConstraint {
            constraint_index: 96,
            seam_source: NodeSeamSource::RaisedStepContact { owner_index: 10 },
            owner: Some(carriageway),
            opposite_owner: Some(stale_curb),
            constrains_shared_height: false,
            is_material_transition: true,
            start_xz: start,
            end_xz: end,
        };
        let heights = NodeHeightSolution {
            node_id: 11,
            piece_kind: RoadSurfaceVisualNodePieceKind::Bend,
            regions: vec![
                test_height_region_with_seams(
                    RoadSurfaceBandKind::Carriageway,
                    carriageway,
                    vec![
                        height_vertex(0.0, 0.0, 0.0),
                        height_vertex(1.0, 0.0, 0.0),
                        height_vertex(1.0, 1.0, 0.0),
                        height_vertex(0.0, 1.0, 0.0),
                    ],
                    vec![actual_seam.clone(), stale_overlapping_seam.clone()],
                ),
                test_height_region_with_seams(
                    RoadSurfaceBandKind::CurbOrShoulder,
                    actual_curb,
                    vec![
                        height_vertex(1.0, 0.0, 0.12),
                        height_vertex(2.0, 0.0, 0.12),
                        height_vertex(2.0, 1.0, 0.12),
                        height_vertex(1.0, 1.0, 0.12),
                    ],
                    vec![actual_seam, stale_overlapping_seam],
                ),
            ],
        };
        let arrangement = NodeArrangement::from_height_solution(&heights)
            .expect("actual edge owner-pair source should authorize the step");

        let segments = arrangement.explicit_vertical_step_segments();
        let expected = NodeExplicitVerticalStepSegment::new(
            NodeArrangementKey::from_point(start),
            NodeArrangementKey::from_point(end),
            carriageway,
            actual_curb,
        )
        .expect("test segment is non-degenerate");
        let stale = NodeExplicitVerticalStepSegment::new(
            NodeArrangementKey::from_point(start),
            NodeArrangementKey::from_point(end),
            carriageway,
            stale_curb,
        )
        .expect("test segment is non-degenerate");

        assert!(segments.contains(&expected));
        assert!(!segments.contains(&stale));
    }

    #[test]
    fn explicit_vertical_step_segments_require_explicit_owner_pair_source() {
        let carriageway = owner(RoadSurfaceBandKind::Carriageway, 2);
        let curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let seam = NodeRegionSeamConstraint {
            constraint_index: 93,
            seam_source: NodeSeamSource::RaisedStepContact { owner_index: 1 },
            owner: Some(curb),
            opposite_owner: None,
            constrains_shared_height: false,
            is_material_transition: true,
            start_xz: RoadVec2::new(1.0, 0.0),
            end_xz: RoadVec2::new(1.0, 1.0),
        };
        let heights = two_region_height_solution_with_material_heights(
            carriageway,
            curb,
            0.0,
            0.12,
            vec![seam.clone()],
            vec![seam],
        );
        let arrangement = NodeArrangement::from_height_solution(&heights)
            .expect("role-only material seam should produce a canonical arrangement");

        let segments = arrangement.explicit_vertical_step_segments();
        let expected = NodeExplicitVerticalStepSegment::new(
            NodeArrangementKey::from_point(RoadVec2::new(1.0, 0.0)),
            NodeArrangementKey::from_point(RoadVec2::new(1.0, 1.0)),
            carriageway,
            curb,
        )
        .expect("test segment is non-degenerate");

        assert!(!segments.contains(&expected));
    }

    #[test]
    fn arrangement_rejects_mismatched_explicit_seam_owner_pair() {
        let carriageway = owner(RoadSurfaceBandKind::Carriageway, 2);
        let adjacent_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let terminal_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 6);
        let seam = NodeRegionSeamConstraint {
            constraint_index: 92,
            seam_source: NodeSeamSource::RaisedStepContact { owner_index: 6 },
            owner: Some(carriageway),
            opposite_owner: Some(terminal_curb),
            constrains_shared_height: false,
            is_material_transition: true,
            start_xz: RoadVec2::new(1.0, 0.0),
            end_xz: RoadVec2::new(1.0, 1.0),
        };
        let heights = NodeHeightSolution {
            node_id: 11,
            piece_kind: RoadSurfaceVisualNodePieceKind::Terminal,
            regions: vec![
                test_height_region_with_seams(
                    RoadSurfaceBandKind::Carriageway,
                    carriageway,
                    vec![
                        height_vertex(0.0, 0.0, 0.0),
                        height_vertex(1.0, 0.0, 0.0),
                        height_vertex(1.0, 1.0, 0.0),
                        height_vertex(0.0, 1.0, 0.0),
                    ],
                    vec![seam.clone()],
                ),
                test_height_region_with_seams(
                    RoadSurfaceBandKind::CurbOrShoulder,
                    adjacent_curb,
                    vec![
                        height_vertex(1.0, 0.0, 0.12),
                        height_vertex(2.0, 0.0, 0.12),
                        height_vertex(2.0, 1.0, 0.12),
                        height_vertex(1.0, 1.0, 0.12),
                    ],
                    vec![seam],
                ),
            ],
        };

        assert!(matches!(
            NodeArrangement::from_height_solution(&heights),
            Err(NodeArrangementError::DuplicateVertexHeightConflict { .. })
        ));
    }

    #[test]
    fn shared_arrangement_edge_reports_missing_source_constraint() {
        let carriageway = owner(RoadSurfaceBandKind::Carriageway, 0);
        let sidewalk = owner(RoadSurfaceBandKind::Sidewalk, 1);
        let heights = two_region_height_solution(carriageway, sidewalk, Vec::new(), Vec::new());

        let arrangement = NodeArrangement::from_height_solution(&heights)
            .expect("height-owned regions should produce canonical arrangement diagnostics");

        assert!(matches!(
            arrangement.diagnostics().first(),
            Some(NodeArrangementDiagnostic::MissingSeamConstraint {
                region_index: 0,
                owner,
                opposite_owner,
                ..
            }) if *owner == carriageway && *opposite_owner == sidewalk
        ));
    }

    #[test]
    fn same_band_arrangement_edge_does_not_require_material_seam_constraint() {
        let first = owner(RoadSurfaceBandKind::Carriageway, 0);
        let second = owner(RoadSurfaceBandKind::Carriageway, 1);
        let heights = NodeHeightSolution {
            node_id: 12,
            piece_kind: RoadSurfaceVisualNodePieceKind::Bend,
            regions: vec![
                test_height_region_with_seams(
                    RoadSurfaceBandKind::Carriageway,
                    first,
                    vec![
                        height_vertex(0.0, 0.0, 0.0),
                        height_vertex(1.0, 0.0, 0.0),
                        height_vertex(1.0, 1.0, 0.0),
                        height_vertex(0.0, 1.0, 0.0),
                    ],
                    Vec::new(),
                ),
                test_height_region_with_seams(
                    RoadSurfaceBandKind::Carriageway,
                    second,
                    vec![
                        height_vertex(1.0, 0.0, 0.0),
                        height_vertex(2.0, 0.0, 0.0),
                        height_vertex(2.0, 1.0, 0.0),
                        height_vertex(1.0, 1.0, 0.0),
                    ],
                    Vec::new(),
                ),
            ],
        };

        let arrangement = NodeArrangement::from_height_solution(&heights)
            .expect("same-band owned regions should share a non-material boundary");

        assert!(arrangement.diagnostics().is_empty());
    }

    #[test]
    fn equally_ranked_conflicting_arrangement_seams_are_reported() {
        let carriageway = owner(RoadSurfaceBandKind::Carriageway, 0);
        let sidewalk = owner(RoadSurfaceBandKind::Sidewalk, 1);
        let first = NodeRegionSeamConstraint {
            constraint_index: 20,
            seam_source: NodeSeamSource::AsphaltBoundary { owner_index: 0 },
            owner: None,
            opposite_owner: None,
            constrains_shared_height: true,
            is_material_transition: true,
            start_xz: RoadVec2::new(1.0, 0.0),
            end_xz: RoadVec2::new(1.0, 1.0),
        };
        let second = NodeRegionSeamConstraint {
            constraint_index: 21,
            seam_source: NodeSeamSource::AsphaltBoundary { owner_index: 1 },
            owner: None,
            opposite_owner: None,
            constrains_shared_height: true,
            is_material_transition: true,
            start_xz: RoadVec2::new(1.0, 0.0),
            end_xz: RoadVec2::new(1.0, 1.0),
        };
        let seams = vec![first, second];
        let heights = two_region_height_solution(carriageway, sidewalk, seams.clone(), seams);

        let arrangement = NodeArrangement::from_height_solution(&heights)
            .expect("height-owned regions should produce canonical arrangement diagnostics");

        assert!(matches!(
            arrangement.diagnostics().first(),
            Some(NodeArrangementDiagnostic::AmbiguousSeamConstraint {
                region_index: 0,
                owner,
                opposite_owner,
                ..
            }) if *owner == carriageway && *opposite_owner == sidewalk
        ));
    }

    #[test]
    fn arrangement_vertex_requires_explicit_owner() {
        let mut arrangement = NodeArrangement::new(9, RoadSurfaceVisualNodePieceKind::Terminal);

        let result = arrangement.insert_vertex(
            RoadVec2::new(1.0, 2.0),
            0.0,
            [],
            height_field_id(RoadSurfaceBandKind::Sidewalk, 0),
            [seam_source(0)],
        );

        assert!(matches!(
            result,
            Err(NodeArrangementError::EmptyOwnerSet { .. })
        ));
    }

    fn two_region_height_solution(
        carriageway: NodeBandOwner,
        sidewalk: NodeBandOwner,
        carriageway_seams: Vec<NodeRegionSeamConstraint>,
        sidewalk_seams: Vec<NodeRegionSeamConstraint>,
    ) -> NodeHeightSolution {
        two_region_height_solution_with_material_heights(
            carriageway,
            sidewalk,
            0.0,
            0.0,
            carriageway_seams,
            sidewalk_seams,
        )
    }

    fn two_region_height_solution_with_material_heights(
        carriageway: NodeBandOwner,
        sidewalk: NodeBandOwner,
        carriageway_height_m: f64,
        sidewalk_height_m: f64,
        carriageway_seams: Vec<NodeRegionSeamConstraint>,
        sidewalk_seams: Vec<NodeRegionSeamConstraint>,
    ) -> NodeHeightSolution {
        NodeHeightSolution {
            node_id: 11,
            piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
            regions: vec![
                test_height_region_with_seams(
                    RoadSurfaceBandKind::Carriageway,
                    carriageway,
                    vec![
                        height_vertex(0.0, 0.0, carriageway_height_m),
                        height_vertex(1.0, 0.0, carriageway_height_m),
                        height_vertex(1.0, 1.0, carriageway_height_m),
                        height_vertex(0.0, 1.0, carriageway_height_m),
                    ],
                    carriageway_seams,
                ),
                test_height_region_with_seams(
                    RoadSurfaceBandKind::Sidewalk,
                    sidewalk,
                    vec![
                        height_vertex(1.0, 0.0, sidewalk_height_m),
                        height_vertex(2.0, 0.0, sidewalk_height_m),
                        height_vertex(2.0, 1.0, sidewalk_height_m),
                        height_vertex(1.0, 1.0, sidewalk_height_m),
                    ],
                    sidewalk_seams,
                ),
            ],
        }
    }

    fn test_height_region_with_seams(
        kind: RoadSurfaceBandKind,
        owner: NodeBandOwner,
        contour: Vec<NodeHeightedVertex>,
        seam_constraints: Vec<NodeRegionSeamConstraint>,
    ) -> NodeHeightedRegion {
        let height_field_id =
            NodeBandHeightFieldId::new(owner.owner_index(), owner.owner_index(), kind);
        let contour = contour
            .into_iter()
            .map(|mut vertex| {
                vertex.height_field_id = height_field_id;
                vertex.grade_authority = Some(NodeGradeVertexAuthority::new(
                    vertex.point_xz,
                    vertex.height_m,
                    owner,
                    height_field_id,
                    NodeGradeCarrierDecision::SourceCarrier { authority: None },
                ));
                vertex
            })
            .collect();
        NodeHeightedRegion {
            kind,
            owner,
            height_field_id,
            shape: vec![contour],
            area_m2: 1.0,
            seam_constraints,
        }
    }

    fn height_vertex(x: f64, z: f64, height_m: f64) -> NodeHeightedVertex {
        NodeHeightedVertex {
            point_xz: RoadVec2::new(x, z),
            height_m,
            height_field_id: height_field_id(RoadSurfaceBandKind::Sidewalk, 0),
            height_authority: None,
            grade_authority: None,
        }
    }
}
