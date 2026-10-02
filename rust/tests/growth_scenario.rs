// SPDX-License-Identifier: GPL-2.0-only

//! Headless one-year growth runs of the starter town (`ECON-12`).
//!
//! `cargo test --test growth_scenario -- --ignored` also runs the `ECON-14` growth target and
//! prints the town's monthly trajectory. See `docs/economy.md#econ-12-growth-scenario`.

use metrum_rise::nodes::sim::core::{GrowthDayRecord, GrowthScenario};

const DAYS_PER_YEAR: u32 = 365;

fn run_year() -> Vec<GrowthDayRecord> {
    GrowthScenario::starter_town().run_days(DAYS_PER_YEAR)
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
    assert_eq!(first.len(), DAYS_PER_YEAR as usize);
    assert!(
        first.iter().any(|day| day.households > 0),
        "the town never admitted a household:\n{}",
        monthly_trajectory(&first)
    );
    if let Some(day) = first.iter().zip(&second).position(|(a, b)| a != b) {
        panic!(
            "runs diverge on day {}: {:?} vs {:?}",
            first[day].day, first[day], second[day]
        );
    }
}

#[test]
#[ignore = "ECON-14: the starter town stalls at 25 households and its treasury runs out in year one"]
fn starter_town_keeps_growing_and_solvent_through_year_one() {
    let year = run_year();
    let half = &year[year.len() / 2];
    let end = year.last().unwrap();
    assert!(
        end.households > half.households && year.iter().all(|day| day.treasury > 0.0),
        "growth stalled or treasury ran out:\n{}",
        monthly_trajectory(&year)
    );
}
