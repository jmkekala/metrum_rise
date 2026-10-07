// SPDX-License-Identifier: GPL-2.0-only

//! Headless growth, recurring business income and municipal funding checks (`ECON-12`, `ECON-14`).
//!
//! Set `METRUM_GROWTH_REPORT` and `METRUM_GROWTH_LONG_REPORT` to text-file paths for the
//! one- and two-year daily accounting. See `docs/economy.md#econ-12-growth-scenario`.

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
fn starter_town_funds_industry_services_households_and_city_for_two_years() {
    let report = GrowthScenario::starter_town().run_diagnosed_days(2 * DAYS_PER_YEAR);
    let year_days = DAYS_PER_YEAR as usize;
    if let Ok(path) = std::env::var("METRUM_GROWTH_LONG_REPORT") {
        std::fs::write(path, report.daily_report()).expect("write requested two-year report");
    }
    for profile in [
        "grocery_basic",
        "food_processor_basic",
        "machinery_factory_basic",
        "personal_service_small",
        "health_essentials_small",
    ] {
        let all = report.business_cash_flow(profile, usize::MAX);
        let last_year = report.business_cash_flow(profile, year_days);
        let last_quarter = report.business_cash_flow(profile, 90);
        assert_eq!(all.bankruptcies, 0, "{profile}: {report}");
        assert_eq!(all.replacement_startup_cash, 0.0, "{profile}: {report}");
        // New sites may receive declared owner equity while the city grows. Operating
        // cash excludes that equity; replacement capital and bankruptcies remain failures.
        assert!(
            last_year.payroll > 0.0 && last_year.sales > last_year.payroll,
            "{profile}: {report}"
        );
        assert!(last_year.operating_cash_change > 0.0, "{profile}: {report}");
        assert!(
            last_quarter.operating_cash_change > 0.0,
            "{profile}: {report}"
        );
        if matches!(profile, "food_processor_basic" | "machinery_factory_basic") {
            assert!(
                last_year.export_sales > 0.0 && last_year.input_purchases > 0.0,
                "{profile}: {report}"
            );
            assert!(
                (last_year.sales - last_year.local_sales - last_year.export_sales).abs() < 0.1,
                "industrial receipts must come from delivered sales: {profile}: {report}"
            );
        }
    }
    assert_eq!(report.empty_household_days(year_days), 0, "{report}");
    assert_eq!(
        report.unpaid_household_transfers(usize::MAX),
        0.0,
        "{report}"
    );
    let last_year_balance = report.recurring_city_balance(year_days);
    assert!(last_year_balance > 0.0, "{report}");
    assert!(
        report.recurring_city_balance(usize::MAX) - last_year_balance > 0.0,
        "{report}"
    );
    assert!(report.recurring_city_balance(90) > 0.0, "{report}");
    assert!(report.days.iter().all(|day| day.treasury > 0.0), "{report}");
    let established = &report.days[year_days - 1];
    let end = report.days.last().unwrap();
    // Paid employment must sustain the established population while export-led growth continues.
    assert!(
        established.residents > report.days[0].residents && established.employed > 0,
        "{report}"
    );
    assert!(
        end.residents >= established.residents && end.employed >= established.employed,
        "{report}"
    );
}
