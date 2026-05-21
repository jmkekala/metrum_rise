//! Node triangulation entry points.

use super::regions::triangulate_arrangement_region;
use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn build_node_triangulation_from_arrangement(
        arrangement: &NodeArrangement,
    ) -> Result<NodeTriangulationSolution, NodeTriangulationError> {
        NodeTriangulationSolution::from_arrangement(arrangement)
    }
}

impl NodeTriangulationSolution {
    pub(crate) fn from_arrangement(
        arrangement: &NodeArrangement,
    ) -> Result<Self, NodeTriangulationError> {
        if arrangement.regions().is_empty() {
            return Err(NodeTriangulationError::EmptyHeightSolution {
                node_id: arrangement.node_id(),
            });
        }

        let mut regions = Vec::with_capacity(arrangement.regions().len());
        for (region_index, region) in arrangement.regions().iter().enumerate() {
            regions.push(triangulate_arrangement_region(
                arrangement.node_id(),
                region_index,
                arrangement,
                region,
            )?);
        }

        Ok(Self {
            node_id: arrangement.node_id(),
            piece_kind: arrangement.piece_kind(),
            regions,
            explicit_vertical_step_segments: arrangement.explicit_vertical_step_segments(),
        })
    }
}
