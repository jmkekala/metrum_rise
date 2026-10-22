// SPDX-License-Identifier: GPL-2.0-only

//! Road preview live-check regressions: the busy-lock publication contract (`ROAD-54`) and the
//! local, exact cell-reservation validation required by dense junction previews (`ROAD-66`).

use super::*;
use crate::nodes::sim::core::{PublishedRoadPreviewChecks, RoadPreviewSnapshot};
use crate::simulation::zoning::cells::{CellBounds, CellSelectionShape, GridFrame};
use glam::DVec2;
use std::sync::{Mutex, RwLock};

// A stroke 25 m beside the test border road: valid and clear until parcels are placed there.
fn compile_preview(core: &SimCore, request_id: u64) -> RoadPreviewSnapshot {
    let (context, _) = road_tool_snapshots_from_core(core).expect("the surface must publish");
    super::super::road_preview::compile_road_preview_from_context(
        &context,
        RoadPreviewRequest {
            enqueued_at: None,
            include_terrain: false,
            request_id,
            surface_generation: core.road_tool_surface_generation,
            points: vec![
                Vector3::new(20.0, 0.0, -25.0),
                Vector3::new(40.0, 0.0, -25.0),
            ],
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: false,
        },
    )
}

fn dense_line(start: Vector3, end: Vector3) -> Vec<Vector3> {
    let segments = (start.distance_to(end) / 2.0).ceil().max(1.0) as usize;
    (0..=segments)
        .map(|index| start.lerp(end, index as f32 / segments as f32))
        .collect()
}

fn add_dense_test_road(core: &mut SimCore, start_node: u32, end_node: u32, lanes_each_way: u8) {
    let start = core.region_graph.node(start_node).pos;
    let end = core.region_graph.node(end_node).pos;
    let geometry = dense_line(start, end);
    let length = start.distance_to(end);
    core.region_graph.add_edge(Edge {
        start_node,
        end_node,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: f32::from(lanes_each_way) * 2.0 * crate::config::LANE_WIDTH,
        fwd_lanes: lanes_each_way,
        bkw_lanes: lanes_each_way,
        speed_limit: 13.89,
        base_cost: length,
        physical_length: length,
        start_clip: 0.0,
        end_clip: 0.0,
        physical_geometry: geometry.clone(),
        geometry,
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access: VehicleFrontageAccess::BothSides,
    });
}

fn dense_four_way_preview_fixture() -> SimCore {
    let mut core = test_core();
    let positions = [
        Vector3::ZERO,
        Vector3::new(0.0, 0.0, 240.0),
        Vector3::new(-180.0, 0.0, -150.0),
        Vector3::new(-140.0, 0.0, 55.0),
        Vector3::new(105.0, 0.0, -65.0),
        Vector3::new(225.0, 0.0, -135.0),
        Vector3::new(100.0, 0.0, -225.0),
    ];
    let nodes: Vec<_> = positions
        .into_iter()
        .map(|position| core.region_graph.add_node(position, NodeType::Junction))
        .collect();
    // Node 0 is the four-way start. Node 4 is its neighboring three-way junction.
    for (start, end, lanes) in [
        (0, 1, 2),
        (0, 2, 2),
        (0, 3, 1),
        (0, 4, 1),
        (4, 5, 1),
        (4, 6, 1),
    ] {
        add_dense_test_road(&mut core, nodes[start], nodes[end], lanes);
    }
    core.transit_network
        .lane_system
        .rebuild(&mut core.region_graph);
    core.precompute_road_mesh_data();
    assert!(
        core.transit_network
            .road_surface
            .published_generation_matches_source()
    );
    core.zoning.generate_cells(
        &core.region_graph,
        CellBounds {
            min: DVec2::splat(-320.0),
            max: DVec2::splat(320.0),
        },
        |_| false,
    );
    let paint = core
        .zoning
        .cells
        .select(CellSelectionShape::Cell, &[DVec2::new(12.0, 120.0)]);
    assert!(!paint.cells.is_empty());
    core.zoning.paint_cells(&paint, 1, |_| false).unwrap();
    core
}

