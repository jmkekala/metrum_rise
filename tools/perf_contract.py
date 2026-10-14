#!/usr/bin/env python3
"""Snapshot-backed economy and agent contracts; no Criterion sampling in routine runs."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
PHASES = ("census", "planning", "execution", "logistics", "lots", "settlement", "routing")
# This map narrows isolated diagnostics only. Any economy selection also runs the complete
# next day: downstream workload changes cannot be inferred safely from changed paths alone.
PATH_PHASES = (
    ("rust/src/simulation/economy/hiring", {"census", "planning", "execution", "settlement"}),
    ("rust/src/simulation/economy/demand/", {"census", "planning", "execution", "lots", "settlement"}),
    ("rust/src/simulation/economy/logistics/", {"census", "planning", "execution", "logistics", "settlement", "routing"}),
    ("rust/src/simulation/economy/households/employment", {"census", "planning", "execution", "settlement", "routing"}),
    ("rust/src/simulation/zoning/", {"planning", "execution", "lots"}),
)
BOUNDED = ("census_buildings", "reservation_rows_copied", "owner_slots_prepared",
           "buyer_sites_built", "spawn_parcels_scanned", "staffing_trials",
           "supplier_searches", "supplier_routes", "job_search_keys")
TEST_PREFIX = "nodes::sim::core::populated_city::contract::"
AGENT_TEST = "simulation::economy::agents::tick::contract::benchmark_agent_tick_contract"


def selected_phases(paths):
    """Unknown and shared paths fail closed to all checks."""
    selected = set()
    for path in paths:
        if path.startswith("docs/") or path in ("README.md", "CLAUDE.md", "AGENTS.md"):
            continue
        if path.startswith(("rust/src/simulation/economy/customers",
                            "rust/src/simulation/economy/logistics/recurring_supply")):
            return list(PHASES)
        for prefix, phases in PATH_PHASES:
            if path.startswith(prefix):
                selected.update(phases)
                break
        else:
            return list(PHASES)
    return [phase for phase in PHASES if phase in selected]


def changed_paths(base):
    tracked = subprocess.check_output(["git", "diff", "--name-only", "-z", base, "--"], cwd=ROOT)
    untracked = subprocess.check_output(["git", "ls-files", "--others", "--exclude-standard", "-z"], cwd=ROOT)
    return [path.decode() for path in (tracked + untracked).split(b"\0") if path]


def check_limits(rows, require_churn=False, workload=None):
    """Return timing and work failures, optionally requiring hourly churn coverage."""
    if workload is None and rows:
        workload = rows[0].get("workload")

    def labeled(messages):
        if not workload:
            return messages
        return [f"{workload}: {message}" for message in messages]

    failures = []
    by_tier = {row["residents"]: row for row in rows}
    if set(by_tier) != {10000, 30000} or len(rows) != 2:
        return labeled(["expected exactly one day result for each of 10000 and 30000 residents"])
    small, large = by_tier[10000], by_tier[30000]
    for row, hour, day in ((small, 40, 10), (large, 120, 30)):
        if require_churn:
            churn = row.get("building_churn", [])
            if len(churn) != 24:
                failures.append(f'{row["residents"]} missing 24-hour building churn coverage')
            else:
                inactive = [index for index, (placed, removed) in enumerate(churn)
                            if placed + removed == 0]
                if inactive:
                    failures.append(f'{row["residents"]} no building placement/removal in hour intervals {inactive}')
        for key, limit in (("hourly_ms", hour), ("daily_ms", day)):
            if not 0 <= row[key] <= limit:
                failures.append(f'{row["residents"]} {key}: {row[key]:.3f} > {limit}')
    if small["households"] <= 0 or large["households"] < small["households"]:
        return labeled(failures + ["invalid final household scaling denominator"])
    ratio = 1.2 * large["households"] / small["households"]
    for name in (*BOUNDED, "cch_searches"):
        def count(row):
            if name == "cch_searches":
                return row["counts"]["cch_point_searches"] + row["counts"]["cch_bucket_sources"]
            return row["counts"][name]
        low, high = count(small), count(large)
        if high > ratio * low:
            failures.append(f"{name}: {low} -> {high}, limit {ratio:.4f}x")
    return labeled(failures)


def check_fiscal_coverage(rows):
    """Require the dedicated fiscal workload to retain its day-16 hourly activity."""
    def fiscal_labels(messages):
        return [f"fiscal_stress: {message}" for message in messages]

    failures = []
    by_tier = {row.get("residents"): row for row in rows}
    if set(by_tier) != {10000, 30000} or len(rows) != 2:
        return fiscal_labels(["expected exactly one fiscal workload result for each of 10000 and 30000 residents"])
    for residents, row in sorted(by_tier.items()):
        if row.get("workload") != "fiscal_stress":
            failures.append(f"{residents} fiscal result has wrong workload")
        if row.get("day") != 16:
            failures.append(f"{residents} fiscal result is not the day-16 fixture")
        churn = row.get("building_churn", [])
        if len(churn) != 24:
            failures.append(f"{residents} fiscal workload missing 24-hour churn coverage")
            continue
        inactive = [index for index, (placed, removed) in enumerate(churn)
                    if placed + removed == 0]
        if inactive:
            failures.append(f"{residents} fiscal workload has inactive hour intervals {inactive}")
    return fiscal_labels(failures)


def build_binary(output):
    started = time.monotonic()
    command = ["cargo", "test", "--release", "--lib", "--no-run", "--message-format=json"]
    process = subprocess.Popen(command, cwd=ROOT / "rust", stdout=subprocess.PIPE, text=True)
    binary = None
    with (output / "build.jsonl").open("w") as log:
        for line in process.stdout:
            log.write(line)
            try:
                item = json.loads(line)
            except json.JSONDecodeError:
                continue
            if item.get("reason") == "compiler-message":
                print(item["message"].get("rendered", ""), file=sys.stderr, end="")
            if item.get("reason") == "compiler-artifact" and item.get("executable") and item["profile"]["test"]:
                binary = Path(item["executable"])
    if process.wait() or binary is None:
        raise RuntimeError("release contract build failed")
    print(f"Build: {time.monotonic() - started:.1f}s (outside runtime budget)", flush=True)
    return binary


REPORT_TAGS = ("PERF_DAY ", "PERF_PHASE ", "PERF_PREPARE ",
               "PERF_FISCAL_PREPARE ", "PERF_FISCAL_DAY ",
               "PERF_LOCALITY ", "PERF_AGENT ")


def run_test(binary, name, env, output, label, expected_tag, expected_records=1, deadline=None):
    command = [str(binary), name, "--exact", "--ignored", "--nocapture", "--test-threads=1"]
    timeout = None if deadline is None else max(0.001, deadline - time.monotonic())
    print(f"Running {label}", flush=True)
    with (output / f"{label}.log").open("w") as log:
        result = subprocess.run(command, cwd=ROOT / "rust", env=env, stdout=log,
                                stderr=subprocess.STDOUT, timeout=timeout)
    text = (output / f"{label}.log").read_text()
    print(text, end="", flush=True)
    if result.returncode:
        raise RuntimeError(f"{label} failed; see {output / (label + '.log')}")
    if "1 passed" not in text:
        raise RuntimeError(f"{label} did not execute exactly one test")
    records = []
    unexpected = []
    for line in text.splitlines():
        # libtest can prepend its test name to stderr's first line.
        for tag in REPORT_TAGS:
            if tag in line:
                if tag != expected_tag:
                    unexpected.append(tag.rstrip())
                    continue
                try:
                    records.append(json.loads(line.split(tag, 1)[1]))
                except json.JSONDecodeError as error:
                    raise RuntimeError(f"{label} emitted invalid {expected_tag.strip()} JSON") from error
    if unexpected:
        raise RuntimeError(f"{label} emitted unexpected reports: {', '.join(unexpected)}")
    if len(records) != expected_records:
        raise RuntimeError(
            f"{label} emitted {len(records)} {expected_tag.strip()} reports; "
            f"expected {expected_records}"
        )
    return records


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--diff", metavar="BASE", help="select isolated phases from all changes against BASE")
    parser.add_argument("--phase", action="append", choices=PHASES, help="isolated diagnosis only; not full acceptance")
    parser.add_argument("--plan", action="store_true", help="print selection without compiling or executing")
    parser.add_argument("--prepare", action="store_true", help="only prepare/verify revision-checked snapshots")
    parser.add_argument("--agents", action="store_true", help="run only the short PERF-06 agent contract")
    parser.add_argument("--binary", type=Path, help="use a prebuilt release library test executable")
    parser.add_argument("--keep-baseline", action="store_true", help="retain this executable and build identity, without replacing an existing baseline")
    parser.add_argument("--artifacts", type=Path, default=ROOT / "rust/target/perf-contract")
    args = parser.parse_args()
    if args.agents and (args.phase or args.prepare or args.diff):
        parser.error("--agents cannot be combined with --phase, --prepare or --diff")
    phases = args.phase or (selected_phases(changed_paths(args.diff)) if args.diff else list(PHASES))
    print(json.dumps({"phases": [] if args.agents else phases,
                      "agents": args.agents or (bool(phases) and not args.phase and not args.prepare),
                      "full_day": bool(phases) and not (args.phase or args.agents or args.prepare),
                      "runtime_budget_s": 300, "workers": os.environ.get("RAYON_NUM_THREADS", "8")}))
    if args.plan or not phases:
        return 0
    output = args.artifacts.resolve()
    output.mkdir(parents=True, exist_ok=True)
    binary = args.binary.resolve() if args.binary else build_binary(output)
    identity = {"binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                "git_head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                "diff_sha256": hashlib.sha256(subprocess.check_output(["git", "diff", "HEAD"], cwd=ROOT)).hexdigest(),
                "workers": os.environ.get("RAYON_NUM_THREADS", "8"),
                "cpu_affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
                "command": sys.argv}
    (output / "identity.json").write_text(json.dumps(identity, indent=2) + "\n")
    if args.keep_baseline:
        baseline = output / "baseline"
        baseline.mkdir(exist_ok=False)
        shutil.copy2(binary, baseline / "contract-tests")
        shutil.copy2(output / "identity.json", baseline / "identity.json")
    baseline_identity = output / "baseline/identity.json"
    baseline = json.loads(baseline_identity.read_text()) if baseline_identity.exists() else None
    print(json.dumps({"build": identity, "retained_baseline": baseline}), flush=True)
    env = dict(os.environ, RAYON_NUM_THREADS=identity["workers"], METRUM_PERF_CACHE=str(output / "snapshots"))
    records = []
    fiscal_prepare_records = []
    fiscal_records = []
    gate_failures = []
    report_started = time.monotonic()
    timed_started = None

    def write_report(status, failures=None, error=None):
        report = {
            "identity": identity,
            "retained_baseline": baseline,
            "runtime_s": time.monotonic() - (timed_started or report_started),
            "diagnostic_only": bool(args.phase),
            "status": status,
            "records": records,
            "fiscal_prepare_records": fiscal_prepare_records,
            "fiscal_records": fiscal_records,
            "failures": failures or [],
        }
        if error is not None:
            report["error"] = str(error)
        (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")

    def invoke(*run_args, record_store=None, **run_kwargs):
        try:
            result = run_test(*run_args, **run_kwargs)
        except (RuntimeError, subprocess.TimeoutExpired, OSError, ValueError) as error:
            completed_days = [row for row in records if "hourly_ms" in row]
            completed_fiscal_days = [row for row in fiscal_records if "hourly_ms" in row]
            failures = list(gate_failures)
            if len(completed_days) == 2:
                failures.extend(check_limits(completed_days, require_churn=False, workload="baseline"))
            fiscal_day_run = len(run_args) > 4 and str(run_args[4]).startswith("fiscal-day-")
            if len(completed_fiscal_days) == 2:
                failures.extend(check_limits(completed_fiscal_days, workload="fiscal_stress"))
                failures.extend(check_fiscal_coverage(completed_fiscal_days))
            elif completed_fiscal_days or fiscal_day_run:
                failures.extend(check_fiscal_coverage(completed_fiscal_days))
            write_report("failed", failures=failures, error=error)
            raise
        (records if record_store is None else record_store).extend(result)
        write_report("running")
        return result

    if args.agents:
        started = time.monotonic()
        agent_records = invoke(binary, AGENT_TEST, env, output, "agents", "PERF_AGENT ", 6, started + 300)
        (output / "agents-report.json").write_text(json.dumps({
            "identity": identity, "retained_baseline": baseline,
            "runtime_s": time.monotonic() - started, "records": agent_records,
        }, indent=2) + "\n")
        write_report("passed")
        print(f"PASS: agent contract in {time.monotonic() - started:.1f}s", flush=True)
        return 0
    for residents in (10000, 30000):
        tier_env = dict(env, METRUM_PERF_RESIDENTS=str(residents))
        invoke(binary, TEST_PREFIX + "perf_contract_prepare", tier_env, output,
               f"prepare-{residents}", "PERF_PREPARE ")
        if not args.phase:
            invoke(binary, TEST_PREFIX + "perf_contract_fiscal_prepare", tier_env, output,
                   f"fiscal-prepare-{residents}", "PERF_FISCAL_PREPARE ",
                   record_store=fiscal_prepare_records)
    if args.prepare:
        write_report("passed")
        return 0
    started = time.monotonic()
    timed_started = started
    deadline = started + 300
    days = []
    for residents in (10000, 30000):
        tier_env = dict(env, METRUM_PERF_RESIDENTS=str(residents))
        if not args.phase:
            result = invoke(binary, TEST_PREFIX + "perf_contract_day", tier_env, output,
                            f"day-{residents}", "PERF_DAY ", 1, deadline)
            days.extend(result)
            invoke(binary, TEST_PREFIX + "perf_contract_fiscal_day", tier_env, output,
                   f"fiscal-day-{residents}", "PERF_FISCAL_DAY ", 1, deadline,
                   record_store=fiscal_records)
        for phase in phases:
            invoke(binary, TEST_PREFIX + "perf_contract_phase",
                   dict(tier_env, METRUM_PERF_PHASE=phase), output,
                   f"{phase}-{residents}", "PERF_PHASE ", 1, deadline)
    if not args.phase:
        gates = (
            ("nodes::sim::core::tests::perf_contract_queued_locality", "PERF_LOCALITY ", 1),
            ("simulation::economy::households::tests::employment::markets::perf_contract_financing_locality", "PERF_LOCALITY ", 1),
            (AGENT_TEST, "PERF_AGENT ", 6),
        )
        for name, tag, count in gates:
            if time.monotonic() >= deadline:
                gate_failures.append("runtime deadline reached before remaining gates")
                break
            label = "agents" if name == AGENT_TEST else name.split("::")[-1]
            try:
                invoke(binary, name, env, output, label, tag, count, deadline)
            except (RuntimeError, subprocess.TimeoutExpired, OSError, ValueError) as error:
                # Independent gates still provide useful evidence after a locality failure.
                gate_failures.append(str(error))
    failures = gate_failures
    if not args.phase:
        failures.extend(check_limits(days, require_churn=False, workload="baseline"))
        failures.extend(check_limits(fiscal_records, workload="fiscal_stress"))
        failures.extend(check_fiscal_coverage(fiscal_records))
    elapsed = time.monotonic() - started
    if elapsed > 300:
        failures.append(f"runtime {elapsed:.1f}s exceeds 300s")
    write_report("failed" if failures else "passed", failures=failures)
    for failure in failures:
        print("FAIL: " + failure, file=sys.stderr)
    print(f'{"FAIL" if failures else "PASS"}: {elapsed:.1f}s; {output / "report.json"}', flush=True)
    return int(bool(failures))


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (RuntimeError, subprocess.TimeoutExpired) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        sys.exit(1)
