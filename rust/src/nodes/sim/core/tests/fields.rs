// SPDX-License-Identifier: GPL-2.0-only

//! Field land reservations, atomic vertex edits and production-site lifecycle regressions.

use super::*;
use crate::assets::asset::BuildingFieldData;
use crate::simulation::buildings::allocator::{
    BuildingSiteEnvironment, ExplicitServicePlacementRejection,
};
use godot::prelude::Vector2;

mod undo_scaling;

fn rectangle(x: f32, z: f32, width: f32, depth: f32) -> Vec<Vector2> {
    vec![
        Vector2::new(x, z),
        Vector2::new(x + width, z),
        Vector2::new(x + width, z + depth),
        Vector2::new(x, z + depth),
    ]
}

/// Shared occupied-world fixture for field lifecycle and city accounting regressions.
pub(in crate::nodes::sim::core) fn farm_fixture() -> (SimCore, usize, Vec<Vector2>) {
    let mut core = test_core();
    core.transit_network.add_road(
        &mut core.region_graph,
        vec![
            Vector3::new(-500.0, 0.0, 0.0),
            Vector3::new(500.0, 0.0, 0.0),
        ],
        1,
        1,
        EdgeClass::Standard,
        &mut core.zoning,
        &mut core.allocator,
    );
    core.transit_network
        .road_surface
        .compile_dirty(&core.region_graph, &core.heightmap);
    let id = register_test_asset(&mut core.allocator, "field_test", ZoneType::Industrial);
    let mut manifest = core.allocator.registry.get(&id).unwrap().manifest.clone();
    let building = manifest.building.as_mut().unwrap();
    building.placement_mode = PlacementMode::Explicit;
    building.economy_profile = Some("grain_farm_basic".into());
    building.field = Some(BuildingFieldData {
        resource: "grain".into(),
        area_mode: "player_polygon".into(),
    });
    core.allocator
        .registry
        .register("test", manifest, String::new());
    let farm = core
        .place_industry_building_internal(&id, 0.0, 8.0)
        .unwrap();
    let site = &core.allocator.building_sites[farm];
    let back = site
        .footprint_world
        .iter()
        .map(|p| p.y)
        .fold(f32::NEG_INFINITY, f32::max);
    (core, farm, rectangle(-8.0, back + 1.0, 400.0, 500.0))
}

/// Adds one mixed-age family and assigns its adults to the farm.
pub(in crate::nodes::sim::core) fn add_farm_household(core: &mut SimCore, farm: usize) -> usize {
    use crate::simulation::economy::agents::{age_group_can_work, household_member_age_group};
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = crate::simulation::economy::definitions::load_runtime_economy_tuning().unwrap();
    let household = core
        .households
        .admit_immigrant_household(&catalog, &tuning, farm, 4);
    core.allocator.claim_vacancy(farm);
    for member in 0..4 {
        let age = household_member_age_group(farm, household, member, 4);
        let agent = core
            .agents
            .spawn_housed_agent_with_age_group(farm, 0.0, 8.0, age);
        core.agents.assign_household_id(agent, household);
        if age_group_can_work(age) {
            core.agents.assign_work_building(agent, farm, 0);
            core.allocator.buildings[farm].worker_count += 1;
        }
    }
    household
}

