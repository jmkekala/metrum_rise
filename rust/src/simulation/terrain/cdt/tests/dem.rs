//! Authored and synthetic DEM validation matrices.

use super::*;

#[test]
fn road_touched_dem_validation_matrix_covers_retaining_wall_tie_ins() {
    assert_road_touched_dem_tie_in_case(
        "ordinary raised road on supportive DEM",
        square_road_loop(3.0, 7.0, 0.20),
        Vec::new(),
        0,
        false,
    );
    assert_road_touched_dem_tie_in_case(
        "near-road DEM samples widen ordinary cut fill",
        square_road_loop(3.0, 7.0, 0.12),
        vec![
            TerrainCdtVertex::new(5.0, 0.0, 2.99),
            TerrainCdtVertex::new(2.99, 0.0, 5.0),
            TerrainCdtVertex::new(7.01, 0.0, 5.0),
            TerrainCdtVertex::new(5.0, 0.0, 7.01),
        ],
        4,
        false,
    );
    assert_road_touched_dem_tie_in_case(
        "raised road above unavoidable cliff DEM",
        square_road_loop(4.0, 6.0, 4.0),
        Vec::new(),
        0,
        true,
    );
    assert_road_touched_dem_tie_in_case(
        "lowered road below unavoidable cliff DEM",
        square_road_loop(4.0, 6.0, -4.0),
        Vec::new(),
        0,
        true,
    );
    assert_road_touched_dem_tie_in_case(
        "near-road DEM widening still leaves explicit retaining wall",
        square_road_loop(4.0, 6.0, 4.0),
        vec![
            TerrainCdtVertex::new(5.0, 0.0, 3.99),
            TerrainCdtVertex::new(3.99, 0.0, 5.0),
            TerrainCdtVertex::new(6.01, 0.0, 5.0),
            TerrainCdtVertex::new(5.0, 0.0, 6.01),
        ],
        4,
        true,
    );
}

#[test]
fn authored_steep_dem_matrix_preserves_sourced_road_touched_contract() {
    let patch = TerrainCdtPatch::new(0.0, 0.0, 40.0, 40.0, [0.0, 0.0, 0.0, 0.0]);
    let cases = vec![
        (
            "road crossing a steep hillside",
            road_loop_from_centerline_with_heights(
                TerrainCdtVertex::new(5.0, authored_cross_slope_height(5.0, 20.0), 20.0),
                TerrainCdtVertex::new(35.0, authored_cross_slope_height(35.0, 20.0), 20.0),
                6.0,
            ),
            authored_dem_samples(patch, 4.0, authored_cross_slope_height),
            false,
        ),
        (
            "road running along a cross-slope",
            road_loop_from_centerline_with_heights(
                TerrainCdtVertex::new(20.0, authored_along_slope_height(20.0, 5.0), 5.0),
                TerrainCdtVertex::new(20.0, authored_along_slope_height(20.0, 35.0), 35.0),
                6.0,
            ),
            authored_dem_samples(patch, 4.0, authored_along_slope_height),
            false,
        ),
        (
            "road crossing an authored ridge and valley",
            road_loop_from_centerline_with_heights(
                TerrainCdtVertex::new(6.0, 0.0, 10.0),
                TerrainCdtVertex::new(34.0, 0.0, 30.0),
                6.0,
            ),
            authored_dem_samples(patch, 4.0, authored_ridge_valley_height),
            false,
        ),
    ];

    for (case_name, road, source_samples, expect_retaining_wall) in cases {
        let source = test_span_boundary_source(200, TerrainCdtRoadBandKind::Sidewalk, 4);
        let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
            patch,
            vec![sourced_road_loop(200, 0, road.clone(), source)],
            source_samples,
        ))
        .unwrap_or_else(|_| panic!("{case_name}: terrain CDT should build"));

        assert_sourced_road_touched_mesh_contract(case_name, &mesh, patch, &[road], source);
        if expect_retaining_wall {
            assert!(
                mesh.stats.retaining_wall_faces > 0,
                "{case_name}: authored extreme DEM should expose retaining-wall tie-in faces"
            );
        }
    }
}
