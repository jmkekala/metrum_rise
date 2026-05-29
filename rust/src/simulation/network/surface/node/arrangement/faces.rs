//! Triangulated arrangement face attachment.

use super::super::backend::RoadVec2;
use super::super::triangulation::{NodeTriangulatedRegion, NodeTriangulationSolution};
use super::seams::NodeSeamSource;
use super::{
    NodeArrangement, NodeArrangementAttachProfile, NodeArrangementError, NodeArrangementFace,
    NodeArrangementFaceId, NodeArrangementVertexId, NodeBandOwner, NodeOwnedRegionId,
};
use std::time::Instant;

fn elapsed_profile_ms(start: Option<Instant>) -> f64 {
    start
        .map(|start| start.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

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

    #[cfg(test)]
    pub(crate) fn attach_triangulation(
        &mut self,
        triangulation: &NodeTriangulationSolution,
    ) -> Result<(), NodeArrangementError> {
        self.attach_triangulation_with_profile(triangulation, false)
            .map(|_| ())
    }

    pub(crate) fn attach_triangulation_with_profile(
        &mut self,
        triangulation: &NodeTriangulationSolution,
        profile_enabled: bool,
    ) -> Result<NodeArrangementAttachProfile, NodeArrangementError> {
        let total_start = profile_enabled.then(Instant::now);
        let mut profile = NodeArrangementAttachProfile {
            arrangement_vertices_before: self.vertices.len(),
            regions: triangulation.regions.len(),
            source_vertices: triangulation
                .regions
                .iter()
                .map(|region| region.vertices.len())
                .sum(),
            source_triangles: triangulation
                .regions
                .iter()
                .map(|region| region.triangles.len())
                .sum(),
            ..NodeArrangementAttachProfile::default()
        };

        let validation_start = profile_enabled.then(Instant::now);
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
        profile.validation_ms = elapsed_profile_ms(validation_start);

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
                let insert_start = profile_enabled.then(Instant::now);
                let vertex_count_before = self.vertices.len();
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
                profile.insert_vertices_ms += elapsed_profile_ms(insert_start);
                profile.vertex_insert_attempts += 3;
                let inserted = self.vertices.len().saturating_sub(vertex_count_before);
                profile.vertices_inserted += inserted;
                profile.vertices_reused += 3usize.saturating_sub(inserted);
                let push_face_start = profile_enabled.then(Instant::now);
                self.push_face(region_id, triangulated_region.owner, vertices);
                profile.push_faces_ms += elapsed_profile_ms(push_face_start);
                profile.faces_pushed += 1;
            }
        }

        let conflict_start = profile_enabled.then(Instant::now);
        self.reject_implicit_material_height_conflicts()?;
        profile.conflict_ms = elapsed_profile_ms(conflict_start);
        profile.arrangement_vertices_after = self.vertices.len();
        profile.total_ms = elapsed_profile_ms(total_start);
        Ok(profile)
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
