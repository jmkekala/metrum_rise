// SPDX-License-Identifier: GPL-2.0-only

//! Road edit plan readiness, synchronous commit adoption and stale-dependency regressions.

use super::*;
use crate::nodes::sim::core::road_commit::RoadCommitRequest;
use crate::nodes::sim::core::road_preview::road_tool_snapshot_from_core;
use crate::nodes::sim::core::{RefinedTerrainPatchBuildInput, RoadEditPlan};
use crate::simulation::network::roadbed::RoadbedSystem;
use godot::prelude::Vector2;

#[test]
fn cold_road_commit_completes_terrain_plan_before_mutation() {
    let mut core = test_core();
    core.precompute_road_mesh_data();
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    let edges_before = core.region_graph.edge_count();

    let outcome = core.commit_road(
        RoadCommitRequest {
            points: vec![Vector3::new(-48.0, 0.0, 0.0), Vector3::new(48.0, 0.0, 0.0)],
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            preview: None,
        },
        std::time::Instant::now(),
        Default::default(),
    );

    assert!(outcome.committed, "{}", outcome.rejection);
    assert_eq!(core.region_graph.edge_count(), edges_before + 1);
    assert!(!core.last_road_edit_metrics.preview_plan_reused);
    assert!(!core.transit_network.road_edit_is_staged());
}

fn prepare_with_lanes(
    core: &mut SimCore,
    points: Vec<Vector3>,
    fwd_lanes: i32,
    bkw_lanes: i32,
) -> RoadEditPlan {
    core.precompute_road_mesh_data();
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    let surface_generation = core.road_tool_surface_generation;
    RoadEditPlan::compile(
        core,
        RoadPreviewRequest {
            surface_generation,
            points,
            fwd_lanes,
            bkw_lanes,
            snap_to_existing_roads: true,
        },
    )
}

fn inputs(core: &mut SimCore) -> Vec<RefinedTerrainPatchBuildInput> {
    core.collect_refined_terrain_patch_build_inputs(2.0)
}

pub(super) fn commit_ready(core: &mut SimCore, points: Vec<Vector3>) -> RoadEditPlan {
    commit_ready_with_lanes(core, points, 1, 1)
}

fn commit_ready_with_lanes(
    core: &mut SimCore,
    points: Vec<Vector3>,
    fwd_lanes: i32,
    bkw_lanes: i32,
) -> RoadEditPlan {
    let plan = prepare_with_lanes(core, points.clone(), fwd_lanes, bkw_lanes);
    assert_eq!(plan.status(core), "ready", "{:?}", plan.validation());
    core.transit_network.begin_road_edit();
    core.transit_network.bulk_load = true;
    let added = core.add_road_internal_with_snap_and_validation(
        points,
        fwd_lanes,
        bkw_lanes,
        true,
        Some(&plan),
    );
    assert!(added.committed, "{}", core.last_road_timing);
    assert!(added.finalized_geometry.is_some());
    core.transit_network.bulk_load = false;
    assert!(
        core.validate_staged_road_render(),
        "{}",
        core.last_road_timing
    );
    assert!(!core.transit_network.road_edit_is_staged());
    plan
}

#[test]
fn kuopio_third_road_preserves_existing_junction_boundary_owners() {
    for (trunk_end_x, branch_end) in [
        (2_889.749_8, (3020.0, -8_622.83)),
        (2_889.749_5, (3020.73, -8_622.673)),
    ] {
        let mut core = test_core();
        core.load_world_definition_internal(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../godot/bootstrap/worlds/kuopio_324km2_10m.sqlite"
            ),
            crate::simulation::vegetation::VegetationConfig::default(),
        )
        .unwrap();
        // The first case reproduces road.log's exact missing outer-boundary coordinates;
        // the second exposes the same short-edge deletion in the third road's new junction.
        for (start, end) in [
            ((2_816.885_7, -8_504.609), (trunk_end_x, -9000.0)),
            ((2839.961, -8661.495), branch_end),
            ((2940.0, -8640.0), (2960.0, -8870.0)),
        ] {
            let points = [start, end]
                .map(|(x, z)| {
                    Vector3::new(
                        x,
                        core.get_world_surface_height_internal(Vector2::new(x, z)),
                        z,
                    )
                })
                .to_vec();
            commit_ready(&mut core, points);
        }
        assert_eq!(core.region_graph.edge_count(), 5);
    }
}

#[test]
fn ready_plan_atomically_adopts_crossings_extensions_and_close_double_t() {
    let mut core = test_core();
    core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 129, 0.0);
    core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
    // Each junction end needs half of its arm for the 3.5 m carriageway edge, the 2 m minimum
    // curb return and the 3 m crosswalk (D1, D8), so neighbouring junctions sit 18 m apart.
    for ends in [
        ((-60.0, 0.0), (60.0, 0.0)),
        ((0.0, -48.0), (0.0, 48.0)),
        ((60.0, 0.0), (100.0, 0.0)),
        ((24.0, -48.0), (24.0, 0.0)),
        ((42.0, 48.0), (42.0, 0.0)),
    ] {
        let points = [ends.0, ends.1]
            .map(|(x, z)| Vector3::new(x, 0.0, z))
            .to_vec();
        let plan = commit_ready(&mut core, points.clone());
        assert_ne!(plan.status(&core), "ready");
        let count = core.region_graph.edge_count();
        assert!(
            !core
                .add_road_internal_with_snap_and_validation(points, 1, 1, true, Some(&plan))
                .committed
        );
        assert_eq!(core.region_graph.edge_count(), count);
    }
}

#[test]
fn complete_readiness_rechecks_water_inputs_and_pending_dependencies() {
    let mut core = test_core();
    core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 129, 0.0);
    core.watermap = WaterSystem::new(257, 257);
    core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
    let points = vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)];
    let plan = prepare_with_lanes(&mut core, points.clone(), 1, 1);
    assert_eq!(plan.status(&core), "ready");
    let mut wrong = points.clone();
    wrong[0].y += 0.01;
    assert!(
        !core
            .add_road_internal_with_snap_and_validation(wrong, 1, 1, true, Some(&plan))
            .committed
    );
    assert_eq!(
        plan.status(&core),
        "ready",
        "an input mismatch must not consume the plan"
    );
    core.watermap
        .replace_baseline_depth_from_dense(&vec![2.0; 257 * 257])
        .unwrap();
    assert_eq!(plan.status(&core), "invalid");
    core.watermap
        .replace_baseline_depth_from_dense(&vec![0.0; 257 * 257])
        .unwrap();
    assert_eq!(plan.status(&core), "ready");
    core.heightmap.set_height(0, 0, 0.25);
    assert_eq!(plan.status(&core), "stale");
    core.precompute_road_mesh_data();
    let rebuilt = RoadEditPlan::compile(
        &core,
        RoadPreviewRequest {
            surface_generation: core.road_tool_surface_generation,
            points,
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
        },
    );
    assert_eq!(rebuilt.status(&core), "ready");
    assert_eq!(core.region_graph.edge_count(), 0);
    assert!(core.undo_stack.is_empty());
}

