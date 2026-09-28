// SPDX-License-Identifier: GPL-2.0-only

//! Road-tool query helpers shared by Godot-facing bridge methods.

use crate::simulation::network::surface::{RoadPreviewValidation, RoadSurfaceSystem};
use crate::simulation::network::types::EdgeClass;
use crate::simulation::water::WaterSystem;
use godot::prelude::Vector3;

/// Stable preview rejection reason for a grounded roadbed that overlaps authored water.
pub(crate) const ROAD_WATER_REQUIRES_BRIDGE_REASON: &str = "water_requires_bridge";

/// Applies the authored-water placement rule to an otherwise validated road candidate.
pub(crate) fn validate_road_candidate_against_water(
    edge_class: EdgeClass,
    prepared_points: &[Vector3],
    fwd_lanes: u8,
    bkw_lanes: u8,
    water: &WaterSystem,
    validation: RoadPreviewValidation,
) -> RoadPreviewValidation {
    if !validation.is_valid || edge_class != EdgeClass::Standard || prepared_points.len() < 2 {
        return validation;
    }
    let half_width_m = RoadSurfaceSystem::preview_candidate_roadbed_half_width_m(
        prepared_points,
        edge_class,
        fwd_lanes,
        bkw_lanes,
    );
    if water.road_corridor_overlaps_visible_water(prepared_points, half_width_m) {
        return validation.with_invalid_reason(ROAD_WATER_REQUIRES_BRIDGE_REASON);
    }
    validation
}

#[cfg(test)]
mod tests {
    use super::{ROAD_WATER_REQUIRES_BRIDGE_REASON, validate_road_candidate_against_water};
    use crate::simulation::network::surface::RoadSurfaceSystem;
    use crate::simulation::network::types::EdgeClass;
    use crate::simulation::terrain::TerrainSystem;
    use crate::simulation::water::WaterSystem;
    use godot::prelude::Vector3;

    #[test]
    fn standard_road_candidate_crossing_water_requires_bridge() {
        let terrain = TerrainSystem::with_chunking(9, 9, 10.0, 4, 0.0);
        let water = test_center_lake();
        let surface = RoadSurfaceSystem::new(30.0);
        let preview = surface.compile_preview_surface(
            &[Vector3::new(-30.0, 0.0, 0.0), Vector3::new(30.0, 0.0, 0.0)],
            1,
            1,
            &terrain,
        );

        assert_eq!(preview.edge_class, EdgeClass::Standard);
        let validation = validate_road_candidate_against_water(
            preview.edge_class,
            &preview.prepared_points,
            1,
            1,
            &water,
            preview.validation,
        );

        assert!(!validation.is_valid);
        assert_eq!(validation.invalid_reason, ROAD_WATER_REQUIRES_BRIDGE_REASON);
    }

    #[test]
    fn bridge_candidate_crossing_water_remains_valid() {
        let terrain = TerrainSystem::with_chunking(9, 9, 10.0, 4, 0.0);
        let water = test_center_lake();
        let surface = RoadSurfaceSystem::new(30.0);
        let preview = surface.compile_preview_surface(
            &[
                Vector3::new(-30.0, 3.0, 0.0),
                Vector3::new(0.0, 3.0, 0.0),
                Vector3::new(30.0, 3.0, 0.0),
            ],
            1,
            1,
            &terrain,
        );

        assert_eq!(preview.edge_class, EdgeClass::Bridge);
        let validation = validate_road_candidate_against_water(
            preview.edge_class,
            &preview.prepared_points,
            1,
            1,
            &water,
            preview.validation,
        );

        assert!(validation.is_valid);
    }

    #[test]
    fn standard_road_candidate_beside_water_remains_valid() {
        let terrain = TerrainSystem::with_chunking(9, 9, 10.0, 4, 0.0);
        let water = test_center_lake();
        let surface = RoadSurfaceSystem::new(30.0);
        let preview = surface.compile_preview_surface(
            &[
                Vector3::new(-30.0, 0.0, 30.0),
                Vector3::new(30.0, 0.0, 30.0),
            ],
            1,
            1,
            &terrain,
        );

        assert_eq!(preview.edge_class, EdgeClass::Standard);
        let validation = validate_road_candidate_against_water(
            preview.edge_class,
            &preview.prepared_points,
            1,
            1,
            &water,
            preview.validation,
        );

        assert!(validation.is_valid);
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
