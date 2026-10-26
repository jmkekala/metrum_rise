// SPDX-License-Identifier: GPL-2.0-only

//! Regression tests for simulation state, snapshots, demand cadence, and budget behavior.

mod building_lod_fixture;
mod building_site_terrain;
mod commute_bounds;
mod construction_throughput;
pub(super) mod fields;
mod road_edit_order;
mod road_families;
mod road_reference_probe;
mod road_plan_scaling;
mod road_preview;
mod road_terrain_plan;
mod terrain_seams;
mod routing_adoption;
mod zoning_buildability;

use super::{
    CityTreasury, DailyBudgetLedgerEntry, RenderSnapshot, RoadPreviewRequest, SimCore,
    absolute_operational_minute, demand_plan_has_non_spawn_actions, demand_plan_without_spawns,
    pedestrian_lane_surface_height, road_tool_snapshot_from_core,
};
use crate::assets::AssetManifest;
use crate::assets::asset::{Anchor, AnchorType, BuildingData, MeshPart, PlacementMode, ZoneClass};
use crate::simulation::agriculture::AgricultureSystem;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::scheduler::{SLOTS_PER_HOUR, SLOTS_PER_MINUTE};
use crate::simulation::economy::agents::{
    AgentSystem, TRANSIT_ACCESS_EGRESS, TRANSIT_ACCESS_INGRESS,
};
use crate::simulation::economy::definitions::load_runtime_economy_catalog;
use crate::simulation::economy::demand::{
    DemandBuildingActionKey, DemandBuildingActionPlan, DemandSpawnAction, PlanningSlot,
};
use crate::simulation::grid::noise::NoiseSystem;
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::network::lanes::{Lane, LaneType};
use crate::simulation::network::roadbed::CURB_STEP_HEIGHT_M;
use crate::simulation::network::types::{
    EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
};
use crate::simulation::network::{TransitNetwork, graph::Edge};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::water::WaterSystem;
use crate::simulation::zoning::{ZoneType, ZoningSystem};
use godot::prelude::Vector3;
use std::collections::HashMap;
use std::collections::HashSet;

fn temp_save_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "metrum_rise_{name}_{}_{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos()
    ))
}

// The first slot of the hour after the clock's.
fn next_hour_slot(core: &SimCore) -> u64 {
    (core.time.absolute_slot() / SLOTS_PER_HOUR + 1) * SLOTS_PER_HOUR
}

fn test_core() -> SimCore {
    let config = WorldConfig::default();
    SimCore {
        treasury: CityTreasury::new(0.0),
        terrain_dirty: false,
        water_dirty: false,
        benchmark_mode: true,
        ..SimCore::new(config)
    }
}

#[test]
fn agent_commute_estimate_uses_active_world_clock() {
    use crate::simulation::economy::agents::tick::estimate_building_origin_trip_seconds;

    let mut core = test_core();
    add_test_border_road(&mut core);
    for (asset, zone) in [
        ("clock_home", ZoneType::Residential),
        ("clock_work", ZoneType::Commercial),
    ] {
        let id = register_test_asset(&mut core.allocator, asset, zone);
        add_test_complete_building(&mut core, id, zone);
    }
    core.allocator.buildings[0].frontage_t = 0.25;
    core.allocator.buildings[1].frontage_t = 0.75;
    core.allocator
        .recompute_derived_transforms(&core.region_graph, &core.zoning)
        .unwrap();
    core.allocator
        .rebuild_entrance_cache(&core.region_graph, &core.transit_network.lane_system);
    let physical_seconds = estimate_building_origin_trip_seconds(
        0,
        1,
        false,
        &core.allocator,
        &core.transit_network,
        &core.region_graph,
        &core.agents.pathfind_count,
        None,
    )
    .expect("the clock fixture must have a feasible walking commute");
    assert!(physical_seconds > 1);
    let tuning = core.demand.runtime_tuning().operational_clock.clone();
    for duration in [tuning.seconds_per_day * 0.5, tuning.seconds_per_day * 2.0] {
        // This is the live clock retained by save/load, independently of current tuning.
        core.time.seconds_per_day = duration;
        core.agents = AgentSystem::new();
        let home = &core.allocator.buildings[0];
        let agent = core
            .agents
            .spawn_housed_agent(0, home.center_x, home.center_y);
        core.agents.has_car[agent] = false;
        core.agents.assign_work_building(agent, 1, 0);
        core.agents.tick(
            &core.allocator,
            &mut core.transit_network,
            &mut core.region_graph,
            0.1,
            &core.time,
        );
        let expected_minutes = (f64::from(physical_seconds) / core.time.seconds_per_minute())
            .ceil()
            .clamp(1.0, f64::from(u16::MAX)) as u16;
        assert_eq!(core.agents.cached_commute_minutes[agent], expected_minutes);
        assert_eq!(
            core.agents.next_commute_refresh_time[agent],
            core.agents.sim_time
                + (f64::from(tuning.travel_estimate_refresh_minutes)
                    * core.time.seconds_per_minute()) as f32
        );
    }
}

#[test]
fn service_funding_rejects_nonfinite_changes_without_mutating_policy() {
    let mut core = test_core();
    add_test_border_road(&mut core);
    let id = register_test_asset(&mut core.allocator, "funding_test", ZoneType::Industrial);
    let mut manifest = core.allocator.registry.get(&id).unwrap().manifest.clone();
    let building = manifest.building.as_mut().unwrap();
    building.placement_mode = PlacementMode::Explicit;
    building.zone_type = None;
    building.density = None;
    building.economy_profile = Some("power_plant_basic".into());
    manifest.validate().unwrap();
    core.allocator
        .registry
        .register("test", manifest, String::new());
    add_test_complete_building(&mut core, id, ZoneType::Industrial);
    core.allocator.buildings[0].economy_profile_runtime_id = core
        .demand
        .runtime_catalog()
        .profile_for_id("power_plant_basic")
        .unwrap()
        .runtime_id;
    let building = &core.allocator.buildings[0];
    let (x, z) = (building.center_x, building.center_y);
    let service = super::SERVICE_POLICY_ELECTRICITY;
    assert!(core.set_service_funding(service, 0.4));
    assert!(core.set_building_service_funding_override_at(x, z, service, 0.6));
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(!core.set_service_funding(service, invalid));
        assert!(!core.set_building_service_funding_override_at(x, z, service, invalid));
        assert!(!core.set_building_service_funding_override_at(invalid, z, service, 0.0));
        assert!(!core.set_building_service_funding_override_at(x, invalid, service, 0.0));
        assert_eq!(core.service_policy.electricity_funding, 0.4);
        assert_eq!(core.allocator.buildings[0].service_funding_override, 0.6);
    }
    assert!(!core.set_service_funding("unknown", 0.0));
    assert!(core.set_service_funding(service, 2.0));
    assert!(core.set_building_service_funding_override_at(x, z, service, -2.0));
    assert_eq!(core.service_policy.electricity_funding, 1.0);
    assert_eq!(core.allocator.buildings[0].service_funding_override, 0.0);
}

#[test]
fn fiscal_policy_api_clamps_values_and_rejects_unknown_ids() {
    let mut core = test_core();

    assert!(
        core.set_fiscal_policy_value(crate::simulation::economy::fiscal::POLICY_INCOME_TAX, 3.0,)
    );
    assert!((core.fiscal_policy.income_tax_rate - 0.75).abs() < f32::EPSILON);

    assert!(core.set_fiscal_policy_value(
        crate::simulation::economy::fiscal::POLICY_UNEMPLOYMENT_MAX_DAYS,
        -10.0,
    ));
    assert_eq!(core.fiscal_policy.unemployment_max_days, 0);

    assert!(!core.set_fiscal_policy_value("unknown_policy", 1.0));
}

#[test]
fn test_core_uses_terrain_render_span_for_road_surface_chunks() {
    let core = test_core();
    assert_eq!(
        core.transit_network.roadbed.chunk_span_m(),
        core.heightmap.chunk_span_m()
    );
    assert_eq!(
        core.heightmap.chunk_span_m(),
        core.config.terrain_render_chunk_span_m()
    );
    assert_eq!(
        core.heightmap.chunk_span_m(),
        core.config.terrain_cell_m * core.heightmap.render_patch_interval_cells() as f32,
        "advertised render span must equal the physical terrain-patch interval span"
    );
    assert_eq!(
        core.transit_network.roadbed.chunk_origin_m(),
        core.config.terrain_world_origin_m(),
        "road render chunks must start at the terrain world minimum"
    );
}

#[test]
fn stale_terrain_acknowledgement_preserves_newer_patch_dirtiness() {
    let mut core = test_core();
    core.heightmap.mark_render_patch_dirty(0, 0);
    let stale_generation = core.terrain_payload_generation_for_patch(0, 0);
    core.bump_terrain_payload_patch_generations(&[(0, 0)]);

    assert!(!core.acknowledge_terrain_render_patch(0, 0, stale_generation));
    assert!(core.heightmap.dirty_render_patches().contains(&(0, 0)));

    let current_generation = core.terrain_payload_generation_for_patch(0, 0);
    assert!(core.acknowledge_terrain_render_patch(0, 0, current_generation));
    assert!(!core.heightmap.dirty_render_patches().contains(&(0, 0)));
}

#[test]
fn stale_terrain_batch_acknowledgement_reports_remaining_dirtiness() {
    let mut core = test_core();
    core.heightmap.mark_render_patch_dirty(0, 0);
    let stale_generation = core.terrain_payload_generation_for_patch(0, 0);
    core.bump_terrain_payload_patch_generations(&[(0, 0)]);

    assert!(!core.acknowledge_terrain_render_patches(&[(0, 0, stale_generation)]));
    assert!(core.terrain_dirty);
    assert!(core.heightmap.dirty_render_patches().contains(&(0, 0)));

    let current_generation = core.terrain_payload_generation_for_patch(0, 0);
    assert!(core.acknowledge_terrain_render_patches(&[(0, 0, current_generation)]));
    assert!(!core.terrain_dirty);
    assert!(!core.heightmap.dirty_render_patches().contains(&(0, 0)));
}

#[test]
fn full_engineered_terrain_refresh_does_not_depend_on_dirty_patches() {
    let mut core = test_core();
    for (patch_x, patch_z, generation) in core.terrain_dirty_patch_states() {
        core.acknowledge_terrain_render_patch(patch_x, patch_z, generation);
    }
    core.engineered_terrain_patch_keys.push((0, 0));
    core.engineered_terrain_patch_margins.insert((0, 0), 8.0);

    core.refresh_all_engineered_terrain_patch_state(
        crate::nodes::sim::core::ROAD_LOCKED_TERRAIN_RENDER_STEP_M,
    );

    assert!(
        core.engineered_terrain_patch_keys.is_empty(),
        "a full refresh must rescan all render patches even when no patch was already dirty"
    );
    assert!(
        core.heightmap.dirty_render_patches().contains(&(0, 0)),
        "ownership transitions must dirty the affected terrain patch"
    );
}

#[test]
fn terrain_stroke_steps_advance_touched_patch_revisions() {
    let mut core = test_core();
    for (patch_x, patch_z, generation) in core.terrain_dirty_patch_states() {
        core.acknowledge_terrain_render_patch(patch_x, patch_z, generation);
    }

    core.start_terrain_stroke_internal();
    core.sculpt_terrain_stroke_step_internal(godot::prelude::Vector2::ZERO, 12.0, 0.1);
    let first_states = core.terrain_dirty_patch_states();
    let &(patch_x, patch_z, stale_generation) = first_states
        .first()
        .expect("terrain stroke should dirty at least one render patch");

    core.sculpt_terrain_stroke_step_internal(godot::prelude::Vector2::ZERO, 12.0, 0.1);
    let current_generation = core.terrain_payload_generation_for_patch(patch_x, patch_z);

    assert!(current_generation > stale_generation);
    assert!(!core.acknowledge_terrain_render_patch(patch_x, patch_z, stale_generation));
    assert!(
        core.heightmap
            .dirty_render_patches()
            .contains(&(patch_x, patch_z))
    );
}

#[test]
fn stale_network_acknowledgement_preserves_newer_render_revision() {
    let mut core = test_core();
    core.mark_network_render_dirty();
    let stale_generation = core.road_tool_surface_generation;
    core.mark_network_render_dirty();
    core.pending_road_mesh_chunks =
        std::sync::Arc::new(std::collections::BTreeSet::from([(1, 2), (3, 4)]));
    core.published_road_mesh_chunks = std::sync::Arc::new(std::collections::BTreeMap::from([(
        (1, 2),
        std::sync::Arc::new(crate::simulation::network::NetworkMeshData::new()),
    )]));

    assert!(!core.acknowledge_network_render_generation(stale_generation));
    assert!(core.network_dirty);
    assert_eq!(core.pending_road_mesh_chunks.len(), 2);
    assert_eq!(core.published_road_mesh_chunks.len(), 1);
    assert!(core.road_mesh_full_replace);

    let current_generation = core.road_tool_surface_generation;
    assert!(core.acknowledge_network_render_generation(current_generation));
    assert!(!core.network_dirty);
    assert!(core.pending_road_mesh_chunks.is_empty());
    assert!(core.published_road_mesh_chunks.is_empty());
    assert!(!core.road_mesh_full_replace);
}