#[test]
fn new_parcel_rejects_ready_plan_without_consuming_it() {
    let mut core = test_core();
    core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 129, 0.0);
    core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
    commit_ready(
        &mut core,
        vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
    );
    let points = vec![Vector3::new(0.0, 0.0, -48.0), Vector3::ZERO];
    let plan = prepare_with_lanes(&mut core, points.clone(), 1, 1);
    assert_eq!(plan.status(&core), "ready");
    let profile = core
        .zoning
        .profiles
        .default_runtime_id_for_zone_type(ZoneType::Residential)
        .unwrap();
    let parcel = core
        .zoning
        .place_or_rezone_default_parcel_at(0.0, -20.0, profile, &core.region_graph)
        .unwrap();
    assert_eq!(plan.status(&core), "invalid");
    let before = core.region_graph.edge_count();
    assert!(
        !core
            .add_road_internal_with_snap_and_validation(points, 1, 1, true, Some(&plan))
            .committed
    );
    assert_eq!(core.region_graph.edge_count(), before);
    core.zoning
        .remove_parcels_by_raw_ids(&std::collections::HashSet::from([parcel.raw()]));
    assert_eq!(plan.status(&core), "ready");
}

// Commits one stroke through the player's path and returns its rejection reason, empty when
// the stroke was committed.
fn commit_stroke(core: &mut SimCore, points: &[Vector3]) -> String {
    core.precompute_road_mesh_data();
    core.commit_road(
        RoadCommitRequest {
            points: points.to_vec(),
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            preview: None,
        },
        std::time::Instant::now(),
        Default::default(),
    )
    .rejection_reason
}

// The road tool's preview of `points` on the view the core publishes now.
fn preview_validation(
    core: &mut SimCore,
    points: &[Vector3],
) -> crate::simulation::network::road_edit::RoadPreviewValidation {
    use crate::nodes::sim::core::road_tool_context::RoadPreviewContext;
    core.precompute_road_mesh_data();
    core.refresh_road_terrain_inputs();
    let view = road_tool_snapshot_from_core(core).expect("current roadbed");
    RoadPreviewContext::capture(view)
        .preview_road(RoadPreviewRequest {
            surface_generation: core.road_tool_surface_generation,
            points: points.to_vec(),
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
        })
        .validation
}

// Why the road tool would not submit `points`, empty when it would.
fn preview_refusal(core: &mut SimCore, points: &[Vector3]) -> &'static str {
    let validation = preview_validation(core, points);
    if validation.is_valid {
        ""
    } else {
        validation.invalid_reason
    }
}

// Fields and painted cells are tested against the pieces the commit installs (`ROAD-68` open
// question 1), so a plan stays ready and the commit refuses the edit, leaving the graph, the
// roadbed and the undo history as they were. The preview runs the same test on the view's
// indices and shows the refusal before the click (open question 25).
fn assert_reservation_refuses(core: &mut SimCore, points: &[Vector3], reason: &str) {
    let before = (
        core.region_graph.edge_count(),
        core.undo_stack.len(),
        core.transit_network.roadbed.generation(),
    );
    assert_eq!(preview_refusal(core, points), reason, "preview");
    assert_eq!(commit_stroke(core, points), reason);
    assert!(!core.transit_network.road_edit_is_staged());
    let after = (
        core.region_graph.edge_count(),
        core.undo_stack.len(),
        core.transit_network.roadbed.generation(),
    );
    assert_eq!((after.0, after.1), (before.0, before.1));
    assert!(core.transit_network.roadbed.failed_nodes().is_empty());
}

// A 256 m world sloped along +x at `grade`, and a 180 m stroke straight up the fall line through
// its centre at the ground's height.
fn fall_line_stroke_at(grade: f32) -> (SimCore, Vec<Vector3>) {
    let mut core = test_core();
    core.treasury.balance = 1.0e9;
    core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 64, 0.0);
    core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
    let height = |x: f32| 60.0 + x * grade;
    for z in 0..257 {
        for x in 0..257 {
            core.heightmap.set_height(
                x,
                z,
                height(x as f32 - 128.0) / crate::config::HEIGHT_SCALE,
            );
        }
    }
    let points = vec![
        Vector3::new(-90.0, height(-90.0), 0.0),
        Vector3::new(90.0, height(90.0), 0.0),
    ];
    (core, points)
}

// The maximum grade is 1:3, refused as `road_too_steep`; from 16% the tool warns without
// refusing (`ROAD-68` open question 37). The preview and the commit plan with the same function,
// so the preview shows the refusal before the click.
#[test]
fn a_road_up_to_one_in_three_commits_and_a_steeper_one_is_refused() {
    for (grade, warned) in [(0.20, true), (0.30, true), (0.10, false)] {
        let (mut core, points) = fall_line_stroke_at(grade);
        let preview = preview_validation(&mut core, &points);
        assert!(preview.is_valid, "{grade}: {}", preview.invalid_reason);
        assert!((preview.max_grade - grade).abs() < 0.02, "{grade}: {}", preview.max_grade);
        assert_eq!(preview.warning_reason == "road_steep", warned, "{grade}");
        assert_eq!(commit_stroke(&mut core, &points), "", "{grade}");
        for edge in core.region_graph.edges().iter().filter(|edge| !edge.deleted) {
            let steepest = crate::simulation::network::grade::steepest_grade(
                &edge.physical_geometry,
            )
            .unwrap();
            assert!(steepest.grade <= 1.0 / 3.0, "{grade}: {steepest:?}");
        }
    }
    let (mut core, points) = fall_line_stroke_at(0.40);
    let preview = preview_validation(&mut core, &points);
    assert_eq!(preview.invalid_reason, "road_too_steep");
    assert!((preview.geometry_measured - 0.40).abs() < 0.02, "{}", preview.geometry_measured);
    assert_eq!(preview.geometry_limit, 1.0 / 3.0);
    let edges = core.region_graph.edge_count();
    assert_eq!(commit_stroke(&mut core, &points), "road_too_steep");
    assert_eq!(core.region_graph.edge_count(), edges);
}

// A node edit may not steepen a road past 1:3 either; one that keeps it within commits
// (`ROAD-68` open question 37).
#[test]
fn a_node_move_that_steepens_a_road_past_one_in_three_is_refused() {
    let (mut core, points) = fall_line_stroke_at(0.10);
    assert_eq!(commit_stroke(&mut core, &points), "");
    let end = core
        .region_graph
        .find_node_within(points[1], crate::config::SNAP_TOLERANCE)
        .unwrap();
    let at = core.region_graph.node(end).pos;
    // The move deforms the road with a smoothstep over its 180 m, so 60 m adds up to 50%.
    assert!(!core.move_network_node_internal(end as i32, at + Vector3::new(0.0, 60.0, 0.0)));
    assert_eq!(
        core.last_road_rejection.as_ref().map(|rejection| rejection.reason.as_str()),
        Some("road_too_steep")
    );
    assert_eq!(core.region_graph.node(end).pos, at);
    assert!(core.move_network_node_internal(end as i32, at + Vector3::new(0.0, 0.5, 0.0)));
}

#[test]
fn new_field_rejects_ready_road_plan_and_releasing_land_restores_readiness() {
    let mut core = test_core();
    commit_ready(
        &mut core,
        vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
    );
    let points = vec![Vector3::new(0.0, 0.0, -48.0), Vector3::ZERO];
    let plan = prepare_with_lanes(&mut core, points.clone(), 1, 1);
    assert_eq!(plan.status(&core), "ready");
    let field = [
        godot::prelude::Vector2::new(-10.0, -40.0),
        godot::prelude::Vector2::new(10.0, -40.0),
        godot::prelude::Vector2::new(10.0, -15.0),
        godot::prelude::Vector2::new(-10.0, -15.0),
    ];
    core.allocator.field_clearance_mut().set(0, &field);
    assert_reservation_refuses(&mut core, &points, "field_overlap");
    core.allocator.field_clearance_mut().clear();
    assert_eq!(preview_refusal(&mut core, &points), "");
    assert_eq!(commit_stroke(&mut core, &points), "");
}

