// SPDX-License-Identifier: GPL-2.0-only

//! Road-tool query helpers shared by Godot-facing bridge methods.

use crate::simulation::network::road_edit::{PreparedRoadInput, RoadPreviewValidation};
use crate::simulation::network::roadbed::preview_candidate_roadbed_half_width_m;
use crate::simulation::network::types::EdgeClass;
use crate::simulation::water::WaterSystem;

/// Stable preview rejection reason for a grounded roadbed that overlaps authored water.
pub(crate) const ROAD_WATER_REQUIRES_BRIDGE_REASON: &str = "water_requires_bridge";
/// Stable rejection reason for a tunnel stroke (`ROAD-61`) whose corridor, bore or approach,
/// lies under or in authored water: its cover or a portal would be a water body.
pub(crate) const TUNNEL_UNDER_WATER_REASON: &str = "tunnel_under_water";

/// Applies the authored-water placement rule to an otherwise validated road candidate: a
/// grounded road needs a bridge over water, and a stroke drawn with the tunnel intent may not
/// pass under it anywhere. O(corridor segments × water cells they touch).
pub(crate) fn validate_road_candidate_against_water(
    prepared: &PreparedRoadInput,
    fwd_lanes: u8,
    bkw_lanes: u8,
    water: &WaterSystem,
    validation: RoadPreviewValidation,
) -> RoadPreviewValidation {
    let (edge_class, prepared_points) = (prepared.class, &prepared.points[..]);
    let checked = edge_class == EdgeClass::Standard || prepared.tunnel;
    if !validation.is_valid || !checked || prepared_points.len() < 2 {
        return validation;
    }
    let half_width_m = preview_candidate_roadbed_half_width_m(
        prepared_points,
        edge_class,
        fwd_lanes,
        bkw_lanes,
        prepared.median,
        prepared.attributes,
    );
    if water.road_corridor_overlaps_visible_water(prepared_points, half_width_m) {
        let reason = if prepared.tunnel {
            TUNNEL_UNDER_WATER_REASON
        } else {
            ROAD_WATER_REQUIRES_BRIDGE_REASON
        };
        return validation.with_invalid_reason(reason);
    }
    validation
}

#[cfg(test)]
mod tests {
    use super::{ROAD_WATER_REQUIRES_BRIDGE_REASON, validate_road_candidate_against_water};
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::road_edit::{
        PreparedRoadInput, RoadPreviewValidation, prepare_road_input_for_tool,
        validate_road_candidate_fast,
    };
    use crate::simulation::network::roadbed::RoadbedSystem;
    use crate::simulation::network::types::EdgeClass;
    use crate::simulation::terrain::TerrainSystem;
    use crate::simulation::water::WaterSystem;
    use godot::prelude::Vector3;

    #[test]
    fn standard_road_candidate_crossing_water_requires_bridge() {
        let water = test_center_lake();
        let (preview, validation) =
            prepare(&[Vector3::new(-30.0, 0.0, 0.0), Vector3::new(30.0, 0.0, 0.0)]);

        assert_eq!(preview.class, EdgeClass::Standard);
        let validation = validate_road_candidate_against_water(&preview, 1, 1, &water, validation);

        assert!(!validation.is_valid);
        assert_eq!(validation.invalid_reason, ROAD_WATER_REQUIRES_BRIDGE_REASON);
    }

    #[test]
    fn bridge_candidate_crossing_water_remains_valid() {
        let water = test_center_lake();
        let (preview, validation) = prepare(&[
            Vector3::new(-30.0, 3.0, 0.0),
            Vector3::new(0.0, 3.0, 0.0),
            Vector3::new(30.0, 3.0, 0.0),
        ]);

        assert_eq!(preview.class, EdgeClass::Bridge);
        let validation = validate_road_candidate_against_water(&preview, 1, 1, &water, validation);

        assert!(validation.is_valid);
    }

    #[test]
    fn standard_road_candidate_beside_water_remains_valid() {
        let water = test_center_lake();
        let (preview, validation) = prepare(&[
            Vector3::new(-30.0, 0.0, 30.0),
            Vector3::new(30.0, 0.0, 30.0),
        ]);

        assert_eq!(preview.class, EdgeClass::Standard);
        let validation = validate_road_candidate_against_water(&preview, 1, 1, &water, validation);

        assert!(validation.is_valid);
    }

    // Prepares and fast-validates a stroke on flat terrain with no existing roads.
    fn prepare(points: &[Vector3]) -> (PreparedRoadInput, RoadPreviewValidation) {
        let terrain = TerrainSystem::with_chunking(9, 9, 10.0, 4, 0.0);
        let graph = RegionGraph::new();
        let prepared = prepare_road_input_for_tool(
            points,
            &terrain,
            &graph,
            &RoadbedSystem::new(30.0),
            false,
            false,
        );
        let validation = validate_road_candidate_fast(&prepared, (1, 1), &terrain, &graph);
        (prepared, validation)
    }

    fn test_center_lake() -> WaterSystem {
        let mut water = WaterSystem::with_chunking(9, 9, 10.0, 4);
        let mut baseline = vec![0.0; 9 * 9];
        for z in 3..=5 {
            for x in 3..=5 {
                baseline[x + z * 9] = 2.0;
            }
        }
        water
            .replace_baseline_depth_from_dense(&baseline)
            .expect("test lake dimensions should match");
        water
    }
}
