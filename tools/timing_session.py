#!/usr/bin/env python3
"""Timing sessions at the R, F and L levels (docs/simulation.md#timing-levels-and-the-session-runner-perf-19).

One command plans a round's timing rows for a control and a branch binary, prepares every
snapshot before the locks, times the rows in the exclusive phase (C B B C by default), reuses a
control's full-day rows of the same boot only after a drift calibration, and writes one summary
with component comparisons. The acceptance definition (`PERF-07`) is `sim02_gate.py`'s.

Machine specifics (CPU list, extra busy processes, quiet windows, lock wrappers) come from a
local, untracked settings file: `rust/target/timing-session.json` of the main checkout, or
`$METRUM_TIMING_SETTINGS`.
"""

import argparse
import concurrent.futures
import datetime
import fnmatch
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import sqlite3
import statistics
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import sim02_gate as gate  # noqa: E402
from perf_contract import changed_paths  # noqa: E402

LEVELS = ("R", "F", "L")
SIDES = ("ctl", "br")
BENCH_ROUNDS = 3
CONTROL_TIER = 30000
PREFIX = gate.PREFIX
FISCAL_PREPARE_TEST = PREFIX + "contract::perf_contract_fiscal_prepare"
COMMIT_PREPARE_TEST = PREFIX + "routing_benchmark::benchmark_populated_commit_prepare"
COMMIT_TEST = PREFIX + "routing_benchmark::benchmark_populated_commit_breakdown"
CONTRACT_TIERS = (10000, 30000)

# ---------------------------------------------------------------------------------------------
# The path rule (review Q2): R suffices only when every changed path is slot or edit work; any
# other source path is per-frame or unlisted, and fails closed to F.

def _module(path):
    """A module and its same-named split directory."""
    return (path + ".rs", path + "/*")


def _modules(parent, names):
    return tuple(pattern for name in names for pattern in _module(f"{parent}/{name}"))


PER_FRAME = (
    *_modules("rust/src/nodes/sim/core", ("frame", "thread", "snapshot", "parking", "lot_adoption",
                                         "turnstile", "trips", "budget")),
    "rust/src/nodes/sim/render/*", "rust/src/simulation/economy/agents/*",
    "rust/src/simulation/network/*", "rust/src/simulation/pathing/*",
    *_module("rust/src/simulation/parking"),
    *_modules("rust/src/simulation/core", ("time", "scheduler")),
    "rust/src/simulation/buildings/*", "rust/src/thread_pools.rs", "rust/src/debug/*",
    "rust/Cargo.*", "rust/rust-toolchain*", "rust-toolchain*", "rust/.cargo/*", "rust/src/lib.rs",
)
SLOT_ONLY = (
    *_modules("rust/src/simulation/economy", ("demand", "households", "logistics", "hiring",
                                              "customers", "fiscal")),
    *_module("rust/src/simulation/zoning"),
    *_modules("rust/src/nodes/sim/core", ("schedule", "entity_phases", "entity_days",
                                         "demand_requests", "household_requests")),
)
EDIT_PATHS = (
    *_modules("rust/src/nodes/sim", ("editing", "undo", "road_tool", "road_pockets", "roundabouts",
                                    "zoning")),
    "rust/src/nodes/sim/core/road_*", "rust/src/nodes/sim/core/graded_*",
    "rust/src/simulation/network/roadbed/*",
)
SAVE_PATHS = ("rust/src/simulation/save/*",)
NEUTRAL = ("docs/*", "tools/*", "godot/*", "rust/tests/*", "rust/benches/*", "*.md", "run.sh", ".gitignore",
           ".gitattributes", "logos/*", "screenshots/*")


def _matches(path, patterns):
    return any(fnmatch.fnmatchcase(path, pattern) for pattern in patterns)


def classify(paths):
    """The path rule's level for a diff: (level, reasons, edit path, save path).
    Unlisted source paths fail closed to F; docs, tools, tests and Godot need no timing."""
    level, reasons = None, []
    edit = save = False
    for path in paths:
        if _matches(path, EDIT_PATHS):
            edit = True
            level = level or "R"
            reasons.append(f"R edit path {path}")
        elif _matches(path, SLOT_ONLY):
            level = level or "R"
            reasons.append(f"R slot path {path}")
        elif _matches(path, SAVE_PATHS):
            save = True
            level = level or "R"
            reasons.append(f"R save path {path} (compare load_s)")
        elif _matches(path, NEUTRAL):
            continue
        else:
            level = "F"
            kind = "per-frame" if _matches(path, PER_FRAME) else "unlisted, fails closed"
            reasons.append(f"F {kind} {path}")
    return level, reasons, edit, save


# ---------------------------------------------------------------------------------------------
# Comparison tolerances (review Q3). Per-run sigma in percent from the pairwise same-binary
# spreads; a difference is beyond noise at 2 sigma sqrt(1/n_ctl + 1/n_br), doubled in a noisy
# session. Raw maxima and over-8 counts are not comparison metrics at n <= 3: reported only.

SIGMA_PCT = {
    "mean_ms": 0.7, "p99_ms": 1.7, "p999_ms": 4.0, "max_unpreempted_ms": 3.0,
    "mean_agent_ms": 1.25, "mean_snapshot_ms": 1.25, "mean_slot_ms": 1.25,
    "mean_pathing_ms": 1.25, "load_s": 3.5,
}
# Components this small are below timer resolution relative to their noise.
FLOOR = {"mean_agent_ms": 0.005, "mean_snapshot_ms": 0.005, "mean_slot_ms": 0.005,
         "mean_pathing_ms": 0.005}
REPORTED = ("max_ms", "max_agent_ms", "max_snapshot_ms", "max_pathing_ms", "max_cell_ms",
            "p99_snapshot_ms", "over_8_ms")
NOISY_RATIO = 2.0
# Cached-control calibration (review Q4): one 32x run of the control against its cached row.
CALIBRATION_PCT = {"mean_ms": 3.0, "p99_ms": 6.0, "p999_ms": 15.0}
CALIBRATION_PREEMPTED_RATIO = 2.0
# A second branch run when the first is within this fraction of a limit or differs from the control.
SECOND_RUN_LIMIT_BAND = 0.05
# Acceptance misses the user accepted on dev (docs/roadmap.md): (tier, speed) -> roadmap id. A
# branch row missing only these is reported as the known exception, not repeated and not failed;
# its comparison against the control still runs. Remove an entry when its row closes.
KNOWN_EXCEPTIONS = {(CONTROL_TIER, 32): "PERF-17"}


def acceptance_misses(day):
    """(new misses, known exception id or None) of one PERF-07 day against the acceptance."""
    values, _ = gate.perf07_acceptance([day])
    if not values:
        return [], None
    missed = [key for key in gate.PERF07_ACCEPTANCE_KEYS
              if values[0][key] is None or values[0][key] > gate.PERF07_ACCEPTANCE_MS]
    known = KNOWN_EXCEPTIONS.get((int(day["residents"]), int(day["speed"])))
    return ([], known) if known and missed else (missed, None)


def tolerance_pct(metric, n_ctl, n_br, noisy=False):
    sigma = SIGMA_PCT[metric]
    return 2.0 * sigma * (1.0 / n_ctl + 1.0 / n_br) ** 0.5 * (2.0 if noisy else 1.0)


def verdict(metric, ctl_values, br_values, noisy=False):
    """(delta percent, tolerance percent, verdict) of the branch mean against the control's."""
    ctl, br = statistics.fmean(ctl_values), statistics.fmean(br_values)
    tol = tolerance_pct(metric, len(ctl_values), len(br_values), noisy)
    delta = 0.0 if ctl == 0 else 100.0 * (br - ctl) / ctl
    floor = FLOOR.get(metric, 0.0)
    if abs(br - ctl) <= floor or abs(delta) <= tol:
        return delta, tol, "level"
    return delta, tol, "worse" if delta > 0 else "better"