// A junction corner reaches beyond the corridors of both its roads: a field in the curb return
// of a T, clear of the side road's and the through road's corridors, refuses the side road.
#[test]
fn a_field_in_a_junction_corner_refuses_the_road() {
    let through = vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)];
    let side = vec![Vector3::new(0.0, 0.0, -48.0), Vector3::ZERO];
    // The corridor half width of a 1+1 road: what the replaced corridor checks covered.
    let corridor_m = crate::config::LANE_WIDTH + crate::config::SIDEWALK_WIDTH;
    let mut probe = test_core();
    commit_ready(&mut probe, through.clone());
    assert_eq!(commit_stroke(&mut probe, &side), "");
    let roadbed = &probe.transit_network.roadbed;
    // The covered point nearest the corner of the two corridors, in the side road's quadrant.
    let corner = (1..40)
        .map(|step| corridor_m + 0.05 * step as f32)
        .map(|offset| (offset, -offset))
        .find(|&(x, z)| roadbed.height_at(x, z).is_some())
        .expect("the junction piece covers the corner outside both corridors");
    let field = [
        Vector2::new(corner.0 - 0.02, corner.1 - 0.02),
        Vector2::new(corner.0 + 0.02, corner.1 - 0.02),
        Vector2::new(corner.0 + 0.02, corner.1 + 0.02),
        Vector2::new(corner.0 - 0.02, corner.1 + 0.02),
    ];
    let mut core = test_core();
    commit_ready(&mut core, through);
    core.allocator.field_clearance_mut().set(0, &field);
    assert!(
        !core
            .allocator
            .field_clearance
            .overlaps_road_corridor(&side, corridor_m)
    );
    assert_reservation_refuses(&mut core, &side, "field_overlap");
}

#[test]
fn painted_cells_reject_a_ready_road_plan_and_erase_restores_readiness() {
    use crate::simulation::zoning::cells::{CellBounds, CellSelectionShape};
    use glam::DVec2;
    let mut core = test_core();
    commit_ready(
        &mut core,
        vec![Vector3::new(-60.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
    );
    let points = vec![Vector3::new(0.0, 0.0, -48.0), Vector3::ZERO];
    let plan = prepare_with_lanes(&mut core, points.clone(), 1, 1);
    assert_eq!(plan.status(&core), "ready");
    core.zoning.generate_cells(
        &core.region_graph,
        CellBounds {
            min: DVec2::splat(-100.0),
            max: DVec2::splat(100.0),
        },
        |_| false,
    );
    let point = DVec2::new(0.0, -25.0);
    let selection = core.zoning.cells.select(CellSelectionShape::Cell, &[point]);
    assert!(!selection.cells.is_empty());
    core.zoning.paint_cells(&selection, 1, |_| false).unwrap();
    assert_reservation_refuses(&mut core, &points, "zoning_cell_overlap");
    let selection = core.zoning.cells.select(CellSelectionShape::Cell, &[point]);
    core.zoning.paint_cells(&selection, 0, |_| false).unwrap();
    assert_eq!(preview_refusal(&mut core, &points), "");
    assert_eq!(commit_stroke(&mut core, &points), "");
}

#[test]
fn structural_road_plans_adopt_with_and_without_terrain_cutouts() {
    for height in [-3.0, 10.0] {
        let mut core = test_core();
        core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 129, 0.0);
        core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
        // The tunnel needs a visible portal; a wholly buried span has no display product
        // and is rejected by the existing surface compiler. The bridge has no cutouts.
        let points = if height < 0.0 {
            vec![
                Vector3::new(-60.0, 0.0, 0.0),
                Vector3::new(-40.0, height, 0.0),
                Vector3::new(60.0, height, 0.0),
            ]
        } else {
            vec![
                Vector3::new(-40.0, height, 0.0),
                Vector3::new(40.0, height, 0.0),
            ]
        };
        let plan = prepare_with_lanes(&mut core, points.clone(), 1, 1);
        assert_eq!(plan.status(&core), "ready");
        core.transit_network.begin_road_edit();
        core.transit_network.bulk_load = true;
        let added =
            core.add_road_internal_with_snap_and_validation(points, 1, 1, true, Some(&plan));
        assert!(added.committed);
        core.transit_network.bulk_load = false;
        assert!(
            core.validate_staged_road_render(),
            "height {height}: {}",
            core.last_road_timing
        );
    }
}

const KUOPIO_REFERENCE_SAVE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../benchmarks/fixtures/kuopio-terrain/kuopio-terrain-map.sqlite"
);

