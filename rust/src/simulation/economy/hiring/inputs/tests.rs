// SPDX-License-Identifier: GPL-2.0-only

//! Census input copy rows, alignment and encoding (`SIM-02-I`).

use super::*;
use crate::simulation::buildings::allocator::indexed_test_building;
use crate::simulation::economy::agents::AgentHandle;
use crate::simulation::economy::definitions::load_runtime_economy_catalog;
use crate::simulation::zoning::ZoneType;

const BUILDINGS: usize = 40;
const HOUSEHOLDS: usize = 50;

// A household told apart by `tag`, at home in `home`.
fn household(tag: u32, home: usize) -> Household {
    Household {
        home_building_id: home,
        power_arrears: Default::default(),
        budget: 100.0 + tag as f32,
        stock: 0.5,
        member_count: 2,
        child_count: 0,
        adult_count: 2,
        employed_adults: 1,
        gross_daily_wages: 3.25,
        shopping_has_car: tag.is_multiple_of(2),
        elder_count: 0,
        consumption_rate: 1.0,
        stock_days: 2.0,
        replenishment_state: 0,
        cooldown_hours: 0,
        replenishment_failure_count: 0,
        reserved_store_building_id: usize::MAX,
        reserved_amount: 0.0,
        reserved_total_cost: 0.0,
        shopping_agent: AgentHandle::NONE,
        shopping_timeout_hours_remaining: 0,
        replenishment_search_cursor: tag,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: (tag % 24) as u16,
        unemployment_days_elapsed: 0,
    }
}

fn city() -> (BuildingAllocator, Vec<Household>) {
    let mut allocator = BuildingAllocator::new();
    for index in 0..BUILDINGS {
        let mut building = indexed_test_building(
            format!("test:building.{index}"),
            ZoneType::Residential,
            index as i32 + 1,
        );
        building.resource_inventory = vec![index as f32; 4];
        allocator.buildings.push(building);
    }
    let households = (0..HOUSEHOLDS)
        .map(|tag| household(tag as u32, tag % BUILDINGS))
        .collect();
    (allocator, households)
}

// Runs the fill's chunks `chunks` of the hour starting at slot 0.
fn advance(
    inputs: &mut CensusInputs,
    allocator: &mut BuildingAllocator,
    households: &[Household],
    chunks: std::ops::Range<u64>,
) {
    for slot in chunks {
        inputs.advance(
            slot,
            allocator,
            households,
            &ShipmentSystem::new(),
            FrozenCensusParameters::default(),
        );
    }
}

// Removes building `removed` with the hooks and live retargeting of both removal paths.
fn remove_building(
    inputs: &mut CensusInputs,
    allocator: &mut BuildingAllocator,
    households: &mut [Household],
    removed: usize,
) {
    inputs.remove_building(removed, allocator);
    let last = allocator.buildings.len() - 1;
    for household in households.iter_mut() {
        if household.home_building_id == removed {
            household.home_building_id = usize::MAX;
        }
    }
    if removed < last {
        inputs.remap_buildings(&HashMap::from([(last, removed)]));
        for household in households.iter_mut() {
            if household.home_building_id == last {
                household.home_building_id = removed;
            }
        }
    }
    allocator.buildings.swap_remove(removed);
}

fn remove_household(inputs: &mut CensusInputs, households: &mut Vec<Household>, removed: usize) {
    inputs.remove_household(removed, households);
    households.swap_remove(removed);
}

// Each copied row is the live entity at its index, with the live references.
fn assert_rows_match(copy: (&[Building], &[Household]), live: (&[Building], &[Household])) {
    assert!(copy.0.len() <= live.0.len() && copy.1.len() <= live.1.len());
    for (row, building) in copy.0.iter().zip(live.0) {
        assert_eq!(row.parcel_id, building.parcel_id);
        assert_eq!(row.asset_id, building.asset_id);
    }
    for (row, household) in copy.1.iter().zip(live.1) {
        assert_eq!(
            (row.replenishment_search_cursor, row.home_building_id),
            (
                household.replenishment_search_cursor,
                household.home_building_id
            )
        );
    }
}