#[test]
fn field_resize_is_atomic_and_recalculates_capacity_without_new_startup_money() {
    let (mut core, farm, polygon) = farm_fixture();
    let center = Vector2::new(
        core.allocator.buildings[farm].center_x,
        core.allocator.buildings[farm].center_y,
    );
    core.commit_field_polygon_internal(farm, polygon.clone())
        .unwrap();
    let catalog = load_runtime_economy_catalog().unwrap();
    assert_eq!(
        core.allocator.worker_capacity_with_catalog(farm, &catalog),
        2
    );
    let budget = core.allocator.buildings[farm].operating_budget;
    let mut resized = polygon.clone();
    resized[2].x += 400.0;
    let result = core
        .resize_field_polygon_internal(farm, center, &polygon, resized.clone())
        .unwrap();
    assert_eq!(result.area_m2, 300_000.0);
    assert_eq!(
        core.allocator.worker_capacity_with_catalog(farm, &catalog),
        3
    );
    assert_eq!(core.allocator.buildings[farm].work_area_scale, 30.0);
    assert_eq!(core.allocator.buildings[farm].operating_budget, budget);
    let revision = core.agriculture.visual_revision();
    let mut crossed = resized.clone();
    crossed[1] = resized[3];
    assert!(
        core.resize_field_polygon_internal(farm, center, &resized, crossed)
            .is_err()
    );
    assert!(
        core.resize_field_polygon_internal(farm, center, &polygon, polygon.clone())
            .is_err()
    );
    assert!(
        core.resize_field_polygon_internal(farm, center + Vector2::ONE, &resized, polygon.clone())
            .is_err()
    );
    assert_eq!(core.agriculture.visual_revision(), revision);
    assert_eq!(
        core.agriculture
            .site_for_building(farm)
            .unwrap()
            .polygon_world,
        resized
    );
    assert_eq!(core.allocator.buildings[farm].work_area_scale, 30.0);
    let result = core
        .resize_field_polygon_internal(farm, center, &resized, polygon)
        .unwrap();
    assert_eq!(result.area_m2, 200_000.0);
    assert_eq!(
        core.allocator.worker_capacity_with_catalog(farm, &catalog),
        2
    );
}

#[test]
fn field_attachment_uses_the_authored_rotated_farm_lot() {
    let (mut core, farm, _) = farm_fixture();
    let asset = core.allocator.buildings[farm].asset_id.clone();
    let mut manifest = core
        .allocator
        .registry
        .get(&asset)
        .unwrap()
        .manifest
        .clone();
    manifest.building.as_mut().unwrap().frontage_forward = Some([1.0, 0.0, 0.0]);
    core.allocator
        .registry
        .register("test", manifest, String::new());
    core.allocator.buildings[farm].width_cells = 2;
    core.allocator.buildings[farm].depth_cells = 12;
    core.allocator
        .rebuild_building_site_clients(core.zoning.config.zone_cell_m);
    core.allocator.rebuild_zone_index();
    let site = &core.allocator.building_sites[farm];
    let max_x = site
        .lot_footprint_world
        .iter()
        .map(|p| p.x)
        .fold(f32::NEG_INFINITY, f32::max);
    let max_z = site
        .lot_footprint_world
        .iter()
        .map(|p| p.y)
        .fold(f32::NEG_INFINITY, f32::max);
    // The displayed lot extends along X; the former road-aligned reconstruction extended along Z.
    let polygon = rectangle(max_x - 9.0, max_z + 1.0, 10.0, 10.0);
    core.commit_field_polygon_internal(farm, polygon).unwrap();
}

#[test]
fn fields_reject_roads_building_sites_parcels_and_other_fields() {
    let (mut core, farm, polygon) = farm_fixture();
    // A connected, concave field can extend alongside the farm, but cannot cross the street.
    let road_overlap = vec![
        polygon[0],
        Vector2::new(30.0, polygon[0].y),
        Vector2::new(30.0, -20.0),
        Vector2::new(100.0, -20.0),
        Vector2::new(100.0, 80.0),
        Vector2::new(-8.0, 80.0),
    ];
    assert!(
        core.validate_field_polygon_internal(farm, &road_overlap)
            .unwrap_err()
            .contains("road")
    );
    let building_overlap = rectangle(-20.0, 8.0, 60.0, 60.0);
    assert!(
        core.validate_field_polygon_internal(farm, &building_overlap)
            .unwrap_err()
            .contains("building site")
    );
    let parcel = core
        .zoning
        .place_or_rezone_parcel_at(120.0, 8.0, 0, 20.0, 80.0, &core.region_graph)
        .unwrap();
    assert!(
        core.validate_field_polygon_internal(farm, &polygon)
            .unwrap_err()
            .contains("reserved zoning")
    );
    core.zoning
        .remove_parcels_by_raw_ids(&HashSet::from([parcel.raw()]));
    core.allocator
        .field_clearance
        .set(100, &rectangle(100.0, 100.0, 20.0, 20.0));
    assert!(
        core.validate_field_polygon_internal(farm, &polygon)
            .unwrap_err()
            .contains("another field")
    );
    core.allocator.field_clearance.clear();
    core.commit_field_polygon_internal(farm, polygon.clone())
        .unwrap();
    // Save/load reconstruction restores the same placement reservation from authoritative sites.
    let restored = AgricultureSystem::from_sites(core.agriculture.sites().to_vec());
    core.allocator.field_clearance.clear();
    restored.apply_work_area_scales(&mut core.allocator);
    assert!(
        core.allocator
            .field_clearance
            .overlaps_polygon(&rectangle(100.0, 100.0, 10.0, 10.0))
    );
    assert!(core.validate_field_polygon_internal(farm, &polygon).is_ok());
}

