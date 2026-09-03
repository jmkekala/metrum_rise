// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: async_payload.rs
//  script_path: rust/src/nodes/simulation_node/tests/async_payload.rs
//  module_name: async_payload
//  version: 0.1.0
//  description: Tests for the asynchronous terrain and water patch payload
//           crossing the simulation-node bridge.
//  kind: test
//  spec: none
//  internal_dependencies: [async_terrain]
//  external_dependencies: []
//  features: [async-payload, terrain-patch, stale-drop, water-patch]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-24
// ========================================================================

//! Async Payload regression tests for the simulation-node bridge.

use super::*;

// ========================================================================
// ASYNC PAYLOAD TESTS
// ========================================================================

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

    core.insert_refined_terrain_patch_cache_entries(vec![Arc::new(
        test_cached_refined_terrain_patch(TERRAIN_CDT_CONTRACT_REVISION, 6),
    )]);

    assert!(!core.refined_terrain_patch_cache.contains_key(&key));

    core.insert_refined_terrain_patch_cache_entries(vec![Arc::new(
        test_cached_refined_terrain_patch(TERRAIN_CDT_CONTRACT_REVISION, 7),
    )]);

    assert!(core.refined_terrain_patch_cache.contains_key(&key));
}

#[test]
fn stale_cache_insert_does_not_replace_reusable_baseline() {
    let mut core = test_core_with_flat_terrain(0.0);
    core.terrain_payload_generation_counter = 7;
    core.terrain_payload_global_generation = 7;
    let baseline = test_cached_refined_terrain_patch(TERRAIN_CDT_CONTRACT_REVISION, 5);
    let key = baseline.key;
    core.refined_terrain_patch_cache
        .insert(key, Arc::new(baseline));

    core.insert_refined_terrain_patch_cache_entries(vec![Arc::new(
        test_cached_refined_terrain_patch(TERRAIN_CDT_CONTRACT_REVISION, 6),
    )]);

    assert_eq!(
        core.refined_terrain_patch_cache
            .get(&key)
            .map(|entry| entry.surface_generation),
        Some(5),
        "stale worker completion must leave the last reusable baseline intact"
    );
}

#[test]
fn stale_compatible_refined_cache_is_build_input_but_never_ready() {
    let mut core = test_core_with_flat_terrain(0.0);
    core.terrain_payload_generation_counter = 7;
    core.terrain_payload_global_generation = 7;
    let cached = test_cached_refined_terrain_patch(TERRAIN_CDT_CONTRACT_REVISION, 6);
    let key = cached.key;
    core.refined_terrain_patch_cache
        .insert(key, Arc::new(cached));
    let request = TerrainPatchPayloadRequest {
        key: TerrainPatchPayloadKey {
            patch_x: key.patch_x,
            patch_z: key.patch_z,
            render_step_mm: key.render_step_mm,
        },
        request_id: 11,
        surface_generation: 7,
    };

    let source = SimulationNode::terrain_patch_payload_build_source_for_request(&mut core, request)
        .expect("stale compatible cache should remain a refined build source");

    assert!(!source.is_ready());
    assert_eq!(source.previous_surface_generation(), Some(6));
    assert!(
        core.refined_terrain_patch_cache.contains_key(&key),
        "capturing an immutable previous generation must not evict the reusable baseline"
    );
}

#[test]
fn compatible_cache_and_stamped_road_chunks_capture_local_tile_scope() {
    let mut core = test_core_with_flat_terrain(0.0);
    let cached = test_cached_refined_terrain_patch(TERRAIN_CDT_CONTRACT_REVISION, 1);
    let key = cached.key;
    core.refined_terrain_patch_cache
        .insert(key, Arc::new(cached));
    core.bump_local_road_terrain_payload_generations(&[(0, 0)], &[(0, 0)]);
    let surface_generation = core.terrain_payload_generation_for_patch(0, 0);
    let request = TerrainPatchPayloadRequest {
        key: TerrainPatchPayloadKey {
            patch_x: 0,
            patch_z: 0,
            render_step_mm: 2000,
        },
        request_id: 13,
        surface_generation,
    };

    let source = SimulationNode::terrain_patch_payload_build_source_for_request(&mut core, request)
        .expect("a compatible baseline and stamped road chunk should produce a local source");
    let tile_keys = source
        .local_tile_keys()
        .expect("local road invalidation should not fall back to a full-patch query");

    assert!(!tile_keys.is_empty());
    assert_eq!(source.previous_surface_generation(), Some(1));
}

