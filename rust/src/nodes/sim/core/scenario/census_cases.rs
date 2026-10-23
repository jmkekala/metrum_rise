// SPDX-License-Identifier: GPL-2.0-only

//! The funding census's rolling input copy and the census over the hour's second half in a
//! growing town, and the demand snapshot front planning reads beside it (`SIM-02-I`).

use super::*;
use crate::nodes::sim::core::schedule::slot_calendar;
use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::economy::hiring::{
    CENSUS_WINDOW_START, CensusProgress, FILL_SLOTS, FrozenCensusParameters, JobFunding,
};

// A town busy with homes, businesses and freight on the road.
pub(super) fn busy_town() -> GrowthScenario {
    let mut scenario = GrowthScenario::starter_town();
    scenario.run_days(8);
    let in_transit = |core: &SimCore| {
        core.logistics
            .shipments
            .iter()
            .any(|shipment| shipment.status.reserves_cargo())
    };
    let end = scenario.day() + 2;
    while !in_transit(&scenario.core) && scenario.day() < end {
        scenario.core.step_frame(FRAME_DT_S);
    }
    let core = &scenario.core;
    assert!(core.allocator.buildings.len() > 10 && core.households.households.len() > 10);
    assert!(in_transit(core));
    scenario
}

// The first slot of the hour after the clock's.
fn next_hour_slot(core: &SimCore) -> u64 {
    (core.time.absolute_slot() / SLOTS_PER_HOUR + 1) * SLOTS_PER_HOUR
}

#[test]
fn the_census_on_the_input_copy_equals_the_live_census_over_an_unchanged_hour() {
    let mut scenario = busy_town();
    let core = &mut scenario.core;
    let start = next_hour_slot(core);
    for slot in start..start + FILL_SLOTS {
        core.advance_census_inputs(slot);
    }
    let inputs = &core.households.census_inputs;
    assert!(inputs.has_pending_front());
    assert!(inputs.front_equals(&core.allocator, &core.households.households));
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let service_funding = core.electricity_funding_by_building();
    let minute_of_day = slot_calendar(start + SLOTS_PER_HOUR).1;
    let mut inputs = std::mem::take(&mut core.households.census_inputs);
    // The finished copy's frozen parameters apply, not these.
    inputs.prepare(
        &mut core.allocator,
        &core.households.households,
        &core.logistics,
        &catalog,
        FrozenCensusParameters {
            treasury: -1.0,
            ..Default::default()
        },
    );
    let mut copied = JobFunding::default();
    copied.refresh_from_inputs(
        &mut inputs,
        &core.transit_network,
        &core.region_graph,
        &catalog,
        &tuning,
    );
    let mut live = JobFunding::default();
    live.refresh(
        &core.allocator,
        &core.households.households,
        &mut core.logistics.clone(),
        &core.transit_network,
        &core.region_graph,
        &catalog,
        &tuning,
        &core.fiscal_policy,
        &service_funding,
        core.treasury.balance,
        minute_of_day,
    );
    assert!(live.capacities.iter().any(|&jobs| jobs > 0));
    copied.assert_same_operations_for_test(&live);
    assert_eq!(copied.service_capacities, live.service_capacities);
    assert_eq!(copied.freight_capital, live.freight_capital);
    assert!(live.freight_capital.iter().any(|&capital| capital > 0.0));
    let books = |funding: &JobFunding| {
        [
            funding.customers.live_fingerprint_for_test(),
            funding.business.live_fingerprint_for_test(),
            funding.input_supply.live_fingerprint_for_test(),
        ]
    };
    assert_eq!(books(&copied), books(&live));
}

#[test]
fn the_census_in_work_unit_slices_equals_one_run_on_its_copy() {
    let mut scenario = busy_town();
    let core = &mut scenario.core;
    let start = next_hour_slot(core);
    for slot in start..start + FILL_SLOTS {
        core.advance_census_inputs(slot);
    }
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let mut inputs = std::mem::take(&mut core.households.census_inputs);
    inputs.prepare(
        &mut core.allocator,
        &core.households.households,
        &core.logistics,
        &catalog,
        FrozenCensusParameters::default(),
    );
    let mut once = JobFunding::default();
    once.refresh_from_inputs(
        &mut inputs,
        &core.transit_network,
        &core.region_graph,
        &catalog,
        &tuning,
    );
    assert!(once.capacities.iter().any(|&jobs| jobs > 0));
    // One work unit stops every step after each row; 2,000 stops a serial sweep after any row
    // that searched a route (`SIM-02-K`).
    for budget in [1, 2_000] {
        let mut sliced = JobFunding::default();
        let mut progress = CensusProgress::default();
        let mut slices = 0;
        while !sliced.step_from_inputs(
            &mut inputs,
            &mut progress,
            &core.transit_network,
            &core.region_graph,
            &catalog,
            &tuning,
            budget,
        ) {
            slices += 1;
        }
        assert!(
            slices
                > if budget == 1 {
                    core.allocator.buildings.len()
                } else {
                    1
                },
            "{budget} work units: {slices} slices"
        );
        assert_same_census(&sliced, &once, &format!("{budget} work units"));
    }
}