def noisy_rows(rows):
    """Labels of rows with more than twice the preempted frames of the quietest row."""
    counts = [(row["label"], row["day"].get("preempted_frames")) for row in rows]
    known = [count for _, count in counts if count is not None]
    if len(known) < 2:
        return []
    floor = max(min(known), 1)
    return [label for label, count in counts if count is not None and count > NOISY_RATIO * floor]


def calibrate(run, cached_rows):
    """(passed, deltas, reasons) of a control's 32x run against its cached 32x rows."""
    deltas, reasons = {}, []
    for metric, limit in CALIBRATION_PCT.items():
        cached = statistics.fmean(row[metric] for row in cached_rows)
        deltas[metric] = 100.0 * (run[metric] - cached) / cached
        if abs(deltas[metric]) > limit:
            reasons.append(f"{metric} {deltas[metric]:+.1f}% beyond {limit:g}%")
    cached_preempted = statistics.fmean(row.get("preempted_frames", 0) for row in cached_rows)
    preempted = run.get("preempted_frames", 0)
    ratio = max(preempted, 1) / max(cached_preempted, 1)
    if not 1 / CALIBRATION_PREEMPTED_RATIO <= ratio <= CALIBRATION_PREEMPTED_RATIO:
        reasons.append(f"preempted frames {preempted} against {cached_preempted:.0f}")
    return not reasons, deltas, reasons


def wants_second_run(tier, speed, br_day, ctl_days):
    """A second branch full day when the first is within 5% of an acceptance limit on either side, where one more sample can change the verdict, or differs from the control
    beyond the single-run tolerance on the mean or p99. A clear miss needs no second day."""
    band = SECOND_RUN_LIMIT_BAND * gate.PERF07_ACCEPTANCE_MS
    near = [] if (tier, speed) in KNOWN_EXCEPTIONS else \
        [key for key in gate.PERF07_ACCEPTANCE_KEYS if br_day.get(key) is not None
         and abs(br_day[key] - gate.PERF07_ACCEPTANCE_MS) <= band]
    if near:
        return f"within {band / gate.PERF07_ACCEPTANCE_MS:.0%} of the limit on {', '.join(near)}"
    if not ctl_days:
        return None
    for metric in ("mean_ms", "p99_ms"):
        delta, tol, result = verdict(metric, [day[metric] for day in ctl_days], [br_day[metric]])
        if result == "worse":
            return f"{metric} {delta:+.1f}% against the control (tolerance {tol:.1f}%)"
    return None


# ---------------------------------------------------------------------------------------------
# Machine identity, local settings and the control cache.

def main_target():
    """`rust/target` of the main checkout, shared by its worktrees."""
    common = subprocess.check_output(["git", "rev-parse", "--path-format=absolute",
                                      "--git-common-dir"], cwd=ROOT, text=True).strip()
    return Path(common).parent / "rust/target"


def load_settings():
    path = Path(os.environ.get("METRUM_TIMING_SETTINGS") or main_target() / "timing-session.json")
    settings = {"cpus": "", "busy_processes": [], "quiet_windows": [], "no_cache_windows": [],
                "prepare_wrapper": [], "timed_wrapper": []}
    if path.exists():
        settings.update(json.loads(path.read_text()))
        settings["source"] = str(path)
    return settings


def boot_id():
    linux = Path("/proc/sys/kernel/random/boot_id")
    if linux.exists():
        return linux.read_text().strip()
    for name in ("kern.bootsessionuuid", "kern.boottime"):
        try:
            value = subprocess.check_output(["sysctl", "-n", name], text=True).strip()
        except (OSError, subprocess.CalledProcessError):
            continue
        if value:
            return hashlib.sha256(value.encode()).hexdigest()[:32]
    raise RuntimeError("no boot id on this system")


def power_state(cpus):
    """Governor, EPP and platform profile of the pinned CPUs where the OS exposes them."""
    state = {}
    for cpu in cpus or []:
        base = Path(f"/sys/devices/system/cpu/cpu{cpu}/cpufreq")
        for name in ("scaling_governor", "energy_performance_preference"):
            if (base / name).exists():
                state.setdefault(name, set()).add((base / name).read_text().strip())
    profile = Path("/sys/firmware/acpi/platform_profile")
    if profile.exists():
        state["platform_profile"] = {profile.read_text().strip()}
    return {key: sorted(value) for key, value in state.items()}


def file_sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


MEASUREMENT_ENV = ("METRUM_PERF07_MINUTES", "METRUM_PERF07_SCHEDSTAT",
                   "METRUM_VEHICLE_GROUND_FULL_DETAIL_M")


def cache_key(machine, binary_sha, snapshot_key, benchmark, tier, speed, workers, cpus, env):
    """Everything that makes a cached row the same measurement (review Q4). The boot id is part
    of the key and of the cache directory, so a reboot expires every row."""
    return {
        "binary_sha256": binary_sha, "snapshot_key": snapshot_key, "benchmark": benchmark,
        "tier": tier, "speed": speed, "workers": str(workers), "cpus": sorted(cpus or []),
        "env": {name: env[name] for name in sorted(env)
                if name in MEASUREMENT_ENV or name == "METRUM_DEBUG"},
        **machine,
    }


def key_hash(key):
    return hashlib.sha256(json.dumps(key, sort_keys=True).encode()).hexdigest()[:24]


class ControlCache:
    """Rows of one boot under `<cache root>/<boot id>/`; other boots' directories are deleted."""

    def __init__(self, root, boot):
        self.dir = Path(root) / boot
        self.boot = boot
        self.root = Path(root)

    def expire_other_boots(self):
        if not self.root.exists():
            return []
        stale = [path for path in self.root.iterdir() if path.is_dir() and path.name != self.boot]
        for path in stale:
            shutil.rmtree(path)
        return [path.name for path in stale]

    def get(self, key):
        path = self.dir / f"{key_hash(key)}.json"
        if not path.exists():
            return None
        entry = json.loads(path.read_text())
        return entry if entry["key"] == key else None

    def put(self, key, rows, session, recorded):
        self.dir.mkdir(parents=True, exist_ok=True)
        entry = {"key": key, "rows": rows, "session": session, "recorded": recorded}
        temporary = self.dir / f".{key_hash(key)}.{os.getpid()}.tmp"
        temporary.write_text(json.dumps(entry, indent=1) + "\n")
        temporary.replace(self.dir / f"{key_hash(key)}.json")


def in_windows(moment, windows, duration_s=0.0):
    """Whether [moment, moment + duration] touches any local `HH:MM-HH:MM` window."""
    end = moment + datetime.timedelta(seconds=duration_s)
    for window in windows:
        start_text, end_text = window.split("-")
        for day in {moment.date(), end.date()}:
            start = datetime.datetime.combine(day, datetime.time.fromisoformat(start_text))
            stop = datetime.datetime.combine(day, datetime.time.fromisoformat(end_text))
            if start <= end and moment <= stop:
                return window
    return None


def busy_processes(names):
    if not shutil.which("pgrep"):
        return []
    return [name for name in names
            if subprocess.run(["pgrep", "-x", name], stdout=subprocess.DEVNULL).returncode == 0]


# ---------------------------------------------------------------------------------------------
# The plan.

def parse_bench(spec):
    """`NAME[;KEY=VALUE...][;cpus=one]`: an ignored test of the binary timed as the change's own
    benchmark. `cpus=one` pins it to the first CPU of the list."""
    parts = [part for part in spec.split(";") if part]
    if not parts or "=" in parts[0]:
        raise ValueError(f"benchmark spec needs a test name first: {spec!r}")
    bench = {"test": parts[0], "env": {}, "cpus": "all"}
    for part in parts[1:]:
        key, sep, value = part.partition("=")
        if not sep:
            raise ValueError(f"benchmark option is not KEY=VALUE: {part!r}")
        if key == "cpus":
            if value not in ("one", "all"):
                raise ValueError("cpus must be one or all")
            bench["cpus"] = value
        else:
            bench["env"][key] = value
    bench["name"] = bench["test"].rsplit("::", 1)[-1]
    return bench


