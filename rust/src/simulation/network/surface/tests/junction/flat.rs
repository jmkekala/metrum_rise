//! Flat junction canonical pipeline tests.

use super::*;

#[test]
fn logged_flat_three_way_right_angle_junction_compiles_explicit_raised_steps() {
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-102.807, 0.0, -14.721), NodeType::Junction);
    let center = graph.add_node(Vector3::new(-35.427, 0.0, -14.721), NodeType::Junction);
    let east = graph.add_node(Vector3::new(37.193, 0.0, -14.721), NodeType::Junction);
    let north = graph.add_node(Vector3::new(-35.427, 0.0, 35.279), NodeType::Junction);
    graph.add_edge(test_edge(
        west,
        center,
        vec![
            Vector3::new(-102.807, 0.0, -14.721),
            Vector3::new(-35.427, 0.0, -14.721),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        north,
        vec![
            Vector3::new(-35.427, 0.0, -14.721),
            Vector3::new(-35.427, 0.0, 35.279),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        east,
        vec![
            Vector3::new(-35.427, 0.0, -14.721),
            Vector3::new(37.193, 0.0, -14.721),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(192, 192);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    assert_canonical_explicit_vertical_steps_have_faces(piece);
}

#[test]
fn flat_bend_angle_matrix_compiles_conflict_first_owned_regions() {
    for angle_degrees in GENERATED_CONFLICT_MATRIX_ANGLES_DEGREES {
        compile_generated_flat_bend(
            angle_degrees,
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
        );
    }
}

#[test]
fn flat_t_junction_angle_matrix_compiles_conflict_first_owned_regions() {
    for angle_degrees in GENERATED_CONFLICT_MATRIX_ANGLES_DEGREES {
        compile_generated_flat_t_junction(
            angle_degrees,
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
        );
    }
}

#[test]
fn flat_four_way_junction_matrix_compiles_conflict_first_owned_regions() {
    for endpoint_angle_degrees in [
        [0.0, 90.0, 180.0, 270.0],
        [0.0, 5.0, 96.0, 181.0],
        [0.0, 35.0, 140.0, 252.0],
        [0.0, 73.0, 180.0, 244.0],
    ] {
        compile_generated_flat_multiway_junction(
            &endpoint_angle_degrees,
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
        );
    }
}

#[test]
fn flat_arbitrary_multiway_junction_matrix_compiles_conflict_first_owned_regions() {
    for endpoint_angle_degrees in [
        [0.0, 11.0, 95.0, 194.0, 278.0],
        [0.0, 37.0, 118.0, 203.0, 291.0],
    ] {
        compile_generated_flat_multiway_junction(
            &endpoint_angle_degrees,
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
        );
    }
    for endpoint_angle_degrees in [
        [0.0, 3.0, 47.0, 121.0, 202.0, 305.0],
        [0.0, 23.0, 61.0, 137.0, 211.0, 304.0],
    ] {
        compile_generated_flat_multiway_junction(
            &endpoint_angle_degrees,
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
        );
    }
}

#[test]
fn flat_bend_reversed_edge_direction_compiles_conflict_first_owned_regions() {
    let from_center = compile_generated_flat_bend(
        30.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
    let to_center = compile_generated_flat_bend(
        30.0,
        GeneratedEdgeDirection::ToCenter,
        GeneratedEditOrder::Forward,
    );
    assert_generated_node_canonical_signature_eq(
        "from_center",
        &from_center,
        "to_center",
        &to_center,
    );
}

#[test]
fn flat_t_junction_reversed_edge_direction_compiles_conflict_first_owned_regions() {
    let from_center = compile_generated_flat_t_junction(
        30.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
    let to_center = compile_generated_flat_t_junction(
        30.0,
        GeneratedEdgeDirection::ToCenter,
        GeneratedEditOrder::Forward,
    );
    assert_generated_node_canonical_signature_eq(
        "from_center",
        &from_center,
        "to_center",
        &to_center,
    );
}

#[test]
fn flat_bend_equivalent_edit_order_compiles_conflict_first_owned_regions() {
    let forward = compile_generated_flat_bend(
        60.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
    let reverse = compile_generated_flat_bend(
        60.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Reverse,
    );
    assert_generated_node_canonical_signature_eq("forward", &forward, "reverse", &reverse);
}

#[test]
fn flat_t_junction_equivalent_edit_order_compiles_conflict_first_owned_regions() {
    let forward = compile_generated_flat_t_junction(
        60.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
    let reverse = compile_generated_flat_t_junction(
        60.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Reverse,
    );
    assert_generated_node_canonical_signature_eq("forward", &forward, "reverse", &reverse);
}

#[test]
fn flat_t_junction_equivalent_edit_order_preserves_exact_raw_polygon_identity() {
    let forward = compile_generated_flat_t_junction_raw_identity(
        60.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
    let reverse = compile_generated_flat_t_junction_raw_identity(
        60.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Reverse,
    );
    assert_canonical_node_raw_polygon_identity_eq("forward", &forward, "reverse", &reverse);
}

#[test]
fn flat_four_way_junction_equivalent_edit_order_preserves_exact_raw_polygon_identity() {
    let forward = compile_generated_flat_four_way_junction_raw_identity(
        73.0,
        244.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
    let reverse = compile_generated_flat_four_way_junction_raw_identity(
        73.0,
        244.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Reverse,
    );
    assert_canonical_node_raw_polygon_identity_eq("forward", &forward, "reverse", &reverse);
}

#[test]
fn flat_four_way_junction_matrix_preserves_exact_raw_polygon_identity() {
    for endpoint_angle_degrees in [
        [0.0, 90.0, 180.0, 270.0],
        [0.0, 5.0, 96.0, 181.0],
        [0.0, 35.0, 140.0, 252.0],
        [0.0, 73.0, 180.0, 244.0],
    ] {
        let forward = compile_generated_flat_multiway_junction_raw_identity(
            &endpoint_angle_degrees,
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
        );
        let reverse = compile_generated_flat_multiway_junction_raw_identity(
            &endpoint_angle_degrees,
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Reverse,
        );
        assert_canonical_node_raw_polygon_identity_eq("forward", &forward, "reverse", &reverse);
    }
}

#[test]
fn flat_five_way_junction_equivalent_edit_order_preserves_exact_raw_polygon_identity() {
    let endpoint_angle_degrees = [0.0, 37.0, 118.0, 203.0, 291.0];
    let forward = compile_generated_flat_multiway_junction_raw_identity(
        &endpoint_angle_degrees,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
    let reverse = compile_generated_flat_multiway_junction_raw_identity(
        &endpoint_angle_degrees,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Reverse,
    );
    assert_canonical_node_raw_polygon_identity_eq("forward", &forward, "reverse", &reverse);
}

#[test]
fn flat_six_way_junction_equivalent_edit_order_preserves_exact_raw_polygon_identity() {
    let endpoint_angle_degrees = [0.0, 23.0, 61.0, 137.0, 211.0, 304.0];
    let forward = compile_generated_flat_multiway_junction_raw_identity(
        &endpoint_angle_degrees,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
    let reverse = compile_generated_flat_multiway_junction_raw_identity(
        &endpoint_angle_degrees,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Reverse,
    );
    assert_canonical_node_raw_polygon_identity_eq("forward", &forward, "reverse", &reverse);
}

#[test]
fn flat_mixed_width_junction_matrix_preserves_exact_raw_polygon_identity() {
    for (endpoint_angle_degrees, edge_widths_m) in [
        ([0.0, 35.0, 140.0, 252.0], [7.0, 10.5, 5.5, 8.75]),
        ([0.0, 90.0, 180.0, 270.0], [6.0, 9.0, 7.5, 11.0]),
    ] {
        let forward = compile_generated_flat_multiway_junction_with_widths_raw_identity(
            &endpoint_angle_degrees,
            &edge_widths_m,
            GeneratedEditOrder::Forward,
        );
        let reverse = compile_generated_flat_multiway_junction_with_widths_raw_identity(
            &endpoint_angle_degrees,
            &edge_widths_m,
            GeneratedEditOrder::Reverse,
        );
        assert_canonical_node_raw_polygon_identity_eq("forward", &forward, "reverse", &reverse);
    }
    for (endpoint_angle_degrees, edge_widths_m) in
        [([0.0, 11.0, 95.0, 194.0, 278.0], [7.0, 12.0, 5.5, 8.0, 10.0])]
    {
        let forward = compile_generated_flat_multiway_junction_with_widths_raw_identity(
            &endpoint_angle_degrees,
            &edge_widths_m,
            GeneratedEditOrder::Forward,
        );
        let reverse = compile_generated_flat_multiway_junction_with_widths_raw_identity(
            &endpoint_angle_degrees,
            &edge_widths_m,
            GeneratedEditOrder::Reverse,
        );
        assert_canonical_node_raw_polygon_identity_eq("forward", &forward, "reverse", &reverse);
    }
    for (endpoint_angle_degrees, edge_widths_m) in [(
        [0.0, 3.0, 47.0, 121.0, 202.0, 305.0],
        [6.5, 9.0, 5.0, 11.0, 8.0, 7.5],
    )] {
        let forward = compile_generated_flat_multiway_junction_with_widths_raw_identity(
            &endpoint_angle_degrees,
            &edge_widths_m,
            GeneratedEditOrder::Forward,
        );
        let reverse = compile_generated_flat_multiway_junction_with_widths_raw_identity(
            &endpoint_angle_degrees,
            &edge_widths_m,
            GeneratedEditOrder::Reverse,
        );
        assert_canonical_node_raw_polygon_identity_eq("forward", &forward, "reverse", &reverse);
    }
}

#[test]
fn flat_mixed_profile_mode_junction_matrix_preserves_exact_raw_polygon_identity() {
    use GeneratedEdgeProfileMode::{Shoulder, SidewalkCurb};

    for (endpoint_angle_degrees, edge_widths_m, edge_profile_modes) in [
        (
            [0.0, 35.0, 140.0, 252.0],
            [7.0, 10.5, 5.5, 8.75],
            [SidewalkCurb, Shoulder, SidewalkCurb, Shoulder],
        ),
        (
            [0.0, 90.0, 180.0, 270.0],
            [6.0, 9.0, 7.5, 11.0],
            [Shoulder, SidewalkCurb, SidewalkCurb, Shoulder],
        ),
    ] {
        let forward =
            compile_generated_flat_multiway_junction_with_widths_and_profile_modes_raw_identity(
                &endpoint_angle_degrees,
                &edge_widths_m,
                &edge_profile_modes,
                GeneratedEdgeDirection::FromCenter,
                GeneratedEditOrder::Forward,
            );
        let reverse =
            compile_generated_flat_multiway_junction_with_widths_and_profile_modes_raw_identity(
                &endpoint_angle_degrees,
                &edge_widths_m,
                &edge_profile_modes,
                GeneratedEdgeDirection::FromCenter,
                GeneratedEditOrder::Reverse,
            );
        assert_canonical_node_raw_polygon_identity_eq("forward", &forward, "reverse", &reverse);
    }
    for (endpoint_angle_degrees, edge_widths_m, edge_profile_modes) in [(
        [0.0, 11.0, 95.0, 194.0, 278.0],
        [7.0, 12.0, 5.5, 8.0, 10.0],
        [SidewalkCurb, Shoulder, SidewalkCurb, Shoulder, SidewalkCurb],
    )] {
        let forward =
            compile_generated_flat_multiway_junction_with_widths_and_profile_modes_raw_identity(
                &endpoint_angle_degrees,
                &edge_widths_m,
                &edge_profile_modes,
                GeneratedEdgeDirection::FromCenter,
                GeneratedEditOrder::Forward,
            );
        let reverse =
            compile_generated_flat_multiway_junction_with_widths_and_profile_modes_raw_identity(
                &endpoint_angle_degrees,
                &edge_widths_m,
                &edge_profile_modes,
                GeneratedEdgeDirection::FromCenter,
                GeneratedEditOrder::Reverse,
            );
        assert_canonical_node_raw_polygon_identity_eq("forward", &forward, "reverse", &reverse);
    }
    for (endpoint_angle_degrees, edge_widths_m, edge_profile_modes) in [(
        [0.0, 3.0, 47.0, 121.0, 202.0, 305.0],
        [6.5, 9.0, 5.0, 11.0, 8.0, 7.5],
        [
            SidewalkCurb,
            Shoulder,
            SidewalkCurb,
            Shoulder,
            SidewalkCurb,
            Shoulder,
        ],
    )] {
        let forward =
            compile_generated_flat_multiway_junction_with_widths_and_profile_modes_raw_identity(
                &endpoint_angle_degrees,
                &edge_widths_m,
                &edge_profile_modes,
                GeneratedEdgeDirection::FromCenter,
                GeneratedEditOrder::Forward,
            );
        let reverse =
            compile_generated_flat_multiway_junction_with_widths_and_profile_modes_raw_identity(
                &endpoint_angle_degrees,
                &edge_widths_m,
                &edge_profile_modes,
                GeneratedEdgeDirection::FromCenter,
                GeneratedEditOrder::Reverse,
            );
        assert_canonical_node_raw_polygon_identity_eq("forward", &forward, "reverse", &reverse);
    }
}

#[test]
fn flat_arbitrary_multiway_junction_matrix_preserves_exact_raw_polygon_identity() {
    for endpoint_angle_degrees in [
        [0.0, 11.0, 95.0, 194.0, 278.0],
        [0.0, 37.0, 118.0, 203.0, 291.0],
    ] {
        let forward = compile_generated_flat_multiway_junction_raw_identity(
            &endpoint_angle_degrees,
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
        );
        let reverse = compile_generated_flat_multiway_junction_raw_identity(
            &endpoint_angle_degrees,
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Reverse,
        );
        assert_canonical_node_raw_polygon_identity_eq("forward", &forward, "reverse", &reverse);
    }
    for endpoint_angle_degrees in [
        [0.0, 3.0, 47.0, 121.0, 202.0, 305.0],
        [0.0, 23.0, 61.0, 137.0, 211.0, 304.0],
    ] {
        let forward = compile_generated_flat_multiway_junction_raw_identity(
            &endpoint_angle_degrees,
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
        );
        let reverse = compile_generated_flat_multiway_junction_raw_identity(
            &endpoint_angle_degrees,
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Reverse,
        );
        assert_canonical_node_raw_polygon_identity_eq("forward", &forward, "reverse", &reverse);
    }
}

#[test]
fn flat_junctionn_canonical_raw_polygon_golden_checks_cover_generated_matrix() {
    assert_canonical_node_raw_polygon_golden(
        "flat_4way_cross",
        &compile_generated_flat_multiway_junction_raw_identity(
            &[0.0, 90.0, 180.0, 270.0],
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
        ),
        CanonicalNodeRawPolygonGolden {
            kind: RoadSurfaceVisualNodePieceKind::JunctionN,
            top_polygon_count: 111,
            carrier_record_count: 193,
            source_segment_record_count: 0,
            polygon_key_set_digest: 583720507171918577,
            top_owner_height_field_digest: 12764992536282700431,
            carrier_owner_source_height_field_digest: 13895527719892952529,
            source_segment_id_digest: 14695981039346656037,
            source_segment_ids: Vec::new(),
        },
    );
    assert_canonical_node_raw_polygon_golden(
        "flat_5way_ugly",
        &compile_generated_flat_multiway_junction_raw_identity(
            &[0.0, 11.0, 95.0, 194.0, 278.0],
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
        ),
        CanonicalNodeRawPolygonGolden {
            kind: RoadSurfaceVisualNodePieceKind::JunctionN,
            top_polygon_count: 237,
            carrier_record_count: 328,
            source_segment_record_count: 4,
            polygon_key_set_digest: 12869823528033325231,
            top_owner_height_field_digest: 7840774215341613341,
            carrier_owner_source_height_field_digest: 12715695643342842061,
            source_segment_id_digest: 15070362842391316786,
            source_segment_ids: vec![
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: CurbOrShoulder, owner_index: 16 }, source_kind: CurbOrShoulder, source_mouth_order_index: 2, source_band_index: 4, segment_start: NodeOwnedRegionArrangementKey { x_key: -4159045, z_key: 5659050 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -3636111, z_key: -318119 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: CurbOrShoulder, owner_index: 22 }, source_kind: CurbOrShoulder, source_mouth_order_index: 3, source_band_index: 4, segment_start: NodeOwnedRegionArrangementKey { x_key: -6915640, z_key: -5331409 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 846726, z_key: -3396035 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: CurbOrShoulder, owner_index: 22 }, source_kind: CurbOrShoulder, source_mouth_order_index: 3, source_band_index: 4, segment_start: NodeOwnedRegionArrangementKey { x_key: -6879352, z_key: -5476954 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 883015, z_key: -3541580 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 11 }, source_kind: Sidewalk, source_mouth_order_index: 1, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -954045, z_key: 4908136 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 4496993, z_key: 5967710 } }".to_owned(),
            ],
        },
    );
    assert_canonical_node_raw_polygon_golden(
        "flat_6way_near_parallel",
        &compile_generated_flat_multiway_junction_raw_identity(
            &[0.0, 3.0, 47.0, 121.0, 202.0, 305.0],
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
        ),
        CanonicalNodeRawPolygonGolden {
            kind: RoadSurfaceVisualNodePieceKind::JunctionN,
            top_polygon_count: 385,
            carrier_record_count: 496,
            source_segment_record_count: 10,
            polygon_key_set_digest: 12531129527583711903,
            top_owner_height_field_digest: 1540250823628247210,
            carrier_owner_source_height_field_digest: 11538454230488521145,
            source_segment_id_digest: 18002399232574592289,
            source_segment_ids: vec![
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 13577723, z_key: 19692301 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 16137461, z_key: 17305305 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 20 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 2, segment_start: NodeOwnedRegionArrangementKey { x_key: -1120218, z_key: 8659972 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 3000086, z_key: 1802633 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 33 }, source_kind: Carriageway, source_mouth_order_index: 5, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 2867033, z_key: 2007517 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 7455643, z_key: -4545700 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: CurbOrShoulder, owner_index: 16 }, source_kind: CurbOrShoulder, source_mouth_order_index: 2, source_band_index: 4, segment_start: NodeOwnedRegionArrangementKey { x_key: -2559738, z_key: 2386995 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 2896250, z_key: 8237824 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: CurbOrShoulder, owner_index: 22 }, source_kind: CurbOrShoulder, source_mouth_order_index: 3, source_band_index: 4, segment_start: NodeOwnedRegionArrangementKey { x_key: -7120390, z_key: 5054706 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -3000086, z_key: -1802633 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: CurbOrShoulder, owner_index: 28 }, source_kind: CurbOrShoulder, source_mouth_order_index: 4, source_band_index: 4, segment_start: NodeOwnedRegionArrangementKey { x_key: -6106348, z_key: -6241997 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 1311123, z_key: -3245143 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 17 }, source_kind: Sidewalk, source_mouth_order_index: 2, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -3656769, z_key: 3409992 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 868444, z_key: 8262688 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 17 }, source_kind: Sidewalk, source_mouth_order_index: 2, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -3656769, z_key: 3409992 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 868444, z_key: 8262688 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 18 }, source_kind: Sidewalk, source_mouth_order_index: 3, source_band_index: 0, segment_start: NodeOwnedRegionArrangementKey { x_key: -991643, z_key: 8737228 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 633964, z_key: 6031762 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 23 }, source_kind: Sidewalk, source_mouth_order_index: 3, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -8406141, z_key: 4282148 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -4285837, z_key: -2575190 } }".to_owned(),
            ],
        },
    );
}

fn compile_generated_flat_bend(
    angle_degrees: f32,
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
) -> GeneratedNodeCanonicalSignature {
    let (graph, center) = generated_bend_graph(
        angle_degrees,
        edge_direction,
        edit_order,
        GeneratedEndpointProfileMode::UseAuthoredPoints,
        flat_generated_point_at_xz,
        flat_generated_edge_points,
    );
    let terrain = flat_terrain(2048, 2048);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "generated flat Bend did not compile; angle_degrees={angle_degrees} edge_direction={edge_direction:?} edit_order={edit_order:?}: {}",
            canonical_node_pipeline_report(
                &surface,
                &graph,
                center,
                RoadSurfaceVisualNodePieceKind::Bend
            )
        );
    }
    let piece = assert_compiled_bend_piece(&surface, &graph, center);
    generated_node_canonical_signature(piece)
}

fn compile_generated_flat_t_junction(
    angle_degrees: f32,
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
) -> GeneratedNodeCanonicalSignature {
    let (graph, center) = generated_t_junction_graph(
        angle_degrees,
        edge_direction,
        edit_order,
        GeneratedEndpointProfileMode::UseAuthoredPoints,
        flat_generated_point_at_xz,
        flat_generated_edge_points,
    );
    let terrain = flat_terrain(2048, 2048);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "generated flat JunctionN did not compile; angle_degrees={angle_degrees} edge_direction={edge_direction:?} edit_order={edit_order:?}: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    generated_node_canonical_signature(piece)
}

fn compile_generated_flat_multiway_junction(
    endpoint_angle_degrees: &[f32],
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
) -> GeneratedNodeCanonicalSignature {
    let (graph, center) = generated_multiway_junction_graph(
        endpoint_angle_degrees,
        edge_direction,
        edit_order,
        GeneratedEndpointProfileMode::UseAuthoredPoints,
        flat_generated_point_at_xz,
        flat_generated_edge_points,
    );
    let terrain = flat_terrain(2048, 2048);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "generated flat multiway JunctionN did not compile; endpoint_angle_degrees={endpoint_angle_degrees:?} edge_direction={edge_direction:?} edit_order={edit_order:?}: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    generated_node_canonical_signature(piece)
}

fn compile_generated_flat_t_junction_raw_identity(
    angle_degrees: f32,
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
) -> CanonicalNodeRawPolygonIdentity {
    let (graph, center) = generated_t_junction_graph(
        angle_degrees,
        edge_direction,
        edit_order,
        GeneratedEndpointProfileMode::UseAuthoredPoints,
        flat_generated_point_at_xz,
        flat_generated_edge_points,
    );
    let terrain = flat_terrain(2048, 2048);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "generated flat JunctionN did not compile for raw identity; angle_degrees={angle_degrees} edge_direction={edge_direction:?} edit_order={edit_order:?}: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
    assert_compiled_junction_piece(&surface, &graph, center);
    canonical_node_raw_polygon_identity(&surface, &graph, center)
}

fn compile_generated_flat_multiway_junction_raw_identity(
    endpoint_angle_degrees: &[f32],
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
) -> CanonicalNodeRawPolygonIdentity {
    let (graph, center) = generated_multiway_junction_graph(
        endpoint_angle_degrees,
        edge_direction,
        edit_order,
        GeneratedEndpointProfileMode::UseAuthoredPoints,
        flat_generated_point_at_xz,
        flat_generated_edge_points,
    );
    let terrain = flat_terrain(2048, 2048);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "generated flat multiway JunctionN did not compile for raw identity; endpoint_angle_degrees={endpoint_angle_degrees:?} edge_direction={edge_direction:?} edit_order={edit_order:?}: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
    assert_compiled_junction_piece(&surface, &graph, center);
    canonical_node_raw_polygon_identity(&surface, &graph, center)
}

fn compile_generated_flat_multiway_junction_with_widths_raw_identity(
    endpoint_angle_degrees: &[f32],
    edge_widths_m: &[f32],
    edit_order: GeneratedEditOrder,
) -> CanonicalNodeRawPolygonIdentity {
    let (graph, center) = generated_multiway_junction_graph_with_edge_widths(
        GENERATED_CONFLICT_MATRIX_EDGE_LENGTH_M,
        endpoint_angle_degrees,
        edge_widths_m,
        GeneratedEdgeDirection::FromCenter,
        edit_order,
        GeneratedEndpointProfileMode::UseAuthoredPoints,
        flat_generated_point_at_xz,
        flat_generated_edge_points,
    );
    let terrain = flat_terrain(2048, 2048);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "generated flat mixed-width multiway JunctionN did not compile for raw identity; endpoint_angle_degrees={endpoint_angle_degrees:?} edge_widths_m={edge_widths_m:?} edit_order={edit_order:?}: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
    assert_compiled_junction_piece(&surface, &graph, center);
    canonical_node_raw_polygon_identity(&surface, &graph, center)
}

fn compile_generated_flat_multiway_junction_with_widths_and_profile_modes_raw_identity(
    endpoint_angle_degrees: &[f32],
    edge_widths_m: &[f32],
    edge_profile_modes: &[GeneratedEdgeProfileMode],
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
) -> CanonicalNodeRawPolygonIdentity {
    let (graph, center) = generated_multiway_junction_graph_with_edge_widths_and_profile_modes(
        GENERATED_CONFLICT_MATRIX_EDGE_LENGTH_M,
        endpoint_angle_degrees,
        edge_widths_m,
        edge_profile_modes,
        edge_direction,
        edit_order,
        GeneratedEndpointProfileMode::UseAuthoredPoints,
        flat_generated_point_at_xz,
        flat_generated_edge_points,
    );
    let terrain = flat_terrain(2048, 2048);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "generated flat mixed-width/profile multiway JunctionN did not compile for raw identity; endpoint_angle_degrees={endpoint_angle_degrees:?} edge_widths_m={edge_widths_m:?} edge_profile_modes={edge_profile_modes:?} edge_direction={edge_direction:?} edit_order={edit_order:?}: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
    assert_compiled_junction_piece(&surface, &graph, center);
    canonical_node_raw_polygon_identity(&surface, &graph, center)
}

fn compile_generated_flat_four_way_junction_raw_identity(
    first_branch_angle_degrees: f32,
    second_branch_angle_degrees: f32,
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
) -> CanonicalNodeRawPolygonIdentity {
    let (graph, center) = generated_four_way_junction_graph(
        first_branch_angle_degrees,
        second_branch_angle_degrees,
        edge_direction,
        edit_order,
        GeneratedEndpointProfileMode::UseAuthoredPoints,
        flat_generated_point_at_xz,
        flat_generated_edge_points,
    );
    let terrain = flat_terrain(2048, 2048);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "generated flat 4-way JunctionN did not compile for raw identity; first_branch_angle_degrees={first_branch_angle_degrees} second_branch_angle_degrees={second_branch_angle_degrees} edge_direction={edge_direction:?} edit_order={edit_order:?}: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
    assert_compiled_junction_piece(&surface, &graph, center);
    canonical_node_raw_polygon_identity(&surface, &graph, center)
}