#[test]
fn buildings_and_zoning_reject_a_reserved_field_even_after_cached_preview() {
    let (mut core, _, _) = farm_fixture();
    let asset = "test:field_test";
    assert!(
        core.get_industry_building_placement_preview_internal(asset, 120.0, 8.0)
            .unwrap()
            .valid
    );
    let geometry = core
        .zoning
        .preview_parcel_at(120.0, 8.0, 20.0, 20.0, &core.region_graph)
        .unwrap();
    core.allocator
        .field_clearance
        .set(100, &rectangle(100.0, 7.0, 60.0, 80.0));
    let preview = core
        .get_industry_building_placement_preview_internal(asset, 120.0, 8.0)
        .unwrap();
    assert!(!preview.valid);
    assert_eq!(
        preview.rejection,
        Some(ExplicitServicePlacementRejection::FieldOverlap)
    );
    assert_eq!(
        core.place_industry_building_internal(asset, 120.0, 8.0),
        Err(ExplicitServicePlacementRejection::FieldOverlap)
    );
    for profile in [
        0,
        core.zoning
            .profiles
            .default_runtime_id_for_zone_type(ZoneType::Residential)
            .unwrap(),
    ] {
        assert_eq!(
            core.allocator
                .zoning_site_feasibility(
                    &geometry,
                    profile,
                    &core.zoning,
                    &core.region_graph,
                    BuildingSiteEnvironment {
                        road_surface: &core.transit_network.road_surface,
                        terrain: &core.heightmap
                    }
                )
                .unwrap_err(),
            "field_overlap"
        );
    }
}

#[test]
fn field_and_explicit_site_revalidate_cell_paint_without_a_derived_lot() {
    use crate::simulation::zoning::cells::{CellBounds, CellSelectionShape};
    use glam::DVec2;
    let (mut core, farm, polygon) = farm_fixture();
    let asset = "test:field_test";
    assert!(core.validate_field_polygon_internal(farm, &polygon).is_ok());
    assert!(
        core.get_industry_building_placement_preview_internal(asset, 120.0, 8.0)
            .unwrap()
            .valid
    );
    core.zoning.generate_cells(
        &core.region_graph,
        CellBounds {
            min: DVec2::splat(-600.0),
            max: DVec2::splat(600.0),
        },
        |_| false,
    );
    let path = [DVec2::new(120.0, 12.0), DVec2::new(120.0, 42.0)];
    let selection = core.zoning.cells.select(CellSelectionShape::Cell, &path);
    assert!(!selection.cells.is_empty());
    core.zoning.paint_cells(&selection, 1, |_| false).unwrap();
    assert!(core.zoning.parcels().is_empty());
    assert!(
        core.validate_field_polygon_internal(farm, &polygon)
            .unwrap_err()
            .contains("reserved zoning")
    );
    assert!(
        !core
            .get_industry_building_placement_preview_internal(asset, 120.0, 8.0)
            .unwrap()
            .valid
    );
    let count = core.allocator.buildings.len();
    assert_eq!(
        core.place_industry_building_internal(asset, 120.0, 8.0),
        Err(ExplicitServicePlacementRejection::SiteOverlap)
    );
    assert_eq!(core.allocator.buildings.len(), count);
    let selection = core.zoning.cells.select(CellSelectionShape::Cell, &path);
    core.zoning.paint_cells(&selection, 0, |_| false).unwrap();
    assert!(core.validate_field_polygon_internal(farm, &polygon).is_ok());
    assert!(
        core.get_industry_building_placement_preview_internal(asset, 120.0, 8.0)
            .unwrap()
            .valid
    );
}

