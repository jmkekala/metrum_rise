// SPDX-License-Identifier: GPL-2.0-only

//! Build and tick cost of the `ECON-13` benchmark city across tiers, projected to 1M residents.
//!
//! Each phase's growth exponent comes from the two largest tiers, so a phase that scales worse
//! than linearly shows before a multi-hour run is attempted. Run alone, release, nothing else
//! running: `cargo test --release --test populated_city_scaling -- --ignored --nocapture`.
//! `POPULATED_CITY_TIERS=10000,30000,100000` (the default) selects the tiers. See
//! `docs/economy.md#econ-13-economy-tick-baseline`.

use metrum_rise::nodes::sim::core::{PopulatedCity, PopulatedCityBuildTimes};
use std::time::Instant;

const TARGET_RESIDENTS: u32 = 1_000_000;
const DEFAULT_TIERS: &str = "10000,30000,100000";
// `economy_tick_benchmark` with one-nanosecond warm-up and measurement times runs one warm-up
// iteration and one per sample: 24 demand-pass hours (25 hours with the midnight they cross),
// then 11 days of hours ending in a settlement. Settlement cost varies by day, so each tier
// runs this whole sequence.
const BENCH_HOURS: usize = 25 + 11 * 24;

struct Tier {
    residents: u32,
    build: PopulatedCityBuildTimes,
    build_s: f64,
    hour_s: f64,
    settlement_s: f64,
    bench_ticks_s: f64,
    rss_mb: f64,
    peak_rss_mb: f64,
}

#[test]
#[ignore = "multi-minute scaling measurement; run alone with --release --ignored --nocapture"]
fn populated_city_scaling() {
    let tiers: Vec<u32> = std::env::var("POPULATED_CITY_TIERS")
        .unwrap_or_else(|_| DEFAULT_TIERS.to_owned())
        .split(',')
        .map(|tier| tier.trim().parse().expect("tier is a resident count"))
        .collect();
    assert!(tiers.len() >= 2, "projection needs at least two tiers");
    let measured: Vec<Tier> = tiers.iter().map(|&residents| measure(residents)).collect();
    let [small, large] = [&measured[measured.len() - 2], &measured[measured.len() - 1]];
    println!(
        "POPULATED_CITY_PROJECTION 1M from {}k and {}k, exponent from those two tiers:",
        small.residents / 1000,
        large.residents / 1000
    );
    let mut build_total = 0.0;
    for (name, field) in PHASES {
        let projected = project(name, field(&small.build), field(&large.build), small, large);
        build_total += projected;
    }
    project("mean hour", small.hour_s, large.hour_s, small, large);
    project(
        "mean settlement",
        small.settlement_s,
        large.settlement_s,
        small,
        large,
    );
    let bench = project(
        "benchmark ticks",
        small.bench_ticks_s,
        large.bench_ticks_s,
        small,
        large,
    );
    project(
        "RSS after build (MB)",
        small.rss_mb,
        large.rss_mb,
        small,
        large,
    );
    project(
        "peak RSS (MB)",
        small.peak_rss_mb,
        large.peak_rss_mb,
        small,
        large,
    );
    println!(
        "  build {:.0} min, benchmark ticks {:.0} min, total {:.1} h",
        build_total / 60.0,
        bench / 60.0,
        (build_total + bench) / 3600.0
    );
}

type Phase = (&'static str, fn(&PopulatedCityBuildTimes) -> f64);

const PHASES: [Phase; 5] = [
    ("street grid", |t| t.street_grid.as_secs_f64()),
    ("zoning", |t| t.zoning.as_secs_f64()),
    ("lots", |t| t.lots.as_secs_f64()),
    ("admission", |t| t.admission.as_secs_f64()),
    ("warm-up", |t| t.warm_up.as_secs_f64()),
];

// Builds one tier, then runs the benchmark's tick sequence, timing only the ticks as it does.
fn measure(residents: u32) -> Tier {
    reset_peak_rss();
    let started = Instant::now();
    let (mut city, build) = PopulatedCity::build_timed(residents);
    let build_s = started.elapsed().as_secs_f64();
    let rss_mb = status_mb("VmRSS:");
    // Admission peaks well above the RSS left after the build (`ECON-16`).
    let peak_rss_mb = status_mb("VmHWM:");
    let (mut hours, mut hour_s, mut settlements, mut settlement_s) = (0, 0.0, 0, 0.0);
    for _ in 0..BENCH_HOURS {
        let times = city.advance_hour();
        hours += 1;
        hour_s += times.hourly.as_secs_f64();
        if let Some(daily) = times.daily {
            settlements += 1;
            settlement_s += daily.as_secs_f64();
        }
    }
    let tier = Tier {
        residents,
        build,
        build_s,
        hour_s: hour_s / f64::from(hours),
        settlement_s: settlement_s / f64::from(settlements.max(1)),
        bench_ticks_s: hour_s + settlement_s,
        rss_mb,
        peak_rss_mb,
    };
    println!(
        "POPULATED_CITY_TIER residents={} build_s={:.1} street_grid_s={:.1} zoning_s={:.1} \
         lots_s={:.1} admission_s={:.1} warm_up_s={:.1} hour_ms={:.1} settlement_ms={:.0} \
         bench_ticks_s={:.1} rss_mb={:.0} peak_rss_mb={:.0} {:?}",
        tier.residents,
        tier.build_s,
        build.street_grid.as_secs_f64(),
        build.zoning.as_secs_f64(),
        build.lots.as_secs_f64(),
        build.admission.as_secs_f64(),
        build.warm_up.as_secs_f64(),
        tier.hour_s * 1000.0,
        tier.settlement_s * 1000.0,
        tier.bench_ticks_s,
        tier.rss_mb,
        tier.peak_rss_mb,
        city.record()
    );
    tier
}

// Projects `large` to the target with the growth exponent between the two tiers, prints it.
fn project(name: &str, small: f64, large: f64, small_tier: &Tier, large_tier: &Tier) -> f64 {
    let ratio = f64::from(large_tier.residents) / f64::from(small_tier.residents);
    let exponent = (large / small.max(f64::MIN_POSITIVE)).ln() / ratio.ln();
    let projected =
        large * (f64::from(TARGET_RESIDENTS) / f64::from(large_tier.residents)).powf(exponent);
    println!("  {name:<22} {large:>10.2} -> {projected:>10.1}  (exponent {exponent:.2})");
    projected
}

// Starts a new peak for `VmHWM`, so each tier reports its own build peak rather than the largest
// earlier one. Linux only; elsewhere the peak stays process-wide.
fn reset_peak_rss() {
    let _ = std::fs::write("/proc/self/clear_refs", "5");
}

// A `kB` field of `/proc/self/status` in MB, or 0 where it is unavailable.
fn status_mb(field: &str) -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find_map(|line| line.strip_prefix(field))
                .and_then(|kb| kb.trim().trim_end_matches("kB").trim().parse::<f64>().ok())
        })
        .map_or(0.0, |kb| kb / 1024.0)
}