fn back_rows(inputs: &CensusInputs) -> (&[Building], &[Household]) {
    (inputs.back.buildings.rows(), inputs.back.households.rows())
}

fn front_rows(inputs: &CensusInputs) -> (&[Building], &[Household]) {
    (&inputs.front.allocator.buildings, &inputs.front.households)
}

#[test]
fn copies_follow_swap_removes_on_both_sides_of_the_cursor() {
    let (mut allocator, mut households) = city();
    let mut inputs = CensusInputs::default();
    // Chunk 119 ends a quarter of the fill: rows below 10 of 40 buildings and 12 of 50
    // households are copied.
    advance(&mut inputs, &mut allocator, &households, 0..120);
    assert_eq!(
        (inputs.back.buildings.len, inputs.back.households.len),
        (10, 12)
    );
    let cases = [
        // A copied row whose moved last entity is not copied yet.
        3, // The last copied row.
        9, // A row beyond the cursor.
        25,
    ];
    for removed in cases {
        remove_building(&mut inputs, &mut allocator, &mut households, removed);
        assert_rows_match(back_rows(&inputs), (&allocator.buildings, &households));
    }
    for removed in [2, 11, 30] {
        remove_household(&mut inputs, &mut households, removed);
        assert_rows_match(back_rows(&inputs), (&allocator.buildings, &households));
    }
    assert_eq!(
        (inputs.back.buildings.len, inputs.back.households.len),
        (10, 12)
    );
    // Live last entities beyond the cursor, until every live entity is copied: a removal then
    // moves the last copy into the removed row.
    while allocator.buildings.len() > inputs.back.buildings.len {
        let last = allocator.buildings.len() - 1;
        remove_building(&mut inputs, &mut allocator, &mut households, last);
    }
    while households.len() > inputs.back.households.len {
        let last = households.len() - 1;
        remove_household(&mut inputs, &mut households, last);
    }
    remove_building(&mut inputs, &mut allocator, &mut households, 1);
    remove_household(&mut inputs, &mut households, 1);
    assert_eq!(
        (inputs.back.buildings.len, inputs.back.households.len),
        (9, 11)
    );
    assert_rows_match(back_rows(&inputs), (&allocator.buildings, &households));
    // Entities created during the fill are copied at its last slot.
    allocator.buildings.push(indexed_test_building(
        "test:building.late".into(),
        ZoneType::Residential,
        100,
    ));
    households.push(household(100, allocator.buildings.len() - 1));
    advance(&mut inputs, &mut allocator, &households, 120..FILL_SLOTS);
    assert!(inputs.pending && inputs.back.next_chunk.is_none());
    assert_eq!(
        inputs.front.allocator.buildings.len(),
        allocator.buildings.len()
    );
    assert_eq!(inputs.front.households.len(), households.len());
    assert_rows_match(front_rows(&inputs), (&allocator.buildings, &households));
    // Between the swap and the census, removals of copied rows move their copies, and a
    // building placed since moves in without freight.
    remove_building(&mut inputs, &mut allocator, &mut households, 0);
    remove_household(&mut inputs, &mut households, 0);
    allocator.buildings.push(indexed_test_building(
        "test:building.after".into(),
        ZoneType::Residential,
        101,
    ));
    inputs.front.freight_capital.fill(7.0);
    remove_building(&mut inputs, &mut allocator, &mut households, 4);
    assert_rows_match(front_rows(&inputs), (&allocator.buildings, &households));
    assert_eq!(inputs.front.freight_capital[4], 0.0);
    let catalog = load_runtime_economy_catalog().unwrap();
    inputs.prepare(
        &mut allocator,
        &households,
        &ShipmentSystem::new(),
        &catalog,
        FrozenCensusParameters {
            treasury: -1.0,
            ..Default::default()
        },
    );
    assert_eq!(
        inputs.front.parameters.treasury, 0.0,
        "the finished copy is read, not a new one"
    );
    assert_eq!(
        inputs.front.allocator.buildings.len(),
        allocator.buildings.len()
    );
    assert_eq!(
        inputs.front.freight_capital.len(),
        allocator.buildings.len()
    );
    assert_eq!(
        inputs.front.service_funding.len(),
        allocator.buildings.len()
    );
    assert_rows_match(front_rows(&inputs), (&allocator.buildings, &households));
    assert_eq!(
        inputs.front.allocator.building_ref_revision,
        allocator.building_ref_revision
    );
}