#[test]
fn field_resize_invalidates_old_and_new_cells_and_rejects_new_paint_atomically() {
    use crate::simulation::zoning::cells::CellSelectionShape;
    use glam::DVec2;

    let (mut core, farm, polygon) = farm_fixture();
    let point = DVec2::new(120.0, 42.0);
    assert!(core.prepare_cell_chunk_internal((0, 0)));
    let preview = core.preview_cell_selection_internal(CellSelectionShape::Cell, &[point]);
    assert!(!preview.selection.cells.is_empty());
    let center = Vector2::new(
        core.allocator.buildings[farm].center_x,
        core.allocator.buildings[farm].center_y,
    );
    core.commit_field_polygon_internal(farm, polygon.clone())
        .unwrap();
    assert!(
        !core.zoning.cells.chunk_state((0, 0)).1,
        "a committed field must invalidate previously generated cells"
    );
    assert!(!core.apply_cell_selection_internal(&preview, 1));
    assert!(core.prepare_cell_chunk_internal((0, 0)));
    assert!(core.zoning.cells.pick(point).is_none());

    let smaller = rectangle(polygon[0].x, polygon[0].y, 40.0, 500.0);
    core.resize_field_polygon_internal(farm, center, &polygon, smaller.clone())
        .unwrap();
    assert!(
        !core.zoning.cells.chunk_state((0, 0)).1,
        "shrinking a field must release previously hidden cells"
    );
    assert!(core.prepare_cell_chunk_internal((0, 0)));
    assert!(core.zoning.cells.pick(point).is_some());
    let preview = core.preview_cell_selection_internal(CellSelectionShape::Cell, &[point]);
    assert!(core.apply_cell_selection_internal(&preview, 1));
    assert!(core.prepare_cell_chunk_internal((0, 0)));
    let state = core.zoning.cells.chunk_state((0, 0));
    let paint = core.zoning.cells.saved_cells();
    let revision = core.agriculture.visual_revision();
    let dependencies = core.cell_tool_dependencies();
    assert!(
        core.resize_field_polygon_internal(farm, center, &smaller, polygon)
            .unwrap_err()
            .contains("reserved zoning")
    );
    assert_eq!(
        core.agriculture
            .site_for_building(farm)
            .unwrap()
            .polygon_world,
        smaller
    );
    assert_eq!(core.agriculture.visual_revision(), revision);
    assert_eq!(core.cell_tool_dependencies(), dependencies);
    assert_eq!(core.zoning.cells.chunk_state((0, 0)), state);
    assert_eq!(core.zoning.cells.saved_cells(), paint);
}

#[test]
fn field_removal_and_undo_refresh_cached_cells_beyond_the_farm_footprint() {
    use glam::DVec2;

    let (mut core, farm, polygon) = farm_fixture();
    core.transit_network.add_road(
        &mut core.region_graph,
        vec![
            Vector3::new(3_596.0, 0.0, 0.0),
            Vector3::new(4_596.0, 0.0, 0.0),
        ],
        1,
        1,
        EdgeClass::Standard,
        &mut core.zoning,
        &mut core.allocator,
    );
    core.transit_network
        .road_surface
        .compile_dirty(&core.region_graph, &core.heightmap);
    let distant_farm = core
        .place_industry_building_internal("test:field_test", 4_096.0, 8.0)
        .unwrap();
    let distant_polygon: Vec<_> = polygon
        .iter()
        .map(|p| *p + Vector2::new(4_096.0, 0.0))
        .collect();
    core.commit_field_polygon_internal(distant_farm, distant_polygon.clone())
        .unwrap();
    core.commit_field_polygon_internal(farm, polygon).unwrap();
    core.publish_pending_building_site_changes();
    let point = DVec2::new(120.0, 42.0);
    assert!(core.prepare_cell_chunk_internal((0, 0)));
    assert!(core.zoning.cells.pick(point).is_none());
    assert!(core.prepare_cell_chunk_internal((8, 0)));
    let distant_state = core.zoning.cells.chunk_state((8, 0));
    assert!(core.push_building_removal_undo(farm));
    assert!(core.allocator.remove_building_for_bulldoze(
        farm,
        &mut core.zoning,
        &mut core.agents,
        &mut core.households,
        &mut core.logistics,
        &mut core.treasury.balance,
    ));
    core.publish_pending_building_site_changes();
    core.seal_building_removal_undo(0.0);
    assert_eq!(
        core.agriculture
            .site_for_building(farm)
            .unwrap()
            .polygon_world,
        distant_polygon
    );
    assert_eq!(core.zoning.cells.chunk_state((8, 0)), distant_state);
    assert!(core.prepare_cell_chunk_internal((0, 0)));
    assert!(
        core.zoning.cells.pick(point).is_some(),
        "removing the owner must release its entire field, beyond the farm lot"
    );
    assert!(core.undo_action_internal());
    assert_eq!(
        core.agriculture
            .site_for_building(distant_farm)
            .unwrap()
            .polygon_world,
        distant_polygon
    );
    assert_eq!(core.zoning.cells.chunk_state((8, 0)), distant_state);
    assert!(core.prepare_cell_chunk_internal((0, 0)));
    assert!(
        core.zoning.cells.pick(point).is_none(),
        "restoring the owner must hide cells under the whole restored field"
    );
}

