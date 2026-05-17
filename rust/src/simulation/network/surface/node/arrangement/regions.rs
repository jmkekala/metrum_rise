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

        contour
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
            .collect()
    }
}
