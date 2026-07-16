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
    assert!(state.has_current_in_flight(key, 2));
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

    assert!(!state.has_pending_or_ready(key));
    state.drop_missing_ready_payload(key);

    assert!(!state.ready_lookup.contains(&key));
    assert!(state.ready.is_empty());

    let request_id = state.request(key);
    assert_eq!(request_id, 1);
    assert!(state.has_pending_or_ready(key));
}

#[test]
fn water_patch_payload_async_failure_releases_matching_request_for_retry() {
    let mut state = WaterPatchPayloadAsyncState::new();
    let key = WaterPatchPayloadKey {
        patch_x: 4,
        patch_z: 7,
    };
    let failed_request_id = state.request(key);

    let retryable = state.ingest_failed(vec![(key, failed_request_id)]);

    assert_eq!(retryable, vec![key]);
    assert!(!state.has_pending_or_ready(key));
    let retry_request_id = state.request(key);
    assert_ne!(retry_request_id, failed_request_id);
    assert_eq!(state.requested.get(&key), Some(&retry_request_id));
    assert_eq!(state.in_flight.get(&key), Some(&retry_request_id));
}

#[test]
fn water_patch_payload_async_stale_failure_keeps_newer_request() {
    let mut state = WaterPatchPayloadAsyncState::new();
    let key = WaterPatchPayloadKey {
        patch_x: 4,
        patch_z: 7,
    };
    let stale_request_id = state.request(key);
    state.remove_key(key);
    let current_request_id = state.request(key);

    let retryable = state.ingest_failed(vec![(key, stale_request_id)]);

    assert!(retryable.is_empty());
    assert_eq!(state.requested.get(&key), Some(&current_request_id));
    assert_eq!(state.in_flight.get(&key), Some(&current_request_id));
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
