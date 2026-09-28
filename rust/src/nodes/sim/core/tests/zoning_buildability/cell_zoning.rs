// SPDX-License-Identifier: GPL-2.0-only

//! Cell paint through asset-sized lots, site validation and ordinary demand growth.

use super::*;
use crate::simulation::zoning::cells::{CellBounds, CellSelectionShape};
use glam::DVec2;

mod curves;
mod merges;
mod movement;
mod terrain;

#[test]
fn queued_cell_preparation_uses_current_roads_and_discards_replaced_worlds() {
    let mut core = test_core();
    let request_generation = core.terrain_payload_global_generation;
    assert!(!core.zoning.cells.chunk_state((0, 0)).1);
    // The road arrives after the render request; no captured geometry may be published.
    road_terrain_plan::commit_ready(
        &mut core,
        vec![Vector3::new(-96.0, 0.0, 0.0), Vector3::new(96.0, 0.0, 0.0)],
    );
    assert!(core.prepare_requested_cell_chunk((0, 0), request_generation));
    assert!(core.zoning.cells.pick(DVec2::new(25.0, 25.0)).is_some());
    let ready = core.zoning.cells.chunk_state((0, 0));
    assert!(ready.1);
    assert!(core.allocator.building_site_query_index_ready());
    assert!(
        core.transit_network
            .road_surface
            .published_generation_matches_source()
    );
    assert!(core.prepare_requested_cell_chunk((0, 0), request_generation));
    assert_eq!(core.zoning.cells.chunk_state((0, 0)), ready);

    core.create_blank_world_internal(512.0, 512.0, 8.0, 128.0, 0.0)
        .unwrap();
    let cold = core.zoning.cells.chunk_state((0, 0));
    assert!(!cold.1);
    assert!(!core.prepare_requested_cell_chunk((0, 0), request_generation));
    assert_eq!(core.zoning.cells.chunk_state((0, 0)), cold);
    assert!(core.prepare_requested_cell_chunk((0, 0), core.terrain_payload_global_generation));
    assert!(core.zoning.cells.pick(DVec2::new(25.0, 25.0)).is_none());
    assert!(!core.prepare_requested_cell_chunk((1, 0), core.terrain_payload_global_generation));
    assert!(!core.zoning.cells.chunk_state((1, 0)).1);
}

#[test]
fn native_rotated_straight_road_keeps_unobstructed_frontage_cells() {
    use crate::simulation::zoning::cells::CellStore;

    let mut core = test_core();
    let start = DVec2::new(-130.0, -210.0);
    let end = DVec2::new(-55.0, 210.0);
    road_terrain_plan::commit_ready(
        &mut core,
        [start, end]
            .map(|p| Vector3::new(p.x as f32, 0.0, p.y as f32))
            .to_vec(),
    );
    let bounds = CellBounds {
        min: DVec2::splat(-400.0),
        max: DVec2::splat(400.0),
    };
    let mut raw = CellStore::default();
    raw.generate_in_bounds(&core.region_graph, &core.config, bounds, |_| false);
    core.preview_cell_selection_internal(
        CellSelectionShape::Brush { radius_m: 600.0 },
        &[DVec2::ZERO],
    );
    let tangent = (end - start).normalize();
    let normal = tangent.perp();
    let mut missing = Vec::new();
    for column in 0..42 {
        for row in 0..6 {
            for side in [-1.0, 1.0] {
                let point = start
                    + tangent * (column as f64 * 10.0 + 5.0)
                    + normal * ((row as f64 * 10.0 + 10.0) * side);
                if core.zoning.cells.pick(point).is_none() {
                    missing.push((point, raw.pick(point).is_some()));
                }
            }
        }
    }
    assert!(
        missing.is_empty(),
        "missing cells (raw candidate exists): {missing:?}"
    );
}