#[test]
fn demolition_undo_preserves_a_newer_resize_of_the_swap_moved_field() {
    use glam::DVec2;

    let (mut core, first, polygon) = farm_fixture();
    let moved_farm = core
        .place_industry_building_internal("test:field_test", 120.0, 8.0)
        .unwrap();
    let polygon: Vec<_> = polygon
        .into_iter()
        .map(|p| p + Vector2::new(120.0, 0.0))
        .collect();
    core.commit_field_polygon_internal(moved_farm, polygon.clone())
        .unwrap();
    assert!(core.push_building_removal_undo(first));
    assert!(core.allocator.remove_building_for_bulldoze(
        first,
        &mut core.zoning,
        &mut core.agents,
        &mut core.households,
        &mut core.logistics,
        &mut core.treasury.balance,
    ));
    core.publish_pending_building_site_changes();
    core.seal_building_removal_undo(0.0);
    let smaller = rectangle(polygon[0].x, polygon[0].y, 40.0, 500.0);
    let center = Vector2::new(
        core.allocator.buildings[first].center_x,
        core.allocator.buildings[first].center_y,
    );
    core.resize_field_polygon_internal(first, center, &polygon, smaller.clone())
        .unwrap();
    assert!(core.prepare_cell_chunk_internal((0, 0)));
    assert!(core.zoning.cells.pick(DVec2::new(240.0, 42.0)).is_some());
    let state = core.zoning.cells.chunk_state((0, 0));
    let dependencies = core.cell_tool_dependencies();
    assert!(!core.undo_action_internal());
    assert_eq!(core.allocator.buildings.len(), 1);
    assert_eq!(
        core.agriculture
            .site_for_building(first)
            .unwrap()
            .polygon_world,
        smaller
    );
    assert_eq!(core.zoning.cells.chunk_state((0, 0)), state);
    assert_eq!(core.cell_tool_dependencies(), dependencies);
}

