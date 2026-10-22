#!/usr/bin/env python3
"""SIM-02 step gate: behaviour envelope, burst limits, invariants, determinism and PERF-07.

See docs/simulation.md#migration. Every check runs ignored release library tests that build
fresh populated cities, so a step is compared against the pre-refactor build without loading
its saves. Results and the build identity go to the artifact directory.
"""

import argparse
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
from perf_contract import build_binary  # noqa: E402

PREFIX = "nodes::sim::core::populated_city::"
ENVELOPE_TEST = PREFIX + "envelope::sim02_envelope"
DETERMINISM_TEST = PREFIX + "envelope::sim02_determinism"
PREPARE_TEST = PREFIX + "contract::perf_contract_prepare"
PERF07_TEST = PREFIX + "frame_budget::perf07_frame_budget"
BASELINE = ROOT / "tools/sim02_envelope_baseline.json"

TIERS = (10000, 30000)
WORKLOADS = ("baseline", "fiscal_stress")
# Every tenth household leaves after the build, so admission launches enough carriers for the
# arrival burst limit (from SIM-02-E). No pre-refactor band exists; it is checked for invariants,
# money conservation and bursts only.
ARRIVAL_WORKLOAD = ("arrivals", 30000)
# Fixture variants: neighbouring city sizes give the natural spread of every metric.
VARIANTS = (0.97, 1.0, 1.03)
PERF07_TIERS = (10000, 30000, 100000)
PERF07_SPEEDS = (1, 32)
BURST_MINUTE_LIMIT = 0.05
BURST_SETTLEMENT_LIMIT = 0.10
# Rounding of the funded fixture treasury (ulp 0.125 at 1e15) bounds honest residuals.
MONEY_RESIDUAL_LIMIT = 5.0
MONEY_RESIDUAL_SLACK = 1.5
# Run-to-run noise of one build (docs/simulation.md#frame-budget-perf-07): the worst frame
# varies by about 20%, the p99 by about 10%; "no worse" allows a little more.
MAX_SLACK = 1.25
P99_SLACK = 1.10
TIMING_FLOOR_MS = 0.5
# PERF-07 acceptance (docs/simulation.md#open-questions, question 7): a tier passes when its
# worst frame no other task preempted, its raw p99.9 and its worst preempted frame less the
# wait are each at most 8 ms. 100k at 32x is deferred (PERF-08) and only reported.
PERF07_ACCEPTANCE_MS = 8.0
PERF07_ACCEPTED_TIERS = ((30000, 1), (30000, 32), (100000, 1))
PERF07_ACCEPTANCE_KEYS = ("max_unpreempted_ms", "p999_ms", "max_preempted_less_wait_ms")


def run_test(binary, name, env, log_path, tags):
    command = [str(binary), name, "--exact", "--ignored", "--nocapture", "--test-threads=1"]
    with log_path.open("w") as log:
        result = subprocess.run(command, cwd=ROOT / "rust", env=env, stdout=log,
                                stderr=subprocess.STDOUT)
    text = log_path.read_text()
    if result.returncode or "1 passed" not in text:
        raise RuntimeError(f"{name} failed; see {log_path}")
    records = {tag: [] for tag in tags}
    for line in text.splitlines():
        for tag in tags:
            marker = tag + " "
            if marker in line:
                records[tag].append(json.loads(line.split(marker, 1)[1]))
    return records


def variant_residents(tier, variant):
    return int(round(tier * variant))


def run_envelope(binary, output, variants, jobs, threads, arrivals=False):
    runs = [(tier, workload, variant) for tier in TIERS for workload in WORKLOADS
            for variant in variants]
    if arrivals:
        runs.append((ARRIVAL_WORKLOAD[1], ARRIVAL_WORKLOAD[0], 1.0))

    def one(run):
        tier, workload, variant = run
        residents = variant_residents(tier, variant)
        env = dict(os.environ, RAYON_NUM_THREADS=str(threads),
                   METRUM_SIM02_RESIDENTS=str(residents), METRUM_SIM02_WORKLOAD=workload)
        label = f"envelope-{workload}-{residents}"
        records = run_test(binary, ENVELOPE_TEST, env, output / f"{label}.log",
                           ("SIM02_DAY", "SIM02_ENVELOPE"))
        print(f"done {label}", flush=True)
        return {"tier": tier, "workload": workload, "variant": variant,
                "residents": residents, "days": records["SIM02_DAY"],
                "summary": records["SIM02_ENVELOPE"][0]}

    with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as pool:
        return list(pool.map(one, runs))


