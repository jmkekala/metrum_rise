// SPDX-License-Identifier: GPL-2.0-only

//! Yards of rigid lots on a curved street: the verge their paving crosses to the road and the
//! hedge two lots share across the wedge between them.

use super::*;
use crate::simulation::buildings::allocator::geometry::{front_gap_at, lot_front_gaps};
use crate::simulation::buildings::allocator::yard::{
    YardHedgeRowWorld, fit_rows_to_shared_sides, shared_lot_sides,
};
use crate::simulation::zoning::{ParcelGeometry, ZoningSystem};

const RADIUS_M: f32 = 60.0;
// Road half-width plus sidewalk for one lane each way.
const OUTER_M: f32 = 3.5 + crate::config::SIDEWALK_WIDTH;

// A road bending round the origin at `RADIUS_M`, from -40 to 40 degrees about +X.
fn curved_road() -> RegionGraph {
    let mut zoning = ZoningSystem::new(&WorldConfig::default());
    let mut allocator = BuildingAllocator::new();
    let mut graph = RegionGraph::new();
    let mut network = TransitNetwork::new();
    let points = (-40..=40)
        .map(|deg| {
            let a = (deg as f32).to_radians();
            Vector3::new(RADIUS_M * a.cos(), 0.0, RADIUS_M * a.sin())
        })
        .collect();
    network.add_road(
        &mut graph,
        points,
        1,
        1,
        crate::simulation::network::types::EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    assert_eq!(
        graph.edge(0).width * 0.5 + crate::config::SIDEWALK_WIDTH,
        OUTER_M
    );
    graph
}

// A 20 x 30 m rigid lot on the outside of the curve whose front line touches the road's outer
// edge at angle `deg`.
fn outside_lot(deg: f32) -> ParcelGeometry {
    let a = deg.to_radians();
    let normal = Vector2::new(a.cos(), a.sin());
    let tangent = normal.orthogonal();
    let front_center = normal * (RADIUS_M + OUTER_M);
    let (half, depth) = (10.0, 30.0);
    let corners = [
        front_center - tangent * half,
        front_center + tangent * half,
        front_center + tangent * half + normal * depth,
        front_center - tangent * half + normal * depth,
    ];
    let min = corners
        .iter()
        .fold(Vector2::splat(f32::INFINITY), |m, c| m.coord_min(*c));
    let max = corners
        .iter()
        .fold(Vector2::splat(f32::NEG_INFINITY), |m, c| m.coord_max(*c));
    ParcelGeometry {
        edge_idx: 0,
        side: 1,
        frontage_center_t: 0.5,
        frontage_m: 2.0 * half,
        depth_m: depth,
        front_center,
        center: front_center + normal * (depth * 0.5),
        tangent,
        normal,
        corners,
        aabb_min: min,
        aabb_max: max,
    }
}

// Each lot's side row on the side facing the other lot, from its rear corner to the front row's
// line 0.75 m inside the lot, and its back row; as `plan_yard_hedge` lays them.
fn side_and_back_rows(lot: &ParcelGeometry, side: usize) -> [YardHedgeRowWorld; 2] {
    let (front, rear) = [
        (lot.corners[0], lot.corners[3]),
        (lot.corners[1], lot.corners[2]),
    ][side];
    let row = |from, to| YardHedgeRowWorld {
        from,
        to,
        join_from: true,
        join_to: true,
    };
    [
        row(rear, front + lot.normal * 0.75),
        row(lot.corners[3], lot.corners[2]),
    ]
}

#[test]
fn lot_front_gaps_measure_the_verge_a_curve_leaves_before_the_road() {
    let graph = curved_road();
    let lot = outside_lot(0.0);
    let gaps = lot_front_gaps(&graph, lot.center, -lot.normal, 10.0, 15.0);
    // A chord of 20 m on the road's 65 m outer edge clears it by sqrt(65^2 + 10^2) - 65 at its
    // ends and touches it in the middle; the polyline's 1 degree facets add under a centimetre.
    let end = ((RADIUS_M + OUTER_M).powi(2) + 100.0).sqrt() - (RADIUS_M + OUTER_M);
    for (got, want) in gaps.iter().zip([end, 0.0, end]) {
        assert!((got - want).abs() < 0.02, "gaps {gaps:?}, want ends {end}");
    }
    let quarter = ((RADIUS_M + OUTER_M).powi(2) + 25.0).sqrt() - (RADIUS_M + OUTER_M);
    assert!((front_gap_at(gaps, 0.5) - quarter).abs() < 0.02);
    // Turned so its front line runs off the end of the road, a corner fronts nothing.
    let past_end = outside_lot(46.0);
    let gaps = lot_front_gaps(&graph, past_end.center, -past_end.normal, 10.0, 15.0);
    assert_eq!(gaps[2], 0.0, "{gaps:?}");
}

#[test]
fn lots_splayed_by_a_curve_share_one_hedge_down_the_wedge_between_them() {
    let mut zoning = ZoningSystem::new(&WorldConfig::default());
    // Two neighbours whose front chords meet at 0 degrees; their sides splay 17.6 degrees apart.
    let half_angle = (10.0f32 / (RADIUS_M + OUTER_M)).atan().to_degrees();
    let a = outside_lot(-half_angle);
    let b = outside_lot(half_angle);
    let a_id = zoning.parcels.insert_new(a, 0).raw();
    let b_id = zoning.parcels.insert_new(b, 0).raw();

    // Corner 0 lies on +Z of its lot's normal: A's side 0 faces B, B's side 1 faces A.
    let mut a_rows = side_and_back_rows(&a, 0);
    let mut b_rows = side_and_back_rows(&b, 1);
    let rear_gap = a_rows[0].from.distance_to(b_rows[0].from);
    assert!(rear_gap > 8.0, "the wedge opens {rear_gap} m at the back");
    let a_sides = shared_lot_sides(&zoning, a_id);
    let b_sides = shared_lot_sides(&zoning, b_id);
    assert_eq!(
        (a_sides.len(), b_sides.len()),
        (1, 1),
        "only the sides facing each other"
    );
    fit_rows_to_shared_sides(&mut a_rows, &a_sides);
    fit_rows_to_shared_sides(&mut b_rows, &b_sides);

    // Both side rows now stand on one line, the wedge's centre, which runs radially at 0 degrees.
    for row in [a_rows[0], b_rows[0]] {
        for p in [row.from, row.to] {
            assert!(p.y.abs() < 0.01, "side row off the shared line: {row:?}");
        }
    }
    assert!(a_rows[0].from.distance_to(b_rows[0].from) < 0.01);
    assert!(a_rows[0].to.distance_to(b_rows[0].to) < 0.01);
    // Each back row runs on to meet it.
    assert!(a_rows[1].from.y.abs() < 0.01 && b_rows[1].to.y.abs() < 0.01);
    assert!(a_rows[1].from.distance_to(a_rows[0].from) < 0.6);
}

#[test]
fn lots_on_a_straight_street_keep_their_hedges_where_they_were_planned() {
    let mut zoning = ZoningSystem::new(&WorldConfig::default());
    let straight = |x: f32| {
        let mut lot = outside_lot(0.0);
        let shift = Vector2::new(0.0, x);
        lot.front_center += shift;
        lot.center += shift;
        lot.corners = lot.corners.map(|c| c + shift);
        lot.aabb_min += shift;
        lot.aabb_max += shift;
        lot
    };
    let (a, b) = (straight(-10.0), straight(10.0));
    let a_id = zoning.parcels.insert_new(a, 0).raw();
    zoning.parcels.insert_new(b, 0);
    let planned = side_and_back_rows(&a, 1);
    let mut rows = planned;
    fit_rows_to_shared_sides(&mut rows, &shared_lot_sides(&zoning, a_id));
    for (got, want) in rows.iter().zip(planned) {
        assert!(got.from.distance_to(want.from) < 1e-4 && got.to.distance_to(want.to) < 1e-4);
    }
}

#[test]
fn paving_drawn_to_the_lot_line_runs_on_across_the_verge_to_the_road() {
    let manifest: crate::assets::AssetManifest = toml::from_str(
        r#"
asset_id = "building.test.curved_driveway"
display_name = "Curved driveway"
[building]
placement_mode = "zoned_private"
zone_type = "residential"
lot_width_cells = 2
lot_depth_cells = 3
frontage_forward = [0, 0, 1]
[[site_surfaces]]
material = "asphalt"
vertices = [[-3, 8], [2, 8], [2, 15], [-3, 15]]
"#,
    )
    .unwrap();
    let mut allocator = BuildingAllocator::new();
    allocator.registry.register("base", manifest, String::new());
    let mut building = support::indexed_test_building(
        "base:building.test.curved_driveway".to_owned(),
        ZoneType::Residential,
        0,
    );
    (building.center_x, building.center_y) = (0.0, 0.0);
    building.facing_dir = Vector2::new(0.0, 1.0);
    (building.width_cells, building.depth_cells) = (2, 3);
    let driveway = |allocator: &mut BuildingAllocator, gaps: [f32; 3]| {
        allocator.buildings = vec![Building {
            front_gap_m: gaps,
            ..building.clone()
        }];
        allocator.rebuild_building_site_clients(10.0);
        allocator.building_sites[0].surfaces[0]
            .vertices_world
            .clone()
    };
    // The road bends away 1 m at both lot corners and touches the lot line at its centre.
    let curved = driveway(&mut allocator, [1.0, 0.0, 1.0]);
    let want = |x: f32| 15.0 + (x / 10.0).powi(2) + 0.1;
    for (got, want) in curved.iter().zip([
        Vector2::new(-3.0, 8.0),
        Vector2::new(2.0, 8.0),
        Vector2::new(2.0, want(2.0)),
        Vector2::new(-3.0, want(-3.0)),
    ]) {
        assert!(got.distance_to(want) < 1e-4, "{curved:?}");
    }
    // Along a straight road the lot line is the sidewalk's edge and nothing moves.
    let straight = driveway(&mut allocator, [0.0; 3]);
    assert_eq!(straight[2], Vector2::new(2.0, 15.0));
}
