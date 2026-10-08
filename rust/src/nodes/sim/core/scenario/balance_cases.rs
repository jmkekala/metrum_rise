// SPDX-License-Identifier: GPL-2.0-only

//! Opt-in economic experiments using ordinary placement, production, freight and settlement.

use super::*;
use godot::prelude::Vector2;

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
    let mut core = SimCore::new(WorldConfig::new(WORLD_SIZE_M, WORLD_SIZE_M, 40.0, 10.0));
    let scan = scan_pack_dir(Path::new(&mods));
    assert!(!scan.packs.is_empty(), "{:?}", scan.warnings);
    core.replace_asset_registry(scan.packs.into_iter().flat_map(|pack| {
        let pack_id = pack.pack.pack_id;
        pack.assets
            .into_iter()
            .map(move |(manifest, dir)| (pack_id.clone(), manifest, dir))
    }));
    core.load_game_internal(&source).unwrap();
    assert!(
        core.allocator
            .buildings
            .iter()
            .all(|b| !b.broken && !b.economy_broken)
    );
    // Resume the loaded world without changing balances, zoning, staffing or demand rules.
    core.time.speed_multiplier = *SIMULATION_SPEED_STEPS.last().unwrap();
    let mut scenario = GrowthScenario { core };
    let initial = scenario.record();
    let report = scenario.run_diagnosed_days(days);
    std::fs::write(
        destination,
        format!("Initial: {initial:?}\n{}", report.daily_report()),
    )
    .unwrap();
    eprintln!("Saved-city replay: {initial:?} -> {:?}", scenario.record());
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
            .any(|lane| lane.frontage_delay_penalty_s > 0.0)
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
            assert_eq!(
                a.construction_remaining_hours,
                b.construction_remaining_hours
            );
        }
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn many_new_industrial_lots_respect_scarce_labor_and_earn_their_owner_capital() {
    use crate::simulation::buildings::allocator::BuildingSiteEnvironment;
    use crate::simulation::economy::agents::age_group_can_work;
    use crate::simulation::economy::definitions::load_runtime_economy_catalog;
    use crate::simulation::network::surface::RoadSurfaceCompileReason;

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
    core.transit_network.road_surface.compile_dirty_with_reason(
        &core.region_graph,
        &core.heightmap,
        RoadSurfaceCompileReason::SimCommit,
    );
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
            road_surface: &core.transit_network.road_surface,
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
    // Planning promises one adult per future home. After arrival, actual second adults
    // legitimately permit more projects; the long-run equity bound uses observed workers.
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
    let asset = if mine {
        "kenney:building.coal.mine"
    } else {
        "kenney:building.industrial.small_farm"
    };
    let owner = core
        .place_industry_building_internal(asset, x, 8.0)
        .expect("explicit producer placement");
    let back = core.allocator.building_sites[owner]
        .lot_footprint_world
        .iter()
        .map(|point| point.y)
        .fold(f32::NEG_INFINITY, f32::max);
    // Large-site experiments use a larger world, keeping the field west of the same town.
    let width = (hectares * 10_000.0).sqrt();
    let depth = hectares * 10_000.0 / width;
    let origin = Vector2::new(x - 10.0, back + 1.0);
    let polygon = vec![
        origin,
        origin + Vector2::new(width, 0.0),
        origin + Vector2::new(width, depth),
        origin + Vector2::new(0.0, depth),
    ];
    if mine {
        assert!(core.paint_world_coal_deposit_internal(
            origin + Vector2::new(width, depth) * 0.5,
            Vector2::new(width, depth).length() * 0.5 + 20.0,
            100.0,
        ));
        core.commit_extractor_polygon_internal(owner, polygon)
            .expect("coal extraction area");
    } else {
        core.commit_field_polygon_internal(owner, polygon)
            .expect("grain field");
    }
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
            // Waiting until every home is occupied leaves no labor for a newly placed plant.
            scenario.run_days(365);
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
        assert_eq!(core.households.job_funding.service_capacities[plant], (20.0 * fraction).ceil() as u32);
        assert_eq!(core.households.job_funding.capacities.get(plant).copied().unwrap_or(0), 0);
        assert!(core.push_building_removal_undo(plant));
        let before = core.treasury.balance;
        assert!(core.allocator.remove_building_for_bulldoze(
            plant, &mut core.zoning, &mut core.agents, &mut core.households,
            &mut core.logistics, &mut core.treasury.balance,
        ));
        core.publish_pending_building_site_changes();
        core.seal_building_removal_undo(core.treasury.balance - before);
        assert!(core.set_service_funding("electricity", 1.0 - fraction));
        assert!(core.undo_action_internal());
        assert_eq!(core.households.job_funding.service_capacities[plant], (20.0 * (1.0 - fraction)).ceil() as u32,
            "undo uses current fiscal policy before the next hourly pass");
        assert_eq!(core.households.job_funding.capacities.get(plant).copied().unwrap_or(0), 0);
    }
}

#[test]
fn power_demolition_undo_restores_the_meter_and_refunds_only_its_bill() {
    let mut scenario = GrowthScenario::starter_town();
    let core = &mut scenario.core;
    let plant = core
        .place_service_building_internal("kenney:building.power.coal_power_plant", -700.0, 8.0)
        .unwrap();
    core.allocator.buildings[plant].construction_remaining_hours = 0;
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
#[ignore = "physical mid-day save continuation, with and without power-plant demolition"]
fn electricity_save_continuation_preserves_bills_and_removed_meters() {
    for remove in [false, true] {
        let mut source = GrowthScenario::starter_town();
        source.run_days(180);
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
        let shopper = h.shopping_agent_id;
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
