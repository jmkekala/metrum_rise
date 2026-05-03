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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct NodeArrangementVertexHeightedKey {
    position: NodeArrangementKey,
    height: NodeArrangementHeightKey,
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
    vertex_by_heighted_key: BTreeMap<NodeArrangementVertexHeightedKey, NodeArrangementVertexId>,
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
            vertex_by_key: BTreeMap::new(),
            vertex_by_heighted_key: BTreeMap::new(),
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
        let heighted_key = NodeArrangementVertexHeightedKey {
            position: key,
            height: height_key,
        };
        let owners = canonical_non_empty_owners(key, owners)?;
        let seam_sources = canonical_sources(seam_sources);

        if let Some(existing_id) = self.vertex_by_key.get(&key).copied() {
            let existing = &mut self.vertices[existing_id.0];
            if existing.height_key != height_key {
                if self.piece_kind != RoadSurfaceVisualNodePieceKind::Bend {
                    if let Some(heighted_existing_id) =
                        self.vertex_by_heighted_key.get(&heighted_key).copied()
                    {
                        let existing = &mut self.vertices[heighted_existing_id.0];
                        merge_sorted_unique(&mut existing.owners, owners);
                        merge_sorted_unique(&mut existing.seam_sources, seam_sources);
                        return Ok(heighted_existing_id);
                    }
                    return Ok(self.push_vertex(
                        key,
                        heighted_key,
                        point_xz,
                        height_m,
                        height_key,
                        owners,
                        height_source,
                        seam_sources,
                        false,
                    ));
                }
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
            heighted_key,
            point_xz,
            height_m,
            height_key,
            owners,
            height_source,
            seam_sources,
            true,
        ))
    }

    fn push_vertex(
        &mut self,
        key: NodeArrangementKey,
        heighted_key: NodeArrangementVertexHeightedKey,
        point_xz: RoadVec2,
        height_m: f64,
        height_key: NodeArrangementHeightKey,
        owners: Vec<NodeBandOwner>,
        height_source: NodeHeightSource,
        seam_sources: Vec<NodeSeamSource>,
        insert_position_key: bool,
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
        if insert_position_key {
            self.vertex_by_key.insert(key, id);
        }
        self.vertex_by_heighted_key.insert(heighted_key, id);
        id
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

        for (region_index, triangulated_region) in triangulation.regions.iter().enumerate() {
            let height_region = heights
                .regions
                .get(region_index)
                .ok_or(NodeArrangementError::MissingHeightRegion { region_index })?;
            validate_region_pair(region_index, height_region, triangulated_region)?;
            let pending = arrangement.pending_region(region_index, height_region)?;
            for edge in pending.loop_edges() {
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
            for edge in pending.loop_edges() {
                let opposite_owner = edge_owners.get(&edge.key).and_then(|owners| {
                    owners.iter().copied().find(|owner| *owner != pending.owner)
                });
                let seam_source = seam_source_for_edge(pending.owner, opposite_owner);
                boundary_edges.push(arrangement.push_edge(
                    edge.start,
                    edge.end,
                    pending.owner,
                    opposite_owner,
                    seam_source,
                ));
            }
            let region_id = arrangement.push_region(
                pending.owner,
                pending.outer_loop,
                pending.holes,
                boundary_edges,
                primary_height_source(&pending.height_sources),
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
            owner: region.owner,
            outer_loop,
            holes,
            height_sources: region.height_sources.clone(),
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
    owner: NodeBandOwner,
    outer_loop: Vec<NodeArrangementVertexId>,
    holes: Vec<Vec<NodeArrangementVertexId>>,
    height_sources: Vec<NodeHeightSource>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeArrangementEdgeKey {
    start: NodeArrangementVertexId,
    end: NodeArrangementVertexId,
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

    fn loop_edges(&self) -> Vec<PendingArrangementEdge> {
        let mut edges = loop_edges(&self.outer_loop);
        for hole in &self.holes {
            edges.extend(loop_edges(hole));
        }
        edges
    }
}

fn loop_edges(loop_vertices: &[NodeArrangementVertexId]) -> Vec<PendingArrangementEdge> {
    if loop_vertices.len() < 2 {
        return Vec::new();
    }
    (0..loop_vertices.len())
        .filter_map(|index| {
            let start = loop_vertices[index];
            let end = loop_vertices[(index + 1) % loop_vertices.len()];
            (start != end).then_some(PendingArrangementEdge {
                key: NodeArrangementEdgeKey::new(start, end),
                start,
                end,
            })
        })
        .collect()
}

impl NodeArrangementEdgeKey {
    fn new(a: NodeArrangementVertexId, b: NodeArrangementVertexId) -> Self {
        if a <= b {
            Self { start: a, end: b }
        } else {
            Self { start: b, end: a }
        }
    }
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
    fn non_bend_arrangement_keeps_height_distinct_duplicate_vertices() {
        let mut arrangement = NodeArrangement::new(7, RoadSurfaceVisualNodePieceKind::JunctionN);
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
                height_source(),
                [seam_source(1)],
            )
            .expect("JunctionN keeps steep endpoint-height duplicates deterministic");
        let high_again = arrangement
            .insert_vertex(
                point,
                2.0,
                [owner(RoadSurfaceBandKind::CurbOrShoulder, 2)],
                height_source(),
                [seam_source(2)],
            )
            .expect("matching duplicate height should merge into the height-distinct vertex");

        assert_ne!(low, high);
        assert_eq!(high, high_again);
        assert_eq!(arrangement.vertices().len(), 2);
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
