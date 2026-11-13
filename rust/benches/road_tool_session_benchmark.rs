// SPDX-License-Identifier: GPL-2.0-only

//! Road tool session benchmark (`ROAD-112` round B): one pointer move of the session (pick,
//! snap, shape, measure; no preview) against the control, today's `get_road_tool_cursor_pos`
//! logic for the same ray, on the fixture of `nodes::sim::road_tool::bench` at the 30k
//! background.
//!
//! Modes besides Criterion, each exiting before it starts:
//! - `METRUM_ROAD_TOOL_LOCALITY=0,1000,10000,30000`: per tier, builds the fixture (outside
//!   timing), warms one trail, then times every move of `METRUM_ROAD_TOOL_ROUNDS` (default 20)
//!   trails one by one for the control, the straight session and the simple-curve session,
//!   printing one `ROAD_TOOL_LOCALITY` JSON line per tier with p50/p99/max in microseconds and the
//!   trail's frame digest; panics when a tier's digest differs from the first tier's.
//! - `METRUM_ROAD_TOOL_ALLOC_PROBE=<n>`: builds the tier `METRUM_ROAD_TOOL_TIER` (default 30000),
//!   warms both session modes over one trail, runs `n` more trails of each and exits. Run under
//!   `heaptrack --record-only` with `n = 0` and `n = 20`: equal allocation counts mean a warmed
//!   move allocates nothing.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use metrum_rise::nodes::sim::road_tool::bench::RoadToolBench;
use std::time::Instant;

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
}

// Every move of `rounds` trails timed one by one, sorted, in microseconds.
fn per_move_us(
    bench: &mut RoadToolBench,
    rounds: usize,
    mut step: impl FnMut(&mut RoadToolBench, usize) -> f32,
) -> Vec<f64> {
    let moves = bench.moves();
    for index in 0..moves {
        black_box(step(bench, index));
    }
    let mut times = Vec::with_capacity(rounds * moves);
    for _ in 0..rounds {
        for index in 0..moves {
            let started = Instant::now();
            black_box(step(bench, index));
            times.push(started.elapsed().as_secs_f64() * 1e6);
        }
    }
    times.sort_by(f64::total_cmp);
    times
}

fn quantiles(times: &[f64]) -> serde_json::Value {
    let at = |q: f64| times[((times.len() - 1) as f64 * q).round() as usize];
    serde_json::json!({"p50": at(0.5), "p99": at(0.99), "max": at(1.0)})
}

// The digest of every frame along one trail, folded in order.
fn trail_digest(bench: &mut RoadToolBench) -> u64 {
    (0..bench.moves()).fold(0, |digest, index| {
        bench.session_move(index);
        digest.rotate_left(5) ^ bench.frame_digest()
    })
}

fn locality(tiers: &str) -> ! {
    let rounds = env_usize("METRUM_ROAD_TOOL_ROUNDS").unwrap_or(20);
    let mut reference = None;
    for tier in tiers
        .split(',')
        .map(|tier| tier.trim().parse::<usize>().expect("tier"))
    {
        let started = Instant::now();
        let mut bench = RoadToolBench::build(tier);
        let setup_s = started.elapsed().as_secs_f64();
        let control = per_move_us(&mut bench, rounds, RoadToolBench::cursor_move);
        bench.restart(true);
        let straight_digest = trail_digest(&mut bench);
        let straight = per_move_us(&mut bench, rounds, RoadToolBench::session_move);
        bench.restart(false);
        let curve_digest = trail_digest(&mut bench);
        let curve = per_move_us(&mut bench, rounds, RoadToolBench::session_move);
        let digests = (straight_digest, curve_digest);
        eprintln!(
            "ROAD_TOOL_LOCALITY {}",
            serde_json::json!({
                "background": tier, "edges": bench.edges(), "setup_s": setup_s,
                "moves": control.len(), "control_us": quantiles(&control),
                "straight_us": quantiles(&straight), "curve_us": quantiles(&curve),
                "digests": [format!("{:016x}", digests.0), format!("{:016x}", digests.1)],
            })
        );
        match reference {
            None => reference = Some(digests),
            Some(reference) => assert_eq!(
                digests, reference,
                "a background of {tier} changed the session's frames"
            ),
        }
    }
    std::process::exit(0)
}

fn allocation_probe(trails: usize) -> ! {
    let mut bench = RoadToolBench::build(env_usize("METRUM_ROAD_TOOL_TIER").unwrap_or(30_000));
    let moves = bench.moves();
    for straight in [true, false] {
        bench.restart(straight);
        for index in 0..moves * (1 + trails) {
            black_box(bench.session_move(index));
        }
    }
    std::process::exit(0)
}

fn bench_session(c: &mut Criterion) {
    if let Ok(tiers) = std::env::var("METRUM_ROAD_TOOL_LOCALITY") {
        locality(&tiers);
    }
    if let Some(trails) = env_usize("METRUM_ROAD_TOOL_ALLOC_PROBE") {
        allocation_probe(trails);
    }
    let mut bench = RoadToolBench::build(30_000);
    let moves = bench.moves();
    let mut group = c.benchmark_group("road_tool_session_30k");
    let mut index = 0;
    group.bench_function("control_cursor", |b| {
        b.iter(|| {
            index += 1;
            black_box(bench.cursor_move(index % moves))
        })
    });
    for (name, straight) in [("session_straight", true), ("session_curve", false)] {
        bench.restart(straight);
        group.bench_function(name, |b| {
            b.iter(|| {
                index += 1;
                black_box(bench.session_move(index % moves))
            })
        });
    }
    group.finish();
}

criterion_group!(benches, bench_session);
criterion_main!(benches);