def perf07_row(side, tier, speed, **extra):
    return {"kind": "perf07", "side": side, "tier": tier, "speed": speed, **extra}


def parse_perf07(spec):
    """`TIER:SPEEDS:ORDER`, e.g. `30000:1+32:ctl,br,br,ctl`: each visit of a side runs every
    speed, as `sim02_gate.py perf07` does. Replaces the level's PERF-07 rows (a custom plan)."""
    try:
        tier, speeds, order = spec.split(":")
        rows = [perf07_row(side, int(tier), int(speed)) for side in order.split(",")
                for speed in speeds.split("+")]
    except ValueError as error:
        raise ValueError(f"PERF-07 spec is not TIER:SPEEDS:ORDER: {spec!r}") from error
    if any(row["side"] not in SIDES for row in rows):
        raise ValueError(f"PERF-07 spec sides must be ctl or br: {spec!r}")
    return rows


def plan_rows(level, merge=False, benches=(), k22=False, contract=False, k22_tiers=None,
              perf07=None):
    """The ordered rows of a level (review's amended table). `reuse` rows come from the control
    cache when the calibration allows it; `second` rows run only when their rule asks."""
    rows = []
    for number in range(1, BENCH_ROUNDS + 1):
        order = SIDES if number % 2 else SIDES[::-1]
        for side in order:
            rows += [{"kind": "test", "side": side, "round": number, **bench} for bench in benches]
    if perf07 is not None:
        rows += perf07
    else:
        rows += level_perf07_rows(level, merge)
    if level == "L" or k22:
        tiers = k22_tiers or str(CONTROL_TIER)
        control = {} if merge or level != "L" else {"reuse": "only"}
        rows += [{"kind": "k22", "side": "ctl", "tiers": tiers, **control},
                 {"kind": "k22", "side": "br", "tiers": tiers}]
    if level == "L" or contract:
        # A landing times the contract head only; the control's rows come from the cache.
        control = {} if merge else {"reuse": "only" if level == "L" else True}
        rows += [{"kind": "contract", "side": "ctl", **control}, {"kind": "contract", "side": "br"}]
    counters = {}
    rows = [dict(row) for row in rows]
    for row in rows:
        prefix = {"perf07": "p07", "test": "bench", "k22": "k22", "contract": "contract"}[row["kind"]]
        counters[prefix] = counters.get(prefix, 0) + 1
        name = f"{prefix}-{row['name']}-{row['round']}" if row["kind"] == "test" \
            else f"{prefix}-{counters[prefix]}"
        row["label"] = f"{name}-{row['side']}"
    return rows


def level_perf07_rows(level, merge):
    """30k at 1x and 32x: timing sessions have no 100k tier."""
    rows = [perf07_row(side, CONTROL_TIER, 32) for side in ("ctl", "br", "br", "ctl")]
    if level == "F":
        rows += [perf07_row("ctl", CONTROL_TIER, 1, reuse=True), perf07_row("br", CONTROL_TIER, 1),
                 perf07_row("br", CONTROL_TIER, 1, second=True)]
    elif level == "L":
        if merge:
            rows += [perf07_row(side, CONTROL_TIER, 1) for side in ("ctl", "br", "br", "ctl")]
        else:
            rows += [perf07_row("ctl", CONTROL_TIER, 1, reuse=True),
                     perf07_row("br", CONTROL_TIER, 1), perf07_row("br", CONTROL_TIER, 1)]
    return rows


def prepare_needs(rows, side):
    """Snapshot tiers each fixture of one side needs before the locks."""
    needs = {"baseline": set(), "fiscal": set(), "commit": set()}
    for row in rows:
        if row["side"] != side or row.get("reuse") == "only":
            continue
        if row["kind"] == "perf07":
            needs["baseline"].add(row["tier"])
        elif row["kind"] == "contract":
            needs["baseline"].update(CONTRACT_TIERS)
            needs["fiscal"].update(CONTRACT_TIERS)
        elif row["kind"] == "k22":
            needs["commit"].update(int(tier) for tier in row["tiers"].split(",") if int(tier) > 0)
    return {key: sorted(value) for key, value in needs.items()}


# Measured row walls on the reference machine (P1, 2026-11-09); for the quiet-window check and
# the plan's estimate only.
ESTIMATE_S = {(30000, 1): 205, (30000, 32): 20, (10000, 1): 120, (10000, 32): 15, "test": 20,
              "k22": 15, "contract": 115}


def estimate_s(row):
    if row["kind"] == "perf07":
        return ESTIMATE_S.get((row["tier"], row["speed"]), 600)
    return ESTIMATE_S[row["kind"]]


# ---------------------------------------------------------------------------------------------
# Snapshots.

def snapshot_name(fixture, tier, key):
    """Builds with the revision in the baseline name (P1) report it from their prepare; older
    builds use the unkeyed name."""
    return f"{fixture}_{tier}_{key[:16]}.sqlite" if key else f"{fixture}_{tier}.sqlite"


def snapshot_meta(path):
    """The key a snapshot was saved under: fixture revision, residents, clock and save version."""
    connection = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    try:
        meta = connection.execute("SELECT fixture_revision, residents, day_index, minute_of_day "
                                  "FROM perf_fixture_meta LIMIT 1").fetchone()
        version = connection.execute("SELECT version FROM save_meta LIMIT 1").fetchone()
    finally:
        connection.close()
    values = {"fixture_revision": meta[0], "residents": meta[1], "day": meta[2], "minute": meta[3],
              "save_version": version[0]}
    values["key"] = hashlib.sha256(json.dumps(values, sort_keys=True).encode()).hexdigest()[:16]
    return values


def sweep_temporary(cache):
    """Removes snapshots a killed prepare left half-written (`.<name>.<pid>.tmp.sqlite`)."""
    removed = []
    for path in Path(cache).glob(".*.tmp.sqlite"):
        try:
            pid = int(path.name.split(".")[-3])
            os.kill(pid, 0)
            continue
        except ProcessLookupError:
            pass
        except (ValueError, PermissionError):
            continue
        path.unlink()
        removed.append(path.name)
    return removed


def file_state(path):
    """A prepare publishes by rename, so a regenerated snapshot is a new inode."""
    stat = Path(path).stat()
    return {"inode": stat.st_ino, "size": stat.st_size}


# ---------------------------------------------------------------------------------------------
# Running rows.

def base_env(session, row=None):
    env = {key: value for key, value in os.environ.items()
           if key not in ("DISPLAY", "WAYLAND_DISPLAY")}
    env.update(METRUM_DEBUG="0", RAYON_NUM_THREADS=str(session["workers"]))
    if row is not None and row["kind"] == "test":
        env.update(row["env"])
    return env


def child_affinity(session, row):
    cpus = session["cpus"]
    if not cpus or not hasattr(os, "sched_setaffinity"):
        return None
    chosen = {cpus[0]} if row is not None and row.get("cpus") == "one" else set(cpus)
    return lambda: os.sched_setaffinity(0, chosen)


def run_command(command, env, log_path, preexec=None, cwd=ROOT / "rust"):
    with open(log_path, "w") as log:
        return subprocess.run(command, cwd=cwd, env=env, stdout=log, stderr=subprocess.STDOUT,
                              preexec_fn=preexec).returncode


def tagged(text, tag):
    records = []
    for line in text.splitlines():
        if tag + " " in line:
            records.append(json.loads(line.split(tag + " ", 1)[1]))
    return records


def test_command(binary, name):
    return [binary, name, "--exact", "--ignored", "--nocapture", "--test-threads=1"]


