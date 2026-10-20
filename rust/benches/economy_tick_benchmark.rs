// SPDX-License-Identifier: GPL-2.0-only

//! Hour-start and day-start economy cost of fully housed cities (`ECON-13`).
//!
//! Each tier builds its city once, outside the timed region, and keeps advancing it frame by
//! frame on the slot clock, an hour at a time through the hour's actions slot. Only the systems
//! under test are timed; every other system still runs at its slots. The hourly benchmark
//! samples the fixed-slot systems at the start of each hour without the calendar close (census
//! publication, the rest of the operational hour and the four planning slots); the daily
//! benchmark samples the calendar close of ten consecutive days. Settlement itself runs at each
//! entity's daily phase in every slot (`SIM-02-G`). The city cannot drift far from its built
//! state during a run. Criterion warns that the target time is too short; that is intended.
//! Only the 10k tier runs by default. The 100k and 1M builds take minutes in release and hours
//! in a debug `cargo test --all-targets`, so they run only on request:
//! `ECONOMY_TICK_TIERS=10k,100k,1m cargo bench --bench economy_tick_benchmark`.

use criterion::{BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main};
use metrum_rise::nodes::sim::core::PopulatedCity;
use std::time::{Duration, Instant};

const TIERS: [(&str, u32); 3] = [("10k", 10_000), ("100k", 100_000), ("1m", 1_000_000)];
const DEFAULT_TIERS: &str = "10k";
const DEMAND_PASS_HOURS_PER_DAY: usize = 23;
const SETTLEMENT_DAYS: usize = 10;

fn build(label: &str, residents: u32) -> PopulatedCity {
    let started = Instant::now();
    let city = PopulatedCity::build(residents);
    eprintln!(
        "economy_tick {label}: built in {:.1}s, {:?}",
        started.elapsed().as_secs_f64(),
        city.record()
    );
    city
}

// Tiers named in `ECONOMY_TICK_TIERS` (comma-separated labels), in that order.
fn selected_tiers() -> Vec<(&'static str, u32)> {
    std::env::var("ECONOMY_TICK_TIERS")
        .unwrap_or_else(|_| DEFAULT_TIERS.to_owned())
        .split(',')
        .map(|label| {
            let label = label.trim();
            *TIERS
                .iter()
                .find(|(name, _)| *name == label)
                .unwrap_or_else(|| panic!("unknown tier {label:?}; expected 10k, 100k or 1m"))
        })
        .collect()
}

fn bench_economy_ticks(c: &mut Criterion) {
    metrum_rise::debug::init();
    let mut group = c.benchmark_group("economy_tick");
    group
        .sampling_mode(SamplingMode::Flat)
        .warm_up_time(Duration::from_nanos(1))
        .measurement_time(Duration::from_nanos(1));
    for (label, residents) in selected_tiers() {
        let mut city = None;
        group.sample_size(DEMAND_PASS_HOURS_PER_DAY);
        group.bench_function(BenchmarkId::new("operational_hour", label), |b| {
            let city = city.get_or_insert_with(|| build(label, residents));
            b.iter_custom(|iters| {
                let mut timed = Duration::ZERO;
                let mut done = 0;
                while done < iters {
                    let hour = city.advance_hour_through_actions();
                    if hour.demand_pass {
                        timed += hour.hourly;
                        done += 1;
                    }
                }
                timed
            });
        });
        group.sample_size(SETTLEMENT_DAYS);
        group.bench_function(BenchmarkId::new("daily_settlement", label), |b| {
            let city = city.get_or_insert_with(|| build(label, residents));
            b.iter_custom(|iters| {
                let mut timed = Duration::ZERO;
                let mut done = 0;
                while done < iters {
                    if let Some(daily) = city.advance_hour_through_actions().daily {
                        timed += daily;
                        done += 1;
                    }
                }
                timed
            });
        });
        if let Some(city) = &city {
            eprintln!("economy_tick {label}: after runs {:?}", city.record());
        }
    }
    group.finish();
}

criterion_group!(benches, bench_economy_ticks);
criterion_main!(benches);
