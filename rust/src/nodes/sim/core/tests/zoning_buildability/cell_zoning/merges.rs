// SPDX-License-Identifier: GPL-2.0-only

//! Native near-end crossings exercise node aliases with pinned paint and occupied cell lots.

use super::*;
use crate::simulation::zoning::parcels;

#[test]
fn occupied_cells_survive_native_node_merge_regeneration_and_undo() {
    for (angle, erased) in [0.0_f64, 0.35, -0.55]
        .into_iter()
        .flat_map(|angle| [false, true].map(|erased| (angle, erased)))
    {
        let mut core = painted_fixture(angle);
        core.benchmark_mode = false;
        let u = DVec2::new(angle.cos(), angle.sin());
        let v = u.perp();
        // Clear the future crossing before deriving lots. The remaining paint and occupied
        // footprint must stay pinned while the road tool changes this endpoint's topology.
        let clear = core.preview_cell_selection_internal(
            CellSelectionShape::Brush { radius_m: 75.0 },
            &[u * 96.0 + v * 30.0],
        );
        assert!(core.apply_cell_selection_internal(&clear, 0));
        assert!(!core.prepare_cell_lots_internal().created.is_empty());
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
            .min_by(|a, b| {
                let position = |id| {
                    let front = core.zoning.parcel_by_raw_id(id).unwrap().front_center();
                    DVec2::new(f64::from(front.x), f64::from(front.y)).dot(u)
                };
                position(a.action.parcel_id).total_cmp(&position(b.action.parcel_id))
            })
            .expect("remaining painted frontage must support a residential site")
            .action;
        let index = core
            .allocator
            .execute_demand_spawn_action(
                action,
                &mut core.zoning,
                &core.region_graph,
                &core.transit_network.road_surface,
                &core.heightmap,
                core.demand.runtime_catalog(),
                core.demand.runtime_tuning(),
            )
            .unwrap();
        let before = core.allocator.buildings[index].clone();
        let original_lot = core
            .zoning
            .parcel_by_raw_id(before.parcel_id)
            .unwrap()
            .clone();
        if erased {
            let clear = core.preview_cell_selection_internal(
                CellSelectionShape::Brush { radius_m: 300.0 },
                &[DVec2::ZERO],
            );
            assert!(core.apply_cell_selection_internal(&clear, 0));
            assert!(!core.zoning.cells.has_paint());
        }
        let paint = core.zoning.cells.saved_cells();
        assert!(erased || !paint.is_empty());
        let pinned: Vec<_> = paint
            .iter()
            .map(|(key, _)| *key)
            .chain(original_lot.cell_lot().unwrap().cells())
            .map(|key| {
                let frame = core.zoning.cells.frame(key.grid).unwrap();
                (key, frame.corners(key.x, key.y))
            })
            .collect();
        let alignments = core.zoning.cells.saved_road_alignments();
        let node_count = core.region_graph.node_count();
        let edge_count = core.region_graph.edge_count();
        let original_road = core.region_graph.edge(before.edge_idx).clone();
        let endpoint = original_road.end_node;
        assert!((0..node_count as u32).all(|id| core.region_graph.get_valid_node(id) == id));

        // The intersection is outside exact node capture but within the final segment's
        // merge interval. Its node must actually alias to the old endpoint, not merely snap
        // the authored road endpoint or split the existing road into another child edge.
        let crossing = u * 95.8;
        road_terrain_plan::commit_ready(
            &mut core,
            [crossing - v * 80.0, crossing + v * 80.0]
                .map(|p| Vector3::new(p.x as f32, 0.0, p.y as f32))
                .to_vec(),
        );
        let aliases: Vec<_> = (node_count as u32..core.region_graph.node_count() as u32)
            .filter(|&id| core.region_graph.get_valid_node(id) == endpoint)
            .collect();
        assert!(
            !aliases.is_empty(),
            "angle {angle}: crossing must merge a node"
        );
        assert_eq!(core.region_graph.edge(before.edge_idx).end_node, endpoint);
        assert_eq!(core.region_graph.node_adjacency(endpoint).len(), 3);
        core.transit_network
            .lane_system
            .rebuild(&mut core.region_graph);
        core.rebuild_building_entrances_internal();

        // Save compacts merged-node aliases. The grid/lot identity must survive that remap,
        // including an occupied claim with no remaining paint to retain its frame for it.
        let path = temp_save_path("merged_cell_lot");
        core.save_game_internal(path.to_str().unwrap(), None)
            .unwrap();
        let mut loaded = test_core();
        loaded.allocator.registry = core.allocator.registry.clone();
        loaded.load_game_internal(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(path).unwrap();
        loaded.preview_cell_selection_internal(
            CellSelectionShape::Brush { radius_m: 300.0 },
            &[DVec2::ZERO],
        );
        loaded.prepare_cell_lots_internal();
        assert_eq!(loaded.zoning.cells.saved_cells(), paint);
        let restored = &loaded.allocator.buildings[index];
        let lot = loaded.zoning.parcel_by_raw_id(before.parcel_id).unwrap();
        assert_eq!(
            (restored.center_x, restored.center_y, restored.facing_dir),
            (before.center_x, before.center_y, before.facing_dir)
        );
        assert_eq!(lot.corners(), original_lot.corners());
        assert_eq!(lot.cell_lot(), original_lot.cell_lot());
        assert_eq!(lot.occupied_building(), Some(index));
        assert_eq!(restored.edge_idx, lot.edge_idx());
        assert_eq!(
            loaded.allocator.entrances[index].edge_idx,
            restored.edge_idx
        );
        for key in lot.cell_lot().unwrap().cells() {
            assert_eq!(loaded.zoning.cells.lot(key), Some(before.parcel_id));
        }
        assert!(
            (0..loaded.region_graph.node_count() as u32)
                .all(|id| loaded.region_graph.get_valid_node(id) == id)
        );

        for undo in [false, true] {
            if undo {
                assert!(core.undo_action_internal());
                core.rebuild_network_surface_terrain_internal();
            }
            core.preview_cell_selection_internal(
                CellSelectionShape::Brush { radius_m: 300.0 },
                &[DVec2::ZERO],
            );
            core.prepare_cell_lots_internal();
            let building = &core.allocator.buildings[index];
            let lot = core.zoning.parcel_by_raw_id(before.parcel_id).unwrap();
            assert_eq!(
                (building.center_x, building.center_y, building.facing_dir),
                (before.center_x, before.center_y, before.facing_dir)
            );
            assert_eq!(building.parcel_id, before.parcel_id);
            assert_eq!(lot.corners(), original_lot.corners());
            assert_eq!(lot.cell_lot(), original_lot.cell_lot());
            assert_eq!(lot.build_generation(), original_lot.build_generation());
            assert_eq!(lot.occupied_building(), Some(index));
            assert_eq!(core.zoning.cells.saved_cells(), paint);
            for &(key, corners) in &pinned {
                assert_eq!(
                    core.zoning
                        .cells
                        .frame(key.grid)
                        .unwrap()
                        .corners(key.x, key.y),
                    corners
                );
                let centre = corners.into_iter().sum::<DVec2>() * 0.25;
                assert_eq!(core.zoning.cells.pick(centre), Some(key));
            }
            for key in lot.cell_lot().unwrap().cells() {
                assert_eq!(core.zoning.cells.lot(key), Some(before.parcel_id));
            }
            assert_eq!(building.edge_idx, lot.edge_idx());
            assert_eq!(building.frontage_t, lot.frontage_center_t());
            let projection = parcels::project_point_to_edge(
                &core.region_graph,
                lot.edge_idx(),
                lot.front_center(),
            )
            .unwrap();
            assert!((building.frontage_t - projection.s_m / projection.edge_len_m).abs() < 1e-6);
            assert_eq!(core.allocator.entrances[index].edge_idx, building.edge_idx);
            if undo {
                assert_eq!(core.region_graph.node_count(), node_count);
                assert_eq!(core.region_graph.edge_count(), edge_count);
                assert!(
                    (0..node_count as u32).all(|id| core.region_graph.get_valid_node(id) == id)
                );
                let road = core.region_graph.edge(before.edge_idx);
                assert_eq!(road.geometry, original_road.geometry);
                assert_eq!(road.physical_geometry, original_road.physical_geometry);
                assert_eq!(core.zoning.cells.saved_road_alignments(), alignments);
                assert_eq!(building.edge_idx, before.edge_idx);
                assert!((building.frontage_t - before.frontage_t).abs() < 1e-6);
            }
        }
    }
}