#[test]
fn stamped_nonoverlapping_road_chunk_carries_previous_generation_without_full_query() {
    let mut core = test_core_with_flat_terrain(0.0);
    let cached = test_cached_refined_terrain_patch(TERRAIN_CDT_CONTRACT_REVISION, 1);
    let key = cached.key;
    core.refined_terrain_patch_cache
        .insert(key, Arc::new(cached));
    core.bump_local_road_terrain_payload_generations(&[(0, 0)], &[(100, 100)]);
    let request = TerrainPatchPayloadRequest {
        key: TerrainPatchPayloadKey {
            patch_x: 0,
            patch_z: 0,
            render_step_mm: 2000,
        },
        request_id: 14,
        surface_generation: core.terrain_payload_generation_for_patch(0, 0),
    };

    let source = SimulationNode::terrain_patch_payload_build_source_for_request(&mut core, request)
        .expect("a proven nonoverlapping road stamp should still produce a local build source");

    assert_eq!(
        source.local_tile_keys(),
        Some(&[][..]),
        "no-overlap invalidation must carry the previous windows without a full patch query"
    );
}

#[test]
fn refined_payload_source_waits_for_batched_terrain_stroke_finalization() {
    let mut core = test_core_with_flat_terrain(0.0);
    core.start_terrain_stroke_internal();
    core.sculpt_terrain_stroke_step_internal(Vector2::new(0.0, 0.0), 5.0, 0.5);
    let request_for_current_generation = |core: &SimCore, request_id| TerrainPatchPayloadRequest {
        key: TerrainPatchPayloadKey {
            patch_x: 0,
            patch_z: 0,
            render_step_mm: 2000,
        },
        request_id,
        surface_generation: core.terrain_payload_generation_for_patch(0, 0),
    };
    let in_stroke_request = request_for_current_generation(&core, 15);

    assert!(
        SimulationNode::terrain_patch_payload_build_source_for_request(
            &mut core,
            in_stroke_request
        )
        .is_none(),
        "refined work must retry while its immutable terrain snapshot is intentionally stale"
    );

    assert!(core.end_terrain_stroke_internal());
    let finalized_request = request_for_current_generation(&core, 16);
    assert!(
        SimulationNode::terrain_patch_payload_build_source_for_request(
            &mut core,
            finalized_request
        )
        .is_some(),
        "stroke finalization publishes the matching terrain/road snapshot"
    );
}

#[test]
fn refined_payload_source_waits_for_matching_road_surface_generation() {
    let mut core = test_core_with_flat_terrain(0.0);
    core.transit_network
        .road_surface
        .mark_world_point_dirty(Vector3::ZERO);
    let request = TerrainPatchPayloadRequest {
        key: TerrainPatchPayloadKey {
            patch_x: 0,
            patch_z: 0,
            render_step_mm: 2000,
        },
        request_id: 17,
        surface_generation: core.terrain_payload_generation_for_patch(0, 0),
    };

    assert!(
        SimulationNode::terrain_patch_payload_build_source_for_request(&mut core, request)
            .is_none(),
        "refined work must not pair the current graph with a stale published road surface"
    );
}

#[test]
fn current_refined_cache_uses_ready_payload_path() {
    let mut core = test_core_with_flat_terrain(0.0);
    core.terrain_payload_generation_counter = 7;
    core.terrain_payload_global_generation = 7;
    let cached = test_cached_refined_terrain_patch(TERRAIN_CDT_CONTRACT_REVISION, 7);
    let key = cached.key;
    core.refined_terrain_patch_cache
        .insert(key, Arc::new(cached));
    let request = TerrainPatchPayloadRequest {
        key: TerrainPatchPayloadKey {
            patch_x: key.patch_x,
            patch_z: key.patch_z,
            render_step_mm: key.render_step_mm,
        },
        request_id: 12,
        surface_generation: 7,
    };

    let source = SimulationNode::terrain_patch_payload_build_source_for_request(&mut core, request)
        .expect("current cache should produce a ready payload");

    assert!(source.is_ready());
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
    let cached = test_cached_refined_terrain_patch(TERRAIN_CDT_CONTRACT_REVISION, 1);
    let key = cached.key;
    core.refined_terrain_patch_cache
        .insert(key, Arc::new(cached));

    core.mark_building_site_terrain_dirty_bounds((5.0, 5.0, 15.0, 15.0));

    assert!(core.road_locked_terrain_patch_keys.is_empty());
    assert!(core.road_locked_terrain_patch_margins.is_empty());
    assert!(!core.terrain_payload_patch_generations.is_empty());
    assert!(
        core.refined_terrain_patch_cache.contains_key(&key),
        "ordinary patch invalidation must retain the previous immutable reuse source"
    );
}