#[test]
fn native_straight_extension_keeps_cells_across_the_join() {
    for (angle, reversed) in [(0.0_f64, false), (0.35, false), (-0.55, true)] {
        let mut core = test_core();
        let u = DVec2::new(angle.cos(), angle.sin());
        let point =
            |station: f64| Vector3::new((u.x * station) as f32, 0.0, (u.y * station) as f32);
        road_terrain_plan::commit_ready(&mut core, vec![point(0.0), point(113.0)]);
        core.preview_cell_selection_internal(
            CellSelectionShape::Brush { radius_m: 350.0 },
            &[DVec2::ZERO],
        );
        let original = core.zoning.cells.pick(u * 55.0 + u.perp() * 15.0).unwrap();
        let points = if reversed {
            vec![point(243.0), point(113.0)]
        } else {
            vec![point(113.0), point(243.0)]
        };
        road_terrain_plan::commit_ready(&mut core, points);
        core.preview_cell_selection_internal(
            CellSelectionShape::Brush { radius_m: 350.0 },
            &[DVec2::ZERO],
        );
        assert!(core.zoning.cells.profile(original).is_some());
        for column in 0..24 {
            for row in 0..6 {
                for sign in [-1.0, 1.0] {
                    let position = u * (column as f64 * 10.0 + 5.0)
                        + u.perp() * (sign * (10.5 + row as f64 * 10.0));
                    assert!(
                        core.zoning.cells.pick(position).is_some(),
                        "gap: angle={angle}, reversed={reversed}, column={column}, row={row}, side={sign}"
                    );
                }
            }
        }
    }
}

fn painted_fixture(angle: f64) -> SimCore {
    let mut core = test_core();
    let direction = DVec2::new(angle.cos(), angle.sin());
    road_terrain_plan::commit_ready(
        &mut core,
        [-96.0, 96.0]
            .map(|s| Vector3::new((direction.x * s) as f32, 0.0, (direction.y * s) as f32))
            .to_vec(),
    );
    let f: Fixture = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../benchmarks/fixtures/kuopio-terrain/residential-buildability.json"
    )))
    .unwrap();
    register_houses(&mut core, f.assets);
    core.zoning.generate_cells(
        &core.region_graph,
        CellBounds {
            min: DVec2::splat(-256.0),
            max: DVec2::splat(256.0),
        },
        |_| false,
    );
    let selection = core
        .zoning
        .cells
        .select(CellSelectionShape::Fill, &[direction.perp() * 12.0]);
    assert!(!selection.cells.is_empty());
    core.zoning.paint_cells(&selection, 1, |_| false).unwrap();
    core
}

#[test]
fn occupied_cell_building_follows_a_road_commit_and_undo_without_moving() {
    use crate::simulation::zoning::parcels;

    for (angle, erased) in [(0.0, false), (0.35, false), (0.0, true)] {
        let mut core = painted_fixture(angle);
        core.benchmark_mode = false;
        core.prepare_cell_lots_internal();
        let candidates = core.allocator.collect_demand_spawn_candidates_by_use(
            &core.zoning,
            &core.region_graph,
            core.demand.runtime_catalog(),
            &[],
            environment(&core),
        );
        // Choose the far end so the new connection moves the attachment to a child edge.
        let action = &candidates
            .residential
            .iter()
            .max_by(|a, b| {
                core.zoning
                    .parcel_by_raw_id(a.action.parcel_id)
                    .unwrap()
                    .front_center()
                    .x
                    .total_cmp(
                        &core
                            .zoning
                            .parcel_by_raw_id(b.action.parcel_id)
                            .unwrap()
                            .front_center()
                            .x,
                    )
            })
            .unwrap()
            .action;
        let building_index = core
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
        let original_building = core.allocator.buildings[building_index].clone();
        let original_lot = core
            .zoning
            .parcel_by_raw_id(action.parcel_id)
            .unwrap()
            .clone();
        let direction = DVec2::new(angle.cos(), angle.sin());
        if erased {
            let selection = core
                .zoning
                .cells
                .select(CellSelectionShape::Fill, &[direction.perp() * 12.0]);
            core.zoning.paint_cells(&selection, 0, |_| false).unwrap();
            assert!(!core.zoning.cells.has_paint());
        }
        let paint = core.zoning.cells.saved_cells();
        let tee = direction * 5.0;
        road_terrain_plan::commit_ready(
            &mut core,
            [tee - direction.perp() * 80.0, tee]
                .map(|p| Vector3::new(p.x as f32, 0.0, p.y as f32))
                .to_vec(),
        );
        core.transit_network
            .lane_system
            .rebuild(&mut core.region_graph);
        core.rebuild_building_entrances_internal();
        assert_ne!(
            core.allocator.buildings[building_index].edge_idx,
            original_building.edge_idx
        );

        for undo in [false, true] {
            if undo {
                assert!(core.undo_action_internal());
                core.rebuild_network_surface_terrain_internal();
            }
            let building = &core.allocator.buildings[building_index];
            let lot = core.zoning.parcel_by_raw_id(action.parcel_id).unwrap();
            assert_eq!(
                (building.center_x, building.center_y, building.facing_dir),
                (
                    original_building.center_x,
                    original_building.center_y,
                    original_building.facing_dir
                )
            );
            assert_eq!(building.parcel_id, original_building.parcel_id);
            assert_eq!(lot.corners(), original_lot.corners());
            assert_eq!(lot.cell_lot(), original_lot.cell_lot());
            assert_eq!(lot.occupied_building(), Some(building_index));
            assert_eq!(lot.build_generation(), original_lot.build_generation());
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
            assert_eq!(
                core.allocator.entrances[building_index].edge_idx,
                building.edge_idx
            );
            if undo {
                assert_eq!(building.edge_idx, original_building.edge_idx);
                assert!((building.frontage_t - original_building.frontage_t).abs() < 1e-6);
            }
        }
    }
}

