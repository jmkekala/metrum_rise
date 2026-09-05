// SPDX-License-Identifier: GPL-2.0-only

//! Node footprint boundary conflict tests.

use super::*;

#[test]
fn node_export_rejects_conflicting_footprint_boundary_heights() {
    let lower_owner = owner(RoadSurfaceBandKind::Carriageway, 0);
    let raised_owner = owner(RoadSurfaceBandKind::Sidewalk, 1);
    let lower_height = height_field(lower_owner);
    let raised_height = height_field(raised_owner);
    let mut arrangement = NodeArrangement::new(84, RoadSurfaceVisualNodePieceKind::JunctionN);
    let lower_start = arrangement
        .insert_vertex(
            RoadVec2::new(0.0, 0.0),
            0.0,
            [lower_owner],
            lower_height,
            [],
        )
        .expect("lower start vertex is valid");
    let lower_end = arrangement
        .insert_vertex(
            RoadVec2::new(1.0, 0.0),
            0.0,
            [lower_owner],
            lower_height,
            [],
        )
        .expect("lower end vertex is valid");
    let lower_apex = arrangement
        .insert_vertex(
            RoadVec2::new(0.0, 1.0),
            0.0,
            [lower_owner],
            lower_height,
            [],
        )
        .expect("lower apex vertex is valid");
    let lower_edges = push_exposed_triangle_boundary_edges(
        &mut arrangement,
        lower_owner,
        lower_height,
        [lower_start, lower_end, lower_apex],
    );
    let lower_region = arrangement.push_region(
        lower_owner,
        lower_height,
        vec![lower_start, lower_end, lower_apex],
        Vec::new(),
        lower_edges,
        0.5,
        Vec::new(),
    );
    arrangement.push_face(
        lower_region,
        lower_owner,
        [lower_start, lower_end, lower_apex],
    );

    let raised_start = arrangement
        .insert_vertex(
            RoadVec2::new(0.0, 0.0),
            0.1,
            [raised_owner],
            raised_height,
            [],
        )
        .expect("raised start vertex is valid");
    let raised_end = arrangement
        .insert_vertex(
            RoadVec2::new(-1.0, 0.0),
            0.1,
            [raised_owner],
            raised_height,
            [],
        )
        .expect("raised end vertex is valid");
    let raised_apex = arrangement
        .insert_vertex(
            RoadVec2::new(0.0, -1.0),
            0.1,
            [raised_owner],
            raised_height,
            [],
        )
        .expect("raised apex vertex is valid");
    let raised_edges = push_exposed_triangle_boundary_edges(
        &mut arrangement,
        raised_owner,
        raised_height,
        [raised_start, raised_end, raised_apex],
    );
    let raised_region = arrangement.push_region(
        raised_owner,
        raised_height,
        vec![raised_start, raised_end, raised_apex],
        Vec::new(),
        raised_edges,
        0.5,
        Vec::new(),
    );
    arrangement.push_face(
        raised_region,
        raised_owner,
        [raised_start, raised_end, raised_apex],
    );
    let footprint_shapes = footprint_shapes_from_points(&[
        RoadVec2::new(0.0, 0.0),
        RoadVec2::new(1.0, 0.0),
        RoadVec2::new(0.0, -1.0),
    ]);

    let error =
        RoadSurfaceSystem::node_surface_regions_from_arrangement(&arrangement, &footprint_shapes)
            .expect_err("footprint boundary height conflicts must not be resolved by max/min");

    assert!(matches!(
        error,
        NodeBoundaryExportError::ConflictingFootprintBoundaryHeight { .. }
    ));
}