// Open questions 8 and 37: the reference save is re-recorded with today's code (question 38) and
// loads as saved: a load repairs nothing (save compatibility is not required, `ROAD-68-K21`),
// every saved control and physical point comes back bitwise and no node fails.
#[test]
fn kuopio_saved_reference_loads_as_saved() {
    let fixture = KUOPIO_REFERENCE_SAVE;
    let mut core = test_core();
    core.load_game_internal(fixture).unwrap();
    let connection =
        rusqlite::Connection::open_with_flags(fixture, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let mut statement = connection.prepare(
        "SELECT edge_id, point_index, x, y, z, physical FROM network_edge_geometry ORDER BY edge_id, physical, point_index",
    ).unwrap();
    let mut rows = statement.query([]).unwrap();
    while let Some(row) = rows.next().unwrap() {
        let edge_id: usize = row.get(0).unwrap();
        let point_index: usize = row.get(1).unwrap();
        let stored = Vector3::new(
            row.get(2).unwrap(),
            row.get(3).unwrap(),
            row.get(4).unwrap(),
        );
        let physical: bool = row.get(5).unwrap();
        let edge = core.region_graph.edge(edge_id);
        let restored = if physical {
            &edge.physical_geometry
        } else {
            &edge.geometry
        };
        assert_eq!(
            restored.get(point_index),
            Some(&stored),
            "saved geometry changed on load: edge {edge_id} point {point_index} physical {physical}"
        );
    }
    core.precompute_road_mesh_data();
    let roadbed = &core.transit_network.roadbed;
    assert!(
        roadbed.is_current() && roadbed.failed_nodes().is_empty(),
        "{:?}",
        roadbed.failed_nodes()
    );
}

// `TERRAIN-07-A` `T2` and `T7` on Kuopio: every road tile of the reference save baked at
// `GL0` and `GL1`; prints the conflict samples and row maxima.
#[test]
#[ignore = "TERRAIN-07-A census: cargo test --release --lib kuopio_graded_census -- --ignored --nocapture"]
fn kuopio_graded_census() {
    let mut core = test_core();
    core.load_game_internal(KUOPIO_REFERENCE_SAVE).unwrap();
    core.precompute_road_mesh_data();
    for level in [
        crate::simulation::terrain::graded::Level::Gl0,
        crate::simulation::terrain::graded::Level::Gl1,
    ] {
        let census = crate::simulation::terrain::graded::tests::graded_census(
            &core.heightmap,
            &core.transit_network.roadbed,
            &core.allocator,
            level,
        );
        eprintln!("GRADED_CENSUS {}", serde_json::json!({"fixture": "kuopio", "census": census}));
    }
}

// `TERRAIN-07-A` `T18` (a): the two-pass exact pruning on Kuopio's steepest road, the 4 x 4
// tiles around the dead end on a 1:2 side slope (`KUOPIO_DEAD_END`): pruned equals
// unpruned bitwise in any batch, order and worker count, with second-pass samples; and the
// lanes equal one point at a time on every road tile.
#[test]
fn kuopio_pruned_bake_equals_every_seam() {
    use crate::simulation::terrain::graded::tests::assert_lanes_equal_scalar_on_road_tiles;
    use crate::simulation::terrain::graded::{Level, TileKey};
    let mut core = test_core();
    core.load_game_internal(KUOPIO_REFERENCE_SAVE).unwrap();
    core.precompute_road_mesh_data();
    let tiles: Vec<TileKey> = (0..4)
        .flat_map(|x| (16..20).map(move |z| TileKey { x, z }))
        .collect();
    let gl0 = [TileKey { x: 1, z: 17 }, TileKey { x: 2, z: 16 }];
    let (_, second_pass) = crate::simulation::terrain::graded::tests::assert_exact_pruning(
        &core.heightmap,
        &core.transit_network.roadbed,
        &core.allocator,
        &tiles,
        &gl0,
        "kuopio",
    );
    assert!(second_pass > 0, "Kuopio's cuts need the second pass");
    // The lanes against one point at a time on every Kuopio road tile.
    let checked = assert_lanes_equal_scalar_on_road_tiles(
        &core.heightmap,
        &core.transit_network.roadbed,
        Level::Gl0,
        "kuopio",
    );
    assert!(checked > 500, "every Kuopio road tile: {checked}");
}

// `TERRAIN-07-A` `T18`: the graded terrain stage of Kuopio road edits against the 3.0 ms budget
// (run on four workers). Every recorded stroke on the source world, each case alone; then
// `kuopio_12`'s strokes again after every other case, the edited neighbourhood fixed and the
// rest of the recorded city added, with the same tiles and texel bits; and on the reference
// save a T up a 1:4 hillside (`KUOPIO_HILLSIDE_T`) and the removal of the dead end beside a 1:2
// slope (`KUOPIO_DEAD_END`), both found by place.
#[test]
#[ignore = "controlled release timing: cargo test --release --lib benchmark_kuopio_graded_edits -- --ignored --nocapture"]
fn benchmark_kuopio_graded_edits() {
    use crate::simulation::terrain::graded::tests::graded_stage;
    let stage = |core: &SimCore, before: &RoadbedSystem| {
        graded_stage(
            &core.heightmap,
            before,
            &core.transit_network.roadbed,
            &core.allocator,
            &core.last_road_terrain_changes,
        )
    };
    let manifest = kuopio_manifest();
    let cases = manifest["cases"].as_array().unwrap();
    let neighbourhood = |case: &serde_json::Value| case["case_id"] == "kuopio_12";
    let mut alone = Vec::new();
    for case in cases {
        let mut core = kuopio_source_core();
        replay_kuopio_case(&mut core, case, |stroke, core, before| {
            let mut record = stage(core, before);
            record["case"] = case["case_id"].clone();
            record["attempt"] = stroke["attempt"].clone();
            record["background_edges"] = 0.into();
            eprintln!("GRADED_KUOPIO_EDIT {record}");
            if neighbourhood(case) {
                alone.push(record);
            }
        });
    }
    let mut core = kuopio_source_core();
    for case in cases.iter().filter(|case| {
        !case["case_id"]
            .as_str()
            .is_some_and(|id| id.starts_with("kuopio_12"))
    }) {
        replay_kuopio_case(&mut core, case, |_, _, _| {});
    }
    let background = core
        .region_graph
        .edges()
        .iter()
        .filter(|edge| !edge.deleted)
        .count();
    let case = cases.iter().find(|case| neighbourhood(case)).unwrap();
    let mut stroke_index = 0;
    replay_kuopio_case(&mut core, case, |stroke, core, before| {
        let mut record = stage(core, before);
        let local = &alone[stroke_index];
        stroke_index += 1;
        assert_eq!(record["tile_keys"], local["tile_keys"], "the same tiles");
        assert_eq!(record["digest"], local["digest"], "the same texel bits");
        record["case"] = case["case_id"].clone();
        record["attempt"] = stroke["attempt"].clone();
        record["background_edges"] = background.into();
        eprintln!("GRADED_KUOPIO_EDIT {record}");
    });

    let mut core = test_core();
    core.load_game_internal(KUOPIO_REFERENCE_SAVE).unwrap();
    core.precompute_road_mesh_data();
    drop(core.transit_network.roadbed.take_terrain_changes());
    let saved_edges = core.region_graph.edge_count();
    // The T: from the hillside road's middle vertex 60 m up the slope across it.
    let (road, ends) = KUOPIO_HILLSIDE_T;
    let road = kuopio_edge_at(&core, road);
    let start = Vector2::new(ends[0][0], ends[0][1]);
    let off_road = core
        .region_graph
        .edge(road)
        .physical_geometry
        .iter()
        .fold(f32::INFINITY, |d, p| {
            d.min(Vector2::new(p.x, p.z).distance_to(start))
        });
    assert!(
        off_road < 0.01,
        "the T starts on a vertex of edge {road}: {off_road} m off"
    );
    let before = core.transit_network.roadbed.snapshot();
    let points = road_tool_points(&mut core, ends);
    commit_ready_with_lanes(&mut core, points, 1, 1);
    let mut record = stage(&core, &before);
    record["edit"] = "hillside_t".into();
    record["edge"] = road.into();
    record["background_edges"] = saved_edges.into();
    eprintln!("GRADED_KUOPIO_EDIT {record}");
    // The removal of the dead end.
    let dead_end = kuopio_edge_at(&core, KUOPIO_DEAD_END);
    let before = core.transit_network.roadbed.snapshot();
    assert!(core.bulldoze_road_edge_for_test(dead_end));
    core.precompute_road_mesh_data();
    let changes = core.transit_network.roadbed.take_terrain_changes();
    let mut record = graded_stage(
        &core.heightmap,
        &before,
        &core.transit_network.roadbed,
        &core.allocator,
        &changes,
    );
    record["edit"] = "remove_dead_end".into();
    record["edge"] = dead_end.into();
    record["background_edges"] = saved_edges.into();
    eprintln!("GRADED_KUOPIO_EDIT {record}");
}

// `TERRAIN-07-A` `T2` (c): the conflict walls on Kuopio and the five worst sites by wall height.
// With `METRUM_CONFLICT_DUMP_DIR` set, each site's graded samples, seams and top triangles are
// written there as `site-<rank>.json` for renders.
#[test]
#[ignore = "TERRAIN-07-A census: cargo test --release --lib kuopio_wall_sites -- --ignored --nocapture"]
fn kuopio_wall_sites() {
    use crate::simulation::terrain::graded::tests::{dump_site, wall_sites};
    let mut core = test_core();
    core.load_game_internal(KUOPIO_REFERENCE_SAVE).unwrap();
    core.precompute_road_mesh_data();
    let (terrain, roadbed) = (&core.heightmap, &core.transit_network.roadbed);
    let sites = wall_sites(terrain, roadbed, 5);
    eprintln!("GRADED_WALLS {}", serde_json::json!({"fixture": "kuopio", "walls": sites}));
    if let Some(dir) = std::env::var_os("METRUM_CONFLICT_DUMP_DIR") {
        for (rank, site) in sites["sites"].as_array().unwrap().iter().enumerate() {
            let (x, z) = (site["x"].as_f64().unwrap(), site["z"].as_f64().unwrap());
            let path = std::path::Path::new(&dir).join(format!("site-{}.json", rank + 1));
            dump_site(terrain, roadbed, x, z, &path);
        }
    }
}

// `TERRAIN-07-C` (`T2` (c)): the conflict wall of the reference save (round 4's one site, where
// two roads leave node 50 at the 1:3 limit and the lower one is walled up to 0.37 m) closes the
// step at every walled seam point, sampled every 5 cm.
#[test]
fn kuopio_conflict_wall_closes_the_step() {
    use crate::simulation::terrain::graded::{Level, TileKey, bake_tiles};
    let mut core = test_core();
    core.load_game_internal(KUOPIO_REFERENCE_SAVE).unwrap();
    core.precompute_road_mesh_data();
    let site = [-16.3, 1182.8];
    let (first, last) = (
        TileKey::at(site[0] - 40.0, site[1] - 40.0),
        TileKey::at(site[0] + 40.0, site[1] + 40.0),
    );
    let tiles: Vec<TileKey> = (first.x..=last.x)
        .flat_map(|x| (first.z..=last.z).map(move |z| TileKey { x, z }))
        .collect();
    let mut payloads = Vec::new();
    bake_tiles(
        &core.heightmap,
        &core.transit_network.roadbed,
        &core.allocator,
        &tiles,
        Level::Gl0,
        &mut payloads,
    )
    .unwrap();
    use crate::simulation::terrain::graded::site_tests::assert_walls_close_the_step;
    let (walled, highest) = assert_walls_close_the_step(
        &core.heightmap,
        &core.transit_network.roadbed,
        &core.allocator,
        &payloads,
    );
    assert!(
        walled > 100 && (highest - 0.37).abs() < 0.01,
        "{walled} walled samples, highest {highest}"
    );
}

// The regenerated reference's refined terrain builds clean. The refined patches the roads reach:
// 27 on the legacy save as recorded before `ROAD-68-C`, 28 on the regenerated one.
#[test]
#[ignore = "TERRAIN-07: patch count and refined CDT patches of the legacy terrain pipeline"]
fn kuopio_saved_reference_terrain_builds_clean() {
    let mut core = test_core();
    core.load_game_internal(KUOPIO_REFERENCE_SAVE).unwrap();
    core.precompute_road_mesh_data();
    let patches = SimCore::build_refined_terrain_patch_cache_entries(inputs(&mut core));
    assert_eq!(patches.len(), 28);
    for patch in patches {
        assert!(
            crate::nodes::simulation_node::SimulationNode::cached_refined_cdt_failure_label(&patch)
                .is_none(),
            "saved terrain {:?}: {:?}",
            patch.key,
            patch
                .mesh_buffers
                .as_ref()
                .map(|b| b.omitted_pathological_terrain_faces)
        );
    }
}

fn kuopio_manifest() -> serde_json::Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../benchmarks/fixtures/kuopio-terrain/placements.json",
    )))
    .unwrap()
}

