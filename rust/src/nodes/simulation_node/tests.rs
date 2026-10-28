// SPDX-License-Identifier: GPL-2.0-only

//! Regression tests for the Godot simulation-node bridge.

use super::async_terrain::{
    PayloadDeferral, TerrainPatchPayload, TerrainPatchPayloadAsyncState, TerrainPatchPayloadData,
    TerrainPatchPayloadKey, TerrainPatchPayloadRequest, WaterPatchPayloadAsyncState,
    WaterPatchPayloadKey,
};
use super::*;
use crate::nodes::sim::core::CityTreasury;
use crate::simulation::terrain::TerrainPatchSnapshot;
use crate::simulation::water::WaterSystem;

mod async_payload;
mod global_pool;
mod road_tool;
mod shutdown;
mod zoning;

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

fn test_core_with_flat_terrain(raw_height: f32) -> SimCore {
    let config = WorldConfig::default();
    let mut core = SimCore {
        heightmap: TerrainSystem::with_chunking(8, 8, 10.0, 4, raw_height),
        treasury: CityTreasury::new(0.0),
        terrain_dirty: false,
        water_dirty: false,
        ..SimCore::new(config)
    };
    core.transit_network
        .roadbed
        .compile_dirty(&core.region_graph);
    core
}

// A flat, empty 2.56 km world shared by the vegetation generator and edit regressions.
pub(super) fn vegetation_test_core(
    config: crate::simulation::vegetation::VegetationConfig,
) -> SimCore {
    let mut core = test_core_with_flat_terrain(20.0);
    core.config = WorldConfig::new(2560.0, 2560.0, 40.0, 10.0);
    core.heightmap = TerrainSystem::with_chunking(257, 257, 10.0, 65, 20.0);
    core.watermap = WaterSystem::from_world_config(&core.config);
    core.vegetation =
        crate::simulation::vegetation::VegetationGenerator::resolve(config, &core.config);
    core
}

fn vegetation_canopy_record_count(core: &SimCore) -> usize {
    // Six floats per record, one 510 m patch away from the world edge rejection band.
    super::vegetation_api::scatter_layer(
        core,
        Vector2::new(-255.0, -255.0),
        510.0,
        core.vegetation.canopy_cell_m,
        0,
        true,
    )
    .len()
        / 6
}

#[test]
fn vegetation_scatter_follows_the_saved_generator_parameters() {
    use crate::simulation::vegetation::VegetationConfig;

    let default_count =
        vegetation_canopy_record_count(&vegetation_test_core(VegetationConfig::default()));
    assert!(default_count > 0, "the default world should grow trees");

    // Clearing the world clears both layers, everywhere, not just outside stands.
    assert_eq!(
        vegetation_canopy_record_count(&vegetation_test_core(VegetationConfig {
            enabled: false,
            ..Default::default()
        })),
        0
    );

    // Coverage moves the stand threshold, so open-country density is the floor and in-stand
    // density the ceiling of what one patch can hold.
    let bare = vegetation_canopy_record_count(&vegetation_test_core(VegetationConfig {
        coverage: 0.0,
        ..Default::default()
    }));
    let solid = vegetation_canopy_record_count(&vegetation_test_core(VegetationConfig {
        coverage: 1.0,
        ..Default::default()
    }));
    assert!(
        bare < default_count && default_count < solid,
        "coverage did not order the patch population: {bare} / {default_count} / {solid}"
    );

    // Four times the stems per hectare is four times the candidate cells at one accept rate.
    let dense = vegetation_canopy_record_count(&vegetation_test_core(VegetationConfig {
        canopy_stems_per_ha: VegetationConfig::DEFAULT_CANOPY_STEMS_PER_HA * 4.0,
        ..Default::default()
    }));
    assert!(
        dense > default_count * 2,
        "quadrupled density produced {dense} against {default_count}"
    );

    // A different seed rearranges the same world without emptying or filling it.
    let reseeded = vegetation_canopy_record_count(&vegetation_test_core(VegetationConfig {
        seed: 9_871_234,
        ..Default::default()
    }));
    assert!(reseeded > 0 && reseeded < solid);
}

#[test]
fn vegetation_legacy_output_fingerprint() {
    use crate::simulation::vegetation::VegetationConfig;
    let mut fingerprints = Vec::new();
    for config in [
        VegetationConfig::default(),
        VegetationConfig {
            seed: 9871234,
            coverage: 0.85,
            ..Default::default()
        },
    ] {
        let core = vegetation_test_core(config);
        for (cell_m, salt, canopy) in [
            (core.vegetation.canopy_cell_m, 0, true),
            (core.vegetation.understory_cell_m, 64, false),
        ] {
            let origin = Vector2::new(-255.0, -255.0);
            let mut records =
                super::vegetation_api::scatter_layer(&core, origin, 510.0, cell_m, salt, canopy);
            // The fingerprints predate world-space packing; relative to the patch they are
            // bit-identical, because this is the subtraction the packing used to do.
            for record in records.as_chunks_mut::<6>().0 {
                record[0] -= origin.x;
                record[2] -= origin.y;
            }
            let fingerprint = records.iter().fold(0xcbf29ce484222325_u64, |h, v| {
                (h ^ u64::from(v.to_bits())).wrapping_mul(0x100000001b3)
            });
            fingerprints.push((records.len(), fingerprint));
        }
    }
    // Captured from the pre-refactor scatter at b6bf1c40: both layers and two world seeds.
    assert_eq!(
        fingerprints,
        vec![
            (1746, 11351230337380857993),
            (19218, 17166373395456776641),
            (4620, 3667363800515279275),
            (52008, 17573628694202338401)
        ]
    );
}
