// SPDX-License-Identifier: GPL-2.0-only

//! Opt-in economic experiments using ordinary placement, production, freight and settlement.

use super::*;
use godot::prelude::Vector2;

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
    // no money is added after simulation starts or to a running business.
    core.treasury.balance += core.treasury.lifetime_build_cost - construction_before;
    owner
}

fn verify_accounting(report: &GrowthReport) {
    assert!(report.max_city_cash_residual() < 0.1, "{report}");
    assert!(report.max_business_cash_residual() < 0.1, "{report}");
    assert!(report.max_household_cash_residual() < 0.1, "{report}");
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
            assert_eq!(
                all.bankruptcies, 1,
                "oversized mine must not be rescued: {report}"
            );
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