#[test]
fn demolition_undo_rechecks_authored_parcels_inside_the_removed_field() {
    use crate::simulation::agriculture::PolygonFootprint;

    for (profile, x, width, depth, touches_field) in [0, 1].into_iter().flat_map(|profile| {
        [
            (profile, 120.0, 20.0, 80.0, true),
            (profile, 0.0, 10.0, 10.0, false),
        ]
    }) {
        let (mut core, farm, polygon) = farm_fixture();
        core.commit_field_polygon_internal(farm, polygon.clone())
            .unwrap();
        assert!(core.push_building_removal_undo(farm));
        assert!(core.allocator.remove_building_for_bulldoze(
            farm,
            &mut core.zoning,
            &mut core.agents,
            &mut core.households,
            &mut core.logistics,
            &mut core.treasury.balance,
        ));
        core.publish_pending_building_site_changes();
        core.seal_building_removal_undo(0.0);
        // Authored-parcel placement does not add an undo entry, matching its Godot API.
        let parcel = core
            .zoning
            .place_or_rezone_parcel_at(x, 8.0, profile, width, depth, &core.region_graph)
            .unwrap();
        assert_eq!(
            core.zoning
                .overlaps_reservation(&PolygonFootprint::new(&polygon)),
            touches_field
        );
        let dependencies = core.cell_tool_dependencies();
        let balance = core.treasury.balance;
        assert!(
            !core.undo_action_internal(),
            "restoring a field must not overlap a later authored parcel, including profile zero"
        );
        assert!(core.allocator.buildings.is_empty());
        assert!(core.agriculture.sites().is_empty());
        assert_eq!(core.treasury.balance, balance);
        assert_eq!(core.cell_tool_dependencies(), dependencies);
        assert!(core.zoning.parcel_by_raw_id(parcel.raw()).is_some());
        core.zoning
            .remove_parcels_by_raw_ids(&HashSet::from([parcel.raw()]));
        assert!(
            core.undo_action_internal(),
            "rejected undo retains its journal"
        );
        assert_eq!(
            core.agriculture
                .site_for_building(farm)
                .unwrap()
                .polygon_world,
            polygon
        );
    }
}

#[test]
fn demolition_undo_rechecks_another_fields_later_expansion() {
    let (mut core, farm, polygon) = farm_fixture();
    let neighbor = core
        .place_industry_building_internal("test:field_test", 480.0, 8.0)
        .unwrap();
    let moved = core
        .place_industry_building_internal("test:field_test", -400.0, 8.0)
        .unwrap();
    assert_ne!(neighbor, moved);
    core.commit_field_polygon_internal(farm, polygon.clone())
        .unwrap();
    let original_neighbor = rectangle(472.0, polygon[0].y, 40.0, 500.0);
    core.commit_field_polygon_internal(neighbor, original_neighbor.clone())
        .unwrap();
    assert!(core.push_building_removal_undo(farm));
    assert!(core.allocator.remove_building_for_bulldoze(
        farm,
        &mut core.zoning,
        &mut core.agents,
        &mut core.households,
        &mut core.logistics,
        &mut core.treasury.balance,
    ));
    core.publish_pending_building_site_changes();
    core.seal_building_removal_undo(0.0);
    let expanded = rectangle(380.0, polygon[0].y, 132.0, 500.0);
    let center = Vector2::new(
        core.allocator.buildings[neighbor].center_x,
        core.allocator.buildings[neighbor].center_y,
    );
    core.resize_field_polygon_internal(neighbor, center, &original_neighbor, expanded.clone())
        .unwrap();
    let dependencies = core.cell_tool_dependencies();
    assert!(
        !core.undo_action_internal(),
        "field restoration must recheck changed owners outside the undo's swap pair"
    );
    assert_eq!(core.allocator.buildings.len(), 2);
    assert_eq!(core.cell_tool_dependencies(), dependencies);
    assert!(core.agriculture.site_for_building(farm).is_none());
    assert_eq!(
        core.agriculture
            .site_for_building(neighbor)
            .unwrap()
            .polygon_world,
        expanded
    );
    core.resize_field_polygon_internal(neighbor, center, &expanded, original_neighbor)
        .unwrap();
    assert!(core.undo_action_internal());
    assert_eq!(
        core.agriculture
            .site_for_building(farm)
            .unwrap()
            .polygon_world,
        polygon
    );
}

