// SPDX-License-Identifier: GPL-2.0-only

//! The road preview poll's busy-lock contract (`ROAD-54`): a poll that finds the simulation lock
//! held returns the live checks published at the end of the last frame, and `None` only while the
//! result is pending or stale.

use super::*;
use crate::nodes::sim::core::{PublishedRoadPreviewChecks, RoadPreviewSnapshot};
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