#[test]
fn steady_hours_copy_into_the_rows_storage_of_earlier_hours() {
    let (mut allocator, households) = city();
    let mut inputs = CensusInputs::default();
    // Addresses of every row store and of each building's asset and inventory storage.
    let storage = |inputs: &CensusInputs| {
        let mut addresses = vec![
            inputs.front.allocator.buildings.as_ptr() as usize,
            inputs.back.buildings.items.as_ptr() as usize,
            inputs.front.households.as_ptr() as usize,
            inputs.back.households.items.as_ptr() as usize,
            inputs.front.freight_capital.as_ptr() as usize,
        ];
        for building in inputs
            .front
            .allocator
            .buildings
            .iter()
            .chain(&inputs.back.buildings.items)
        {
            addresses.push(building.asset_id.as_ptr() as usize);
            addresses.push(building.resource_inventory.as_ptr() as usize);
        }
        addresses.sort_unstable();
        addresses
    };
    let mut hour = |inputs: &mut CensusInputs| {
        advance(inputs, &mut allocator, &households, 0..SLOTS_PER_HOUR);
        assert!(inputs.pending);
        inputs.consume();
    };
    // Two hours grow both copies; later hours reuse their storage.
    hour(&mut inputs);
    hour(&mut inputs);
    let warm = storage(&inputs);
    hour(&mut inputs);
    hour(&mut inputs);
    assert_eq!(storage(&inputs), warm);
}

#[test]
fn rows_changed_in_place_are_copied_again_and_other_remaps_drop_the_copies() {
    let (mut allocator, households) = city();
    let mut inputs = CensusInputs::default();
    advance(&mut inputs, &mut allocator, &households, 0..240);
    allocator.buildings[5].asset_id = "test:building.upgraded".into();
    allocator.buildings[30].asset_id = "test:building.later".into();
    allocator.census_copy_changes.rows.extend([5, 30]);
    advance(&mut inputs, &mut allocator, &households, 240..241);
    assert_eq!(
        inputs.back.buildings.rows()[5].asset_id,
        "test:building.upgraded"
    );
    assert!(allocator.census_copy_changes.rows.is_empty());
    allocator.buildings[6].asset_id = "test:building.everywhere".into();
    allocator.census_copy_changes.all = true;
    advance(&mut inputs, &mut allocator, &households, 241..FILL_SLOTS);
    assert_rows_match(front_rows(&inputs), (&allocator.buildings, &households));
    // A remap the copy did not announce, as undo makes, drops both copies.
    inputs.remap_buildings(&HashMap::from([(1, 2), (2, 1)]));
    assert!(!inputs.pending && inputs.back.next_chunk.is_none());
    // A missed slot abandons the fill.
    advance(&mut inputs, &mut allocator, &households, 0..10);
    advance(&mut inputs, &mut allocator, &households, 11..12);
    assert!(inputs.back.next_chunk.is_none());
}

