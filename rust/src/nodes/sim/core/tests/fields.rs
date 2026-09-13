// SPDX-License-Identifier: GPL-2.0-only

//! Field land reservations, atomic vertex edits and production-site lifecycle regressions.

use super::*;
use crate::assets::asset::BuildingFieldData;
use crate::simulation::buildings::allocator::{
    BuildingSiteEnvironment, ExplicitServicePlacementRejection,
};
use godot::prelude::Vector2;

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
            .contains("zoning parcel")
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
