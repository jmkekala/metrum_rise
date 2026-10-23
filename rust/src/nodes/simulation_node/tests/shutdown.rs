// SPDX-License-Identifier: GPL-2.0-only

//! Freeing a node ends its threads and jobs before the free returns (`CODE-17`).

use super::*;
use std::sync::Condvar;

// Godot unloads the library right after it frees the node, so `stop_workers` must not return
// while the simulation thread or a payload job can still run or hold the core. The caller's reference is then the last one, and the caller frees the core.
#[test]
fn stopping_the_workers_leaves_the_last_core_reference_to_the_caller() {
    let core = test_core_with_flat_terrain(0.0);
    let query = Arc::new(RwLock::new(road_tool_snapshot_from_core(&core).unwrap()));
    let core = Arc::new(Mutex::new(core));

    // The sender stays alive: the thread ends because it is told to, not by disconnection of
    // the command channel.
    let (commands, command_rx) = std::sync::mpsc::channel();
    let sim_thread = {
        let core = Arc::clone(&core);
        std::thread::spawn(move || {
            run_sim_thread(
                core,
                Arc::new(RwLock::new(RenderSnapshot::default())),
                query,
                command_rx,
            );
        })
    };

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

    stop_workers(&commands, Some(sim_thread), &jobs);
    assert_eq!(Arc::strong_count(&core), 1);
}