#[test]
fn saved_copies_restore_bit_for_bit() {
    let (mut allocator, mut households) = city();
    let mut inputs = CensusInputs::default();
    let building = &mut allocator.buildings[7];
    building.edge_idx = 3;
    building.resource_inventory = vec![1.5, 0.0, 2.25];
    building.zone_type = ZoneType::Commercial;
    building.side = -1;
    building.worker_count = 7;
    building.funded_activity = 0.375;
    households[4].reserved_store_building_id = 7;
    households[4].power_arrears.imported = 0.125;
    households[4].shopping_agent = AgentHandle::from_parts(9, 2);
    let parameters = FrozenCensusParameters {
        treasury: 12_345.678,
        fiscal: CityFiscalPolicy::default(),
        electricity_funding: 0.4,
        minute_of_day: 420,
    };
    for slot in 0..SLOTS_PER_HOUR {
        inputs.advance(
            slot,
            &mut allocator,
            &households,
            &ShipmentSystem::new(),
            parameters,
        );
    }
    advance(&mut inputs, &mut allocator, &households, 0..250);
    allocator.census_copy_changes.rows.extend([2, 2, 39]);
    // Edge 3 saves as 1; edges the save drops become placeholders.
    let edge = |edge: usize| if edge == 3 { 1 } else { u64::MAX };
    let saved = inputs.save(&allocator.census_copy_changes, edge);
    assert!(saved.front.is_some() && saved.back.is_some());
    let mut restored = CensusInputs::restore(&saved).unwrap();
    assert_eq!(
        restored.save(&allocator.census_copy_changes, |edge| edge as u64),
        saved
    );
    assert_eq!(restored.front.allocator.buildings[7].edge_idx, 1);
    assert_eq!(restored.front.allocator.buildings[0].edge_idx, usize::MAX);
    assert_eq!(restored.front.parameters.treasury, parameters.treasury);
    assert_eq!(restored.back.next_chunk, Some(250));
    assert_eq!(restored.back.buildings.len, inputs.back.buildings.len);
    assert_eq!(
        restored.front.households[4].shopping_agent,
        households[4].shopping_agent
    );
    let mut loaded = BuildingAllocator::new();
    loaded.census_copy_changes.all = true;
    restored.finish_load(&mut loaded);
    assert_eq!(loaded.census_copy_changes, allocator.census_copy_changes);
    let mut truncated = saved.clone();
    truncated.back.as_mut().unwrap().pop();
    assert!(CensusInputs::restore(&truncated).is_err());
}

#[test]
fn a_read_front_defers_rows_changed_in_place_until_a_restart() {
    let (mut allocator, households) = city();
    let mut inputs = CensusInputs::default();
    advance(&mut inputs, &mut allocator, &households, 0..FILL_SLOTS);
    let catalog = load_runtime_economy_catalog().unwrap();
    let prepare = |inputs: &mut CensusInputs, allocator: &mut BuildingAllocator| {
        inputs.prepare(
            allocator,
            &households,
            &ShipmentSystem::new(),
            &catalog,
            FrozenCensusParameters::default(),
        );
    };
    prepare(&mut inputs, &mut allocator);
    assert!(inputs.is_read());
    // In place and appended while the census reads the front: neither joins it.
    allocator.buildings[5].asset_id = "test:building.upgraded".into();
    allocator.census_copy_changes.rows.push(5);
    allocator.buildings.push(indexed_test_building(
        "test:building.late".into(),
        ZoneType::Residential,
        100,
    ));
    advance(
        &mut inputs,
        &mut allocator,
        &households,
        FILL_SLOTS..FILL_SLOTS + 2,
    );
    assert_eq!(
        inputs.front.allocator.buildings[5].asset_id,
        "test:building.5"
    );
    assert_eq!(inputs.front_len().0, BUILDINGS);
    assert_eq!(inputs.deferred.rows, [5]);
    // The deferred rows survive a save.
    let saved = inputs.save(&allocator.census_copy_changes, |edge| edge as u64);
    let restored = CensusInputs::restore(&saved).unwrap();
    assert!(restored.is_read() && restored.deferred.rows == [5]);
    // A restart takes both.
    prepare(&mut inputs, &mut allocator);
    assert_eq!(
        inputs.front.allocator.buildings[5].asset_id,
        "test:building.upgraded"
    );
    assert_eq!(inputs.front_len().0, BUILDINGS + 1);
    assert!(inputs.deferred.rows.is_empty() && inputs.is_read());
    inputs.consume();
    assert!(!inputs.is_read() && !inputs.pending);
}
