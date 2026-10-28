// SPDX-License-Identifier: GPL-2.0-only

//! Async Payload regression tests for the simulation-node bridge.

use super::*;

#[test]
fn payload_workers_release_rayon_when_simulation_core_is_busy() {
    use super::super::async_terrain::WaterPatchPayloadRequest;
    use std::sync::mpsc;
    use std::time::Duration;

    let core = Arc::new(Mutex::new(test_core_with_flat_terrain(0.0)));
    let guard = core.lock().unwrap();
    let terrain_request = TerrainPatchPayloadRequest {
        key: TerrainPatchPayloadKey {
            patch_x: 0,
            patch_z: 0,
            render_step_mm: 0,
        },
        request_id: 1,
        surface_generation: guard.terrain_payload_generation_for_patch(0, 0),
    };
    let water_request = WaterPatchPayloadRequest {
        key: WaterPatchPayloadKey {
            patch_x: 0,
            patch_z: 0,
        },
        request_id: 2,
        source_generation: guard.watermap.render_generation(),
        surface_generation: guard.cached_road_mesh_generation,
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let (tx, rx) = mpsc::channel();
    let worker_core = Arc::clone(&core);
    pool.spawn(move || {
        let (terrain, terrain_retry, _) = SimulationNode::try_prepare_terrain_patch_payload_sources(
            &worker_core,
            vec![terrain_request],
        );
        let (water, water_retry) =
            SimulationNode::try_prepare_water_patch_payloads(&worker_core, vec![water_request]);
        tx.send((terrain.len(), terrain_retry, water.len(), water_retry))
            .unwrap();
    });
    // Release the guard before asserting, so a regression cannot leave the test worker blocked.
    let result = rx.recv_timeout(Duration::from_secs(2));
    drop(guard);
    let (terrain_count, terrain_retry, water_count, water_retry) =
        result.expect("payload snapshots must yield the worker while SimCore is held");
    assert_eq!((terrain_count, water_count), (0, 0));
    assert_eq!(terrain_retry.len(), 1);
    assert_eq!(terrain_retry[0].request_id, terrain_request.request_id);
    assert_eq!(water_retry.len(), 1);
    assert_eq!(water_retry[0].request_id, water_request.request_id);
    pool.install(|| {
        let (terrain, failed, refused) =
            SimulationNode::try_prepare_terrain_patch_payload_sources(&core, terrain_retry);
        assert!(failed.is_empty() && refused.is_empty());
        assert_eq!(terrain.len(), 1);
        let (water, failed) = SimulationNode::try_prepare_water_patch_payloads(&core, water_retry);
        assert!(failed.is_empty());
        assert_eq!(water.len(), 1);
    });
}

#[test]
fn terrain_patch_payload_async_clear_drops_stale_world_payloads() {
    let mut state = TerrainPatchPayloadAsyncState::new();
    let key = TerrainPatchPayloadKey {
        patch_x: 0,
        patch_z: 0,
        render_step_mm: 0,
    };
    let request_id = state.request(key, 1);
    let payload = TerrainPatchPayload {
        key,
        request_id,
        surface_generation: 1,
        data: TerrainPatchPayloadData::Regular {
            patch: test_patch(),
            height_bytes: Vec::new(),
        },
        mask_bytes: Vec::new(),
    };
    state.completed.push(payload);
    let completed = std::mem::take(&mut state.completed);
    state.ingest_completed(completed);

    assert!(state.ready_lookup.contains(&key));
    assert!(state.payloads.contains_key(&key));

    state.clear();

    assert!(state.requested.is_empty());
    assert!(state.in_flight.is_empty());
    assert!(state.ready.is_empty());
    assert!(state.ready_lookup.is_empty());
    assert!(state.payloads.is_empty());
    assert!(state.completed.is_empty());
}

#[test]
fn terrain_patch_payload_async_generation_change_requeues_key() {
    let mut state = TerrainPatchPayloadAsyncState::new();
    let key = TerrainPatchPayloadKey {
        patch_x: 0,
        patch_z: 0,
        render_step_mm: 2000,
    };
    let old_request_id = state.request(key, 1);
    let old_payload = TerrainPatchPayload {
        key,
        request_id: old_request_id,
        surface_generation: 1,
        data: TerrainPatchPayloadData::Regular {
            patch: test_patch(),
            height_bytes: Vec::new(),
        },
        mask_bytes: Vec::new(),
    };
    state.ingest_completed(vec![old_payload]);
    assert!(state.has_current_request(key, 1));
    assert!(state.has_current_ready(key, 1));

    state.drop_stale_key_for_generation(key, 2);
    assert!(!state.has_current_request(key, 1));
    assert!(!state.has_current_ready(key, 1));

    let new_request_id = state.request(key, 2);
    assert_ne!(old_request_id, new_request_id);
    assert!(state.has_current_request(key, 2));
    assert!(state.has_in_flight(key));
}

#[test]
fn terrain_patch_payload_async_generation_change_coalesces_physical_worker() {
    let mut state = TerrainPatchPayloadAsyncState::new();
    let key = TerrainPatchPayloadKey {
        patch_x: 2,
        patch_z: 3,
        render_step_mm: 2000,
    };
    let stale_request_id = state.request(key, 1);

    state.drop_stale_key_for_generation(key, 2);

    assert!(!state.has_current_request(key, 2));
    assert!(state.has_in_flight(key));

    state.ingest_failed(vec![TerrainPatchPayloadRequest {
        key,
        request_id: stale_request_id,
        surface_generation: 1,
    }]);

    assert!(!state.has_in_flight(key));
    let current_request_id = state.request(key, 2);
    assert_ne!(current_request_id, stale_request_id);
    assert!(state.has_current_request(key, 2));
    assert!(state.has_in_flight(key));
}

#[test]
fn terrain_patch_payload_async_stale_completion_never_becomes_ready() {
    let mut state = TerrainPatchPayloadAsyncState::new();
    let key = TerrainPatchPayloadKey {
        patch_x: 2,
        patch_z: 3,
        render_step_mm: 2000,
    };
    let stale_request_id = state.request(key, 1);
    state.drop_stale_key_for_generation(key, 2);

    state.ingest_completed(vec![TerrainPatchPayload {
        key,
        request_id: stale_request_id,
        surface_generation: 1,
        data: TerrainPatchPayloadData::Regular {
            patch: test_patch(),
            height_bytes: Vec::new(),
        },
        mask_bytes: Vec::new(),
    }]);

    assert!(!state.has_in_flight(key));
    assert!(!state.has_current_ready(key, 1));
    assert!(!state.ready_lookup.contains(&key));
}

#[test]
fn terrain_patch_payload_async_failure_releases_matching_request() {
    let mut state = TerrainPatchPayloadAsyncState::new();
    let key = TerrainPatchPayloadKey {
        patch_x: 3,
        patch_z: 4,
        render_step_mm: 2000,
    };
    let request_id = state.request(key, 5);

    state.ingest_failed(vec![TerrainPatchPayloadRequest {
        key,
        request_id,
        surface_generation: 5,
    }]);

    assert!(!state.requested.contains_key(&key));
    assert!(!state.in_flight.contains_key(&key));
}

#[test]
fn water_patch_payload_async_state_drops_missing_ready_key_before_requeue() {
    let mut state = WaterPatchPayloadAsyncState::new();
    let key = WaterPatchPayloadKey {
        patch_x: 0,
        patch_z: 0,
    };
    state.ready_lookup.insert(key);
    state.ready.push(key);

    assert!(!state.has_current_pending_or_ready(key, 3, 9));
    state.drop_missing_ready_payload(key);

    assert!(!state.ready_lookup.contains(&key));
    assert!(state.ready.is_empty());

    let request_id = state.request(key, 3, 9);
    assert_eq!(request_id, 1);
    assert!(state.has_current_pending_or_ready(key, 3, 9));
}

#[test]
fn water_patch_payload_async_failure_releases_matching_request_for_retry() {
    let mut state = WaterPatchPayloadAsyncState::new();
    let key = WaterPatchPayloadKey {
        patch_x: 4,
        patch_z: 7,
    };
    let failed_request_id = state.request(key, 4, 12);

    let retryable = state.ingest_failed(vec![WaterPatchPayloadRequest {
        key,
        request_id: failed_request_id,
        source_generation: 4,
        surface_generation: 12,
    }]);

    assert_eq!(retryable, vec![key]);
    assert!(!state.has_current_pending_or_ready(key, 4, 12));
    let retry_request_id = state.request(key, 4, 12);
    assert_ne!(retry_request_id, failed_request_id);
    let retry_state = WaterPatchPayloadRequestState {
        request_id: retry_request_id,
        source_generation: 4,
        surface_generation: 12,
    };
    assert_eq!(state.requested.get(&key), Some(&retry_state));
    assert_eq!(state.in_flight.get(&key), Some(&retry_state));
}

#[test]
fn water_patch_payload_async_stale_failure_keeps_newer_request() {
    let mut state = WaterPatchPayloadAsyncState::new();
    let key = WaterPatchPayloadKey {
        patch_x: 4,
        patch_z: 7,
    };
    let stale_request_id = state.request(key, 2, 7);
    state.remove_key(key);
    let current_request_id = state.request(key, 3, 8);

    let retryable = state.ingest_failed(vec![WaterPatchPayloadRequest {
        key,
        request_id: stale_request_id,
        source_generation: 2,
        surface_generation: 7,
    }]);

    assert!(retryable.is_empty());
    let current_state = WaterPatchPayloadRequestState {
        request_id: current_request_id,
        source_generation: 3,
        surface_generation: 8,
    };
    assert_eq!(state.requested.get(&key), Some(&current_state));
    assert_eq!(state.in_flight.get(&key), Some(&current_state));
}

#[test]
fn water_queries_wait_for_matching_surface_publication() {
    let mut core = test_core_with_flat_terrain(0.0);
    let request = WaterPatchPayloadRequest {
        key: WaterPatchPayloadKey {
            patch_x: 0,
            patch_z: 0,
        },
        request_id: 1,
        source_generation: core.watermap.render_generation(),
        surface_generation: core.cached_road_mesh_generation,
    };
    assert!(
        SimulationNode::water_patch_payload_for_request(&core, request).is_some(),
        "the matching empty-road generation should produce a water payload"
    );
    assert!(
        SimulationNode::water_patch_mesh_key_for_request(&core, 0, 0, 1).is_some(),
        "the matching empty-road generation should produce a water mesh input"
    );

    // Any pending roadbed work leaves the published surface behind the graph.
    core.transit_network.roadbed.mark_node_dirty(0);

    assert!(
        SimulationNode::water_patch_payload_for_request(&core, request).is_none(),
        "water payload clipping must not combine the current graph with stale road surfaces"
    );
    assert!(
        SimulationNode::water_patch_mesh_key_for_request(&core, 0, 0, 1).is_none(),
        "water mesh clipping must wait for matching road surface publication"
    );
}

#[test]
fn blank_world_reset_publishes_empty_road_surface_for_water_payloads() {
    let mut core = test_core_with_flat_terrain(0.0);
    core.create_blank_world_internal(40.0, 40.0, 10.0, 512.0, 0.0)
        .expect("the blank-world reset should succeed");

    assert!(
        core.transit_network.roadbed.is_current(),
        "the replacement empty network must be compiled before renderer refresh"
    );
    assert_eq!(
        core.cached_road_mesh_generation, core.road_tool_surface_generation,
        "the replacement empty network must publish the current render generation"
    );

    let request = WaterPatchPayloadRequest {
        key: WaterPatchPayloadKey {
            patch_x: 0,
            patch_z: 0,
        },
        request_id: 1,
        source_generation: core.watermap.render_generation(),
        surface_generation: core.road_tool_surface_generation,
    };
    assert!(
        SimulationNode::water_patch_payload_for_request(&core, request).is_some(),
        "water payloads must remain available immediately after a blank-world reset"
    );
    assert!(
        SimulationNode::water_patch_mesh_key_for_request(&core, 0, 0, 1).is_some(),
        "water meshes must remain available immediately after a blank-world reset"
    );
}

#[test]
fn water_only_query_generation_change_keeps_published_surface_payloads_available() {
    let mut core = test_core_with_flat_terrain(0.0);
    let published_surface_generation = core.cached_road_mesh_generation;

    core.rebuild_authored_water_preview_internal()
        .expect("the authored-water rebuild should succeed");

    assert!(
        core.road_tool_surface_generation > published_surface_generation,
        "authored water must advance the road-tool query snapshot"
    );
    assert_eq!(
        core.cached_road_mesh_generation, published_surface_generation,
        "water-only changes must not relabel or rebuild unchanged road geometry"
    );

    let request = WaterPatchPayloadRequest {
        key: WaterPatchPayloadKey {
            patch_x: 0,
            patch_z: 0,
        },
        request_id: 1,
        source_generation: core.watermap.render_generation(),
        surface_generation: published_surface_generation,
    };
    assert!(
        SimulationNode::water_patch_payload_for_request(&core, request).is_some(),
        "water-only query changes must not invalidate matching road-clipped payloads"
    );
    assert!(
        SimulationNode::water_patch_mesh_key_for_request(&core, 0, 0, 1).is_some(),
        "water-only query changes must not invalidate matching road-clipped meshes"
    );
}

// T32 (a): a road across a flooded patch drops exactly the mesh cells its top triangles meet,
// the builder emits the rest, and the mesh key follows the patch's terrain payload generation.
#[test]
fn water_meshes_drop_the_cells_a_road_meets_keyed_by_the_patch_generation() {
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::roadbed::fixtures::{connect, point};
    use crate::simulation::network::types::NodeType;

    let mut core = test_core_with_flat_terrain(0.0);
    core.create_blank_world_internal(40.0, 40.0, 10.0, 512.0, 0.0)
        .expect("the blank-world reset should succeed");
    let (width, height) = (core.watermap.width, core.watermap.height);
    core.watermap
        .replace_baseline_depth_from_dense(&vec![1.0; width * height])
        .expect("the flood fills every sample");
    // A 7 m road along z = 0 across the 40 m world, centred on the origin: its top triangles lie
    // within the two middle rows of 10 m cells and reach into no other row.
    let mut graph = RegionGraph::new();
    let west = graph.add_node(point(-30.0, 0.0, false), NodeType::Junction);
    let east = graph.add_node(point(30.0, 0.0, false), NodeType::Junction);
    connect(&mut graph, west, east, 7.0, false);
    graph.rebuild_intersection_clips();
    core.region_graph = graph;
    core.transit_network.roadbed.clear();
    core.transit_network
        .roadbed
        .compile_dirty(&core.region_graph);
    let cell_m = core.config.zone_cell_m;
    core.allocator.prepare_building_site_query_index(cell_m);

    let (key, patch) = SimulationNode::water_patch_mesh_key_for_request(&core, 0, 0, 1)
        .expect("the flooded patch has a mesh key");
    assert_eq!(key.cover_generation, core.terrain_payload_generation_for_patch(0, 0));
    let input = SimulationNode::water_patch_mesh_build_input(&core, key, patch);
    let rows: Vec<Vec<bool>> = input.dropped.chunks(4).map(<[bool]>::to_vec).collect();
    assert_eq!(
        rows,
        [[false; 4], [true; 4], [true; 4], [false; 4]],
        "the road drops the two middle rows"
    );
    let [mesh] = SimCore::build_water_patch_mesh_cache_entries(vec![input])
        .try_into()
        .expect("one water mesh");
    assert_eq!(mesh.stats.road_clipped_cells, 8);
    assert_eq!(mesh.stats.emitted_triangles, 16);
    assert!(
        mesh.vertices
            .iter()
            .all(|vertex| vertex.z.abs() >= 10.0),
        "only the outer rows are emitted"
    );

    core.bump_terrain_payload_patch_generations(&[(0, 0)]);
    let (bumped, _) = SimulationNode::water_patch_mesh_key_for_request(&core, 0, 0, 1)
        .expect("the flooded patch has a mesh key");
    assert_ne!(bumped, key, "a new patch generation misses the cached mesh");
}

#[test]
fn plain_terrain_payload_is_served_for_every_patch() {
    // TERRAIN-07-D item 4: every patch renders its plain payload and the graded tiles draw the
    // road's terrain over it. Refusing it left the road's patches uninstalled forever.
    let mut core = test_core_with_flat_terrain(0.0);
    let surface_generation = core.terrain_payload_generation_for_patch(0, 0);
    let request = TerrainPatchPayloadRequest {
        key: TerrainPatchPayloadKey {
            patch_x: 0,
            patch_z: 0,
            render_step_mm: 0,
        },
        request_id: 1,
        surface_generation,
    };

    let payload = SimulationNode::terrain_patch_payload_for_request(&mut core, request)
        .expect("every patch serves its plain payload");
    assert!(matches!(payload.data, TerrainPatchPayloadData::Regular { .. }));
}

#[test]
fn a_request_outside_the_world_is_refused_not_retried() {
    // TERRAIN-07-D round 4: a refusal by contract is reported apart from the transient retries,
    // so Godot logs it and stops asking instead of retrying every frame.
    let mut core = test_core_with_flat_terrain(0.0);
    let request = TerrainPatchPayloadRequest {
        key: TerrainPatchPayloadKey {
            patch_x: 999,
            patch_z: 999,
            render_step_mm: 0,
        },
        request_id: 1,
        surface_generation: core.terrain_payload_generation_for_patch(999, 999),
    };
    assert!(
        SimulationNode::terrain_patch_payload_for_request(&mut core, request)
            .is_err_and(|deferral| deferral == PayloadDeferral::Refused("patch_outside_world")),
    );
}
