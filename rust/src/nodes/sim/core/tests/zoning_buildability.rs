// SPDX-License-Identifier: GPL-2.0-only

//! Portable hillside roadside-site and feasibility invalidation regressions.

use super::*;
use crate::simulation::buildings::allocator::BuildingSiteEnvironment;

#[derive(serde::Deserialize)]
struct Fixture {
    width: usize,
    height: usize,
    cell_m: f32,
    x0: usize,
    z0: usize,
    samples: Vec<Vec<f32>>,
    nodes: Vec<(u32, f32, f32, f32)>,
    edges: Vec<FixtureEdge>,
    parcels: Vec<(u64, usize, i8, f32, f32, f32, u16)>,
    assets: Vec<String>,
}

#[derive(serde::Deserialize)]
struct FixtureEdge {
    id: usize,
    start: u32,
    end: u32,
    width: f32,
    start_clip: f32,
    end_clip: f32,
    points: Vec<[f32; 3]>,
}

fn fixture() -> SimCore {
    let f: Fixture = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../benchmarks/fixtures/kuopio-terrain/residential-buildability.json"
    )))
    .unwrap();
    let mut core = test_core();
    core.config.width_m = (f.width - 1) as f32 * f.cell_m;
    core.config.height_m = (f.height - 1) as f32 * f.cell_m;
    core.config.terrain_cell_m = f.cell_m;
    core.config.terrain_chunk_m = 512.0;
    core.heightmap = TerrainSystem::from_world_config(&core.config);
    core.transit_network = TransitNetwork::new_for_world(&core.config);
    core.zoning = ZoningSystem::new(&core.config);
    for (z, row) in f.samples.iter().enumerate() {
        for (x, &height) in row.iter().enumerate() {
            core.heightmap.set_height(f.x0 + x, f.z0 + z, height);
        }
    }
    for (id, x, y, z) in f.nodes {
        assert_eq!(
            core.region_graph
                .add_node(Vector3::new(x, y, z), NodeType::Junction),
            id
        );
    }
    for e in f.edges {
        let points: Vec<_> = e
            .points
            .iter()
            .map(|p| Vector3::new(p[0], p[1], p[2]))
            .collect();
        let length = points.windows(2).map(|p| p[0].distance_to(p[1])).sum();
        let edge = Edge {
            start_node: e.start,
            end_node: e.end,
            width: e.width,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            fwd_lanes: 1,
            bkw_lanes: 1,
            physical_length: length,
            start_clip: e.start_clip,
            end_clip: e.end_clip,
            geometry: points.clone(),
            physical_geometry: points,
            ..Default::default()
        };
        assert_eq!(core.region_graph.add_edge(edge), e.id);
        core.transit_network
            .road_surface
            .mark_edge_dirty(&core.region_graph, e.id);
    }
    core.region_graph.rebuild_adjacency_list();
    core.precompute_road_mesh_data();
    for (id, edge, side, t, width, depth, profile) in f.parcels {
        core.zoning
            .restore_parcel_from_attachment(
                id,
                edge,
                side,
                t,
                width,
                depth,
                profile,
                &core.region_graph,
            )
            .unwrap();
    }
    for asset in f.assets {
        let mut manifest: AssetManifest = toml::from_str(&asset).unwrap();
        // Keep this feasibility fixture's structural box geometry explicit and engine-independent.
        for part in &mut manifest.mesh_parts {
            part.imported_bounds = Some([[-0.75, 0.0, -0.75], [0.75, 1.0, 0.75]]);
        }
        core.allocator
            .registry
            .register("kenney", manifest, String::new());
    }
    core
}

fn environment(core: &SimCore) -> BuildingSiteEnvironment<'_> {
    BuildingSiteEnvironment {
        road_surface: &core.transit_network.road_surface,
        terrain: &core.heightmap,
    }
}