// A core on the recorded session's source world, before any road.
fn kuopio_source_core() -> SimCore {
    let mut core = test_core();
    core.load_world_definition_internal(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../godot/bootstrap/worlds/kuopio_324km2_10m.sqlite"
        ),
        crate::simulation::vegetation::VegetationConfig::default(),
    )
    .unwrap();
    core
}

// A stroke's points at XZ `ends`, each at the height the road tool picks there: a road's surface
// where the end lies on one, else the ground. Roads in a cut or on a fill lie off the terrain,
// and an end at terrain height beside them would read as a bridge or tunnel.
fn road_tool_points(core: &mut SimCore, ends: [[f32; 2]; 2]) -> Vec<Vector3> {
    core.precompute_road_mesh_data();
    ends.map(|[x, z]| {
        Vector3::new(
            x,
            core.get_world_surface_height_internal(Vector2::new(x, z)),
            z,
        )
    })
    .to_vec()
}

// One recorded stroke's points: its recovered XZ ends at the road tool's heights.
fn recorded_stroke_points(core: &mut SimCore, stroke: &serde_json::Value) -> Vec<Vector3> {
    let ends =
        ["start_xz", "end_xz"].map(|key| [0, 1].map(|i| stroke[key][i].as_f64().unwrap() as f32));
    road_tool_points(core, ends)
}

// The Kuopio edits `TERRAIN-07-A` makes on the reference save, by place rather than edge id, so a
// re-recorded or replaced save keeps them (`ROAD-68` open question 38): each road's midpoint on
// the network recorded 2026-09-08 (its edges 43 and 69; the re-recorded save of `ROAD-68-K14`
// keeps every edge's id and midpoint).
// The dead end beside a 1:2 side slope, removed.
const KUOPIO_DEAD_END: [f32; 2] = [116.03, 1091.28];
// The road a T climbs a 1:4 hillside from, and the T's ends: its middle vertex, then 60 m up.
const KUOPIO_HILLSIDE_T: ([f32; 2], [[f32; 2]; 2]) = (
    [-1366.95, 2476.65],
    [[-1367.94, 2492.99], [-1427.83, 2489.35]],
);

// The live road whose midpoint lies nearest `midpoint`, asserted within 1 m. Test-only linear
// scan.
fn kuopio_edge_at(core: &SimCore, [x, z]: [f32; 2]) -> usize {
    let graph = &core.region_graph;
    let distance = |id: usize| {
        BuildingAllocator::sample_pos_on_edge(graph, id, 0.5).distance_to(Vector2::new(x, z))
    };
    let edge = (0..graph.edge_count())
        .filter(|&id| {
            let edge = graph.edge(id);
            !edge.deleted && edge.primary_type == TransitType::Road
        })
        .min_by(|&a, &b| distance(a).total_cmp(&distance(b)))
        .unwrap();
    assert!(
        distance(edge) < 1.0,
        "no road at {x}, {z}: edge {edge} is {} m off",
        distance(edge)
    );
    edge
}

// Commits one recorded stroke at the heights `recorded_stroke_points` picks.
fn commit_recorded_stroke(core: &mut SimCore, stroke: &serde_json::Value) {
    let points = recorded_stroke_points(core, stroke);
    commit_ready_with_lanes(
        core,
        points,
        stroke["fwd_lanes"].as_i64().unwrap() as i32,
        stroke["bkw_lanes"].as_i64().unwrap() as i32,
    );
}

// Replays one recorded Kuopio case on `core`, stroke by stroke through the commit at the heights
// `recorded_stroke_points` picks, and hands each committed stroke to `after_stroke` with the
// roadbed before it (an `O(1)` snapshot).
fn replay_kuopio_case(
    core: &mut SimCore,
    case: &serde_json::Value,
    mut after_stroke: impl FnMut(&serde_json::Value, &mut SimCore, &RoadbedSystem),
) {
    for stroke in case["segments"].as_array().unwrap() {
        let points = recorded_stroke_points(core, stroke);
        let before = core.transit_network.roadbed.snapshot();
        commit_ready_with_lanes(
            core,
            points,
            stroke["fwd_lanes"].as_i64().unwrap() as i32,
            stroke["bkw_lanes"].as_i64().unwrap() as i32,
        );
        after_stroke(stroke, core, &before);
    }
}