#[test]
fn removed_road_publishes_chunk_tombstones_without_rebuilding_the_world() {
    let mut core = test_core();
    add_test_border_road(&mut core);
    core.mark_network_render_dirty();
    core.precompute_road_mesh_data();
    let old_chunks = core
        .cached_road_mesh_chunks
        .keys()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    assert!(!old_chunks.is_empty());
    assert!(core.acknowledge_network_render_generation(core.road_tool_surface_generation));

    let edge = core.region_graph.edge(0);
    let endpoints = [edge.start_node, edge.end_node];
    core.region_graph.edge_mut(0).deleted = true;
    core.region_graph.rebuild_adjacency_list();
    core.transit_network
        .lane_system
        .rebuild(&mut core.region_graph);
    core.transit_network.roadbed.mark_edge_dirty(0);
    for node_id in endpoints {
        core.transit_network.roadbed.mark_node_dirty(node_id);
    }
    core.mark_local_network_render_dirty();
    core.precompute_road_mesh_data();

    assert!(core.cached_road_mesh_chunks.is_empty());
    assert!(
        old_chunks
            .iter()
            .all(|chunk| core.pending_road_mesh_chunks.contains(chunk)),
        "every removed cache entry must stay in the pending ledger as a Godot tombstone"
    );
    assert!(!core.road_mesh_full_replace);
}

#[test]
fn local_road_precompute_reuses_distant_chunk_mesh_allocation() {
    let mut core = test_core();
    for (start, end) in [
        (
            Vector3::new(-1_000.0, 0.0, 64.0),
            Vector3::new(-900.0, 0.0, 64.0),
        ),
        (
            Vector3::new(900.0, 0.0, 64.0),
            Vector3::new(1_000.0, 0.0, 64.0),
        ),
    ] {
        let start_node = core.region_graph.add_node(start, NodeType::Junction);
        let end_node = core.region_graph.add_node(end, NodeType::Junction);
        core.region_graph
            .add_edge(test_road_edge(start_node, end_node, vec![start, end]));
    }
    core.region_graph.rebuild_adjacency_list();
    core.region_graph.rebuild_intersection_clips();
    core.transit_network
        .lane_system
        .rebuild(&mut core.region_graph);
    core.mark_network_render_dirty();
    core.precompute_road_mesh_data();
    assert!(core.acknowledge_network_render_generation(core.road_tool_surface_generation));

    let distant_chunk = core.transit_network.roadbed.chunk_key_at(950.0, 64.0);
    let distant_mesh = std::sync::Arc::clone(
        core.cached_road_mesh_chunks
            .get(&distant_chunk)
            .expect("distant road mesh chunk"),
    );

    core.transit_network.roadbed.mark_edge_dirty(0);
    core.mark_local_network_render_dirty();
    core.precompute_road_mesh_data();

    assert!(std::sync::Arc::ptr_eq(
        core.cached_road_mesh_chunks
            .get(&distant_chunk)
            .expect("unaffected distant chunk must remain cached"),
        &distant_mesh
    ));
    assert!(!core.pending_road_mesh_chunks.contains(&distant_chunk));
}

#[test]
fn unacknowledged_local_road_generations_accumulate_chunk_payloads() {
    let mut core = test_core();
    for (start, end) in [
        (
            Vector3::new(-1_000.0, 0.0, 64.0),
            Vector3::new(-900.0, 0.0, 64.0),
        ),
        (
            Vector3::new(900.0, 0.0, 64.0),
            Vector3::new(1_000.0, 0.0, 64.0),
        ),
    ] {
        let start_node = core.region_graph.add_node(start, NodeType::Junction);
        let end_node = core.region_graph.add_node(end, NodeType::Junction);
        core.region_graph
            .add_edge(test_road_edge(start_node, end_node, vec![start, end]));
    }
    core.region_graph.rebuild_adjacency_list();
    core.region_graph.rebuild_intersection_clips();
    core.transit_network
        .lane_system
        .rebuild(&mut core.region_graph);
    core.mark_network_render_dirty();
    core.precompute_road_mesh_data();
    assert!(core.acknowledge_network_render_generation(core.road_tool_surface_generation));

    core.transit_network.roadbed.mark_edge_dirty(0);
    core.mark_local_network_render_dirty();
    core.precompute_road_mesh_data();
    let first_generation = core.cached_road_mesh_generation;
    let first_pending = core.pending_road_mesh_chunks.as_ref().clone();
    let first_payloads = core.published_road_mesh_chunks.as_ref().clone();
    assert!(!first_pending.is_empty());
    assert!(!first_payloads.is_empty());

    core.transit_network.roadbed.mark_edge_dirty(1);
    core.mark_local_network_render_dirty();
    core.precompute_road_mesh_data();

    assert!(core.cached_road_mesh_generation > first_generation);
    assert!(first_pending.is_subset(&core.pending_road_mesh_chunks));
    assert!(first_payloads.iter().all(|(chunk, mesh)| {
        std::sync::Arc::ptr_eq(
            core.published_road_mesh_chunks
                .get(chunk)
                .expect("a skipped generation must retain its pending upsert"),
            mesh,
        )
    }));
    assert!(core.pending_road_mesh_chunks.len() > first_pending.len());

    assert!(core.acknowledge_network_render_generation(core.cached_road_mesh_generation));
    assert!(core.pending_road_mesh_chunks.is_empty());
    assert!(core.published_road_mesh_chunks.is_empty());
}

#[test]
fn local_network_render_invalidation_preserves_unrelated_terrain_generation() {
    let mut core = test_core();
    core.cached_road_mesh_chunks = std::collections::BTreeMap::from([(
        (0, 0),
        std::sync::Arc::new(crate::simulation::network::NetworkMeshData::new()),
    )]);
    core.cached_road_mesh_generation = core.road_tool_surface_generation;
    core.cached_network_node_positions = std::sync::Arc::new(vec![Vector3::new(12.0, 3.0, -8.0)]);
    core.cached_network_node_positions_dirty = false;
    let previous_mesh = std::sync::Arc::clone(
        core.cached_road_mesh_chunks
            .get(&(0, 0))
            .expect("seeded road mesh chunk"),
    );
    let previous_node_positions = std::sync::Arc::clone(&core.cached_network_node_positions);
    let unchanged_generation = core.terrain_payload_generation_for_patch(9, 9);
    let road_query_generation = core.road_tool_surface_generation;

    core.bump_terrain_payload_patch_generations(&[(1, 2)]);
    core.mark_local_network_render_dirty();

    assert!(
        core.terrain_payload_generation_for_patch(1, 2) > unchanged_generation,
        "the road-touched patch must advance"
    );
    assert_eq!(
        core.terrain_payload_generation_for_patch(9, 9),
        unchanged_generation,
        "a local road edit must not cancel unrelated terrain payload work"
    );
    assert!(core.road_tool_surface_generation > road_query_generation);
    assert!(core.network_dirty);
    assert!(std::sync::Arc::ptr_eq(
        core.cached_road_mesh_chunks
            .get(&(0, 0))
            .expect("local invalidation must retain the cached chunk"),
        &previous_mesh
    ));
    let snapshot = core.build_snapshot();
    assert_eq!(
        snapshot.network_generation, road_query_generation,
        "a retained mesh must keep its last-successful generation token"
    );
    assert!(
        std::sync::Arc::ptr_eq(&snapshot.node_positions, &previous_node_positions),
        "a failed-generation snapshot must retain node positions matching the last-good mesh"
    );

    core.mark_network_render_dirty();
    assert!(
        core.cached_road_mesh_chunks.is_empty() && core.road_mesh_full_replace,
        "world-wide invalidation must discard a mesh from the previous world generation"
    );
}

#[test]
fn build_snapshot_reuses_cached_network_nodes_until_network_dirty() {
    let mut core = test_core();
    add_test_border_road(&mut core);
    core.mark_network_render_dirty();
    core.precompute_road_mesh_data();

    let first = core.build_snapshot();
    assert_eq!(first.node_positions.len(), 2);
    let first_nodes = std::sync::Arc::clone(&first.node_positions);

    let second = core.build_snapshot();
    assert!(std::sync::Arc::ptr_eq(&first_nodes, &second.node_positions));

    core.mark_network_render_dirty();
    core.precompute_road_mesh_data();
    let third = core.build_snapshot();
    assert_eq!(third.node_positions.len(), 2);
    assert!(!std::sync::Arc::ptr_eq(&first_nodes, &third.node_positions));
}

#[test]
fn build_snapshot_publishes_terrain_layout_before_first_tick() {
    let mut core = test_core();
    let expected_width = core.heightmap.width;
    let expected_height = core.heightmap.height;
    let expected_world_size = core.heightmap.world_size();

    let snapshot = core.build_snapshot();

    assert_eq!(snapshot.heightmap_width, expected_width);
    assert_eq!(snapshot.heightmap_height, expected_height);
    assert_eq!(
        snapshot.terrain_world_size,
        godot::prelude::Vector2::new(expected_world_size.0, expected_world_size.1)
    );
    assert!(snapshot.terrain_patch_cols > 0);
    assert!(snapshot.terrain_patch_rows > 0);
}

#[test]
fn recycled_snapshot_keeps_capacity_without_stale_instances() {
    let mut core = test_core();
    let mut recycled = RenderSnapshot::default();
    let mut transforms = Vec::with_capacity(64);
    transforms.extend_from_slice(&[1.0; 16]);
    recycled.pedestrian_transforms.insert(7, transforms);
    let original_capacity = recycled.pedestrian_transforms[&7].capacity();

    let snapshot = core.build_snapshot_reusing(recycled);

    assert!(snapshot.pedestrian_transforms[&7].is_empty());
    assert_eq!(
        snapshot.pedestrian_transforms[&7].capacity(),
        original_capacity
    );
}

#[test]
fn road_preview_rejects_a_mismatched_surface_generation() {
    let mut core = test_core();
    core.precompute_road_mesh_data();
    let snapshot =
        road_tool_snapshot_from_core(&core).expect("the initial surface must be publishable");
    let (_, validation) = snapshot.validate(&RoadPreviewRequest {
        surface_generation: snapshot.surface_generation.wrapping_add(1),
        points: vec![Vector3::new(-5.0, 0.0, 0.0), Vector3::new(5.0, 0.0, 0.0)],
        fwd_lanes: 1,
        bkw_lanes: 1,
        snap_to_existing_roads: true,
    });

    assert!(!validation.is_valid);
    assert_eq!(validation.invalid_reason, "stale_surface_generation");
}

#[test]
fn failed_junction_publishes_a_gap_not_its_previous_mesh() {
    let mut core = test_core();
    let center_pos = Vector3::ZERO;
    let center = core.region_graph.add_node(center_pos, NodeType::Junction);
    let mut endpoints = Vec::new();
    let mut edge_ids = Vec::new();
    for (endpoint_pos, starts_at_center) in [
        (Vector3::new(-80.0, 0.0, 0.0), false),
        (Vector3::new(80.0, 0.0, 0.0), true),
        (Vector3::new(0.0, 0.0, -80.0), false),
        (Vector3::new(0.0, 0.0, 80.0), true),
    ] {
        let endpoint = core.region_graph.add_node(endpoint_pos, NodeType::Junction);
        let (start, end, points) = if starts_at_center {
            (center, endpoint, vec![center_pos, endpoint_pos])
        } else {
            (endpoint, center, vec![endpoint_pos, center_pos])
        };
        endpoints.push(endpoint);
        edge_ids.push(
            core.region_graph
                .add_edge(test_road_edge(start, end, points)),
        );
    }
    core.region_graph.rebuild_adjacency_list();
    core.region_graph.rebuild_intersection_clips();
    core.precompute_road_mesh_data();

    let published_mesh = std::sync::Arc::clone(&core.published_road_mesh_chunks);
    assert!(
        !published_mesh.is_empty(),
        "the flat four-way junction must publish a baseline mesh"
    );
    let published_generation = core.cached_road_mesh_generation;
    let published_node_positions = core.build_snapshot().node_positions;
    assert!(
        road_tool_snapshot_from_core(&core).is_some(),
        "the baseline road-tool source must be publishable"
    );

    for (&endpoint, height_m) in endpoints.iter().zip([80.0, -80.0, 64.0, -64.0]) {
        let mut pos = core.region_graph.node(endpoint).pos;
        pos.y = height_m;
        core.region_graph.set_node_pos(endpoint, pos);
    }
    for &edge_idx in &edge_ids {
        let edge = core.region_graph.edge(edge_idx);
        let points = vec![
            core.region_graph.node(edge.start_node).pos,
            core.region_graph.node(edge.end_node).pos,
        ];
        let edge = core.region_graph.edge_mut(edge_idx);
        edge.geometry = points.clone();
        edge.physical_geometry = points;
    }
    core.region_graph
        .finalize_junction_endpoint_profiles(
            &HashSet::from([center]),
            Some(&core.heightmap),
            &mut HashSet::new(),
        );
    // Steep planar roads alone are not a contradiction. Break the actual shared seam.
    core.region_graph
        .edge_mut(edge_ids[0])
        .physical_geometry
        .last_mut()
        .unwrap()
        .y += 1.0;
    core.region_graph.rebuild_intersection_clips();
    for &edge_idx in &edge_ids {
        core.transit_network.roadbed.mark_edge_dirty(edge_idx);
    }
    core.transit_network.roadbed.mark_node_dirty(center);
    core.mark_local_network_render_dirty();
    core.precompute_road_mesh_data();

    assert!(
        !core.transit_network.roadbed.failed_nodes().is_empty(),
        "the contradictory JunctionN must exercise the failed-production path"
    );
    // The failed junction publishes as a gap: its previous mesh must not stay on screen.
    assert!(
        core.transit_network.roadbed.height_at(0.0, 0.0).is_none(),
        "the failed junction must have no surface"
    );
    assert_ne!(
        core.cached_road_mesh_generation, published_generation,
        "a failed node must not hold the previous mesh generation"
    );
    assert!(
        published_mesh.iter().any(|(chunk, mesh)| !core
            .cached_road_mesh_chunks
            .get(chunk)
            .is_some_and(|current| std::sync::Arc::ptr_eq(current, mesh))),
        "the chunks under the failed junction must be republished"
    );
    assert!(
        road_tool_snapshot_from_core(&core).is_some(),
        "the road tool must see the current roadbed, gap included"
    );
    let failed_snapshot = core.build_snapshot();
    assert_eq!(
        failed_snapshot.network_generation,
        core.cached_road_mesh_generation
    );
    assert!(std::sync::Arc::ptr_eq(
        &failed_snapshot.road_mesh_chunks,
        &core.published_road_mesh_chunks
    ));
    assert!(!std::sync::Arc::ptr_eq(
        &failed_snapshot.node_positions,
        &published_node_positions
    ));
}

