// SPDX-License-Identifier: GPL-2.0-only

//! Generated contour source scan regression tests.

#[test]
fn height_solution_has_no_unsourced_generated_height_path() {
    let source = [
        include_str!("../../../height.rs"),
        include_str!("../../build.rs"),
        include_str!("../../carriers.rs"),
        include_str!("../../evaluate.rs"),
        include_str!("../../field.rs"),
        include_str!("../../grade.rs"),
        include_str!("../../authority.rs"),
        include_str!("../../handoff.rs"),
        include_str!("../../model.rs"),
        include_str!("../../patch.rs"),
        include_str!("../../seams.rs"),
        include_str!("../../source_edges.rs"),
        include_str!("../../triangles.rs"),
        include_str!("../../vertices.rs"),
    ]
    .join("\n");
    for forbidden in [
        concat!("heighted_shape_with_", "canonical_contour_insertions"),
        concat!("heighted_contour_with_", "canonical_insertions"),
        concat!("fill_canonical_contour_", "height_insertions"),
        concat!("reheight_terminal_", "cap_band_from_base"),
        concat!("reheight_point_", "from_base"),
        concat!("from_terminal_cap_band_", "with_base"),
        concat!("evaluate_region_", "scoped_height"),
        concat!("bounded_region_", "scoped_edge_height"),
        concat!("region_scoped_", "carrier"),
        concat!("HEIGHT_SOURCE_EDGE_", "NEIGHBOR_UNITS"),
        concat!("HEIGHT_SOURCE_EDGE_", "DEDUP_DRIFT_UNITS"),
        concat!("allow_missing_height_points_", "backfill"),
        concat!("subdivided_", "height_chord"),
    ] {
        assert!(
            !source.contains(forbidden),
            "canonical arrangement vertices must be inside their explicit height carrier, not supplied by `{forbidden}`"
        );
    }
}
