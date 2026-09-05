// SPDX-License-Identifier: GPL-2.0-only

//! Final node footprint height-conflict tests.

use super::*;

#[test]
fn node_export_rejects_final_owned_footprint_height_conflict_without_step_authority() {
    let lower_owner = owner(RoadSurfaceBandKind::Carriageway, 0);
    let raised_owner = owner(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let lower_height = height_field(lower_owner);
    let raised_height = height_field(raised_owner);
    let mut arrangement = NodeArrangement::new(86, RoadSurfaceVisualNodePieceKind::JunctionN);

    let lower_a = arrangement
        .insert_vertex(
            RoadVec2::new(0.0, 0.0),
            0.0,
            [lower_owner],
            lower_height,
            [],
        )
        .expect("lower vertex is valid");
    let lower_b = arrangement
        .insert_vertex(
            RoadVec2::new(1.0, 0.0),
            0.0,
            [lower_owner],
            lower_height,
            [],
        )
        .expect("lower vertex is valid");
    let lower_c = arrangement
        .insert_vertex(
            RoadVec2::new(1.0, 1.0),
            0.0,
            [lower_owner],
            lower_height,
            [],
        )
        .expect("lower vertex is valid");
    let lower_d = arrangement
        .insert_vertex(
            RoadVec2::new(0.0, 1.0),
            0.0,
            [lower_owner],
            lower_height,
            [],
        )
        .expect("lower vertex is valid");
    let lower_boundary_edges = vec![
        arrangement.push_edge(
            lower_a,
            lower_b,
            lower_owner,
            lower_height,
            None,
            None,
            true,
            false,
            false,
            NodeSeamSource::for_owner(lower_owner),
            Vec::new(),
        ),
        arrangement.push_edge(
            lower_b,
            lower_c,
            lower_owner,
            lower_height,
            None,
            None,
            true,
            false,
            false,
            NodeSeamSource::for_owner(lower_owner),
            Vec::new(),
        ),
        arrangement.push_edge(
            lower_d,
            lower_a,
            lower_owner,
            lower_height,
            None,
            None,
            true,
            false,
            false,
            NodeSeamSource::for_owner(lower_owner),
            Vec::new(),
        ),
    ];
    let lower_region = arrangement.push_region(
        lower_owner,
        lower_height,
        vec![lower_a, lower_b, lower_c, lower_d],
        Vec::new(),
        lower_boundary_edges,
        1.0,
        Vec::new(),
    );
    arrangement.push_face(lower_region, lower_owner, [lower_a, lower_b, lower_c]);
    arrangement.push_face(lower_region, lower_owner, [lower_a, lower_c, lower_d]);

    let raised_a = arrangement
        .insert_vertex(
            RoadVec2::new(0.0, 1.0),
            0.12,
            [raised_owner],
            raised_height,
            [],
        )
        .expect("raised vertex is valid");
    let raised_b = arrangement
        .insert_vertex(
            RoadVec2::new(1.0, 1.0),
            0.12,
            [raised_owner],
            raised_height,
            [],
        )
        .expect("raised vertex is valid");
    let raised_c = arrangement
        .insert_vertex(
            RoadVec2::new(1.0, 2.0),
            0.12,
            [raised_owner],
            raised_height,
            [],
        )
        .expect("raised vertex is valid");
    let raised_d = arrangement
        .insert_vertex(
            RoadVec2::new(0.0, 2.0),
            0.12,
            [raised_owner],
            raised_height,
            [],
        )
        .expect("raised vertex is valid");
    let raised_boundary_edges = vec![
        arrangement.push_edge(
            raised_b,
            raised_c,
            raised_owner,
            raised_height,
            None,
            None,
            true,
            false,
            false,
            NodeSeamSource::for_owner(raised_owner),
            Vec::new(),
        ),
        arrangement.push_edge(
            raised_c,
            raised_d,
            raised_owner,
            raised_height,
            None,
            None,
            true,
            false,
            false,
            NodeSeamSource::for_owner(raised_owner),
            Vec::new(),
        ),
        arrangement.push_edge(
            raised_d,
            raised_a,
            raised_owner,
            raised_height,
            None,
            None,
            true,
            false,
            false,
            NodeSeamSource::for_owner(raised_owner),
            Vec::new(),
        ),
    ];
    let raised_region = arrangement.push_region(
        raised_owner,
        raised_height,
        vec![raised_a, raised_b, raised_c, raised_d],
        Vec::new(),
        raised_boundary_edges,
        1.0,
        Vec::new(),
    );
    arrangement.push_face(raised_region, raised_owner, [raised_a, raised_b, raised_c]);
    arrangement.push_face(raised_region, raised_owner, [raised_a, raised_c, raised_d]);

    let footprint_shapes = footprint_shapes_from_points(&[
        RoadVec2::new(0.0, 0.0),
        RoadVec2::new(1.0, 0.0),
        RoadVec2::new(1.0, 2.0),
        RoadVec2::new(0.0, 2.0),
    ]);
    assert!(
        arrangement.explicit_vertical_step_segments().is_empty(),
        "test setup must not carry materialized step authority"
    );
    let error =
        RoadSurfaceSystem::node_surface_regions_from_arrangement(&arrangement, &footprint_shapes)
            .expect_err("shared footprint boundary heights require explicit step authority");
    assert!(
        matches!(
            error,
            NodeBoundaryExportError::ConflictingFootprintBoundaryHeight { .. }
        ),
        "expected structured footprint height conflict, got {error:?}"
    );
}