#[test]
fn bulk_road_finalizer_solves_profiles_before_surface_compile() {
    let mut core = test_core();
    let center_pos = Vector3::new(0.0, 10.0, 0.0);
    let west_pos = Vector3::new(-48.0, 10.0, 0.0);
    let north_pos = Vector3::new(0.0, 22.0, 48.0);
    let center = core.region_graph.add_node(center_pos, NodeType::Junction);
    let west = core.region_graph.add_node(west_pos, NodeType::Junction);
    let north = core.region_graph.add_node(north_pos, NodeType::Junction);

    let stable_edge =
        core.region_graph
            .add_edge(test_road_edge(west, center, vec![west_pos, center_pos]));
    let new_edge =
        core.region_graph
            .add_edge(test_road_edge(center, north, vec![center_pos, north_pos]));

    core.transit_network.bulk_dirty_edges.insert(new_edge);
    core.transit_network.roadbed.mark_edge_dirty(new_edge);
    core.transit_network.roadbed.mark_node_dirty(center);
    core.transit_network.roadbed.mark_node_dirty(north);

    let stable_before = core.region_graph.edge(stable_edge).geometry.clone();
    let finalized = core.finalize_bulk_road_geometry_for_dirty_edges();

    assert!(finalized.affected_nodes.contains(&center));
    assert!(finalized.dirty_edges.contains(&new_edge));
    assert!(core.transit_network.bulk_dirty_edges.is_empty());
    assert_eq!(
        core.region_graph.edge(stable_edge).geometry,
        stable_before,
        "stable bend authority edge must not be rewritten by bulk finalization"
    );
    assert!(
        core.region_graph
            .edge(new_edge)
            .physical_geometry
            .iter()
            .all(|point| { (point.y - (10.0 + point.z * 0.25)).abs() < 1.0e-4 }),
        "the supported bend must retain its hillside grade instead of adding a flat platform"
    );
    assert!(
        core.transit_network
            .roadbed
            .dirty_edges()
            .contains(&new_edge),
        "changed profile edge must stay marked for the following road-surface compile"
    );
}

#[test]
fn planned_topology_terrain_rejection_restores_split_dependents() {
    use crate::simulation::buildings::allocator::EdgeOccupancy;
    let mut core = test_core();
    core.benchmark_mode = false;
    core.treasury.balance = 10_000.0;
    assert!(
        core.add_road_internal(
            vec![
                Vector3::new(-100.0, 0.0, 0.0),
                Vector3::new(100.0, 0.0, 0.0)
            ],
            1,
            1,
        )
        .committed
    );
    core.precompute_road_mesh_data();
    let asset = register_test_asset(&mut core.allocator, "plan-rollback", ZoneType::Residential);
    add_test_complete_building(&mut core, asset, ZoneType::Residential);
    let building = &mut core.allocator.buildings[0];
    building.cell_x = 20;
    building.frontage_t = 0.85;
    core.allocator
        .recompute_derived_transforms(&core.region_graph, &core.zoning)
        .unwrap();
    core.allocator
        .rebuild_entrance_cache(&core.region_graph, &core.transit_network.lane_system);
    let original = core.allocator.buildings[0].clone();
    core.allocator.edge_occupancy.insert(
        0,
        EdgeOccupancy {
            cells_long: 32,
            left: vec![false; 32],
            right: vec![true; 32],
        },
    );
    let profile = core.region_graph.edge(0).physical_geometry.clone();
    let terrain_before = core
        .heightmap
        .render_patch_keys_for_world_bounds(-120.0, -100.0, 120.0, 120.0)
        .into_iter()
        .map(|(x, z)| ((x, z), core.heightmap.visual_patch_snapshot(x, z)))
        .collect::<Vec<_>>();
    let undo_count = core.undo_stack.len();
    let lane_count = core.transit_network.lane_system.lanes.len();
    let points = vec![Vector3::new(0.0, 0.0, -80.0), Vector3::ZERO];
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    let plan = super::RoadEditPlan::compile(
        &core,
        RoadPreviewRequest {
            surface_generation: core.road_tool_surface_generation,
            points: points.clone(),
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
        },
    );
    assert!(plan.topology_for(&core.region_graph).is_some());
    assert_eq!(plan.status(&core), "ready", "{:?}", plan.validation());
    core.transit_network.begin_road_edit();
    core.transit_network.bulk_load = true;
    let result = core.add_road_internal_with_snap_and_validation(points, 1, 1, true, Some(&plan));
    assert!(result.committed && result.finalized_geometry.is_some());
    assert_ne!(core.allocator.buildings[0].edge_idx, original.edge_idx);
    core.transit_network.bulk_load = false;
    core.terrain_stroke_active = true;
    // Refined terrain cannot be prepared against an unfinished authored terrain stroke.
    assert!(!core.validate_staged_road_render());
    assert!(core.last_road_timing.contains("terrain_input_unavailable"));
    assert_eq!(core.region_graph.edge_count(), 1);
    assert_eq!(core.region_graph.edge(0).physical_geometry, profile);
    let restored = &core.allocator.buildings[0];
    assert_eq!(
        (restored.edge_idx, restored.cell_x, restored.frontage_t),
        (original.edge_idx, original.cell_x, original.frontage_t)
    );
    assert_eq!(core.allocator.edge_occupancy[&0].cells_long, 32);
    assert_eq!(core.allocator.edge_occupancy[&0].right, vec![true; 32]);
    assert_eq!(core.allocator.edge_occupancy.len(), 1);
    assert_eq!(core.undo_stack.len(), undo_count);
    assert_eq!(core.transit_network.lane_system.lanes.len(), lane_count);
    assert_eq!(core.treasury.balance, 10_000.0);
    assert!(!core.transit_network.road_edit_is_staged());
    for ((x, z), before) in terrain_before {
        assert_eq!(core.heightmap.visual_patch_snapshot(x, z), before);
    }
}

#[test]
fn stale_topology_plan_is_rejected_before_authoritative_mutation() {
    let mut core = test_core();
    core.precompute_road_mesh_data();
    let points = vec![Vector3::new(-24.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)];
    let plan = super::RoadEditPlan::compile(
        &core,
        RoadPreviewRequest {
            surface_generation: core.road_tool_surface_generation,
            points: points.clone(),
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
        },
    );
    assert!(plan.topology_for(&core.region_graph).is_some());
    core.road_tool_surface_generation += 1;
    core.transit_network.bulk_load = true;
    let result = core.add_road_internal_with_snap_and_validation(points, 1, 1, true, Some(&plan));
    assert!(!result.committed);
    assert!(result.finalized_geometry.is_none());
    assert!(core.transit_network.bulk_dirty_edges.is_empty());
    assert_eq!(core.region_graph.edge_count(), 0);
}

fn add_test_border_road(core: &mut SimCore) {
    let border = core
        .region_graph
        .add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Border);
    let junction = core
        .region_graph
        .add_node(Vector3::new(180.0, 0.0, 0.0), NodeType::Junction);
    core.region_graph.add_edge(Edge {
        start_node: border,
        end_node: junction,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 13.89,
        base_cost: 180.0,
        physical_length: 180.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(180.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(180.0, 0.0, 0.0)],
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access: VehicleFrontageAccess::BothSides,
    });
    core.transit_network
        .lane_system
        .rebuild(&mut core.region_graph);
}

fn add_test_complete_building(core: &mut SimCore, asset_id: String, zone_type: ZoneType) {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let zone_profile_runtime_id = core
        .zoning
        .profiles
        .default_runtime_id_for_zone_type(zone_type)
        .expect("test zone profile");
    core.allocator.buildings.push(Building {
        build_generation: 0,
        center_x: 0.0,
        center_y: 0.0,
        support_height_m: 0.0,
        width_cells: 2,
        depth_cells: 2,
        zone_profile_runtime_id,
        parcel_id: 0,
        applicant_seed: 0,
        zone_type,
        facing_dir: godot::prelude::Vector2::ZERO,
        frontage_t: 0.5,
        side_offset: 0.0,
        front_gap_m: [0.0; 3],
        budget_distress: false,
        is_deserted: false,
        edge_idx: 0,
        side: 1,
        cell_x: 3,
        cell_y: 0,
        occupancy: 0,
        worker_count: 0,
        service_funding_override: -1.0,
        asset_id,
        level: 1,
        construction_total_hours: 0,
        construction_complete_slot: 0,
        broken: false,
        economy_profile_runtime_id: 0,
        economy_broken: false,
        resource_inventory: vec![0.0; catalog.resource_count()],
        revenue: 0.0,
        operating_budget: 0.0,
        profit_tax_budget_baseline: 0.0,
        last_day_profit: 0.0,
        shipment_cooldown_until_slot: 0,
        daily_owa_input_value: 0.0,
        daily_local_input_value: 0.0,
        daily_city_funded_input_cost: 0.0,
        daily_household_sales_value: 0.0,
        daily_power_service_units: 0.0,
        daily_electricity: Default::default(),
        recent_electricity: Default::default(),
        hourly_economy_activity: 0.0,
        daily_power_served_units: 0.0,
        recent_power_service_units: 0.0,
        recent_power_served_units: 0.0,
        paid_workers_in_window: 0,
        funded_job_capacity: 0,
        unpaid_cash_wages_in_window: 0,
        recent_household_sales_value: 0.0,
        commercial_activity_floor_scale: 0.0,
        service_sales_capacity: 0.0,
        service_sales_profile_runtime_id: 0,
        funded_activity: 0.0,
        work_area_scale: 1.0,
        pending_redevelopment: false,
        rezone_grace_days_remaining: 0,
    });
    core.allocator
        .recompute_derived_transforms(&core.region_graph, &core.zoning)
        .expect("test building transforms");
    core.allocator
        .rebuild_entrance_cache(&core.region_graph, &core.transit_network.lane_system);
}

#[test]
fn load_game_rebuilds_entrances_after_registry_restore() {
    let mut source = test_core();
    add_test_border_road(&mut source);
    let asset_id = register_test_asset(
        &mut source.allocator,
        "load_registry_residential",
        ZoneType::Residential,
    );
    add_test_complete_building(&mut source, asset_id, ZoneType::Residential);
    source.budget_history.push_back(DailyBudgetLedgerEntry {
        day_index: 7,
        income: 300.0,
        expenses: 125.0,
        net: 175.0,
        treasury: 1_175.0,
        power_coverage: 0.8,
        ..DailyBudgetLedgerEntry::default()
    });

    let save_path = temp_save_path("load_registry_entrances");
    source
        .save_game_internal(save_path.to_str().expect("utf-8 temp path"), None)
        .expect("save test world");

    let mut loaded = test_core();
    register_test_asset(
        &mut loaded.allocator,
        "load_registry_residential",
        ZoneType::Residential,
    );
    loaded.sculpt_terrain_stroke_step_internal(godot::prelude::Vector2::ZERO, 16.0, 0.5);
    assert!(loaded.terrain_stroke_active && loaded.terrain_stroke_has_changes);
    loaded.last_tick_duration = 123.0;
    loaded.last_agent_tick_us = 456;
    loaded.last_road_timing = "previous world".to_owned();
    loaded.last_surface_debug_edges.push(0);
    loaded.camera_aabb = (-100.0, -100.0, 100.0, 100.0);
    loaded
        .load_game_internal(save_path.to_str().expect("utf-8 temp path"))
        .expect("load test world");
    let _ = std::fs::remove_file(save_path);

    assert_eq!(loaded.allocator.entrances.len(), 1);
    assert!(loaded.allocator.household_capacity(0) > 0);
    assert_eq!(
        loaded.allocator.first_rehousing_vacancy(|_| true),
        Some(0),
        "loaded vacancies must use the restored registry before the first tick"
    );
    let entrance = &loaded.allocator.entrances[0];
    assert_ne!(entrance.foot_lane_fwd, usize::MAX);
    assert_ne!(entrance.foot_lane_bkw, usize::MAX);
    assert_ne!(entrance.car_lane_fwd, usize::MAX);
    assert_ne!(entrance.car_lane_bkw, usize::MAX);
    assert_eq!(loaded.allocator.building_sites.len(), 1);
    assert_eq!(loaded.budget_history.len(), 1);
    assert_eq!(loaded.budget_history[0].day_index, 7);
    assert_eq!(loaded.budget_history[0].net, 175.0);
    assert!(
        !loaded.end_terrain_stroke_internal(),
        "old brush cancellation must not rebuild the loaded world"
    );
    assert_eq!(loaded.last_tick_duration, 0.0);
    assert_eq!(loaded.last_agent_tick_us, 0);
    assert!(loaded.last_road_timing.is_empty());
    assert!(loaded.last_surface_debug_edges.is_empty());
    assert_eq!(loaded.camera_aabb, (0.0, 0.0, 0.0, 0.0));
}