def metrics(run):
    """Per-run envelope metrics, normalised so neighbouring city sizes are comparable."""
    days = run["days"]
    target = run["residents"]
    n = len(days)

    def mean(key, per=None):
        values = []
        for day in days:
            value = day[key]
            if per is not None:
                denominator = per(day)
                value = value / denominator if denominator else 0.0
            values.append(value)
        return sum(values) / n

    final = days[-1]
    return {
        "households_per_target": mean("households") / target,
        "residents_per_target": mean("residents_now") / target,
        "employment_rate": mean("employed", lambda d: d["residents_now"]),
        "jobs_per_target": mean("jobs") / target,
        "buildings_per_target": mean("buildings") / target,
        "vacancy_rate": mean("vacant_homes", lambda d: d["households"]),
        "unhoused_rate": mean("unhoused_households", lambda d: d["households"]),
        "final_households_per_target": final["households"] / target,
        "final_employment_rate": final["employed"] / max(final["residents_now"], 1),
        "departures_per_household": final["departures"] / max(final["households"], 1),
        "net_income_per_resident_day": mean("net_income", lambda d: d["residents_now"]),
        "bankruptcies_per_1k_buildings_day": 1000 * mean("bankruptcies", lambda d: d["buildings"]),
        "deliveries_per_1k_buildings_day": 1000 * mean("freight_deliveries", lambda d: d["buildings"]),
        "out_of_stock_rate": mean("out_of_stock_households", lambda d: d["households"]),
        "trip_failures_per_1k_residents_day": 1000 * mean("trip_failures", lambda d: d["residents_now"]),
    }


# Absolute floors keep a metric that is zero in every variant from having zero tolerance.
FLOORS = {
    "bankruptcies_per_1k_buildings_day": 0.5,
    "deliveries_per_1k_buildings_day": 1.0,
    "unhoused_rate": 0.002,
    "vacancy_rate": 0.005,
    "out_of_stock_rate": 0.005,
    "departures_per_household": 0.005,
    "trip_failures_per_1k_residents_day": 0.5,
    "net_income_per_resident_day": 0.5,
}
RELATIVE_FLOOR = 0.02
# Trip failures must not rise; every other metric may move either way within its band.
UPPER_ONLY = {"trip_failures_per_1k_residents_day"}


def bands(runs):
    result = {}
    for tier in TIERS:
        for workload in WORKLOADS:
            group = [run for run in runs if run["tier"] == tier and run["workload"] == workload]
            values = [metrics(run) for run in group]
            base = next(metrics(run) for run in group if run["variant"] == 1.0)
            entry = {}
            for name in base:
                low = min(v[name] for v in values)
                high = max(v[name] for v in values)
                width = max(0.5 * (high - low), RELATIVE_FLOOR * abs(base[name]),
                            FLOORS.get(name, 0.0))
                entry[name] = {"base": base[name], "low": low - width, "high": high + width}
            result[f"{workload}-{tier}"] = entry
    return result


def burst_summary(run):
    days = run["days"]
    kinds = {}
    for name in ("freight", "arrival", "shopping"):
        shares = [d["bursts"]["kinds"][name]["worst_minute_share"] for d in days
                  if d["bursts"]["kinds"][name]["qualifying_hours"] > 0]
        kinds[name] = {"worst_minute_share": max(shares) if shares else None,
                       "qualifying_days": len(shares)}
    settlement = [d["bursts"]["worst_settlement_hour_share"] for d in days
                  if d["bursts"]["settlements"] > 0]
    return {"kinds": kinds, "worst_settlement_hour_share": max(settlement) if settlement else None}


