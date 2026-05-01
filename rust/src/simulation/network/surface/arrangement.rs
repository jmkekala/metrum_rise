//! Canonical node-arrangement identity and ownership data model.

#![allow(dead_code)]

use super::backend::RoadVec2;
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
    seam_source: NodeSeamSource,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeOwnedRegion {
    id: NodeOwnedRegionId,
    owner: NodeBandOwner,
    outer_loop: Vec<NodeArrangementVertexId>,
    holes: Vec<Vec<NodeArrangementVertexId>>,
    boundary_edges: Vec<NodeArrangementEdgeId>,
    height_source: NodeHeightSource,
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
    vertex_by_key: BTreeMap<NodeArrangementKey, NodeArrangementVertexId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum NodeArrangementError {
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
}

impl NodeBandOwner {
    pub(crate) fn new(kind: RoadSurfaceBandKind, owner_index: usize) -> Self {
        Self { kind, owner_index }
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
            vertex_by_key: BTreeMap::new(),
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

        if let Some(existing_id) = self.vertex_by_key.get(&key).copied() {
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
        self.vertex_by_key.insert(key, id);
        Ok(id)
    }

    pub(crate) fn push_edge(
        &mut self,
        start: NodeArrangementVertexId,
        end: NodeArrangementVertexId,
        owner: NodeBandOwner,
        opposite_owner: Option<NodeBandOwner>,
        seam_source: NodeSeamSource,
    ) -> NodeArrangementEdgeId {
        let id = NodeArrangementEdgeId(self.edges.len());
        self.edges.push(NodeArrangementEdge {
            id,
            start,
            end,
            owner,
            opposite_owner,
            seam_source,
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
    ) -> NodeOwnedRegionId {
        let id = NodeOwnedRegionId(self.regions.len());
        self.regions.push(NodeOwnedRegion {
            id,
            owner,
            outer_loop,
            holes,
            boundary_edges,
            height_source,
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
}

fn quantize_m(value_m: f64) -> i64 {
    (value_m * NODE_ARRANGEMENT_KEY_SCALE).round() as i64
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

    fn seam_source(owner_index: usize) -> NodeSeamSource {
        NodeSeamSource::FootprintBoundary { owner_index }
    }

    #[test]
    fn duplicate_arrangement_vertex_key_merges_matching_height_context() {
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
                height_source(),
                [seam_source(1)],
            )
            .expect("matching-height vertex should merge");

        assert_eq!(first, second);
        let vertex = &arrangement.vertices()[first.0];
        assert_eq!(vertex.owners.len(), 2);
        assert_eq!(vertex.seam_sources.len(), 2);
    }

    #[test]
    fn duplicate_arrangement_vertex_key_rejects_height_conflict() {
        let mut arrangement = NodeArrangement::new(7, RoadSurfaceVisualNodePieceKind::Bend);
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
}