#[test]
fn unmatched_surface_preserves_road_ownership_while_site_ownership_refreshes() {
    let mut core = test_core_with_flat_terrain(0.0);
    let key = (0, 0);
    let road_margin_m = 12.0;
    core.road_locked_terrain_patch_keys.push(key);
    core.road_locked_terrain_patch_margins
        .insert(key, road_margin_m);
    core.building_site_owned_terrain_patch_keys.insert(key);
    core.engineered_terrain_patch_keys.push(key);
    core.engineered_terrain_patch_margins
        .insert(key, road_margin_m);
    core.transit_network
        .road_surface
        .mark_world_point_dirty(Vector3::ZERO);

    core.refresh_engineered_terrain_patch_ownership_for_keys(
        crate::nodes::sim::core::ROAD_LOCKED_TERRAIN_RENDER_STEP_M,
        &[key],
    );

    assert_eq!(
        core.road_locked_terrain_patch_margins.get(&key),
        Some(&road_margin_m),
        "a site-only refresh must retain road ownership from the last-good surface"
    );
    assert!(core.road_locked_terrain_patch_keys.contains(&key));
    assert!(
        !core.building_site_owned_terrain_patch_keys.contains(&key),
        "site ownership must still refresh independently against the live allocator"
    );
    assert_eq!(
        core.engineered_terrain_patch_margins.get(&key),
        Some(&road_margin_m)
    );
}

#[test]
fn water_road_clip_queries_wait_for_matching_surface_publication() {
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
        SimulationNode::water_patch_mesh_build_input_for_request(&core, 0, 0, 1).is_some(),
        "the matching empty-road generation should produce a water mesh input"
    );

    core.transit_network
        .road_surface
        .mark_world_point_dirty(Vector3::ZERO);

    assert!(
        SimulationNode::water_patch_payload_for_request(&core, request).is_none(),
        "water payload clipping must not combine the current graph with stale road surfaces"
    );
    assert!(
        SimulationNode::water_patch_mesh_build_input_for_request(&core, 0, 0, 1).is_none(),
        "water mesh clipping must wait for matching road surface publication"
    );
}

#[test]
fn blank_world_reset_publishes_empty_road_surface_for_water_payloads() {
    let mut core = test_core_with_flat_terrain(0.0);
    core.create_blank_world_internal(40.0, 40.0, 10.0, 512.0, 0.0)
        .expect("the blank-world reset should succeed");

    assert!(
        core.transit_network
            .road_surface
            .published_generation_matches_source(),
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
        SimulationNode::water_patch_mesh_build_input_for_request(&core, 0, 0, 1).is_some(),
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
        SimulationNode::water_patch_mesh_build_input_for_request(&core, 0, 0, 1).is_some(),
        "water-only query changes must not invalidate matching road-clipped meshes"
    );
}

