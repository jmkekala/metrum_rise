// SPDX-License-Identifier: GPL-2.0-only

//! Headless one-year growth runs of the starter town (`ECON-12`).
//!
//! `cargo test --test growth_scenario -- --ignored` also runs the `ECON-14` growth target and
//! reports the town's accounting and first failures. Set `METRUM_GROWTH_REPORT` to a text-file
//! path to save every daily observation. See `docs/economy.md#econ-12-growth-scenario`.

use metrum_rise::nodes::sim::core::{GrowthDayRecord, GrowthReport, GrowthScenario};

const DAYS_PER_YEAR: u32 = 365;

fn run_year() -> GrowthReport {
    GrowthScenario::starter_town().run_diagnosed_days(DAYS_PER_YEAR)
}

fn monthly_trajectory(year: &[GrowthDayRecord]) -> String {
    year.iter()
        .step_by(30)
        .chain(year.last())
        .map(|r| {
            format!(
                "day {:>3}: households {:>3}  residents {:>3}  employed {:>3}/{:>3} jobs  \
                 buildings {:>3}  treasury {:>9.0}",
                r.day, r.households, r.residents, r.employed, r.jobs, r.buildings, r.treasury
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn concurrent_runs_record_identical_years() {
    // Concurrent runs share the Rayon pool, so any dependence on scheduling diverges here.
    let (first, second) = rayon::join(run_year, run_year);
    assert_eq!(first.days.len(), DAYS_PER_YEAR as usize);
    assert!(
        first.days.iter().any(|day| day.households > 0),
        "the town never admitted a household:\n{}",
        monthly_trajectory(&first.days)
    );
    if let Some(day) = first
        .days
        .iter()
        .zip(&second.days)
        .position(|(a, b)| a != b)
    {
        panic!(
            "runs diverge on day {}: {:?} vs {:?}",
            first.days[day].day, first.days[day], second.days[day]
        );
    }
    assert!(first.has_same_accounting(&second), "accounting diverged");
    assert!(
        first.max_city_cash_residual() < 0.1,
        "city flows do not reconcile:\n{first}"
    );
    assert!(
        first.max_business_cash_residual() < 0.1,
        "business flows do not reconcile:\n{first}"
    );
    assert!(
        first.max_household_cash_residual() < 0.1,
        "household flows do not reconcile:\n{first}"
    );
    if let Ok(path) = std::env::var("METRUM_GROWTH_REPORT") {
        std::fs::write(path, first.daily_report()).expect("write requested village report");
    }
}

#[test]
fn read_only_observer_preserves_growth() {
    let (plain, diagnosed) = rayon::join(
        || GrowthScenario::starter_town().run_days(DAYS_PER_YEAR),
        || {
            let mut scenario = GrowthScenario::starter_town();
            let mut days = scenario.run_days(40);
            // Attaching mid-run must not count old infrastructure or business capital again.
            let report = scenario.run_diagnosed_days(DAYS_PER_YEAR - 40);
            assert!(report.max_city_cash_residual() < 0.1, "{report}");
            assert!(report.max_business_cash_residual() < 0.1, "{report}");
            days.extend(report.days);
            days
        },
    );
    assert_eq!(plain, diagnosed);
}

#[test]
fn starter_grocery_pays_its_way_after_startup_through_two_years() {
    let report = GrowthScenario::starter_town().run_diagnosed_days(2 * DAYS_PER_YEAR);
    let all = report.business_cash_flow("grocery_basic", usize::MAX);
    let last_year = report.business_cash_flow("grocery_basic", DAYS_PER_YEAR as usize);
    if let Ok(path) = std::env::var("METRUM_GROWTH_LONG_REPORT") {
        std::fs::write(path, report.daily_report()).expect("write requested two-year report");
    }
    assert_eq!(all.bankruptcies, 0, "{report}");
    assert_eq!(last_year.startup_cash, 0.0, "{report}");
    assert!(
        last_year.payroll > 0.0 && last_year.sales > last_year.payroll,
        "{report}"
    );
    assert!(last_year.operating_cash_change > 0.0, "{report}");
    assert_eq!(report.empty_household_days(DAYS_PER_YEAR as usize), 0, "{report}");
}

#[test]
#[ignore = "ECON-14: healthy growth and solvency remain an unmet target"]
fn starter_town_keeps_growing_and_solvent_through_year_one() {
    let year = run_year();
    let half = &year.days[year.days.len() / 2];
    let end = year.days.last().unwrap();
    assert!(
        end.households > half.households && year.days.iter().all(|day| day.treasury > 0.0),
        "growth stalled or treasury ran out:\n{year}"
    );
}
