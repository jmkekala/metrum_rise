// SPDX-License-Identifier: GPL-2.0-only

//! A construction site's parking in a played town (`PERF-25`): its lot is no supply and its door
//! holds no list until the building is finished; completion publishes both as a rebuild from
//! scratch has them, a site bulldozed before completion leaves nothing behind, and a game saved
//! in between completes as the unbroken one does, on one worker as on eight.

use super::save_cases::{differing_tables, save};
use super::*;
use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
use crate::simulation::parking::lots::lot_bays;
use std::path::PathBuf;

fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "metrum-construction-parking-{name}-{}.sqlite",
        std::process::id()
    ))
}

fn assert_same_saves(first: &Path, second: &Path, what: &str) {
    assert!(
        std::fs::read(first).unwrap() == std::fs::read(second).unwrap(),
        "{what}: tables differing in storage order: {:?}",
        differing_tables(first, second)
    );
}

// The bays building `site`'s lot validates now, whether or not they count as supply.
fn lot_size(core: &SimCore, site: usize) -> usize {
    lot_bays(
        &core.allocator,
        site,
        &core.region_graph,
        &core.transit_network.roadbed,
    )
    .0
    .len()
}

// Steps the town until a spawn leaves a construction site whose lot has bays, and returns it.
fn site_with_a_lot(core: &mut SimCore) -> usize {
    loop {
        assert!(core.time.day_index < 10, "no construction site with a lot");
        core.step_frame(FRAME_DT_S);
        if let Some(site) = (0..core.allocator.buildings.len()).find(|&b| {
            core.allocator.buildings[b].is_under_construction() && lot_size(core, b) > 0
        }) {
            return site;
        }
    }
}

// Asserts that building `site` has no parking: no bay, no list and no economy ends.
fn assert_no_parking(core: &SimCore, site: usize) {
    let handle = core.allocator.building_handles.handle(site).to_bits();
    assert_eq!(core.parking.building_place_count(handle), 0, "the site's bays are supply");
    assert!(core.parking.candidate_products(handle).is_empty(), "the site's door has a list");
    assert!(core.parking.economy_ends(handle).is_none());
}

// Steps until construction site `site` has completed, and returns its index then.
fn complete(core: &mut SimCore, site: usize) -> usize {
    let handle = core.allocator.building_handles.handle(site);
    let deadline = core.allocator.buildings[site].construction_complete_slot;
    while core.time.absolute_slot() < deadline {
        core.step_frame(FRAME_DT_S);
    }
    let site = core.allocator.building_handles.resolve(handle).expect("the finished site");
    assert!(!core.allocator.buildings[site].is_under_construction());
    site
}

#[test]
fn a_construction_site_has_no_parking_until_completion_publishes_what_a_rebuild_gives() {
    let mut town = GrowthScenario::starter_town();
    let core = &mut town.core;
    let site = site_with_a_lot(core);
    assert!(core.transit_network.cch_current(), "the site's own publication");
    assert_no_parking(core, site);
    // Every door, the site's included, holds what a rebuild gives: the site's none.
    core.audit_every_parking_catchment();
    let site = complete(core, site);
    let handle = core.allocator.building_handles.handle(site).to_bits();
    assert_eq!(core.parking.building_place_count(handle), lot_size(core, site));
    assert!(!core.parking.candidate_products(handle).is_empty(), "the door lists its own bays");
    core.audit_every_parking_catchment();
}

#[test]
fn a_construction_site_bulldozed_before_completion_leaves_no_parking_behind() {
    let mut town = GrowthScenario::starter_town();
    let core = &mut town.core;
    let site = site_with_a_lot(core);
    let handle = core.allocator.building_handles.handle(site);
    let deadline = core.allocator.buildings[site].construction_complete_slot;
    assert!(core.bulldoze_building(site));
    assert!(core.allocator.building_handles.resolve(handle).is_none());
    assert_eq!(core.parking.building_place_count(handle.to_bits()), 0);
    assert!(core.parking.candidate_products(handle.to_bits()).is_empty());
    core.audit_every_parking_catchment();
    // Its deadline passes without a completion: no bay appears, every list stays exact.
    while core.time.absolute_slot() <= deadline {
        core.step_frame(FRAME_DT_S);
    }
    assert_eq!(core.parking.building_place_count(handle.to_bits()), 0);
    core.audit_every_parking_catchment();
}

#[test]
fn a_game_saved_mid_construction_completes_as_the_unbroken_game_does() {
    let mut source = GrowthScenario::starter_town();
    let site = site_with_a_lot(&mut source.core);
    let saved = temp("mid");
    save(&mut source.core, &saved);
    let mut loaded = GrowthScenario::starter_town();
    loaded
        .core
        .load_game_internal(saved.to_str().unwrap())
        .unwrap();
    loaded.core.time.speed_multiplier = source.core.time.speed_multiplier;
    assert!(loaded.core.allocator.buildings[site].is_under_construction());
    assert_no_parking(&loaded.core, site);
    // To the completion and an hour past it.
    let end = source.core.allocator.buildings[site].construction_complete_slot + SLOTS_PER_HOUR;
    while source.core.time.absolute_slot() < end {
        source.core.step_frame(FRAME_DT_S);
        loaded.core.step_frame(FRAME_DT_S);
    }
    let site = source.core.allocator.building_handles.handle(site).to_bits();
    assert!(source.core.parking.building_place_count(site) > 0, "the lot is published");
    let (first, second) = (temp("continued"), temp("continued-loaded"));
    save(&mut source.core, &first);
    save(&mut loaded.core, &second);
    assert_same_saves(&first, &second, "the saved and the loaded game");
    for path in [saved, first, second] {
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn a_construction_site_completes_identically_on_one_and_eight_workers() {
    let saves = [1, 8].map(|workers| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| {
                let mut town = GrowthScenario::starter_town();
                let site = site_with_a_lot(&mut town.core);
                complete(&mut town.core, site);
                let path = temp(&format!("workers-{workers}"));
                save(&mut town.core, &path);
                path
            })
    });
    assert_same_saves(&saves[0], &saves[1], "1 and 8 workers");
    for path in saves {
        std::fs::remove_file(path).unwrap();
    }
}