def prepare_chains(session, side, out):
    """One side's prepares as independent chains: per tier the baseline, then its fiscal city
    (which starts from the baseline), and the K22 commit cities. A step returns a list of
    (fixture, tier, record)."""
    binary = session["binaries"][side]
    env = dict(base_env(session), METRUM_PERF_CACHE=session["caches"][side])
    needs = session["needs"][side]

    def economy(test, tag, fixture, tier, log):
        def step():
            records = gate.run_test(binary, test, dict(env, METRUM_PERF_RESIDENTS=str(tier)),
                                    out / log, (tag,))
            return [(fixture, str(tier), records[tag][0])]
        return step

    def commit():
        records = gate.run_test(binary, COMMIT_PREPARE_TEST,
                                dict(env, METRUM_COMMIT_TIERS=",".join(map(str, needs["commit"]))),
                                out / f"commit-prepare-{side}.log", ("COMMIT_PREPARE",))
        return [("commit", str(record["residents"]), record) for record in records["COMMIT_PREPARE"]]

    chains = []
    for tier in needs["baseline"]:
        chain = [economy(gate.PREPARE_TEST, "PERF_PREPARE", "baseline", tier,
                         f"prepare-{tier}-{side}.log")]
        if tier in needs["fiscal"]:
            chain.append(economy(FISCAL_PREPARE_TEST, "PERF_FISCAL_PREPARE", "fiscal", tier,
                                 f"fiscal-prepare-{tier}-{side}.log"))
        chains.append(chain)
    return chains + ([[commit]] if needs["commit"] else [])


def prepare_sides(session, out, parallel):
    """Runs every side's chains, concurrently when `parallel` (generation is outside the timed
    phase; two sides with one key may both generate it, and the last rename wins)."""
    for side in SIDES:
        Path(session["caches"][side]).mkdir(parents=True, exist_ok=True)
    jobs = [(side, chain) for side in SIDES for chain in prepare_chains(session, side, out)]

    def run(job):
        return job[0], [item for step in job[1] for item in step()]

    with concurrent.futures.ThreadPoolExecutor(max_workers=len(jobs) if parallel else 1) as pool:
        done = list(pool.map(run, jobs)) if jobs else []
    result = {side: {"baseline": {}, "fiscal": {}, "commit": {}} for side in SIDES}
    for side, items in done:
        for fixture, tier, record in items:
            result[side][fixture][tier] = record
    return result


def snapshot_files(session, side, prepared):
    """The snapshot file of every prepared fixture, with its metadata and file state."""
    cache = Path(session["caches"][side])
    files = {}
    for fixture, name in (("baseline", "populated_city"), ("fiscal", "fiscal_stress"),
                          ("commit", "commit_city")):
        for tier, record in prepared[fixture].items():
            path = cache / snapshot_name(name, tier, record.get("key"))
            entry = {"path": str(path), **file_state(path)}
            if fixture != "commit":
                entry["meta"] = snapshot_meta(path)
            files[f"{fixture}-{tier}"] = entry
    return files


def phase_prepare(session, path):
    """Before the locks' timed phase: prepare every side, then verify that a second pass
    generates nothing (two old builds with different keys would overwrite one unkeyed file)."""
    out = Path(session["artifacts"]) / "prepare"
    out.mkdir(exist_ok=True)
    started = time.monotonic()
    swept = {side: sweep_temporary(session["caches"][side]) for side in SIDES
             if Path(session["caches"][side]).exists()}
    session["prepare"] = prepare_sides(session, out, parallel=True)
    for side, names in swept.items():
        session["prepare"][side]["swept"] = names
    verify_dir = out / "verify"
    verify_dir.mkdir(exist_ok=True)
    problems = []
    verified = prepare_sides(session, verify_dir, parallel=False)
    for side in SIDES:
        again = verified[side]
        for fixture in ("baseline", "fiscal", "commit"):
            for tier, record in again[fixture].items():
                if record.get("generation_s") is not None:
                    problems.append(f"{side} {fixture} {tier} generated again in the verify pass: "
                                    "two binaries share an unkeyed snapshot; give each its own cache")
        session["prepare"][side]["files"] = snapshot_files(session, side, again)
    session["prepare_s"] = time.monotonic() - started
    session["prepare_problems"] = problems
    save_session(session, path)
    for problem in problems:
        print("PREPARE FAIL: " + problem, file=sys.stderr)
    return not problems


def check_snapshots(session, row):
    """Refuses a row whose snapshot changed or vanished after the prepare phase: the timed phase
    never generates."""
    files = session["prepare"][row["side"]]["files"]
    wanted = []
    if row["kind"] == "perf07":
        wanted = [f"baseline-{row['tier']}"]
    elif row["kind"] == "contract":
        wanted = [f"{fixture}-{tier}" for fixture in ("baseline", "fiscal") for tier in CONTRACT_TIERS]
    elif row["kind"] == "k22":
        wanted = [f"commit-{tier}" for tier in row["tiers"].split(",") if int(tier) > 0]
    for name in wanted:
        entry = files.get(name)
        if entry is None:
            return f"{name} was not prepared"
        if not Path(entry["path"]).exists():
            return f"{name} snapshot {entry['path']} is missing"
        if file_state(entry["path"]) != {"inode": entry["inode"], "size": entry["size"]}:
            return f"{name} snapshot {entry['path']} changed after the prepare phase"
        if "meta" in entry and snapshot_meta(entry["path"]) != entry["meta"]:
            return f"{name} snapshot {entry['path']} has another key than at the prepare phase"
    return None


def row_snapshot_key(session, row):
    """The baseline snapshot key of a row's tier. A cache-only row's tier was not prepared: its
    file, when an earlier session left it in the side's cache, is read under this binary's name."""
    side, tier = row["side"], row.get("tier") or CONTROL_TIER
    prepared = session["prepare"][side]
    entry = prepared.get("files", {}).get(f"baseline-{tier}")
    if entry is not None:
        return entry["meta"]["key"]
    records = list(prepared.get("baseline", {}).values())
    if not records:
        return None
    path = Path(session["caches"][side]) / snapshot_name("populated_city", tier, records[0].get("key"))
    try:
        return snapshot_meta(path)["key"]
    except (sqlite3.Error, TypeError):
        return None


def run_perf07(session, row, out):
    side, tier, speed = row["side"], row["tier"], row["speed"]
    directory = out / row["label"]
    directory.mkdir(exist_ok=True)
    prepare_log = Path(session["artifacts"]) / "prepare/verify" / f"prepare-{tier}-{side}.log"
    if prepare_log.exists():
        shutil.copy2(prepare_log, directory / f"prepare-{tier}.log")
    env = dict(base_env(session), METRUM_PERF_RESIDENTS=str(tier),
               METRUM_PERF_CACHE=session["caches"][side], METRUM_PERF07_SPEED=str(speed))
    log = directory / f"perf07-{tier}-{speed}x.log"
    command = test_command(session["binaries"][side], gate.PERF07_TEST)
    status = run_command(command, env, log, child_affinity(session, row))
    text = log.read_text()
    days = tagged(text, "PERF07_DAY")
    if status or "1 passed" not in text or not days:
        raise RuntimeError(f"PERF-07 {row['label']} failed; see {log}")
    day = days[0]
    accepted, failures = gate.perf07_acceptance([day])
    report = {"identity": row["identity"], "failures": failures, "notes": [], "perf07": [day],
              "perf07_table": gate.perf07_table([day]), "perf07_acceptance": accepted,
              "runtime_s": row.get("wall_s")}
    (directory / "report.json").write_text(json.dumps(report, indent=1) + "\n")
    (out / f"{row['label']}.log").write_text("\n".join(report["perf07_table"]) + "\n")
    return {"day": day}


BENCH_VALUE = re.compile(r"(\w+_(?:us|ms))=([0-9.]+)")
BENCH_JSON = re.compile(r"([A-Z][A-Z0-9_]+) (\{.*\})\s*$")