def check_envelope(runs, baseline, identical, bursts_required):
    failures = []
    notes = []
    for run in runs:
        key = f"{run['workload']}-{run['tier']}"
        if run["variant"] != 1.0:
            continue
        summary = run["summary"]
        if summary["violations"]:
            failures.append(f"{key}: invariant violations {summary['violations']} "
                            f"{summary['violation_examples'][:3]}")
        # f32 tax accumulators already leak about 1e-4 of daily tax at 30k (SIM-02-A), so the
        # limit follows the pre-refactor residual of the same workload.
        residual_workload = "baseline" if run["workload"] == ARRIVAL_WORKLOAD[0] else run["workload"]
        baseline_residual = max(value["max_hour_money_residual"]
                                for name, value in baseline["summaries"].items()
                                if name.startswith(residual_workload + "-")
                                and abs(int(name.rsplit("-", 1)[1]) - run["tier"]) <= 0.05 * run["tier"])
        limit = MONEY_RESIDUAL_SLACK * baseline_residual + MONEY_RESIDUAL_LIMIT
        if summary["max_hour_money_residual"] > limit:
            failures.append(f"{key}: money residual {summary['max_hour_money_residual']:.3f} "
                            f"> {limit:.3f}")
        current = metrics(run)
        for name, band in baseline["bands"].get(key, {}).items():
            value = current[name]
            low = -float("inf") if name in UPPER_ONLY else band["low"]
            if not low <= value <= band["high"]:
                notes.append(f"{key}: {name} {value:.6g} outside [{band['low']:.6g}, "
                             f"{band['high']:.6g}] (base {band['base']:.6g})")
        if identical and key in baseline["fingerprints"]:
            expected = baseline["fingerprints"][key]
            got = [day["fingerprint"] for day in run["days"]]
            if got != expected:
                first = next((i for i, (a, b) in enumerate(zip(got, expected)) if a != b),
                             min(len(got), len(expected)))
                failures.append(f"{key}: not identical from sample {first}")
        summary_bursts = burst_summary(run)
        if "departures" in bursts_required:
            for name, kind in summary_bursts["kinds"].items():
                share = kind["worst_minute_share"]
                if share is not None and share > BURST_MINUTE_LIMIT:
                    failures.append(f"{key}: {name} busiest minute {share:.3f}")
            if run["workload"] == ARRIVAL_WORKLOAD[0] \
                    and not summary_bursts["kinds"]["arrival"]["qualifying_days"]:
                failures.append(f"{key}: no hour with enough arrivals to check")
        share = summary_bursts["worst_settlement_hour_share"]
        if "settlements" in bursts_required and share is not None \
                and share > BURST_SETTLEMENT_LIMIT:
            failures.append(f"{key}: busiest settlement hour {share:.3f}")
    return failures, notes


def run_perf07(binary, output, tiers, speeds):
    rows = []
    for tier in tiers:
        env = dict(os.environ, RAYON_NUM_THREADS=os.environ.get("RAYON_NUM_THREADS", "8"),
                   METRUM_PERF_RESIDENTS=str(tier),
                   METRUM_PERF_CACHE=str(output / "snapshots"))
        run_test(binary, PREPARE_TEST, env, output / f"prepare-{tier}.log", ("PERF_PREPARE",))
        for speed in speeds:
            records = run_test(binary, PERF07_TEST, dict(env, METRUM_PERF07_SPEED=str(speed)),
                               output / f"perf07-{tier}-{speed}x.log", ("PERF07_DAY",))
            row = records["PERF07_DAY"][0]
            # Builds before SIM-02-K part 7 report no preemption fields.
            split = (f" (unpreempted {row['max_unpreempted_ms']:.2f})", f" ({row['over_8_ms_preempted']} "
                     f"preempted, {row['over_8_ms_unpreempted']} unpreempted)") \
                if "max_unpreempted_ms" in row else ("", "")
            print(f"PERF-07 {tier} {speed}x: max {row['max_ms']:.2f} ms{split[0]}, "
                  f"p99 {row['p99_ms']:.2f}, p99.9 {row['p999_ms']:.2f}, "
                  f">8 ms {row['over_8_ms']}{split[1]}, >16.7 ms {row['over_16_7_ms']}", flush=True)
            rows.append(row)
    return rows


