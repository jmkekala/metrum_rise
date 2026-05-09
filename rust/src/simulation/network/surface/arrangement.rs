//! Canonical node-arrangement identity and ownership data model.

use super::backend::{ROAD_OVERLAY_COORDINATE_SCALE, RoadVec2};
use super::height::{NodeHeightSolution, NodeHeightedRegion, NodeHeightedVertex};
use super::triangulation::{NodeTriangulatedRegion, NodeTriangulationSolution};
use super::{RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind};
use std::collections::{BTreeMap, BTreeSet};

const NODE_ARRANGEMENT_KEY_SCALE: f64 = ROAD_OVERLAY_COORDINATE_SCALE;
const NODE_ARRANGEMENT_HEIGHT_SCALE: f64 = 1000.0;
const NODE_ARRANGEMENT_MM_SCALE: f64 = 1000.0;

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
pub(crate) struct NodeExplicitVerticalStepSegment {
    start: NodeArrangementKey,
    end: NodeArrangementKey,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) enum NodeSeamSource {
    AsphaltBoundary { owner_index: usize },
    AsphaltCurbContact { owner_index: usize },
    CurbSidewalkContact { owner_index: usize },
    SidewalkOuter { owner_index: usize },
    FootprintBoundary { owner_index: usize },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeRegionSeamConstraint {
    pub(crate) constraint_index: usize,
    pub(crate) seam_source: NodeSeamSource,
    pub(crate) owner: Option<NodeBandOwner>,
    pub(crate) opposite_owner: Option<NodeBandOwner>,
    pub(crate) constrains_shared_height: bool,
    pub(crate) is_material_transition: bool,
    pub(crate) start_xz: RoadVec2,
    pub(crate) end_xz: RoadVec2,
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
}

impl NodeArrangementKey {
    pub(crate) fn from_point(point: RoadVec2) -> Self {
        Self {
            x_key: quantize_m(point.x),
            z_key: quantize_m(point.y),
        }
    }

    pub(crate) fn x_key(self) -> i64 {
        self.x_key
    }

    pub(crate) fn z_key(self) -> i64 {
        self.z_key
    }

    pub(crate) fn x_mm(self) -> i64 {
        ((self.x_key as f64 / NODE_ARRANGEMENT_KEY_SCALE) * NODE_ARRANGEMENT_MM_SCALE).round()
            as i64
    }

    pub(crate) fn z_mm(self) -> i64 {
        ((self.z_key as f64 / NODE_ARRANGEMENT_KEY_SCALE) * NODE_ARRANGEMENT_MM_SCALE).round()
            as i64
    }
}

impl NodeExplicitVerticalStepSegment {
    pub(crate) fn new(
        a: NodeArrangementKey,
        b: NodeArrangementKey,
        owner: NodeBandOwner,
        opposite_owner: NodeBandOwner,
    ) -> Option<Self> {
        if a == b {
            return None;
        }
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        let (owner, opposite_owner) = if owner <= opposite_owner {
            (owner, opposite_owner)
        } else {
            (opposite_owner, owner)
        };
        Some(Self {
            start,
            end,
            owner,
            opposite_owner,
        })
    }

    pub(crate) fn start(self) -> NodeArrangementKey {
        self.start
    }

    pub(crate) fn end(self) -> NodeArrangementKey {
        self.end
    }

    pub(crate) fn owner(self) -> NodeBandOwner {
        self.owner
    }

    pub(crate) fn opposite_owner(self) -> NodeBandOwner {
        self.opposite_owner
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

    pub(crate) fn explicit_vertical_step_segments(&self) -> Vec<NodeExplicitVerticalStepSegment> {
        let mut segments = BTreeSet::new();
        for region in &self.regions {
            for constraint in &region.seam_constraints {
                if let Some(segment) = explicit_vertical_step_segment_from_constraint(constraint) {
                    segments.insert(segment);
                }
            }
        }
        for edge in &self.edges {
            if !edge.is_explicit_vertical_step() {
                continue;
            }
            if self.piece_kind == RoadSurfaceVisualNodePieceKind::Terminal
                && self.edge_has_owner_pair_source_constraint(edge)
            {
                continue;
            }
            let Some(opposite_owner) = edge.opposite_owner else {
                continue;
            };
            let Some(start) = self.vertices.get(edge.start.0).map(|vertex| vertex.key) else {
                continue;
            };
            let Some(end) = self.vertices.get(edge.end.0).map(|vertex| vertex.key) else {
                continue;
            };
            if let Some(segment) =
                NodeExplicitVerticalStepSegment::new(start, end, edge.owner, opposite_owner)
            {
                segments.insert(segment);
            }
        }
        segments.into_iter().collect()
    }

    pub(crate) fn diagnostics(&self) -> &[NodeArrangementDiagnostic] {
        &self.diagnostics
    }

    pub(crate) fn insert_vertex(
        &mut self,
        point_xz: RoadVec2,
        height_m: f64,
        owners: impl IntoIterator<Item = NodeBandOwner>,
        height_field_id: NodeBandHeightFieldId,
        seam_sources: impl IntoIterator<Item = NodeSeamSource>,
    ) -> Result<NodeArrangementVertexId, NodeArrangementError> {
        let key = NodeArrangementKey::from_point(point_xz);
        let height_key = NodeArrangementHeightKey(quantize_height_m(height_m));
        let owners = canonical_non_empty_owners(key, owners)?;
        let seam_sources = canonical_sources(seam_sources);
        let context_key = NodeArrangementVertexContextKey {
            position: key,
            owners: owners.clone(),
            height_field_id,
        };

        if let Some(conflict) =
            self.height_owner_conflict_at_key(key, height_key, &owners, height_field_id)
        {
            return Err(NodeArrangementError::DuplicateVertexHeightConflict {
                key,
                existing_height_mm: conflict.0,
                incoming_height_mm: height_key.0,
            });
        }

        if let Some(existing_id) = self.vertex_by_context_key.get(&context_key).copied() {
            let existing = &mut self.vertices[existing_id.0];
            if existing.height_key != height_key {
                return Err(NodeArrangementError::DuplicateVertexHeightConflict {
                    key,
                    existing_height_mm: existing.height_key.0,
                    incoming_height_mm: height_key.0,
                });
            }
            merge_sorted_unique(&mut existing.owners, owners);
            merge_sorted_unique(&mut existing.seam_sources, seam_sources);
            return Ok(existing_id);
        }

        Ok(self.push_vertex(
            key,
            context_key,
            point_xz,
            height_m,
            height_key,
            owners,
            height_field_id,
            seam_sources,
        ))
    }

    fn height_owner_conflict_at_key(
        &self,
        key: NodeArrangementKey,
        height_key: NodeArrangementHeightKey,
        owners: &[NodeBandOwner],
        height_field_id: NodeBandHeightFieldId,
    ) -> Option<NodeArrangementHeightKey> {
        self.vertices
            .iter()
            .find(|vertex| {
                vertex.key == key
                    && vertex.height_key != height_key
                    && (vertex.height_field_id == height_field_id
                        || owners_overlap(&vertex.owners, owners)
                        || owners_share_non_curb_band_kind(&vertex.owners, owners))
            })
            .map(|vertex| vertex.height_key)
    }

    fn push_vertex(
        &mut self,
        key: NodeArrangementKey,
        context_key: NodeArrangementVertexContextKey,
        point_xz: RoadVec2,
        height_m: f64,
        height_key: NodeArrangementHeightKey,
        owners: Vec<NodeBandOwner>,
        height_field_id: NodeBandHeightFieldId,
        seam_sources: Vec<NodeSeamSource>,
    ) -> NodeArrangementVertexId {
        let id = NodeArrangementVertexId(self.vertices.len());
        self.vertices.push(NodeArrangementVertex {
            id,
            key,
            point_xz,
            height_m,
            height_key,
            owners,
            height_field_id,
            seam_sources,
        });
        self.vertex_by_context_key.insert(context_key, id);
        id
    }

    pub(crate) fn push_edge(
        &mut self,
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
    ) -> NodeArrangementEdgeId {
        let id = NodeArrangementEdgeId(self.edges.len());
        self.edges.push(NodeArrangementEdge {
            id,
            start,
            end,
            owner,
            height_field_id,
            opposite_owner,
            opposite_height_field_id,
            exposed_boundary,
            constrains_shared_height,
            is_material_transition,
            seam_source,
            source_constraint_indices,
        });
        id
    }

    pub(crate) fn push_region(
        &mut self,
        owner: NodeBandOwner,
        height_field_id: NodeBandHeightFieldId,
        outer_loop: Vec<NodeArrangementVertexId>,
        holes: Vec<Vec<NodeArrangementVertexId>>,
        boundary_edges: Vec<NodeArrangementEdgeId>,
        area_m2: f32,
        seam_constraints: Vec<NodeRegionSeamConstraint>,
    ) -> NodeOwnedRegionId {
        let id = NodeOwnedRegionId(self.regions.len());
        self.regions.push(NodeOwnedRegion {
            id,
            owner,
            height_field_id,
            outer_loop,
            holes,
            boundary_edges,
            area_m2,
            seam_constraints,
        });
        id
    }

    pub(crate) fn push_face(
        &mut self,
        region: NodeOwnedRegionId,
        owner: NodeBandOwner,
        vertices: [NodeArrangementVertexId; 3],
    ) -> NodeArrangementFaceId {
        let id = NodeArrangementFaceId(self.faces.len());
        self.faces.push(NodeArrangementFace {
            id,
            region,
            owner,
            vertices,
        });
        id
    }

    pub(crate) fn from_height_solution(
        heights: &NodeHeightSolution,
    ) -> Result<Self, NodeArrangementError> {
        let mut arrangement = Self::new(heights.node_id, heights.piece_kind);
        let mut pending_regions = Vec::with_capacity(heights.regions.len());
        let mut edge_owners =
            BTreeMap::<NodeArrangementEdgeKey, Vec<NodeArrangementEdgeOwner>>::new();
        let mut edge_use_counts = BTreeMap::<NodeArrangementEdgeKey, usize>::new();

        for (region_index, height_region) in heights.regions.iter().enumerate() {
            let pending = arrangement.pending_region(region_index, height_region)?;
            let pending_edge_owner = NodeArrangementEdgeOwner {
                owner: pending.owner,
                height_field_id: pending.height_field_id,
            };
            for edge in pending.loop_edges(&arrangement.vertices) {
                *edge_use_counts.entry(edge.key).or_default() += 1;
                edge_owners
                    .entry(edge.key)
                    .and_modify(|owners| merge_sorted_unique(owners, vec![pending_edge_owner]))
                    .or_insert_with(|| vec![pending_edge_owner]);
            }
            pending_regions.push(pending);
        }

        for pending in pending_regions {
            let mut boundary_edges = Vec::with_capacity(pending.edge_count());
            for edge in pending.loop_edges(&arrangement.vertices) {
                let opposite = edge_owners.get(&edge.key).and_then(|owners| {
                    owners
                        .iter()
                        .copied()
                        .find(|owner| owner.owner != pending.owner)
                });
                let opposite_owner = opposite.map(|owner| owner.owner);
                let opposite_height_field_id = opposite.map(|owner| owner.height_field_id);
                let source_constraints = source_constraints_for_edge(
                    edge,
                    &pending.seam_constraints,
                    &arrangement.vertices,
                );
                if let Some(opposite_owner) = opposite_owner {
                    if source_constraints.is_empty() {
                        arrangement.diagnostics.push(
                            NodeArrangementDiagnostic::MissingSeamConstraint {
                                region_index: pending.region_index,
                                owner: pending.owner,
                                opposite_owner,
                                start: edge.key.start,
                                end: edge.key.end,
                            },
                        );
                    } else if source_constraints_are_ambiguous(&source_constraints) {
                        arrangement.diagnostics.push(
                            NodeArrangementDiagnostic::AmbiguousSeamConstraint {
                                region_index: pending.region_index,
                                owner: pending.owner,
                                opposite_owner,
                                start: edge.key.start,
                                end: edge.key.end,
                            },
                        );
                    }
                }
                let seam_source = source_constraints
                    .first()
                    .map(|constraint| constraint.seam_source.clone())
                    .unwrap_or_else(|| seam_source_for_owner(pending.owner));
                let constrains_shared_height = source_constraints
                    .first()
                    .is_some_and(|constraint| constraint.constrains_shared_height);
                let is_material_transition = source_constraints
                    .first()
                    .is_some_and(|constraint| constraint.is_material_transition);
                let source_constraint_indices = canonical_sources(
                    source_constraints
                        .iter()
                        .map(|constraint| constraint.constraint_index),
                );
                boundary_edges.push(arrangement.push_edge(
                    edge.start,
                    edge.end,
                    pending.owner,
                    pending.height_field_id,
                    opposite_owner,
                    opposite_height_field_id,
                    edge_use_counts.get(&edge.key).copied() == Some(1),
                    constrains_shared_height,
                    is_material_transition,
                    seam_source,
                    source_constraint_indices,
                ));
            }
            arrangement.push_region(
                pending.owner,
                pending.height_field_id,
                pending.outer_loop,
                pending.holes,
                boundary_edges,
                pending.area_m2,
                pending.seam_constraints,
            );
        }

        arrangement.reject_implicit_material_height_conflicts()?;
        Ok(arrangement)
    }

    pub(crate) fn attach_triangulation(
        &mut self,
        triangulation: &NodeTriangulationSolution,
    ) -> Result<(), NodeArrangementError> {
        if self.node_id != triangulation.node_id || self.piece_kind != triangulation.piece_kind {
            return Err(NodeArrangementError::InputSolutionMismatch {
                height_node_id: self.node_id,
                triangulation_node_id: triangulation.node_id,
                height_piece_kind: self.piece_kind,
                triangulation_piece_kind: triangulation.piece_kind,
            });
        }
        if self.regions.len() != triangulation.regions.len() {
            return Err(NodeArrangementError::TriangulationRegionCountMismatch {
                arrangement_region_count: self.regions.len(),
                triangulation_region_count: triangulation.regions.len(),
            });
        }

        for (region_index, triangulated_region) in triangulation.regions.iter().enumerate() {
            let region_id = NodeOwnedRegionId(region_index);
            let arrangement_region = self
                .regions
                .get(region_index)
                .ok_or(NodeArrangementError::MissingHeightRegion { region_index })?;
            if arrangement_region.owner != triangulated_region.owner {
                return Err(NodeArrangementError::RegionOwnerMismatch { region_index });
            }
            for triangle in &triangulated_region.triangles {
                let vertices = [
                    self.insert_triangulated_vertex(
                        region_index,
                        triangulated_region,
                        triangle.vertices[0],
                    )?,
                    self.insert_triangulated_vertex(
                        region_index,
                        triangulated_region,
                        triangle.vertices[1],
                    )?,
                    self.insert_triangulated_vertex(
                        region_index,
                        triangulated_region,
                        triangle.vertices[2],
                    )?,
                ];
                self.push_face(region_id, triangulated_region.owner, vertices);
            }
        }

        self.reject_implicit_material_height_conflicts()?;
        Ok(())
    }

    fn pending_region(
        &mut self,
        region_index: usize,
        region: &NodeHeightedRegion,
    ) -> Result<PendingArrangementRegion, NodeArrangementError> {
        if region.shape.is_empty() {
            return Err(NodeArrangementError::DegenerateRegionContour {
                region_index,
                contour_index: 0,
            });
        }

        let mut contours = Vec::with_capacity(region.shape.len());
        for (contour_index, contour) in region.shape.iter().enumerate() {
            contours.push(self.insert_heighted_contour(
                region_index,
                contour_index,
                region,
                contour,
            )?);
        }
        let mut contours = contours.into_iter();
        let outer_loop = contours
            .next()
            .ok_or(NodeArrangementError::DegenerateRegionContour {
                region_index,
                contour_index: 0,
            })?;
        let holes = contours.collect();
        Ok(PendingArrangementRegion {
            region_index,
            owner: region.owner,
            height_field_id: region.height_field_id,
            outer_loop,
            holes,
            area_m2: region.area_m2,
            seam_constraints: region.seam_constraints.clone(),
        })
    }

    fn insert_heighted_contour(
        &mut self,
        region_index: usize,
        contour_index: usize,
        region: &NodeHeightedRegion,
        contour: &[NodeHeightedVertex],
    ) -> Result<Vec<NodeArrangementVertexId>, NodeArrangementError> {
        if contour.len() < 3 {
            return Err(NodeArrangementError::DegenerateRegionContour {
                region_index,
                contour_index,
            });
        }

        contour
            .iter()
            .map(|vertex| {
                self.insert_vertex(
                    vertex.point_xz,
                    vertex.height_m,
                    [region.owner],
                    vertex.height_field_id,
                    [seam_source_for_owner(region.owner)],
                )
            })
            .collect()
    }

    fn insert_triangulated_vertex(
        &mut self,
        region_index: usize,
        region: &NodeTriangulatedRegion,
        vertex_index: usize,
    ) -> Result<NodeArrangementVertexId, NodeArrangementError> {
        let vertex = region.vertices.get(vertex_index).ok_or(
            NodeArrangementError::MissingTriangulatedVertex {
                region_index,
                vertex_index,
            },
        )?;
        self.insert_vertex(
            RoadVec2::new(vertex.point_world.x, vertex.point_world.z),
            vertex.point_world.y,
            [region.owner],
            vertex.height_field_id,
            [seam_source_for_owner(region.owner)],
        )
    }

    fn reject_implicit_material_height_conflicts(&self) -> Result<(), NodeArrangementError> {
        let mut vertices_by_key =
            BTreeMap::<NodeArrangementKey, Vec<NodeArrangementVertexId>>::new();
        for vertex in &self.vertices {
            vertices_by_key
                .entry(vertex.key)
                .or_default()
                .push(vertex.id);
        }

        for (key, vertex_ids) in vertices_by_key {
            for left_index in 0..vertex_ids.len() {
                for right_index in left_index + 1..vertex_ids.len() {
                    let left = &self.vertices[vertex_ids[left_index].0];
                    let right = &self.vertices[vertex_ids[right_index].0];
                    if left.height_key == right.height_key
                        || left.height_field_id == right.height_field_id
                        || owners_share_band_kind(&left.owners, &right.owners)
                    {
                        continue;
                    }
                    if !self.has_explicit_material_seam_at_key_between(
                        key,
                        &left.owners,
                        &right.owners,
                    ) && !self.has_explicit_material_seam_endpoint_path_at_key_between(
                        key,
                        &left.owners,
                        &right.owners,
                    ) {
                        return Err(NodeArrangementError::DuplicateVertexHeightConflict {
                            key,
                            existing_height_mm: left.height_key.0,
                            incoming_height_mm: right.height_key.0,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    fn has_explicit_material_seam_endpoint_path_at_key_between(
        &self,
        key: NodeArrangementKey,
        left_owners: &[NodeBandOwner],
        right_owners: &[NodeBandOwner],
    ) -> bool {
        let adjacency = self.material_seam_endpoint_owner_adjacency_at_key(key);
        if adjacency.is_empty() {
            return false;
        }

        let right_owners = right_owners.iter().copied().collect::<BTreeSet<_>>();
        let mut visited = BTreeSet::new();
        let mut pending = left_owners.to_vec();
        while let Some(owner) = pending.pop() {
            if !visited.insert(owner) {
                continue;
            }
            if right_owners.contains(&owner) {
                return true;
            }
            if let Some(neighbors) = adjacency.get(&owner) {
                pending.extend(neighbors.iter().copied());
            }
        }
        false
    }

    fn material_seam_endpoint_owner_adjacency_at_key(
        &self,
        key: NodeArrangementKey,
    ) -> BTreeMap<NodeBandOwner, BTreeSet<NodeBandOwner>> {
        let mut owners_by_constraint = BTreeMap::<usize, Vec<NodeBandOwner>>::new();
        for region in &self.regions {
            for constraint in &region.seam_constraints {
                if !constraint.is_material_transition
                    || !seam_constraint_touches_key(constraint, key)
                {
                    continue;
                }
                let owners = owners_for_material_seam_constraint(constraint, region.owner);
                merge_sorted_unique(
                    owners_by_constraint
                        .entry(constraint.constraint_index)
                        .or_default(),
                    owners,
                );
            }
        }

        let mut adjacency = BTreeMap::<NodeBandOwner, BTreeSet<NodeBandOwner>>::new();
        for mut owners in owners_by_constraint.into_values() {
            owners.sort_unstable();
            owners.dedup();
            for left_index in 0..owners.len() {
                for right_index in left_index + 1..owners.len() {
                    let left = owners[left_index];
                    let right = owners[right_index];
                    adjacency.entry(left).or_default().insert(right);
                    adjacency.entry(right).or_default().insert(left);
                }
            }
        }
        adjacency
    }

    fn has_explicit_material_seam_at_key_between(
        &self,
        key: NodeArrangementKey,
        left_owners: &[NodeBandOwner],
        right_owners: &[NodeBandOwner],
    ) -> bool {
        self.edges.iter().any(|edge| {
            edge.is_material_transition
                && !edge.source_constraint_indices.is_empty()
                && (self.piece_kind != RoadSurfaceVisualNodePieceKind::Terminal
                    || !self.edge_has_owner_pair_source_constraint(edge))
                && self.edge_touches_key(edge, key)
                && edge.opposite_owner.is_some_and(|opposite_owner| {
                    owner_sets_match_edge(left_owners, right_owners, edge.owner, opposite_owner)
                })
        })
    }

    fn edge_touches_key(&self, edge: &NodeArrangementEdge, key: NodeArrangementKey) -> bool {
        let Some(start) = self.vertices.get(edge.start.0) else {
            return false;
        };
        let Some(end) = self.vertices.get(edge.end.0) else {
            return false;
        };
        start.key == key || end.key == key
    }

    fn edge_has_owner_pair_source_constraint(&self, edge: &NodeArrangementEdge) -> bool {
        let Some(start) = self.vertices.get(edge.start.0).map(|vertex| vertex.key) else {
            return false;
        };
        let Some(end) = self.vertices.get(edge.end.0).map(|vertex| vertex.key) else {
            return false;
        };
        self.regions.iter().any(|region| {
            region.seam_constraints.iter().any(|constraint| {
                (constraint.owner.is_some() || constraint.opposite_owner.is_some())
                    && edge
                        .source_constraint_indices
                        .contains(&constraint.constraint_index)
                    && seam_constraint_covers_edge(constraint, start, end)
            })
        })
    }
}

fn seam_constraint_touches_key(
    constraint: &NodeRegionSeamConstraint,
    key: NodeArrangementKey,
) -> bool {
    let start = NodeArrangementKey::from_point(constraint.start_xz);
    let end = NodeArrangementKey::from_point(constraint.end_xz);
    point_key_lies_on_segment(key, start, end)
}

fn seam_constraint_covers_edge(
    constraint: &NodeRegionSeamConstraint,
    edge_start: NodeArrangementKey,
    edge_end: NodeArrangementKey,
) -> bool {
    let constraint_start = NodeArrangementKey::from_point(constraint.start_xz);
    let constraint_end = NodeArrangementKey::from_point(constraint.end_xz);
    point_key_lies_on_segment(edge_start, constraint_start, constraint_end)
        && point_key_lies_on_segment(edge_end, constraint_start, constraint_end)
}

impl NodeArrangementVertexId {
    pub(crate) fn index(self) -> usize {
        self.0
    }
}

impl NodeArrangementEdgeId {
    pub(crate) fn index(self) -> usize {
        self.0
    }
}

impl NodeArrangementVertex {
    pub(crate) fn point_xz(&self) -> RoadVec2 {
        self.point_xz
    }

    pub(crate) fn height_m(&self) -> f64 {
        self.height_m
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

    pub(crate) fn boundary_edges(&self) -> &[NodeArrangementEdgeId] {
        &self.boundary_edges
    }

    pub(crate) fn area_m2(&self) -> f32 {
        self.area_m2
    }
}

impl NodeArrangementEdge {
    pub(crate) fn start(&self) -> NodeArrangementVertexId {
        self.start
    }

    pub(crate) fn end(&self) -> NodeArrangementVertexId {
        self.end
    }

    pub(crate) fn owner(&self) -> NodeBandOwner {
        self.owner
    }

    pub(crate) fn is_exposed_boundary(&self) -> bool {
        self.exposed_boundary
    }

    pub(crate) fn seam_source(&self) -> NodeSeamSource {
        self.seam_source
    }

    pub(crate) fn is_explicit_vertical_step(&self) -> bool {
        self.is_material_transition
            && !self.constrains_shared_height
            && !self.source_constraint_indices.is_empty()
            && matches!(self.seam_source, NodeSeamSource::AsphaltCurbContact { .. })
            && self.opposite_owner.is_some_and(|opposite_owner| {
                owners_form_asphalt_curb_pair(self.owner, opposite_owner)
            })
    }
}

impl NodeArrangementFace {
    pub(crate) fn owner(&self) -> NodeBandOwner {
        self.owner
    }

    pub(crate) fn vertices(&self) -> [NodeArrangementVertexId; 3] {
        self.vertices
    }
}

fn quantize_m(value_m: f64) -> i64 {
    (value_m * NODE_ARRANGEMENT_KEY_SCALE).round() as i64
}

fn quantize_height_m(value_m: f64) -> i64 {
    (value_m * NODE_ARRANGEMENT_HEIGHT_SCALE).round() as i64
}

#[derive(Clone)]
struct PendingArrangementRegion {
    region_index: usize,
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
    outer_loop: Vec<NodeArrangementVertexId>,
    holes: Vec<Vec<NodeArrangementVertexId>>,
    area_m2: f32,
    seam_constraints: Vec<NodeRegionSeamConstraint>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeArrangementEdgeOwner {
    owner: NodeBandOwner,
    height_field_id: NodeBandHeightFieldId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeArrangementEdgeKey {
    start: NodeArrangementKey,
    end: NodeArrangementKey,
}

#[derive(Clone, Copy)]
struct PendingArrangementEdge {
    key: NodeArrangementEdgeKey,
    start: NodeArrangementVertexId,
    end: NodeArrangementVertexId,
}

impl PendingArrangementRegion {
    fn edge_count(&self) -> usize {
        self.outer_loop.len() + self.holes.iter().map(Vec::len).sum::<usize>()
    }

    fn loop_edges(&self, vertices: &[NodeArrangementVertex]) -> Vec<PendingArrangementEdge> {
        let mut edges = loop_edges(&self.outer_loop, vertices);
        for hole in &self.holes {
            edges.extend(loop_edges(hole, vertices));
        }
        edges
    }
}

fn loop_edges(
    loop_vertices: &[NodeArrangementVertexId],
    vertices: &[NodeArrangementVertex],
) -> Vec<PendingArrangementEdge> {
    if loop_vertices.len() < 2 {
        return Vec::new();
    }
    (0..loop_vertices.len())
        .filter_map(|index| {
            let start = loop_vertices[index];
            let end = loop_vertices[(index + 1) % loop_vertices.len()];
            let start_key = vertices.get(start.0)?.key;
            let end_key = vertices.get(end.0)?.key;
            (start != end && start_key != end_key).then_some(PendingArrangementEdge {
                key: NodeArrangementEdgeKey::new(start_key, end_key),
                start,
                end,
            })
        })
        .collect()
}

impl NodeArrangementEdgeKey {
    fn new(a: NodeArrangementKey, b: NodeArrangementKey) -> Self {
        if a <= b {
            Self { start: a, end: b }
        } else {
            Self { start: b, end: a }
        }
    }
}

fn source_constraints_for_edge<'a>(
    edge: PendingArrangementEdge,
    constraints: &'a [NodeRegionSeamConstraint],
    vertices: &[NodeArrangementVertex],
) -> Vec<&'a NodeRegionSeamConstraint> {
    let Some(start) = vertices.get(edge.start.0).map(|vertex| vertex.key) else {
        return Vec::new();
    };
    let Some(end) = vertices.get(edge.end.0).map(|vertex| vertex.key) else {
        return Vec::new();
    };
    let mut matches = constraints
        .iter()
        .filter(|constraint| {
            let constraint_start = NodeArrangementKey::from_point(constraint.start_xz);
            let constraint_end = NodeArrangementKey::from_point(constraint.end_xz);
            point_key_lies_on_segment(start, constraint_start, constraint_end)
                && point_key_lies_on_segment(end, constraint_start, constraint_end)
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|constraint| {
        (
            !constraint.constrains_shared_height,
            !constraint.is_material_transition,
            seam_source_priority(&constraint.seam_source),
            constraint.constraint_index,
        )
    });
    matches.dedup_by_key(|constraint| constraint.constraint_index);
    matches
}

fn source_constraints_are_ambiguous(constraints: &[&NodeRegionSeamConstraint]) -> bool {
    let Some(first) = constraints.first() else {
        return false;
    };
    let first_priority = seam_constraint_priority(first);
    constraints
        .iter()
        .skip(1)
        .take_while(|constraint| seam_constraint_priority(constraint) == first_priority)
        .any(|constraint| constraint.seam_source != first.seam_source)
}

fn seam_constraint_priority(constraint: &NodeRegionSeamConstraint) -> (bool, bool, usize) {
    (
        !constraint.constrains_shared_height,
        !constraint.is_material_transition,
        seam_source_priority(&constraint.seam_source),
    )
}

fn explicit_vertical_step_segment_from_constraint(
    constraint: &NodeRegionSeamConstraint,
) -> Option<NodeExplicitVerticalStepSegment> {
    if !constraint.is_material_transition
        || constraint.constrains_shared_height
        || !matches!(
            constraint.seam_source,
            NodeSeamSource::AsphaltCurbContact { .. }
        )
    {
        return None;
    }
    let owner = constraint.owner?;
    let opposite_owner = constraint.opposite_owner?;
    if !owners_form_asphalt_curb_pair(owner, opposite_owner) {
        return None;
    }
    NodeExplicitVerticalStepSegment::new(
        NodeArrangementKey::from_point(constraint.start_xz),
        NodeArrangementKey::from_point(constraint.end_xz),
        owner,
        opposite_owner,
    )
}

fn owners_for_material_seam_constraint(
    constraint: &NodeRegionSeamConstraint,
    fallback_owner: NodeBandOwner,
) -> Vec<NodeBandOwner> {
    match (constraint.owner, constraint.opposite_owner) {
        (Some(owner), Some(opposite_owner)) => vec![owner, opposite_owner],
        (Some(owner), None) | (None, Some(owner)) => vec![owner],
        (None, None) => vec![fallback_owner],
    }
}

pub(crate) fn seam_source_priority(source: &NodeSeamSource) -> usize {
    match source {
        NodeSeamSource::AsphaltCurbContact { .. } => 0,
        NodeSeamSource::CurbSidewalkContact { .. } => 1,
        NodeSeamSource::AsphaltBoundary { .. } => 2,
        NodeSeamSource::SidewalkOuter { .. } => 3,
        NodeSeamSource::FootprintBoundary { .. } => 4,
    }
}

fn point_key_lies_on_segment(
    point: NodeArrangementKey,
    start: NodeArrangementKey,
    end: NodeArrangementKey,
) -> bool {
    if point == start || point == end {
        return true;
    }
    if start == end {
        return false;
    }
    let dx = i128::from(end.x_key - start.x_key);
    let dz = i128::from(end.z_key - start.z_key);
    let px = i128::from(point.x_key - start.x_key);
    let pz = i128::from(point.z_key - start.z_key);
    if px * dz - pz * dx != 0 {
        return false;
    }
    let inside_x = if start.x_key == end.x_key {
        point.x_key == start.x_key
    } else {
        point.x_key > start.x_key.min(end.x_key) && point.x_key < start.x_key.max(end.x_key)
    };
    let inside_z = if start.z_key == end.z_key {
        point.z_key == start.z_key
    } else {
        point.z_key > start.z_key.min(end.z_key) && point.z_key < start.z_key.max(end.z_key)
    };
    inside_x && inside_z
}

fn seam_source_for_owner(owner: NodeBandOwner) -> NodeSeamSource {
    match owner.kind {
        RoadSurfaceBandKind::Carriageway => NodeSeamSource::AsphaltBoundary {
            owner_index: owner.owner_index,
        },
        RoadSurfaceBandKind::CurbOrShoulder => NodeSeamSource::AsphaltCurbContact {
            owner_index: owner.owner_index,
        },
        RoadSurfaceBandKind::Sidewalk => NodeSeamSource::SidewalkOuter {
            owner_index: owner.owner_index,
        },
        _ => NodeSeamSource::FootprintBoundary {
            owner_index: owner.owner_index,
        },
    }
}

fn canonical_non_empty_owners(
    key: NodeArrangementKey,
    owners: impl IntoIterator<Item = NodeBandOwner>,
) -> Result<Vec<NodeBandOwner>, NodeArrangementError> {
    let owners = canonical_sources(owners);
    if owners.is_empty() {
        return Err(NodeArrangementError::EmptyOwnerSet { key });
    }
    Ok(owners)
}

fn owners_share_band_kind(a: &[NodeBandOwner], b: &[NodeBandOwner]) -> bool {
    a.iter()
        .any(|a_owner| b.iter().any(|b_owner| a_owner.kind == b_owner.kind))
}

fn owners_share_non_curb_band_kind(a: &[NodeBandOwner], b: &[NodeBandOwner]) -> bool {
    a.iter().any(|a_owner| {
        a_owner.kind != RoadSurfaceBandKind::CurbOrShoulder
            && b.iter().any(|b_owner| a_owner.kind == b_owner.kind)
    })
}

fn owners_overlap(a: &[NodeBandOwner], b: &[NodeBandOwner]) -> bool {
    a.iter()
        .any(|a_owner| b.iter().any(|b_owner| a_owner == b_owner))
}

fn owners_form_asphalt_curb_pair(a: NodeBandOwner, b: NodeBandOwner) -> bool {
    matches!(
        (a.kind, b.kind),
        (
            RoadSurfaceBandKind::Carriageway,
            RoadSurfaceBandKind::CurbOrShoulder
        ) | (
            RoadSurfaceBandKind::CurbOrShoulder,
            RoadSurfaceBandKind::Carriageway
        )
    )
}

fn owner_sets_match_edge(
    left_owners: &[NodeBandOwner],
    right_owners: &[NodeBandOwner],
    edge_owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    (left_owners.contains(&edge_owner) && right_owners.contains(&opposite_owner))
        || (left_owners.contains(&opposite_owner) && right_owners.contains(&edge_owner))
}

fn canonical_sources<T>(sources: impl IntoIterator<Item = T>) -> Vec<T>
where
    T: Ord,
{
    let mut sources = sources.into_iter().collect::<Vec<_>>();
    sources.sort();
    sources.dedup();
    sources
}

fn merge_sorted_unique<T>(target: &mut Vec<T>, incoming: Vec<T>)
where
    T: Ord,
{
    if incoming.is_empty() {
        return;
    }
    target.extend(incoming);
    target.sort();
    target.dedup();
}

#[cfg(test)]
mod tests {
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
    fn duplicate_arrangement_vertex_key_rejects_same_band_height_conflict_across_sources() {
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

        let result = arrangement.insert_vertex(
            point,
            2.0,
            [owner(RoadSurfaceBandKind::Sidewalk, 1)],
            height_field_id(RoadSurfaceBandKind::Sidewalk, 1),
            [seam_source(1)],
        );

        assert!(matches!(
            result,
            Err(NodeArrangementError::DuplicateVertexHeightConflict { .. })
        ));
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
                [NodeSeamSource::AsphaltCurbContact { owner_index: 0 }],
            )
            .expect("lower curb rail vertex should insert");
        let raised = arrangement
            .insert_vertex(
                point,
                0.12,
                [owner(RoadSurfaceBandKind::CurbOrShoulder, 1)],
                height_field_id(RoadSurfaceBandKind::CurbOrShoulder, 1),
                [NodeSeamSource::CurbSidewalkContact { owner_index: 1 }],
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
    fn explicit_vertical_step_segments_use_constraint_owner_pair() {
        let carriageway = owner(RoadSurfaceBandKind::Carriageway, 2);
        let adjacent_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let terminal_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 6);
        let start = RoadVec2::new(1.0, 0.0);
        let end = RoadVec2::new(1.0, 2.0);
        let seam = NodeRegionSeamConstraint {
            constraint_index: 91,
            seam_source: NodeSeamSource::AsphaltCurbContact { owner_index: 6 },
            owner: Some(carriageway),
            opposite_owner: Some(terminal_curb),
            constrains_shared_height: false,
            is_material_transition: true,
            start_xz: start,
            end_xz: end,
        };
        let mut arrangement = NodeArrangement::new(11, RoadSurfaceVisualNodePieceKind::Terminal);

        arrangement.push_region(
            carriageway,
            height_field_id(RoadSurfaceBandKind::Carriageway, 2),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            1.0,
            vec![seam.clone()],
        );
        arrangement.push_region(
            adjacent_curb,
            height_field_id(RoadSurfaceBandKind::CurbOrShoulder, 1),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            1.0,
            vec![seam],
        );

        let segments = arrangement.explicit_vertical_step_segments();
        let expected = NodeExplicitVerticalStepSegment::new(
            NodeArrangementKey::from_point(start),
            NodeArrangementKey::from_point(end),
            carriageway,
            terminal_curb,
        )
        .expect("test segment is non-degenerate");
        let wrong = NodeExplicitVerticalStepSegment::new(
            NodeArrangementKey::from_point(start),
            NodeArrangementKey::from_point(end),
            carriageway,
            adjacent_curb,
        )
        .expect("test segment is non-degenerate");

        assert!(segments.contains(&expected));
        assert!(!segments.contains(&wrong));
    }

    #[test]
    fn arrangement_rejects_mismatched_explicit_seam_owner_pair() {
        let carriageway = owner(RoadSurfaceBandKind::Carriageway, 2);
        let adjacent_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let terminal_curb = owner(RoadSurfaceBandKind::CurbOrShoulder, 6);
        let seam = NodeRegionSeamConstraint {
            constraint_index: 92,
            seam_source: NodeSeamSource::AsphaltCurbContact { owner_index: 6 },
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
        }
    }
}