def bench_values(text):
    """Timings a benchmark prints, by name in output order: `name_us=1.5` pairs, and the median
    of a JSON report's `samples_ms`. Benchmarks print no common format; this is the summary's."""
    values = {}
    for line in text.splitlines():
        for name, value in BENCH_VALUE.findall(line):
            values.setdefault(name, []).append(float(value))
        match = BENCH_JSON.search(line)
        if match:
            try:
                record = json.loads(match.group(2))
            except json.JSONDecodeError:
                continue
            if isinstance(record.get("samples_ms"), list) and record["samples_ms"]:
                values.setdefault(f"{match.group(1)}.median_ms", []).append(
                    statistics.median(record["samples_ms"]))
    return values


def run_test_row(session, row, out):
    log = out / f"{row['label']}.log"
    status = run_command(test_command(session["binaries"][row["side"]], row["test"]),
                         base_env(session, row), log, child_affinity(session, row))
    text = log.read_text()
    if status or "1 passed" not in text:
        raise RuntimeError(f"{row['label']} failed; see {log}")
    return {"bench": bench_values(text)}


def run_k22(session, row, out):
    log = out / f"{row['label']}.log"
    env = dict(base_env(session), METRUM_PERF_CACHE=session["caches"][row["side"]],
               METRUM_COMMIT_TIERS=row["tiers"])
    status = run_command(test_command(session["binaries"][row["side"]], COMMIT_TEST), env, log,
                         child_affinity(session, row))
    text = log.read_text()
    if status or "1 passed" not in text:
        raise RuntimeError(f"{row['label']} failed; see {log}")
    generated = [fixture["residents"] for fixture in tagged(text, "COMMIT_FIXTURE")
                 if fixture.get("source") == "generated" and fixture["residents"] > 0]
    if generated:
        raise RuntimeError(f"{row['label']} generated commit cities {generated} in the timed phase")
    tiers = {}
    for record in tagged(text, "COMMIT_BREAKDOWN"):
        tiers.setdefault(str(record.get("residents", "?")), []).append(record)
    summary = {tier: {key: statistics.median(record[key] for record in records)
                      for key in ("click_to_ack_ms", "commit_ms") if key in records[0]}
               for tier, records in tiers.items()}
    return {"k22": summary}


def run_contract(session, row, out):
    directory = out / row["label"]
    directory.mkdir(exist_ok=True)
    link = directory / "snapshots"
    if not link.exists():
        link.symlink_to(session["caches"][row["side"]])
    command = [sys.executable, str(ROOT / "tools/perf_contract.py"), "--binary",
               session["binaries"][row["side"]], "--artifacts", str(directory)]
    if session.get("base"):
        command += ["--diff", session["base"]]
    status = run_command(command, base_env(session), out / f"{row['label']}.log",
                         child_affinity(session, row), cwd=ROOT)
    report_path = directory / "report.json"
    report = json.loads(report_path.read_text()) if report_path.exists() else {}
    generated = [record for record in report.get("records", []) + report.get("fiscal_prepare_records", [])
                 if "generation_s" in record and record["generation_s"] is not None]
    if generated:
        raise RuntimeError(f"{row['label']} generated snapshots in the timed phase")
    days = {str(record["residents"]): {key: record[key] for key in ("hourly_ms", "daily_ms")}
            for record in report.get("records", []) if "hourly_ms" in record}
    return {"contract": {"status": report.get("status", f"exit {status}"),
                         "failures": report.get("failures", []), "days": days}}


RUNNERS = {"perf07": run_perf07, "test": run_test_row, "k22": run_k22, "contract": run_contract}


# ---------------------------------------------------------------------------------------------
# The timed phase.

def row_cache_key(session, row, side=None):
    side = side or row["side"]
    benchmark = {"perf07": gate.PERF07_TEST, "k22": COMMIT_TEST,
                 "contract": "perf_contract"}.get(row["kind"], row.get("test"))
    env = base_env(session, row)
    return cache_key(session["machine"], session["binary_sha"][side], row_snapshot_key(session, row),
                     benchmark, row.get("tier") or row.get("tiers"), row.get("speed"),
                     env["RAYON_NUM_THREADS"], session["cpus"], env)


def precheck(session, label, out, wait_s):
    """Waits for a quiet machine and outside the quiet windows; False when it never came. The
    wait is added to the session's `held_s`, apart from the timed phase's working time."""
    started = time.monotonic()
    try:
        return wait_quiet(session, label, out, started + wait_s)
    finally:
        session["held_s"] = session.get("held_s", 0.0) + time.monotonic() - started


def wait_quiet(session, label, out, deadline):
    settings = session["settings"]
    names = watched_processes(session)
    while True:
        busy = busy_processes(names)
        window = in_windows(datetime.datetime.now(), settings.get("quiet_windows", []),
                            session["estimates"].get(label, 0))
        if not busy and not window:
            return True
        reason = f"busy: {', '.join(busy)}" if busy else f"quiet window {window}"
        with open(out / "holds.txt", "a") as holds:
            holds.write(f"{datetime.datetime.now():%H:%M:%S} {label} held, {reason}\n")
        if time.monotonic() >= deadline:
            return False
        time.sleep(20)


def watched_processes(session):
    return sorted(set(["godot", "cargo", "rustc"] + session["settings"].get("busy_processes", [])))


def machine_check(session, path):
    lines = [f"{datetime.datetime.now():%Y-%m-%d %H:%M:%S}",
             "load average " + " ".join(f"{value:.2f}" for value in os.getloadavg()),
             f"boot {boot_id()}"]
    for name in watched_processes(session):
        lines.append(f"{name}: {'running' if busy_processes([name]) else 'none'}")
    Path(path).write_text("\n".join(lines) + "\n")


def decide_reuse(session, row, cache):
    """Whether a `reuse` control row comes from the cache: its key must hit, and this session's
    first control 32x run must calibrate against the cached 32x row."""
    if row_snapshot_key(session, row) is None:
        return None, "no prepared snapshot to key a cached row"
    entry = cache.get(row_cache_key(session, row))
    if entry is None:
        return None, "no cached row of this key and boot"
    calibration = session.get("calibration")
    if calibration is None:
        calibration_row = perf07_row("ctl", CONTROL_TIER, 32)
        cached = cache.get(row_cache_key(session, calibration_row))
        runs = [result for result in session["results"].values()
                if result.get("side") == "ctl" and result.get("tier") == CONTROL_TIER
                and result.get("speed") == 32 and "day" in result]
        if cached is None or not runs:
            calibration = {"passed": False, "reasons": ["no cached or session control 32x row"]}
        else:
            passed, deltas, reasons = calibrate(runs[0]["day"], [r["day"] for r in cached["rows"]])
            calibration = {"passed": passed, "deltas": deltas, "reasons": reasons,
                           "run": runs[0]["label"], "cached_session": cached["session"],
                           "cached_recorded": cached["recorded"]}
        session["calibration"] = calibration
    if not calibration["passed"]:
        return None, "calibration failed: " + "; ".join(calibration["reasons"])
    return entry, None