#[test]
fn painted_cells_use_asset_lots_and_the_existing_growth_path() {
    for angle in [0.0, 0.35, 0.8] {
        let mut core = painted_fixture(angle);
        let paint = core.zoning.cells.saved_cells();
        let report = core.prepare_cell_lots_internal();
        assert!(!report.created.is_empty(), "angle {angle}");
        assert_eq!(core.zoning.cells.saved_cells(), paint);
        assert!(
            report.created.iter().all(|id| core
                .zoning
                .parcels
                .get(*id)
                .unwrap()
                .cell_lot()
                .is_some())
        );
        let candidates = core.allocator.collect_demand_spawn_candidates_by_use(
            &core.zoning,
            &core.region_graph,
            core.demand.runtime_catalog(),
            &[],
            environment(&core),
        );
        assert!(!candidates.residential.is_empty(), "angle {angle}");
        let action = &candidates.residential[0].action;
        let building = core
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
        assert_eq!(
            core.allocator.buildings[building].parcel_id,
            action.parcel_id
        );
        assert!(
            !core
                .zoning
                .parcel_by_raw_id(action.parcel_id)
                .unwrap()
                .is_available()
        );
        let revision = core.zoning.cells.revision();
        assert!(core.prepare_cell_lots_internal().created.is_empty());
        assert_eq!(
            core.zoning.cells.revision(),
            revision,
            "idle demand must not regenerate"
        );
    }
}

#[test]
fn missing_assets_leave_paint_until_a_catalog_refresh_schedules_growth() {
    let mut core = painted_fixture(0.0);
    let registry = core.allocator.registry.clone();
    core.allocator.registry.clear();
    let paint = core.zoning.cells.saved_cells();
    assert!(core.prepare_cell_lots_internal().created.is_empty());
    assert_eq!(core.zoning.cells.saved_cells(), paint);
    core.allocator.registry = registry;
    core.zoning.invalidate_cell_lot_assets();
    assert!(!core.prepare_cell_lots_internal().created.is_empty());
    assert_eq!(core.zoning.cells.saved_cells(), paint);
}