#[test]
#[ignore = "isolated release measurement of blank-world replacement with retained assets"]
fn benchmark_blank_world_reset_with_assets() {
    use std::time::Instant;

    for asset_count in [128, 4096] {
        let mut core = test_core();
        for index in 0..asset_count {
            register_test_asset(
                &mut core.allocator,
                &format!("reset_asset_{index}"),
                ZoneType::Residential,
            );
        }
        let first = core
            .allocator
            .registry
            .get("test:reset_asset_0")
            .unwrap()
            .manifest
            .clone();
        let revision = core.allocator.registry.revision();
        let mut samples = Vec::with_capacity(11);
        // The benchmark measures replacement itself. Asset registration and the initial
        // test world are outside timing; all measured worlds have the same empty layout.
        for sample in 0..14 {
            let start = Instant::now();
            for _ in 0..5 {
                core.create_blank_world_internal(256.0, 256.0, 8.0, 128.0, 50.0)
                    .unwrap();
            }
            let milliseconds = start.elapsed().as_secs_f64() * 1000.0 / 5.0;
            if sample >= 3 {
                samples.push(milliseconds);
            }
        }
        assert_eq!(core.allocator.registry.len(), asset_count);
        assert_eq!(core.allocator.registry.revision(), revision);
        assert_eq!(
            core.allocator
                .registry
                .get("test:reset_asset_0")
                .unwrap()
                .manifest
                .asset_id,
            first.asset_id
        );
        assert_eq!(
            core.allocator
                .registry
                .household_capacity("test:reset_asset_0"),
            6
        );
        assert_eq!(core.heightmap.sample_height_world(0.0, 0.0), 50.0);
        assert!(core.allocator.buildings.is_empty() && core.region_graph.edges().is_empty());
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "blank_world_reset assets={asset_count} median_ms={:.6}",
            samples[samples.len() / 2]
        );
    }
}

fn test_road_edge(start_node: u32, end_node: u32, geometry: Vec<Vector3>) -> Edge {
    let physical_length = geometry
        .windows(2)
        .map(|points| points[0].distance_to(points[1]))
        .sum();
    Edge {
        start_node,
        end_node,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 13.89,
        base_cost: physical_length,
        physical_length,
        start_clip: 0.0,
        end_clip: 0.0,
        physical_geometry: geometry.clone(),
        geometry,
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access: VehicleFrontageAccess::BothSides,
    }
}

fn register_test_asset(
    allocator: &mut BuildingAllocator,
    asset_id: &str,
    zone_type: ZoneType,
) -> String {
    let (zone_class, household_capacity, worker_capacity, economy_profile) = match zone_type {
        ZoneType::Residential => (ZoneClass::Residential, Some(6), None, None),
        ZoneType::Commercial => (
            ZoneClass::Commercial,
            None,
            Some(4),
            Some("grocery_basic".to_owned()),
        ),
        ZoneType::Industrial => (
            ZoneClass::Industrial,
            None,
            Some(4),
            Some("food_processor_basic".to_owned()),
        ),
        _ => panic!("test asset requires a baseline private-use zone"),
    };
    let manifest = AssetManifest {
        asset_id: asset_id.to_owned(),
        display_name: "Test".to_owned(),
        asset_set: None,
        tags: vec![],
        thumbnail: None,
        lods: vec![],
        mesh_parts: vec![MeshPart::single_lod0("main", "lod0.glb")],
        anchors: vec![Anchor {
            anchor_type: AnchorType::Entrance,
            name: "main".to_owned(),
            position: [0.0, 0.0, 0.5],
            forward: [0.0, 0.0, 1.0],
            width_m: None,
            length_m: None,
            vehicle_class: None,
        }],
        site_surfaces: vec![],
        building: Some(BuildingData {
            window_brightness: 3.0,
            appearance: None,
            flat_size_m2: household_capacity.map(|_| 80.0),
            placement_mode: PlacementMode::ZonedPrivate,
            zone_type: Some(zone_class),
            density: Some("low".to_owned()),
            lot_width_cells: 2,
            lot_depth_cells: 2,
            frontage_forward: None,
            min_zone_width_cells: None,
            min_zone_depth_cells: None,
            level: 1,
            household_capacity,
            worker_capacity,
            service_class: None,
            economy_profile,
            extractor: None,
            field: None,
            yard_hedge: None,
            yard_planting: Vec::new(),
        }),
        prop: None,
        vehicle: None,
        character: None,
    };
    allocator.registry.register("test", manifest, String::new());
    format!("test:{asset_id}")
}

fn place_test_parcel_run(core: &mut SimCore, zone_type: ZoneType, start_x: f32, end_x: f32) {
    let profile = core
        .zoning
        .profiles
        .default_runtime_id_for_zone_type(zone_type)
        .expect("test zoning profile");
    core.zoning
        .place_parcel_run_at(
            start_x,
            -20.0,
            end_x,
            -20.0,
            profile,
            20.0,
            30.0,
            0.0,
            &core.region_graph,
        )
        .expect("test parcel run");
}

#[test]
fn absolute_operational_minute_is_day_stable() {
    assert_eq!(absolute_operational_minute(1, 0), 0);
    assert_eq!(absolute_operational_minute(1, 1439), 1439);
    assert_eq!(absolute_operational_minute(2, 0), 1440);
    assert_eq!(absolute_operational_minute(3, 60), 2940);
}

#[test]
fn immediate_demand_plan_strips_spawns_only() {
    let mut plan = DemandBuildingActionPlan::default();
    plan.residential.spawns.push(DemandSpawnAction {
        parcel_id: 7,
        asset_id: "building.residential.test".to_owned(),
    });
    plan.residential.despawns.push(DemandBuildingActionKey {
        parcel_id: 8,
        edge_idx: 1,
        side: 1,
        cell_x: 2,
        width_cells: 3,
        depth_cells: 4,
        level: 1,
        asset_id: "building.residential.old".to_owned(),
    });

    let immediate = demand_plan_without_spawns(&plan);

    assert!(immediate.residential.spawns.is_empty());
    assert_eq!(immediate.residential.despawns.len(), 1);
    assert!(demand_plan_has_non_spawn_actions(&immediate));
}

#[test]
fn max_demand_cheat_runtime_queues_and_executes_rci_spawns() {
    let mut core = test_core();
    add_test_border_road(&mut core);
    register_test_asset(&mut core.allocator, "residential", ZoneType::Residential);
    register_test_asset(&mut core.allocator, "commercial", ZoneType::Commercial);
    register_test_asset(&mut core.allocator, "industrial", ZoneType::Industrial);
    place_test_parcel_run(&mut core, ZoneType::Residential, 10.0, 50.0);
    place_test_parcel_run(&mut core, ZoneType::Commercial, 60.0, 100.0);
    place_test_parcel_run(&mut core, ZoneType::Industrial, 110.0, 150.0);

    core.apply_money_and_max_demand_cheat(1_000_000.0);
    // The next hour's planning slots, through its actions slot, which queues the spawns.
    let hour = next_hour_slot(&core);
    core.run_slots_through_for_test(hour + PlanningSlot::Actions as u64);

    assert!(
        core.pending_demand_spawns
            .iter()
            .any(|pending| pending.zone_type == ZoneType::Residential)
    );
    assert!(
        core.pending_demand_spawns
            .iter()
            .any(|pending| pending.zone_type == ZoneType::Commercial)
    );
    assert!(
        core.pending_demand_spawns
            .iter()
            .any(|pending| pending.zone_type == ZoneType::Industrial)
    );

    // One start a minute, each at its construction-start event, within the hour.
    let queued_spawn_count = core.pending_demand_spawns.len() as u64;
    assert!(queued_spawn_count < 56);
    core.run_slots_through_for_test(
        hour + PlanningSlot::Actions as u64 + (queued_spawn_count + 1) * SLOTS_PER_MINUTE,
    );
    assert!(core.pending_demand_spawns.is_empty());
    assert!(
        core.allocator
            .buildings
            .iter()
            .any(|building| building.zone_type == ZoneType::Residential)
    );
    assert!(
        core.allocator
            .buildings
            .iter()
            .any(|building| building.zone_type == ZoneType::Commercial)
    );
    assert!(
        core.allocator
            .buildings
            .iter()
            .any(|building| building.zone_type == ZoneType::Industrial)
    );
}

// Loading rebuilds the entrance cache, the site clients and the zone index at once; the first
// building appended after it must not copy them (`SIM-02-K`).
#[test]
fn rebuilt_building_tables_keep_spare_rows_for_the_next_building() {
    let mut core = test_core();
    add_test_border_road(&mut core);
    let home = register_test_asset(&mut core.allocator, "spare_home", ZoneType::Residential);
    for _ in 0..16 {
        add_test_complete_building(&mut core, home.clone(), ZoneType::Residential);
    }
    let allocator = &mut core.allocator;
    allocator.rebuild_entrance_cache(&core.region_graph, &core.transit_network.lane_system);
    allocator.rebuild_building_site_clients(core.config.zone_cell_m);
    allocator.rebuild_zone_index();
    let len = allocator.buildings.len();
    assert_eq!(allocator.entrances.len(), len);
    assert_eq!(allocator.building_sites.len(), len);
    assert_eq!(allocator.vacancy_pos.len(), len);
    assert!(allocator.entrances.capacity() > len);
    assert!(allocator.building_sites.capacity() > len);
    assert!(allocator.vacancy_pos.capacity() > len);
}

#[test]
fn hourly_planning_does_not_queue_the_same_parcel_twice() {
    let mut core = test_core();
    add_test_border_road(&mut core);
    register_test_asset(
        &mut core.allocator,
        "queued_residential",
        ZoneType::Residential,
    );
    register_test_asset(
        &mut core.allocator,
        "queued_industrial",
        ZoneType::Industrial,
    );
    place_test_parcel_run(&mut core, ZoneType::Residential, 10.0, 50.0);
    place_test_parcel_run(&mut core, ZoneType::Industrial, 110.0, 150.0);
    core.apply_money_and_max_demand_cheat(1_000_000.0);
    let hour = next_hour_slot(&core);
    core.run_slots_through_for_test(hour + PlanningSlot::Actions as u64);
    assert!(!core.pending_demand_spawns.is_empty());
    // Keep the first batch queued while another hour plans. Other parcels may still grow.
    for pending in &mut core.pending_demand_spawns {
        pending.due_minute += 120;
    }
    core.reschedule_construction_events();
    core.run_slots_through_for_test(hour + SLOTS_PER_HOUR + PlanningSlot::Actions as u64);
    let mut parcels: Vec<_> = core
        .pending_demand_spawns
        .iter()
        .map(|pending| pending.action.parcel_id)
        .collect();
    parcels.sort_unstable();
    assert!(parcels.windows(2).all(|pair| pair[0] != pair[1]));
}

#[test]
fn hourly_passes_read_the_previous_hours_demand_sweep() {
    let mut core = test_core();
    add_test_border_road(&mut core);
    let home = register_test_asset(&mut core.allocator, "swept_home", ZoneType::Residential);
    for _ in 0..2 {
        add_test_complete_building(&mut core, home.clone(), ZoneType::Residential);
    }
    let capacity = core.allocator.household_capacity(0);
    assert_eq!(core.allocator.household_capacity(1), capacity);
    let vacant = |core: &SimCore| {
        core.demand
            .last_admission_debug_summary()
            .vacant_household_slots
    };
    core.align_events_to_clock();
    let mut slot = core.time.absolute_slot();
    let hour = slot.div_ceil(SLOTS_PER_HOUR) * SLOTS_PER_HOUR;
    let mut report = super::frame::FrameStepReport::default();
    let mut step_to = |core: &mut SimCore, end: u64| {
        while slot < end {
            slot += 1;
            core.step_slot(slot, &mut report, &mut ());
        }
    };
    // The hour's admission is planned at its actions slot, slot 3 (`SIM-02-I`).
    let actions = PlanningSlot::Actions as u64;
    // The first pass has no front and builds one from the city.
    step_to(&mut core, hour + actions);
    assert_eq!(vacant(&core), 2 * capacity);
    // A home that fails after its chunk of the sweep counts until the next hour's sweep.
    step_to(&mut core, hour + 600);
    core.allocator.buildings[0].broken = true;
    step_to(&mut core, hour + SLOTS_PER_HOUR + actions);
    assert_eq!(vacant(&core), 2 * capacity);
    step_to(&mut core, hour + 2 * SLOTS_PER_HOUR + actions);
    assert_eq!(vacant(&core), capacity);
}

fn construction_execution_fixture() -> (SimCore, Vec<DemandSpawnAction>, Vec<DemandSpawnAction>) {
    let mut core = test_core();
    add_test_border_road(&mut core);
    let home = register_test_asset(&mut core.allocator, "execution_home", ZoneType::Residential);
    let factory = register_test_asset(
        &mut core.allocator,
        "execution_factory",
        ZoneType::Industrial,
    );
    place_test_parcel_run(&mut core, ZoneType::Residential, 10.0, 90.0);
    place_test_parcel_run(&mut core, ZoneType::Industrial, 110.0, 170.0);
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    let mut homes = Vec::new();
    let mut factories = Vec::new();
    for parcel in core.zoning.parcels() {
        let zone = core
            .zoning
            .profiles
            .profile_by_runtime_id(parcel.zone_profile_runtime_id())
            .unwrap()
            .zone_type;
        let (actions, asset) = if zone == ZoneType::Residential {
            (&mut homes, &home)
        } else {
            (&mut factories, &factory)
        };
        actions.push(DemandSpawnAction {
            parcel_id: parcel.id().raw(),
            asset_id: asset.clone(),
        });
    }
    assert!(homes.len() >= 2 && factories.len() >= 2);
    (core, homes, factories)
}

fn execute_test_construction(
    core: &mut SimCore,
    action: &DemandSpawnAction,
    zone: ZoneType,
    minute: u16,
) {
    queue_test_spawn(core, u64::from(minute), zone, action);
    // A planning slot proves a business spawn's permit before its start (`SIM-02-I`).
    if zone != ZoneType::Residential {
        core.prove_construction_permits_for_test(1, minute - 1);
    }
    assert_eq!(core.execute_pending_demand_spawns_for_minute(1, minute), 1);
}

