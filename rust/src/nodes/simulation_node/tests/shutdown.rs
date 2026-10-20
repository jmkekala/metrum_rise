// SPDX-License-Identifier: GPL-2.0-only

//! Freeing a node ends its threads and jobs before the free returns (`CODE-17`).

use super::*;
use std::sync::Condvar;

// Godot unloads the library right after it frees the node, so `stop_workers` must not return
// while the simulation thread, the preview worker or a payload job can still run or hold the
// core. The caller's reference is then the last one, and the caller frees the core.
#[test]
fn stopping_the_workers_leaves_the_last_core_reference_to_the_caller() {
    let core = test_core_with_flat_terrain(0.0);
    let (context, query) = road_tool_snapshots_from_core(&core).unwrap();
    let context = Arc::new(RwLock::new(context));
    let query = Arc::new(RwLock::new(query));
    let result = Arc::new(RwLock::new(None));
    let core = Arc::new(Mutex::new(core));

    // Both senders stay alive: the threads end because they are told to, not by disconnection
    // of the command channel.
    let (commands, command_rx) = std::sync::mpsc::channel();
    let sim_thread = {
        let (core, context, query, result) = (
            Arc::clone(&core),
            Arc::clone(&context),
            Arc::clone(&query),
            Arc::clone(&result),
        );
        std::thread::spawn(move || {
            run_sim_thread(
                core,
                Arc::new(RwLock::new(RenderSnapshot::default())),
                context,
                query,
                result,
                Arc::new(PublishedRoadPreviewChecks::default()),
                command_rx,
            );
        })
    };
    let (mut road_preview, road_preview_rx) = road_preview_channel();
    let road_preview_worker = {
        let (core, context, result) = (Arc::clone(&core), context, Arc::clone(&result));
        std::thread::spawn(move || {
            run_road_preview_worker(core, context, result, road_preview_rx);
        })
    };
    assert!(road_preview.submit(RoadPreviewRequest {
        enqueued_at: None,
        include_terrain: true,
        request_id: 1,
        surface_generation: 1,
        points: vec![Vector3::new(-20.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
        fwd_lanes: 1,
        bkw_lanes: 1,
        snap_to_existing_roads: false,
    }));

    // A payload job holding the core until the gate opens.
    let jobs = Arc::new(async_jobs::AsyncJobs::default());
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    {
        let (core, gate) = (Arc::clone(&core), Arc::clone(&gate));
        jobs.spawn(move || {
            let (open, wake) = &*gate;
            drop(
                wake.wait_while(open.lock().unwrap(), |open| !*open)
                    .unwrap(),
            );
            drop(core);
        });
    }
    assert!(Arc::strong_count(&core) > 1);
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();

    stop_workers(
        &commands,
        &mut road_preview,
        [Some(sim_thread), Some(road_preview_worker)],
        &jobs,
    );
    assert_eq!(Arc::strong_count(&core), 1);
    assert!(!road_preview.submit(RoadPreviewRequest {
        enqueued_at: None,
        include_terrain: false,
        request_id: 2,
        surface_generation: 1,
        points: Vec::new(),
        fwd_lanes: 1,
        bkw_lanes: 1,
        snap_to_existing_roads: false,
    }));
}