#[test]
#[ignore = "field edit and cached-cell refresh locality; run release alone without profiling"]
fn field_cell_cache_edit_scaling() {
    use crate::simulation::zoning::cells::{CellKey, CellStore};
    use glam::DVec2;
    use std::time::Instant;

    let (mut core, farm, polygon) = farm_fixture();
    let point = DVec2::new(120.0, 42.0);
    assert!(core.prepare_cell_chunk_internal((0, 0)));
    let grid = core.zoning.cells.pick(point).unwrap().grid;
    let frame = core.zoning.cells.frame(grid).unwrap();
    let smaller = rectangle(polygon[0].x, polygon[0].y, 40.0, 500.0);
    let mut previous_count = 0;
    let mut remote_chunks = std::collections::BTreeSet::new();
    let mut previous_local = None;
    for background in [0, 1_000, 10_000, 100_000] {
        // Retained paint need not have visible frontage. Match the existing populated-city
        // fixtures while isolating field edits from unrelated road/building construction.
        for index in previous_count..background {
            let key = CellKey {
                grid,
                x: 200 + (index % 700) as i32,
                y: 200 + (index / 700) as i32,
            };
            assert!(core.zoning.cells.restore_saved_cell(key, 1));
            let center = frame.world(f64::from(key.x) + 0.5, f64::from(key.y) + 0.5);
            remote_chunks.insert((
                (center.x / 512.0).floor() as i32,
                (center.y / 512.0).floor() as i32,
            ));
        }
        previous_count = background;
        for &chunk in &remote_chunks {
            core.zoning
                .cells
                .complete_generated_chunk(chunk, CellStore::chunk_bounds(chunk));
        }
        let remote_states: Vec<_> = remote_chunks
            .iter()
            .map(|&chunk| (chunk, core.zoning.cells.chunk_state(chunk)))
            .collect();
        let saved = core.zoning.cells.saved_cells();
        core.commit_field_polygon_internal(farm, smaller.clone())
            .unwrap();
        let mut commit_samples = Vec::with_capacity(42);
        let mut refresh_samples = Vec::with_capacity(42);
        for iteration in 0..24 {
            for (shape, hidden) in [(&polygon, true), (&smaller, false)] {
                let input = shape.clone();
                let start = Instant::now();
                core.commit_field_polygon_internal(farm, input).unwrap();
                let commit_ms = start.elapsed().as_secs_f64() * 1_000.0;
                let start = Instant::now();
                assert!(core.prepare_cell_chunk_internal((0, 0)));
                let refresh_ms = start.elapsed().as_secs_f64() * 1_000.0;
                assert_eq!(core.zoning.cells.pick(point).is_none(), hidden);
                if iteration >= 3 {
                    commit_samples.push(commit_ms);
                    refresh_samples.push(refresh_ms);
                }
            }
        }
        assert_eq!(core.zoning.cells.saved_cells(), saved);
        for (chunk, state) in remote_states {
            assert_eq!(core.zoning.cells.chunk_state(chunk), state);
        }
        let mut local = Vec::new();
        core.zoning
            .cells
            .visit_chunk_cells((0, 0), |key| local.push(key));
        local.sort_unstable();
        if let Some(previous) = &previous_local {
            assert_eq!(&local, previous);
        }
        previous_local = Some(local);
        commit_samples.sort_by(f64::total_cmp);
        refresh_samples.sort_by(f64::total_cmp);
        println!(
            "FIELD_CELL_CACHE_SCALING {}",
            serde_json::json!({
                "remote_painted_cells": background, "remote_chunks": remote_chunks.len(),
                "samples": 42, "rayon_threads": rayon::current_num_threads(),
                "commit_p50_ms": (commit_samples[20] + commit_samples[21]) * 0.5,
                "commit_p95_ms": commit_samples[39],
                "refresh_p50_ms": (refresh_samples[20] + refresh_samples[21]) * 0.5,
                "refresh_p95_ms": refresh_samples[39],
                "identical_local_products": true, "remote_paint_and_versions_unchanged": true,
            })
        );
    }
}