#[test]
fn catalog_refresh_replaces_empty_lots_but_preserves_occupied_coverage() {
    let mut core = painted_fixture(0.0);
    let first = core.prepare_cell_lots_internal();
    let candidates = core.allocator.collect_demand_spawn_candidates_by_use(
        &core.zoning,
        &core.region_graph,
        core.demand.runtime_catalog(),
        &[],
        environment(&core),
    );
    let action = &candidates.residential[0].action;
    core.allocator
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
    let occupied = core
        .zoning
        .parcel_by_raw_id(action.parcel_id)
        .unwrap()
        .clone();
    let paint = core.zoning.cells.saved_cells();
    let mut house = core
        .allocator
        .registry
        .get(&action.asset_id)
        .unwrap()
        .manifest
        .clone();
    house.building.as_mut().unwrap().lot_width_cells = 3;
    core.allocator.registry.clear();
    core.allocator
        .registry
        .register("replacement", house, String::new());
    core.zoning.invalidate_cell_lot_assets();
    let report = core.prepare_cell_lots_internal();
    assert_eq!(report.retired.len(), first.created.len() - 1);
    assert!(!report.created.is_empty());
    assert!(
        report
            .created
            .iter()
            .all(|id| core.zoning.parcels.get(*id).unwrap().frontage_m() == 30.0)
    );
    let after = core.zoning.parcel_by_raw_id(action.parcel_id).unwrap();
    assert_eq!(after.corners(), occupied.corners());
    assert_eq!(after.cell_lot(), occupied.cell_lot());
    assert!(!after.is_available());
    assert_eq!(core.zoning.cells.saved_cells(), paint);
}

#[test]
fn no_build_toggle_preserves_paint_and_refreshes_lots_before_entrances() {
    let mut core = painted_fixture(0.0);
    let count = core.prepare_cell_lots_internal().created.len();
    assert!(count > 0);
    let paint = core.zoning.cells.saved_cells();
    core.set_no_building_spawn_internal(0, true);
    assert!(core.prepare_cell_lots_internal().created.is_empty());
    assert!(core.zoning.parcels().is_empty());
    assert_eq!(core.zoning.cells.saved_cells(), paint);
    core.set_no_building_spawn_internal(0, false);
    assert_eq!(core.zoning.parcels().len(), count);
    assert!(core.prepare_cell_lots_internal().created.is_empty());
    assert_eq!(core.zoning.cells.saved_cells(), paint);
}

#[test]
fn road_class_changes_preserve_paint_and_restore_ground_frontage_lots() {
    for angle in [0.0, 0.35] {
        for class in [1, 2] {
            let mut core = painted_fixture(angle);
            core.prepare_cell_lots_internal();
            let paint = core.zoning.cells.saved_cells();
            let footprints: Vec<_> = core.zoning.parcels().iter().map(|p| p.corners()).collect();
            assert!(!footprints.is_empty());
            let preview = core.preview_cell_selection_internal(
                CellSelectionShape::Cell,
                &[DVec2::new(angle.cos(), angle.sin()).perp() * -15.0],
            );
            assert!(!preview.selection.cells.is_empty());
            core.set_edge_class_internal(0, class);
            assert_eq!(
                core.region_graph.edge(0).class,
                if class == 1 {
                    EdgeClass::Bridge
                } else {
                    EdgeClass::Tunnel
                }
            );
            assert!(!core.apply_cell_selection_internal(&preview, 1));
            assert_eq!(core.zoning.cells.saved_cells(), paint);
            assert!(core.zoning.parcels().is_empty());
            core.set_edge_class_internal(0, 0);
            assert_eq!(core.zoning.cells.saved_cells(), paint);
            assert_eq!(
                core.zoning
                    .parcels()
                    .iter()
                    .map(|p| p.corners())
                    .collect::<Vec<_>>(),
                footprints
            );
            assert!(core.prepare_cell_lots_internal().created.is_empty());
        }
    }
}

#[test]
fn cell_gesture_preview_is_nonmutating_and_eligibility_changes_reject_commit() {
    let mut core = painted_fixture(0.0);
    let paint = core.zoning.cells.saved_cells();
    let undo_len = core.undo_stack.len();
    let preview =
        core.preview_cell_selection_internal(CellSelectionShape::Fill, &[DVec2::new(0.0, -20.0)]);
    assert!(!preview.selection.cells.is_empty());
    assert_eq!(core.zoning.cells.saved_cells(), paint);
    assert_eq!(core.undo_stack.len(), undo_len);
    core.set_no_building_spawn_internal(0, true);
    assert!(!core.apply_cell_selection_internal(&preview, 1));
    assert_eq!(core.zoning.cells.saved_cells(), paint);
    assert_eq!(core.undo_stack.len(), undo_len);
}

