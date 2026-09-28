// SPDX-License-Identifier: GPL-2.0-only

//! Terrain authoring changes growth feasibility while retaining authoritative cell paint.

use super::*;
use godot::prelude::Vector2;

#[test]
fn terrain_stroke_revalidates_cell_growth_and_undo_restores_feasibility() {
    for angle in [0.0, 0.35] {
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
        let action = candidates.residential[0].action.clone();
        let lot = core
            .zoning
            .parcel_by_raw_id(action.parcel_id)
            .unwrap()
            .clone();
        let paint = core.zoning.cells.saved_cells();
        let frames: Vec<_> = core.zoning.cells.saved_frames().collect();
        let preview = core.preview_cell_selection_internal(
            CellSelectionShape::Cell,
            &[DVec2::new(angle.cos(), angle.sin()).perp() * -15.0],
        );
        let undo_count = core.undo_stack.len();
        let revision = core.zoning.cell_external_revision();
        let terrain_generation = core.heightmap.source_generation();
        let center = lot.center();
        core.start_terrain_stroke_internal();
        core.sculpt_terrain_stroke_step_internal(Vector2::new(center.x, center.y), 20.0, 10.0);
        assert!(core.end_terrain_stroke_internal());
        assert_eq!(core.undo_stack.len(), undo_count + 1);
        assert!(core.heightmap.source_generation() > terrain_generation);
        assert!(core.zoning.cell_external_revision() > revision);
        assert!(!core.apply_cell_selection_internal(&preview, 1));
        assert_eq!(core.zoning.cells.saved_cells(), paint);
        for (id, frame) in &frames {
            assert_eq!(core.zoning.cells.frame(*id), Some(*frame));
        }
        let after = core.zoning.parcel_by_raw_id(action.parcel_id).unwrap();
        assert_eq!(after.corners(), lot.corners());
        assert_eq!(after.cell_lot(), lot.cell_lot());
        let blocked = core.allocator.collect_demand_spawn_candidates_by_use(
            &core.zoning,
            &core.region_graph,
            core.demand.runtime_catalog(),
            &[],
            environment(&core),
        );
        assert!(
            !blocked
                .residential
                .iter()
                .any(|c| c.action.parcel_id == action.parcel_id)
        );
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
                .is_err(),
            "a queued action must revalidate the sculpted site"
        );
        assert!(core.allocator.buildings.is_empty());
        assert!(core.undo_action_internal());
        core.rebuild_network_surface_terrain_internal();
        core.prepare_cell_lots_internal();
        assert_eq!(core.zoning.cells.saved_cells(), paint);
        let restored = core.allocator.collect_demand_spawn_candidates_by_use(
            &core.zoning,
            &core.region_graph,
            core.demand.runtime_catalog(),
            &[],
            environment(&core),
        );
        assert!(
            restored
                .residential
                .iter()
                .any(|c| c.action.parcel_id == action.parcel_id)
        );
        let index = core
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
        assert_eq!(core.allocator.buildings[index].parcel_id, action.parcel_id);
        assert_eq!(
            core.zoning
                .parcel_by_raw_id(action.parcel_id)
                .unwrap()
                .corners(),
            lot.corners()
        );
    }
}
