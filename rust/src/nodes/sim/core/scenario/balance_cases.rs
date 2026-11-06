// SPDX-License-Identifier: GPL-2.0-only

//! Opt-in economic experiments using ordinary placement, production, freight and settlement.

use super::*;
use crate::simulation::economy::demand::PlanningSlot;

#[test]
#[ignore = "manual saved-city diagnosis; requires METRUM_DIAG_SAVE, METRUM_DIAG_MODS and METRUM_DIAG_REPORT"]
fn diagnose_saved_city_growth() {
    let source = std::env::var("METRUM_DIAG_SAVE").expect("input save path");
    let mods = std::env::var("METRUM_DIAG_MODS").expect("matching asset pack directory");
    let destination = std::env::var("METRUM_DIAG_REPORT").expect("output text report path");
    assert_ne!(Path::new(&source), Path::new(&destination));
    let days: u32 = std::env::var("METRUM_DIAG_DAYS")
        .unwrap_or_else(|_| "30".into())
        .parse()
        .unwrap();
    assert!(days > 0);
    let mut scenario = load_saved_scenario(&source, &mods);
    assert!(
        scenario
            .core
            .allocator
            .buildings
            .iter()
            .all(|b| !b.broken && !b.economy_broken)
    );
    let initial = scenario.record();
    let report = scenario.run_diagnosed_days(days);
    std::fs::write(
        destination,
        format!("Initial: {initial:?}\n{}", report.daily_report()),
    )
    .unwrap();
    eprintln!("Saved-city replay: {initial:?} -> {:?}", scenario.record());
}

// Loads a saved city with the asset packs it was built from, at the fastest game speed and
// otherwise unchanged: balances, zoning, staffing and demand rules stay as saved.
fn load_saved_scenario(source: &str, mods: &str) -> GrowthScenario {
    let mut core = SimCore::new(WorldConfig::new(WORLD_SIZE_M, WORLD_SIZE_M, 40.0, 10.0));
    let scan = scan_pack_dir(Path::new(mods));
    assert!(!scan.packs.is_empty(), "{:?}", scan.warnings);
    core.replace_asset_registry(scan.packs.into_iter().flat_map(|pack| {
        let pack_id = pack.pack.pack_id;
        pack.assets
            .into_iter()
            .map(move |(manifest, dir)| (pack_id.clone(), manifest, dir))
    }));
    core.load_game_internal(source).unwrap();
    core.time.speed_multiplier = *SIMULATION_SPEED_STEPS.last().unwrap();
    GrowthScenario {
        core,
        departures: 0,
    }
}

#[test]
#[ignore = "manual saved-city replay; requires METRUM_REPLAY_SAVE and METRUM_REPLAY_MODS"]
fn replay_saved_city_without_impossible_turns() {
    // NET-07: a route needing a turn the lanes do not build fails as "no turn" when a car
    // reaches it. Replays ordinary frames for METRUM_REPLAY_DAYS (default 1) and reports
    // every trip failure line.
    let source = std::env::var("METRUM_REPLAY_SAVE").expect("input save path");
    let mods = std::env::var("METRUM_REPLAY_MODS").expect("matching asset pack directory");
    let days: u32 = std::env::var("METRUM_REPLAY_DAYS")
        .unwrap_or_else(|_| "1".into())
        .parse()
        .unwrap();
    let mut scenario = load_saved_scenario(&source, &mods);
    let initial = scenario.record();
    scenario.run_days(days);
    let last = scenario.record();
    let lines = &scenario.core.agents.trip_failures.lines;
    let no_turn: Vec<_> = lines
        .iter()
        .filter(|line| line.contains(": no turn "))
        .collect();
    eprintln!(
        "REPLAY_TRIP_FAILURES {}",
        serde_json::json!({ "days": days, "failures": lines.len(), "no_turn": no_turn.len(),
            "initial": format!("{initial:?}"), "final": format!("{last:?}") })
    );
    for line in lines {
        eprintln!("REPLAY_FAILURE {line}");
    }
    assert!(no_turn.is_empty(), "{no_turn:#?}");
}

#[test]
fn bankrupt_grocery_is_cleared_and_replaced_without_rezoning() {
    let mut scenario = GrowthScenario::starter_town();
    scenario.run_days(365);
    let catalog = scenario.core.demand.runtime_catalog();
    let grocery_profile = catalog.profile_for_id("grocery_basic").unwrap().runtime_id;
    let failed = scenario
        .core
        .allocator
        .buildings
        .iter()
        .position(|building| {
            building.economy_profile_runtime_id == grocery_profile && !building.is_deserted
        })
        .unwrap();
    let building = &mut scenario.core.allocator.buildings[failed];
    let identity = (building.parcel_id, building.build_generation);
    // A one-off uninsured loss makes this firm insolvent. Ordinary settlement, bankruptcy,
    // employment, clearance and private entry must recover the category without player edits.
    building.operating_budget = -1_000_000.0;
    building.budget_distress = true;
    let report = scenario.run_diagnosed_days(180);
    if let Ok(path) = std::env::var("METRUM_GROWTH_RECOVERY_REPORT") {
        std::fs::write(path, report.daily_report()).unwrap();
    }
    let failed_firm = report
        .business_outcomes()
        .find(|firm| (firm.parcel_id, firm.generation) == identity)
        .unwrap();
    assert!(
        failed_firm.bankrupt && failed_firm.removed,
        "{failed_firm:?}"
    );
    let replacements: Vec<_> = report
        .business_outcomes()
        .filter(|firm| firm.profile_id == "grocery_basic" && firm.startup_cash > 0.0)
        .collect();
    assert_eq!(
        replacements.len(),
        1,
        "replacement capacity must not be duplicated: {replacements:?}"
    );
    let replacement = replacements[0];
    assert!(
        !replacement.bankrupt && !replacement.removed,
        "{replacement:?}"
    );
    assert!(replacement.operating_hours >= 90 * 24, "{replacement:?}");
    assert!(replacement.operating_cash_change > 0.0, "{replacement:?}");
    assert!(
        replacement.staffed_worker_hours >= replacement.capacity_worker_hours * 0.95,
        "{replacement:?}"
    );
    assert!(scenario.record().residents > 100, "{:?}", scenario.record());
}

#[test]
fn queued_growth_continues_identically_after_save_load() {
    let mut source = GrowthScenario::starter_town();
    // Save after supporting construction has started, while further placements remain queued.
    while source.core.pending_demand_spawns.is_empty()
        || !source
            .core
            .allocator
            .buildings
            .iter()
            .any(|building| building.is_under_construction())
    {
        assert!(
            source.core.time.day_index < 3,
            "startup must offer queued construction"
        );
        source.core.step_frame(FRAME_DT_S);
    }
    assert_growth_continues_after_reload(source);
}

#[test]
fn busy_city_continues_identically_after_save_load() {
    let mut source = GrowthScenario::starter_town();
    source.run_days(10);
    assert!(!source.core.logistics.shipments.is_empty());
    assert!(
        source
            .core
            .transit_network
            .lane_system
            .lanes
            .iter()
            .any(|lane| lane.live_frontage_delay_s > 0.0)
    );
    assert_growth_continues_after_reload(source);
}