#[test]
fn ordinary_building_removal_remaps_saved_field_owner_and_reservation() {
    let (mut core, first, polygon) = farm_fixture();
    let farm = core
        .place_industry_building_internal("test:field_test", 120.0, 8.0)
        .unwrap();
    let polygon: Vec<_> = polygon
        .into_iter()
        .map(|p| p + Vector2::new(120.0, 0.0))
        .collect();
    core.commit_field_polygon_internal(farm, polygon.clone())
        .unwrap();
    let household = add_farm_household(&mut core, farm);
    assert!(core.allocator.remove_building_for_bulldoze(
        first,
        &mut core.zoning,
        &mut core.agents,
        &mut core.households,
        &mut core.logistics,
        &mut core.treasury.balance
    ));
    core.publish_pending_building_site_changes();
    assert_eq!(
        core.households.households[household].home_building_id,
        first
    );
    assert!(core.agents.home_building.iter().all(|&home| home == first));
    assert_eq!(core.agents.work_building[0], first);
    assert!(core.agriculture.site_for_building(farm).is_none());
    assert_eq!(
        core.agriculture
            .site_for_building(first)
            .unwrap()
            .polygon_world,
        polygon
    );
    assert!(
        core.validate_field_polygon_internal(first, &polygon)
            .is_ok()
    );
    assert!(core.allocator.remove_building_for_bulldoze(
        first,
        &mut core.zoning,
        &mut core.agents,
        &mut core.households,
        &mut core.logistics,
        &mut core.treasury.balance
    ));
    core.publish_pending_building_site_changes();
    assert!(core.agriculture.sites().is_empty());
    assert!(core.allocator.field_clearance.is_empty());
    assert_eq!(
        core.households.households[household].home_building_id,
        usize::MAX
    );
    assert!(core.agents.household_id.iter().all(|&id| id == household));
    assert_eq!(core.households.households[household].member_count, 4);
    assert!(
        core.agents
            .home_building
            .iter()
            .all(|&home| home == usize::MAX)
    );
}

#[test]
fn saved_fields_rebuild_clearance_after_sqlite_load() {
    let (mut core, farm, polygon) = farm_fixture();
    core.commit_field_polygon_internal(farm, polygon.clone())
        .unwrap();
    let household = add_farm_household(&mut core, farm);
    let path = temp_save_path("field_clearance");
    core.save_game_internal(path.to_str().unwrap(), None)
        .unwrap();
    let mut loaded = test_core();
    loaded.allocator.registry = core.allocator.registry.clone();
    loaded.load_game_internal(path.to_str().unwrap()).unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(
        loaded.households.households[household].home_building_id,
        farm
    );
    assert_eq!(loaded.households.households[household].member_count, 4);
    assert_eq!(loaded.allocator.household_capacity(farm), 1);
    assert_eq!(loaded.allocator.buildings[farm].occupancy, 1);
    assert!(loaded.agents.home_building.iter().all(|&home| home == farm));
    assert_eq!(loaded.agents.work_building[0], farm);
    assert_eq!(
        loaded
            .agriculture
            .site_for_building(farm)
            .unwrap()
            .polygon_world,
        polygon
    );
    assert!(
        loaded
            .allocator
            .field_clearance
            .overlaps_polygon(&rectangle(100.0, 100.0, 10.0, 10.0))
    );
    assert_eq!(
        loaded
            .allocator
            .worker_capacity_with_catalog(farm, loaded.demand.runtime_catalog()),
        2
    );
}

#[test]
fn demolition_undo_restores_field_land_and_area() {
    let (mut core, farm, polygon) = farm_fixture();
    core.commit_field_polygon_internal(farm, polygon.clone())
        .unwrap();
    let household = add_farm_household(&mut core, farm);
    assert!(core.push_building_removal_undo(farm));
    assert!(core.allocator.remove_building_for_bulldoze(
        farm,
        &mut core.zoning,
        &mut core.agents,
        &mut core.households,
        &mut core.logistics,
        &mut core.treasury.balance
    ));
    core.publish_pending_building_site_changes();
    core.seal_building_removal_undo(0.0);
    assert!(core.agriculture.sites().is_empty());
    assert!(core.allocator.field_clearance.is_empty());
    assert!(core.undo_action_internal());
    assert_eq!(core.households.households[household].home_building_id, farm);
    assert_eq!(core.allocator.buildings[farm].occupancy, 1);
    assert!(core.agents.home_building.iter().all(|&home| home == farm));
    assert_eq!(core.agents.work_building[0], farm);
    assert_eq!(
        core.agriculture
            .site_for_building(farm)
            .unwrap()
            .polygon_world,
        polygon
    );
    assert!(
        core.allocator
            .field_clearance
            .overlaps_polygon(&rectangle(100.0, 100.0, 10.0, 10.0))
    );
    assert_eq!(core.allocator.buildings[farm].work_area_scale, 20.0);
}
