//! Triangulated arrangement face attachment.

use super::super::backend::RoadVec2;
use super::super::triangulation::{NodeTriangulatedRegion, NodeTriangulationSolution};
use super::seams::NodeSeamSource;
use super::{
    NodeArrangement, NodeArrangementError, NodeArrangementFace, NodeArrangementFaceId,
    NodeArrangementVertexId, NodeBandOwner, NodeOwnedRegionId,
};

impl NodeArrangement {
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
        self.insert_vertex_with_grade_authority(
            RoadVec2::new(vertex.point_world.x, vertex.point_world.z),
            vertex.point_world.y,
            [region.owner],
            vertex.height_field_id,
            [NodeSeamSource::for_owner(region.owner)],
            vertex.grade_authority,
        )
    }
}