fn assert_growth_continues_after_reload(mut source: GrowthScenario) {
    let path = std::env::temp_dir().join(format!(
        "metrum-demand-continuation-{}-{}.sqlite",
        std::process::id(),
        source.core.time.day_index
    ));
    source
        .core
        .save_game_internal(path.to_str().unwrap(), None)
        .unwrap();
    let mut loaded = GrowthScenario::starter_town();
    loaded
        .core
        .load_game_internal(path.to_str().unwrap())
        .unwrap();
    loaded.core.time.speed_multiplier = source.core.time.speed_multiplier;
    assert_eq!(
        source.core.transit_network.frontage_delay_elapsed_s,
        loaded.core.transit_network.frontage_delay_elapsed_s,
    );
    let queued = |scenario: &GrowthScenario| -> Vec<_> {
        scenario
            .core
            .pending_demand_spawns
            .iter()
            .map(|pending| {
                (
                    pending.due_minute,
                    pending.zone_type,
                    pending.action.parcel_id,
                    pending.action.asset_id.clone(),
                    pending.planned_day_index,
                    pending.planned_minute_of_day,
                )
            })
            .collect()
    };
    assert_eq!(queued(&source), queued(&loaded));
    let target_day = source.core.time.day_index + 30;
    while source.core.time.day_index < target_day {
        source.core.step_frame(FRAME_DT_S);
        loaded.core.step_frame(FRAME_DT_S);
        assert_eq!(
            source.core.transit_network.frontage_delay_elapsed_s,
            loaded.core.transit_network.frontage_delay_elapsed_s,
        );
        // Cash is checked to cent-level precision; discrete simulation outcomes remain exact.
        let mut source_record = source.record();
        let mut loaded_record = loaded.record();
        assert!(
            (source_record.treasury - loaded_record.treasury).abs() < 0.01,
            "treasury differs after reload at minute {}: {source_record:?} vs {loaded_record:?}",
            source.core.time.minute_of_day
        );
        source_record.treasury = 0.0;
        loaded_record.treasury = 0.0;
        assert_eq!(source_record, loaded_record);
        assert_eq!(queued(&source), queued(&loaded));
        assert_eq!(
            source.core.households.households.len(),
            loaded.core.households.households.len()
        );
        for (a, b) in source
            .core
            .households
            .households
            .iter()
            .zip(&loaded.core.households.households)
        {
            assert!(
                (a.budget - b.budget).abs() < 0.01,
                "household cash differs: {} vs {}",
                a.budget,
                b.budget
            );
            assert_eq!(a.replenishment_state, b.replenishment_state);
        }
        let left = &source.core.allocator.buildings;
        let right = &loaded.core.allocator.buildings;
        assert_eq!(left.len(), right.len());
        for (a, b) in left.iter().zip(right) {
            assert_eq!(a.parcel_id, b.parcel_id);
            assert_eq!(a.asset_id, b.asset_id);
            assert_eq!(a.center_x, b.center_x);
            assert_eq!(a.center_y, b.center_y);
            assert_eq!(a.worker_count, b.worker_count);
            assert!(
                (a.operating_budget - b.operating_budget).abs() < 0.01,
                "firm cash differs after reload at day {} minute {} ({}): {} vs {}",
                source.core.time.day_index,
                source.core.time.minute_of_day,
                a.asset_id,
                a.operating_budget,
                b.operating_budget
            );
            assert_eq!(a.resource_inventory, b.resource_inventory);
            assert_eq!(a.construction_complete_slot, b.construction_complete_slot);
        }
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn many_new_industrial_lots_respect_scarce_labor_and_earn_their_owner_capital() {
    use crate::simulation::buildings::allocator::BuildingSiteEnvironment;
    use crate::simulation::economy::agents::age_group_can_work;
    use crate::simulation::economy::definitions::load_runtime_economy_catalog;

    let mut scenario = GrowthScenario::starter_town();
    scenario.run_days(365);
    let before = scenario.record();
    assert!(before.residents > 20 && before.employed > 0);
    let core = &mut scenario.core;
    let industrial_lots = |core: &SimCore| {
        core.zoning
            .parcels()
            .iter()
            .filter(|parcel| {
                core.zoning
                    .profiles
                    .profile_by_runtime_id(parcel.zone_profile_runtime_id())
                    .is_some_and(|profile| profile.zone_type == ZoneType::Industrial)
            })
            .count()
    };
    let lots_before = industrial_lots(core);
    commit_road(core, (MAIN_STREET_END_X_M, 0.0), (600.0, 0.0));
    commit_road(core, (400.0, -250.0), (400.0, 250.0));
    // A whole industrial street arrives in one player expansion, while only a short
    // extension supplies new homes. Old vacant homes and unemployed adults also count.
    paint_cross_street(core, 400.0, ZoneType::Industrial);
    commit_road(core, (0.0, 250.0), (0.0, 390.0));
    paint(
        core,
        ZoneType::Residential,
        ZONE_BRUSH,
        &[DVec2::new(0.0, 280.0), DVec2::new(0.0, 360.0)],
    );
    core.prepare_cell_lots_internal();
    core.allocator
        .prepare_building_site_query_index(core.config.zone_cell_m);
    core.transit_network
        .roadbed
        .compile_dirty(&core.region_graph);
    let new_lots = industrial_lots(core) - lots_before;
    assert!(
        new_lots >= 20,
        "the expansion must offer many industrial lots, got {new_lots}"
    );
    let catalog = load_runtime_economy_catalog().unwrap();
    let candidates = core.allocator.collect_demand_spawn_candidates_by_use(
        &core.zoning,
        &core.region_graph,
        &catalog,
        &[],
        BuildingSiteEnvironment {
            road_surface: &core.transit_network.roadbed,
            terrain: &core.heightmap,
        },
    );
    let possible_new_homes: u32 = candidates
        .residential
        .iter()
        .map(|candidate| {
            core.allocator
                .registry
                .household_capacity(&candidate.action.asset_id)
        })
        .sum();
    let standing_home_slots: u32 = core
        .allocator
        .buildings
        .iter()
        .enumerate()
        .filter(|(_, b)| b.zone_type == ZoneType::Residential && !b.is_deserted && !b.broken)
        .map(|(index, b)| {
            if b.is_under_construction() {
                core.allocator.registry.household_capacity(&b.asset_id)
            } else {
                core.allocator
                    .household_capacity(index)
                    .saturating_sub(b.occupancy)
            }
        })
        .sum();
    let unemployed = (0..core.agents.len())
        .filter(|&agent| {
            core.agents.pending_household_size[agent] == 0
                && age_group_can_work(core.agents.age_group[agent])
                && core.agents.home_building[agent] < core.allocator.buildings.len()
                && core.agents.work_building[agent] >= core.allocator.buildings.len()
        })
        .count() as u32;
    let labor_bound = unemployed
        + core.households.pending_arrivals.adults
        + standing_home_slots
        + possible_new_homes;
    let housed_adults = |core: &SimCore| -> u32 {
        core.households
            .households
            .iter()
            .filter(|household| {
                crate::simulation::economy::households::household_is_housed(
                    household,
                    &core.allocator,
                )
            })
            .map(|household| u32::from(household.adult_count))
            .sum()
    };
    let initial_adults = housed_adults(core);
    assert_eq!(
        core.households.pending_arrivals.adults, 0,
        "start from a settled town"
    );
    let import_multiplier = core.demand.runtime_tuning().owa_import_price_multiplier;
    let profiles = ["food_processor_basic", "machinery_factory_basic"];
    let min_workers = profiles
        .iter()
        .map(|id| catalog.profile_for_id(id).unwrap().worker_capacity)
        .min()
        .unwrap();
    assert!(
        labor_bound > 0 && labor_bound < new_lots as u32 * min_workers,
        "the expansion must have a real but scarce workforce: {labor_bound} workers, {new_lots} lots"
    );
    let max_owner_per_worker = profiles
        .iter()
        .map(|id| {
            let profile = catalog.profile_for_id(id).unwrap();
            f64::from(profile.zoned_startup_budget(
                &catalog,
                core.demand.runtime_tuning().owa_import_price_multiplier,
            )) / f64::from(profile.worker_capacity)
        })
        .fold(0.0, f64::max);

    let report = scenario.run_diagnosed_days(270);
    // Planning promises each future home's applicant adults (`DEM-02-C`); admission may
    // admit other families, so the long-run equity bound uses observed workers.
    let supplied_workers =
        unemployed + housed_adults(&scenario.core).saturating_sub(initial_adults);
    assert!(supplied_workers > 0 && supplied_workers < new_lots as u32 * min_workers);
    if let Ok(path) = std::env::var("METRUM_GROWTH_EXPANSION_REPORT") {
        std::fs::write(path, format!(
            "Before expansion: {before:?}; new industrial lots {new_lots}; unemployed {unemployed}; standing home slots {standing_home_slots}; possible new homes {possible_new_homes}; prospective workers {labor_bound}; observed workforce supplied {supplied_workers}; owner capital upper bound {:.2}\n{}",
            f64::from(supplied_workers) * max_owner_per_worker, report.daily_report(),
        )).unwrap();
    }
    verify_accounting(&report);
    let entrants: Vec<_> = report
        .business_outcomes()
        .filter(|firm| profiles.contains(&firm.profile_id.as_str()) && firm.startup_cash > 0.0)
        .collect();
    assert!(
        !entrants.is_empty(),
        "new viable industrial opportunity must actually open: {report}"
    );
    let owner_spent: f64 = entrants.iter().map(|firm| firm.startup_cash).sum();
    assert!(
        owner_spent <= f64::from(supplied_workers) * max_owner_per_worker + 0.01,
        "entry spent {owner_spent} owner capital for only {supplied_workers} supplied workers: {report}"
    );
    assert!(
        entrants.len() < new_lots,
        "zoning is not permission to fund every factory"
    );
    for firm in report
        .business_outcomes()
        .filter(|firm| profiles.contains(&firm.profile_id.as_str()))
    {
        assert!(
            firm.operating_cash_change > 0.0,
            "expansion must not hide an incumbent or entrant's losses: {firm:?}"
        );
    }
    for firm in entrants {
        let profile = catalog.profile_for_id(&firm.profile_id).unwrap();
        assert_eq!(
            firm.startup_cash,
            f64::from(profile.zoned_startup_budget(&catalog, import_multiplier)),
            "each entrant receives only its declared opening equity: {firm:?}"
        );
        assert!(!firm.bankrupt && !firm.removed, "{firm:?}");
        assert!(
            firm.operating_hours >= 90 * 24,
            "the new project must open promptly: {firm:?}"
        );
        assert!(
            firm.capacity_worker_hours > 0.0
                && firm.staffed_worker_hours >= firm.capacity_worker_hours * 0.95,
            "reserved workers must become actual sustained staffing: {firm:?}"
        );
        assert!(
            firm.operating_cash_change > 0.0,
            "survival on unspent owner capital is not acceptance: {firm:?}"
        );
    }
}

fn place_producer(scenario: &mut GrowthScenario, mine: bool, hectares: f32, x: f32) -> usize {
    let core = &mut scenario.core;
    let construction_before = core.treasury.lifetime_build_cost;
    // Large-site experiments use a larger world, keeping the field west of the same town.
    let owner = place_producer_site(core, mine, hectares, x);
    // Declared one-time infrastructure endowment: these cases isolate business operation.
    // The real placement charges stay in lifetime construction costs and report headers;
    // no money is added during the observed operating window or to a running business.
    core.treasury.balance += core.treasury.lifetime_build_cost - construction_before;
    owner
}

fn verify_accounting(report: &GrowthReport) {
    assert!(report.max_city_cash_residual() < 0.1, "{report}");
    assert!(report.max_business_cash_residual() < 0.1, "{report}");
    assert!(report.max_household_cash_residual() < 0.1, "{report}");
}

#[test]
#[ignore = "manual electricity balance: growing village, real fuel freight and 1/2/3 staffed slots"]
fn coal_power_balance_matrix() {
    let directory = std::env::var("METRUM_POWER_REPORT_DIR")
        .unwrap_or_else(|_| "/tmp/metrum-power-balance".into());
    std::fs::create_dir_all(&directory).unwrap();
    for mine in [false, true] {
        for workers in [1_u32, 2, 3] {
            let mut scenario = GrowthScenario::starter_town();
            // Add services while housing remains available, so their vacancies can recruit.
            // The starter town fills its zoned land within days and cannot grow afterwards,
            // leaving no labor for a later plant; place it at founding, like `producer_town`.
            if mine {
                place_producer(&mut scenario, true, 1.0, -900.0);
            }
            let residents_before = scenario.record().residents;
            let core = &mut scenario.core;
            let plant = core
                .place_service_building_internal(
                    "kenney:building.power.coal_power_plant",
                    -700.0,
                    8.0,
                )
                .expect("power plant placement funded by the existing city treasury");
            let x = core.allocator.buildings[plant].center_x;
            let z = core.allocator.buildings[plant].center_y;
            assert!(core.set_building_service_funding_override_at(
                x,
                z,
                "electricity",
                workers as f32 / 20.0
            ));
            let report = scenario.run_diagnosed_days(365);
            // Other buildings may have been swap-removed during the run.
            let plant = scenario
                .core
                .allocator
                .nearest_building_idx_at(x, z)
                .unwrap();
            assert_eq!(
                scenario.core.allocator.buildings[plant].asset_id,
                "kenney:building.power.coal_power_plant"
            );
            let history = &scenario.core.budget_history;
            let sum = |f: fn(&super::super::budget::DailyBudgetLedgerEntry) -> f64| {
                history.iter().rev().take(90).map(f).sum::<f64>()
            };
            let name = format!(
                "power-{workers}-workers-{}",
                if mine { "local-coal" } else { "imported-coal" }
            );
            let summary = format!(
                "{name}: residents before {residents_before}, after {}; trailing 90 days generation {:.2} kWh, imports {:.2} kWh, coal {:.2} kg, revenue {:.2}, fuel/machinery/freight purchases {:.2}, wages {:.2}, net {:.2}; city recurring {:.2}; empty pantry-days {}; actual workers {}\n",
                report.days.last().unwrap().residents,
                sum(|d| d.power_produced),
                sum(|d| d.power_unmet),
                sum(|d| d.coal_consumed),
                sum(|d| d.electricity_revenue),
                sum(|d| d.electricity_fuel_cost),
                sum(|d| d.electricity_wage_cost),
                sum(|d| d.electricity_net),
                report.recurring_city_balance(90),
                report.empty_household_days(90),
                scenario.core.allocator.buildings[plant].worker_count,
            );
            eprint!("{summary}");
            std::fs::write(
                Path::new(&directory).join(format!("{name}.txt")),
                format!("{summary}{}", report.daily_report()),
            )
            .unwrap();
            verify_accounting(&report);
            assert_eq!(
                scenario.core.allocator.buildings[plant].worker_count, workers,
                "{summary}"
            );
            assert!(sum(|d| d.power_produced) > 0.0, "{summary}");
            assert_eq!(report.empty_household_days(90), 0, "{summary}");
            assert!(report.recurring_city_balance(90) > 0.0, "{summary}");
        }
    }
}

#[test]
fn service_placement_and_undo_register_fiscal_capacity_before_the_hourly_census() {
    for fraction in [0.0, 0.5, 1.0] {
        let mut scenario = GrowthScenario::starter_town();
        let core = &mut scenario.core;
        assert!(core.set_service_funding("electricity", fraction));
        let plant = core
            .place_service_building_internal("kenney:building.power.coal_power_plant", -700.0, 8.0)
            .unwrap();
        assert_eq!(core.allocator.buildings[plant].worker_count, 0);
        assert_eq!(
            core.households.job_funding.service_capacities[plant],
            (20.0 * fraction).ceil() as u32
        );
        assert_eq!(
            core.households
                .job_funding
                .capacities
                .get(plant)
                .copied()
                .unwrap_or(0),
            0
        );
        assert!(core.push_building_removal_undo(plant));
        let before = core.treasury.balance;
        assert!(core.allocator.remove_building_for_bulldoze(
            plant,
            &mut core.zoning,
            &mut core.agents,
            &mut core.households,
            &mut core.logistics,
            &mut core.treasury.balance,
        ));
        core.publish_pending_building_site_changes();
        core.seal_building_removal_undo(core.treasury.balance - before);
        assert!(core.set_service_funding("electricity", 1.0 - fraction));
        assert!(core.undo_action_internal());
        assert_eq!(
            core.households.job_funding.service_capacities[plant],
            (20.0 * (1.0 - fraction)).ceil() as u32,
            "undo uses current fiscal policy before the next hourly pass"
        );
        assert_eq!(
            core.households
                .job_funding
                .capacities
                .get(plant)
                .copied()
                .unwrap_or(0),
            0
        );
    }
}

#[test]
fn power_demolition_undo_restores_the_meter_and_refunds_only_its_bill() {
    let mut scenario = GrowthScenario::starter_town();
    let core = &mut scenario.core;
    let plant = core
        .place_service_building_internal("kenney:building.power.coal_power_plant", -700.0, 8.0)
        .unwrap();
    core.allocator.buildings[plant].construction_complete_slot = 0;
    core.allocator.buildings[plant]
        .daily_electricity
        .workplace
        .record(10.0, 0.0);
    let before = core.treasury.balance;
    assert!(core.push_building_removal_undo(plant));
    assert!(core.allocator.remove_building_for_bulldoze(
        plant,
        &mut core.zoning,
        &mut core.agents,
        &mut core.households,
        &mut core.logistics,
        &mut core.treasury.balance,
    ));
    core.publish_pending_building_site_changes();
    core.seal_building_removal_undo(core.treasury.balance - before);
    assert_eq!(core.treasury.balance, before - 2.5);
    assert_eq!(
        core.households.removed_electricity.settlement.demand_units,
        10.0
    );
    assert!(core.undo_action_internal());
    assert_eq!(core.treasury.balance, before);
    assert_eq!(
        core.households.removed_electricity.settlement.demand_units,
        0.0
    );
    assert_eq!(
        core.allocator.buildings[plant]
            .daily_electricity
            .workplace
            .consumed_kwh,
        10.0
    );
    assert!(core.push_building_removal_undo(plant));
    assert!(core.allocator.remove_building_for_bulldoze(
        plant,
        &mut core.zoning,
        &mut core.agents,
        &mut core.households,
        &mut core.logistics,
        &mut core.treasury.balance,
    ));
    core.publish_pending_building_site_changes();
    core.seal_building_removal_undo(core.treasury.balance - before);
    // Isolate the fiscal boundary from unrelated geometry/population changes.
    core.time.day_index += 1;
    assert!(!core.undo_action_internal());
    assert_eq!(core.treasury.balance, before - 2.5);
}

#[test]
fn growing_town_continues_identically_after_a_mid_hour_save_load() {
    let source = growing_town_mid_hour();
    assert!(!source.core.allocator.dirty_index);
    assert_continues_identically_after_reload(source, "growing");
}

#[test]
fn stale_vacancy_index_continues_identically_after_a_save_load() {
    let mut source = growing_town_mid_hour();
    // The hour's last minute with nothing else due to read the index before the hour: no
    // construction start and no cell-lot region.
    loop {
        let core = &source.core;
        let now = u64::from(core.time.day_index - 1) * 1440 + u64::from(core.time.minute_of_day);
        if core.time.minute_of_day % 60 == 59
            && !core.zoning.has_dirty_cell_lots()
            && core
                .pending_demand_spawns
                .iter()
                .all(|pending| pending.due_minute > now + 1)
        {
            break;
        }
        assert!(
            source.core.time.day_index < 5,
            "no quiet hour end in two days"
        );
        source.core.step_frame(FRAME_DT_S);
    }
    // A player's placement. Every editing path reads the index before it returns, so a save
    // normally finds it clean; this save is taken as if the placement had left it stale for
    // its next reader, the next frame's flow-field drain; the `housing requests` queue then
    // reviews every household against the rebuilt index.
    source
        .core
        .place_service_building_internal(COAL_POWER_PLANT, -560.0, 8.0)
        .unwrap();
    let core = &mut source.core;
    let slot = core.time.absolute_slot();
    let households = core.households.households.len();
    core.households.handles.register_appended(households);
    for household_id in 0..households {
        core.households.request_housing_review(household_id, slot);
    }
    core.allocator.dirty_index = true;
    assert!(!source.core.households.housing_reviews.is_empty());
    assert_continues_identically_after_reload(source, "stale");
}

// A road placed mid-hour rebuilds the CCH, so the freight supplier index's front and its sweep
// in progress hold the old graph's components: the game builds its own index at the next
// planning pass and abandons the sweep. A save taken before the next slot must continue the
// same way instead of labelling the stale buffers with the loaded graph's components.
#[test]
fn a_supplier_index_left_stale_by_a_road_edit_continues_identically_after_a_save_load() {
    let mut source = growing_town_mid_hour();
    assert!(
        source.core.logistics.supplier_roll.saved().sweep.is_some(),
        "a supplier sweep in progress"
    );
    commit_road(&mut source.core, (MAIN_STREET_END_X_M, 0.0), (450.0, 0.0));
    assert_continues_identically_after_reload(source, "supplier-roll");
}

#[test]
fn queued_freight_requests_continue_identically_after_a_mid_hour_save_load() {
    use crate::simulation::economy::logistics::{FreightRequest, FreightRequestKind};
    let mut source = growing_town_mid_hour();
    // A backlog of one inbound and one export request per building, which the queue serves
    // over many slots after the save, beside the requests the phases keep queueing.
    let core = &mut source.core;
    assert!(
        core.logistics.next_shipment_id > 0,
        "the town ships freight"
    );
    let slot = core.time.absolute_slot();
    let buildings = core.allocator.buildings.len();
    core.allocator.building_handles.register_appended(buildings);
    for building_idx in 0..buildings {
        let building = core.allocator.building_handles.handle(building_idx);
        for kind in [FreightRequestKind::Inbound, FreightRequestKind::Export] {
            core.logistics.freight_requests.push_back(FreightRequest {
                slot,
                building,
                kind,
            });
        }
    }
    assert!(core.logistics.freight_requests.len() >= 64);
    assert_continues_identically_after_reload(source, "freight");
}

#[test]
fn queued_household_requests_continue_identically_after_a_mid_hour_save_load() {
    let mut source = growing_town_mid_hour();
    // A rehousing, review, restock and visit request per household and a job search per agent,
    // which the queues serve over the slots after the save, beside the requests the phases keep
    // queueing.
    let core = &mut source.core;
    let slot = core.time.absolute_slot();
    let households = core.households.households.len();
    core.households.handles.register_appended(households);
    for household_id in 0..households {
        let handle = core.households.handles.handle(household_id);
        let queues = &mut core.households;
        queues.rehousing.push(slot, handle);
        queues.housing_reviews.push(slot, handle);
        queues.restock_requests.push(slot, handle);
        queues.visit_requests.push(slot, handle);
    }
    // And a job search per agent.
    let agents = core.agents.len();
    core.agents.handles.register_appended(agents);
    for agent_idx in 0..agents {
        let handle = core.agents.agent_handle(agent_idx);
        core.households.job_requests.push(slot, handle);
    }
    assert!(households >= 16, "a backlog beyond one slot's quota");
    assert!(agents >= 64, "a job search backlog beyond one slot's quota");
    assert_continues_identically_after_reload(source, "household-requests");
}

// A starter town with a power plant, two days in and saved at mid-hour: homes have been claimed
// and released, so the vacancy lists are no longer in building order, and households keep
// arriving.
fn growing_town_mid_hour() -> GrowthScenario {
    let mut source = GrowthScenario::starter_town();
    source
        .core
        .place_service_building_internal(COAL_POWER_PLANT, -700.0, 8.0)
        .unwrap();
    assert!(
        source
            .core
            .set_building_service_funding_override_at(-700.0, 8.0, "electricity", 0.05)
    );
    source.run_days(2);
    while !(10..=50).contains(&(source.core.time.minute_of_day % 60)) {
        source.core.step_frame(FRAME_DT_S);
    }
    assert!(
        source
            .core
            .allocator
            .vacancy_index
            .iter()
            .any(|list| !list.is_sorted()),
        "the fixture must save a vacancy order a rebuild would not recover"
    );
    source
}

// Saves `source`, loads it into a fresh town and steps both for a day, comparing homes, cash,
// occupancy, the vacancy index and the freight supplier index's buffers every frame.
fn assert_continues_identically_after_reload(source: GrowthScenario, name: &str) {
    assert_continues_identically_after_checked_reload(source, name, |_, _| {});
}

// [`assert_continues_identically_after_reload`], first checking the games with `check` once
// loaded.
fn assert_continues_identically_after_checked_reload(
    source: GrowthScenario,
    name: &str,
    check: impl FnOnce(&SimCore, &SimCore),
) {
    assert_continues_identically_after_prepared_reload(source, name, |_| {}, check);
}

// [`assert_continues_identically_after_checked_reload`], first readying the fresh town the save
// loads into with `prepare` (assets the source registered).
fn assert_continues_identically_after_prepared_reload(
    mut source: GrowthScenario,
    name: &str,
    prepare: impl FnOnce(&mut SimCore),
    check: impl FnOnce(&SimCore, &SimCore),
) -> GrowthScenario {
    let path = std::env::temp_dir().join(format!(
        "metrum-{name}-continuation-{}.sqlite",
        std::process::id()
    ));
    source
        .core
        .save_game_settled(path.to_str().unwrap(), None)
        .unwrap();
    let mut loaded = GrowthScenario::starter_town();
    prepare(&mut loaded.core);
    loaded
        .core
        .load_game_internal(path.to_str().unwrap())
        .unwrap();
    std::fs::remove_file(path).unwrap();
    let (a, b) = (&source.core.logistics, &loaded.core.logistics);
    assert_eq!(
        a.freight_requests, b.freight_requests,
        "queued freight requests"
    );
    assert_eq!(a.next_shipment_id, b.next_shipment_id, "next shipment id");
    let (a, b) = (&source.core.households, &loaded.core.households);
    assert_eq!(
        a.housing_reviews, b.housing_reviews,
        "queued housing reviews"
    );
    assert_eq!(a.rehousing, b.rehousing, "queued rehousings");
    assert_eq!(a.restock_requests, b.restock_requests, "queued restocks");
    assert_eq!(a.visit_requests, b.visit_requests, "queued visits");
    assert_eq!(a.visit_credit, b.visit_credit, "visit credit");
    assert_eq!(a.job_requests, b.job_requests, "queued job searches");
    assert_eq!(a.job_board.saved(), b.job_board.saved(), "job board");
    assert_eq!(
        a.destinations.saved(),
        b.destinations.saved(),
        "household destinations"
    );
    loaded.core.time.speed_multiplier = source.core.time.speed_multiplier;
    let nodes = SavedNodes::of(&source.core, &loaded.core);
    assert_same_demand_queues(&source.core, &loaded.core, "at load");
    check(&source.core, &loaded.core);
    let target_day = source.core.time.day_index + 1;
    loop {
        let when = (source.core.time.day_index, source.core.time.minute_of_day);
        let (a, b) = (&source.core.allocator, &loaded.core.allocator);
        assert_eq!(a.dirty_index, b.dirty_index, "{when:?}");
        // A stale index is rebuilt by its next reader in both games.
        if !a.dirty_index {
            assert_eq!(
                (&a.vacancy_index, &a.vacancy_pos),
                (&b.vacancy_index, &b.vacancy_pos),
                "vacancies at {when:?}"
            );
            // The saved game's order is kept incrementally, the loaded one's rebuilt at load.
            assert!(a.admission_order.same_as(&b.admission_order), "{when:?}");
        }
        assert_eq!(
            source.core.logistics.supplier_roll.saved(),
            loaded.core.logistics.supplier_roll.saved(),
            "supplier index at {when:?}"
        );
        // A queue's marks are a function of its requests, so a load's rebuild equals them.
        for core in [&source.core, &loaded.core] {
            let households = &core.households;
            assert!(
                households.rehousing.marks_follow_queue()
                    && households.housing_reviews.marks_follow_queue()
                    && households.restock_requests.marks_follow_queue()
                    && households.visit_requests.marks_follow_queue()
                    && households.job_requests.marks_follow_queue(),
                "queue marks at {when:?}"
            );
        }
        if source.core.time.day_index >= target_day {
            break;
        }
        source.core.step_frame(FRAME_DT_S);
        loaded.core.step_frame(FRAME_DT_S);
        let when = (source.core.time.day_index, source.core.time.minute_of_day);
        assert_eq!(serde_json::to_value(&source.core.agents.parking_trip).unwrap(), serde_json::to_value(&loaded.core.agents.parking_trip).unwrap(), "parking continuation at {when:?}");
        assert_same_flow_fields(&source.core, &loaded.core, &nodes, when);
        assert_same_demand_queues(&source.core, &loaded.core, &format!("{when:?}"));
        // In the hour's planning slots both games lend the published books, or skip, alike.
        if source.core.time.absolute_slot() % crate::simulation::core::scheduler::SLOTS_PER_HOUR
            <= crate::simulation::economy::demand::PlanningSlot::Actions as u64
        {
            assert_eq!(
                super::census_cases::published_books_lent(&source.core),
                super::census_cases::published_books_lent(&loaded.core),
                "published books at {when:?}"
            );
        }
        assert_eq!(
            hour_plan(&source.core),
            hour_plan(&loaded.core),
            "hour plan at {when:?}"
        );
        assert_eq!(
            source.core.agents.len(),
            loaded.core.agents.len(),
            "agents at {when:?}"
        );
        let queued = |core: &SimCore| {
            core.pending_demand_spawns
                .iter()
                .map(|pending| (pending.id, pending.permit))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            queued(&source.core),
            queued(&loaded.core),
            "queued spawns at {when:?}"
        );
        let (a, b) = (&source.core.households, &loaded.core.households);
        assert_eq!(
            a.households.len(),
            b.households.len(),
            "households at {when:?}"
        );
        for (x, y) in a.households.iter().zip(&b.households) {
            assert_eq!(x.home_building_id, y.home_building_id, "home at {when:?}");
            assert_eq!(x.budget.to_bits(), y.budget.to_bits(), "budget at {when:?}");
            assert_eq!(x.stock.to_bits(), y.stock.to_bits(), "stock at {when:?}");
        }
        let (a, b) = (&source.core.allocator, &loaded.core.allocator);
        assert_eq!(
            a.buildings.len(),
            b.buildings.len(),
            "buildings at {when:?}"
        );
        for (x, y) in a.buildings.iter().zip(&b.buildings) {
            assert_eq!(
                (x.parcel_id, x.occupancy),
                (y.parcel_id, y.occupancy),
                "{when:?}"
            );
            assert_eq!(
                x.operating_budget.to_bits(),
                y.operating_budget.to_bits(),
                "{when:?}"
            );
        }
    }
    source
}

// The demand queues (`SIM-02-J`): level changes with their permits, building removals,
// departures, and admissions with the admission credit; an admission's border by position, which
// a save renumbers.
fn assert_same_demand_queues(source: &SimCore, loaded: &SimCore, when: &str) {
    assert_eq!(
        source.demand.level_change_requests, loaded.demand.level_change_requests,
        "queued level changes {when}"
    );
    assert_eq!(
        source.allocator.removal_requests, loaded.allocator.removal_requests,
        "queued building removals {when}"
    );
    let (a, b) = (&source.households, &loaded.households);
    assert_eq!(a.exits, b.exits, "queued departures {when}");
    assert_eq!(
        (a.admission_credit, a.admission_rate),
        (b.admission_credit, b.admission_rate),
        "admission credit {when}"
    );
    assert_eq!(
        a.pending_arrivals, b.pending_arrivals,
        "pending arrivals {when}"
    );
    let admissions = |core: &SimCore| {
        core.households
            .admission_requests
            .iter()
            .map(|admission| {
                let border = core.region_graph.get_valid_node(admission.border_node);
                let position = core.region_graph.node(border).pos;
                (
                    admission.home,
                    admission.size,
                    admission.composition,
                    position.x.to_bits(),
                    position.z.to_bits(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        admissions(source),
        admissions(loaded),
        "queued admissions {when}"
    );
}

// Registers a level-2 twin of every residential asset of the starter town's packs, so its homes
// have an upgrade (`SIM-02-J` tests): the same lot, capacity and door one level up.
fn register_home_upgrades(core: &mut SimCore) {
    let registry = &mut core.allocator.registry;
    let twins: Vec<_> = registry
        .qualified_ids()
        .filter_map(|id| {
            let entry = registry.get(id)?;
            let building = entry.manifest.building.as_ref()?;
            (building.zone_type == Some(crate::assets::ZoneClass::Residential)
                && building.level == 1
                && entry.manifest.asset_set.is_none())
            .then(|| {
                let mut manifest = entry.manifest.clone();
                manifest.asset_id = format!("{}_level_2", manifest.asset_id);
                manifest.building.as_mut().unwrap().level = 2;
                (entry.pack_id.clone(), manifest, entry.asset_dir.clone())
            })
        })
        .collect();
    assert!(!twins.is_empty(), "the starter town has homes to upgrade");
    for (pack, manifest, dir) in twins {
        registry.register(&pack, manifest, dir);
    }
}

// Queues one level change that fits now, with its permit, as the hour's actions slot queues a
// proved one (`SIM-02-J`): the first building, in dense order, whose upgrade or else downgrade
// fits, preferring `zone`. Returns the building.
fn queue_fitting_level_change(core: &mut SimCore, zone: Option<ZoneType>) -> Option<usize> {
    use crate::simulation::buildings::allocator::BuildingSiteEnvironment;
    use crate::simulation::economy::demand::{QueuedLevelChange, level_change_permit};
    let cell_m = core.config.zone_cell_m;
    core.allocator.prepare_building_site_query_index(cell_m);
    let catalog = core.demand.shared_runtime_catalog();
    let allocator = &core.allocator;
    let (building_idx, target) = (0..allocator.buildings.len()).find_map(|index| {
        let building = &allocator.buildings[index];
        if zone.is_some_and(|zone| building.zone_type != zone) {
            return None;
        }
        [
            allocator.upgrade_target(building),
            allocator.downgrade_target(building),
        ]
        .into_iter()
        .flatten()
        .find(|target| {
            allocator.level_change_fits(
                index,
                target,
                &catalog,
                cell_m,
                &core.region_graph,
                BuildingSiteEnvironment {
                    road_surface: &core.transit_network.roadbed,
                    terrain: &core.heightmap,
                },
            )
        })
        .map(|target| (index, target.to_owned()))
    })?;
    let permit = level_change_permit(
        &allocator.buildings[building_idx],
        Some(&allocator.derived_entrance(
            building_idx,
            &core.region_graph,
            &core.transit_network.lane_system,
        )),
        &core.region_graph,
        &core.transit_network.lane_system,
    );
    let building = allocator.building_handles.handle(building_idx);
    core.demand
        .level_change_requests
        .push_back(QueuedLevelChange {
            slot: core.time.absolute_slot(),
            building,
            target_asset_id: target,
            permit,
        });
    Some(building_idx)
}

// Takes the home of the first housed household in a single-household home, as if it had lost
// it a week ago: the home's place is released and the household and its members are homeless.
// Returns the household and its former home.
fn evict_a_household(core: &mut SimCore) -> (usize, usize) {
    let household = (0..core.households.households.len())
        .find(|&id| {
            let home = core.households.households[id].home_building_id;
            core.households.households[id].member_count > 0
                && core
                    .allocator
                    .buildings
                    .get(home)
                    .is_some_and(|home| home.occupancy == 1 && home.worker_count == 0)
        })
        .expect("a household alone in its home");
    let home = core.households.households[household].home_building_id;
    core.allocator.release_vacancy(home);
    core.households.households[household].home_building_id = usize::MAX;
    core.households.households[household].unhoused_days_elapsed = 7;
    for agent in 0..core.agents.len() {
        if core.agents.household_id[agent] == household {
            core.agents.home_building[agent] = usize::MAX;
        }
    }
    (household, home)
}

// Marks empty, sound and complete building `building_idx` deserted, as a failed site is, and
// queues its planned clearance (`SIM-02-J`). Returns its handle.
fn queue_clearance(
    core: &mut SimCore,
    building_idx: usize,
) -> crate::simulation::core::handles::Handle<crate::simulation::buildings::allocator::Building> {
    use crate::simulation::buildings::allocator::{BuildingRemovalKind, BuildingRemovalRequest};
    let allocator = &mut core.allocator;
    allocator.mark_building_deserted(building_idx);
    assert!(allocator.can_demand_despawn(building_idx));
    allocator
        .building_handles
        .register_appended(allocator.buildings.len());
    let building = allocator.building_handles.handle(building_idx);
    allocator
        .removal_requests
        .push_back(BuildingRemovalRequest {
            slot: core.time.absolute_slot(),
            building,
            kind: BuildingRemovalKind::Clearance,
        });
    building
}

// The demand queues carry out the actions slot's decisions over the hour (`SIM-02-J`): a town
// saved right after an hour's actions slot, with admissions, a level change, a planned
// clearance and a departure waiting, serves them from the removal window's start on and
// continues identically every frame for a day. The departure is a household the player's bulldozer left unhoused long
// enough to leave.
#[test]
fn a_save_with_the_demand_queues_waiting_continues_identically() {
    use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
    let mut source = GrowthScenario::starter_town();
    register_home_upgrades(&mut source.core);
    loop {
        assert!(source.day() < 6, "no hour left admissions waiting");
        let hour = source.core.time.absolute_slot() / SLOTS_PER_HOUR + 1;
        step_to_slot(
            &mut source.core,
            hour * SLOTS_PER_HOUR + PlanningSlot::Actions as u64,
        );
        if source.core.households.admission_requests.len() >= 2
            && source.core.households.households.len() >= 8
        {
            break;
        }
    }
    let core = &mut source.core;
    // The household leaves the city; its emptied home is cleared.
    let (unhoused, home) = evict_a_household(core);
    assert!(core.households.exit_eligible(
        unhoused,
        core.demand.household_exit_policy(),
        &core.allocator
    ));
    core.households.queue_exit(unhoused);
    let cleared = queue_clearance(core, home);
    let changed = queue_fitting_level_change(core, None).expect("a level change that fits");
    let changed = core.allocator.building_handles.handle(changed);
    let leaving = core.households.handles.handle(unhoused);
    let households = core.households.households.len();
    let buildings = core.allocator.buildings.len();
    let source = assert_continues_identically_after_prepared_reload(
        source,
        "demand-queues",
        register_home_upgrades,
        |source, _| {
            assert!(!source.demand.level_change_requests.is_empty());
            assert!(!source.allocator.removal_requests.is_empty());
            assert!(!source.households.exits.is_empty());
            assert!(source.households.admission_requests.len() >= 2);
            assert_eq!(source.households.households.len(), households);
            assert_eq!(source.allocator.buildings.len(), buildings);
        },
    );
    // Served from the removal window: the household left, the home was cleared, the change was
    // applied.
    let core = &source.core;
    assert!(
        core.households.handles.resolve(leaving).is_none(),
        "the household left"
    );
    assert!(
        core.allocator.building_handles.resolve(cleared).is_none(),
        "the home was cleared"
    );
    let changed = core
        .allocator
        .building_handles
        .resolve(changed)
        .expect("the changed building");
    assert_eq!(
        core.allocator.buildings[changed].level, 2,
        "the level change was applied"
    );
}

// Removals wait for the hour's removal window (`SIM-02-J`): a planned clearance queued while
// the census runs over the hour's second half stays until the next hour's removal window, while
// a level change queued then is applied at once, an in-place change. The census runs on without
// a restart and is ready at its publishing slot, the books it published stay lent through the
// next hour's planning slots, and the window's first slot removes the building; the next census,
// over the copy that followed the removal, runs without a restart.
#[test]
fn a_removal_requested_in_the_census_window_waits_for_the_removal_window_and_restarts_nothing() {
    use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
    use crate::simulation::economy::hiring::{CENSUS_WINDOW_START, REMOVAL_WINDOW_START};
    let mut scenario = super::census_cases::busy_town();
    let core = &mut scenario.core;
    let hour = core.time.absolute_slot() / SLOTS_PER_HOUR + 1;
    step_to_slot(core, hour * SLOTS_PER_HOUR + CENSUS_WINDOW_START + 20);
    assert!(core.households.census_inputs.is_read(), "a census runs");
    let restarts = core.households.census_run.restarts;
    let (_, home) = evict_a_household(core);
    let building = queue_clearance(core, home);
    // A level change in the window is an in-place change: applied at once, it restarts nothing.
    register_home_upgrades(core);
    let changed = queue_fitting_level_change(core, Some(ZoneType::Residential))
        .expect("a home whose upgrade fits");
    let changed = core.allocator.building_handles.handle(changed);
    let now = core.time.absolute_slot();
    step_to_slot(core, now + 1);
    let changed = core.allocator.building_handles.resolve(changed).unwrap();
    assert_eq!(
        core.allocator.buildings[changed].level, 2,
        "applied in the window"
    );
    let next = (hour + 1) * SLOTS_PER_HOUR;
    step_to_slot(core, next - 1);
    assert!(core.allocator.building_handles.resolve(building).is_some());
    let key = core.households.census_key(&core.transit_network);
    assert!(
        core.households.census_ready(next, key),
        "the census is current at its publication"
    );
    for slot in next..next + REMOVAL_WINDOW_START {
        step_to_slot(core, slot);
        assert!(
            core.allocator.building_handles.resolve(building).is_some(),
            "the removal waits at slot {}",
            slot % SLOTS_PER_HOUR
        );
        assert!(
            super::census_cases::published_books_lent(core),
            "planning slot {} lends the published books",
            slot % SLOTS_PER_HOUR
        );
    }
    assert_eq!(
        core.households.census_run.restarts, restarts,
        "no census restarted"
    );
    step_to_slot(core, next + REMOVAL_WINDOW_START);
    assert!(
        core.allocator.building_handles.resolve(building).is_none(),
        "removed at the removal window's first slot"
    );
    // The filling input copy followed the swap-remove: the census over it runs through the
    // window without a restart and is current at its publication.
    assert!(core.households.census_inputs.is_filling());
    let publish = next + SLOTS_PER_HOUR;
    step_to_slot(core, publish - 1);
    let key = core.households.census_key(&core.transit_network);
    assert!(core.households.census_ready(publish, key));
    assert_eq!(core.households.census_run.restarts, restarts);
}

// A queued level change is applied only to the building state its permit was proved for
// (`SIM-02-J`): after the building changed level another way, the queue rejects the stale
// permit and proves nothing again. A business permit also names the door.
#[test]
fn a_stale_level_change_permit_is_rejected() {
    use crate::simulation::buildings::allocator::BuildingSiteEnvironment;
    use crate::simulation::economy::demand::level_change_permit;
    let mut scenario = GrowthScenario::starter_town();
    register_home_upgrades(&mut scenario.core);
    scenario.run_days(2);
    let core = &mut scenario.core;
    let building_idx = queue_fitting_level_change(core, None).expect("a level change that fits");
    let request = core.demand.level_change_requests.back().unwrap().clone();
    let before = core.allocator.buildings[building_idx].asset_id.clone();
    // The same change made another way: the building no longer stands as proved.
    let catalog = core.demand.shared_runtime_catalog();
    assert!(
        core.allocator
            .apply_level_change_action(
                building_idx,
                &request.target_asset_id,
                &catalog,
                core.config.zone_cell_m,
                &core.region_graph,
                BuildingSiteEnvironment {
                    road_surface: &core.transit_network.roadbed,
                    terrain: &core.heightmap,
                },
            )
            .is_some()
    );
    let changed = core.allocator.buildings[building_idx].clone();
    assert_ne!(changed.asset_id, before);
    core.serve_level_change_queue(usize::MAX, &mut ());
    assert!(core.demand.level_change_requests.is_empty());
    let building = &core.allocator.buildings[building_idx];
    assert_eq!(
        (&building.asset_id, building.level),
        (&changed.asset_id, changed.level),
        "a stale permit is rejected, so the change is not made twice"
    );
    // A business permit names its door; a home's does not.
    let original = core.allocator.derived_entrance(
        building_idx,
        &core.region_graph,
        &core.transit_network.lane_system,
    );
    let mut entrance = original.clone();
    entrance.door_pos.x += 1.0;
    let graph = &core.region_graph;
    let lanes = &core.transit_network.lane_system;
    let mut business = changed.clone();
    business.zone_type = ZoneType::Commercial;
    let digest = |building, entrance| level_change_permit(building, Some(entrance), graph, lanes);
    assert_ne!(digest(&business, &original), digest(&business, &entrance));
    let mut home = changed;
    home.zone_type = ZoneType::Residential;
    assert_eq!(digest(&home, &original), digest(&home, &entrance));
}

// One action of the hour's plan: kind, parcel, side, cell, width, depth, level, asset, target.
type PlanRow = (u8, u64, i8, usize, u16, u16, u8, String, String);

// The hour's plan between its planning slots (`SIM-02-I`): each zone's spawns, upgrades,
// downgrades and despawns, then the optional homes, by parcel, lot and asset; building keys
// leave out their edge, which a save renumbers.
fn hour_plan(core: &SimCore) -> Vec<PlanRow> {
    use crate::simulation::economy::demand::DemandBuildingActionKey;
    let key = |kind: u8, key: &DemandBuildingActionKey, target: &str| {
        (
            kind,
            key.parcel_id,
            key.side,
            key.cell_x,
            key.width_cells,
            key.depth_cells,
            key.level,
            key.asset_id.clone(),
            target.to_owned(),
        )
    };
    let spawn = |kind: u8, action: &crate::simulation::economy::demand::DemandSpawnAction| {
        (
            kind,
            action.parcel_id,
            0,
            0,
            0,
            0,
            0,
            action.asset_id.clone(),
            String::new(),
        )
    };
    let plan = &core.demand.building_actions;
    let mut rows = Vec::new();
    for (zone, use_plan) in [&plan.residential, &plan.commercial, &plan.industrial]
        .into_iter()
        .enumerate()
    {
        let zone = zone as u8 * 8;
        rows.extend(use_plan.spawns.iter().map(|action| spawn(zone, action)));
        rows.extend(
            use_plan
                .upgrades
                .iter()
                .map(|action| key(zone + 1, &action.building, &action.target_asset_id)),
        );
        rows.extend(
            use_plan
                .downgrades
                .iter()
                .map(|action| key(zone + 2, &action.building, &action.target_asset_id)),
        );
        rows.extend(
            use_plan
                .despawns
                .iter()
                .map(|action| key(zone + 3, action, "")),
        );
    }
    rows.extend(
        core.demand
            .optional_homes
            .iter()
            .map(|action| spawn(32, action)),
    );
    rows
}

// How a save renumbered the saved game's road nodes: the loaded game's id of each node the save
// kept, and both games' node counts at the load, past which both append the same nodes in the
// same order.
struct SavedNodes {
    saved: std::collections::HashMap<u32, u32>,
    source_count: usize,
    loaded_count: usize,
}

impl SavedNodes {
    fn of(source: &SimCore, loaded: &SimCore) -> Self {
        let saved = crate::simulation::save::saved_node_numbering(
            &source.region_graph,
            &source.allocator,
            &source.agents,
        );
        assert_eq!(
            saved.len(),
            loaded.region_graph.node_count(),
            "the loaded graph holds exactly the saved nodes"
        );
        Self {
            saved,
            source_count: source.region_graph.node_count(),
            loaded_count: loaded.region_graph.node_count(),
        }
    }

    // The loaded game's id of the saved game's node `node`; `None` for a node the save dropped.
    fn loaded(&self, node: u32) -> Option<u32> {
        match (node as usize).checked_sub(self.source_count) {
            Some(appended) => Some((self.loaded_count + appended) as u32),
            None => self.saved.get(&node).copied(),
        }
    }
}

// The flow fields both games route on, car and foot, for every private zone (`SIM-02-I`). A
// field names nodes, so the saved game's are translated through the save's numbering: each
// node the save kept has the loaded node's next node and nearest building, and a node the save
// dropped (one no live edge or agent names) is unreachable.
fn assert_same_flow_fields(a: &SimCore, b: &SimCore, nodes: &SavedNodes, when: (u32, u16)) {
    use crate::simulation::pathing::flow_field::FlowField;
    let same = |x: &FlowField, y: &FlowField| {
        let mut kept = 0;
        for (node, (&next, &building)) in x.next_node.iter().zip(&x.nearest_building).enumerate() {
            let Some(loaded) = nodes.loaded(node as u32) else {
                if (next, building) != (u32::MAX, usize::MAX) {
                    return false;
                }
                continue;
            };
            kept += 1;
            let next = (next != u32::MAX).then(|| nodes.loaded(next)).flatten();
            let loaded = loaded as usize;
            if y.next_node.get(loaded).copied() != Some(next.unwrap_or(u32::MAX))
                || y.nearest_building.get(loaded) != Some(&building)
            {
                return false;
            }
        }
        kept == y.next_node.len() && y.nearest_building.len() == y.next_node.len()
    };
    let (a, b) = (
        &a.transit_network.flow_fields,
        &b.transit_network.flow_fields,
    );
    for zone in [
        ZoneType::Residential,
        ZoneType::Commercial,
        ZoneType::Industrial,
    ] {
        for (mode, x, y) in [
            ("car", a.car(zone), b.car(zone)),
            ("foot", a.foot(zone), b.foot(zone)),
        ] {
            let equal = match (x, y) {
                (Some(x), Some(y)) => same(x, y),
                (x, y) => x.is_none() && y.is_none(),
            };
            assert!(equal, "{zone:?} {mode} flow field at {when:?}");
        }
    }
}

// Whether a commercial or industrial spawn waits in the queue under a granted permit.
fn business_permit_queued(core: &SimCore) -> bool {
    core.pending_demand_spawns.iter().any(|pending| {
        pending.zone_type != ZoneType::Residential && pending.permit.granted().is_some()
    })
}

// A starter town stepped to a mid-hour moment with a business spawn queued under its permit.
fn town_with_a_queued_business_permit() -> GrowthScenario {
    let mut source = GrowthScenario::starter_town();
    loop {
        assert!(
            source.day() < 5,
            "no business spawn waited mid-hour with a permit"
        );
        source.core.step_frame(FRAME_DT_S);
        let core = &source.core;
        if (10..=50).contains(&(core.time.minute_of_day % 60)) && business_permit_queued(core) {
            return source;
        }
    }
}

// A game saved mid-hour with commercial or industrial spawns queued under their planning slot's
// permits continues identically every frame for a day: permits are saved, and each start checks
// its permit against its door, profile and level, so no proof runs at the start (`SIM-02-I`).
#[test]
fn queued_business_permits_continue_identically_after_a_mid_hour_save_load() {
    assert_continues_identically_after_reload(
        town_with_a_queued_business_permit(),
        "queued-permits",
    );
}

// The hour's plan lives between its planning slots (`SIM-02-I`): a game saved after the
// residential, commercial or industrial slot of an hour whose slots planned spawns continues
// identically every frame for a day, its saved plan proved again by the later slots on the
// restored published census and queued and executed at the actions slot as the uninterrupted
// game does, with the same flow fields.
#[test]
fn the_hours_plan_continues_identically_after_a_save_between_its_slots() {
    use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
    let industrial = PlanningSlot::Industrial as u64;
    for saved_after in [
        PlanningSlot::Residential,
        PlanningSlot::Commercial,
        PlanningSlot::Industrial,
    ]
    .map(|slot| slot as u64)
    {
        let mut source = GrowthScenario::starter_town();
        loop {
            assert!(
                source.day() < 4,
                "no hour planned spawns before slot {saved_after}"
            );
            let core = &mut source.core;
            let hour = core.time.absolute_slot() / SLOTS_PER_HOUR + 1;
            while core.time.absolute_slot() < hour * SLOTS_PER_HOUR + saved_after {
                core.step_frame(FRAME_DT_S / 16.0);
            }
            let plan = &core.demand.building_actions;
            // The industrial slot's save also needs a business spawn to prove at the actions
            // slot.
            let business = !plan.commercial.spawns.is_empty() || !plan.industrial.spawns.is_empty();
            if !plan.residential.spawns.is_empty() && (saved_after < industrial || business) {
                break;
            }
        }
        assert_continues_identically_after_checked_reload(
            source,
            &format!("planning-slot-{saved_after}"),
            super::census_cases::assert_same_published_census,
        );
    }
}

// Steps `core` in short frames until absolute slot `slot` has run.
fn step_to_slot(core: &mut SimCore, slot: u64) {
    while core.time.absolute_slot() < slot {
        core.step_frame(FRAME_DT_S / 16.0);
    }
}

// The hour's later planning slots read the census its first slot published (`SIM-02-I`). After
// a fiscal stress change at that slot (commercial and industrial property taxes at 500 a day),
// a census of the loaded city would fund the new policy's books while the published one keeps
// the policy its copy froze: a busy town saved at slot 1 restores the published books, plans
// the commercial and industrial slots, proves the actions slot's permits and continues as the
// uninterrupted game does.
#[test]
fn a_save_at_slot_1_after_a_fiscal_change_plans_on_the_published_census() {
    use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
    use crate::simulation::economy::fiscal::{
        POLICY_COMMERCIAL_PROPERTY_TAX, POLICY_INDUSTRIAL_PROPERTY_TAX,
    };
    let mut source = super::census_cases::busy_town();
    let hour = source.core.time.absolute_slot() / SLOTS_PER_HOUR + 1;
    step_to_slot(&mut source.core, hour * SLOTS_PER_HOUR);
    for policy in [
        POLICY_COMMERCIAL_PROPERTY_TAX,
        POLICY_INDUSTRIAL_PROPERTY_TAX,
    ] {
        assert!(source.core.set_fiscal_policy_value(policy, 500.0));
    }
    step_to_slot(&mut source.core, hour * SLOTS_PER_HOUR + 1);
    assert_continues_identically_after_checked_reload(
        source,
        "fiscal-slot-1",
        super::census_cases::assert_same_published_census,
    );
}

// A game saved after the hour's actions slot, before the first queued start a minute later
// (`SIM-02-I`): the queue's permits and counted flags, the arrivals paced from the actions slot,
// the cleared plan, the hour's admission and the indices the actions rebuilt continue as the
// uninterrupted game's.
#[test]
fn a_save_after_the_actions_slot_before_the_first_start_continues_identically() {
    use crate::simulation::core::scheduler::{SLOTS_PER_HOUR, SLOTS_PER_MINUTE};
    let mut source = GrowthScenario::starter_town();
    loop {
        assert!(
            source.day() < 4,
            "no hour queued a business spawn under its permit"
        );
        let hour = source.core.time.absolute_slot() / SLOTS_PER_HOUR + 1;
        step_to_slot(
            &mut source.core,
            hour * SLOTS_PER_HOUR + PlanningSlot::Actions as u64,
        );
        if business_permit_queued(&source.core) {
            break;
        }
    }
    let core = &source.core;
    assert_eq!(
        core.time.absolute_slot() % SLOTS_PER_HOUR / SLOTS_PER_MINUTE,
        PlanningSlot::Actions as u64 / SLOTS_PER_MINUTE
    );
    assert!(
        hour_plan(core).is_empty(),
        "the actions slot cleared the hour's plan"
    );
    assert_continues_identically_after_checked_reload(source, "after-actions", |source, loaded| {
        assert!(hour_plan(loaded).is_empty());
        assert_eq!(
            source.demand.households_to_admit_today,
            loaded.demand.households_to_admit_today
        );
        let counted = |core: &SimCore| {
            core.pending_demand_spawns
                .iter()
                .map(|pending| (pending.id, pending.permit, pending.counted))
                .collect::<Vec<_>>()
        };
        assert_eq!(counted(source), counted(loaded));
    });
}

// A save renumbers edges past a deleted one and a load rebuilds lanes compactly, while the
// session tombstones and appends: after a road split and a bulldozed stub far from town, the
// loaded game numbers the town's edges and lanes differently, and every queued permit still
// matches its door, so the game continues identically (`SIM-02-I`).
#[test]
fn queued_business_permits_survive_edge_and_lane_renumbering_across_a_save() {
    let mut source = town_with_a_queued_business_permit();
    let core = &mut source.core;
    let x = BORDER_INSET_M - core.heightmap.half_world_extents().0 + 120.0;
    super::commit_road(core, (x, 0.0), (x, 80.0));
    let stub = core
        .region_graph
        .edges()
        .iter()
        .rposition(|edge| {
            !edge.deleted
                && edge
                    .geometry
                    .iter()
                    .any(|point| (point.x - x).abs() < 1.0 && (point.z - 80.0).abs() < 1.0)
        })
        .expect("the stub was committed");
    assert!(core.bulldoze_road_edge_for_test(stub));
    assert!(
        core.region_graph.edges().iter().any(|edge| edge.deleted),
        "the split and the bulldoze leave edges a save renumbers"
    );
    assert!(business_permit_queued(core));
    assert_continues_identically_after_checked_reload(
        source,
        "renumbered-permits",
        |source, loaded| {
            assert_ne!(
                source.transit_network.lane_system.lanes.len(),
                loaded.transit_network.lane_system.lanes.len(),
                "the loaded game numbers its lanes compactly"
            );
            assert!(business_permit_queued(loaded));
        },
    );
}

#[test]
fn city_hour_totals_continue_identically_after_a_mid_hour_save_load() {
    let mut source = GrowthScenario::starter_town();
    source
        .core
        .place_service_building_internal("kenney:building.power.coal_power_plant", -700.0, 8.0)
        .unwrap();
    assert!(
        source
            .core
            .set_building_service_funding_override_at(-700.0, 8.0, "electricity", 0.05)
    );
    // The town is full by then: no arrival is still on its way.
    source.run_days(10);
    // Save with the hour's totals part-folded by the phases that already ran (`SIM-02-I`).
    while !(10..=50).contains(&(source.core.time.minute_of_day % 60)) {
        source.core.step_frame(FRAME_DT_S);
    }
    assert!(source.core.households.electricity_hour.demand_kwh > 0.0);
    let path = std::env::temp_dir().join(format!(
        "metrum-city-hour-continuation-{}.sqlite",
        std::process::id()
    ));
    source
        .core
        .save_game_internal(path.to_str().unwrap(), None)
        .unwrap();
    let mut loaded = GrowthScenario::starter_town();
    loaded
        .core
        .load_game_internal(path.to_str().unwrap())
        .unwrap();
    std::fs::remove_file(path).unwrap();
    loaded.core.time.speed_multiplier = source.core.time.speed_multiplier;
    // The census loading rebuilt must not feed the hour's remaining phases: the next hour's
    // census replaces this one.
    for building_idx in 0..loaded.core.allocator.buildings.len() {
        loaded
            .core
            .households
            .job_funding
            .set_activity(building_idx, 0.0);
    }
    let hours = |core: &SimCore| core.time.day_index * 24 + u32::from(core.time.minute_of_day) / 60;
    let target = hours(&source.core) + 3;
    loop {
        assert_city_hour_identical(&source.core, &loaded.core);
        if hours(&source.core) >= target {
            break;
        }
        source.core.step_frame(FRAME_DT_S);
        loaded.core.step_frame(FRAME_DT_S);
    }
    assert!(source.core.households.electricity_hour.dispatch > 0.0);
    assert!(source.core.households.service_sales.gross_revenue > 0.0);
}

// The hourly city totals, the sales pool and the cash and meters they move, bit for bit.
fn assert_city_hour_identical(left: &SimCore, right: &SimCore) {
    let (a, b) = (&left.households, &right.households);
    let hour = |h: &crate::simulation::economy::households::HouseholdSystem| {
        let e = &h.electricity_hour;
        let s = &h.service_sales;
        let mut bits = vec![
            u64::from(e.dispatch.to_bits()),
            u64::from(e.coverage.to_bits()),
            e.demand_kwh.to_bits(),
            e.capacity_kwh.to_bits(),
            e.coal_burned_kg.to_bits(),
            e.coal_stock_kg.to_bits(),
            u64::from(s.residents),
            u64::from(s.gross_revenue.to_bits()),
            u64::from(s.hour_residents),
            s.hour_collected.to_bits(),
            h.coal_inventory_kg.to_bits(),
            h.day_totals.coal_consumed_kg.to_bits(),
        ];
        for values in [
            &s.gross_by_resource,
            &s.capacity_by_resource,
            &s.hour_capacity_by_resource,
        ] {
            bits.extend(values.iter().map(|v| u64::from(v.to_bits())));
        }
        for values in [&s.drawable_by_resource, &s.undrawn_by_resource] {
            bits.extend(values.iter().map(|v| v.to_bits()));
        }
        bits
    };
    assert_eq!(hour(a), hour(b), "minute {}", left.time.minute_of_day);
    assert_eq!(a.households.len(), b.households.len());
    for (id, (x, y)) in a.households.iter().zip(&b.households).enumerate() {
        let (lx, ly) = (&a.daily_ledgers()[id], &b.daily_ledgers()[id]);
        assert_eq!(x.budget.to_bits(), y.budget.to_bits());
        assert_eq!(
            (lx.electricity.consumed_kwh, lx.electricity.local_kwh),
            (ly.electricity.consumed_kwh, ly.electricity.local_kwh)
        );
    }
    let (left, right) = (&left.allocator.buildings, &right.allocator.buildings);
    assert_eq!(left.len(), right.len());
    for (x, y) in left.iter().zip(right) {
        assert_eq!(
            [
                x.operating_budget,
                x.revenue,
                x.daily_power_service_units,
                x.daily_electricity.workplace.local_kwh,
                x.daily_electricity.residential.local_kwh,
                x.commercial_activity_floor_scale,
                x.funded_activity,
                x.service_sales_capacity,
            ]
            .map(f32::to_bits),
            [
                y.operating_budget,
                y.revenue,
                y.daily_power_service_units,
                y.daily_electricity.workplace.local_kwh,
                y.daily_electricity.residential.local_kwh,
                y.commercial_activity_floor_scale,
                y.funded_activity,
                y.service_sales_capacity,
            ]
            .map(f32::to_bits),
            "{}",
            x.asset_id
        );
        assert_eq!(x.resource_inventory, y.resource_inventory);
    }
}

#[test]
#[ignore = "physical mid-day save continuation, with and without power-plant demolition"]
fn electricity_save_continuation_preserves_bills_and_removed_meters() {
    for remove in [false, true] {
        // Place the plant at founding, while housing remains to recruit its worker: the
        // starter town fills its zoned land within days and cannot grow afterwards.
        let mut source = GrowthScenario::starter_town();
        let plant = source
            .core
            .place_service_building_internal("kenney:building.power.coal_power_plant", -700.0, 8.0)
            .unwrap();
        assert!(source.core.set_building_service_funding_override_at(
            -700.0,
            8.0,
            "electricity",
            0.05
        ));
        source.run_days(60);
        while source.core.time.minute_of_day < 720 {
            source.core.step_frame(FRAME_DT_S);
        }
        let plant = source
            .core
            .allocator
            .nearest_building_idx_at(-700.0, 8.0)
            .unwrap_or(plant);
        assert_eq!(source.core.allocator.buildings[plant].worker_count, 1);
        assert!(source.core.allocator.buildings[plant].daily_power_service_units > 0.0);
        if remove {
            let core = &mut source.core;
            assert!(core.allocator.remove_building_for_bulldoze(
                plant,
                &mut core.zoning,
                &mut core.agents,
                &mut core.households,
                &mut core.logistics,
                &mut core.treasury.balance,
            ));
            core.publish_pending_building_site_changes();
        }
        let path = std::env::temp_dir().join(format!(
            "metrum-power-continuation-{}-{remove}.sqlite",
            std::process::id()
        ));
        source
            .core
            .save_game_internal(path.to_str().unwrap(), None)
            .unwrap();
        let mut loaded = GrowthScenario::starter_town();
        loaded
            .core
            .load_game_internal(path.to_str().unwrap())
            .unwrap();
        // Loading intentionally pauses gameplay; resume both worlds at the same speed.
        loaded.core.time.speed_multiplier = source.core.time.speed_multiplier;
        assert_eq!(source.core.treasury.balance, loaded.core.treasury.balance);
        source.run_days(1);
        loaded.run_days(1);
        let a = source.core.budget_history.back().unwrap();
        let b = loaded.core.budget_history.back().unwrap();
        for (name, left, right) in [
            ("generation", a.power_produced, b.power_produced),
            ("local consumption", a.power_consumed, b.power_consumed),
            ("imports", a.power_unmet, b.power_unmet),
            (
                "bills",
                a.electricity_consumption_cost,
                b.electricity_consumption_cost,
            ),
            (
                "fuel purchases",
                a.electricity_fuel_cost,
                b.electricity_fuel_cost,
            ),
            ("coal consumed", a.coal_consumed, b.coal_consumed),
            ("revenue", a.electricity_revenue, b.electricity_revenue),
            ("net cash", a.electricity_net, b.electricity_net),
        ] {
            assert!(
                (left - right).abs() < 0.01,
                "removed={remove} {name}: {left} vs {right}"
            );
        }
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn production_area_construction_charges_only_successful_added_area() {
    for mine in [false, true] {
        let mut scenario = GrowthScenario::starter_town();
        let before = scenario.core.treasury.lifetime_build_cost;
        let hectares = if mine { 1.0 } else { 4.0 };
        let owner = place_producer(&mut scenario, mine, hectares, -900.0);
        let core = &mut scenario.core;
        let building = &core.allocator.buildings[owner];
        let rate = if mine { 25_000.0 } else { 20_000.0 };
        let footprint = f64::from(building.width_cells) * f64::from(building.depth_cells) * 2_500.0;
        assert_eq!(
            core.treasury.lifetime_build_cost - before,
            footprint + rate * f64::from(hectares)
        );
        let private_cash = building.operating_budget;
        let polygon = if mine {
            core.resource_extraction
                .site_for_building(owner)
                .unwrap()
                .polygon_world
                .clone()
        } else {
            core.agriculture
                .site_for_building(owner)
                .unwrap()
                .polygon_world
                .clone()
        };
        let commit = |core: &mut SimCore, polygon| {
            if mine {
                core.commit_extractor_polygon_internal(owner, polygon)
                    .map(|s| s.area_m2)
            } else {
                core.commit_field_polygon_internal(owner, polygon)
                    .map(|s| s.area_m2)
            }
        };
        let before_cash = core.treasury.balance;
        assert_eq!(
            core.industry_area_quote_internal(owner, (hectares + 1.0) * 10_000.0)
                .unwrap()
                .0,
            rate
        );
        assert_eq!(
            core.treasury.balance, before_cash,
            "quotes cannot spend money"
        );
        assert!(commit(core, Vec::new()).is_err());
        commit(core, polygon.clone()).unwrap();
        assert_eq!(
            core.treasury.balance, before_cash,
            "invalid and unchanged areas cost nothing"
        );
        let mut expanded = polygon.clone();
        let depth_added = 10_000.0 / (polygon[1].x - polygon[0].x);
        expanded[2].y += depth_added;
        expanded[3].y += depth_added;
        commit(core, expanded).unwrap();
        assert_eq!(core.treasury.balance, before_cash - rate);
        commit(core, polygon).unwrap();
        assert_eq!(
            core.treasury.balance,
            before_cash - rate,
            "shrinking grants no refund"
        );
        assert_eq!(
            core.allocator.buildings[owner].operating_budget, private_cash,
            "no recapitalization on edits"
        );
    }
}

#[test]
fn hundred_hectare_start_distinguishes_mechanized_farm_from_labor_intensive_mine() {
    for mine in [false, true] {
        let mut scenario = GrowthScenario::starter_town_in_world(4096.0);
        place_producer(&mut scenario, mine, 100.0, -1900.0);
        let report = scenario.run_diagnosed_days(90);
        verify_accounting(&report);
        let profile = if mine {
            "coal_mine_basic"
        } else {
            "grain_farm_basic"
        };
        let all = report.business_cash_flow(profile, usize::MAX);
        if mine {
            assert!(all.operating_cash_change < 0.0, "{report}");
            let firm = report
                .business_outcomes()
                .find(|firm| firm.profile_id == profile)
                .unwrap();
            assert!(
                firm.ending_cash < firm.opening_cash,
                "mine must spend its owner's capital: {firm:?}"
            );
            assert!(
                firm.staffed_worker_hours < firm.capacity_worker_hours,
                "unprofitable full staffing must not be advertised against startup equity: {firm:?}"
            );
            // Withdrawing unfunded jobs can extend the owner's cash runway. Losses and
            // zero subsequent equity, rather than an arbitrary 90-day failure, prove no rescue.
        } else {
            assert!(all.operating_cash_change > 0.0, "{report}");
            assert_eq!(
                all.bankruptcies, 0,
                "six-worker farm must trade sustainably: {report}"
            );
        }
        assert_eq!(
            all.startup_cash, 0.0,
            "no fresh equity after the run begins"
        );
    }
}

#[test]
fn explicit_producers_are_observed_across_growth_and_building_remaps() {
    let mut scenario = GrowthScenario::starter_town();
    place_producer(&mut scenario, false, 4.0, -900.0);
    place_producer(&mut scenario, false, 4.0, -500.0);
    let report = scenario.run_diagnosed_days(30);
    verify_accounting(&report);
    let farms = report.business_cash_flow("grain_farm_basic", usize::MAX);
    assert!(farms.payroll > 0.0 && farms.sales > 0.0, "{report}");
    assert_eq!(
        farms.startup_cash, 0.0,
        "opening cash must not be counted twice"
    );
}

#[test]
fn starter_producers_restock_and_pay_workers_from_sales() {
    for mine in [false, true] {
        let mut scenario = GrowthScenario::starter_town();
        place_producer(&mut scenario, mine, if mine { 1.0 } else { 4.0 }, -900.0);
        let profile = if mine {
            "coal_mine_basic"
        } else {
            "grain_farm_basic"
        };
        let report = scenario.run_diagnosed_days(180);
        verify_accounting(&report);
        let all = report.business_cash_flow(profile, usize::MAX);
        let recent = report.business_cash_flow(profile, 90);
        assert_eq!(all.bankruptcies, 0, "{report}");
        assert_eq!(
            all.startup_cash, 0.0,
            "opening capital is already accounted for"
        );
        assert!(
            recent.operating_cash_change > 0.0 && recent.payroll > 0.0,
            "{report}"
        );
        assert!(
            recent.input_purchases > 0.0,
            "opening machinery must not be the only delivery: {report}"
        );
        assert!(
            (all.sales - all.local_sales - all.export_sales).abs() < 0.1,
            "{report}"
        );
    }
}

#[test]
#[ignore = "manual ECON-14 producer balance matrix; records failures as evidence before tuning"]
fn producer_balance_matrix() {
    let days = std::env::var("METRUM_BALANCE_DAYS").map_or(730, |n| n.parse().unwrap());
    let directory = std::env::var("METRUM_BALANCE_REPORT_DIR")
        .unwrap_or_else(|_| "/tmp/metrum-producer-balance".into());
    std::fs::create_dir_all(&directory).unwrap();
    for (mine, hectares) in [
        (false, 4.0),
        (false, 10.0),
        (false, 20.0),
        (false, 100.0),
        (true, 0.25),
        (true, 1.0),
        (true, 4.0),
        (true, 20.0),
        (true, 100.0),
    ] {
        let large = hectares >= 100.0;
        let mut scenario =
            GrowthScenario::starter_town_in_world(if large { 4096.0 } else { WORLD_SIZE_M });
        let construction_before = scenario.core.treasury.lifetime_build_cost;
        place_producer(
            &mut scenario,
            mine,
            hectares,
            if large { -1900.0 } else { -900.0 },
        );
        let construction_capital = scenario.core.treasury.lifetime_build_cost - construction_before;
        let profile = if mine {
            "coal_mine_basic"
        } else {
            "grain_farm_basic"
        };
        let report = scenario.run_diagnosed_days(days);
        let all = report.business_cash_flow(profile, usize::MAX);
        let recent = report.business_cash_flow(profile, 90);
        let name = format!("{profile}-{hectares}ha");
        std::fs::write(
            Path::new(&directory).join(format!("{name}.txt")),
            format!("One-time city infrastructure endowment for this producer: {construction_capital:.2}; all charged as construction before observation. No later capital injections.\n{}", report.daily_report()),
        )
        .unwrap();
        eprintln!(
            "{name}: {:?}; cash change total {:.2}, trailing 90 days {:.2}; payroll {:.2}, sales local/export {:.2}/{:.2}, purchases {:.2}, bankruptcies {}; city recurring {:.2}; empty pantry-days {}",
            report.days.last().unwrap(),
            all.operating_cash_change,
            recent.operating_cash_change,
            all.payroll,
            all.local_sales,
            all.export_sales,
            all.input_purchases,
            all.bankruptcies,
            report.recurring_city_balance(90),
            report.empty_household_days(90)
        );
        verify_accounting(&report);
    }
}

#[test]
#[ignore = "manual site-level cash and freight diagnosis for the village growth scenario"]
fn village_industrial_cash_trace() {
    use crate::simulation::economy::definitions::load_runtime_economy_catalog;
    use crate::simulation::economy::logistics::ShipmentEndpoint;
    let catalog = load_runtime_economy_catalog().unwrap();
    let mut scenario = GrowthScenario::starter_town();
    for day in 1..=730 {
        scenario.run_days(1);
        if day % 20 != 0 {
            continue;
        }
        eprintln!("SITE DAY {day}: {:?}", scenario.record());
        for (idx, b) in scenario.core.allocator.buildings.iter().enumerate() {
            if b.zone_type != ZoneType::Industrial {
                continue;
            }
            let Some(profile) = catalog.profile_by_runtime_id(b.economy_profile_runtime_id) else {
                continue;
            };
            eprintln!(
                "  site {idx} {} at {:?}: workers {} cash {:.2} profit {:.2} revenue {:.2} deserted {} stock {:?}",
                profile.id,
                (b.center_x, b.center_y),
                b.worker_count,
                b.operating_budget,
                b.last_day_profit,
                b.revenue,
                b.is_deserted,
                b.resource_inventory
            );
            for s in &scenario.core.logistics.shipments {
                if s.source == ShipmentEndpoint::Building(idx)
                    || s.destination == ShipmentEndpoint::Building(idx)
                {
                    eprintln!(
                        "    {:?} -> {:?} {:?}: resource {} amount {} fee {:.2}",
                        s.source,
                        s.destination,
                        s.status,
                        s.resource_runtime_id,
                        s.amount,
                        s.freight_cost
                    );
                }
            }
        }
    }
}

#[test]
#[ignore = "manual ECON-14 trace of a repeatedly failing physical shopping trip"]
fn producer_household_shopping_trace() {
    let mut scenario = GrowthScenario::starter_town_in_world(4096.0);
    place_producer(&mut scenario, false, 100.0, -1900.0);
    scenario.run_days(90);
    let hid = scenario
        .core
        .households
        .households
        .iter()
        .enumerate()
        .max_by_key(|(_, household)| household.replenishment_failure_count)
        .unwrap()
        .0;
    let home = scenario.core.households.households[hid].home_building_id;
    eprintln!("TRACE household {hid} home {home}");
    for (idx, b) in scenario.core.allocator.buildings.iter().enumerate() {
        if idx == home || b.zone_type == ZoneType::Commercial {
            eprintln!(
                "  building {idx} {:?} at ({}, {}) profile {} stock {:?}",
                b.zone_type,
                b.center_x,
                b.center_y,
                b.economy_profile_runtime_id,
                b.resource_inventory
            );
        }
    }
    let end_day = scenario.day() + 2;
    let mut previous = None;
    while scenario.day() < end_day {
        let step = scenario.core.step_frame(FRAME_DT_S);
        let core = &scenario.core;
        let h = &core.households.households[hid];
        let shopper = core
            .agents
            .resolve_agent(h.shopping_agent)
            .unwrap_or(usize::MAX);
        let a = &core.agents;
        let state = (
            h.replenishment_state,
            shopper,
            (shopper < a.len()).then(|| {
                (
                    a.transit[shopper],
                    a.current_building[shopper],
                    a.activity[shopper],
                    a.target_building[shopper],
                    a.planned_target_building[shopper],
                    a.planned_activity[shopper],
                )
            }),
        );
        if previous != Some(state) || step.hourly_ticks > 0 {
            eprintln!(
                "day {} minute {} HH state {} stock {:.2} cash {:.2} store {} amount {:.2} paid {:.2} timeout {} failures {} shopper {} {:?}",
                core.time.day_index,
                core.time.minute_of_day,
                h.replenishment_state,
                h.stock,
                h.budget,
                h.reserved_store_building_id,
                h.reserved_amount,
                h.reserved_total_cost,
                h.shopping_timeout_hours_remaining,
                h.replenishment_failure_count,
                shopper,
                state.2
            );
            for idx in 0..a.len() {
                if a.household_id[idx] == hid {
                    eprintln!(
                        "  member {idx} age {} car {} at ({:.1},{:.1}) transit {} activity {} current {} work {} planned {} target {}",
                        a.age_group[idx],
                        a.has_car[idx],
                        a.pos_x[idx],
                        a.pos_y[idx],
                        a.transit[idx],
                        a.activity[idx],
                        a.current_building[idx],
                        a.work_building[idx],
                        a.planned_target_building[idx],
                        a.target_building[idx]
                    );
                }
            }
            previous = Some(state);
        }
    }
}

#[test]
#[ignore = "manual DEM-02-C probe: residential land outlasts industrial land, as in the player save"]
fn residential_rich_town_housing_balance() {
    // Starter-town streets with residential paint on two full cross streets, but industry on
    // one short frontage only, so housing need ends while residential sites remain.
    let mut core = SimCore::new(WorldConfig::new(WORLD_SIZE_M, WORLD_SIZE_M, 40.0, 10.0));
    core.precompute_road_mesh_data();
    install_bootstrap_assets(&mut core);
    let border_x = BORDER_INSET_M - core.heightmap.half_world_extents().0;
    commit_road(&mut core, (border_x, 0.0), (MAIN_STREET_END_X_M, 0.0));
    for x in CROSS_STREET_X_M {
        commit_road(
            &mut core,
            (x, -CROSS_STREET_HALF_LENGTH_M),
            (x, CROSS_STREET_HALF_LENGTH_M),
        );
    }
    let border_y = core.heightmap.sample_height_world(border_x, 0.0);
    let border = core.check_border_candidate_internal(Vector3::new(border_x, border_y, 0.0));
    core.set_border_connection_internal(border as i32);
    for x in [CROSS_STREET_X_M[0], CROSS_STREET_X_M[1]] {
        paint_cross_street(&mut core, f64::from(x), ZoneType::Residential);
    }
    let east = f64::from(CROSS_STREET_X_M[2]);
    paint(
        &mut core,
        ZoneType::Industrial,
        ZONE_BRUSH,
        &[
            DVec2::new(east, JUNCTION_CLEARANCE_M),
            DVec2::new(east, 90.0),
        ],
    );
    paint(
        &mut core,
        ZoneType::Commercial,
        ZONE_BRUSH,
        &[DVec2::new(30.0, 0.0), DVec2::new(170.0, 0.0)],
    );
    core.time.speed_multiplier = SIMULATION_SPEED_STEPS[SIMULATION_SPEED_STEPS.len() - 1];
    let mut scenario = GrowthScenario {
        core,
        departures: 0,
    };
    let largest = scenario.largest_home_capacity();
    let days: u32 = std::env::var("METRUM_DIAG_DAYS").map_or(60, |n| n.parse().unwrap());
    let report = scenario.run_diagnosed_days(days);
    for record in &report.days {
        eprintln!("RICH {record:?} largest_home={largest}");
    }
    if let Ok(path) = std::env::var("METRUM_DIAG_REPORT") {
        std::fs::write(path, report.daily_report()).unwrap();
    }
}

// A player's bulldoze swap-removes a building and marks every flow field; a save taken before
// the next planning slot routes both games on the same fields and continues identically
// (`SIM-02-I`).
#[test]
fn a_bulldoze_before_the_next_planning_slot_continues_identically_after_a_save_load() {
    let mut source = growing_town_mid_hour();
    let core = &mut source.core;
    let last = core.allocator.buildings.len() - 1;
    let target = (0..last)
        .find(|&index| {
            let building = &core.allocator.buildings[index];
            building.zone_type == ZoneType::Residential && !building.is_under_construction()
        })
        .expect("a completed home that is not the last building");
    assert!(core.bulldoze_building(target));
    assert!(
        (10..=50).contains(&(core.time.minute_of_day % 60)),
        "saved before the next planning slot"
    );
    assert_continues_identically_after_reload(source, "bulldoze");
}

// A removal leaves the zone index for its next reader: the removed building's zone entry names
// the building the swap-remove moved there, and the moved one's is missing. The next frame's
// flow-field drain reads it, so it rebuilds it first and builds every field from the current
// buildings, as a loaded game does (`SIM-02-I`). The entrance cache follows the swap-remove
// (`SIM-02-K`).
#[test]
fn the_flow_field_drain_rebuilds_a_stale_zone_index_first() {
    use crate::simulation::network::types::TransitFlags;
    use crate::simulation::pathing::flow_field::FlowField;
    let mut town = GrowthScenario::starter_town();
    town.run_days(1);
    let core = &mut town.core;
    let last = core.allocator.buildings.len() - 1;
    // A completed building whose zone list the moved last building does not belong to as a
    // completed one, so the stale list names a wrong source.
    let completed_in = |index: usize, zone: ZoneType| {
        let building = &core.allocator.buildings[index];
        building.zone_type == zone && !building.is_under_construction()
    };
    let target = [
        ZoneType::Residential,
        ZoneType::Commercial,
        ZoneType::Industrial,
    ]
    .into_iter()
    .find_map(|zone| {
        (0..last).find(|&index| completed_in(index, zone) && !completed_in(last, zone))
    })
    .expect("a completed building of another zone than the last");
    // The lifecycle removal a phase or the bulldoze tool makes, without the repair after it.
    core.agents.evacuate_building_for_removal(
        target,
        &mut core.households,
        &mut core.allocator,
        &core.transit_network,
        &core.region_graph,
    );
    assert!(core.allocator.remove_building_for_bulldoze(
        target,
        &mut core.zoning,
        &mut core.agents,
        &mut core.households,
        &mut core.logistics,
        &mut core.treasury.balance,
    ));
    assert!(core.allocator.dirty_index && !core.allocator.entrances_dirty);
    // A frame too short to reach the next slot: only the drain runs.
    let slot = core.time.absolute_slot();
    core.step_frame(FRAME_DT_S / 1000.0);
    assert_eq!(core.time.absolute_slot(), slot);
    assert!(!core.allocator.dirty_index && !core.allocator.entrances_dirty);
    let graph = &core.region_graph;
    for zone in [
        ZoneType::Residential,
        ZoneType::Commercial,
        ZoneType::Industrial,
    ] {
        let fields = &core.transit_network.flow_fields;
        for (flags, field) in [
            (TransitFlags::CAR, fields.car(zone)),
            (TransitFlags::FOOT, fields.foot(zone)),
        ] {
            let mut sources = Vec::new();
            core.allocator
                .collect_sources_for_zone(zone, graph, flags, &mut sources);
            assert!(
                field.cloned()
                    == (!sources.is_empty()).then(|| FlowField::build(&sources, graph, flags)),
                "{zone:?} {flags} field from the stale index"
            );
        }
    }
}

// A starter town at the first hour whose commercial slot planned a store, just after that slot,
// with a power plant placed then: a service placed after the publication withdraws the
// published books (`SIM-02-I`).
fn town_with_a_service_placed_after_the_commercial_slot() -> GrowthScenario {
    use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
    let mut source = GrowthScenario::starter_town();
    loop {
        assert!(
            source.day() < 4,
            "no hour planned a store at the commercial slot"
        );
        let hour = source.core.time.absolute_slot() / SLOTS_PER_HOUR + 1;
        step_to_slot(
            &mut source.core,
            hour * SLOTS_PER_HOUR + PlanningSlot::Commercial as u64,
        );
        if !source
            .core
            .demand
            .building_actions
            .commercial
            .spawns
            .is_empty()
        {
            break;
        }
    }
    assert!(super::census_cases::published_books_lent(&source.core));
    source
        .core
        .place_service_building_internal("kenney:building.power.coal_power_plant", -700.0, 8.0)
        .unwrap();
    assert!(!super::census_cases::published_books_lent(&source.core));
    source
}

// A save in the hour's planning slots on books not valid for the live city (`SIM-02-I`): the
// save keeps that they were not valid, so in both games the industrial and actions slots skip,
// the actions slot queues the store the commercial slot planned with its permit void until the
// next hour's epoch, and the loaded game continues every frame as the saved one does.
#[test]
fn a_save_in_the_planning_slots_on_invalid_books_skips_the_hour_identically() {
    use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
    // The uninterrupted game: the actions slot queues the store without a permit.
    let mut town = town_with_a_service_placed_after_the_commercial_slot();
    let hour = town.core.time.absolute_slot() / SLOTS_PER_HOUR;
    step_to_slot(
        &mut town.core,
        hour * SLOTS_PER_HOUR + PlanningSlot::Actions as u64,
    );
    assert!(!super::census_cases::published_books_lent(&town.core));
    let stores: Vec<_> = town
        .core
        .pending_demand_spawns
        .iter()
        .filter(|pending| pending.zone_type == ZoneType::Commercial)
        .collect();
    assert!(
        !stores.is_empty(),
        "the actions slot queues the planned store"
    );
    assert!(
        stores
            .iter()
            .all(|pending| pending.permit.granted().is_none()),
        "the skipped epoch grants no business permit"
    );
    // Saved after the commercial slot, the loaded game skips and voids alike.
    assert_continues_identically_after_checked_reload(
        town_with_a_service_placed_after_the_commercial_slot(),
        "invalid-books-commercial",
        super::census_cases::assert_same_published_census,
    );
}