#[test]
fn the_input_copy_follows_building_and_household_removals() {
    let mut scenario = busy_town();
    let core = &mut scenario.core;
    // Tags tell households apart; nothing below reads them.
    for (tag, household) in core.households.households.iter_mut().enumerate() {
        household.replenishment_search_cursor = tag as u32;
    }
    let start = next_hour_slot(core);
    for slot in start..start + FILL_SLOTS / 2 {
        core.advance_census_inputs(slot);
    }
    let remove_building = |core: &mut SimCore, index: usize| {
        assert!(core.allocator.remove_building_for_bulldoze(
            index,
            &mut core.zoning,
            &mut core.agents,
            &mut core.households,
            &mut core.logistics,
            &mut core.treasury.balance,
        ));
    };
    let remove_household = |core: &mut SimCore, index: usize| {
        let mut ids = vec![index];
        core.households
            .remove_households(&mut ids, &mut core.agents, &mut core.allocator);
    };
    // A household whose shopper is out advances its search cursor and drops its store when its
    // home or store goes, live state the copy taken before keeps as copied: the buildings
    // removed are the first at or after each index that no such household uses.
    let unused_by_shoppers = |core: &SimCore, from: usize| {
        use crate::simulation::economy::households::{
            REPLENISHMENT_SHOPPING_RETURNING, REPLENISHMENT_SHOPPING_TO_STORE,
        };
        (from..core.allocator.buildings.len())
            .find(|&index| {
                !core.households.households.iter().any(|household| {
                    matches!(
                        household.replenishment_state,
                        REPLENISHMENT_SHOPPING_TO_STORE | REPLENISHMENT_SHOPPING_RETURNING
                    ) && (household.home_building_id == index
                        || household.reserved_store_building_id == index)
                })
            })
            .unwrap()
    };
    // Half the fill is copied: rows on both sides of the cursor, and the last rows.
    let buildings = core.allocator.buildings.len();
    for index in [1, buildings / 2 - 1, buildings / 2 + 1, buildings - 4] {
        let index = unused_by_shoppers(core, index);
        remove_building(core, index);
        core.households
            .census_inputs
            .assert_aligned(&core.allocator, &core.households.households);
    }
    let households = core.households.households.len();
    for index in [0, households / 2 - 1, households / 2 + 1, households - 1] {
        remove_household(core, index);
        core.households
            .census_inputs
            .assert_aligned(&core.allocator, &core.households.households);
    }
    assert!(core.households.census_inputs.is_filling());
    for slot in start + FILL_SLOTS / 2..start + FILL_SLOTS {
        core.advance_census_inputs(slot);
    }
    let inputs = &core.households.census_inputs;
    assert!(inputs.has_pending_front());
    inputs.assert_aligned(&core.allocator, &core.households.households);
    // Between the swap and the census.
    let index = unused_by_shoppers(core, 2);
    remove_building(core, index);
    remove_household(core, 3);
    core.households
        .census_inputs
        .assert_aligned(&core.allocator, &core.households.households);
    // The next hour's first slot runs the census the window did not and publishes it.
    core.publish_census(start + SLOTS_PER_HOUR);
    assert!(!core.households.census_inputs.has_pending_front());
    assert_eq!(
        core.households.job_funding.capacities.len(),
        core.allocator.buildings.len()
    );
}

// Steps short frames until slot `chunk` of an hour has just run.
fn step_to_chunk(core: &mut SimCore, chunk: u64) {
    while core.time.absolute_slot() % SLOTS_PER_HOUR != chunk {
        core.step_frame(FRAME_DT_S / 16.0);
    }
}

// The census's outputs and books, bit for bit.
pub(super) fn assert_same_census(left: &JobFunding, right: &JobFunding, label: &str) {
    left.assert_same_operations_for_test(right);
    assert_eq!(left.service_capacities, right.service_capacities, "{label}");
    assert_eq!(left.freight_capital, right.freight_capital, "{label}");
    assert_eq!(
        left.census_book_fingerprints_for_test(),
        right.census_book_fingerprints_for_test(),
        "{label}"
    );
}

// Whether the planning slots and the permit epoch lend the published census's books at the
// clock's minute, as `has_current_input_supply` decides it (`SIM-02-I`): books valid for the
// live city's key, priced for the hour's freight phase and quoted for its connected borders.
pub(super) fn published_books_lent(core: &SimCore) -> bool {
    let mut borders = Vec::new();
    crate::simulation::economy::logistics::collect_connected_border_nodes(
        &core.region_graph,
        &mut borders,
    );
    core.households.job_funding.has_current_input_supply(
        core.households.census_key(&core.transit_network),
        &borders,
        crate::simulation::economy::logistics::freight_price_phase(
            core.demand.runtime_tuning(),
            core.time.minute_of_day,
        ),
    )
}

// The census the hour's first slot published, which the hour's later planning slots, permit
// epoch, workforce reset and project reservations read (`SIM-02-I`): a game loaded before the
// hour's actions slot lends its books exactly when the saved game does, and lent books are the
// same books, funded jobs and activity, with the same buildings seen complete. Books not lent
// are read by nobody before the next publication replaces them.
pub(super) fn assert_same_published_census(source: &SimCore, loaded: &SimCore) {
    let lent = published_books_lent(source);
    assert_eq!(
        lent,
        published_books_lent(loaded),
        "published census validity"
    );
    if lent {
        let (a, b) = (
            &source.households.job_funding,
            &loaded.households.job_funding,
        );
        assert_same_census(a, b, "published census");
        // One word past the buildings covers every bit either census set.
        let rows = source
            .allocator
            .buildings
            .len()
            .max(loaded.allocator.buildings.len())
            + 64;
        assert!(
            (0..rows).all(|index| a.saw_operating(index) == b.saw_operating(index)),
            "the published census saw the same buildings complete"
        );
    }
}

