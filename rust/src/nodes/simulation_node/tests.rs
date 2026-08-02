//! Regression tests for the Godot simulation-node bridge.

use super::async_terrain::{
    TerrainPatchPayload, TerrainPatchPayloadAsyncState, TerrainPatchPayloadData,
    TerrainPatchPayloadKey, TerrainPatchPayloadRequest, WaterPatchPayloadAsyncState,
    WaterPatchPayloadKey,
};
use super::variant_export::{TerrainCdtSourceExport, TerrainCdtTriangleBufferExport};
#[cfg(test)]
use super::*;
use crate::simulation::agriculture::AgricultureSystem;
use crate::simulation::extraction::ResourceExtractionSystem;
use crate::simulation::resources::ResourceDepositSystem;
use crate::simulation::terrain::TerrainPatchSnapshot;
use crate::simulation::terrain::cdt::{
    TerrainCdtEarthworkSupportPolicy, TerrainCdtEdgeClass, TerrainCdtNodePieceKind,
    TerrainCdtRoadBandKind, TerrainCdtRoadLoopSourceEdge, TerrainCdtSpanRegionRole,
};

mod async_payload;
mod cdt;
mod road_tool;
mod zoning;

fn export_has_world_xz(
    export: &TerrainCdtTriangleBufferExport,
    patch: &TerrainPatchSnapshot,
    world_x: f32,
    world_z: f32,
) -> bool {
    let center_x = patch.world_origin_x + patch.world_size_x * 0.5;
    let center_z = patch.world_origin_z + patch.world_size_z * 0.5;
    export.vertices.iter().any(|vertex| {
        (vertex.x - (world_x - center_x)).abs() <= 0.001
            && (vertex.z - (world_z - center_z)).abs() <= 0.001
    })
}

fn test_patch() -> TerrainPatchSnapshot {
    TerrainPatchSnapshot {
        patch_x: 0,
        patch_z: 0,
        sample_width: 2,
        sample_height: 2,
        texture_width: 2,
        texture_height: 2,
        inner_offset_x: 0,
        inner_offset_z: 0,
        world_origin_x: 0.0,
        world_origin_z: 0.0,
        world_size_x: 10.0,
        world_size_z: 10.0,
        height_data: vec![0.0; 4],
    }
}

fn test_snap_graph() -> crate::simulation::network::graph::RegionGraph {
    use crate::simulation::network::graph::Edge;
    use crate::simulation::network::types::{
        EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
    };

    let mut graph = crate::simulation::network::graph::RegionGraph::new();
    let start = graph.add_node(Vector3::ZERO, NodeType::Junction);
    let end = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
    graph.add_edge(Edge {
        start_node: start,
        end_node: end,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: 0.0,
        physical_length: 20.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::ZERO, Vector3::new(20.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::ZERO, Vector3::new(20.0, 0.0, 0.0)],
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access: VehicleFrontageAccess::BothSides,
    });
    graph
}

fn test_cached_refined_terrain_patch(
    contract_revision: i64,
    surface_generation: u64,
) -> CachedRefinedTerrainPatch {
    CachedRefinedTerrainPatch {
        key: RefinedTerrainPatchCacheKey {
            patch_x: 0,
            patch_z: 0,
            render_step_mm: 2000,
        },
        contract_revision,
        surface_generation,
        patch: test_patch(),
        input_road_loops: 0,
        input_source_samples: 0,
        windows: Vec::new(),
        mesh_buffers: None,
        requires_engineered_refinement: false,
        requires_road_clipping: false,
        clip_source_count: 0,
        road_clip_source_count: 0,
        road_clip_loop_count: 0,
        site_clip_loop_count: 0,
        omitted_margin_clip_loop_count: 0,
        clip_error_label: None,
        clip_query_margin_m: 8.0,
        cdt_ms: 0.0,
        reused_windows: 0,
    }
}

fn assert_refined_payload_cache_key(payload: &TerrainPatchPayload, patch_x: usize) {
    let TerrainPatchPayloadData::Refined { patch } = &payload.data else {
        panic!("expected refined terrain payload");
    };
    assert_eq!(patch.key.patch_x, patch_x);
}

fn empty_cdt_stats() -> TerrainCdtStats {
    TerrainCdtStats {
        input_vertices: 0,
        constraint_edges: 0,
        road_constraint_edges: 0,
        building_site_constraint_edges: 0,
        accepted_faces: 0,
        rejected_road_faces: 0,
        preserved_road_constraint_edges: 0,
        preserved_building_site_constraint_edges: 0,
        spade_missing_road_constraint_edges: 0,
        rejected_road_constraint_edges: 0,
        internal_road_constraint_edges: 0,
        invalid_constraint_edges: 0,
        max_face_y_delta_m: 0.0,
        max_face_slope_ratio: 0.0,
        longest_triangle_edge_m: 0.0,
        road_seam_faces: 0,
        road_seam_max_y_delta_m: 0.0,
        road_seam_max_slope_ratio: 0.0,
        retaining_wall_faces: 0,
        retaining_wall_max_y_delta_m: 0.0,
        retaining_wall_max_slope_ratio: 0.0,
        accepted_seam_edges: 0,
        merged_subbudget_seam_edges: 0,
        retaining_wall_required_seam_edges: 0,
        retaining_wall_required_seam_faces: 0,
        blocking_degenerate_seam_edges: 0,
        tie_in_widened_source_samples: 0,
        tie_in_widened_max_y_delta_m: 0.0,
        tie_in_widened_max_slope_ratio: 0.0,
    }
}