#[test]
fn code_17_four_way_preview_stream_keeps_live_validation_local_and_commit_exact() {
    use crate::simulation::zoning::cells::CellKey;
    use std::time::{Duration, Instant};

    let core = Mutex::new(dense_four_way_preview_fixture());
    let checks = PublishedRoadPreviewChecks::default();
    let result = RwLock::new(None);
    let sim = core.lock().unwrap();
    let (context, _) = road_tool_snapshots_from_core(&sim).unwrap();
    let generation = sim.road_tool_surface_generation;
    let center = Vector3::ZERO;
    let target = Vector3::new(150.0, 0.0, 115.0);
    let mut published_time = Duration::ZERO;
    let mut ready = None;

    for request_id in 1..=36 {
        let u = request_id as f32 / 36.0;
        let endpoint = center.lerp(target, u)
            + Vector3::new(0.0, 0.0, (u * std::f32::consts::TAU).sin() * 4.0);
        let points = vec![center, endpoint];
        let preview = super::super::road_preview::compile_road_preview_from_context(
            &context,
            RoadPreviewRequest {
                enqueued_at: None,
                include_terrain: false,
                request_id,
                surface_generation: generation,
                points: points.clone(),
                fwd_lanes: 1,
                bkw_lanes: 1,
                snap_to_existing_roads: true,
            },
        );
        *result.write().unwrap() = Some(preview);
        let publish_start = Instant::now();
        checks.publish_frame(&sim, &result);
        published_time += publish_start.elapsed();
        let published = {
            let result = result.read().unwrap();
            checks
                .poll(&core, result.as_ref().unwrap())
                .expect("the held-core poll must return this frame's checks")
        };
        let result = result.read().unwrap();
        let preview = result.as_ref().unwrap();
        let authoritative = preview.edit_plan().map(|plan| plan.road_status(&sim));
        assert_eq!(published.plan_state, authoritative);
        if authoritative == Some("ready") {
            ready = Some((preview.edit_plan().unwrap(), points));
        }
    }
    assert!(
        published_time < Duration::from_millis(500),
        "moving preview publication held the simulation lock too long"
    );
    let (plan, points) = ready.expect("the stream must include a committable fifth arm");

    let republish_start = Instant::now();
    for _ in 0..120 {
        checks.publish_frame(&sim, &result);
    }
    assert!(
        republish_start.elapsed() < Duration::from_millis(100),
        "unchanged frames must reuse the exact cell-reservation result"
    );
    drop(sim);

    let mut sim = core.into_inner().unwrap();
    let grid = sim.zoning.cells.saved_frames().count() as u64 + 1;
    assert!(sim.zoning.cells.restore_saved_frame(
        grid,
        GridFrame::new(DVec2::new(3.0, 7.0), DVec2::new(0.8, 0.6), 10.0).unwrap(),
    ));
    for index in 0..20_000 {
        assert!(sim.zoning.cells.restore_saved_cell(
            CellKey {
                grid,
                x: 2_000 + index % 200,
                y: 2_000 + index / 200,
            },
            1,
        ));
    }
    let locality_start = Instant::now();
    assert_eq!(plan.road_status(&sim), "ready");
    assert!(
        locality_start.elapsed() < Duration::from_millis(100),
        "remote reservations must not enter the affected-owner query"
    );

    let plan = plan.complete_for_commit(&sim);
    assert_eq!(plan.status(&sim), "ready");
    let edges_before = sim.region_graph.edge_count();
    let added = sim.add_road_internal_with_snap_and_validation(points, 1, 1, true, Some(&plan));
    assert!(added.committed);
    assert!(sim.region_graph.edge_count() > edges_before);
}

#[test]
fn a_preview_poll_during_a_held_lock_returns_the_checks_of_the_last_frame() {
    let mut core = test_core();
    add_test_border_road(&mut core);
    core.precompute_road_mesh_data();
    let result = RwLock::new(Some(compile_preview(&core, 7)));
    let core = Mutex::new(core);
    let checks = PublishedRoadPreviewChecks::default();
    let poll = |result: &RwLock<Option<RoadPreviewSnapshot>>| {
        checks.poll(&core, result.read().unwrap().as_ref().unwrap())
    };

    // The test thread holding the lock stands in for the simulation thread.
    let mut sim = core.lock().unwrap();
    assert_eq!(
        poll(&result),
        None,
        "a result nothing has evaluated is pending"
    );
    checks.publish_frame(&sim, &result);
    let clear = poll(&result).expect("a held lock returns the published checks");
    assert_eq!(clear.plan_state, Some("ready"));
    assert!(
        clear
            .clearance
            .is_some_and(|clearance| clearance.is_clear())
    );

    // Changes under the lock appear at the next frame end, like the render snapshot.
    place_test_parcel_run(&mut sim, ZoneType::Residential, 10.0, 50.0);
    assert_eq!(poll(&result), Some(clear));
    checks.publish_frame(&sim, &result);
    let blocked = poll(&result).unwrap();
    let clearance = blocked.clearance.unwrap();
    assert!(clearance.overlapping_parcels > 0 && clearance.first_overlapping_parcel.is_some());
    assert_eq!(clearance.zoning_revision, sim.zoning.overlay_revision());
    assert_eq!(
        blocked.plan_state, None,
        "a refused corridor reports no plan state"
    );
    drop(sim);
    assert_eq!(
        poll(&result),
        Some(blocked),
        "a free poll evaluates the same state"
    );

    // Checks of a superseded result never answer for a newer one. A poll that takes the lock
    // publishes its evaluation, so a refetch right after it needs no frame.
    let newer = compile_preview(&core.lock().unwrap(), 8);
    *result.write().unwrap() = Some(newer);
    let sim = core.lock().unwrap();
    assert_eq!(poll(&result), None, "the newer result is pending");
    drop(sim);
    let fresh = poll(&result).expect("a free poll evaluates");
    let mut sim = core.lock().unwrap();
    assert_eq!(poll(&result), Some(fresh));

    // A frame that meets the worker replacing the result publishes nothing.
    {
        let _replacing = result.write().unwrap();
        checks.publish_frame(&sim, &result);
    }
    assert_eq!(poll(&result), None);
    checks.publish_frame(&sim, &result);
    assert_eq!(poll(&result), Some(fresh));

    // A result for a superseded road surface is stale: frames stop evaluating it, and only a
    // poll that takes the lock still exports it.
    sim.road_tool_surface_generation += 1;
    checks.publish_frame(&sim, &result);
    assert_eq!(
        poll(&result),
        None,
        "a held lock returns null for a stale result"
    );
    drop(sim);
    assert!(poll(&result).is_some());
    let _sim = core.lock().unwrap();
    assert_eq!(
        poll(&result),
        None,
        "polling a stale result publishes nothing"
    );
}
