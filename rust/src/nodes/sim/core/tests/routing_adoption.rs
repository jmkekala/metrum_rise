// SPDX-License-Identifier: GPL-2.0-only

//! `ROAD-68-G` (D7): a road commit rebuilds the routing hierarchy as a pure job and adopts it a
//! fixed number of frames later. Held frames run nothing, so neither the job's speed nor a
//! save, a paused clock or a second commit inside the hold changes the game.

use super::*;
use crate::nodes::sim::core::frame::FRAME_DT_S;
use crate::nodes::sim::core::road_commit::RoadCommitRequest;
use crate::simulation::network::cch_adoption::CCH_ADOPTION_FRAMES;
use crate::simulation::pathing::cch::CchGraph;
use godot::prelude::Vector2;
use std::time::Duration;

fn world() -> SimCore {
    let mut core = test_core();
    core.create_blank_world_internal(512.0, 512.0, 8.0, 128.0, 0.0)
        .expect("blank world");
    core.treasury.balance = 1.0e9;
    core.time.speed_multiplier = 1.0;
    core
}

fn commit(core: &mut SimCore, from: (f32, f32), to: (f32, f32)) {
    let surface = |core: &SimCore, (x, z): (f32, f32)| {
        Vector3::new(
            x,
            core.get_world_surface_height_internal(Vector2::new(x, z)),
            z,
        )
    };
    let points = vec![surface(core, from), surface(core, to)];
    let outcome = core.commit_road(
        RoadCommitRequest {
            points,
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
        },
        std::time::Instant::now(),
        Default::default(),
    );
    assert!(outcome.committed, "{}", outcome.rejection_reason);
}

// Steps the hold's frames, checking each holds the clock, then the adoption frame.
fn step_through_hold(core: &mut SimCore) -> crate::nodes::sim::core::frame::FrameStepReport {
    let clock = (
        core.time.day_index,
        core.time.minute_of_day,
        core.time.absolute_slot(),
    );
    for _ in 0..CCH_ADOPTION_FRAMES {
        assert!(core.step_frame(FRAME_DT_S).routing_held);
        let now = (
            core.time.day_index,
            core.time.minute_of_day,
            core.time.absolute_slot(),
        );
        assert_eq!(now, clock, "a held frame advances nothing");
    }
    let adoption = core.step_frame(FRAME_DT_S);
    assert!(!adoption.routing_held);
    assert!(!core.transit_network.cch_adoption_pending());
    adoption
}

// The installed hierarchy is the live graph's: same size as a fresh build, same costs.
fn assert_current(core: &SimCore) {
    let graph = &core.region_graph;
    let fresh = CchGraph::build(graph);
    let cch = &core.transit_network.cch_graph;
    assert_eq!(cch.size(), fresh.size());
    assert!(core.transit_network.cch_current());
}

#[test]
fn a_commit_adopts_its_routing_after_the_hold() {
    let mut core = world();
    commit(&mut core, (-150.0, 0.0), (150.0, 0.0));
    let before = core.transit_network.cch_graph.build_generation;
    assert!(core.transit_network.cch_adoption_pending());
    assert!(!core.transit_network.cch_current());
    step_through_hold(&mut core);
    assert_ne!(core.transit_network.cch_graph.build_generation, before);
    assert_current(&core);
}

#[test]
fn a_late_rebuild_blocks_the_adoption_frame_only() {
    let mut core = world();
    core.transit_network
        .delay_cch_jobs_for_test(Duration::from_millis(150));
    commit(&mut core, (-150.0, 0.0), (150.0, 0.0));
    let adoption = step_through_hold(&mut core);
    assert!(
        adoption.adoption_ms > 50.0,
        "the adoption frame waited {} ms",
        adoption.adoption_ms
    );
    assert_current(&core);
}

// The rebuild needs every shared-pool worker (a broadcast) once the adoption frame has begun.
// An adoption frame that waited for it on a pool worker would hold one of them, so the
// rebuild and the frame would wait for each other: the hold's caller waits outside the pool.
#[test]
fn the_adoption_wait_leaves_every_pool_worker_to_the_rebuild() {
    let mut core = world();
    core.transit_network
        .prelude_cch_jobs_for_test(std::sync::Arc::new(|| {
            std::thread::sleep(Duration::from_millis(100));
            rayon::broadcast(|_| ());
        }));
    commit(&mut core, (-150.0, 0.0), (150.0, 0.0));
    let (done, adopted) = std::sync::mpsc::channel();
    let stepping = std::thread::spawn(move || {
        step_through_hold(&mut core);
        assert_current(&core);
        let _ = done.send(());
    });
    // Other tests share the pool, so the broadcast may wait for their work; a deadlock never
    // ends.
    adopted
        .recv_timeout(Duration::from_secs(120))
        .expect("the adoption frame and its rebuild waited for each other");
    stepping.join().unwrap();
}

#[test]
fn a_paused_game_counts_its_frames_and_adopts_on_time() {
    let mut core = world();
    core.time.speed_multiplier = 0.0;
    commit(&mut core, (-150.0, 0.0), (150.0, 0.0));
    step_through_hold(&mut core);
    assert_current(&core);
}

#[test]
fn a_second_commit_supersedes_the_pending_rebuild_and_restarts_the_hold() {
    let mut core = world();
    commit(&mut core, (-150.0, 0.0), (150.0, 0.0));
    core.step_frame(FRAME_DT_S);
    commit(&mut core, (0.0, -150.0), (0.0, 150.0));
    step_through_hold(&mut core);
    assert_current(&core);
}

#[test]
fn a_save_inside_the_hold_installs_the_rebuild_and_ends_the_hold() {
    let mut core = world();
    commit(&mut core, (-150.0, 0.0), (150.0, 0.0));
    let path = std::env::temp_dir().join(format!(
        "metrum_road68g_hold_save_{}.sqlite",
        std::process::id()
    ));
    core.save_game_settled(path.to_str().unwrap(), None)
        .unwrap();
    let _ = std::fs::remove_file(&path);
    assert!(!core.transit_network.cch_adoption_pending());
    assert_current(&core);
    let clock = core.time.absolute_slot();
    let frame = core.step_frame(FRAME_DT_S);
    assert!(!frame.routing_held);
    assert!(core.time.absolute_slot() >= clock);
}