def perf07_table(rows):
    """Markdown table of the runs. A frame counts as preempted when one of its threads waited
    `preempted_wait_ms` or more for a CPU; the gate limits stay on the raw numbers."""
    def value(row, key, digits):
        return "–" if row.get(key) is None else f"{row[key]:,.{digits}f}"

    lines = ["| Tier | Speed | Max | Max unpreempted | p99 | p99.9 | Mean | Frames > 8 ms | "
             "preempted | unpreempted | > 16.7 ms | Preempted frames |",
             "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |"]
    for row in rows:
        lines.append(f"| {row['residents'] // 1000}k | {row['speed']:g}× | {value(row, 'max_ms', 1)} | "
                     f"{value(row, 'max_unpreempted_ms', 1)} | {value(row, 'p99_ms', 2)} | "
                     f"{value(row, 'p999_ms', 2)} | {value(row, 'mean_ms', 2)} | "
                     f"{value(row, 'over_8_ms', 0)} | {value(row, 'over_8_ms_preempted', 0)} | "
                     f"{value(row, 'over_8_ms_unpreempted', 0)} | {value(row, 'over_16_7_ms', 0)} | "
                     f"{value(row, 'preempted_frames', 0)} / {value(row, 'frames', 0)} |")
    return lines


def perf07_acceptance(rows):
    """The acceptance values of every measured tier the limit covers, and the failures. A run
    without scheduler statistics cannot be judged and is reported as a failure."""
    accepted, failures = [], []
    for row in rows:
        if (row["residents"], int(row["speed"])) not in PERF07_ACCEPTED_TIERS:
            continue
        values = {key: row.get(key) for key in PERF07_ACCEPTANCE_KEYS}
        missing = [key for key, value in values.items() if value is None]
        over = [key for key, value in values.items() if value is not None and value > PERF07_ACCEPTANCE_MS]
        accepted.append({"residents": row["residents"], "speed": row["speed"], **values,
                         "pass": not missing and not over})
        label = f"PERF-07 acceptance {row['residents']} {row['speed']:g}x"
        failures += [f"{label} {key} {values[key]:.2f} > {PERF07_ACCEPTANCE_MS:g}" for key in over]
        failures += [f"{label} {key} not measured" for key in missing]
    return accepted, failures


def portable_command(argv):
    """The command line with local paths replaced by the docs' placeholders."""
    names = {"--binary": "$TEST_BINARY", "--artifacts": "$ARTIFACTS"}
    command = []
    for item in argv:
        if command and command[-1] in names:
            item = names[command[-1]]
        elif item.startswith(str(ROOT)):
            item = str(Path(item).relative_to(ROOT))
        command.append(item)
    return command


def compare_perf07(rows, baseline_rows):
    """Failures where a tier and speed got worse than the baseline report beyond noise."""
    baseline = {(row["residents"], row["speed"]): row for row in baseline_rows}
    failures = []
    for row in rows:
        before = baseline.get((row["residents"], row["speed"]))
        if before is None:
            continue
        for key, slack in (("max_ms", MAX_SLACK), ("p99_ms", P99_SLACK)):
            limit = before[key] * slack + TIMING_FLOOR_MS
            if row[key] > limit:
                failures.append(f"PERF-07 {row['residents']} {row['speed']:g}x {key} "
                                f"{row[key]:.2f} > {limit:.2f} (baseline {before[key]:.2f})")
    return failures