#[test]
fn completed_homes_rehouse_existing_residents_before_hourly_arrivals() {
    use crate::simulation::economy::agents::{AGE_ADULT, HouseholdAgeComposition};
    use crate::simulation::economy::definitions::load_runtime_economy_tuning;

    let (mut core, homes, _) = construction_execution_fixture();
    execute_test_construction(&mut core, &homes[0], ZoneType::Residential, 1);
    let slots = core
        .allocator
        .registry
        .household_capacity(&homes[0].asset_id);
    core.allocator.buildings[0].set_construction_hours_left(1);
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    for _ in 0..slots {
        let household = core.households.admit_immigrant_household(
            &catalog,
            &tuning,
            usize::MAX,
            1,
            HouseholdAgeComposition {
                adult_count: 1,
                ..Default::default()
            },
        );
        // Existing displaced residents have savings; immigrant seed cash alone need not
        // meet the ordinary relocation reserve requirement.
        core.households.households[household].budget = 1_000.0;
        let agent = core
            .agents
            .spawn_housed_agent_with_age_group(usize::MAX, 0.0, 0.0, AGE_ADULT);
        core.agents.assign_household_id(agent, household);
    }
    // The home completes at its slot, before the hour whose phases rehouse its residents.
    core.allocator.complete_construction(0);
    let hour = next_hour_slot(&core);
    core.run_slots_through_for_test(hour + SLOTS_PER_HOUR - 1);
    assert!(!core.allocator.buildings[0].is_under_construction());
    assert_eq!(core.allocator.buildings[0].occupancy, slots);
    assert!(
        core.households
            .households
            .iter()
            .all(|household| household.home_building_id == 0)
    );
    assert!(core.agents.home_building.iter().all(|&home| home == 0));
    assert_eq!(core.households.pending_arrivals.households, 0);
}

#[test]
fn future_homes_rehouse_nonworkers_before_funding_factory_construction() {
    use crate::simulation::economy::agents::HouseholdAgeComposition;
    use crate::simulation::economy::definitions::load_runtime_economy_tuning;

    let (mut core, homes, factories) = construction_execution_fixture();
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    for _ in 0..4 {
        let household = core.households.admit_immigrant_household(
            &catalog,
            &tuning,
            usize::MAX,
            1,
            HouseholdAgeComposition {
                elder_count: 1,
                ..Default::default()
            },
        );
        core.households.households[household].budget = 10_000.0;
    }
    execute_test_construction(&mut core, &homes[0], ZoneType::Residential, 1);
    execute_test_construction(&mut core, &homes[1], ZoneType::Residential, 2);
    execute_test_construction(&mut core, &factories[0], ZoneType::Industrial, 3);
    assert_eq!(
        core.allocator.buildings.len(),
        2,
        "four displaced nonworker households leave only eight prospective workers"
    );
    assert!(
        core.allocator
            .buildings
            .iter()
            .all(|building| building.operating_budget == 0.0)
    );
    execute_test_construction(&mut core, &homes[2], ZoneType::Residential, 4);
    execute_test_construction(&mut core, &factories[0], ZoneType::Industrial, 5);
    assert_eq!(core.allocator.buildings.len(), 4);
    assert_eq!(core.allocator.buildings[3].worker_count, 0);
    assert!(core.allocator.buildings[3].operating_budget > 0.0);
}

// Homes and factories queued in one batch: the planning slot's permits count the queued
// homes' applicants as the planner does, and the second factory cannot reuse the first one's
// workers; the next slot replays the placed factory as construction (`SIM-02-I`).
#[test]
fn queued_customers_count_queued_homes_and_reserve_labour_once() {
    let (mut core, homes, factories) = construction_execution_fixture();
    let queue = |core: &mut SimCore, spawns: &[(u64, ZoneType, &DemandSpawnAction)]| {
        for &(due_minute, zone_type, action) in spawns {
            core.pending_demand_spawns
                .push_back(super::state::PendingDemandSpawnAction {
                    id: 0,
                    due_minute,
                    zone_type,
                    action: action.clone(),
                    planned_day_index: 1,
                    planned_minute_of_day: 0,
                    permit: Default::default(),
                    counted: false,
                });
        }
    };
    queue(
        &mut core,
        &[
            (1, ZoneType::Residential, &homes[0]),
            (2, ZoneType::Residential, &homes[1]),
            (3, ZoneType::Industrial, &factories[0]),
            (4, ZoneType::Industrial, &factories[1]),
        ],
    );
    core.prove_construction_permits_for_test(1, 0);
    let permits: Vec<_> = core
        .pending_demand_spawns
        .iter()
        .map(|pending| pending.permit.granted().is_some())
        .collect();
    assert_eq!(
        permits,
        [false, false, true, false],
        "queued homes back the first buyer, whose workers the second cannot reuse"
    );
    for minute in 1..=4 {
        assert_eq!(core.execute_pending_demand_spawns_for_minute(1, minute), 1);
    }
    assert_eq!(core.allocator.buildings.len(), 3);
    assert!(core.allocator.buildings[2].operating_budget > 0.0);
    queue(&mut core, &[(5, ZoneType::Industrial, &factories[1])]);
    core.prove_construction_permits_for_test(1, 4);
    assert!(
        core.pending_demand_spawns[0].permit.granted().is_none(),
        "placed construction keeps its labour after its queue entry retires"
    );
}

#[test]
fn queued_customer_permits_survive_a_load_before_their_start() {
    let mut outcomes = Vec::new();
    for reload in [false, true] {
        let (mut core, homes, factories) = construction_execution_fixture();
        execute_test_construction(&mut core, &homes[0], ZoneType::Residential, 1);
        execute_test_construction(&mut core, &homes[1], ZoneType::Residential, 2);
        for (index, action) in factories.iter().take(2).enumerate() {
            core.pending_demand_spawns
                .push_back(super::state::PendingDemandSpawnAction {
                    id: 0,
                    due_minute: 3 + index as u64,
                    zone_type: ZoneType::Industrial,
                    action: action.clone(),
                    planned_day_index: 1,
                    planned_minute_of_day: 0,
                    permit: Default::default(),
                    counted: false,
                });
        }
        core.time.minute_of_day = 2;
        core.prove_construction_permits_for_test(1, 2);
        // Queue priority: the earlier buyer holds the workers before either has started.
        assert!(core.pending_demand_spawns[0].permit.granted().is_some());
        assert!(core.pending_demand_spawns[1].permit.granted().is_none());
        if reload {
            let path = temp_save_path("queued_customer_reservations");
            core.save_game_internal(path.to_str().unwrap(), None)
                .unwrap();
            let (mut loaded, _, _) = construction_execution_fixture();
            loaded.load_game_internal(path.to_str().unwrap()).unwrap();
            std::fs::remove_file(path).unwrap();
            core = loaded;
        }
        assert_eq!(core.execute_pending_demand_spawns_for_minute(1, 3), 1);
        assert_eq!(core.execute_pending_demand_spawns_for_minute(1, 4), 1);
        assert!(core.pending_demand_spawns.is_empty());
        assert_eq!(core.allocator.buildings.len(), 3);
        let firm = &core.allocator.buildings[2];
        outcomes.push((
            firm.parcel_id,
            firm.asset_id.clone(),
            firm.operating_budget.to_bits(),
            firm.construction_complete_slot,
            core.treasury.balance.to_bits(),
        ));
    }
    assert_eq!(outcomes[0], outcomes[1]);
}

#[test]
// Permits are proved at the planning slot, before any edit. A start places only with a
// permit for its current door: an edit that moves a queued factory's door rejects it, a price
// change alone does not, and a load keeps every permit and decision (`SIM-02-I`).
fn queued_projects_start_only_at_their_permitted_doors_through_placement_and_reload() {
    for edit in [
        "unchanged",
        "future-door",
        "frontage-policy",
        "frontage-recovery",
        "road-split",
    ] {
        let mut outcomes = Vec::new();
        for reload in [false, true] {
            let (mut core, mut homes, mut factories) = construction_execution_fixture();
            core.transit_network
                .rebuild_cch_and_check(&core.region_graph);
            assert!(homes.len() >= 4 && factories.len() >= 3);
            factories[1].asset_id = register_test_asset(
                &mut core.allocator,
                "execution_middle_factory",
                ZoneType::Industrial,
            );
            if edit == "road-split" {
                use crate::simulation::zoning::cells::{CellBounds, CellSelectionShape};
                use glam::DVec2;
                // The numerical fixture's manual parcels keep authored attachments. Use
                // gameplay cell lots for the road-edit attachment reconciliation contract.
                core.benchmark_mode = false;
                let home_asset = homes[0].asset_id.clone();
                let factory_asset = factories[0].asset_id.clone();
                let middle_asset = factories[1].asset_id.clone();
                let removed = homes
                    .iter()
                    .chain(&factories)
                    .map(|action| action.parcel_id)
                    .collect();
                assert_eq!(
                    core.zoning.remove_parcels_by_raw_ids(&removed),
                    homes.len() + factories.len()
                );
                core.zoning.generate_cells(
                    &core.region_graph,
                    CellBounds {
                        min: DVec2::new(0.0, -128.0),
                        max: DVec2::new(256.0, 128.0),
                    },
                    |_| false,
                );
                for (zone, start, end) in [
                    (ZoneType::Residential, 10.0, 90.0),
                    (ZoneType::Industrial, 110.0, 179.0),
                ] {
                    let selection = core.zoning.cells.select(
                        CellSelectionShape::Marquee,
                        &[DVec2::new(start, -6.0), DVec2::new(end, -35.0)],
                    );
                    assert!(!selection.cells.is_empty());
                    let profile = core
                        .zoning
                        .profiles
                        .default_runtime_id_for_zone_type(zone)
                        .unwrap();
                    core.zoning
                        .paint_cells(&selection, profile, |_| false)
                        .unwrap();
                }
                core.prepare_cell_lots_internal();
                homes.clear();
                factories.clear();
                for parcel in core.zoning.parcels() {
                    assert!(parcel.cell_lot().is_some());
                    let residential = core
                        .zoning
                        .profiles
                        .profile_by_runtime_id(parcel.zone_profile_runtime_id())
                        .unwrap()
                        .zone_type
                        == ZoneType::Residential;
                    let (actions, asset) = if residential {
                        (&mut homes, &home_asset)
                    } else {
                        (&mut factories, &factory_asset)
                    };
                    actions.push(DemandSpawnAction {
                        parcel_id: parcel.id().raw(),
                        asset_id: asset.clone(),
                    });
                }
                assert_eq!(homes.len(), 4);
                assert_eq!(factories.len(), 3);
                factories[1].asset_id = middle_asset;
            }
            // Applicants for these three homes' places bring 24 adults (`DEM-02-C`).
            for (index, home) in homes.iter().take(3).enumerate() {
                execute_test_construction(&mut core, home, ZoneType::Residential, index as u16 + 1);
            }
            for (index, action) in factories.iter().take(3).enumerate() {
                core.pending_demand_spawns
                    .push_back(super::state::PendingDemandSpawnAction {
                        id: 0,
                        due_minute: 5 + index as u64,
                        zone_type: ZoneType::Industrial,
                        action: action.clone(),
                        planned_day_index: 1,
                        planned_minute_of_day: 0,
                        permit: Default::default(),
                        counted: false,
                    });
            }
            core.prove_construction_permits_for_test(1, 4);
            let permits: Vec<_> = core
                .pending_demand_spawns
                .iter()
                .map(|pending| pending.permit.granted())
                .collect();
            assert!(permits[0].is_some() && permits[1].is_some() && permits[2].is_none());
            if edit == "future-door" {
                // Only the second project changes; the warm queue still holds its old door.
                let mut manifest = core
                    .allocator
                    .registry
                    .get(&factories[1].asset_id)
                    .unwrap()
                    .manifest
                    .clone();
                manifest.anchors.clear();
                core.allocator
                    .registry
                    .register("test", manifest, String::new());
            } else if edit == "frontage-policy" {
                let before = core.allocator.demand_site_entrance(
                    factories[1].parcel_id,
                    &factories[1].asset_id,
                    &core.zoning,
                    &core.region_graph,
                    &core.transit_network.lane_system,
                );
                core.set_vehicle_frontage_access_internal(0, 0);
                assert_ne!(
                    before,
                    core.allocator.demand_site_entrance(
                        factories[1].parcel_id,
                        &factories[1].asset_id,
                        &core.zoning,
                        &core.region_graph,
                        &core.transit_network.lane_system
                    )
                );
            } else if edit == "road-split" {
                let before = core
                    .zoning
                    .parcel_by_raw_id(factories[1].parcel_id)
                    .unwrap()
                    .edge_idx();
                road_terrain_plan::commit_ready(
                    &mut core,
                    vec![
                        Vector3::new(100.0, 0.0, 0.0),
                        Vector3::new(100.0, 0.0, 90.0),
                    ],
                );
                core.transit_network
                    .lane_system
                    .rebuild(&mut core.region_graph);
                core.transit_network
                    .rebuild_cch_and_check(&core.region_graph);
                core.rebuild_building_entrances_internal();
                assert_ne!(
                    core.zoning
                        .parcel_by_raw_id(factories[1].parcel_id)
                        .unwrap()
                        .edge_idx(),
                    before,
                    "the actual road edit must remap the future project's frontage"
                );
            }
            // A permitted factory whose door the edit left unchanged places.
            let expected: Vec<_> = factories
                .iter()
                .take(3)
                .zip(&permits)
                .filter(|(action, permit)| {
                    permit.is_some_and(|permit| {
                        let entrance = core.allocator.demand_site_entrance(
                            action.parcel_id,
                            &action.asset_id,
                            &core.zoning,
                            &core.region_graph,
                            &core.transit_network.lane_system,
                        );
                        crate::simulation::economy::demand::door_digest(
                            entrance.as_ref(),
                            &core.region_graph,
                            &core.transit_network.lane_system,
                        ) == permit.door
                    })
                })
                .map(|(action, _)| action.parcel_id)
                .collect();
            match edit {
                "unchanged" | "frontage-recovery" => assert_eq!(
                    expected,
                    [factories[0].parcel_id, factories[1].parcel_id],
                    "{edit}: prices alone leave the permitted doors"
                ),
                _ => assert!(
                    !expected.contains(&factories[1].parcel_id),
                    "{edit}: the edit moves the second factory's door"
                ),
            }
            let mut sequence = Vec::new();
            for minute in 5..=7 {
                if edit == "frontage-recovery" && minute >= 6 {
                    for lane in &mut core.transit_network.lane_system.lanes {
                        lane.frontage_delay_penalty_s = if minute == 6 { 1_000_000.0 } else { 0.0 };
                    }
                    core.transit_network.frontage_delay_revision += 1;
                }
                core.time.day_index = 1;
                core.time.minute_of_day = minute - 1;
                if reload {
                    // Loading reconstructs every queue door and financial book from current
                    // inputs. Compare that fresh census with ordinary in-order warm execution.
                    let path = temp_save_path("queued_edits_and_recovery");
                    core.save_game_internal(path.to_str().unwrap(), None)
                        .unwrap();
                    let mut loaded = test_core();
                    loaded.allocator.registry = core.allocator.registry.clone();
                    loaded.load_game_internal(path.to_str().unwrap()).unwrap();
                    std::fs::remove_file(path).unwrap();
                    assert_eq!(
                        loaded.allocator.buildings.len(),
                        core.allocator.buildings.len(),
                        "{edit}, minute={minute}: valid supporting homes and placed firms must survive reload"
                    );
                    core = loaded;
                }
                assert_eq!(core.execute_pending_demand_spawns_for_minute(1, minute), 1);
                sequence.push((
                    core.allocator
                        .buildings
                        .iter()
                        .map(|building| {
                            (
                                building.parcel_id,
                                building.asset_id.clone(),
                                building.operating_budget.to_bits(),
                                building.construction_complete_slot,
                                building.worker_count,
                            )
                        })
                        .collect::<Vec<_>>(),
                    core.treasury.balance.to_bits(),
                ));
            }
            assert!(core.pending_demand_spawns.is_empty());
            let firms: Vec<_> = core
                .allocator
                .buildings
                .iter()
                .filter(|building| building.zone_type == ZoneType::Industrial)
                .collect();
            assert_eq!(
                firms.iter().map(|firm| firm.parcel_id).collect::<Vec<_>>(),
                expected,
                "{edit}, reload={reload}"
            );
            let profile = core
                .demand
                .runtime_catalog()
                .profile_for_id("food_processor_basic")
                .unwrap();
            let capital = profile.zoned_startup_budget(
                core.demand.runtime_catalog(),
                core.demand.runtime_tuning().owa_import_price_multiplier,
            );
            assert!(firms.iter().all(|firm| firm.operating_budget == capital
                && firm.worker_count == 0
                && firm.is_under_construction()));
            outcomes.push(sequence);
        }
        assert_eq!(
            outcomes[0], outcomes[1],
            "{edit}: reload must preserve each placement and equity decision"
        );
    }
}