#[test]
fn loading_another_visible_cell_chunk_does_not_invalidate_a_retained_gesture() {
    let mut core = painted_fixture(0.0);
    road_terrain_plan::commit_ready(
        &mut core,
        vec![Vector3::new(600.0, 0.0, 0.0), Vector3::new(796.0, 0.0, 0.0)],
    );
    let preview =
        core.preview_cell_selection_internal(CellSelectionShape::Cell, &[DVec2::new(25.0, -20.0)]);
    assert!(core.prepare_cell_chunk_internal((1, 0)));
    assert_ne!(
        preview.selection.revision,
        core.zoning.cells.revision(),
        "the second view must actually add previously unseen geometry"
    );
    assert!(core.apply_cell_selection_internal(&preview, 1));
    assert!(
        preview
            .selection
            .cells
            .iter()
            .all(|&key| core.zoning.cells.profile(key) == Some(1))
    );
}

#[test]
fn consecutive_cell_gesture_undo_preserves_buildings_created_after_paint() {
    let mut core = painted_fixture(0.0);
    let paint = core.zoning.cells.saved_cells();
    let undo_len = core.undo_stack.len();
    let first =
        core.preview_cell_selection_internal(CellSelectionShape::Fill, &[DVec2::new(0.0, -20.0)]);
    assert!(core.apply_cell_selection_internal(&first, 1));
    core.prepare_cell_lots_internal();
    let candidates = core.allocator.collect_demand_spawn_candidates_by_use(
        &core.zoning,
        &core.region_graph,
        core.demand.runtime_catalog(),
        &[],
        environment(&core),
    );
    let action = candidates
        .residential
        .iter()
        .map(|c| &c.action)
        .find(|action| {
            core.zoning
                .parcel_by_raw_id(action.parcel_id)
                .unwrap()
                .cell_lot()
                .unwrap()
                .cells()
                .any(|key| first.selection.cells.contains(&key))
        })
        .unwrap()
        .clone();
    let building = core
        .allocator
        .execute_demand_spawn_action(
            &action,
            &mut core.zoning,
            &core.region_graph,
            &core.transit_network.road_surface,
            &core.heightmap,
            core.demand.runtime_catalog(),
            core.demand.runtime_tuning(),
        )
        .unwrap();
    let second =
        core.preview_cell_selection_internal(CellSelectionShape::Cell, &[DVec2::new(0.0, 20.0)]);
    assert!(core.apply_cell_selection_internal(&second, 0));
    core.prepare_cell_lots_internal();
    assert_eq!(core.undo_stack.len(), undo_len + 2);
    assert!(core.undo_action_internal());
    core.prepare_cell_lots_internal();
    assert!(core.undo_action_internal());
    assert_eq!(core.undo_stack.len(), undo_len);
    assert_eq!(
        core.zoning
            .cells
            .saved_cells()
            .into_iter()
            .filter(|(_, p)| *p != 0)
            .collect::<Vec<_>>(),
        paint
    );
    let parcel = core.zoning.parcel_by_raw_id(action.parcel_id).unwrap();
    assert_eq!(parcel.occupied_building(), Some(building));
    assert_eq!(parcel.zone_profile_runtime_id(), 0);
    assert!(
        parcel
            .cell_lot()
            .unwrap()
            .cells()
            .all(|key| core.zoning.cells.lot(key) == Some(action.parcel_id))
    );
    assert_eq!(
        core.allocator.buildings[building].parcel_id,
        action.parcel_id
    );
}

#[test]
fn erased_lot_invalidates_queued_growth_without_scanning_the_pending_queue() {
    let mut core = painted_fixture(0.0);
    core.prepare_cell_lots_internal();
    let candidates = core.allocator.collect_demand_spawn_candidates_by_use(
        &core.zoning,
        &core.region_graph,
        core.demand.runtime_catalog(),
        &[],
        environment(&core),
    );
    let action = candidates.residential[0].action.clone();
    let key = core
        .zoning
        .parcel_by_raw_id(action.parcel_id)
        .unwrap()
        .cell_lot()
        .unwrap()
        .cells()
        .next()
        .unwrap();
    let point = core
        .zoning
        .cells
        .frame(key.grid)
        .unwrap()
        .world(f64::from(key.x) + 0.5, f64::from(key.y) + 0.5);
    let preview = core.preview_cell_selection_internal(CellSelectionShape::Cell, &[point]);
    assert!(core.apply_cell_selection_internal(&preview, 0));
    assert!(core.zoning.parcel_by_raw_id(action.parcel_id).is_none());
    assert!(
        core.allocator
            .execute_demand_spawn_action(
                &action,
                &mut core.zoning,
                &core.region_graph,
                &core.transit_network.road_surface,
                &core.heightmap,
                core.demand.runtime_catalog(),
                core.demand.runtime_tuning(),
            )
            .is_err()
    );
    assert!(core.undo_action_internal());
    core.prepare_cell_lots_internal();
    assert_eq!(core.zoning.cells.profile(key), Some(1));
}