#[test]
fn the_census_stepped_over_the_window_equals_one_run_on_its_copy_and_restarts_on_network_changes() {
    let mut scenario = busy_town();
    let core = &mut scenario.core;
    // The census starts at the window's first slot, after the frontage prices.
    step_to_chunk(core, CENSUS_WINDOW_START + 20);
    assert!(core.households.census_inputs.is_read());
    assert!(!core.households.census_run.is_done());
    let restarts = core.households.census_run.restarts;
    // A network change part way restarts it at the next slot.
    core.transit_network.frontage_delay_revision += 1;
    let structure = core.households.structure;
    step_to_chunk(core, CENSUS_WINDOW_START + 22);
    assert_eq!(core.households.census_run.restarts, restarts + 1);
    // Appends and in-place changes do not.
    step_to_chunk(core, SLOTS_PER_HOUR - 1);
    assert!(core.households.census_run.is_done());
    if core.households.structure == structure {
        assert_eq!(core.households.census_run.restarts, restarts + 1);
    }
    let once = core
        .households
        .census_of_front_for_test(&core.transit_network, &core.region_graph);
    assert!(once.capacities.iter().any(|&jobs| jobs > 0));
    assert_same_census(core.households.census_run.funding(), &once, "window");
    // The next hour's first slot publishes it before the operational hour.
    core.publish_census(next_hour_slot(core));
    assert_same_census(&core.households.job_funding, &once, "published");
    assert!(!core.households.census_inputs.is_read());
    assert_eq!(
        core.households.job_funding.published,
        Some(core.households.census_key(&core.transit_network))
    );
}

// Loading gives census storage that holds no census a copy of the live books, as an
// uninterrupted game's holds the previous published census, with the capacity of the arenas a
// census refills (`SIM-02-K`): what the storage held never changes the census.
#[test]
fn the_census_on_a_copy_of_the_live_books_equals_one_on_fresh_storage() {
    let mut scenario = busy_town();
    let core = &mut scenario.core;
    step_to_chunk(core, SLOTS_PER_HOUR - 1);
    assert!(core.households.job_funding.planning_prepared);
    let fresh = core
        .households
        .census_of_front_for_test(&core.transit_network, &core.region_graph);
    assert!(fresh.capacities.iter().any(|&jobs| jobs > 0));
    let live = &core.households.job_funding;
    let mut copy = live.clone();
    copy.reserve_census_arenas_like(live);
    let warm = core.households.census_of_front_on_for_test(
        copy,
        &core.transit_network,
        &core.region_graph,
    );
    assert_same_census(&warm, &fresh, "copy of the live books");
}

// Loading prepares the route scratch of the slots' trips and of the job search, and the job
// search's network-leg storage, which the first slots after it built (`SIM-02-K`). Scratch
// only memoizes storage, so a loaded game runs the same on it as on cold scratch.
#[test]
fn route_scratch_prepared_at_load_runs_as_cold_scratch_does() {
    let source = busy_town();
    let saved = std::env::temp_dir().join(format!(
        "metrum-route-scratch-{}.sqlite",
        std::process::id()
    ));
    let path = saved.to_str().unwrap();
    source.core.save_game_internal(path, None).unwrap();
    let load = || {
        let mut loaded = GrowthScenario::starter_town();
        loaded.core.load_game_internal(path).unwrap();
        loaded.core.time.speed_multiplier = source.core.time.speed_multiplier;
        loaded
    };
    let (mut warm, mut cold) = (load(), load());
    std::fs::remove_file(&saved).unwrap();
    let core = &warm.core;
    assert!(
        core.slot_route_scratch
            .prepared_for_test(&core.transit_network, true)
    );
    assert!(
        core.households
            .job_search_scratch_prepared_for_test(&core.transit_network)
    );
    cold.core.slot_route_scratch = Default::default();
    cold.core.households.discard_job_search_scratch_for_test();
    let fingerprint = |core: &SimCore| {
        (
            core.treasury.balance.to_bits(),
            core.agents.work_building.clone(),
            core.agents
                .pos_x
                .iter()
                .map(|x| x.to_bits())
                .collect::<Vec<_>>(),
            core.households
                .households
                .iter()
                .map(|household| (household.budget.to_bits(), household.employed_adults))
                .collect::<Vec<_>>(),
            core.logistics.shipments.len(),
        )
    };
    let end = warm.core.time.absolute_slot() + 3 * SLOTS_PER_HOUR;
    while warm.core.time.absolute_slot() < end {
        warm.core.step_frame(FRAME_DT_S);
        cold.core.step_frame(FRAME_DT_S);
        assert_eq!(fingerprint(&warm.core), fingerprint(&cold.core));
    }
    // The cold game's slots priced and planned trips on the scratch they prepared.
    let core = &cold.core;
    assert!(
        core.slot_route_scratch
            .prepared_for_test(&core.transit_network, true)
    );
}

// Every finished census's book is prepared for planning again from minute 59's first slot
// (`SIM-02-K`), so the residential slot finds its site rows laid out: a game whose window
// prepares the refilled books plays as one whose planning slots prepare them.
#[test]
fn census_books_prepared_in_the_window_play_as_books_the_planning_slots_prepare() {
    let source = busy_town();
    let saved = std::env::temp_dir().join(format!(
        "metrum-census-book-preparation-{}.sqlite",
        std::process::id()
    ));
    let path = saved.to_str().unwrap();
    source.core.save_game_internal(path, None).unwrap();
    let load = || {
        let mut loaded = GrowthScenario::starter_town();
        loaded.core.load_game_internal(path).unwrap();
        loaded.core.time.speed_multiplier = source.core.time.speed_multiplier;
        loaded
    };
    let (mut window, mut planning) = (load(), load());
    std::fs::remove_file(&saved).unwrap();
    let fingerprint = |core: &SimCore| {
        (
            core.treasury.balance.to_bits(),
            core.allocator.buildings.len(),
            core.agents.work_building.clone(),
            core.households
                .households
                .iter()
                .map(|household| (household.budget.to_bits(), household.employed_adults))
                .collect::<Vec<_>>(),
            core.logistics.shipments.len(),
            core.pending_demand_spawns.len(),
        )
    };
    // Window preparations of a finished census's book in each game.
    let prepared_in_window = |core: &SimCore, before: bool| {
        let census = &core.households.census_run;
        usize::from(census.is_done() && !before && census.funding().planning_prepared)
    };
    let (mut window_preparations, mut planning_preparations) = (0, 0);
    let end = window.core.time.absolute_slot() + 3 * SLOTS_PER_HOUR;
    while window.core.time.absolute_slot() < end {
        // The other game marks each finished book prepared, so its residential slot prepares it.
        if let Some(book) = planning.core.households.unprepared_census_book() {
            book.planning_prepared = true;
        }
        let before = [&window, &planning]
            .map(|game| game.core.households.census_run.funding().planning_prepared);
        window.core.step_frame(FRAME_DT_S);
        planning.core.step_frame(FRAME_DT_S);
        window_preparations += prepared_in_window(&window.core, before[0]);
        planning_preparations += prepared_in_window(&planning.core, before[1]);
        assert_eq!(fingerprint(&window.core), fingerprint(&planning.core));
    }
    assert!(window_preparations >= 2, "{window_preparations}");
    assert!(planning_preparations < window_preparations);
}