def execute_row(session, row, out, cache):
    label = row["label"]
    if row.get("reuse"):
        entry, reason = decide_reuse(session, row, cache)
        if entry is not None:
            first = entry["rows"][0]
            session["results"][label] = {**first, "label": label, "reused_from": entry["session"],
                                         "reused_recorded": entry["recorded"]}
            log_line(out, f"{label} reused from {entry['session']} {entry['recorded'][0]}")
            return
        if row["reuse"] == "only":
            session["results"][label] = {"label": label, "side": row["side"], "skipped": reason}
            log_line(out, f"{label} skipped: {reason}")
            return
        log_line(out, f"{label} timed: {reason}")
    if row.get("second"):
        reason = second_run_reason(session, row)
        if reason is None:
            session["results"][label] = {"label": label, "side": row["side"], "skipped": "not needed"}
            return
        log_line(out, f"{label} timed: {reason}")
    refusal = None
    if hash_of(session, row["side"]) != session["binary_sha"][row["side"]]:
        refusal = "binary changed since the plan"
    else:
        refusal = check_snapshots(session, row)
    result = {"label": label, "side": row["side"], "kind": row["kind"], "tier": row.get("tier"),
              "speed": row.get("speed")}
    if refusal:
        result["refused"] = refusal
        session["results"][label] = result
        log_line(out, f"{label} REFUSED: {refusal}")
        return
    row["identity"] = {"binary_sha256": session["binary_sha"][row["side"]],
                       "snapshot_key": row_snapshot_key(session, row), "boot_id": session["boot"],
                       "cpu_affinity": session["cpus"], "workers": session["workers"]}
    started = time.monotonic()
    result["start"] = f"{datetime.datetime.now():%H:%M:%S}"
    try:
        result.update(RUNNERS[row["kind"]](session, row, out))
        result["exit"] = 0
    except (RuntimeError, OSError, ValueError) as error:
        result["error"] = str(error)
        result["exit"] = 1
    result["wall_s"] = time.monotonic() - started
    result["end"] = f"{datetime.datetime.now():%H:%M:%S}"
    result["identity"] = row["identity"]
    session["results"][label] = result
    log_line(out, f"{label} exit={result['exit']} wall={result['wall_s']:.1f}s")


def second_run_reason(session, row):
    first = [result for result in session["results"].values()
             if result.get("side") == "br" and result.get("tier") == row["tier"]
             and result.get("speed") == row["speed"] and "day" in result]
    ctl = [result["day"] for result in session["results"].values()
           if result.get("side") == "ctl" and result.get("tier") == row["tier"]
           and result.get("speed") == row["speed"] and "day" in result]
    return wants_second_run(row["tier"], row["speed"], first[0]["day"], ctl) if first else None


_HASHES = {}


def hash_of(session, side):
    path = session["binaries"][side]
    if path not in _HASHES:
        _HASHES[path] = file_sha256(path)
    return _HASHES[path]


def log_line(out, text):
    line = f"{datetime.datetime.now():%H:%M:%S} {text}"
    print(line, flush=True)
    with open(out / "summary.txt", "a") as summary:
        summary.write(line + "\n")


def phase_timed(session, path, labels=None):
    out = Path(session["artifacts"])
    cache = ControlCache(session["cache_root"], session["boot"])
    if boot_id() != session["boot"]:
        raise RuntimeError("the machine rebooted after the plan; plan again")
    _HASHES.clear()
    for side in SIDES:
        if hash_of(session, side) != session["binary_sha"][side]:
            raise RuntimeError(f"{side} binary SHA-256 is not the plan's; refusing the session")
    if session["cpus"] and hasattr(os, "sched_setaffinity"):
        os.sched_setaffinity(0, set(session["cpus"]))
    machine_check(session, out / "precheck.txt")
    started = time.monotonic()
    session.setdefault("results", {})
    for row in session["rows"]:
        if labels is not None and row["label"] not in labels:
            continue
        needs_run = not (row.get("reuse") == "only")
        if needs_run and not precheck(session, row["label"], out, session["wait_s"]):
            session["results"][row["label"]] = {"label": row["label"], "side": row["side"],
                                                "refused": "machine never became quiet"}
            log_line(out, f"{row['label']} REFUSED: machine never became quiet")
            continue
        execute_row(session, row, out, cache)
        save_session(session, path)
    session["timed_s"] = session.get("timed_s", 0.0) + time.monotonic() - started
    machine_check(session, out / "postcheck.txt")
    # A rerun whose rows were all refused or failed leaves the cache as it was.
    timed = [row["label"] for row in session["rows"]
             if (labels is None or row["label"] in labels)
             and session["results"].get(row["label"], {}).get("exit") == 0
             and not session["results"][row["label"]].get("reused_from")]
    if timed:
        update_cache(session, cache)
    save_session(session, path)


def update_cache(session, cache):
    """Caches this session's timed rows by key, except rows recorded in a no-cache window. A
    binary's cached 1x and 32x rows of a tier stay from one session: an existing entry is
    replaced only by a session that timed that binary's 1x day of the tier itself."""
    def timed(row):
        result = session["results"].get(row["label"], {})
        return result.get("exit") == 0 and not result.get("reused_from")

    fresh_days = {(row["side"], row["tier"]) for row in session["rows"]
                  if row["kind"] == "perf07" and row["speed"] == 1 and timed(row)}
    groups = {}
    for row in session["rows"]:
        result = session["results"].get(row["label"], {})
        if not timed(row) or row["kind"] == "test":
            continue
        if (row["side"], row.get("tier")) not in fresh_days \
                and cache.get(row_cache_key(session, row)) is not None:
            continue
        start = datetime.datetime.combine(datetime.date.today(),
                                          datetime.time.fromisoformat(result["start"]))
        if in_windows(start, session["settings"].get("no_cache_windows", []), result["wall_s"]):
            continue
        key = row_cache_key(session, row)
        groups.setdefault(key_hash(key), (key, []))[1].append(result)
    for key, results in groups.values():
        cache.put(key, results, session["artifacts_label"], [result["start"] for result in results])


# ---------------------------------------------------------------------------------------------
# Summary.

def perf07_groups(session):
    groups = {}
    for row in session["rows"]:
        result = session["results"].get(row["label"], {})
        if row["kind"] == "perf07" and "day" in result:
            groups.setdefault((row["tier"], row["speed"]), {"ctl": [], "br": []})[row["side"]].append(result)
    return groups


def summary_lines(session):
    lines = [f"Timing session {session['artifacts_label']}: level {session['level']}"
             f"{' (merge)' if session.get('merge') else ''}; path rule {session['path_level'] or 'none'}"]
    if session.get("level_warning"):
        lines.append("WARNING: " + session["level_warning"])
    lines.append(f"boot {session['boot'][:8]}, cpus {session['cpus'] or 'unpinned'}, "
                 f"workers {session['workers']}")
    for side in SIDES:
        files = session.get("prepare", {}).get(side, {}).get("files", {})
        keys = ", ".join(f"{name} {entry['meta']['key']}" for name, entry in files.items()
                         if "meta" in entry)
        lines.append(f"{side} {session['binary_sha'][side][:16]}; snapshot keys: {keys or '–'}")
    if "timed_s" in session:
        working = session["timed_s"] - session.get("held_s", 0)
        lines.append(f"prepare {session.get('prepare_s', 0):.0f} s; timed phase {working:.0f} s "
                     f"({working / 60:.1f} min) plus {session.get('held_s', 0):.0f} s held for a "
                     "quiet machine")
    else:
        lines.append(f"prepare {session.get('prepare_s', 0):.0f} s; timed phase running")
    for side in SIDES:
        prepared = session.get("prepare", {}).get(side, {})
        generated = [f"{fixture} {tier}" for fixture in ("baseline", "fiscal", "commit")
                     for tier, record in prepared.get(fixture, {}).items()
                     if record.get("generation_s") is not None]
        if generated:
            lines.append(f"{side} prepare generated: {', '.join(generated)} (outside the locks)")
    calibration = session.get("calibration")
    if calibration:
        if calibration.get("deltas"):
            deltas = calibration["deltas"]
            lines.append(f"calibration {'passed' if calibration['passed'] else 'FAILED'}: "
                         f"{calibration['run']} against {calibration['cached_session']} "
                         f"{calibration['cached_recorded'][0]}: {deltas['mean_ms']:+.1f}% mean / "
                         f"{deltas['p99_ms']:+.1f}% p99 / {deltas['p999_ms']:+.1f}% p99.9"
                         + ("" if calibration["passed"] else "; " + "; ".join(calibration["reasons"])))
        else:
            lines.append("calibration not possible: " + "; ".join(calibration["reasons"]))
    lines.append("")
    lines.append("| row | start | wall s | result |")
    lines.append("| --- | --- | --- | --- |")
    for row in session["rows"]:
        result = session["results"].get(row["label"], {})
        if result.get("reused_from"):
            status = f"reused from {result['reused_from']} {result['reused_recorded'][0]}"
        elif "skipped" in result:
            status = "skipped: " + result["skipped"]
        elif "refused" in result:
            status = "REFUSED: " + result["refused"]
        elif "error" in result:
            status = "FAILED: " + result["error"]
        elif not result:
            status = "not run"
        else:
            status = row_status(result)
        lines.append(f"| {row['label']} | {result.get('start', '')} | "
                     f"{result.get('wall_s', 0):.0f} | {status} |")
    lines += bench_lines(session)
    for (tier, speed), sides in sorted(perf07_groups(session).items()):
        lines += [""] + comparison_lines(tier, speed, sides)
    return lines