#[test]
fn queued_cell_growth_reloads_and_revalidates_erased_or_disabled_lots() {
    use crate::nodes::sim::core::PendingDemandSpawnAction;

    for angle in [0.0, 0.35] {
        for transition in ["unchanged", "erase", "no_build"] {
            let mut core = painted_fixture(angle);
            core.prepare_cell_lots_internal();
            let candidates = core.allocator.collect_demand_spawn_candidates_by_use(
                &core.zoning,
                &core.region_graph,
                core.demand.runtime_catalog(),
                &[],
                environment(&core),
            );
            let action = candidates.residential[0].action.clone();
            core.pending_demand_spawns
                .push_back(PendingDemandSpawnAction {
                    due_minute: absolute_operational_minute(0, 1),
                    zone_type: ZoneType::Residential,
                    action: action.clone(),
                    planned_day_index: 0,
                    planned_minute_of_day: 0,
                });
            let parcel = core.zoning.parcel_by_raw_id(action.parcel_id).unwrap();
            if transition == "erase" {
                let key = parcel.cell_lot().unwrap().cells().next().unwrap();
                let point = core
                    .zoning
                    .cells
                    .frame(key.grid)
                    .unwrap()
                    .world(f64::from(key.x) + 0.5, f64::from(key.y) + 0.5);
                let preview =
                    core.preview_cell_selection_internal(CellSelectionShape::Cell, &[point]);
                assert!(core.apply_cell_selection_internal(&preview, 0));
            } else if transition == "no_build" {
                core.set_no_building_spawn_internal(parcel.edge_idx() as i32, true);
            }
            let paint = core.zoning.cells.saved_cells();
            let path = temp_save_path("queued_cell_growth");
            core.save_game_internal(path.to_str().unwrap(), None)
                .unwrap();
            let mut loaded = test_core();
            loaded.allocator.registry = core.allocator.registry.clone();
            loaded.load_game_internal(path.to_str().unwrap()).unwrap();
            std::fs::remove_file(path).unwrap();
            assert_eq!(loaded.pending_demand_spawns.len(), 1);
            assert_eq!(
                loaded.pending_demand_spawns[0].action.parcel_id,
                action.parcel_id
            );
            assert_eq!(
                loaded.pending_demand_spawns[0].action.asset_id,
                action.asset_id
            );
            loaded.prepare_cell_lots_internal();
            assert_eq!(loaded.zoning.cells.saved_cells(), paint);
            assert!(loaded.allocator.buildings.is_empty());
            assert_eq!(loaded.execute_pending_demand_spawns_for_minute(0, 0), 0);
            assert_eq!(loaded.execute_pending_demand_spawns_for_minute(0, 1), 1);
            assert!(loaded.pending_demand_spawns.is_empty());
            assert_eq!(
                loaded.allocator.buildings.len(),
                usize::from(transition == "unchanged"),
                "angle {angle}, transition {transition}"
            );
            if transition == "unchanged" {
                let building = &loaded.allocator.buildings[0];
                assert_eq!(building.parcel_id, action.parcel_id);
                let lot = loaded.zoning.parcel_by_raw_id(action.parcel_id).unwrap();
                assert_eq!(lot.occupied_building(), Some(0));
                assert!(
                    lot.cell_lot()
                        .unwrap()
                        .cells()
                        .all(|key| loaded.zoning.cells.lot(key) == Some(action.parcel_id))
                );
            }
        }
    }
}