// The window's preparation never rebuilds a stale zone index (`SIM-02-K`): the rebuild resets
// the vacancy order admission reads, so it waits for the index's own reader.
#[test]
fn a_stale_zone_index_defers_the_census_books_preparation() {
    use crate::simulation::economy::hiring::{
        PLANNING_PREPARATION_REST_SLOT, PLANNING_PREPARATION_SLOT,
    };
    let mut scenario = busy_town();
    let core = &mut scenario.core;
    step_to_chunk(core, PLANNING_PREPARATION_SLOT - 1);
    let census = &core.households.census_run;
    assert!(census.is_done() && !census.funding().planning_prepared);
    let slot = core.time.absolute_slot() + 1;
    core.allocator.dirty_index = true;
    core.advance_census(slot);
    assert!(core.allocator.dirty_index);
    assert!(!core.households.census_run.funding().planning_prepared);
    // Its reader rebuilds it; the preparation's second-half slot prepares the book.
    core.allocator.rebuild_zone_index();
    core.advance_census(slot + PLANNING_PREPARATION_REST_SLOT - PLANNING_PREPARATION_SLOT);
    assert!(core.households.census_run.funding().planning_prepared);
}

// The census publishes every buyer's financing; the preparation's mask slot, after a second
// half in its own slot, refreshes those finance masks, so the hour's first masked planning
// claim refreshes none of them (`SIM-02-K`).
#[test]
fn the_census_books_preparation_refreshes_its_business_finance_masks() {
    use crate::simulation::economy::hiring::{
        PLANNING_PREPARATION_MASKS_SLOT, PLANNING_PREPARATION_REST_SLOT, PLANNING_PREPARATION_SLOT,
    };
    let mut scenario = busy_town();
    let core = &mut scenario.core;
    step_to_chunk(core, PLANNING_PREPARATION_SLOT - 1);
    let census = &core.households.census_run;
    assert!(census.is_done() && !census.funding().planning_prepared);
    assert!(census.funding().business.changed_finance_buyers_for_test() > 0);
    let slot = core.time.absolute_slot() + 1;
    core.advance_census(slot);
    let business = &core.households.census_run.funding().business;
    assert!(business.changed_finance_buyers_for_test() > 0);
    core.advance_census(slot + PLANNING_PREPARATION_REST_SLOT - PLANNING_PREPARATION_SLOT);
    let funding = core.households.census_run.funding();
    assert!(funding.planning_prepared);
    assert!(funding.business.changed_finance_buyers_for_test() > 0);
    core.advance_census(slot + PLANNING_PREPARATION_MASKS_SLOT - PLANNING_PREPARATION_SLOT);
    let business = &core.households.census_run.funding().business;
    assert_eq!(business.changed_finance_buyers_for_test(), 0);
}

#[test]
fn the_census_continues_identically_after_save_load_in_the_fill_the_window_and_before_publishing() {
    let mut source = busy_town();
    // Mid-fill: the hour's copy is part done.
    step_to_chunk(&mut source.core, FILL_SLOTS / 3);
    assert!(source.core.households.census_inputs.is_filling());
    assert_census_continues_after_reload(&mut source, "mid-fill");
    // Right after the swap: the finished copy waits for the census window.
    step_to_chunk(&mut source.core, FILL_SLOTS - 1);
    assert!(source.core.households.census_inputs.has_pending_front());
    assert!(!source.core.households.census_inputs.is_read());
    assert_census_continues_after_reload(&mut source, "after-swap");
    // Mid-window: the census is part done.
    step_to_chunk(&mut source.core, CENSUS_WINDOW_START + 200);
    assert!(source.core.households.census_inputs.is_read());
    assert!(!source.core.households.census_run.is_done());
    assert_census_continues_after_reload(&mut source, "mid-census");
    // The window's last slot has run: the finished census waits for the next hour's first.
    step_to_chunk(&mut source.core, SLOTS_PER_HOUR - 1);
    assert!(source.core.households.census_run.is_done());
    assert_census_continues_after_reload(&mut source, "before-publish");
}

// The hour's planning slots read the census its first slot published (`SIM-02-I`): a save
// anywhere before the actions slot (after the residential slot, between two planning slots a
// game minute apart, just after the commercial or industrial slot) keeps its copy, and the
// loaded game recomputes the same census, books and all, saves the same copy again and runs the
// next censuses as the saved game does.
#[test]
fn the_published_census_continues_identically_after_save_load_in_the_planning_slots() {
    use crate::simulation::economy::demand::PlanningSlot;
    let mut source = busy_town();
    let commercial = PlanningSlot::Commercial as u64;
    let industrial = PlanningSlot::Industrial as u64;
    let actions = PlanningSlot::Actions as u64;
    for chunk in [0, 1, commercial, industrial - 1, industrial, actions - 1] {
        step_to_chunk(&mut source.core, chunk);
        assert_census_continues_after_reload(&mut source, &format!("planning-slot-{chunk}"));
    }
}