#[test]
fn failed_supporting_housing_prevents_owner_funding_and_labor_is_not_reused() {
    let (mut core, homes, factories) = construction_execution_fixture();
    let mut failed_home = homes[1].clone();
    failed_home.asset_id = "missing:home".to_owned();
    execute_test_construction(&mut core, &failed_home, ZoneType::Residential, 1);
    execute_test_construction(&mut core, &homes[0], ZoneType::Residential, 2);
    assert_eq!(core.allocator.buildings.len(), 1);
    execute_test_construction(&mut core, &factories[0], ZoneType::Industrial, 3);
    assert_eq!(
        core.allocator.buildings.len(),
        1,
        "six housed workers cannot staff ten jobs"
    );
    assert_eq!(
        core.allocator
            .buildings
            .iter()
            .map(|b| b.operating_budget)
            .sum::<f32>(),
        0.0
    );

    execute_test_construction(&mut core, &homes[1], ZoneType::Residential, 4);
    execute_test_construction(&mut core, &factories[0], ZoneType::Industrial, 5);
    assert_eq!(core.allocator.buildings.len(), 3);
    execute_test_construction(&mut core, &factories[1], ZoneType::Industrial, 6);
    let industry: Vec<_> = core
        .allocator
        .buildings
        .iter()
        .filter(|b| b.zone_type == ZoneType::Industrial)
        .collect();
    assert_eq!(
        industry.len(),
        1,
        "a second factory cannot reuse the first factory's workforce"
    );
    let catalog = core.demand.runtime_catalog();
    let profile = catalog.profile_for_id("food_processor_basic").unwrap();
    assert_eq!(
        industry[0].operating_budget,
        profile.zoned_startup_budget(
            catalog,
            core.demand.runtime_tuning().owa_import_price_multiplier
        )
    );
}

// Queues `action` as the hour's planning pass would, with a fresh spawn id.
fn queue_test_spawn(
    core: &mut SimCore,
    due_minute: u64,
    zone_type: ZoneType,
    action: &DemandSpawnAction,
) {
    let id = core.next_spawn_id;
    core.next_spawn_id += 1;
    core.pending_demand_spawns
        .push_back(super::state::PendingDemandSpawnAction {
            id,
            due_minute,
            zone_type,
            action: action.clone(),
            planned_day_index: 1,
            planned_minute_of_day: 0,
            permit: Default::default(),
            counted: false,
        });
}

// A home that fails to place voids the queued business permits, which counted its applicants:
// the factory it backed waits for the next planning slot, which proves it again on the city
// without that home, instead of spending its equity on workers that will not come
// (`SIM-02-I`).
#[test]
fn a_failed_home_voids_the_business_permits_its_applicants_backed() {
    use crate::simulation::economy::demand::SpawnPermit;
    let (mut core, homes, factories) = construction_execution_fixture();
    queue_test_spawn(&mut core, 1, ZoneType::Residential, &homes[0]);
    queue_test_spawn(&mut core, 2, ZoneType::Residential, &homes[1]);
    queue_test_spawn(&mut core, 3, ZoneType::Industrial, &factories[0]);
    core.prove_construction_permits_for_test(1, 0);
    assert!(core.pending_demand_spawns[2].permit.granted().is_some());
    // The second home's lot is gone by its start.
    let removed = std::iter::once(homes[1].parcel_id).collect();
    assert_eq!(core.zoning.remove_parcels_by_raw_ids(&removed), 1);
    for minute in 1..=2 {
        assert_eq!(core.execute_pending_demand_spawns_for_minute(1, minute), 1);
    }
    assert_eq!(core.pending_demand_spawns[0].permit, SpawnPermit::Voided);
    assert_eq!(
        core.execute_pending_demand_spawns_for_minute(1, 3),
        0,
        "the factory waits for the next planning slot"
    );
    assert_eq!(core.pending_demand_spawns.len(), 1);
    assert_eq!(core.pending_demand_spawns[0].due_minute, 61);
    assert_eq!(core.allocator.buildings.len(), 1);
    // The next planning slot proves it again on the one placed home, which cannot staff it.
    core.prove_construction_permits_for_test(1, 60);
    assert_eq!(core.pending_demand_spawns[0].permit, SpawnPermit::Denied);
    assert_eq!(core.execute_pending_demand_spawns_for_minute(1, 61), 1);
    assert_eq!(core.allocator.buildings.len(), 1);
    assert_eq!(core.allocator.buildings[0].zone_type, ZoneType::Residential);
}

// Every home's applicant stream is seeded by a stable identity (`SIM-02-I`): a queued home by
// its spawn id, which it keeps once placed, so planning and the planning slot's permits count
// the applicants it draws once placed, as the workforce calendar counts a placed home's. An
// earlier spawn that fails to place shifts no other home's seed.
#[test]
fn queued_homes_estimate_the_applicants_they_draw_once_placed() {
    use crate::simulation::economy::households::applicant_places_and_members;
    let (mut core, homes, factories) = construction_execution_fixture();
    assert!(homes.len() >= 4);
    let queue = [
        (ZoneType::Residential, &homes[0]),
        (ZoneType::Residential, &homes[1]),
        (ZoneType::Residential, &homes[2]),
        (ZoneType::Residential, &homes[3]),
        (ZoneType::Industrial, &factories[0]),
    ];
    for (minute, &(zone_type, action)) in queue.iter().enumerate() {
        queue_test_spawn(&mut core, minute as u64 + 1, zone_type, action);
    }
    let ids: Vec<_> = core
        .pending_demand_spawns
        .iter()
        .map(|pending| pending.id)
        .collect();
    core.prove_construction_permits_for_test(1, 0);
    let estimates: Vec<_> = [0, 1, 2, 3]
        .map(|position| {
            core.demand
                .queued_home_estimate_for_test(ids[position], queue[position].1.parcel_id)
                .unwrap()
        })
        .into();
    // The second home's lot is gone by its start: the later homes keep their seeds.
    let removed = std::iter::once(homes[1].parcel_id).collect();
    assert_eq!(core.zoning.remove_parcels_by_raw_ids(&removed), 1);
    for minute in 1..=queue.len() as u16 {
        core.execute_pending_demand_spawns_for_minute(1, minute);
    }
    for (position, (seed, workers, members)) in [0, 1, 2, 3].into_iter().zip(estimates) {
        let home = queue[position].1;
        assert_eq!(seed, ids[position] as usize);
        let Some(placed) = core
            .allocator
            .buildings
            .iter()
            .position(|building| building.parcel_id == home.parcel_id)
        else {
            assert_eq!(position, 1, "only the home without a lot fails to place");
            continue;
        };
        assert_eq!(
            core.allocator.buildings[placed].applicant_seed,
            ids[position]
        );
        let (places, drawn_members) = applicant_places_and_members(
            core.allocator.flat_size_m2(placed),
            core.allocator.applicant_seed(placed),
            0..core.allocator.registry.household_capacity(&home.asset_id),
        );
        assert_eq!(
            (workers, members),
            (places[1] + 2 * places[2], drawn_members)
        );
    }
}

#[test]
fn independent_factories_can_start_together_when_each_has_its_own_workforce() {
    let (mut core, homes, factories) = construction_execution_fixture();
    assert!(homes.len() >= 4);
    for (index, home) in homes.iter().take(4).enumerate() {
        execute_test_construction(&mut core, home, ZoneType::Residential, index as u16 + 1);
    }
    execute_test_construction(&mut core, &factories[0], ZoneType::Industrial, 5);
    execute_test_construction(&mut core, &factories[1], ZoneType::Industrial, 6);
    let industry: Vec<_> = core
        .allocator
        .buildings
        .iter()
        .filter(|building| building.zone_type == ZoneType::Industrial)
        .collect();
    assert_eq!(
        industry.len(),
        2,
        "the first unfinished factory cannot veto an independent entrant"
    );
    assert!(
        industry
            .iter()
            .all(|building| building.is_under_construction()
                && building.worker_count == 0
                && building.revenue == 0.0)
    );
}

#[test]
fn business_planning_prices_the_actual_freight_route() {
    for expensive in [false, true] {
        let (mut core, _, _) = construction_execution_fixture();
        // The hour's census start publishes the live delays as prices.
        if expensive {
            for lane in &mut core.transit_network.lane_system.lanes {
                lane.live_frontage_delay_s = 1_000_000.0;
            }
        }
        let hour = next_hour_slot(&core);
        core.run_slots_through_for_test(hour + PlanningSlot::Actions as u64);
        let factories = core
            .pending_demand_spawns
            .iter()
            .filter(|pending| pending.zone_type == ZoneType::Industrial)
            .count();
        if expensive {
            assert_eq!(
                factories, 0,
                "exports cannot fund the proposed site's freight bill"
            );
        } else {
            assert!(
                factories > 0,
                "a reachable, profitable site must still grow"
            );
        }
    }
}

// The planning slot proves each queued factory's permit with one census and epoch; a start
// checks only its permit against its current door, profile and level (`SIM-02-I`), so the
// warm check stays local however many buildings the city has.
#[test]
#[ignore = "manual release locality of queued-project start checks; setup and planning slot timed separately"]
fn benchmark_queued_project_validation() {
    use std::hint::black_box;
    use std::time::{Duration, Instant};

    const WARM_CHECKS: usize = 1_024;
    const PUBLISHED_CHECKS: usize = 16;
    for background in [0, 2_048, 16_384, 131_072] {
        let (mut core, homes, factories) = construction_execution_fixture();
        execute_test_construction(&mut core, &homes[0], ZoneType::Residential, 1);
        execute_test_construction(&mut core, &homes[1], ZoneType::Residential, 2);
        // Only the census consumes these background records. Do not run physical placement,
        // agent movement or indexed building queries on this deliberately isolated fixture.
        let background_home = core.allocator.buildings[0].clone();
        core.allocator
            .buildings
            .resize(background + 2, background_home);
        let balance = core.treasury.balance;
        let mut samples = [[0.0_f64; 3]; 9];
        for round in 0..12 {
            let start = Instant::now();
            let permits: Vec<_> = factories
                .iter()
                .map(|factory| {
                    black_box(core.permit_for_test(factory, ZoneType::Industrial, 1, 2))
                        .expect("each factory alone has the workers it needs")
                })
                .collect();
            let slot_ms = start.elapsed().as_secs_f64() * 1_000.0 / factories.len() as f64;
            let mut warm = Duration::ZERO;
            for index in 0..WARM_CHECKS {
                // Checks neither spend owner money nor claim workers or customers nor count the
                // city; a live revision changes no price a permit read.
                queued_check_live_revision(&mut core, index);
                let start = Instant::now();
                assert!(black_box(queued_permit_check(&core, &factories, &permits, index)).is_ok());
                warm += start.elapsed();
            }
            let mut published = Duration::ZERO;
            for index in 0..PUBLISHED_CHECKS {
                queued_check_live_revision(&mut core, index);
                core.transit_network.publish_frontage_prices();
                let start = Instant::now();
                assert!(black_box(queued_permit_check(&core, &factories, &permits, index)).is_ok());
                published += start.elapsed();
            }
            assert_eq!(core.treasury.balance, balance);
            assert_eq!(core.allocator.buildings.len(), background + 2);
            if round >= 3 {
                samples[round - 3] = [
                    slot_ms,
                    warm.as_secs_f64() * 1_000.0,
                    published.as_secs_f64() * 1_000.0 / PUBLISHED_CHECKS as f64,
                ];
            }
        }
        let median = |column| {
            let mut values = samples.map(|sample| sample[column]);
            values.sort_by(f64::total_cmp);
            values[4]
        };
        eprintln!(
            "queued_validation background={background} census_and_permit_ms={:.3} \
             warm_checks_after_live_revisions={WARM_CHECKS}_ms={:.3} \
             check_after_published_revision_ms={:.3}",
            median(0),
            median(1),
            median(2)
        );
    }
}