def bench_lines(session):
    """Per benchmark and printed value: each side's median over rounds and the ratio br/ctl.
    Values are matched by position; no tolerance is known for them, so no verdict."""
    table = {}
    for row in session["rows"]:
        result = session["results"].get(row["label"], {})
        for name, values in result.get("bench", {}).items():
            for index, value in enumerate(values):
                key = (row["name"], f"{name}[{index}]" if len(values) > 1 else name)
                table.setdefault(key, {"ctl": [], "br": []})[row["side"]].append(value)
    if not table:
        return []
    lines = ["", "| benchmark | value | ctl median | br median | br/ctl | rounds |",
             "| --- | --- | --- | --- | --- | --- |"]
    for (bench, name), sides in table.items():
        if not sides["ctl"] or not sides["br"]:
            continue
        ctl, br = statistics.median(sides["ctl"]), statistics.median(sides["br"])
        ratio = f"{br / ctl:.3f}" if ctl else "–"
        lines.append(f"| {bench} | {name} | {ctl:.4g} | {br:.4g} | {ratio} | "
                     f"{len(sides['ctl'])}/{len(sides['br'])} |")
    return lines


def row_status(result):
    if "day" in result:
        day = result["day"]
        return (f"mean {day['mean_ms']:.3f}, p99 {day['p99_ms']:.2f}, p99.9 {day['p999_ms']:.2f}, "
                f"max unpreempted {day.get('max_unpreempted_ms', 0):.2f}, preempted "
                f"{day.get('preempted_frames', '–')}")
    if "k22" in result:
        return "; ".join(f"{tier}: click-to-ack {values.get('click_to_ack_ms', 0):.2f} ms, commit "
                         f"{values.get('commit_ms', 0):.2f} ms" for tier, values in result["k22"].items())
    if "bench" in result:
        return f"ok, {sum(len(values) for values in result['bench'].values())} values"
    if "contract" in result:
        contract = result["contract"]
        days = "; ".join(f"{tier}: hour {values['hourly_ms']:.1f} / day {values['daily_ms']:.1f} ms"
                         for tier, values in contract["days"].items())
        return f"{contract['status']}; {days}"
    return "ok"


def comparison_lines(tier, speed, sides):
    rows = sides["ctl"] + sides["br"]
    noisy = noisy_rows(rows)
    lines = [f"PERF-07 {tier // 1000}k {speed}x: ctl n={len(sides['ctl'])}, br n={len(sides['br'])}"
             + (f"; NOISY rows {', '.join(noisy)} (>{NOISY_RATIO:g}x the quietest row's preempted "
                f"frames; tolerances doubled; rerun with --rerun)" if noisy else "")]
    accepted = [(result["label"], gate.perf07_acceptance([result["day"]])) for result in rows]
    covered = (tier, speed) in gate.PERF07_ACCEPTED_TIERS
    for label, (values, failures) in accepted:
        if values:
            shown = ", ".join(f"{values[0][key]:.2f}" if values[0][key] is not None else "–"
                              for key in gate.PERF07_ACCEPTANCE_KEYS)
            known = KNOWN_EXCEPTIONS.get((tier, speed))
            verdict_text = "PASS" if values[0]["pass"] else \
                f"known exception ({known})" if known else "FAIL"
            lines.append(f"  acceptance {label} (unpreempted, p99.9, preempted less wait): {shown}: "
                         f"{verdict_text}")
    if not covered:
        lines.append("  (not an acceptance tier)")
    if not sides["ctl"] or not sides["br"]:
        return lines
    lines.append("| metric | ctl | br | delta | tolerance | verdict |")
    lines.append("| --- | --- | --- | --- | --- | --- |")
    for metric in SIGMA_PCT:
        ctl = [result["day"][metric] for result in sides["ctl"] if metric in result["day"]]
        br = [result["day"][metric] for result in sides["br"] if metric in result["day"]]
        if not ctl or not br:
            continue
        delta, tol, result = verdict(metric, ctl, br, bool(noisy))
        lines.append(f"| {metric} | {statistics.fmean(ctl):.4g} | {statistics.fmean(br):.4g} | "
                     f"{delta:+.1f}% | {tol:.1f}% | {result.upper() if result == 'worse' else result} |")
    for metric in REPORTED:
        ctl = [result["day"][metric] for result in sides["ctl"] if metric in result["day"]]
        br = [result["day"][metric] for result in sides["br"] if metric in result["day"]]
        if ctl and br:
            lines.append(f"| {metric} | {statistics.fmean(ctl):.4g} | {statistics.fmean(br):.4g} | "
                         f"| | reported |")
    return lines


def worse_metrics(session):
    worse = []
    for (tier, speed), sides in perf07_groups(session).items():
        if not sides["ctl"] or not sides["br"]:
            continue
        noisy = bool(noisy_rows(sides["ctl"] + sides["br"]))
        for metric in SIGMA_PCT:
            ctl = [r["day"][metric] for r in sides["ctl"] if metric in r["day"]]
            br = [r["day"][metric] for r in sides["br"] if metric in r["day"]]
            if ctl and br and verdict(metric, ctl, br, noisy)[2] == "worse":
                worse.append(f"{tier} {speed}x {metric}")
    return worse


# ---------------------------------------------------------------------------------------------
# Command line.

def save_session(session, path):
    temporary = Path(str(path) + ".tmp")
    temporary.write_text(json.dumps(session, indent=1, default=sorted) + "\n")
    temporary.replace(path)