fn test_core_with_flat_terrain(raw_height: f32) -> SimCore {
    let config = WorldConfig::default();
    let mut core = SimCore {
        time: TimeSystem::new(),
        heightmap: TerrainSystem::with_chunking(8, 8, 10.0, 4, raw_height),
        watermap: WaterSystem::from_world_config(&config),
        region_graph: crate::simulation::network::graph::RegionGraph::new(),
        transit_network: TransitNetwork::new_with_surface_chunk_span(config.terrain_chunk_m),
        zoning: ZoningSystem::new(&config),
        pollution: PollutionSystem::new(&config),
        noise: NoiseSystem::new(&config),
        desirability: DesirabilitySystem::new(&config),
        demand: DemandSystem::new(),
        pending_demand_spawns: std::collections::VecDeque::new(),
        allocator: BuildingAllocator::new(),
        agents: AgentSystem::new(),
        households: HouseholdSystem::new(),
        logistics: ShipmentSystem::new(),
        config,
        treasury: CityTreasury::new(0.0),
        service_policy: Default::default(),
        fiscal_policy: Default::default(),
        budget_history: std::collections::VecDeque::new(),
        budget_last_lifetime_build_cost: 0.0,
        debug_household_admissions_since_daily: 0,
        undo_stack: std::collections::VecDeque::new(),
        world_lake_fills: Vec::new(),
        world_open_water_fills: Vec::new(),
        resource_deposits: ResourceDepositSystem::from_world_config(&config),
        resource_extraction: ResourceExtractionSystem::new(),
        agriculture: AgricultureSystem::new(),
        world_lake_fill_preview: None,
        authored_water_patch_fill_debug_cache: std::collections::HashMap::new(),
        terrain_stroke_active: false,
        terrain_stroke_has_changes: false,
        terrain_dirty: false,
        water_dirty: false,
        network_dirty: false,
        benchmark_mode: false,
        last_tick_duration: 0.0,
        last_agent_tick_us: 0,
        last_road_timing: String::new(),
        last_surface_debug_edges: Vec::new(),
        refined_terrain_patch_cache: std::collections::HashMap::new(),
        road_locked_terrain_patch_keys: Vec::new(),
        road_locked_terrain_patch_margins: std::collections::BTreeMap::new(),
        building_site_owned_terrain_patch_keys: std::collections::HashSet::new(),
        engineered_terrain_patch_keys: Vec::new(),
        engineered_terrain_patch_margins: std::collections::BTreeMap::new(),
        terrain_payload_generation_counter: 1,
        terrain_payload_global_generation: 1,
        terrain_payload_patch_generations: std::collections::HashMap::new(),
        refined_terrain_assembly_ledgers: std::collections::HashMap::new(),
        cached_road_mesh_data: None,
        cached_road_mesh_generation: 0,
        cached_network_node_positions: std::sync::Arc::new(Vec::new()),
        cached_network_node_positions_dirty: true,
        road_tool_surface_generation: 1,
        camera_aabb: (0.0, 0.0, 0.0, 0.0),
    };
    core.transit_network
        .road_surface
        .compile_dirty(&core.region_graph, &core.heightmap);
    core
}

fn source_export_for_samples(
    samples: &[&[TerrainCdtRoadBoundarySource]],
) -> TerrainCdtSourceExport {
    let mut export = TerrainCdtSourceExport::with_sample_capacity(samples.len());
    for sources in samples {
        export.push_sources(sources);
    }
    export
}

fn span_source() -> TerrainCdtRoadBoundarySource {
    TerrainCdtRoadBoundarySource::SpanSupportBoundary {
        edge_idx: 123,
        edge_class: TerrainCdtEdgeClass::Bridge,
        support_policy: TerrainCdtEarthworkSupportPolicy::BridgeEndpointAbutments,
        source_band_index: 7,
        band_kind: TerrainCdtRoadBandKind::Sidewalk,
        role: TerrainCdtSpanRegionRole::NonRoad,
        start_section_index: 2,
        end_section_index: 5,
        start_s_m: 10.5,
        end_s_m: 14.0,
    }
}

fn standard_span_source() -> TerrainCdtRoadBoundarySource {
    TerrainCdtRoadBoundarySource::SpanSupportBoundary {
        edge_idx: 123,
        edge_class: TerrainCdtEdgeClass::Standard,
        support_policy: TerrainCdtEarthworkSupportPolicy::StandardFullGroundedSpan,
        source_band_index: 7,
        band_kind: TerrainCdtRoadBandKind::Sidewalk,
        role: TerrainCdtSpanRegionRole::NonRoad,
        start_section_index: 2,
        end_section_index: 5,
        start_s_m: 10.5,
        end_s_m: 14.0,
    }
}

fn node_source() -> TerrainCdtRoadBoundarySource {
    TerrainCdtRoadBoundarySource::NodeFootprintBoundary {
        node_id: 77,
        node_kind: TerrainCdtNodePieceKind::JunctionN,
        owner_kind: TerrainCdtRoadBandKind::CurbOrShoulder,
        owner_index: 3,
        boundary_source: None,
    }
}