// Replays every recorded Kuopio case on the source world, stroke by stroke through the commit,
// and hands each committed stroke to `after_stroke`.
fn replay_kuopio_recorded_cases(
    mut after_stroke: impl FnMut(&str, &serde_json::Value, &mut SimCore),
) {
    let manifest = kuopio_manifest();
    for case in manifest["cases"].as_array().unwrap() {
        let case_id = case["case_id"].as_str().unwrap();
        let mut core = kuopio_source_core();
        replay_kuopio_case(&mut core, case, |stroke, core, _| {
            after_stroke(case_id, stroke, core)
        });
    }
}

// The recorded session in one world: every stroke the session accepted, once, in attempt order
// (the cases share their opening strokes), committed through the commit on the source world.
// The reference save is this world, saved (`ROAD-68` open question 38).
fn replay_kuopio_accepted_strokes_in_one_world(
    mut after_stroke: impl FnMut(&serde_json::Value, &mut SimCore),
) -> SimCore {
    let manifest = kuopio_manifest();
    let mut strokes: Vec<&serde_json::Value> = manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|case| case["segments"].as_array().unwrap())
        .filter(|stroke| stroke["original_accepted"].as_bool().unwrap())
        .collect();
    strokes.sort_by_key(|stroke| stroke["attempt"].as_i64().unwrap());
    strokes.dedup_by_key(|stroke| stroke["attempt"].as_i64().unwrap());
    let mut core = kuopio_source_core();
    for stroke in strokes {
        commit_recorded_stroke(&mut core, stroke);
        after_stroke(stroke, &mut core);
    }
    core
}

