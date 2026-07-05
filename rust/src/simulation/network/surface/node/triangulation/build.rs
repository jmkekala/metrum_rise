//! Node triangulation entry points.

use super::coverage::overlay_shape_from_arrangement_region;
use super::regions::triangulate_arrangement_region;
use super::*;
use rayon::prelude::*;

const PARALLEL_NODE_REGION_TRIANGULATION_MIN_ITEMS: usize = 8;

enum NodeTriangulatedRegionBuildResult {
    Region(NodeTriangulatedRegion),
    Discarded(NodeTriangulationError),
}

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

        let triangulated_regions = if arrangement.regions().len()
            >= PARALLEL_NODE_REGION_TRIANGULATION_MIN_ITEMS
        {
            arrangement
                .regions()
                .par_iter()
                .enumerate()
                .map(|(region_index, region)| {
                    triangulate_arrangement_region_for_solution(arrangement, region_index, region)
                })
                .collect::<Vec<_>>()
        } else {
            arrangement
                .regions()
                .iter()
                .enumerate()
                .map(|(region_index, region)| {
                    triangulate_arrangement_region_for_solution(arrangement, region_index, region)
                })
                .collect::<Vec<_>>()
        };

        let mut regions = Vec::with_capacity(triangulated_regions.len());
        let mut first_discarded_error = None;
        for result in triangulated_regions {
            match result {
                Ok(NodeTriangulatedRegionBuildResult::Region(region)) => regions.push(region),
                Ok(NodeTriangulatedRegionBuildResult::Discarded(error)) => {
                    first_discarded_error.get_or_insert(error);
                }
                Err(error) => return Err(error),
            }
        }

        if regions.is_empty() {
            if let Some(error) = first_discarded_error {
                return Err(error);
            }
        }

        Ok(Self {
            node_id: arrangement.node_id(),
            piece_kind: arrangement.piece_kind(),
            regions,
            explicit_vertical_step_segments: arrangement.explicit_vertical_step_segments(),
        })
    }
}

fn triangulate_arrangement_region_for_solution(
    arrangement: &NodeArrangement,
    region_index: usize,
    region: &NodeOwnedRegion,
) -> Result<NodeTriangulatedRegionBuildResult, NodeTriangulationError> {
    triangulate_arrangement_region(arrangement.node_id(), region_index, arrangement, region)
        .map(NodeTriangulatedRegionBuildResult::Region)
        .or_else(|error| {
            if triangulation_error_is_discardable_numeric_region(&error, arrangement, region) {
                Ok(NodeTriangulatedRegionBuildResult::Discarded(error))
            } else {
                Err(error)
            }
        })
}

fn triangulation_error_is_discardable_numeric_region(
    error: &NodeTriangulationError,
    arrangement: &NodeArrangement,
    region: &NodeOwnedRegion,
) -> bool {
    if !matches!(
        error,
        NodeTriangulationError::DegenerateRegionContour { .. }
            | NodeTriangulationError::EmptyTriangulation { .. }
    ) {
        return false;
    }
    let shape = overlay_shape_from_arrangement_region(arrangement, region);
    let area_m2 = RoadSurfaceSystem::overlay_shape_area_m2(&shape);
    area_m2 <= RoadSurfaceSystem::overlay_numeric_area_budget_for_shape(&shape)
}
