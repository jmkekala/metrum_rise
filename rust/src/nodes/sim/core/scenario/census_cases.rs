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
    CENSUS_WINDOW_START, CopyNow, FILL_SLOTS, JobFunding, LiveCensusParameters,
};

// A town busy with homes, businesses and freight on the road.
fn busy_town() -> GrowthScenario {
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
        CopyNow::Live(LiveCensusParameters {
            treasury: -1.0,
            fiscal: &Default::default(),
            service_funding: &[],
            minute_of_day: 0,
        }),
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
    // Half the fill is copied: rows on both sides of the cursor, and the last rows.
    let buildings = core.allocator.buildings.len();
    for index in [1, buildings / 2 - 1, buildings / 2 + 1, buildings - 4] {
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
    remove_building(core, 2);
    remove_household(core, 3);
    core.households
        .census_inputs
        .assert_aligned(&core.allocator, &core.households.households);
    let service_funding = core.electricity_funding_by_building();
    core.households.refresh_funding_census(
        &mut core.allocator,
        &mut core.logistics,
        &core.transit_network,
        &core.region_graph,
        &service_funding,
        &core.fiscal_policy,
        core.treasury.balance,
        core.time.minute_of_day,
    );
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
fn assert_same_census(left: &JobFunding, right: &JobFunding, label: &str) {
    left.assert_same_operations_for_test(right);
    assert_eq!(left.service_capacities, right.service_capacities, "{label}");
    assert_eq!(left.freight_capital, right.freight_capital, "{label}");
    assert_eq!(
        left.census_book_fingerprints_for_test(),
        right.census_book_fingerprints_for_test(),
        "{label}"
    );
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
    );
    let copy = |path: &std::path::Path| -> Row {
        let rows = rusqlite::Connection::open(path)
            .unwrap()
            .query_row(
                "SELECT front, back, changes, deferred, census_publish_slot, census_current \
                 FROM census_inputs",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
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
        // Loading rebuilt a census of the live city, which nothing reads before the next
        // hour's first slot publishes the census of the copy.
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
    crate::nodes::sim::core::populated_city::census_churn::report_census_churn(
        &mut scenario.core,
        days * 24,
        "starter-town",
    );
}

// Starts a queued construction of a firm (with recipe inputs) or a home in a census window,
// out of queue order (the queue's own start event then finds it gone), and returns the game and
// a copy loaded at that moment whose census restarts at its next slot and so covers the site.
fn start_in_window(firm: bool) -> (GrowthScenario, GrowthScenario) {
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
            break;
        }
    }
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

// A home started in the census window appends a site the census's copy lacks. Planning at the
// next hour's first slot still lends the published census, which treats the site as unfunded
// and without supply, and plans as on a census that covers the site.
#[test]
fn planning_lends_the_published_census_after_a_home_starts_in_the_window() {
    let (mut source, mut covering) = start_in_window(false);
    let planned = [&source, &covering].map(|game| game.core.demand.planner_censuses);
    for core in [&mut source.core, &mut covering.core] {
        step_to_chunk(core, 0);
    }
    assert_eq!(
        [&source, &covering].map(|game| game.core.demand.planner_censuses),
        planned,
        "both passes lend the published census"
    );
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

// A firm started in the window would buy through business input ports the published books do
// not lay out for it, so planning runs its own census; the covering game lends its census.
#[test]
fn planning_runs_its_own_census_after_a_firm_starts_in_the_window() {
    let (mut source, mut covering) = start_in_window(true);
    let planned = [&source, &covering].map(|game| game.core.demand.planner_censuses);
    for core in [&mut source.core, &mut covering.core] {
        step_to_chunk(core, 0);
    }
    assert_eq!(
        [&source, &covering].map(|game| game.core.demand.planner_censuses),
        [planned[0] + 1, planned[1]]
    );
}

// A household that arrives during the demand snapshot's sweep and is hired at the next hour's
// first slot joins the front as it was before that hire, as the sweep took every other
// household. Taken after it, its job would still be open in the swept workplace while its
// adult no longer counted as unemployed, and admission would count that job twice.
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
        step_to_chunk(core, 0);
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
