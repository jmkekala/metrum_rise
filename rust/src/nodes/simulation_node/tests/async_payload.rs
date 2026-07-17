//! Async Payload regression tests for the simulation-node bridge.

use super::*;

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
fn refined_terrain_patch_cache_rejects_stale_contract_or_generation() {
    let mut cached = test_cached_refined_terrain_patch(TERRAIN_CDT_CONTRACT_REVISION, 7);

    assert!(SimulationNode::refined_terrain_patch_cache_entry_is_current(&cached, 7));

    cached.contract_revision = TERRAIN_CDT_CONTRACT_REVISION - 1;

    assert!(!SimulationNode::refined_terrain_patch_cache_entry_is_current(&cached, 7));

    cached.contract_revision = TERRAIN_CDT_CONTRACT_REVISION;
    cached.surface_generation = 6;

    assert!(!SimulationNode::refined_terrain_patch_cache_entry_is_current(&cached, 7));
}

#[test]
fn refined_terrain_patch_cache_insert_drops_stale_generation() {
    let mut core = test_core_with_flat_terrain(0.0);
    core.road_tool_surface_generation = 7;
    core.terrain_payload_generation_counter = 7;
    core.terrain_payload_global_generation = 7;
    let key = RefinedTerrainPatchCacheKey {
        patch_x: 0,
        patch_z: 0,
        render_step_mm: 2000,
    };

    core.insert_refined_terrain_patch_cache_entries(vec![test_cached_refined_terrain_patch(
        TERRAIN_CDT_CONTRACT_REVISION,
        6,
    )]);

    assert!(!core.refined_terrain_patch_cache.contains_key(&key));

    core.insert_refined_terrain_patch_cache_entries(vec![test_cached_refined_terrain_patch(
        TERRAIN_CDT_CONTRACT_REVISION,
        7,
    )]);

    assert!(core.refined_terrain_patch_cache.contains_key(&key));
}

#[test]
fn terrain_payload_patch_generation_does_not_invalidate_unrelated_patch() {
    let mut core = test_core_with_flat_terrain(0.0);
    let unchanged = core.terrain_payload_generation_for_patch(9, 9);

    core.bump_terrain_payload_patch_generations(&[(1, 2), (1, 3)]);

    let changed = core.terrain_payload_generation_for_patch(1, 2);
    assert!(changed > unchanged);
    assert_eq!(core.terrain_payload_generation_for_patch(1, 3), changed);
    assert_eq!(core.terrain_payload_generation_for_patch(9, 9), unchanged);
}

#[test]
fn building_site_dirty_patch_does_not_persist_as_road_owned() {
    let mut core = test_core_with_flat_terrain(0.0);

    core.mark_building_site_terrain_dirty_bounds((5.0, 5.0, 15.0, 15.0));

    assert!(core.road_locked_terrain_patch_keys.is_empty());
    assert!(core.road_locked_terrain_patch_margins.is_empty());
    assert!(!core.terrain_payload_patch_generations.is_empty());
}

#[test]
fn raw_terrain_payload_is_rejected_for_road_locked_patch() {
    let mut core = test_core_with_flat_terrain(0.0);
    core.road_locked_terrain_patch_margins.insert((0, 0), 8.0);
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

    assert!(
        SimulationNode::terrain_patch_payload_build_source_for_request(&mut core, request)
            .is_none(),
        "road-owned patches must never publish raw heightmap payloads"
    );
}

#[test]
fn refined_async_payloads_follow_request_keys_after_cache_sort() {
    let request_high_key = TerrainPatchPayloadKey {
        patch_x: 9,
        patch_z: 0,
        render_step_mm: 2000,
    };
    let request_low_key = TerrainPatchPayloadKey {
        patch_x: 1,
        patch_z: 0,
        render_step_mm: 2000,
    };
    let requests = vec![
        TerrainPatchPayloadRequest {
            key: request_high_key,
            request_id: 11,
            surface_generation: 7,
        },
        TerrainPatchPayloadRequest {
            key: request_low_key,
            request_id: 12,
            surface_generation: 7,
        },
    ];
    let mut entry_low = test_cached_refined_terrain_patch(TERRAIN_CDT_CONTRACT_REVISION, 7);
    entry_low.key.patch_x = 1;
    let mut entry_high = test_cached_refined_terrain_patch(TERRAIN_CDT_CONTRACT_REVISION, 7);
    entry_high.key.patch_x = 9;
    let entries = vec![entry_low, entry_high];
    let mut built = Vec::new();

    SimulationNode::append_refined_terrain_patch_payloads_for_requests(
        &mut built, &requests, &entries,
    );

    assert_eq!(built.len(), 2);
    assert_eq!(built[0].key, request_high_key);
    assert_refined_payload_cache_key(&built[0], 9);
    assert_eq!(built[1].key, request_low_key);
    assert_refined_payload_cache_key(&built[1], 1);
}
