//! Canonical node-arrangement identity and ownership data model.

#![allow(dead_code)]

use super::backend::RoadVec2;
use super::height::{NodeHeightSolution, NodeHeightedRegion, NodeHeightedVertex};
use super::triangulation::{NodeTriangulatedRegion, NodeTriangulationSolution};
use super::{IncidentEdgeSide, RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind};
use std::collections::BTreeMap;

const NODE_ARRANGEMENT_KEY_SCALE: f64 = 1000.0;

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
    x_mm: i64,
    z_mm: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct NodeArrangementHeightKey(i64);

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct NodeArrangementVertexContextKey {
    position: NodeArrangementKey,
    owners: Vec<NodeBandOwner>,
    height_source: NodeHeightSource,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct NodeBandOwner {
    kind: RoadSurfaceBandKind,
    owner_index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) enum NodeHeightSource {
    IncidentMouthBand {
        edge_idx: usize,
        side: IncidentEdgeSide,
        band_index: usize,
    },
    EndpointBand {
        edge_idx: usize,
        side: IncidentEdgeSide,
        band_index: usize,
    },
    ArrangementConstraint {
        constraint_index: usize,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) enum NodeSeamSource {
    SpanHandoff {
        edge_idx: usize,
        side: IncidentEdgeSide,
    },
    AsphaltBoundary {
        owner_index: usize,
    },
    AsphaltCurbContact {
        owner_index: usize,
    },
    CurbSidewalkContact {
        owner_index: usize,
    },
    SidewalkOuter {
        owner_index: usize,
    },
    FootprintBoundary {
        owner_index: usize,
    },
    TerminalEndBand {
        edge_idx: usize,
        side: IncidentEdgeSide,
        band_index: usize,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeRegionSeamConstraint {
    pub(crate) constraint_index: usize,
    pub(crate) seam_source: NodeSeamSource,
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
    height_source: NodeHeightSource,
    seam_sources: Vec<NodeSeamSource>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeArrangementEdge {
    id: NodeArrangementEdgeId,
    start: NodeArrangementVertexId,
    end: NodeArrangementVertexId,
    owner: NodeBandOwner,
    opposite_owner: Option<NodeBandOwner>,
    exposed_boundary: bool,
    seam_source: NodeSeamSource,
    source_constraint_indices: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeOwnedRegion {
    id: NodeOwnedRegionId,
    owner: NodeBandOwner,
    outer_loop: Vec<NodeArrangementVertexId>,
    holes: Vec<Vec<NodeArrangementVertexId>>,
    boundary_edges: Vec<NodeArrangementEdgeId>,
    height_source: NodeHeightSource,
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
            x_mm: quantize_m(point.x),
            z_mm: quantize_m(point.y),
        }
    }

    pub(crate) fn x_mm(self) -> i64 {
        self.x_mm
    }

    pub(crate) fn z_mm(self) -> i64 {
        self.z_mm
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

    pub(crate) fn insert_vertex(
        &mut self,
        point_xz: RoadVec2,
        height_m: f64,
        owners: impl IntoIterator<Item = NodeBandOwner>,
        height_source: NodeHeightSource,
        seam_sources: impl IntoIterator<Item = NodeSeamSource>,
    ) -> Result<NodeArrangementVertexId, NodeArrangementError> {
        let key = NodeArrangementKey::from_point(point_xz);
        let height_key = NodeArrangementHeightKey(quantize_m(height_m));
        let owners = canonical_non_empty_owners(key, owners)?;
        let seam_sources = canonical_sources(seam_sources);
        let context_key = NodeArrangementVertexContextKey {
            position: key,
            owners: owners.clone(),
            height_source: height_source.clone(),
        };

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
            height_source,
            seam_sources,
        ))
    }

    fn push_vertex(
        &mut self,
        key: NodeArrangementKey,
        context_key: NodeArrangementVertexContextKey,
        point_xz: RoadVec2,
        height_m: f64,
        height_key: NodeArrangementHeightKey,
        owners: Vec<NodeBandOwner>,
        height_source: NodeHeightSource,
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
            height_source,
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
        opposite_owner: Option<NodeBandOwner>,
        exposed_boundary: bool,
        seam_source: NodeSeamSource,
        source_constraint_indices: Vec<usize>,
    ) -> NodeArrangementEdgeId {
        let id = NodeArrangementEdgeId(self.edges.len());
        self.edges.push(NodeArrangementEdge {
            id,
            start,
            end,
            owner,
            opposite_owner,
            exposed_boundary,
            seam_source,
            source_constraint_indices,
        });
        id
    }

    pub(crate) fn push_region(
        &mut self,
        owner: NodeBandOwner,
        outer_loop: Vec<NodeArrangementVertexId>,
        holes: Vec<Vec<NodeArrangementVertexId>>,
        boundary_edges: Vec<NodeArrangementEdgeId>,
        height_source: NodeHeightSource,
        seam_constraints: Vec<NodeRegionSeamConstraint>,
    ) -> NodeOwnedRegionId {
        let id = NodeOwnedRegionId(self.regions.len());
        self.regions.push(NodeOwnedRegion {
            id,
            owner,
            outer_loop,
            holes,
            boundary_edges,
            height_source,
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

    pub(crate) fn from_height_solution_and_triangulation(
        heights: &NodeHeightSolution,
        triangulation: &NodeTriangulationSolution,
    ) -> Result<Self, NodeArrangementError> {
        if heights.node_id != triangulation.node_id
            || heights.piece_kind != triangulation.piece_kind
        {
            return Err(NodeArrangementError::InputSolutionMismatch {
                height_node_id: heights.node_id,
                triangulation_node_id: triangulation.node_id,
                height_piece_kind: heights.piece_kind,
                triangulation_piece_kind: triangulation.piece_kind,
            });
        }

        let mut arrangement = Self::new(heights.node_id, heights.piece_kind);
        let mut pending_regions = Vec::with_capacity(triangulation.regions.len());
        let mut edge_owners = BTreeMap::<NodeArrangementEdgeKey, Vec<NodeBandOwner>>::new();
        let mut edge_use_counts = BTreeMap::<NodeArrangementEdgeKey, usize>::new();

        for (region_index, triangulated_region) in triangulation.regions.iter().enumerate() {
            let height_region = heights
                .regions
                .get(region_index)
                .ok_or(NodeArrangementError::MissingHeightRegion { region_index })?;
            validate_region_pair(region_index, height_region, triangulated_region)?;
            let pending = arrangement.pending_region(region_index, height_region)?;
            for edge in pending.loop_edges(&arrangement.vertices) {
                *edge_use_counts.entry(edge.key).or_default() += 1;
                edge_owners
                    .entry(edge.key)
                    .and_modify(|owners| merge_sorted_unique(owners, vec![pending.owner]))
                    .or_insert_with(|| vec![pending.owner]);
            }
            pending_regions.push(pending);
        }

        let mut arrangement_region_ids = Vec::with_capacity(pending_regions.len());
        for pending in pending_regions {
            let mut boundary_edges = Vec::with_capacity(pending.edge_count());
            for edge in pending.loop_edges(&arrangement.vertices) {
                let opposite_owner = edge_owners.get(&edge.key).and_then(|owners| {
                    owners.iter().copied().find(|owner| *owner != pending.owner)
                });
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
                    .unwrap_or_else(|| seam_source_for_edge(pending.owner, opposite_owner));
                let source_constraint_indices = canonical_sources(
                    source_constraints
                        .iter()
                        .map(|constraint| constraint.constraint_index),
                );
                boundary_edges.push(arrangement.push_edge(
                    edge.start,
                    edge.end,
                    pending.owner,
                    opposite_owner,
                    edge_use_counts.get(&edge.key).copied() == Some(1),
                    seam_source,
                    source_constraint_indices,
                ));
            }
            let region_id = arrangement.push_region(
                pending.owner,
                pending.outer_loop,
                pending.holes,
                boundary_edges,
                primary_height_source(&pending.height_sources),
                pending.seam_constraints,
            );
            arrangement_region_ids.push(region_id);
        }

        for (region_index, triangulated_region) in triangulation.regions.iter().enumerate() {
            let region_id = arrangement_region_ids[region_index];
            for triangle in &triangulated_region.triangles {
                let vertices = [
                    arrangement.insert_triangulated_vertex(
                        region_index,
                        triangulated_region,
                        triangle.vertices[0],
                    )?,
                    arrangement.insert_triangulated_vertex(
                        region_index,
                        triangulated_region,
                        triangle.vertices[1],
                    )?,
                    arrangement.insert_triangulated_vertex(
                        region_index,
                        triangulated_region,
                        triangle.vertices[2],
                    )?,
                ];
                arrangement.push_face(region_id, triangulated_region.owner, vertices);
            }
        }

        Ok(arrangement)
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
            outer_loop,
            holes,
            height_sources: region.height_sources.clone(),
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
                    primary_height_source(&vertex.height_sources),
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
            primary_height_source(&vertex.height_sources),
            [seam_source_for_owner(region.owner)],
        )
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
    pub(crate) fn point_xz(&self) -> RoadVec2 {
        self.point_xz
    }

    pub(crate) fn height_m(&self) -> f64 {
        self.height_m
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

    pub(crate) fn opposite_owner(&self) -> Option<NodeBandOwner> {
        self.opposite_owner
    }

    pub(crate) fn is_exposed_boundary(&self) -> bool {
        self.exposed_boundary
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

fn quantize_m(value_m: f64) -> i64 {
    (value_m * NODE_ARRANGEMENT_KEY_SCALE).round() as i64
}

#[derive(Clone)]
struct PendingArrangementRegion {
    region_index: usize,
    owner: NodeBandOwner,
    outer_loop: Vec<NodeArrangementVertexId>,
    holes: Vec<Vec<NodeArrangementVertexId>>,
    height_sources: Vec<NodeHeightSource>,
    seam_constraints: Vec<NodeRegionSeamConstraint>,
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

pub(crate) fn seam_source_priority(source: &NodeSeamSource) -> usize {
    match source {
        NodeSeamSource::SpanHandoff { .. } => 0,
        NodeSeamSource::AsphaltCurbContact { .. } => 1,
        NodeSeamSource::CurbSidewalkContact { .. } => 2,
        NodeSeamSource::AsphaltBoundary { .. } => 3,
        NodeSeamSource::TerminalEndBand { .. } => 4,
        NodeSeamSource::SidewalkOuter { .. } => 5,
        NodeSeamSource::FootprintBoundary { .. } => 6,
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
    let dx = i128::from(end.x_mm - start.x_mm);
    let dz = i128::from(end.z_mm - start.z_mm);
    let px = i128::from(point.x_mm - start.x_mm);
    let pz = i128::from(point.z_mm - start.z_mm);
    if px * dz - pz * dx != 0 {
        return false;
    }
    let inside_x = if start.x_mm == end.x_mm {
        point.x_mm == start.x_mm
    } else {
        point.x_mm > start.x_mm.min(end.x_mm) && point.x_mm < start.x_mm.max(end.x_mm)
    };
    let inside_z = if start.z_mm == end.z_mm {
        point.z_mm == start.z_mm
    } else {
        point.z_mm > start.z_mm.min(end.z_mm) && point.z_mm < start.z_mm.max(end.z_mm)
    };
    inside_x && inside_z
}

fn validate_region_pair(
    region_index: usize,
    height_region: &NodeHeightedRegion,
    triangulated_region: &NodeTriangulatedRegion,
) -> Result<(), NodeArrangementError> {
    if height_region.kind == triangulated_region.kind
        && height_region.owner == triangulated_region.owner
    {
        return Ok(());
    }
    Err(NodeArrangementError::RegionOwnerMismatch { region_index })
}

fn primary_height_source(sources: &[NodeHeightSource]) -> NodeHeightSource {
    sources
        .first()
        .cloned()
        .unwrap_or(NodeHeightSource::ArrangementConstraint {
            constraint_index: 0,
        })
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

fn seam_source_for_edge(
    owner: NodeBandOwner,
    opposite_owner: Option<NodeBandOwner>,
) -> NodeSeamSource {
    let Some(opposite_owner) = opposite_owner else {
        return seam_source_for_owner(owner);
    };
    match (owner.kind, opposite_owner.kind) {
        (RoadSurfaceBandKind::Carriageway, RoadSurfaceBandKind::CurbOrShoulder)
        | (RoadSurfaceBandKind::CurbOrShoulder, RoadSurfaceBandKind::Carriageway) => {
            NodeSeamSource::AsphaltCurbContact {
                owner_index: owner.owner_index,
            }
        }
        (RoadSurfaceBandKind::Carriageway, _) | (_, RoadSurfaceBandKind::Carriageway) => {
            NodeSeamSource::AsphaltBoundary {
                owner_index: owner.owner_index,
            }
        }
        (RoadSurfaceBandKind::CurbOrShoulder, RoadSurfaceBandKind::Sidewalk)
        | (RoadSurfaceBandKind::Sidewalk, RoadSurfaceBandKind::CurbOrShoulder) => {
            NodeSeamSource::CurbSidewalkContact {
                owner_index: owner.owner_index,
            }
        }
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
    use super::super::triangulation::{NodeTriangulatedTriangle, NodeTriangulatedVertex};
    use super::*;

    fn owner(kind: RoadSurfaceBandKind, owner_index: usize) -> NodeBandOwner {
        NodeBandOwner::new(kind, owner_index)
    }

    fn height_source() -> NodeHeightSource {
        NodeHeightSource::IncidentMouthBand {
            edge_idx: 7,
            side: IncidentEdgeSide::Start,
            band_index: 2,
        }
    }

    fn other_height_source() -> NodeHeightSource {
        NodeHeightSource::EndpointBand {
            edge_idx: 8,
            side: IncidentEdgeSide::End,
            band_index: 3,
        }
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
                height_source(),
                [seam_source(2)],
            )
            .expect("first vertex should insert");
        let second = arrangement
            .insert_vertex(
                point,
                3.25,
                [owner(RoadSurfaceBandKind::Sidewalk, 2)],
                height_source(),
                [seam_source(2)],
            )
            .expect("matching context vertex should merge");

        assert_eq!(first, second);
        let vertex = &arrangement.vertices()[first.0];
        assert_eq!(vertex.owners, vec![owner(RoadSurfaceBandKind::Sidewalk, 2)]);
        assert_eq!(vertex.seam_sources, vec![seam_source(2)]);
    }

    #[test]
    fn duplicate_arrangement_vertex_key_keeps_distinct_height_source_contexts() {
        let mut arrangement = NodeArrangement::new(42, RoadSurfaceVisualNodePieceKind::JunctionN);
        let point = RoadVec2::new(12.345, -6.789);

        let first = arrangement
            .insert_vertex(
                point,
                3.25,
                [owner(RoadSurfaceBandKind::Sidewalk, 2)],
                height_source(),
                [seam_source(2)],
            )
            .expect("first vertex should insert");
        let second = arrangement
            .insert_vertex(
                point,
                3.25,
                [owner(RoadSurfaceBandKind::CurbOrShoulder, 1)],
                other_height_source(),
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
                height_source(),
                [seam_source(0)],
            )
            .expect("first vertex should insert");

        let result = arrangement.insert_vertex(
            point,
            1.01,
            [owner(RoadSurfaceBandKind::Sidewalk, 0)],
            height_source(),
            [seam_source(0)],
        );

        assert!(matches!(
            result,
            Err(NodeArrangementError::DuplicateVertexHeightConflict { .. })
        ));
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
                height_source(),
                [seam_source(0)],
            )
            .expect("first vertex should insert");
        let high = arrangement
            .insert_vertex(
                point,
                2.0,
                [owner(RoadSurfaceBandKind::Sidewalk, 1)],
                other_height_source(),
                [seam_source(1)],
            )
            .expect("explicit owner context keeps steep endpoint-height duplicates deterministic");

        assert_ne!(low, high);
        assert_eq!(arrangement.vertices().len(), 2);
    }

    #[test]
    fn arrangement_edges_match_opposite_owners_by_canonical_xz_segment() {
        let carriageway = owner(RoadSurfaceBandKind::Carriageway, 0);
        let sidewalk = owner(RoadSurfaceBandKind::Sidewalk, 1);
        let generic_seam = NodeRegionSeamConstraint {
            constraint_index: 2,
            seam_source: NodeSeamSource::FootprintBoundary { owner_index: 0 },
            constrains_shared_height: true,
            is_material_transition: false,
            start_xz: RoadVec2::new(1.0, 0.0),
            end_xz: RoadVec2::new(1.0, 1.0),
        };
        let shared_seam = NodeRegionSeamConstraint {
            constraint_index: 17,
            seam_source: NodeSeamSource::AsphaltBoundary { owner_index: 0 },
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
        let triangulation = NodeTriangulationSolution {
            node_id: 11,
            piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
            regions: vec![
                test_triangulated_region(
                    RoadSurfaceBandKind::Carriageway,
                    carriageway,
                    vec![
                        road_vertex(0.0, 0.0, 0.0),
                        road_vertex(1.0, 0.0, 0.0),
                        road_vertex(1.0, 0.0, 1.0),
                        road_vertex(0.0, 0.0, 1.0),
                    ],
                ),
                test_triangulated_region(
                    RoadSurfaceBandKind::Sidewalk,
                    sidewalk,
                    vec![
                        road_vertex(1.0, 0.0, 0.0),
                        road_vertex(2.0, 0.0, 0.0),
                        road_vertex(2.0, 0.0, 1.0),
                        road_vertex(1.0, 0.0, 1.0),
                    ],
                ),
            ],
        };

        let arrangement =
            NodeArrangement::from_height_solution_and_triangulation(&heights, &triangulation)
                .expect("matching height and triangulation should arrange");

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
    fn shared_arrangement_edge_reports_missing_source_constraint() {
        let carriageway = owner(RoadSurfaceBandKind::Carriageway, 0);
        let sidewalk = owner(RoadSurfaceBandKind::Sidewalk, 1);
        let heights = two_region_height_solution(carriageway, sidewalk, Vec::new(), Vec::new());
        let triangulation = two_region_triangulation(carriageway, sidewalk);

        let arrangement =
            NodeArrangement::from_height_solution_and_triangulation(&heights, &triangulation)
                .expect("missing seam source is diagnostic-only until the full node model hardcut");

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
            constrains_shared_height: true,
            is_material_transition: true,
            start_xz: RoadVec2::new(1.0, 0.0),
            end_xz: RoadVec2::new(1.0, 1.0),
        };
        let second = NodeRegionSeamConstraint {
            constraint_index: 21,
            seam_source: NodeSeamSource::AsphaltBoundary { owner_index: 1 },
            constrains_shared_height: true,
            is_material_transition: true,
            start_xz: RoadVec2::new(1.0, 0.0),
            end_xz: RoadVec2::new(1.0, 1.0),
        };
        let seams = vec![first, second];
        let heights = two_region_height_solution(carriageway, sidewalk, seams.clone(), seams);
        let triangulation = two_region_triangulation(carriageway, sidewalk);

        let arrangement =
            NodeArrangement::from_height_solution_and_triangulation(&heights, &triangulation)
                .expect(
                    "ambiguous seam source is diagnostic-only until the full node model hardcut",
                );

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
            height_source(),
            [seam_source(0)],
        );

        assert!(matches!(
            result,
            Err(NodeArrangementError::EmptyOwnerSet { .. })
        ));
    }

    fn test_height_region(
        kind: RoadSurfaceBandKind,
        owner: NodeBandOwner,
        contour: Vec<NodeHeightedVertex>,
    ) -> NodeHeightedRegion {
        test_height_region_with_seams(kind, owner, contour, Vec::new())
    }

    fn two_region_height_solution(
        carriageway: NodeBandOwner,
        sidewalk: NodeBandOwner,
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
                        height_vertex(0.0, 0.0, 0.0),
                        height_vertex(1.0, 0.0, 0.0),
                        height_vertex(1.0, 1.0, 0.0),
                        height_vertex(0.0, 1.0, 0.0),
                    ],
                    carriageway_seams,
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
                    sidewalk_seams,
                ),
            ],
        }
    }

    fn two_region_triangulation(
        carriageway: NodeBandOwner,
        sidewalk: NodeBandOwner,
    ) -> NodeTriangulationSolution {
        NodeTriangulationSolution {
            node_id: 11,
            piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
            regions: vec![
                test_triangulated_region(
                    RoadSurfaceBandKind::Carriageway,
                    carriageway,
                    vec![
                        road_vertex(0.0, 0.0, 0.0),
                        road_vertex(1.0, 0.0, 0.0),
                        road_vertex(1.0, 0.0, 1.0),
                        road_vertex(0.0, 0.0, 1.0),
                    ],
                ),
                test_triangulated_region(
                    RoadSurfaceBandKind::Sidewalk,
                    sidewalk,
                    vec![
                        road_vertex(1.0, 0.0, 0.0),
                        road_vertex(2.0, 0.0, 0.0),
                        road_vertex(2.0, 0.0, 1.0),
                        road_vertex(1.0, 0.0, 1.0),
                    ],
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
        NodeHeightedRegion {
            kind,
            owner,
            source_mouth_order_index: owner.owner_index(),
            source_band_index: owner.owner_index(),
            shape: vec![contour],
            area_m2: 1.0,
            height_sources: vec![height_source()],
            seam_constraints,
        }
    }

    fn test_triangulated_region(
        kind: RoadSurfaceBandKind,
        owner: NodeBandOwner,
        vertices: Vec<NodeTriangulatedVertex>,
    ) -> NodeTriangulatedRegion {
        NodeTriangulatedRegion {
            kind,
            owner,
            source_mouth_order_index: owner.owner_index(),
            source_band_index: owner.owner_index(),
            vertices,
            boundary_constraints: vec![[0, 1], [1, 2], [2, 3], [3, 0]],
            triangles: vec![
                NodeTriangulatedTriangle {
                    vertices: [0, 1, 2],
                },
                NodeTriangulatedTriangle {
                    vertices: [0, 2, 3],
                },
            ],
            area_m2: 1.0,
            height_sources: vec![height_source()],
        }
    }

    fn height_vertex(x: f64, z: f64, height_m: f64) -> NodeHeightedVertex {
        NodeHeightedVertex {
            point_xz: RoadVec2::new(x, z),
            height_m,
            height_sources: vec![height_source()],
        }
    }

    fn road_vertex(x: f64, y: f64, z: f64) -> NodeTriangulatedVertex {
        NodeTriangulatedVertex {
            point_world: super::super::backend::RoadVec3::new(x, y, z),
            height_sources: vec![height_source()],
        }
    }
}
