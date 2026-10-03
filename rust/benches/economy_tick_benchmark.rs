// SPDX-License-Identifier: GPL-2.0-only

//! Hourly and daily economy tick cost of fully housed cities (`ECON-13`).
//!
//! Each tier builds its city once, outside the timed region, and keeps advancing it hour by
//! hour. Only the tick under test is timed; the other ticks still run so the city keeps its real
//! cadence. Every sample is one tick: the hourly benchmark samples each demand-pass hour of one
//! day and the daily benchmark ten consecutive days, so the city cannot drift far from its built
//! state during a run. Criterion warns that the target time is too short; that is intended.
//! Select tiers with a filter, e.g. `cargo bench --bench economy_tick_benchmark -- /10k`.

use criterion::{BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main};
use metrum_rise::nodes::sim::core::PopulatedCity;
use std::time::{Duration, Instant};

const TIERS: [(&str, u32); 3] = [("10k", 10_000), ("100k", 100_000), ("1m", 1_000_000)];
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

fn bench_economy_ticks(c: &mut Criterion) {
    let mut group = c.benchmark_group("economy_tick");
    group
        .sampling_mode(SamplingMode::Flat)
        .warm_up_time(Duration::from_nanos(1))
        .measurement_time(Duration::from_nanos(1));
    for (label, residents) in TIERS {
        let mut city = None;
        group.sample_size(DEMAND_PASS_HOURS_PER_DAY);
        group.bench_function(BenchmarkId::new("operational_hour", label), |b| {
            let city = city.get_or_insert_with(|| build(label, residents));
            b.iter_custom(|iters| {
                let mut timed = Duration::ZERO;
                let mut done = 0;
                while done < iters {
                    let hour = city.advance_hour();
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
                    if let Some(daily) = city.advance_hour().daily {
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