def build_session(args, settings):
    out = args.artifacts.resolve()
    if (out / "session.json").exists():
        raise SystemExit(f"{out} already holds a session; use --rerun or a new directory")
    if args.head:
        paths = subprocess.check_output(["git", "diff", "--name-only", args.base, args.head],
                                        cwd=ROOT, text=True).split()
    else:
        paths = changed_paths(args.base) if args.base else []
    path_level, reasons, edit, save = classify(paths)
    level = args.level or path_level or "R"
    warning = None
    if args.level and path_level and LEVELS.index(path_level) > LEVELS.index(args.level):
        warning = f"the brief says {args.level}, the path rule says {path_level}"
    elif args.level and args.base and path_level is None:
        warning = f"the brief says {args.level}; the path rule finds no timed path"
    benches = [parse_bench(spec) for spec in args.bench]
    rows = plan_rows(level, args.merge, benches, k22=args.k22 or edit,
                     contract=args.contract,
                     k22_tiers=args.k22_tiers,
                     perf07=[row for spec in args.perf07 for row in parse_perf07(spec)] or None)
    cpus = [int(cpu) for cpu in (args.cpus or settings.get("cpus") or "").split(",") if cpu]
    binaries = {"ctl": str(args.control.resolve()), "br": str(args.branch.resolve())}
    cache_default = main_target() / "timing-cache/snapshots"
    caches = {"ctl": str((args.control_cache or cache_default).resolve()),
              "br": str((args.branch_cache or cache_default).resolve())}
    boot = boot_id()
    machine = {"boot_id": boot, "kernel": platform.release(), "power": power_state(cpus),
               # The gate's acceptance code (review Q4); the runner's own environment for a
               # row is in the key explicitly.
               "tools": hashlib.sha256((ROOT / "tools/sim02_gate.py").read_bytes()).hexdigest()[:16]}
    session = {
        "artifacts": str(out), "artifacts_label": out.name, "level": level, "merge": args.merge,
        "path_level": path_level, "path_reasons": reasons, "level_warning": warning,
        "base": args.base, "rows": rows, "binaries": binaries, "caches": caches,
        "binary_sha": {side: file_sha256(path) for side, path in binaries.items()},
        "cpus": cpus, "workers": args.workers, "boot": boot, "machine": machine,
        "cache_root": str(main_target() / "timing-cache/rows"), "settings": settings,
        "wait_s": args.wait_min * 60, "results": {},
        "needs": {side: prepare_needs(rows, side) for side in SIDES},
        "estimates": {row["label"]: estimate_s(row) for row in rows},
        "planned": f"{datetime.datetime.now():%Y-%m-%d %H:%M:%S}",
    }
    if cpus and not hasattr(os, "sched_setaffinity"):
        session["notes"] = ["CPU pinning is not available on this OS; rows run unpinned"]
        session["cpus"] = []
    return session


def plan_lines(session):
    lines = [f"level {session['level']}{' (merge)' if session['merge'] else ''}; path rule: "
             f"{session['path_level'] or 'no timed path'}"]
    lines += ["  " + reason for reason in session["path_reasons"]]
    if session["level_warning"]:
        lines.append("WARNING: " + session["level_warning"])
    for side in SIDES:
        lines.append(f"{side}: {session['binaries'][side]} ({session['binary_sha'][side][:16]}), "
                     f"cache {session['caches'][side]}, prepare {session['needs'][side]}")
    total = 0
    for row in session["rows"]:
        estimate = session["estimates"][row["label"]]
        note = " (cache only)" if row.get("reuse") == "only" else " (cached under calibration)" \
            if row.get("reuse") else " (only when needed)" if row.get("second") else ""
        if not row.get("reuse") and not row.get("second"):
            total += estimate
        lines.append(f"  {row['label']}: ~{estimate} s{note}")
    lines.append(f"estimated timed phase at least {total / 60:.1f} min")
    return lines


def run_phase_child(phase, session_path, settings):
    """Runs a phase in a child that holds the phase's locks (local `*_wrapper` commands)."""
    wrapper = settings.get(f"{phase}_wrapper") or []
    command = [*wrapper, sys.executable, str(Path(__file__).resolve()), "--phase", phase,
               "--session", str(session_path)]
    return subprocess.run(command).returncode


def finish(session, session_path):
    lines = summary_lines(session)
    with open(Path(session["artifacts"]) / "summary.txt", "a") as summary:
        summary.write("\n" + "\n".join(lines) + "\n")
    print("\n".join(lines))
    failed = [label for label, result in session["results"].items()
              if "#" not in label and ("refused" in result or "error" in result)]
    worse = worse_metrics(session)
    missed = [f"{label} {', '.join(misses)}" for label, result in session["results"].items()
              if "#" not in label and result.get("side") == "br" and "day" in result
              for misses in [acceptance_misses(result["day"])[0]] if misses]
    if failed:
        print("FAIL: rows " + ", ".join(failed), file=sys.stderr)
        return 1
    if missed:
        print("ACCEPTANCE: " + "; ".join(missed), file=sys.stderr)
        return 2
    if worse:
        print("WORSE: " + ", ".join(worse), file=sys.stderr)
        return 3
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--level", choices=LEVELS, help="the brief's level; default: the path rule's")
    parser.add_argument("--merge", action="store_true", help="L in merge form: the control timed fresh")
    parser.add_argument("--base", help="classify `git diff --name-only BASE` by the path rule")
    parser.add_argument("--head", help="with --base: classify BASE..HEAD instead of the work tree")
    parser.add_argument("--perf07", action="append", default=[],
                        help="custom PERF-07 rows TIER:SPEEDS:ORDER, e.g. 30000:1+32:ctl,br,br,ctl")
    parser.add_argument("--control", type=Path, help="control release library test executable")
    parser.add_argument("--branch", type=Path, help="branch release library test executable")
    parser.add_argument("--control-cache", type=Path, help="control snapshot cache directory")
    parser.add_argument("--branch-cache", type=Path, help="branch snapshot cache directory")
    parser.add_argument("--artifacts", type=Path, help="session directory (new, or existing for --rerun)")
    parser.add_argument("--bench", action="append", default=[],
                        help="the change's own benchmark: NAME[;KEY=VALUE...][;cpus=one]")
    parser.add_argument("--k22", action="store_true", help="add the K22 commit rows")
    parser.add_argument("--k22-tiers", help="METRUM_COMMIT_TIERS for the K22 rows")
    parser.add_argument("--contract", action="store_true", help="add the economy contract rows")
    parser.add_argument("--cpus", help="CPUs to pin to; default: the local settings' list")
    parser.add_argument("--workers", type=int, default=8)
    parser.add_argument("--wait-min", type=float, default=30.0,
                        help="minutes to wait for a quiet machine before refusing a row")
    parser.add_argument("--dry-run", action="store_true", help="print the plan only")
    parser.add_argument("--rerun", help="comma list of row labels to time again in --artifacts")
    parser.add_argument("--summary", action="store_true",
                        help="print the summary of the session in --artifacts; runs nothing")
    parser.add_argument("--phase", choices=("prepare", "timed"), help=argparse.SUPPRESS)
    parser.add_argument("--session", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()

    if args.phase:
        session = json.loads(args.session.read_text())
        if args.phase == "prepare":
            return 0 if phase_prepare(session, args.session) else 1
        phase_timed(session, args.session, session.get("rerun_labels"))
        return 0

    if args.summary:
        session = json.loads((args.artifacts.resolve() / "session.json").read_text())
        print("\n".join(summary_lines(session)))
        return 0
    settings = load_settings()
    if args.rerun:
        session_path = args.artifacts.resolve() / "session.json"
        session = json.loads(session_path.read_text())
        labels = args.rerun.split(",")
        for label in labels:
            if label in session["results"]:
                session["results"][label + "#1"] = session["results"].pop(label)
        session["rerun_labels"] = labels
        save_session(session, session_path)
        status = run_phase_child("timed", session_path, session["settings"])
        session = json.loads(session_path.read_text())
        session.pop("rerun_labels", None)
        return status or finish(session, session_path)

    if not (args.control and args.branch and args.artifacts):
        parser.error("--control, --branch and --artifacts are required")
    session = build_session(args, settings)
    print("\n".join(plan_lines(session)))
    if args.dry_run:
        return 0
    out = Path(session["artifacts"])
    out.mkdir(parents=True, exist_ok=True)
    session_path = out / "session.json"
    expired = ControlCache(session["cache_root"], session["boot"]).expire_other_boots()
    if expired:
        print(f"expired cached rows of {len(expired)} earlier boot(s)")
    (out / "binaries.sha256").write_text("".join(
        f"{session['binary_sha'][side]}  {session['binaries'][side]}\n" for side in SIDES))
    (out / "plan.txt").write_text("\n".join(plan_lines(session)) + "\n")
    save_session(session, session_path)
    if run_phase_child("prepare", session_path, settings):
        print("FAIL: prepare phase; nothing timed", file=sys.stderr)
        return 1
    status = run_phase_child("timed", session_path, settings)
    session = json.loads(session_path.read_text())
    return status or finish(session, session_path)


if __name__ == "__main__":
    sys.exit(main())