def identity(binary, command):
    return {
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "git_head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "diff_sha256": hashlib.sha256(subprocess.check_output(["git", "diff", "HEAD"], cwd=ROOT)).hexdigest(),
        "workers": os.environ.get("RAYON_NUM_THREADS", "8"),
        "cpu_affinity": sorted(os.sched_getaffinity(0)),
        "command": command,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("checks", nargs="+",
                        choices=("envelope", "determinism", "perf07", "record-baseline"))
    parser.add_argument("--binary", type=Path, help="prebuilt release library test executable")
    parser.add_argument("--artifacts", type=Path, default=ROOT / "rust/target/sim02-gate")
    parser.add_argument("--identical", action="store_true",
                        help="require day-by-day identical fingerprints against the baseline")
    parser.add_argument("--bursts", default="",
                        help="comma list of burst limits to enforce: departures,settlements")
    parser.add_argument("--jobs", type=int, default=4, help="parallel envelope processes")
    parser.add_argument("--perf07-tiers", default=",".join(map(str, PERF07_TIERS)))
    parser.add_argument("--perf07-speeds", default=",".join(map(str, PERF07_SPEEDS)))
    parser.add_argument("--perf07-baseline", type=Path,
                        help="report.json of an earlier perf07 run to compare against")
    parser.add_argument("--cpus", default="",
                        help="comma list of CPUs to pin this run and its tests to, e.g. one "
                             "thread per performance core on a hybrid CPU")
    args = parser.parse_args()
    if args.cpus:
        os.sched_setaffinity(0, {int(cpu) for cpu in args.cpus.split(",")})
    output = args.artifacts.resolve()
    output.mkdir(parents=True, exist_ok=True)
    binary = args.binary.resolve() if args.binary else build_binary(output)
    report = {"identity": identity(binary, portable_command(sys.argv)), "failures": [], "notes": []}
    started = time.monotonic()
    bursts = {item for item in args.bursts.split(",") if item}

    if "record-baseline" in args.checks:
        runs = run_envelope(binary, output, VARIANTS, args.jobs, 6)
        baseline = {
            "identity": report["identity"],
            "bands": bands(runs),
            "fingerprints": {f"{r['workload']}-{r['tier']}": [d["fingerprint"] for d in r["days"]]
                             for r in runs if r["variant"] == 1.0},
            "bursts": {f"{r['workload']}-{r['tier']}": burst_summary(r)
                       for r in runs if r["variant"] == 1.0},
            "summaries": {f"{r['workload']}-{r['residents']}": r["summary"] for r in runs},
            "metrics": {f"{r['workload']}-{r['residents']}": metrics(r) for r in runs},
        }
        BASELINE.write_text(json.dumps(baseline, indent=1, sort_keys=True) + "\n")
        (output / "envelope.json").write_text(json.dumps(runs) + "\n")
        report["baseline"] = str(BASELINE.relative_to(ROOT))
    if "envelope" in args.checks:
        baseline = json.loads(BASELINE.read_text())
        runs = run_envelope(binary, output, (1.0,), args.jobs, 6,
                            arrivals="departures" in bursts)
        (output / "envelope.json").write_text(json.dumps(runs) + "\n")
        failures, notes = check_envelope(runs, baseline, args.identical, bursts)
        report["failures"] += failures
        report["notes"] += notes
        report["envelope"] = {f"{r['workload']}-{r['tier']}": {
            "metrics": metrics(r), "bursts": burst_summary(r),
            "money_residual": r["summary"]["max_hour_money_residual"],
            "violations": r["summary"]["violations"]} for r in runs}
    if "determinism" in args.checks:
        env = dict(os.environ, RAYON_NUM_THREADS=os.environ.get("RAYON_NUM_THREADS", "8"),
                   METRUM_SIM02_RESIDENTS="10000")
        try:
            records = run_test(binary, DETERMINISM_TEST, env, output / "determinism.log",
                               ("SIM02_DETERMINISM",))
            report["determinism"] = records["SIM02_DETERMINISM"][0]
        except RuntimeError as error:
            report["failures"].append(f"determinism: {error}")
    if "perf07" in args.checks:
        tiers = [int(value) for value in args.perf07_tiers.split(",")]
        speeds = [int(value) for value in args.perf07_speeds.split(",")]
        report["perf07"] = run_perf07(binary, output, tiers, speeds)
        report["perf07_table"] = perf07_table(report["perf07"])
        print("\n".join(report["perf07_table"]))
        report["perf07_acceptance"], acceptance_failures = perf07_acceptance(report["perf07"])
        report["failures"] += acceptance_failures
        for row in report["perf07_acceptance"]:
            shown = ", ".join("–" if row[key] is None else f"{row[key]:.2f}"
                              for key in PERF07_ACCEPTANCE_KEYS)
            print(f"PERF-07 acceptance {row['residents']} {row['speed']:g}x (unpreempted, p99.9, "
                  f"preempted less wait): {shown}: {'PASS' if row['pass'] else 'FAIL'}")
        if args.perf07_baseline:
            baseline_rows = json.loads(args.perf07_baseline.read_text())["perf07"]
            report["failures"] += compare_perf07(report["perf07"], baseline_rows)
    report["runtime_s"] = time.monotonic() - started
    (output / "report.json").write_text(json.dumps(report, indent=1) + "\n")
    for note in report["notes"]:
        print("NOTE: " + note)
    for failure in report["failures"]:
        print("FAIL: " + failure, file=sys.stderr)
    print(f'{"FAIL" if report["failures"] else "PASS"}: {report["runtime_s"]:.0f}s; '
          f'{output / "report.json"}')
    return int(bool(report["failures"]))


if __name__ == "__main__":
    sys.exit(main())
