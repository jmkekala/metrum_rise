// SPDX-License-Identifier: GPL-2.0-only

//! Native curved road commit/undo preserves painted grids, buildings and entrance attachments.

use super::*;
use crate::simulation::zoning::parcels;

fn curved_core(angle: f64) -> (SimCore, DVec2, DVec2) {
    let mut core = test_core();
    core.benchmark_mode = false;
    let u = DVec2::new(angle.cos(), angle.sin());
    let v = u.perp();
    let points = (0..=24)
        .map(|i| {
            let x = i as f64 * 12.0 - 144.0;
            let point = u * x + v * (x * x * 0.001);
            Vector3::new(point.x as f32, 0.0, point.y as f32)
        })
        .collect();
    road_terrain_plan::commit_ready(&mut core, points);
    let f: Fixture = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../benchmarks/fixtures/kuopio-terrain/residential-buildability.json"
    )))
    .unwrap();
    register_houses(&mut core, f.assets);
    let centre = u * 90.0 + v * 40.0;
    let paint = core
        .preview_cell_selection_internal(CellSelectionShape::Brush { radius_m: 55.0 }, &[centre]);
    assert!(paint.selection.cells.len() > 30);
    assert!(core.apply_cell_selection_internal(&paint, 1));
    (core, u, v)
}

#[test]
fn occupied_curved_grid_survives_native_branch_commit_and_undo() {
    for (angle, erased) in [0.0, 0.35]
        .into_iter()
        .flat_map(|angle| [false, true].map(|erased| (angle, erased)))
    {
        let (mut core, u, v) = curved_core(angle);
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
            .max_by(|a, b| {
                let front = |id| core.zoning.parcel_by_raw_id(id).unwrap().front_center();
                front(a.action.parcel_id)
                    .x
                    .total_cmp(&front(b.action.parcel_id).x)
            })
            .expect("curved paint must support an actual residential site")
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
            .parcel_by_raw_id(action.parcel_id)
            .unwrap()
            .clone();
        if erased {
            let erase = core.preview_cell_selection_internal(
                CellSelectionShape::Brush { radius_m: 55.0 },
                &[u * 90.0 + v * 40.0],
            );
            assert!(core.apply_cell_selection_internal(&erase, 0));
            assert!(!core.zoning.cells.has_paint());
        }
        let paint = core.zoning.cells.saved_cells();
        let tee = u * -48.0 + v * (48.0 * 48.0 * 0.001);
        let perpendicular = (v + u * 0.096).normalize();
        road_terrain_plan::commit_ready(
            &mut core,
            [tee - perpendicular * 80.0, tee]
                .map(|p| Vector3::new(p.x as f32, 0.0, p.y as f32))
                .to_vec(),
        );
        core.transit_network
            .lane_system
            .rebuild(&mut core.region_graph);
        core.rebuild_building_entrances_internal();
        assert_ne!(core.allocator.buildings[index].edge_idx, before.edge_idx);
        for undo in [false, true] {
            if undo {
                assert!(core.undo_action_internal());
                core.rebuild_network_surface_terrain_internal();
            }
            let building = &core.allocator.buildings[index];
            let lot = core.zoning.parcel_by_raw_id(before.parcel_id).unwrap();
            assert_eq!(
                (building.center_x, building.center_y, building.facing_dir),
                (before.center_x, before.center_y, before.facing_dir)
            );
            assert_eq!(lot.corners(), original_lot.corners());
            assert_eq!(lot.cell_lot(), original_lot.cell_lot());
            assert_eq!(lot.build_generation(), original_lot.build_generation());
            assert_eq!(lot.occupied_building(), Some(index));
            assert_eq!(core.zoning.cells.saved_cells(), paint);
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
                assert_eq!(building.edge_idx, before.edge_idx);
                assert!((building.frontage_t - before.frontage_t).abs() < 1e-6);
            }
        }
    }
}