// A planning slot's setup is held for its plan eight slots later and kept by a save in between
// (`SIM-02-K`): at each setup slot and just before each plan, the loaded game holds and saves
// the same setup, and both games plan the slot, and run the rest of its minute, to the same
// hour plan, construction queue, demand queues and diagnostics.
#[test]
fn a_planning_setup_continues_identically_after_save_load_inside_its_span() {
    use crate::simulation::core::scheduler::SLOTS_PER_MINUTE;
    use crate::simulation::economy::demand::PlanningSlot;
    let mut source = busy_town();
    for planning in [
        PlanningSlot::Residential,
        PlanningSlot::Commercial,
        PlanningSlot::Industrial,
        PlanningSlot::Actions,
    ] {
        for chunk in [planning.setup_slot(), planning as u64 - 1] {
            step_to_chunk(&mut source.core, chunk);
            let label = format!("setup-slot-{chunk}");
            let held = source.core.demand.saved_planning_setup();
            assert!(held.is_some(), "{label}: no setup held");
            let mut loaded = reloaded(&source.core, &label);
            assert_eq!(loaded.core.demand.saved_planning_setup(), held, "{label}");
            // The minute's last slot: its plan has run, the next setup not yet.
            let minute_end = planning.setup_slot() + SLOTS_PER_MINUTE - 1;
            for core in [&mut source.core, &mut loaded.core] {
                step_to_chunk(core, minute_end);
            }
            assert!(
                loaded.core.demand.saved_planning_setup().is_none(),
                "{label}"
            );
            assert_eq!(
                planning_tables(&source.core, &format!("{label}-source")),
                planning_tables(&loaded.core, &format!("{label}-loaded")),
                "{label}: the plan differs after loading"
            );
        }
    }
}

// The plan on a setup taken in an unchanged city equals the plan that counts and scans itself
// (`SIM-02-K`): two copies of a game at each planning slot's setup slot plan the slot at once,
// one on the held setup and one without, to the same hour plan, construction queue, demand
// queues and diagnostics.
#[test]
fn a_plan_on_its_setup_equals_one_that_scans_an_unchanged_city_itself() {
    use crate::simulation::economy::demand::PlanningSlot;
    let mut source = busy_town();
    for planning in [
        PlanningSlot::Residential,
        PlanningSlot::Commercial,
        PlanningSlot::Industrial,
        PlanningSlot::Actions,
    ] {
        step_to_chunk(&mut source.core, planning.setup_slot());
        let label = format!("frozen-{planning:?}");
        let mut games = [
            reloaded(&source.core, &label),
            reloaded(&source.core, &label),
        ];
        games[1].core.demand.drop_planning_setup_for_test();
        let tables = games.each_mut().map(|game| {
            let core = &mut game.core;
            let slot = core.time.absolute_slot();
            core.allocator.now_slot = slot - planning.setup_slot() + planning as u64;
            let (day_index, minute_of_day) = slot_calendar(slot);
            core.execute_demand_planning_slot(planning, day_index, minute_of_day);
            planning_tables(core, &label)
        });
        assert_eq!(tables[0], tables[1], "{label}: the sliced plan differs");
        // The plan read its setup.
        assert!(
            games[0].core.demand.saved_planning_setup().is_none(),
            "{label}"
        );
    }
}

// A copy of `core` through a save and a load.
fn reloaded(core: &SimCore, label: &str) -> GrowthScenario {
    let path = std::env::temp_dir().join(format!(
        "metrum-planning-setup-{label}-{}.sqlite",
        std::process::id()
    ));
    core.save_game_internal(path.to_str().unwrap(), None)
        .unwrap();
    let mut loaded = GrowthScenario::starter_town();
    loaded
        .core
        .load_game_internal(path.to_str().unwrap())
        .unwrap();
    loaded.core.time.speed_multiplier = core.time.speed_multiplier;
    std::fs::remove_file(path).unwrap();
    loaded
}