#[test]
fn conservative_road_grading_pad_filters_only_margin_only_engineered_patches() {
    use crate::simulation::network::graph::{Edge, RegionGraph};
    use crate::simulation::network::types::{
        EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
    };

    let road_ownership_for_patch_offset = |road_offset_m: f32| {
        let mut core = test_core_with_flat_terrain(0.0);
        let key = (0, 0);
        let render_step_m = crate::nodes::sim::core::ROAD_LOCKED_TERRAIN_RENDER_STEP_M;
        let (_, min_z, max_x, max_z) = core
            .heightmap
            .render_patch_world_bounds(key.0, key.1)
            .expect("the test terrain must expose its first render patch");
        let road_x = max_x + road_offset_m;
        let start_z = min_z + (max_z - min_z) * 0.25;
        let end_z = min_z + (max_z - min_z) * 0.75;
        let mut graph = RegionGraph::new();
        let start = graph.add_node(Vector3::new(road_x, 0.0, start_z), NodeType::Junction);
        let end = graph.add_node(Vector3::new(road_x, 0.0, end_z), NodeType::Junction);
        graph.add_edge(Edge {
            start_node: start,
            end_node: end,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            lanes: crate::simulation::network::graph::LaneLayout::from_counts(1, 1),
            speed_limit: 50.0,
            base_cost: 0.0,
            physical_length: end_z - start_z,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![
                Vector3::new(road_x, 0.0, start_z),
                Vector3::new(road_x, 0.0, end_z),
            ],
            physical_geometry: vec![
                Vector3::new(road_x, 0.0, start_z),
                Vector3::new(road_x, 0.0, end_z),
            ],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
            frontage_class: Default::default(),
        });
        core.region_graph = graph;
        // This fixture replaces the graph directly, so it must explicitly invalidate the
        // previously published empty surface before querying road-owned terrain margins.
        core.transit_network.road_surface.clear();
        core.transit_network
            .road_surface
            .compile_dirty(&core.region_graph, &core.heightmap);
        core.refresh_engineered_terrain_patch_ownership_for_keys(render_step_m, &[key]);
        core.road_locked_terrain_patch_margins.contains_key(&key)
    };

    assert!(
        !road_ownership_for_patch_offset(32.0),
        "a padded query candidate with no exact local grading influence must remain regular terrain"
    );
    assert!(
        road_ownership_for_patch_offset(8.0),
        "a neighboring road whose grading influence enters the patch must remain road-owned"
    );
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
    let entry_low = Arc::new(entry_low);
    let entry_high = Arc::new(entry_high);
    let entries = vec![Arc::clone(&entry_low), Arc::clone(&entry_high)];
    let mut built = Vec::new();

    SimulationNode::append_refined_terrain_patch_payloads_for_requests(
        &mut built, &requests, &entries,
    );

    assert_eq!(built.len(), 2);
    assert_eq!(built[0].key, request_high_key);
    assert_refined_payload_cache_key(&built[0], 9);
    let TerrainPatchPayloadData::Refined { patch } = &built[0].data else {
        panic!("expected refined payload");
    };
    assert!(Arc::ptr_eq(patch, &entry_high));
    assert_eq!(built[1].key, request_low_key);
    assert_refined_payload_cache_key(&built[1], 1);
    let TerrainPatchPayloadData::Refined { patch } = &built[1].data else {
        panic!("expected refined payload");
    };
    assert!(Arc::ptr_eq(patch, &entry_low));
}

#[test]
fn refined_async_payloads_publish_current_cdt_failures_without_retry() {
    let request_key = TerrainPatchPayloadKey {
        patch_x: 4,
        patch_z: 2,
        render_step_mm: 2000,
    };
    let requests = vec![TerrainPatchPayloadRequest {
        key: request_key,
        request_id: 21,
        surface_generation: 8,
    }];
    let mut failed_entry = test_cached_refined_terrain_patch(TERRAIN_CDT_CONTRACT_REVISION, 8);
    failed_entry.key.patch_x = request_key.patch_x;
    failed_entry.key.patch_z = request_key.patch_z;
    failed_entry.requires_engineered_refinement = true;
    failed_entry.requires_road_clipping = true;
    failed_entry.clip_source_count = 1;
    let failed_entry = Arc::new(failed_entry);
    let entries = vec![Arc::clone(&failed_entry)];

    assert!(SimulationNode::refined_requests_without_entries(&requests, &entries).is_empty());

    let mut built = Vec::new();
    SimulationNode::append_refined_terrain_patch_payloads_for_requests(
        &mut built, &requests, &entries,
    );

    assert_eq!(built.len(), 1);
    assert_eq!(built[0].key, request_key);
    let TerrainPatchPayloadData::RefinedFailure { patch, error_label } = &built[0].data else {
        panic!("expected failed refined payload");
    };
    assert!(Arc::ptr_eq(patch, &failed_entry));
    assert_eq!(*error_label, "missing_road_clip_sources");
}
