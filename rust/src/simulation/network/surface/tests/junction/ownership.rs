// SPDX-License-Identifier: GPL-2.0-only

//! Junction ownership and overlay tests.

use super::*;

#[test]
fn junction_node_non_road_surface_is_footprint_minus_asphalt() {
    let mut graph = RegionGraph::new();
    let left = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let right = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    let up = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
    graph.add_edge(test_edge(
        left,
        center,
        vec![Vector3::new(-24.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        right,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        up,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 24.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_adjacency_list();

    let terrain = flat_terrain(96, 96);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    assert_compiled_junction_piece(&surface, &graph, center);
}

#[test]
fn footpath_join_claims_only_the_near_sidewalk_side() {
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-32.0, 0.0, 0.0), NodeType::Junction);
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let east = graph.add_node(Vector3::new(32.0, 0.0, 0.0), NodeType::Junction);
    let north = graph.add_node(Vector3::new(0.0, 0.0, 32.0), NodeType::Junction);
    graph.add_edge(test_edge(
        west,
        center,
        vec![Vector3::new(-32.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        east,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(32.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        north,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 32.0)],
        3.0,
        EdgeClass::Standard,
        TransitType::Foot,
        TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();
    graph.rebuild_adjacency_list();

    let terrain = flat_terrain(128, 128);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    let footpath_regions = piece
        .owned_regions
        .iter()
        .filter(|region| region.kind == RoadSurfaceBandKind::Footpath)
        .collect::<Vec<_>>();
    assert!(
        !footpath_regions.is_empty(),
        "foot-only incident edges must export explicit footpath-owned node regions"
    );

    let footpath_centroids = footpath_regions
        .iter()
        .flat_map(|region| region.polygon.triangles().map(triangle_centroid_xz))
        .collect::<Vec<_>>();
    assert!(
        footpath_centroids.iter().any(|centroid| centroid.y > 2.0),
        "the footpath join must reach the sidewalk side it connects to; centroids={footpath_centroids:?}"
    );
    assert!(
        footpath_centroids
            .iter()
            .all(|centroid| centroid.y >= -0.25),
        "the footpath join must not mirror across the road into the opposite sidewalk; centroids={footpath_centroids:?}"
    );

    let footpath_polygons = footpath_regions
        .iter()
        .map(|region| region.polygon.clone())
        .collect::<Vec<_>>();
    assert!(
        point_inside_visual_polygons(&footpath_polygons, Vector2::new(0.0, 7.0)),
        "the near sidewalk side must be connected to the footpath-owned region"
    );
    assert!(
        !point_inside_visual_polygons(&footpath_polygons, Vector2::new(0.0, -7.0)),
        "the opposite sidewalk side must keep its own owner instead of being claimed by the footpath"
    );
}

#[test]
fn node_overlay_preserves_skinny_closure_shapes() {
    let shapes = RoadSurfaceSystem::overlay_union_contours(&[vec![
        [0.0, 0.0],
        [2.0, 0.0],
        [2.0, 0.0005],
        [0.0, 0.0005],
    ]])
    .unwrap();

    assert_eq!(
        shapes.len(),
        1,
        "millimetre-scale deterministic closure slivers must not be filtered before rendering"
    );
}

// Saved physical geometry and clips for headlight-house node 8, edges 1, 7 and 8.
fn headlight_house_t_junction() -> (RegionGraph, u32) {
    #[derive(serde::Deserialize)]
    struct SavedArm {
        start: u32,
        end: u32,
        width: f32,
        fwd: u8,
        bkw: u8,
        start_clip: f32,
        end_clip: f32,
        points: Vec<[f32; 3]>,
    }
    let arms: Vec<SavedArm> =
        serde_json::from_str(include_str!("../data/headlight_house_t_junction.json")).unwrap();
    let mut graph = RegionGraph::new();
    let mut nodes = BTreeMap::new();
    for arm in arms {
        let points = arm
            .points
            .into_iter()
            .map(|[x, y, z]| Vector3::new(x, y, z))
            .collect::<Vec<_>>();
        let start = *nodes
            .entry(arm.start)
            .or_insert_with(|| graph.add_node(points[0], NodeType::Junction));
        let end = *nodes
            .entry(arm.end)
            .or_insert_with(|| graph.add_node(*points.last().unwrap(), NodeType::Junction));
        let mut edge = test_edge(
            start,
            end,
            points,
            arm.width,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        );
        edge.fwd_lanes = arm.fwd;
        edge.bkw_lanes = arm.bkw;
        edge.speed_limit = 50.0 / 3.6;
        edge.start_clip = arm.start_clip;
        edge.end_clip = arm.end_clip;
        graph.add_edge(edge);
    }
    (graph, nodes[&8])
}

#[test]
fn headlight_house_crosswalks_use_local_road_normals_and_shared_routes() {
    let (mut graph, center) = headlight_house_t_junction();
    let mut lanes = crate::simulation::network::lanes::LaneSystem::new();
    lanes.rebuild(&mut graph);
    let mut checked = 0;
    for &lane_id in &lanes.node_lanes[&(center as usize)] {
        let Some(marking) = lanes.crosswalk_marking(lane_id) else {
            continue;
        };
        let edge = graph.edge(marking.edge_id);
        let middle = (marking.start + marking.end) * 0.5;
        let point = Vector2::new(middle.x, middle.z);
        let (tangent, distance) = edge
            .physical_geometry
            .windows(2)
            .map(|pair| {
                let start = Vector2::new(pair[0].x, pair[0].z);
                let delta = Vector2::new(pair[1].x, pair[1].z) - start;
                let t = ((point - start).dot(delta) / delta.length_squared()).clamp(0.0, 1.0);
                (delta.normalized(), point.distance_to(start + delta * t))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        let across = Vector2::new(
            marking.end.x - marking.start.x,
            marking.end.z - marking.start.z,
        );
        assert!(distance < 0.0001, "crossing must use one road station");
        assert!(
            tangent.dot(across.normalized()).abs() < 0.00001,
            "crossing must be square to its local road tangent: {marking:?}"
        );
        assert!(
            (across.length()
                - (edge.fwd_lanes + edge.bkw_lanes) as f32 * crate::config::LANE_WIDTH)
                .abs()
                < 0.0001,
            "crossing must span the asphalt between both curbs"
        );
        for &connection in &lanes.node_lanes[&(center as usize)] {
            if lanes
                .crosswalk(connection)
                .map(|crosswalk| crosswalk.edge_id)
                != Some(marking.edge_id)
            {
                continue;
            }
            assert!(lanes.lanes[connection].geometry.windows(2).any(|pair| {
                (pair[0].distance_to(marking.start) < 0.0001
                    && pair[1].distance_to(marking.end) < 0.0001)
                    || (pair[1].distance_to(marking.start) < 0.0001
                        && pair[0].distance_to(marking.end) < 0.0001)
            }));
        }
        checked += 1;
    }
    assert_eq!(checked, 3);

    let terrain = flat_terrain(512, 512);
    let mut surface = RoadSurfaceSystem::new(128.0);
    surface.compile_dirty(&graph, &terrain);
    let mesh = crate::simulation::network::render::road::RoadRenderer
        .generate_mesh_data_with_surface(&graph, &lanes, &terrain, &surface);
    // Crossing paint shares asphalt vertices; no corner can float above it or hit the curb.
    let painted: Vec<_> = mesh
        .road_vertices
        .iter()
        .zip(&mesh.road_crossings)
        .filter(|(_, paint)| paint.z > 0.0)
        .map(|(p, _)| p)
        .collect();
    assert!(!painted.is_empty());
    assert!(
        painted.iter().all(|vertex| vertex.y.abs() < 0.001),
        "zebra vertices must stay on asphalt below the raised curb"
    );
}

#[test]
fn headlight_house_curved_sidewalk_join_stays_on_retained_bands() {
    let (graph, center) = headlight_house_t_junction();
    let terrain = flat_terrain(512, 512);
    let mut surface = RoadSurfaceSystem::new(128.0);
    surface.compile_dirty(&graph, &terrain);
    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    let compile_input = surface.visual_node_compile_input(&graph, center).unwrap();
    let input = RoadSurfaceSystem::build_node_arrangement_input_from_mouths(
        center,
        compile_input.kind,
        &compile_input.mouths,
    )
    .unwrap();
    let plan = node::joins::side_join_plan(&input).unwrap();
    let shapes_for = |points: Vec<backend::RoadVec3>| {
        let mut contour = overlay_contour_from_world_points(&points);
        if RoadSurfaceSystem::overlay_contour_area(&contour) < 0.0 {
            contour.reverse();
        }
        RoadSurfaceSystem::overlay_union_contours(&[contour]).unwrap()
    };
    let same_xz = |a: backend::RoadVec3, b: backend::RoadVec3| {
        backend::road_vec3_xz(a).distance(backend::road_vec3_xz(b)) < 0.0001
    };
    let mut checked = 0;
    // Saved edges 1 and 8 continue the same curve on the side opposite the T arm.
    let from = input
        .mouths
        .iter()
        .find(|mouth| mouth.edge_idx == 0)
        .unwrap();
    let to = input
        .mouths
        .iter()
        .find(|mouth| mouth.edge_idx == 2)
        .unwrap();
    for from_band in &from.band_intervals {
        if from_band.band_kind != RoadSurfaceBandKind::Sidewalk {
            continue;
        }
        let Some(to_band) = to.band_intervals.iter().find(|band| {
            band.band_kind == RoadSurfaceBandKind::Sidewalk
                && same_xz(from_band.endpoint_start_world, band.endpoint_end_world)
                && same_xz(from_band.endpoint_end_world, band.endpoint_start_world)
        }) else {
            continue;
        };
        let source_contours = [from_band, to_band].map(|band| {
            band.start_path_world
                .iter()
                .chain(band.end_path_world.iter().rev())
                .copied()
                .collect::<Vec<_>>()
        });
        let from_shapes = shapes_for(source_contours[0].clone());
        let to_shapes = shapes_for(source_contours[1].clone());
        let retained =
            RoadSurfaceSystem::overlay_binary_shapes(&from_shapes, &to_shapes, OverlayRule::Union)
                .unwrap();
        for join in plan.bands_by_mouth.iter().flatten().filter(|join| {
            join.band_kind == RoadSurfaceBandKind::Sidewalk
                && join.gap.from_edge_idx == from.edge_idx
                && join.gap.to_edge_idx == to.edge_idx
                && join.source_band_index == from_band.band_index
        }) {
            let joined = shapes_for(join.contour_world.clone());
            let extra = RoadSurfaceSystem::overlay_binary_shapes(
                &joined,
                &retained,
                OverlayRule::Difference,
            )
            .unwrap();
            assert!(
                overlay_area_m2(&extra) < 0.001,
                "same-side sidewalk join emitted an offset ribbon outside its source bands: {} m²",
                overlay_area_m2(&extra)
            );
        }
        checked += 1;
    }
    assert_eq!(
        checked, 2,
        "saved continuous curved sidewalk must be checked"
    );

    // Check actual emitted triangles as well as the pre-boolean same-side band domains.
    let mut contours = overlay_contours_from_top_polygons(piece.sidewalk_surface_polygons());
    for span in surface.compiled_visual_span_pieces().values() {
        for quad in span.sidewalk_surface_polygons() {
            contours.extend(
                quad.triangles()
                    .iter()
                    .map(|triangle| overlay_contour_from_world_points(triangle)),
            );
        }
    }
    let summed_area = contours
        .iter()
        .map(|contour| f64::from(RoadSurfaceSystem::overlay_contour_area(contour).abs()))
        .sum::<f64>();
    let union = RoadSurfaceSystem::overlay_union_contours(&contours).unwrap();
    assert!(
        (summed_area - f64::from(overlay_area_m2(&union))).abs() < 0.01,
        "emitted sidewalk triangles must not overlap at the curved ownership seam"
    );
}