// One simulated second of the live delay cache from a check-specific congestion state: every
// vehicle lane changes, as in a busy city's per-second update.
fn queued_check_live_revision(core: &mut SimCore, index: usize) {
    for lane in &mut core.transit_network.lane_system.lanes {
        lane.live_frontage_delay_s = 1.0 + (index % 17) as f32;
    }
    core.agents
        .update_frontage_delay_cache(&mut core.transit_network, &core.region_graph, 1.0);
}

// The start's check of a queued factory's permit, cycling over `factories`.
fn queued_permit_check(
    core: &SimCore,
    factories: &[DemandSpawnAction],
    permits: &[crate::simulation::economy::demand::ConstructionPermit],
    index: usize,
) -> Result<(), crate::simulation::economy::demand::StartRefusal> {
    let index = index % factories.len();
    core.demand.check_construction_permit(
        ZoneType::Industrial,
        &factories[index],
        crate::simulation::economy::demand::SpawnPermit::Granted(permits[index]),
        &core.allocator,
        &core.zoning,
        &core.region_graph,
        &core.transit_network,
    )
}

#[test]
#[ignore = "short PERF-01 locality acceptance; setup and planning slot timed separately"]
fn perf_contract_queued_locality() {
    use std::hint::black_box;
    use std::time::Instant;

    // One fixed batch per tier: the former ~1 ms batch was dominated by timing noise.
    const WARM_CHECKS: usize = 131_072;
    let construction_epochs = crate::debug::work::Work::ConstructionEpochs as usize;
    let census_buildings = crate::debug::work::Work::CensusBuildings as usize;
    let mut results = Vec::new();

    for background in [16_384, 131_072] {
        let setup = Instant::now();
        let (mut core, homes, factories) = construction_execution_fixture();
        execute_test_construction(&mut core, &homes[0], ZoneType::Residential, 1);
        execute_test_construction(&mut core, &homes[1], ZoneType::Residential, 2);
        let background_home = core.allocator.buildings[0].clone();
        core.allocator
            .buildings
            .resize(background + 2, background_home);
        let setup_ms = setup.elapsed().as_secs_f64() * 1_000.0;

        // The planning slot: one census and one epoch prove the permit.
        let cold = Instant::now();
        let permits: Vec<_> = factories
            .iter()
            .map(|factory| {
                black_box(core.permit_for_test(factory, ZoneType::Industrial, 1, 2))
                    .expect("each factory alone has the workers it needs")
            })
            .collect();
        let cold_ms = cold.elapsed().as_secs_f64() * 1_000.0 / factories.len() as f64;

        let revision = core.transit_network.frontage_delay_revision;
        queued_check_live_revision(&mut core, 0);
        core.transit_network.publish_frontage_prices();
        assert_ne!(
            core.transit_network.frontage_delay_revision, revision,
            "the locality check must exercise a published frontage revision"
        );
        let published = Instant::now();
        assert!(black_box(queued_permit_check(&core, &factories, &permits, 0)).is_ok());
        let published_ms = published.elapsed().as_secs_f64() * 1_000.0;

        for index in 0..1_024 {
            queued_check_live_revision(&mut core, index);
            assert!(black_box(queued_permit_check(&core, &factories, &permits, index)).is_ok());
        }
        let before = crate::debug::work::snapshot();
        let mut warm = std::time::Duration::ZERO;
        for index in 0..WARM_CHECKS {
            queued_check_live_revision(&mut core, index);
            let started = Instant::now();
            let result = black_box(queued_permit_check(&core, &factories, &permits, index));
            warm += started.elapsed();
            assert!(result.is_ok());
        }
        let warm_batch_ms = warm.as_secs_f64() * 1_000.0;
        let after = crate::debug::work::snapshot();
        assert_eq!(
            after[construction_epochs], before[construction_epochs],
            "warm queued checks opened a construction epoch"
        );
        assert_eq!(
            after[census_buildings], before[census_buildings],
            "warm queued checks counted census rows"
        );
        results.push(serde_json::json!({
            "background": background,
            "setup_ms": setup_ms,
            "cold_ms": cold_ms,
            "published_revision_ms": published_ms,
            "warm_checks": WARM_CHECKS,
            "warm_batch_ms": warm_batch_ms,
            "warm_check_ms": warm_batch_ms / WARM_CHECKS as f64,
        }));
    }

    let small = results[0]["warm_batch_ms"].as_f64().unwrap();
    let large = results[1]["warm_batch_ms"].as_f64().unwrap();
    eprintln!(
        "PERF_LOCALITY {}",
        serde_json::json!({ "kind": "queued", "results": results })
    );
    assert!(
        large <= small * 1.2,
        "warm queued locality exceeded PERF-01 bound: 16k={small:.3} ms, 131k={large:.3} ms"
    );
}

// Planning slots price published frontage delays only: per-second congestion leaves a queued
// factory's permit unchanged until the operational hour publishes it (`SIM-02-I`).
#[test]
fn live_frontage_delay_reaches_queued_checks_only_when_published() {
    let (mut core, homes, factories) = construction_execution_fixture();
    execute_test_construction(&mut core, &homes[0], ZoneType::Residential, 1);
    execute_test_construction(&mut core, &homes[1], ZoneType::Residential, 2);
    let permit =
        |core: &mut SimCore| core.permit_for_test(&factories[0], ZoneType::Industrial, 1, 2);
    assert!(permit(&mut core).is_some());
    let revision = core.transit_network.frontage_delay_revision;
    // Per-second congestion anywhere, including the project's own frontage, leaves every
    // price-keyed cache and the permit unchanged until the operational hour publishes it.
    for lane in &mut core.transit_network.lane_system.lanes {
        lane.live_frontage_delay_s = 1_000_000.0;
    }
    core.agents
        .update_frontage_delay_cache(&mut core.transit_network, &core.region_graph, 1.0);
    assert_eq!(core.transit_network.frontage_delay_revision, revision);
    assert!(permit(&mut core).is_some());
    core.transit_network.publish_frontage_prices();
    assert_ne!(core.transit_network.frontage_delay_revision, revision);
    assert!(
        permit(&mut core).is_none(),
        "the market cannot fund the operation at the published delay"
    );
    // Publishing unchanged prices is not a revision.
    let revision = core.transit_network.frontage_delay_revision;
    core.transit_network.publish_frontage_prices();
    assert_eq!(core.transit_network.frontage_delay_revision, revision);
}

// Each planning slot reprices a queued factory's freight before its start can spend owner
// money: no permit at a price the market cannot fund, and the permitted door's carrier quote
// is the one live hiring and dispatch pay (`SIM-02-I`).
#[test]
fn queued_business_reprices_freight_before_owner_funding() {
    let (mut core, homes, factories) = construction_execution_fixture();
    execute_test_construction(&mut core, &homes[0], ZoneType::Residential, 1);
    execute_test_construction(&mut core, &homes[1], ZoneType::Residential, 2);
    let action = &factories[0];
    let permit = |core: &mut SimCore| core.permit_for_test(action, ZoneType::Industrial, 1, 2);
    assert!(permit(&mut core).is_some());
    let before = core.allocator.buildings.len();
    for lane in &mut core.transit_network.lane_system.lanes {
        lane.frontage_delay_penalty_s = 1_000_000.0;
    }
    core.transit_network.frontage_delay_revision += 1;
    assert!(permit(&mut core).is_none());
    execute_test_construction(&mut core, action, ZoneType::Industrial, 3);
    assert_eq!(core.allocator.buildings.len(), before);
    assert!(
        core.allocator
            .buildings
            .iter()
            .all(|b| b.operating_budget == 0.0)
    );
    for lane in &mut core.transit_network.lane_system.lanes {
        lane.frontage_delay_penalty_s = 0.0;
    }
    core.transit_network.frontage_delay_revision += 1;
    let edge = core
        .zoning
        .parcel_by_raw_id(action.parcel_id)
        .unwrap()
        .edge_idx();
    core.region_graph.edge_mut(edge).allowed_types = TransitFlags::FOOT;
    assert!(
        permit(&mut core).is_none(),
        "a gateway elsewhere cannot supply an entrance without vehicle access"
    );
    core.region_graph.edge_mut(edge).allowed_types = TransitFlags::FOOT | TransitFlags::CAR;
    let entrance = core
        .allocator
        .demand_site_entrance(
            action.parcel_id,
            &action.asset_id,
            &core.zoning,
            &core.region_graph,
            &core.transit_network.lane_system,
        )
        .unwrap();
    let borders = [0]; // add_test_border_road marks its first endpoint as the gateway.
    let quote = core
        .logistics
        .site_carrier_quote(
            &entrance,
            &core.transit_network,
            &core.region_graph,
            &borders,
            core.demand.runtime_tuning(),
        )
        .unwrap();
    execute_test_construction(&mut core, action, ZoneType::Industrial, 4);
    assert_eq!(core.allocator.buildings.len(), before + 1);
    let placed = &core.allocator.entrances[before];
    assert_eq!(entrance.door_pos, placed.door_pos);
    assert_eq!(entrance.car_lane_fwd, placed.car_lane_fwd);
    assert_eq!(entrance.car_lane_bkw, placed.car_lane_bkw);
    let mut quotes = vec![None; core.allocator.buildings.len()];
    core.logistics.write_border_carrier_quotes(
        &core.allocator,
        &core.transit_network,
        &core.region_graph,
        &borders,
        core.demand.runtime_catalog(),
        core.demand.runtime_tuning(),
        &mut quotes,
    );
    assert_eq!(
        quotes[before],
        Some(quote),
        "the proposed price must equal live hiring/dispatch"
    );
}

// A queued factory's permit stands for its planning slot's hour; the next slot proves it again
// on the city without the bulldozed home that backed it (`SIM-02-I`).
#[test]
fn construction_execution_revalidates_after_a_supporting_home_is_bulldozed() {
    let (mut core, homes, factories) = construction_execution_fixture();
    execute_test_construction(&mut core, &homes[0], ZoneType::Residential, 1);
    execute_test_construction(&mut core, &homes[1], ZoneType::Residential, 2);
    core.pending_demand_spawns
        .push_back(super::state::PendingDemandSpawnAction {
            id: 0,
            due_minute: 3,
            zone_type: ZoneType::Industrial,
            action: factories[0].clone(),
            planned_day_index: 1,
            planned_minute_of_day: 0,
            permit: Default::default(),
            counted: false,
        });
    core.prove_construction_permits_for_test(1, 2);
    assert!(core.pending_demand_spawns[0].permit.granted().is_some());
    assert!(core.allocator.remove_building_for_bulldoze(
        0,
        &mut core.zoning,
        &mut core.agents,
        &mut core.households,
        &mut core.logistics,
        &mut core.treasury.balance,
    ));
    core.prove_construction_permits_for_test(1, 2);
    assert!(core.pending_demand_spawns[0].permit.granted().is_none());
    assert_eq!(core.execute_pending_demand_spawns_for_minute(1, 3), 1);
    assert_eq!(core.allocator.buildings.len(), 1);
    assert_eq!(core.allocator.buildings[0].zone_type, ZoneType::Residential);
    execute_test_construction(&mut core, &homes[0], ZoneType::Residential, 4);
    execute_test_construction(&mut core, &factories[0], ZoneType::Industrial, 5);
    assert_eq!(
        core.allocator.buildings.len(),
        3,
        "replacement housing restores the independent opportunity"
    );
}

#[test]
fn pedestrian_lane_surface_height_matches_lane_semantics() {
    let sidewalk = Lane {
        edge_id: 7,
        lane_idx: 100,
        lane_type: LaneType::Foot,
        ..Lane::default()
    };
    assert_eq!(
        pedestrian_lane_surface_height(&sidewalk, 4.0),
        4.0 + CURB_STEP_HEIGHT_M
    );

    let crosswalk = Lane {
        edge_id: usize::MAX,
        lane_type: LaneType::Foot,
        ..Lane::default()
    };
    assert_eq!(pedestrian_lane_surface_height(&crosswalk, 4.0), 4.0);

    let footpath = Lane {
        edge_id: 7,
        lane_idx: 0,
        lane_type: LaneType::Foot,
        ..Lane::default()
    };
    assert_eq!(pedestrian_lane_surface_height(&footpath, 4.0), 4.0);
}
fn lane_change_snapshot_fixture(direction: f32) -> SimCore {
    use crate::simulation::network::lanes::{Lane, LaneType};
    let mut core = test_core();
    let length = Vector3::new(80.0, 8.0, 0.0).length();
    for lateral in [0.0, 3.5] {
        core.transit_network.lane_system.lanes.push(Lane {
            geometry: vec![
                Vector3::new(0.0, 0.0, direction * lateral),
                Vector3::new(direction * 80.0, 8.0, direction * lateral),
            ],
            length,
            cum_dist: vec![0.0, length],
            lane_type: LaneType::Vehicle,
            edge_id: 0,
            is_fwd: direction > 0.0,
            ..Default::default()
        });
    }
    core
}