// Re-records the Kuopio reference save (`ROAD-68` open question 38): the recorded session's
// accepted strokes replayed in one world with today's code and saved with `save_game_settled`,
// at the camera and the header time the current save holds, so a re-record of the same city
// writes the same bytes (`ROAD-68-K16`). An interim reference until the user's new save replaces
// it; either save is then pinned by the steps roads.md lists under question 38.
#[test]
#[ignore = "manual fixture recorder: rewrites benchmarks/fixtures/kuopio-terrain/kuopio-terrain-map.sqlite"]
fn record_kuopio_reference_save() {
    let connection = rusqlite::Connection::open_with_flags(
        KUOPIO_REFERENCE_SAVE,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let saved_at_unix: i64 = connection
        .query_row("SELECT saved_at_unix FROM save_meta", [], |row| row.get(0))
        .unwrap();
    let camera = {
        connection
            .query_row(
                "SELECT pivot_x, pivot_y, pivot_z, yaw, pitch, distance, orthogonal FROM camera_state WHERE singleton = 1",
                [],
                |row| {
                    Ok(crate::simulation::save::SavedCameraState {
                        pivot: [row.get(0)?, row.get(1)?, row.get(2)?],
                        yaw: row.get(3)?,
                        pitch: row.get(4)?,
                        distance: row.get(5)?,
                        orthogonal: row.get(6)?,
                    })
                },
            )
            .ok()
    };
    drop(connection);
    let mut replayed = replay_kuopio_accepted_strokes_in_one_world(|_, _| {});
    // Saved once and loaded, so the load builds the lanes and their connections the replay's bulk
    // commits defer, then saved as the game saves.
    let path = temp_save_path("kuopio_reference");
    replayed
        .save_game_settled_at(path.to_str().unwrap(), None, saved_at_unix)
        .unwrap();
    let mut core = test_core();
    core.load_game_internal(path.to_str().unwrap()).unwrap();
    let _ = std::fs::remove_file(&path);
    core.precompute_road_mesh_data();
    core.save_game_settled_at(KUOPIO_REFERENCE_SAVE, camera, saved_at_unix)
        .unwrap();
}

// Every recorded stroke commits and leaves every node solved. `kuopio_05` attempt 14 ends on a
// road beyond the stroke's first ring of incidence: the solve re-derives the link from that
// road's T to attempt 13's T and must pin it at the far T from all of that node's arms, which
// the plan's excerpt lacked, so the far T failed `MouthOffPlane` (`ROAD-68` open question 8).
#[test]
fn kuopio_recorded_strokes_commit_with_every_node_solved() {
    let mut profile_count = 0;
    replay_kuopio_recorded_cases(|case_id, stroke, core| {
        let roadbed = &core.transit_network.roadbed;
        assert!(
            roadbed.failed_nodes().is_empty(),
            "{case_id} attempt {}: {:?}",
            stroke["attempt"],
            roadbed.failed_nodes()
        );
        profile_count += core
            .region_graph
            .edges()
            .iter()
            .filter(|edge| !edge.deleted)
            .count();
    });
    assert_eq!(
        profile_count, 158,
        "must exercise every recorded checkpoint"
    );
}

// The fixed quality gates of the rendered replay (`placements.json`'s `quality_limits`) on every
// live edge's physical profile: offset from the source terrain at most 5 m, grade at most 1.0 (by
// the road grade rule's one grade, `grade::steepest_grade`) and pitch change at most 30 degrees
// per knot. One line per edge over a limit.
fn profile_quality_failures(core: &SimCore) -> Vec<String> {
    let mut failures = Vec::new();
    for (edge_idx, edge) in core.region_graph.edges().iter().enumerate() {
        if edge.deleted {
            continue;
        }
        let mut offset = 0.0_f32;
        let grade = crate::simulation::network::grade::steepest_grade(&edge.physical_geometry)
            .map_or(0.0, |steepest| steepest.grade);
        let mut pitch_change = 0.0_f32;
        let mut last_pitch: Option<f32> = None;
        for point in &edge.physical_geometry {
            let source =
                core.heightmap.sample_height_world(point.x, point.z) * crate::config::HEIGHT_SCALE;
            offset = offset.max((point.y - source).abs());
        }
        for span in edge.physical_geometry.windows(2) {
            let run = (span[1].x - span[0].x).hypot(span[1].z - span[0].z);
            let dy = span[1].y - span[0].y;
            if run == 0.0 {
                assert_eq!(dy, 0.0, "edge {edge_idx}: vertical road span {span:?}");
                continue;
            }
            let pitch = dy.atan2(run).to_degrees();
            if let Some(previous) = last_pitch {
                pitch_change = pitch_change.max((pitch - previous).abs());
            }
            last_pitch = Some(pitch);
        }
        if offset > 5.0 || grade > 1.0 || pitch_change > 30.0 {
            failures.push(format!(
                "edge {edge_idx}: offset {offset}, grade {grade}, pitch change {pitch_change}"
            ));
        }
    }
    failures
}

// The quality gates across every live edge after each recorded edit. Beside steep junctions
// three profiles left the ground by 5.08-5.76 m while the plane was sampled 12 m out inside
// wider mouths (`ROAD-68` open question 31). Under the 1:3 maximum grade, the arms the stroke
// solve cuts to the source-delta limit on ground steeper than 33% left it by up to 0.36 m in
// their junction blends (`kuopio_04` attempt 10, `kuopio_06` attempts 17 and 18, `kuopio_08`
// attempt 22) until the blends kept the same band (open question 39).
#[test]
fn kuopio_recorded_profiles_preserve_quality_limits() {
    let mut failures = Vec::new();
    replay_kuopio_recorded_cases(|case_id, stroke, core| {
        for failure in profile_quality_failures(core) {
            failures.push(format!("{case_id} attempt {} {failure}", stroke["attempt"]));
        }
    });
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// The same gates on the reference save's own network, which no replay reads. The save before
// `ROAD-68-K14` held the recorded session's profiles, from the stroke solve before 2026-09-09
// (one 16% clamp for every span, pinned ends), which every load keeps as saved (open question
// 8): edges 13 and 14 leapt 27.1 m and 23.7 m within 5.9 m from their dead ends onto a four-way
// 29.4 m above the ground, the five tallest walls of the `TERRAIN-07-A` wall census (`ROAD-68`
// open question 38). The re-recorded save holds today's profiles, inside the source band beside
// its junctions too (question 39).
#[test]
fn kuopio_saved_reference_profiles_preserve_quality_limits() {
    let mut core = test_core();
    core.load_game_internal(KUOPIO_REFERENCE_SAVE).unwrap();
    let failures = profile_quality_failures(&core);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// The refined terrain after each recorded stroke: exported seams, tile seams, window bounds and
// the CDT's constraint edges.
#[test]
#[ignore = "TERRAIN-07: exported and tile seams, window bounds and CDT constraints of the legacy terrain pipeline"]
fn kuopio_recorded_terrain_builds_clean() {
    replay_kuopio_recorded_cases(|case_id, stroke, core| {
        let cold = SimCore::build_refined_terrain_patch_cache_entries(inputs(core));
        let mut tile_heights = std::collections::BTreeMap::new();
        for patch in &cold {
            if let Some(buffers) = &patch.mesh_buffers {
                let mut exported_heights = HashMap::new();
                for point in &buffers.terrain_vertices {
                    if let Some(previous) =
                        exported_heights.insert((point.x.to_bits(), point.z.to_bits()), point.y)
                    {
                        assert!(
                            (previous - point.y).abs() < 0.001,
                            "{case_id} {} patch {:?}: exported seam at {point:?}, previous height {previous}",
                            stroke["attempt"],
                            patch.key
                        );
                    }
                }
                assert_eq!(
                    buffers.omitted_pathological_terrain_faces, 0,
                    "{case_id} attempt {} patch {:?}: terrain export removed faces",
                    stroke["attempt"], patch.key
                );
            }
            for window in &patch.windows {
                let mesh = window.mesh_result.as_ref().unwrap();
                let bounds = window.cdt_patch;
                for vertex in &mesh.vertices {
                    assert!(
                        vertex.x >= bounds.min_x
                            && vertex.x <= bounds.max_x
                            && vertex.z >= bounds.min_z
                            && vertex.z <= bounds.max_z,
                        "{case_id} {}: vertex outside {:?}: {vertex:?}",
                        stroke["attempt"],
                        window.key
                    );
                    if vertex.x == bounds.min_x
                        || vertex.x == bounds.max_x
                        || vertex.z == bounds.min_z
                        || vertex.z == bounds.max_z
                    {
                        let key = (
                            (vertex.x * 1000.0).round() as i64,
                            (vertex.z * 1000.0).round() as i64,
                        );
                        if let Some((previous, previous_window)) =
                            tile_heights.insert(key, (vertex.height_m, window.key))
                        {
                            assert!(
                                (previous - vertex.height_m).abs() < 0.001,
                                "{case_id} {}: tile seam {key:?} height {previous} vs {} in {previous_window:?} and {:?}",
                                stroke["attempt"],
                                vertex.height_m,
                                window.key
                            );
                        }
                    }
                }
                assert_eq!(
                    mesh.stats.invalid_constraint_edges, 0,
                    "{case_id} attempt {} window {:?}: {:?}",
                    stroke["attempt"], window.key, mesh.invalid_constraint_samples
                );
                assert_eq!(
                    mesh.stats.spade_missing_road_constraint_edges, 0,
                    "{case_id} attempt {} window {:?}: {:?}",
                    stroke["attempt"], window.key, mesh.unpreserved_road_constraint_samples
                );
            }
        }
    });
}

#[test]
fn orthogonal_zoning_block_terrain_previews_and_commits() {
    orthogonal_zoning_block_at_angle(0.0);
}

#[test]
fn rotated_orthogonal_zoning_block_terrain_previews_and_commits() {
    for angle in [0.35, -0.55, 0.8, 2.2] {
        orthogonal_zoning_block_at_angle(angle);
    }
}

fn orthogonal_zoning_block_at_angle(angle: f64) {
    let u = glam::DVec2::new(angle.cos(), angle.sin());
    let v = u.perp();
    let rotate = |x, z| u * x + v * z;
    let mut core = test_core();
    core.benchmark_mode = false;
    core.config = WorldConfig::new(2048.0, 2048.0, 50.0, 10.0)
        .with_terrain_resolution(8.0)
        .with_chunking(512.0, 0.0);
    core.heightmap = TerrainSystem::from_world_config(&core.config);
    core.watermap = WaterSystem::from_world_config(&core.config);
    core.transit_network = TransitNetwork::new_for_world(&core.config);
    core.zoning = ZoningSystem::new(&core.config);
    core.pollution = PollutionSystem::new(&core.config);
    core.noise = NoiseSystem::new(&core.config);
    for segment in [
        [(-100.0, -200.0), (-100.0, 200.0)],
        [(-100.0, -150.0), (200.0, -150.0)],
        [(30.0, -150.0), (30.0, 150.0)],
        [(-100.0, 150.0), (30.0, 150.0)],
    ] {
        commit_ready(
            &mut core,
            segment
                .map(|(x, z)| {
                    let p = rotate(x, z);
                    Vector3::new(p.x as f32, 0.0, p.y as f32)
                })
                .to_vec(),
        );
    }
    let corner = rotate(30.0, -150.0);
    let junction = core
        .region_graph
        .find_node_within(Vector3::new(corner.x as f32, 0.0, corner.y as f32), 0.001)
        .unwrap();
    if angle == 0.0 {
        assert_eq!(core.region_graph.node(junction).pos.x, 30.0);
    }
    let path = temp_save_path("orthogonal_road_frames");
    core.save_game_internal(path.to_str().unwrap(), None)
        .unwrap();
    let before: Vec<_> = core
        .zoning
        .cells
        .saved_road_alignments()
        .into_iter()
        .filter(|(id, _)| !core.region_graph.edge(*id).deleted)
        .map(|(_, alignment)| serde_json::to_value(alignment).unwrap())
        .collect();
    let original = assert_orthogonal_block_cells(&mut core, angle);
    core.load_game_internal(path.to_str().unwrap()).unwrap();
    std::fs::remove_file(path).unwrap();
    let after: Vec<_> = core
        .zoning
        .cells
        .saved_road_alignments()
        .into_iter()
        .map(|(_, alignment)| serde_json::to_value(alignment).unwrap())
        .collect();
    assert_eq!(after, before, "road choices survive compacted edge ids");
    assert!(core.prepare_cell_chunk_internal((0, -1)));
    assert!(core.prepare_cell_chunk_internal((-1, 0)));
    assert_eq!(
        assert_orthogonal_block_cells(&mut core, angle),
        original,
        "cold reload and different viewport order retain exact geometry"
    );
}

fn assert_orthogonal_block_cells(
    core: &mut SimCore,
    angle: f64,
) -> Vec<(crate::simulation::zoning::cells::GridFrame, i32, i32)> {
    let u = glam::DVec2::new(angle.cos(), angle.sin());
    let v = u.perp();
    let preview = core.preview_cell_selection_internal(
        crate::simulation::zoning::cells::CellSelectionShape::Brush { radius_m: 600.0 },
        &[glam::DVec2::ZERO],
    );
    assert!(!preview.selection.cells.is_empty());
    let mut frame = None;
    let mut missing = Vec::new();
    let mut mismatched = Vec::new();
    for x in 0..12 {
        for y in 0..25 {
            let point = u * (-90.0 + x as f64 * 10.0) + v * (-120.0 + y as f64 * 10.0);
            let Some(cell) = core.zoning.cells.pick(point) else {
                missing.push((x, y));
                continue;
            };
            if *frame.get_or_insert(cell.grid) != cell.grid {
                mismatched.push((x, y, cell.grid));
            }
        }
    }
    assert!(
        missing.is_empty() && mismatched.is_empty(),
        "block angle {angle}: missing={missing:?}, different grids={mismatched:?}, frames={:?}",
        core.zoning.cells.saved_frames().collect::<Vec<_>>()
    );
    let mut cells: Vec<_> = preview
        .selection
        .cells
        .into_iter()
        .map(|key| (core.zoning.cells.frame(key.grid).unwrap(), key.x, key.y))
        .collect();
    cells.sort_unstable();
    cells
}

#[test]
fn branch_from_existing_t_compiles_four_way_junction_on_flat_and_slope() {
    // The sloped benchmark leg ending at (-70.6, -44.4) put a shared carriageway vertex on
    // an exact half-millimetre height; f32 rounding split its bands into a false conflict.
    for slope_end_m in [0.0, 6.4] {
        let mut core = test_core();
        core.create_blank_world_internal(512.0, 512.0, 8.0, 128.0, 0.0)
            .unwrap();
        core.slope_terrain_internal(
            Vector2::ZERO,
            256.0,
            Vector2::new(-128.0, 0.0),
            0.0,
            Vector2::new(128.0, 0.0),
            slope_end_m,
            1.0,
        );
        let surface = |core: &SimCore, x: f32, z: f32| {
            Vector3::new(
                x,
                core.get_world_surface_height_internal(Vector2::new(x, z)),
                z,
            )
        };
        for ((ax, az), (bx, bz)) in [((-110.0, 0.0), (110.0, 0.0)), ((-65.0, 0.0), (-65.0, 60.0))] {
            let points = vec![surface(&core, ax, az), surface(&core, bx, bz)];
            assert!(
                core.add_road_internal_with_snap(points, 1, 1, true)
                    .committed
            );
        }
        core.precompute_road_mesh_data();
        core.allocator
            .prepare_building_site_query_index(core.config.zone_cell_m);
        let snapshot = road_tool_snapshot_from_core(&core).unwrap();
        for step in 0..16 {
            let angle = std::f32::consts::TAU * step as f32 / 16.0;
            let (x, z) = (-60.0 + 15.0 * angle.cos(), -55.0 + 15.0 * angle.sin());
            let (_, validation) = snapshot.validate(&RoadPreviewRequest {
                surface_generation: snapshot.surface_generation,
                points: vec![surface(&core, -65.0, 0.0), surface(&core, x, z)],
                fwd_lanes: 1,
                bkw_lanes: 1,
                snap_to_existing_roads: true,
            });
            assert!(
                validation.is_valid,
                "slope {slope_end_m} leg end ({x}, {z}): {validation:?}"
            );
        }
    }
}

// A west border trunk and `background` roads in render patches 500 m and more east of it,
// built in bulk and
// finished with the world terrain refresh, as the populated-city fixture and the in-game
// benchmark city build their roads (`ROAD-68-D` part 6).
fn bulk_trunk_world(background: usize) -> SimCore {
    let mut core = SimCore::new(WorldConfig::new(1536.0, 1536.0, 40.0, 10.0));
    core.precompute_road_mesh_data();
    core.transit_network.bulk_load = true;
    let edge_x = core.heightmap.half_world_extents().0 - 1.0;
    let mut add = |(x0, z0): (f32, f32), (x1, z1): (f32, f32)| {
        let outcome = core.add_road_internal(
            vec![Vector3::new(x0, 0.0, z0), Vector3::new(x1, 0.0, z1)],
            1,
            1,
        );
        assert!(outcome.committed, "{}", core.last_road_timing);
    };
    add((-edge_x, 0.0), (-300.0, 0.0));
    for k in 0..background {
        let z = -600.0 + 150.0 * k as f32;
        add((300.0, z), (700.0, z));
    }
    let core_ref = &mut core;
    core_ref
        .transit_network
        .finalize_bulk_load(&mut core_ref.region_graph, &mut core_ref.allocator);
    core.fit_lanes_to_road_surface();
    core.refresh_all_engineered_terrain_patch_state(
        crate::nodes::sim::core::ROAD_LOCKED_TERRAIN_RENDER_STEP_M,
    );
    core
}

// The patches whose refined terrain one T-branch commit on the trunk built: every cache entry
// it added or replaced.
fn patches_built_by_trunk_branch(core: &mut SimCore) -> Vec<(usize, usize)> {
    let entry =
        |key: &crate::nodes::sim::core::RefinedTerrainPatchCacheKey| (key.patch_x, key.patch_z);
    let before: std::collections::BTreeMap<_, _> = core
        .refined_terrain_patch_cache
        .iter()
        .map(|(key, patch)| (entry(key), std::sync::Arc::as_ptr(patch)))
        .collect();
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    let outcome = core.commit_road(
        RoadCommitRequest {
            points: vec![
                Vector3::new(-500.0, 0.0, 0.0),
                Vector3::new(-500.0, 0.0, 120.0),
            ],
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            preview: None,
        },
        std::time::Instant::now(),
        Default::default(),
    );
    assert!(outcome.committed, "{}", outcome.rejection);
    let mut built: Vec<(usize, usize)> = core
        .refined_terrain_patch_cache
        .iter()
        .filter(|(key, patch)| before.get(&entry(key)) != Some(&std::sync::Arc::as_ptr(patch)))
        .map(|(key, _)| entry(key))
        .collect();
    built.sort_unstable();
    built
}

// The first commit after a world-scale road build validates only the terrain its own edit
// changed: roads built elsewhere before it (bulk load, load) are the world refresh's, not the
// commit's, so the patches it builds do not grow with the city (`ROAD-68-D` part 6). Before,
// the roadbed kept every compile's changed piece bounds until a road edit took them, and the
// first commit at 100k residents graded and built every road patch of the city.
#[test]
fn the_first_commit_builds_only_its_own_terrain_as_the_city_grows() {
    let alone = patches_built_by_trunk_branch(&mut bulk_trunk_world(0));
    let in_city = patches_built_by_trunk_branch(&mut bulk_trunk_world(5));
    assert!(!alone.is_empty(), "the T must build the terrain around it");
    assert_eq!(
        alone, in_city,
        "background roads must not add terrain to the commit"
    );
}