// The rows a save writes for the hour's plan and what the actions slot queues from it.
fn planning_tables(core: &SimCore, label: &str) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    let path = std::env::temp_dir().join(format!(
        "metrum-planning-tables-{label}-{}.sqlite",
        std::process::id()
    ));
    core.save_game_internal(path.to_str().unwrap(), None)
        .unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    let tables = [
        "demand_hour_plan",
        "pending_demand_spawns",
        "demand_diagnostics",
        "demand_state",
        "level_change_requests",
        "building_removal_requests",
        "admission_requests",
    ]
    .map(|table| {
        let mut stmt = conn.prepare(&format!("SELECT * FROM {table}")).unwrap();
        let columns = stmt.column_count();
        stmt.query_map([], |row| {
            (0..columns)
                .map(|column| row.get::<_, rusqlite::types::Value>(column))
                .collect::<Result<Vec<_>, _>>()
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
    })
    .to_vec();
    drop(conn);
    std::fs::remove_file(path).unwrap();
    tables
}

// A census the window left stale runs whole at the hour's first slot (`SIM-02-I`): after a
// frontage price change past the window's last slot it reads the window's copy again, and after
// an undo there, which drops both copies, a copy of the city taken then. Either way the
// publication keeps the copy it read, so a save in the hour's planning slots restores it.
#[test]
fn a_census_run_whole_at_its_publication_keeps_its_copy_for_the_planning_slots() {
    let mut source = busy_town();
    for copied_now in [false, true] {
        for chunk in 0..3 {
            step_to_chunk(&mut source.core, SLOTS_PER_HOUR - 1);
            let core = &mut source.core;
            if copied_now {
                assert!(core.bulldoze_building(core.allocator.buildings.len() - 1));
                assert!(core.undo_action_internal());
                assert!(!core.households.census_inputs.has_pending_front());
            } else {
                core.transit_network.frontage_delay_revision += 1;
            }
            let key = core.households.census_key(&core.transit_network);
            assert!(!core.households.census_ready(next_hour_slot(core), key));
            step_to_chunk(&mut source.core, chunk);
            let kind = if copied_now { "copied" } else { "stale" };
            let label = format!("{kind}-slot-{chunk}");
            // The planning slots lend these books, so the save keeps their copy.
            assert!(published_books_lent(&source.core), "{label}");
            assert_census_continues_after_reload(&mut source, &label);
        }
    }
}

// Saves `source`, loads the save, checks that the loaded game saves the same copy and census
// state and that both run the next three hours' censuses bit for bit.
fn assert_census_continues_after_reload(source: &mut GrowthScenario, label: &str) {
    let path = |name: &str| {
        std::env::temp_dir().join(format!(
            "metrum-census-inputs-{label}-{name}-{}.sqlite",
            std::process::id()
        ))
    };
    let (saved, resaved) = (path("source"), path("loaded"));
    source
        .core
        .save_game_internal(saved.to_str().unwrap(), None)
        .unwrap();
    let mut loaded = GrowthScenario::starter_town();
    loaded
        .core
        .load_game_internal(saved.to_str().unwrap())
        .unwrap();
    loaded.core.time.speed_multiplier = source.core.time.speed_multiplier;
    loaded
        .core
        .save_game_internal(resaved.to_str().unwrap(), None)
        .unwrap();
    type Row = (
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Vec<u8>,
        Vec<u8>,
        Option<i64>,
        Option<bool>,
        Option<Vec<u8>>,
    );
    let copy = |path: &std::path::Path| -> Row {
        let rows = rusqlite::Connection::open(path)
            .unwrap()
            .query_row(
                "SELECT front, back, changes, deferred, census_publish_slot, census_current, \
                 published FROM census_inputs",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .unwrap();
        std::fs::remove_file(path).unwrap();
        rows
    };
    let (source_copy, loaded_copy) = (copy(&saved), copy(&resaved));
    assert!(
        source_copy.0.is_some() || source_copy.1.is_some(),
        "{label}"
    );
    assert_eq!(source_copy, loaded_copy, "{label}: the loaded copy differs");
    // Saved before the hour's actions slot: the published census its planning slots read.
    let planning = source.core.time.absolute_slot() % SLOTS_PER_HOUR
        < crate::simulation::economy::demand::PlanningSlot::Actions as u64;
    assert_eq!(source_copy.6.is_some(), planning, "{label}");
    if planning {
        assert_same_published_census(&source.core, &loaded.core);
    }
    // A finished census is recomputed at load.
    if source.core.households.census_run.is_done() && source_copy.4.is_some() {
        assert!(loaded.core.households.census_run.is_done(), "{label}");
        assert_same_census(
            source.core.households.census_run.funding(),
            loaded.core.households.census_run.funding(),
            label,
        );
    }
    let hours = |core: &SimCore| core.time.absolute_slot() / SLOTS_PER_HOUR;
    let target = hours(&source.core) + 3;
    let mut censuses = 0;
    while hours(&source.core) < target {
        let frame = source.core.step_frame(FRAME_DT_S);
        loaded.core.step_frame(FRAME_DT_S);
        censuses += frame.hourly_ticks;
        let (a, b) = (&source.core, &loaded.core);
        // A load after the hour's actions slot rebuilt a census of the live city, which only
        // the derived jobs shown read before the next hour's first slot publishes the census
        // of the copy; a load before it restored the published census (above).
        if censuses > 0 {
            let (x, y) = (&a.households.job_funding, &b.households.job_funding);
            assert_eq!(x.capacities, y.capacities, "{label}: funded jobs differ");
            assert_eq!(
                x.activities()
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                y.activities()
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                "{label}: funded activity differs"
            );
            assert_eq!(x.freight_capital, y.freight_capital, "{label}");
            if frame.hourly_ticks > 0 {
                assert_same_census(x, y, label);
            }
        }
        assert_eq!(a.allocator.buildings.len(), b.allocator.buildings.len());
        for (left, right) in a.allocator.buildings.iter().zip(&b.allocator.buildings) {
            assert_eq!(left.funded_job_capacity, right.funded_job_capacity);
            assert_eq!(left.worker_count, right.worker_count);
            assert_eq!(
                left.operating_budget.to_bits(),
                right.operating_budget.to_bits()
            );
        }
        assert_eq!(a.households.households.len(), b.households.households.len());
    }
    assert!(censuses >= 2, "{label}: the copy fed censuses");
}

#[test]
#[ignore = "SIM-02-I census window churn in the growing starter town; run alone"]
fn growth_census_window_churn() {
    let days = std::env::var("METRUM_CHURN_DAYS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(6);
    let mut scenario = GrowthScenario::starter_town();
    scenario.run_days(1);
    let skips = crate::nodes::sim::core::populated_city::census_churn::report_census_churn(
        &mut scenario.core,
        days * 24,
        "starter-town",
    );
    assert_eq!(skips, 0);
}

// Starts a queued construction of a firm (with recipe inputs) or a home in a census window,
// out of queue order (the queue's own start event then finds it gone), and returns the game and
// a copy loaded at that moment whose census restarts at its next slot and so covers the site.
fn start_in_window(firm: bool) -> (GrowthScenario, GrowthScenario) {
    let (mut source, _) = start_site_in_window(firm);
    let path = std::env::temp_dir().join(format!(
        "metrum-window-start-{firm}-{}.sqlite",
        std::process::id()
    ));
    source
        .core
        .save_game_internal(path.to_str().unwrap(), None)
        .unwrap();
    let mut covering = GrowthScenario::starter_town();
    covering
        .core
        .load_game_internal(path.to_str().unwrap())
        .unwrap();
    std::fs::remove_file(path).unwrap();
    covering.core.time.speed_multiplier = source.core.time.speed_multiplier;
    covering.core.transit_network.frontage_delay_revision += 1;
    for core in [&mut source.core, &mut covering.core] {
        step_to_chunk(core, SLOTS_PER_HOUR - 1);
    }
    let census = |game: &GrowthScenario| game.core.households.census_run.funding().capacities.len();
    let live = source.core.allocator.buildings.len();
    assert!(census(&source) < live, "the site is not in the census");
    // The restart took the site; the window appended more to both games since.
    assert_eq!(census(&covering), census(&source) + 1);
    (source, covering)
}

// Starts a queued firm (with recipe inputs) or home in a census window, out of queue order, and
// returns the game and the new site's index.
fn start_site_in_window(firm: bool) -> (GrowthScenario, usize) {
    let mut source = GrowthScenario::starter_town();
    let window = CENSUS_WINDOW_START + 10..SLOTS_PER_HOUR - 60;
    let catalog = load_runtime_economy_catalog().unwrap();
    let has_inputs = |core: &SimCore, building: usize| {
        catalog
            .profile_by_runtime_id(core.allocator.buildings[building].economy_profile_runtime_id)
            .is_some_and(|profile| !profile.inputs.is_empty())
    };
    loop {
        assert!(source.day() < 30, "no start could be placed in a window");
        source.core.step_frame(FRAME_DT_S);
        let core = &mut source.core;
        let chunk = core.time.absolute_slot() % SLOTS_PER_HOUR;
        if !window.contains(&chunk) || !core.households.census_inputs.is_read() {
            continue;
        }
        let Some(queued) = core
            .pending_demand_spawns
            .iter()
            .position(|pending| (pending.zone_type != ZoneType::Residential) == firm)
        else {
            continue;
        };
        let pending = core.pending_demand_spawns.remove(queued).unwrap();
        let id = pending.id;
        core.pending_demand_spawns.push_front(pending);
        let buildings = core.allocator.buildings.len();
        let (day, minute) = (core.time.day_index, core.time.minute_of_day);
        core.start_pending_demand_spawn(id, day, minute);
        if core.allocator.buildings.len() > buildings && has_inputs(core, buildings) == firm {
            return (source, buildings);
        }
    }
}

// A workplace that completes between the census's copy and the next hour's first slot is still
// under construction for planning (`SIM-02-I`): the census published there did not see it
// complete and funds none of its jobs, so only a constructing claim can hold its workers; from
// the hour after, the census funds its jobs and planning claims none. No hour claims its workers
// twice.
#[test]
fn a_workplace_completed_after_the_copy_keeps_one_claim_until_a_census_sees_it() {
    let (mut scenario, site) = start_site_in_window(true);
    let core = &mut scenario.core;
    // Started in the window, the site completes in a later hour's window, after that copy.
    let complete = core.allocator.buildings[site].construction_complete_slot;
    assert!(complete % SLOTS_PER_HOUR >= CENSUS_WINDOW_START);
    while core.time.absolute_slot() < complete {
        core.step_frame(FRAME_DT_S / 16.0);
    }
    assert!(!core.allocator.buildings[site].is_under_construction());
    let claims = |core: &SimCore| {
        let census = &core.households.job_funding;
        (
            census.saw_operating(site),
            census.capacities.get(site).copied().unwrap_or(0),
            core.demand.constructing_workers_for_test(site),
        )
    };
    // Each planning slot runs at the hour's first slot; read its claims in the slot after.
    step_to_chunk(core, 1);
    let (seen, funded, _) = claims(core);
    assert!(!seen && funded == 0, "{:?}", claims(core));
    step_to_chunk(core, 0);
    step_to_chunk(core, 1);
    let (seen, funded, constructing) = claims(core);
    assert!(
        seen && funded > 0 && constructing == 0,
        "{:?}",
        claims(core)
    );
}

// A home or a firm started in the census window appends a site the census's copy lacks.
// Planning at the next hour's first slot lends the published census, whose books treat the site
// as unfunded and without supply and lay out a firm's buyer ports after the census's own, and
// plans as on a census that covers the site; the pass asserts that it lends a current census and
// that the books return exactly.
fn assert_planning_lends_the_census_after_a_start_in_the_window(firm: bool) {
    let (mut source, mut covering) = start_in_window(firm);
    // The hour's planning slots and its actions, slots 0 to 3 (`SIM-02-I`).
    for core in [&mut source.core, &mut covering.core] {
        step_to_chunk(core, 3);
    }
    let plan = |game: &GrowthScenario| {
        let core = &game.core;
        (
            format!("{:?}", core.demand.building_actions),
            core.pending_demand_spawns
                .iter()
                .map(|pending| (pending.due_minute, pending.action.parcel_id))
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(plan(&source), plan(&covering));
}

#[test]
fn planning_lends_the_published_census_after_a_home_starts_in_the_window() {
    assert_planning_lends_the_census_after_a_start_in_the_window(false);
}

#[test]
fn planning_lends_the_published_census_after_a_firm_starts_in_the_window() {
    assert_planning_lends_the_census_after_a_start_in_the_window(true);
}

// A household that arrives during the demand snapshot's sweep and is hired at the next hour's
// first slot joins the front as it was before that hire, as the sweep took every other
// household. Taken after it, its job would still be open in the swept workplace while its
// adult no longer counted as unemployed, and admission would count that job twice. Job
// searches run from their queue at any slot since `SIM-02-J`, so the arrivals' searches are
// queued at the hour's last slot, for the next hour's first slot that serves queues: its second,
// since the first is queue-free (`QUEUE_FREE_SLOTS`, `SIM-02-K`).
#[test]
fn an_arrival_hired_at_the_next_hour_joins_the_snapshot_as_before_its_hire() {
    let mut scenario = GrowthScenario::starter_town();
    let counts = |core: &SimCore| {
        (
            core.households.households.len(),
            core.allocator.buildings.len(),
        )
    };
    loop {
        assert!(scenario.day() < 5, "no arrival was hired at the next hour");
        let core = &mut scenario.core;
        step_to_chunk(core, 0);
        let start = counts(core);
        step_to_chunk(core, SLOTS_PER_HOUR - 1);
        let end = counts(core);
        // Arrivals only: no building placed and no household removed in the hour.
        if end.1 != start.1 || end.0 <= start.0 {
            continue;
        }
        let expected = core.demand.topped_up_snapshot_front_for_test(
            &core.allocator,
            &core.households,
            &core.fiscal_policy,
        );
        let employed = |core: &SimCore| {
            core.households.households[start.0..end.0]
                .iter()
                .map(|household| u32::from(household.employed_adults))
                .sum::<u32>()
        };
        let before = employed(core);
        let slot = core.time.absolute_slot();
        let agents = core.agents.len();
        core.agents.handles.register_appended(agents);
        for agent_idx in 0..agents {
            if (start.0..end.0).contains(&core.agents.household_id[agent_idx]) {
                let agent = core.agents.agent_handle(agent_idx);
                core.households.job_requests.push(slot, agent);
            }
        }
        step_to_chunk(core, 1);
        if counts(core) != end || employed(core) == before {
            continue;
        }
        assert!(expected.is_some());
        assert_eq!(core.demand.saved_snapshot_sweep().front, expected);
        break;
    }
}

// Policy changed at minute 40 reaches the census in progress: the copy's frozen policy takes
// the new values, the per-building override reaches its copied row, and the census restarts,
// so the census the next hour publishes carries the new ceilings, and a game saved before it
// publishes continues identically (`SIM-02-I`).
#[test]
fn a_policy_change_in_the_census_window_reaches_the_next_census_and_survives_save_load() {
    let mut source = GrowthScenario::producer_town();
    source.run_days(2);
    let core = &mut source.core;
    step_to_chunk(core, 640);
    assert!(core.households.census_inputs.is_read());
    let plant = core
        .allocator
        .buildings
        .iter()
        .position(|building| building.asset_id == COAL_POWER_PLANT)
        .unwrap();
    let (x, z) = {
        let building = &core.allocator.buildings[plant];
        (building.center_x, building.center_y)
    };
    let ceiling = |core: &SimCore| core.households.census_run.funding().service_capacities[plant];
    let before = ceiling(core);
    let restarts = core.households.census_run.restarts;
    assert!(core.set_service_funding("electricity", 0.8));
    assert!(core.set_building_service_funding_override_at(x, z, "electricity", 0.5));
    assert!(core.set_fiscal_policy_value(
        crate::simulation::economy::fiscal::POLICY_COMMERCIAL_PROPERTY_TAX,
        40.0
    ));
    step_to_chunk(core, SLOTS_PER_HOUR - 1);
    assert_eq!(core.households.census_run.restarts, restarts + 1);
    assert!(core.households.census_run.is_done());
    let parameters = core.households.census_inputs.front_parameters_for_test();
    assert_eq!(
        format!("{:?}", parameters.fiscal),
        format!("{:?}", core.fiscal_policy)
    );
    assert_eq!(parameters.electricity_funding, 0.8);
    // The census of the next hour, as one of the live city has it.
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let mut live = JobFunding::default();
    live.refresh(
        &core.allocator,
        &core.households.households,
        &mut core.logistics.clone(),
        &core.transit_network,
        &core.region_graph,
        &catalog,
        &tuning,
        &core.fiscal_policy,
        &core.electricity_funding_by_building(),
        core.treasury.balance,
        core.time.minute_of_day,
    );
    assert_ne!(live.service_capacities[plant], before);
    assert_eq!(ceiling(core), live.service_capacities[plant]);
    assert_census_continues_after_reload(&mut source, "policy");
}

// A building that leaves between the hour's planning slots moves the published books' indices,
// so the later slots never read their rows: a skipped commercial or industrial slot plans
// nothing for its zone this hour (the residential slot's homes, planned before the removal,
// are still queued at the actions slot), and a skipped actions slot leaves every queued
// business permit voided, to wait for the next hour's slot (`SIM-02-I`).
#[test]
fn a_building_removed_between_the_planning_slots_skips_the_later_slots() {
    use crate::simulation::economy::demand::SpawnPermit;
    let mut scenario = GrowthScenario::starter_town();
    scenario.run_days(1);
    let core = &mut scenario.core;
    // Slot 0 of the next hour has run: homes are planned.
    let hour = core.time.absolute_slot() / SLOTS_PER_HOUR + 1;
    while core.time.absolute_slot() < hour * SLOTS_PER_HOUR {
        core.step_frame(FRAME_DT_S / 16.0);
    }
    assert_eq!(core.time.absolute_slot(), hour * SLOTS_PER_HOUR);
    let queued = core.pending_demand_spawns.len();
    // The last building moves into the removed one's index.
    assert!(core.bulldoze_building(0));
    let actions = crate::simulation::economy::demand::PlanningSlot::Actions as u64;
    while core.time.absolute_slot() < hour * SLOTS_PER_HOUR + actions {
        core.step_frame(FRAME_DT_S / 16.0);
    }
    // Spawns are queued at the actions slot, with its minute.
    let (day, minute) = slot_calendar(hour * SLOTS_PER_HOUR + actions);
    assert!(
        core.pending_demand_spawns
            .iter()
            .skip(queued)
            .all(|pending| pending.zone_type == ZoneType::Residential
                || (pending.planned_day_index, pending.planned_minute_of_day) != (day, minute)),
        "no business zone slot planned after the removal"
    );
    assert!(
        core.pending_demand_spawns
            .iter()
            .filter(|pending| pending.zone_type != ZoneType::Residential)
            .all(|pending| pending.permit == SpawnPermit::Voided),
        "every queued business spawn waits for the next hour's permits"
    );
}
