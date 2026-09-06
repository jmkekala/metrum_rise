// SPDX-License-Identifier: GPL-2.0-only

//! Span visual-piece coverage tests.

use super::*;

#[test]
fn span_visual_pieces_compile_explicit_band_polygons() {
    let terrain = flat_terrain(64, 64);
    let mut graph = RegionGraph::new();
    let a = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let b = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        a,
        b,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let span_piece = surface
        .compiled_visual_span_pieces()
        .get(&edge_idx)
        .unwrap();
    assert!(!span_piece.outer_boundary_loops.is_empty());
    assert!(!span_piece.road_surface_polygons.is_empty());
    assert!(!span_piece.curb_surface_polygons.is_empty());
    assert!(!span_piece.raised_step_face_polygons.is_empty());
    assert!(!span_piece.sidewalk_surface_polygons.is_empty());
    assert!(!span_piece.span_owned_regions.is_empty());
    assert_eq!(
        span_piece
            .span_owned_regions
            .iter()
            .filter(|region| region.role == RoadSurfaceSpanRegionRole::Asphalt)
            .count(),
        span_piece.road_surface_polygons.len()
    );
    assert_eq!(
        span_piece
            .span_owned_regions
            .iter()
            .filter(|region| region.role == RoadSurfaceSpanRegionRole::CurbOrShoulder)
            .count(),
        span_piece.curb_surface_polygons.len()
    );
    assert_eq!(
        span_piece
            .span_owned_regions
            .iter()
            .filter(|region| region.role == RoadSurfaceSpanRegionRole::NonRoad)
            .count(),
        span_piece.sidewalk_surface_polygons.len()
    );
    assert!(
        span_piece.span_owned_regions.iter().all(|region| {
            region.edge_idx == edge_idx
                && region.end_section_index == region.start_section_index + 1
                && region.end_s_m > region.start_s_m
        }),
        "span owned regions must preserve edge, section interval, and solved section authority"
    );
    assert!(!span_piece.span_earthwork_support_regions.is_empty());
    assert_eq!(
        span_piece.span_earthwork_support_regions.len(),
        span_piece.span_owned_regions.len(),
        "grounded standard span support regions should cover the same solved band-owned footprint as the visible span"
    );
    assert!(
        std::sync::Arc::ptr_eq(
            &span_piece.span_owned_regions,
            &span_piece.span_earthwork_support_regions
        ),
        "identical grounded visible/earthwork ranges must share one immutable region solve"
    );
    for role in [
        RoadSurfaceSpanRegionRole::Asphalt,
        RoadSurfaceSpanRegionRole::CurbOrShoulder,
        RoadSurfaceSpanRegionRole::NonRoad,
    ] {
        assert!(
            span_piece
                .span_earthwork_support_regions
                .iter()
                .any(|region| region.role == role),
            "span earthwork support regions must retain role/material provenance for {role:?}"
        );
    }
    assert!(
        span_piece
            .span_earthwork_support_regions
            .iter()
            .all(|region| {
                region.edge_idx == edge_idx
                    && region.end_section_index == region.start_section_index + 1
                    && region.end_s_m > region.start_s_m
                    && RoadSurfaceSystem::polygon_has_area_xz(&region.polygon.points_world)
            }),
        "span earthwork support regions must preserve edge, section interval, source band, and top-surface geometry"
    );
    assert_eq!(
        span_piece.span_raised_step_sources.len(),
        span_piece.raised_step_face_polygons.len()
    );
    assert!(
        span_piece.span_raised_step_sources.iter().all(|source| {
            source.lower_owner.kind != source.raised_owner.kind
                && source.end_section_index == source.start_section_index + 1
                && source.end_s_m > source.start_s_m
                && source.start_raised_world.y > source.start_lower_world.y
                && source.end_raised_world.y > source.end_lower_world.y
        }),
        "span raised-step faces must carry owner-pair and solved section provenance"
    );
    assert!(
        span_piece
            .road_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(
        span_piece
            .curb_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(
        span_piece.curb_surface_polygons.iter().all(|polygon| {
            polygon.triangles_world.iter().all(|triangle| {
                let min_y = triangle[0].y.min(triangle[1].y).min(triangle[2].y);
                let max_y = triangle[0].y.max(triangle[1].y).max(triangle[2].y);
                max_y - min_y <= 0.001
            })
        }),
        "curb top surface must be flat; vertical drop belongs to explicit raised-step faces"
    );
    assert!(
        span_piece
            .raised_step_face_polygons
            .iter()
            .all(|polygon| !RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(
        span_piece
            .sidewalk_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(!span_piece.earthwork_surface_polygons.is_empty());
    assert!(!span_piece.earthwork_outer_boundary_loops.is_empty());
    assert!(!span_piece.render_earthwork_faces.is_empty());
    assert_span_earthwork_faces_have_support_provenance(span_piece, edge_idx, EdgeClass::Standard);
    assert!(
        span_piece
            .earthwork_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(
        span_piece
            .render_earthwork_faces
            .iter()
            .all(|face| RoadSurfaceSystem::polygon_has_area_xz(&face.polygon.points_world))
    );
    assert_ne!(
        span_piece.earthwork_outer_boundary_loops,
        span_piece.outer_boundary_loops
    );
}

#[test]
fn logged_iso_junction_approaches_keep_span_boundary_closed() {
    let terrain = flat_terrain(32, 32);
    let approaches = [
        vec![
            Vector3::new(2660.5205, 96.00684, -8439.679),
            Vector3::new(2654.8027, 96.00069, -8441.475),
            Vector3::new(2649.085, 96.11087, -8443.2705),
            Vector3::new(2649.072, 95.98117, -8443.274),
            Vector3::new(2647.1638, 96.19458, -8443.874),
            Vector3::new(2645.2559, 96.302795, -8444.474),
            Vector3::new(2643.3672, 96.4315, -8445.066),
            Vector3::new(2643.3477, 96.43288, -8445.072),
            Vector3::new(2641.4395, 96.57346, -8445.672),
            Vector3::new(2639.5315, 96.72066, -8446.271),
            Vector3::new(2637.6494, 96.86405, -8446.862),
            Vector3::new(2637.6233, 96.865875, -8446.87),
            Vector3::new(2635.7153, 96.99178, -8447.47),
            Vector3::new(2633.8071, 97.099304, -8448.069),
            Vector3::new(2631.9316, 97.1837, -8448.658),
            Vector3::new(2631.8992, 97.18441, -8448.668),
            Vector3::new(2629.991, 97.21571, -8449.268),
            Vector3::new(2628.3213, 97.23196, -8449.792),
            Vector3::new(2626.2139, 97.25004, -8450.454),
            Vector3::new(2620.4958, 97.21966, -8452.25),
            Vector3::new(2614.7783, 97.14138, -8454.046),
            Vector3::new(2609.0605, 97.0903, -8455.842),
            Vector3::new(2603.3428, 97.12962, -8457.638),
            Vector3::new(2597.625, 97.250206, -8459.434),
            Vector3::new(2591.9072, 97.40034, -8461.2295),
            Vector3::new(2586.1895, 97.42548, -8463.025),
            Vector3::new(2580.4717, 97.4279, -8464.821),
            Vector3::new(2574.754, 97.63164, -8466.617),
            Vector3::new(2569.0361, 98.14152, -8468.413),
            Vector3::new(2563.3186, 99.100426, -8470.209),
            Vector3::new(2557.6006, 100.044205, -8472.005),
            Vector3::new(2551.8828, 100.714035, -8473.801),
            Vector3::new(2546.165, 101.2337, -8475.597),
            Vector3::new(2540.4473, 101.64737, -8477.393),
            Vector3::new(2534.7295, 101.80831, -8479.188),
            Vector3::new(2529.0117, 101.83623, -8480.984),
            Vector3::new(2523.294, 101.84559, -8482.78),
        ],
        vec![
            Vector3::new(2660.5205, 96.00684, -8439.679),
            Vector3::new(2654.783, 96.00182, -8441.406),
            Vector3::new(2649.0454, 96.112976, -8443.134),
            Vector3::new(2649.03, 95.98347, -8443.139),
            Vector3::new(2647.115, 96.19704, -8443.715),
            Vector3::new(2645.2, 96.30547, -8444.292),
            Vector3::new(2643.308, 96.43405, -8444.861),
            Vector3::new(2643.285, 96.435684, -8444.868),
            Vector3::new(2641.3699, 96.576355, -8445.445),
            Vector3::new(2639.4548, 96.723595, -8446.021),
            Vector3::new(2637.5706, 96.86667, -8446.589),
            Vector3::new(2637.5396, 96.868835, -8446.599),
            Vector3::new(2635.6248, 96.9949, -8447.175),
            Vector3::new(2633.7095, 97.10265, -8447.751),
            Vector3::new(2631.833, 97.18707, -8448.316),
            Vector3::new(2631.7944, 97.187935, -8448.328),
            Vector3::new(2629.8794, 97.22013, -8448.904),
            Vector3::new(2628.2036, 97.23718, -8449.409),
            Vector3::new(2626.0955, 97.25621, -8450.044),
            Vector3::new(2620.358, 97.22578, -8451.771),
            Vector3::new(2614.6208, 97.1451, -8453.499),
            Vector3::new(2608.8833, 97.09384, -8455.227),
            Vector3::new(2603.1458, 97.132774, -8456.954),
            Vector3::new(2597.4082, 97.26052, -8458.682),
            Vector3::new(2591.6707, 97.425575, -8460.41),
            Vector3::new(2585.9333, 97.44583, -8462.138),
            Vector3::new(2580.1958, 97.43921, -8463.865),
            Vector3::new(2574.4583, 97.61807, -8465.593),
            Vector3::new(2568.7207, 98.110245, -8467.32),
            Vector3::new(2562.9836, 99.05844, -8469.048),
            Vector3::new(2557.246, 99.97823, -8470.775),
            Vector3::new(2551.5085, 100.64799, -8472.503),
            Vector3::new(2545.771, 101.16062, -8474.23),
            Vector3::new(2540.0334, 101.57015, -8475.958),
            Vector3::new(2534.2961, 101.738205, -8477.686),
            Vector3::new(2528.5586, 101.80405, -8479.413),
            Vector3::new(2522.821, 101.828186, -8481.141),
        ],
    ];
    for points in approaches {
        let mut graph = RegionGraph::new();
        let a = graph.add_node(points[0], NodeType::Junction);
        let b = graph.add_node(*points.last().unwrap(), NodeType::Junction);
        let edge_idx = graph.add_edge(test_edge(
            a,
            b,
            points,
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        let mut surface = RoadSurfaceSystem::new(512.0);
        surface.compile_dirty(&graph, &terrain);
        let span = surface
            .compiled_visual_span_pieces()
            .get(&edge_idx)
            .expect("logged road approach must compile its span");
        assert_eq!(
            span.outer_boundary_loops.len(),
            1,
            "road span must not contain folded-band holes"
        );
    }
}
