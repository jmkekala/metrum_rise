//! Height-owned arrangement region construction.

use super::super::height::{NodeHeightedRegion, NodeHeightedVertex};
use super::build::quantize_height_m;
use super::edges::{PendingArrangementEdge, loop_edges};
use super::seams::{NodeRegionSeamConstraint, NodeSeamSource};
use super::{
    NodeArrangement, NodeArrangementEdgeId, NodeArrangementError, NodeArrangementVertex,
    NodeArrangementVertexId, NodeBandHeightFieldId, NodeBandOwner, NodeOwnedRegion,
    NodeOwnedRegionId,
};

#[derive(Clone)]
pub(super) struct PendingArrangementRegion {
    pub(super) region_index: usize,
    pub(super) owner: NodeBandOwner,
    pub(super) height_field_id: NodeBandHeightFieldId,
    pub(super) outer_loop: Vec<NodeArrangementVertexId>,
    pub(super) holes: Vec<Vec<NodeArrangementVertexId>>,
    pub(super) area_m2: f32,
    pub(super) seam_constraints: Vec<NodeRegionSeamConstraint>,
}

impl PendingArrangementRegion {
    pub(super) fn edge_count(&self) -> usize {
        self.outer_loop.len() + self.holes.iter().map(Vec::len).sum::<usize>()
    }

    pub(super) fn loop_edges(
        &self,
        vertices: &[NodeArrangementVertex],
    ) -> Vec<PendingArrangementEdge> {
        let mut edges = loop_edges(&self.outer_loop, vertices);
        for hole in &self.holes {
            edges.extend(loop_edges(hole, vertices));
        }
        edges
    }
}

impl NodeArrangement {
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

    pub(super) fn pending_region(
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

        let mut loop_vertices = contour
            .iter()
            .map(|vertex| {
                let grade_authority = vertex.grade_authority.ok_or_else(|| {
                    NodeArrangementError::MissingGradeAuthority {
                        region_index,
                        contour_index,
                        key: super::NodeArrangementKey::from_point(vertex.point_xz),
                        owner: region.owner,
                        height_field_id: vertex.height_field_id,
                        height_mm: quantize_height_m(vertex.height_m),
                    }
                })?;
                self.insert_vertex_with_grade_authority(
                    vertex.point_xz,
                    vertex.height_m,
                    [region.owner],
                    vertex.height_field_id,
                    [NodeSeamSource::for_owner(region.owner)],
                    grade_authority,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        clean_arrangement_loop_vertices(&mut loop_vertices, &self.vertices);
        if loop_vertices.len() < 3 {
            return Err(NodeArrangementError::DegenerateRegionContour {
                region_index,
                contour_index,
            });
        }
        Ok(loop_vertices)
    }
}

fn clean_arrangement_loop_vertices(
    loop_vertices: &mut Vec<NodeArrangementVertexId>,
    vertices: &[NodeArrangementVertex],
) {
    loop {
        let starting_len = loop_vertices.len();
        remove_consecutive_identical_arrangement_vertices(loop_vertices, vertices);
        remove_immediate_backtracking_arrangement_vertices(loop_vertices, vertices);
        if loop_vertices.len() == starting_len {
            break;
        }
        if loop_vertices.len() < 3 {
            break;
        }
    }
}

fn remove_consecutive_identical_arrangement_vertices(
    loop_vertices: &mut Vec<NodeArrangementVertexId>,
    vertices: &[NodeArrangementVertex],
) {
    loop_vertices.dedup_by(|left, right| {
        arrangement_vertices_share_exact_key_height_and_field(*left, *right, vertices)
    });
    if loop_vertices.len() >= 2
        && arrangement_vertices_share_exact_key_height_and_field(
            loop_vertices[0],
            *loop_vertices
                .last()
                .expect("arrangement loop has last vertex"),
            vertices,
        )
    {
        loop_vertices.pop();
    }
}

fn remove_immediate_backtracking_arrangement_vertices(
    loop_vertices: &mut Vec<NodeArrangementVertexId>,
    vertices: &[NodeArrangementVertex],
) {
    if loop_vertices.len() < 3 {
        return;
    }
    let Some(index) = (0..loop_vertices.len()).find(|index| {
        let previous = loop_vertices[(*index + loop_vertices.len() - 1) % loop_vertices.len()];
        let next = loop_vertices[(*index + 1) % loop_vertices.len()];
        arrangement_vertices_share_exact_key_height_and_field(previous, next, vertices)
    }) else {
        return;
    };
    loop_vertices.remove(index);
}

fn arrangement_vertices_share_exact_key_height_and_field(
    left: NodeArrangementVertexId,
    right: NodeArrangementVertexId,
    vertices: &[NodeArrangementVertex],
) -> bool {
    let Some(left) = vertices.get(left.index()) else {
        return false;
    };
    let Some(right) = vertices.get(right.index()) else {
        return false;
    };
    left.key() == right.key()
        && left.height_mm() == right.height_mm()
        && left.height_field_id() == right.height_field_id()
}