#[test]
fn hillside_residential_sites_select_constructible_houses() {
    let mut core = fixture();
    let before = (
        core.zoning.parcels().len(),
        core.allocator.buildings.len(),
        core.heightmap.source_generation(),
        core.heightmap.visual_generation(),
    );
    for id in [6, 7, 8, 9, 16] {
        let p = core.zoning.parcel_by_raw_id(id).unwrap();
        let geometry = core
            .zoning
            .parcel_geometry_at(p.center().x, p.center().y)
            .unwrap();
        assert_eq!(
            core.allocator.zoning_site_feasibility(
                &geometry,
                1,
                &core.zoning,
                &core.region_graph,
                core.demand.runtime_catalog(),
                environment(&core)
            ),
            Ok(()),
            "parcel {id}"
        );
    }
    assert_eq!(
        before,
        (
            core.zoning.parcels().len(),
            core.allocator.buildings.len(),
            core.heightmap.source_generation(),
            core.heightmap.visual_generation()
        ),
        "zoning must not engineer the ground"
    );
    for id in [6, 7, 8, 9, 16] {
        let candidates = core.allocator.collect_demand_spawn_candidates_by_use(
            &core.zoning,
            &core.region_graph,
            core.demand.runtime_catalog(),
            &[],
            environment(&core),
        );
        let action = &candidates
            .residential
            .iter()
            .find(|c| c.action.parcel_id == id)
            .expect("buildable candidate")
            .action;
        if id == 7 {
            assert_eq!(
                action.asset_id,
                "kenney:building.residential.family_single_house"
            );
        }
        let result = core.allocator.execute_demand_spawn_action(
            action,
            &mut core.zoning,
            &core.region_graph,
            &core.transit_network.road_surface,
            &core.heightmap,
            core.demand.runtime_catalog(),
            core.demand.runtime_tuning(),
        );
        assert!(result.is_ok(), "parcel {id}: {result:?}");
        core.publish_pending_building_site_changes();
    }
    assert_eq!(core.allocator.buildings.len(), 5);
    let patches = core.collect_refined_terrain_patch_build_inputs(2.0);
    for patch in SimCore::build_refined_terrain_patch_cache_entries(patches) {
        assert!(
            crate::nodes::simulation_node::SimulationNode::cached_refined_cdt_failure_label(&patch)
                .is_none(),
            "terrain failed: {:?}",
            crate::nodes::simulation_node::SimulationNode::cached_refined_cdt_failure_label(&patch)
        );
    }
}

#[test]
fn feasibility_invalidates_changed_terrain_and_assets() {
    let mut core = fixture();
    let p = core.zoning.parcel_by_raw_id(7).unwrap();
    let geometry = core
        .zoning
        .parcel_geometry_at(p.center().x, p.center().y)
        .unwrap();
    let check = |core: &SimCore| {
        core.allocator.zoning_site_feasibility(
            &geometry,
            1,
            &core.zoning,
            &core.region_graph,
            core.demand.runtime_catalog(),
            environment(core),
        )
    };
    assert!(check(&core).is_ok());
    let solves = core.allocator.site_feasibility_solve_count();
    assert!(check(&core).is_ok());
    assert_eq!(core.allocator.site_feasibility_solve_count(), solves);
    core.allocator.clear();
    assert_eq!(core.allocator.site_feasibility_solve_count(), 0);
    assert!(check(&core).is_ok());
    assert!(
        core.allocator.site_feasibility_solve_count() > 0,
        "reset must discard even empty-city cached verdicts"
    );
    // A changed road generation must not reuse a cached success from retained old products.
    core.transit_network
        .road_surface
        .mark_edge_dirty(&core.region_graph, 1);
    assert!(check(&core).is_err());
    core.precompute_road_mesh_data();
    assert!(check(&core).is_ok());
    let solves = core.allocator.site_feasibility_solve_count();
    core.heightmap.set_height(1000, 1000, 30.0);
    assert!(check(&core).is_ok());
    assert_eq!(
        core.allocator.site_feasibility_solve_count(),
        solves,
        "remote terrain must reuse local feasibility"
    );
    let grid = core
        .heightmap
        .grid_rect_for_world_bounds(
            geometry.aabb_min.x - 25.0,
            geometry.aabb_min.y - 25.0,
            geometry.aabb_max.x + 25.0,
            geometry.aabb_max.y + 25.0,
        )
        .unwrap();
    for z in grid.2..=grid.3 {
        for x in grid.0..=grid.1 {
            core.heightmap.set_height(x, z, 200.0);
        }
    }
    assert!(
        check(&core).is_err(),
        "cached buildability must not survive a local terrain edit"
    );
    let before = core.allocator.buildings.len();
    assert!(
        core.allocator
            .execute_demand_spawn_action(
                &DemandSpawnAction {
                    parcel_id: 7,
                    asset_id: "kenney:building.residential.family_single_house".into()
                },
                &mut core.zoning,
                &core.region_graph,
                &core.transit_network.road_surface,
                &core.heightmap,
                core.demand.runtime_catalog(),
                core.demand.runtime_tuning()
            )
            .is_err(),
        "a stale queued action must revalidate before insertion"
    );
    assert_eq!(core.allocator.buildings.len(), before);
    assert!(core.zoning.parcel_by_raw_id(7).unwrap().is_available());
    core.allocator.registry.clear();
    assert!(check(&core).unwrap_err().contains("asset"));
    assert!(
        core.allocator
            .zoning_site_feasibility(
                &geometry,
                0,
                &core.zoning,
                &core.region_graph,
                core.demand.runtime_catalog(),
                environment(&core)
            )
            .is_ok(),
        "unzoning must remain possible"
    );
}