#[test]
fn car_snapshot_keeps_lane_change_curve_in_both_directions() {
    use crate::simulation::economy::agents::{MODE_CAR, TRANSIT_NETWORK};

    for direction in [-1.0, 1.0] {
        let mut core = lane_change_snapshot_fixture(direction);
        let length = core.transit_network.lane_system.lanes[0].length;
        let id = core.agents.spawn_housed_agent(0, 0.0, 0.0);
        core.agents.transit[id] = TRANSIT_NETWORK;
        core.agents.transit_mode[id] = MODE_CAR;
        core.agents.current_lane_id[id] = 1;
        core.agents.lane_change_from_lane_id[id] = 0;
        core.agents.lane_change_start_d[id] = length * 0.125;
        core.agents.lane_change_length_m[id] = length * 0.5;
        for (fraction, lateral) in [(0.125, 0.0), (0.375, 1.75), (0.625, 3.5)] {
            core.agents.lane_distance[id] = length * fraction;
            let snapshot = core.build_snapshot();
            let (&key, pose) = snapshot.car_transforms.iter().next().unwrap();
            assert_eq!(pose.len(), 12);
            assert!((pose[3] - direction * 80.0 * fraction).abs() < 0.001);
            assert!((pose[7] - 8.0 * fraction - 0.02).abs() < 0.001);
            assert!((pose[11] - direction * lateral).abs() < 0.001);
            assert_eq!(snapshot.car_ground_flags[&key], [0]);
            if lateral == 1.75 {
                assert!(
                    pose[10] * direction < -0.01,
                    "car must turn into the lane change"
                );
            }
        }
        // A remapped/unrelated source lane must not bend a car away from its current road.
        core.transit_network.lane_system.lanes[0].edge_id = 1;
        core.agents.lane_distance[id] = length * 0.375;
        let snapshot = core.build_snapshot();
        let pose = snapshot.car_transforms.values().next().unwrap();
        assert!((pose[11] - direction * 3.5).abs() < 0.001);
    }
}

#[test]
fn culled_snapshot_matches_full_pose_through_lane_changes() {
    use crate::simulation::economy::agents::{MODE_CAR, TRANSIT_NETWORK};

    // Key, transform bits, render ids and ground flags of one car bucket.
    type CarBucket = (u8, Vec<u32>, Vec<i64>, Vec<u8>);

    // Car instances by bucket, as bits, ignoring empty recycled buckets.
    fn cars(snapshot: &RenderSnapshot) -> Vec<CarBucket> {
        let mut out: Vec<_> = snapshot
            .car_transforms
            .iter()
            .filter(|(_, transforms)| !transforms.is_empty())
            .map(|(&key, transforms)| {
                (
                    key,
                    transforms.iter().map(|v| v.to_bits()).collect(),
                    snapshot.car_render_ids[&key].clone(),
                    snapshot.car_ground_flags[&key].clone(),
                )
            })
            .collect();
        out.sort_by_key(|bucket| bucket.0);
        out
    }

    for direction in [-1.0_f32, 1.0] {
        let mut core = lane_change_snapshot_fixture(direction);
        let length = core.transit_network.lane_system.lanes[0].length;
        // Cars before, inside and after their lane-change blend, both ways between the lanes.
        for k in 0..240 {
            let id = core.agents.spawn_housed_agent(0, 0.0, 0.0);
            core.agents.transit[id] = TRANSIT_NETWORK;
            core.agents.transit_mode[id] = MODE_CAR;
            core.agents.current_lane_id[id] = k % 2;
            core.agents.lane_change_from_lane_id[id] = (1 - k % 2) as u32;
            core.agents.lane_change_start_d[id] = length * (k % 7) as f32 / 10.0;
            core.agents.lane_change_length_m[id] = length * 0.3;
            core.agents.lane_distance[id] = length * k as f32 / 240.0;
        }
        let mut culled = RenderSnapshot::default();
        let mut full = RenderSnapshot::default();
        let mut instances = 0;
        // Camera edges every 2.5 m along the road and across and beyond both lanes.
        for step in 0..34 {
            let x = direction * step as f32 * 2.5;
            for z in [-1.5, -0.5, 0.3, 1.75, 2.9, 3.6, 4.6] {
                let z = direction * z;
                for camera in [
                    (
                        x.min(x + direction * 30.0),
                        x.max(x + direction * 30.0),
                        -10.0,
                        z,
                    ),
                    (
                        x.min(x + direction * 30.0),
                        x.max(x + direction * 30.0),
                        z,
                        10.0,
                    ),
                    (-100.0, x, z.min(0.0), z.max(0.0)),
                ] {
                    core.camera_aabb = camera;
                    core.agent_instances_both_ways(&mut culled, &mut full);
                    assert_eq!(cars(&culled), cars(&full), "camera {camera:?}");
                    instances += culled.car_render_ids.values().map(Vec::len).sum::<usize>();
                }
            }
        }
        assert!(instances > 0);
    }
}

#[test]
#[ignore = "unprofiled lane-change snapshot measurement; run alone with --release --ignored --nocapture"]
fn lane_change_snapshot_benchmark() {
    use crate::simulation::economy::agents::{MODE_CAR, TRANSIT_NETWORK};

    for count in [100, 1_000, 10_000] {
        let mut core = lane_change_snapshot_fixture(1.0);
        let length = core.transit_network.lane_system.lanes[0].length;
        for _ in 0..count {
            let id = core.agents.spawn_housed_agent(0, 0.0, 0.0);
            core.agents.transit[id] = TRANSIT_NETWORK;
            core.agents.transit_mode[id] = MODE_CAR;
            core.agents.current_lane_id[id] = 1;
            core.agents.lane_distance[id] = length * 0.375;
            core.agents.lane_change_start_d[id] = length * 0.125;
            core.agents.lane_change_length_m[id] = length * 0.5;
        }
        for active in [false, true] {
            core.agents
                .lane_change_from_lane_id
                .fill(if active { 0 } else { u32::MAX });
            let mut snapshot = core.build_snapshot();
            for _ in 0..10 {
                snapshot = core.build_snapshot_reusing(snapshot);
            }
            let mut samples = Vec::with_capacity(100);
            for _ in 0..100 {
                let start = std::time::Instant::now();
                snapshot = std::hint::black_box(core.build_snapshot_reusing(snapshot));
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            samples.sort_by(f64::total_cmp);
            eprintln!(
                "lane_change_snapshot: cars={count} active={active} p50_ms={:.4} p90_ms={:.4}",
                samples[50], samples[90]
            );
        }
    }
}

#[test]
fn queued_buyers_and_supplier_replacement_survive_reload_at_every_step() {
    use crate::simulation::economy::agents::HouseholdAgeComposition;
    use crate::simulation::economy::definitions::load_runtime_economy_tuning;
    use crate::simulation::economy::demand::{DemandLevelChangeAction, demand_building_action_key};

    let mut outcomes = Vec::new();
    for reload in [false, true] {
        let (mut core, homes, factories) = construction_execution_fixture();
        core.transit_network
            .rebuild_cch_and_check(&core.region_graph);
        // The live processor gains a level-2 tier, so its replacement must revalidate the
        // queued groceries that buy its packaged food.
        let mut manifest = core
            .allocator
            .registry
            .get(&factories[0].asset_id)
            .unwrap()
            .manifest
            .clone();
        manifest.asset_set = Some("execution_processor".to_owned());
        core.allocator
            .registry
            .register("test", manifest.clone(), String::new());
        manifest.asset_id = "execution_factory_l2".to_owned();
        let tier = manifest.building.as_mut().unwrap();
        tier.level = 2;
        tier.worker_capacity = Some(8);
        core.allocator
            .registry
            .register("test", manifest, String::new());
        let target = "test:execution_factory_l2".to_owned();
        let shop = register_test_asset(&mut core.allocator, "execution_shop", ZoneType::Commercial);
        let commercial = core
            .zoning
            .profiles
            .default_runtime_id_for_zone_type(ZoneType::Commercial)
            .unwrap();
        core.zoning
            .place_parcel_run_at(
                10.0,
                20.0,
                170.0,
                20.0,
                commercial,
                20.0,
                30.0,
                0.0,
                &core.region_graph,
            )
            .unwrap();
        core.allocator
            .prepare_building_site_query_index(core.config.zone_cell_m);
        let shops: Vec<_> = core
            .zoning
            .parcels()
            .iter()
            .filter(|parcel| {
                core.zoning
                    .profiles
                    .profile_by_runtime_id(parcel.zone_profile_runtime_id())
                    .unwrap()
                    .zone_type
                    == ZoneType::Commercial
            })
            .map(|parcel| DemandSpawnAction {
                parcel_id: parcel.id().raw(),
                asset_id: shop.clone(),
            })
            .take(3)
            .collect();
        assert_eq!(shops.len(), 3);
        for (index, home) in homes.iter().take(4).enumerate() {
            execute_test_construction(&mut core, home, ZoneType::Residential, index as u16 + 1);
        }
        execute_test_construction(&mut core, &factories[0], ZoneType::Industrial, 5);
        let supplier = core.allocator.buildings.len() - 1;
        for building in &mut core.allocator.buildings {
            building.construction_complete_slot = 0;
        }
        core.allocator.buildings[supplier].worker_count = 4;
        core.allocator.buildings[supplier].operating_budget = 1_000_000.0;
        let catalog = load_runtime_economy_catalog().unwrap();
        let tuning = load_runtime_economy_tuning().unwrap();
        for home in 0..4 {
            for _ in 0..3 {
                let household = core.households.admit_immigrant_household(
                    &catalog,
                    &tuning,
                    home,
                    2,
                    HouseholdAgeComposition {
                        adult_count: 2,
                        ..Default::default()
                    },
                );
                let family = &mut core.households.households[household];
                family.budget = 10_000.0;
                family.gross_daily_wages = 600.0;
                family.employed_adults = 1;
                family.shopping_has_car = true;
                core.allocator.buildings[home].occupancy += 1;
            }
        }
        for (index, action) in shops.iter().enumerate() {
            core.pending_demand_spawns
                .push_back(super::state::PendingDemandSpawnAction {
                    id: 0,
                    due_minute: 8 + index as u64,
                    zone_type: ZoneType::Commercial,
                    action: action.clone(),
                    planned_day_index: 1,
                    planned_minute_of_day: 0,
                    permit: Default::default(),
                    counted: false,
                });
        }
        core.refresh_demand_construction_queue(1, 6);
        let mut sequence = Vec::new();
        for minute in 7..=10 {
            core.time.day_index = 1;
            core.time.minute_of_day = minute - 1;
            if reload {
                // Loading keeps every queued permit and rebuilds the supplier books cold from
                // persisted buildings, zoning and the queue.
                let path = temp_save_path("queued_supplier_replacement");
                core.save_game_internal(path.to_str().unwrap(), None)
                    .unwrap();
                let mut loaded = test_core();
                loaded.allocator.registry = core.allocator.registry.clone();
                loaded.load_game_internal(path.to_str().unwrap()).unwrap();
                std::fs::remove_file(path).unwrap();
                core = loaded;
            }
            // Minute 7 is a planning slot: it proves the queued groceries' permits and runs the
            // processor's replacement in the same epoch. Later minutes start on those permits.
            let mut proofs = Vec::new();
            if minute == 7 {
                let mut plan = DemandBuildingActionPlan::default();
                plan.industrial.upgrades.push(DemandLevelChangeAction {
                    building: demand_building_action_key(&core.allocator.buildings[supplier]),
                    target_asset_id: target.clone(),
                });
                proofs = core.run_planning_slot_actions_for_test(&plan, 1, minute);
                assert_eq!(
                    core.allocator.buildings[supplier].asset_id, target,
                    "reload={reload}"
                );
                let first = proofs[0]
                    .as_ref()
                    .expect("the slot's epoch proves the queued grocery");
                assert!(
                    first
                        .2
                        .iter()
                        .any(|&(_, source, ..)| source == Some(supplier)),
                    "the queued grocery's packaged food comes from the processor it replaces"
                );
            } else {
                assert_eq!(core.execute_pending_demand_spawns_for_minute(1, minute), 1);
            }
            let permits: Vec<_> = core
                .pending_demand_spawns
                .iter()
                .map(|pending| pending.permit.granted())
                .collect();
            if minute == 7 {
                // Each queued grocery the slot proved has its permit; the town funds one.
                assert!(
                    permits
                        .iter()
                        .zip(&proofs)
                        .all(|(permit, proof)| permit.is_some() == proof.is_some()),
                    "reload={reload}"
                );
            }
            sequence.push((
                core.allocator
                    .buildings
                    .iter()
                    .map(|building| {
                        (
                            building.parcel_id,
                            building.asset_id.clone(),
                            building.operating_budget.to_bits(),
                            building.construction_complete_slot,
                            building.worker_count,
                        )
                    })
                    .collect::<Vec<_>>(),
                core.treasury.balance.to_bits(),
                proofs,
                permits,
            ));
        }
        assert!(
            core.allocator
                .buildings
                .iter()
                .any(|building| building.zone_type == ZoneType::Commercial)
        );
        outcomes.push(sequence);
    }
    assert_eq!(
        outcomes[0], outcomes[1],
        "reload must preserve each replacement, placement and queued permit"
    );
}
