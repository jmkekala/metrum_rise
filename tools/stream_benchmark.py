#!/usr/bin/env python3
"""The rendered camera-move benchmark (`PERF-23`, docs/terrain.md "Streaming core") and its verdict
tools (`TOOLS-20`).

`run` launches Godot's `stream` benchmark matrix (godot/scripts/benchmarks/stream_frame_benchmark.gd)
in gamescope's headless backend at 1000 Hz, in an isolated profile (`godot_isolated.py`), pinned to
the timing CPUs with eight workers, under the timing locks and outside the quiet windows of the
local timing settings (`timing_session.py`'s). `series` runs a C B B C quartet the same way in one
hold of the locks, the process check, start and load taken inside it before each row. `--library`
and `--scripts` run another build's library and scripts on this tree's assets: a control is
`--library <control .so> --scripts <its godot/scripts>`, with this tree's benchmark scripts copied
over its own. `summary` prints a report's per-phase table; with `--scripts` (an instrumented run,
`--detail`) also each phase's mean milliseconds per frame by renderer. `compare` gives a series'
verdict by same-binary spread, `digests` compares two runs' settle points (`equal`, `band`,
`different`), and `stills` captures the idle matrix's screenshots and compares them (byte identity,
the systematic-pixel test, a capture's variants against its `full`). `--shadows` sets the sun-shadow
variant sequence (`RENDER-15`, METRUM_STREAM_SHADOWS) of a run or a series' branch rows, and
`split` turns a run's `full,X,X,full` blocks into per-variant quartets for `compare`.
`--gpu-profile` adds Godot's render timestamps (`gpu_passes` per sample, shown by `summary`).
`--phases census` (untimed) writes the draw census of a few poses into `<out>/<label>.census`, and
`census` prints its tables (`RENDER-44`, `tools/draw_census.py`).
`--drawn` runs the drawn probe (METRUM_STREAM_DRAWN, `RENDER-32-N`: an instrumented run) and
`drawn` prints its table: per sample and layer, entered-to-drawn and in-view holes in frames.
`flight` prints a flight run's terrain streaming (`--phases flight_low,flight_high,flight_jump`,
`TERRAIN-41`): per sample the builds a frame, the backlog over time and, instrumented, one build's
split, the payload workers' time and the install ledger by owner.
Every row names its display driver: `--display-driver x11` (the default, the rows on record) or
`wayland` (`RENDER-62`, analysis A8: gamescope exposes its Wayland socket). Godot 4.7's Wayland
driver can hang after its window closes (godot#123059): a Wayland row whose Godot is still there
20 s after its report fails as an exit hang, its Godot stopped. Stills always run on x11.
`--as-played` (`RENDER-27`, analysis A-BG) runs a row as a player would: unpinned, the game's
default pools (`RAYON_NUM_THREADS` unset). `--cpus <list or profile>` runs the whole game inside a
CPU set (a machine profile; profiles are named in the local settings' `cpu_profiles`, since CPU
numbering is the machine's), with eight workers unless `--as-played`. Both rows sample Godot's main
thread from outside every 10 ms on Linux (`/proc/<pid>/task/<tid>/schedstat`, `sched`, `stat` and
its CPU's `scaling_cur_freq`) and add to their report per phase: on-CPU and run-queue wait ms and
timeslices a frame, CPU changes a second, the clock of the CPU it ran on and the E-cores' share
(`main_thread_sched`, printed by `summary`), with the placement and cpufreq policy (`row_mode`).

    ./run.sh --benchmark-stream                     # builds release, then `run` on Kuopio
    tools/stream_benchmark.py run --label c30k-br1 --save <populated_city_30000.sqlite> --out <dir>
    tools/stream_benchmark.py series --input-label 30k --ctl-library <.so> --ctl-scripts <dir> --out <dir> ...
    tools/stream_benchmark.py compare <dir> 30k kuopio
    tools/stream_benchmark.py digests <a.metrics.json> <b.metrics.json>
    tools/stream_benchmark.py stills capture|compare|systematic ...
    tools/stream_benchmark.py summary <dir>/c30k-br1.metrics.json [--scripts]
    tools/stream_benchmark.py run --label 30k-asplayed --as-played --phases still,turns --turns 1,0.5 ...
    tools/stream_benchmark.py run --label 30k-2p --as-played --cpus 2p-smt ...    # a machine profile
    tools/stream_benchmark.py run --label 30k-drawn --drawn --phases turns,zoom --turns 0.5 ... ; \
        tools/stream_benchmark.py drawn <dir>/30k-drawn.metrics.json
    tools/stream_benchmark.py run --label 30k-shadows --shadows full,none,none,full ... ; \
        tools/stream_benchmark.py split <dir>/30k-shadows.metrics.json; tools/stream_benchmark.py compare <dir> 30k-shadows.none
"""

import argparse
import datetime
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import signal
import statistics
import subprocess
import sys
import threading
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
import godot_isolated  # noqa: E402  (the isolated profile, TOOLS-19)
import timing_session  # noqa: E402  (shares the machine settings and window checks)

ROOT = Path(__file__).resolve().parent.parent
# The benchmark's own scripts: copied over another build's scripts so both sides measure alike.
BENCH_SCRIPTS = ["benchmarks/stream_frame_benchmark.gd", "benchmarks/gameplay_road_benchmark.gd",
                 "core/perf_debug.gd", "benchmarks/shadow_variants.gd", "benchmarks/draw_census.gd",
                 "benchmarks/pipeline_inventory.gd"]
WORKERS = "8"
# Set in the child a run or series starts under the timed locks.
LOCKED_ENV = "METRUM_STREAM_LOCKED"
# A series' rows in run order: control, branch, branch, control.
SERIES = [("ctl", 1), ("br", 1), ("br", 2), ("ctl", 2)]
# Seconds a Wayland row's Godot may stay after writing its report before the row fails as an
# exit hang (`RENDER-62`; a clean quit takes well under one).
EXIT_HANG_S = 20


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def player_mods():
    return godot_isolated.player_mods()


def make_project(profile, godot_dir, library, scripts, mods):
    """An isolated copy of the project (`godot_isolated.make_project`): the library copied,
    another build's scripts copied with this tree's benchmark scripts over them, and a copy of the
    packs at `mods` (if any) in the profile, so the run never writes the player's."""
    return godot_isolated.make_project(profile, godot_dir, library, scripts, mods, "MetrumStream", BENCH_SCRIPTS)


def watched_processes(settings):
    """Processes that hold a timed row: builds, Godot (the user playing) and the local busy ones."""
    return sorted(set(["godot", "cargo", "rustc"] + settings.get("busy_processes", [])))


def wait_for_quiet(settings, limit_s):
    """Waits while a quiet window is near or a watched process runs; the run is one row of at most
    `limit_s`. Called inside the timing locks, right before each row."""
    names = watched_processes(settings)
    while True:
        window = timing_session.in_windows(datetime.datetime.now(), settings.get("quiet_windows", []),
                                           limit_s)
        busy = timing_session.busy_processes(names)
        if not window and not busy:
            return
        print(f"stream_benchmark: waiting ({window or ''} {' '.join(busy)})", flush=True)
        time.sleep(30)


def rows_of(args):
    """The rows a `run` or `series` makes: `(label, library, scripts, shadows)` in run order. A
    series is C B B C: control, branch, branch, control, labelled `<input>-ctl1`, `-br1`, `-br2`,
    `-ctl2`; `--shadows` applies to its branch rows only."""
    godot_dir = Path(args.godot_dir).resolve()
    library = Path(args.library or godot_dir / "bin/libmetrum_rise.so").resolve()
    scripts = Path(args.scripts).resolve() if args.scripts else None
    if args.command == "run":
        return [(args.label, library, scripts, args.shadows, args.label)]
    control = (Path(args.ctl_library).resolve(), Path(args.ctl_scripts).resolve() if args.ctl_scripts else None, "")
    sides = {"ctl": control, "br": (library, scripts, args.shadows)}
    return [(f"{args.input_label}-{side}{n}", *sides[side], side) for side, n in SERIES]


def run(args):
    """`run` and `series`: every row of the command inside one hold of the local timed locks.
    Outside them the command runs itself again under `timed_wrapper`, so the process check, the
    recorded start and load, and every row of a quartet fall inside the locked window."""
    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    rows = rows_of(args)
    for label, *_ in rows:
        if (out / f"{label}.metrics.json").exists():
            print(f"stream_benchmark: {out / label}.metrics.json exists; choose a new label", file=sys.stderr)
            return 2
    settings = timing_session.load_settings()
    row_placement(args, settings)  # an unknown --cpus refuses before any lock
    wrapper = list(settings.get("timed_wrapper", []))
    if wrapper and os.environ.get(LOCKED_ENV) != "1":
        env = dict(os.environ, **{LOCKED_ENV: "1"})
        return subprocess.run(wrapper + [sys.executable, str(Path(__file__).resolve())] + args.argv,
                              env=env).returncode
    status = 0
    for label, library, scripts, shadows, cache_key in rows:
        status = max(status, run_row(args, settings, out, label, library, scripts, shadows, cache_key))
    return status


# The shader and pipeline caches a run keeps across runs (`RENDER-43`): Godot's shader cache and
# Vulkan pipeline cache in user://, and the cache home (the driver's own shader cache).
USER_CACHES = ["shader_cache", "vulkan"]


def copy_caches(profile, name, store, seed):
    """`--shader-cache`: with `seed`, copies the caches kept in `store` into a new profile's user://
    `name` and cache home; otherwise saves the profile's caches into `store`, replacing its own."""
    places = [(godot_isolated.user_dir(profile, name) / entry, store / entry) for entry in USER_CACHES]
    places.append((Path(profile) / godot_isolated.XDG_HOMES["XDG_CACHE_HOME"], store / "cache_home"))
    for live, kept in places:
        source, target = (kept, live) if seed else (live, kept)
        if not source.is_dir():
            continue
        shutil.rmtree(target, ignore_errors=True)
        shutil.copytree(source, target, symlinks=True)


def godot_of_row(root_pid, log_file):
    """The row's Godot: `root_pid` (the row's own child) or a process under it whose command line
    holds the row's `--log-file`, unique to the row. None when it is gone."""
    parents, godots = {}, []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            parents[int(entry.name)] = int((entry / "stat").read_text().rsplit(")", 1)[1].split()[1])
            argv = (entry / "cmdline").read_bytes().split(b"\0")
        except OSError:
            continue
        if Path(argv[0].decode(errors="replace")).name == "godot" and log_file.encode() in argv:
            godots.append(int(entry.name))
    for pid in godots:
        ancestor = pid
        while ancestor in parents and ancestor > 1:
            if ancestor == root_pid:
                return pid
            ancestor = parents[ancestor]
    return None


def run_watched(command, env, log, metrics, log_file):
    """Runs a Wayland row's command; returns `(status, exit_hang)`. Once the report is written,
    a Godot still there EXIT_HANG_S later is stopped (godot#123059: it would never end)."""
    child = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT)
    written = None
    while True:
        try:
            return child.wait(timeout=1), False
        except subprocess.TimeoutExpired:
            pass
        if written is None and metrics.exists():
            written = time.monotonic()
        if written is not None and time.monotonic() - written > EXIT_HANG_S:
            pid = godot_of_row(child.pid, log_file)
            if pid is not None:
                os.kill(pid, signal.SIGKILL)
            else:
                child.terminate()  # `timeout` passes it on to gamescope
            return child.wait(), True


def run_row(args, settings, out, label, library, scripts, shadows, cache_key):
    godot_dir = Path(args.godot_dir).resolve()
    metrics = out / f"{label}.metrics.json"
    save = str(Path(args.save).resolve()) if args.save else ""
    world = str(Path(args.world).resolve())
    profile, project, name, env = godot_isolated.create(
        out, "stream.", godot_dir=godot_dir, library=library, scripts=scripts, mods=player_mods(),
        name_prefix="MetrumStream", own_scripts=BENCH_SCRIPTS)
    store = Path(args.shader_cache).resolve() / cache_key if args.shader_cache else None
    try:
        if store is not None:
            copy_caches(profile, name, store, seed=True)
        cpus, workers, mode = row_placement(args, settings)
        identity = {
            "label": label, "library": str(library), "library_sha256": sha256(library),
            "scripts": str(scripts or godot_dir / "scripts"),
            "benchmark_sha256": sha256(godot_dir / "scripts" / BENCH_SCRIPTS[0]),
            "revision": subprocess.run(["git", "-C", str(godot_dir), "rev-parse", "HEAD"],
                                       capture_output=True, text=True).stdout.strip(),
            "input": save or world, "input_sha256": sha256(save)[:16] if save else "",
            "resolution": args.resolution, "speed": args.speed, "detail": args.detail,
            "phases": args.phases, "drawn": args.drawn, "still_s": args.still_s, "settle_cap_s": args.settle_cap_s, "shadows": shadows, "gpu_profile": args.gpu_profile,
            "cpus": cpus, "workers": workers,
            "shader_cache": str(store) if store is not None else "", "shader_cache_warm": bool(store and store.is_dir()),
            "menu_dwell_s": args.menu_dwell,
            "locked": os.environ.get(LOCKED_ENV) == "1" or not settings.get("timed_wrapper"),
            "display_driver": args.display_driver,
        }
        # New keys only on the new rows: a pinned row's identity stays as it was.
        identity.update(mode)
        width, height = args.resolution.split("x")
        env.update({
            "RAYON_NUM_THREADS": workers, "METRUM_DEBUG": "0", "METRUM_DEBUG_PERF": "1" if args.detail else "0",
            "METRUM_CRASH_DIAGNOSTICS": "0",
            "METRUM_GAMEPLAY_BENCHMARK_MODE": "windowed", "METRUM_GAMEPLAY_BENCHMARK_MATRIX": "stream",
            "METRUM_GAMEPLAY_BENCHMARK_RUN_ID": label, "METRUM_GAMEPLAY_BENCHMARK_WORLD_PATH": world,
            "METRUM_GAMEPLAY_BENCHMARK_METRICS_PATH": str(metrics),
            "METRUM_GAMEPLAY_BENCHMARK_PROFILED": "0", "METRUM_GAMEPLAY_BENCHMARK_GPU_PROFILED": "0",
            "METRUM_GAMEPLAY_BENCHMARK_GIT_REVISION": identity["revision"],
            "METRUM_GAMEPLAY_BENCHMARK_GIT_DIRTY": "0", "METRUM_GAMEPLAY_BENCHMARK_DIFF_SHA256": "none",
            "METRUM_GAMEPLAY_BENCHMARK_TIMEOUT_SEC": str(args.settle_timeout),
            "METRUM_IDLE_BENCH_SAVE_PATH": save, "METRUM_STREAM_SPEED": str(args.speed),
            "METRUM_STREAM_PHASES": args.phases, "METRUM_STREAM_TURNS": args.turns,
            "METRUM_STREAM_STILL_S": str(args.still_s), "METRUM_STREAM_SETTLE_CAP_S": str(args.settle_cap_s),
            "METRUM_STREAM_DIGESTS": "1" if args.digests else "0", "METRUM_STREAM_SHADOWS": shadows,
            "METRUM_STREAM_GPU_PASSES": "1" if args.gpu_profile else "0",
            "METRUM_STREAM_CENSUS_DIR": str(out / f"{label}.census"),
            "METRUM_STREAM_CENSUS_HOLD": os.environ.get("METRUM_STREAM_CENSUS_HOLD", "0"),
            "METRUM_STREAM_DRAWN": "1" if args.drawn else "0",
            "METRUM_STREAM_MENU_DWELL_S": "" if args.menu_dwell is None else str(args.menu_dwell),
        })
        if args.as_played:
            # The game's own pools: one worker a logical CPU of the row's set.
            env.pop("RAYON_NUM_THREADS")
        command = ["timeout", "--kill-after=10s", str(args.timeout)]
        if cpus:
            command = ["taskset", "-c", cpus] + command
        wayland = args.display_driver == "wayland"
        godot_log = str(out / f"{label}.godot.log")
        command += godot_isolated.gamescope_command(
            ["--backend", "headless", "-r", "1000", "-W", width, "-H", height],
            ["godot", "--path", str(project), "--log-file", godot_log, "--display-driver", args.display_driver,
             "--windowed", "--resolution", args.resolution], expose_wayland=wayland)
        command += (["--gpu-profile"] if args.gpu_profile else []) + ["--", "--gameplay-road-benchmark"]
        wait_for_quiet(settings, args.timeout)
        identity["start"] = datetime.datetime.now().isoformat(timespec="seconds")
        identity["loadavg"] = os.getloadavg()
        sampler = MainThreadSampler(str(out / f"{label}.godot.log")) if mode.get("sched") else None
        if sampler is not None:
            sampler.start()
        try:
            with open(out / f"{label}.out", "w") as log:
                if wayland:
                    status, identity["exit_hang"] = run_watched(command, env, log, metrics, godot_log)
                else:
                    status = subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT).returncode
        finally:
            if sampler is not None:
                sampler.stop()
        identity["end"] = datetime.datetime.now().isoformat(timespec="seconds")
        if store is not None and status == 0 and not identity.get("exit_hang"):
            copy_caches(profile, name, store, seed=False)
        identity["status"] = status
        identity["script_errors"] = (out / f"{label}.out").read_text(errors="replace").count("SCRIPT ERROR")
        (out / f"{label}.identity.json").write_text(json.dumps(identity, indent=1) + "\n")
    finally:
        shutil.rmtree(profile, ignore_errors=True)
    if identity.get("exit_hang"):
        print(f"stream_benchmark: {label} failed: exit hang (Godot still there {EXIT_HANG_S} s after its "
              f"report, stopped; godot#123059); see {out / (label + '.out')}", file=sys.stderr)
        return 1
    if status == 0 and identity["script_errors"] == 0 and metrics.exists():
        report = json.loads(metrics.read_text())
        if mode:
            # An as-played or machine-profile row: its placement and the main thread's scheduling
            # go into its own report (a pinned row's report stays the game's).
            report["row_mode"] = mode
            if sampler is not None:
                (out / f"{label}.sched.json").write_text(json.dumps(sampler.record(), separators=(",", ":")) + "\n")
                report["main_thread_sched"] = sched_phases(report, sampler.record())
            else:
                report["main_thread_sched"] = {"available": False, "reason": "Linux only"}
            metrics.write_text(json.dumps(report))
        if report.get("success"):
            print(f"== {label}")
            print_summary(report, args.detail)
            return 0
    print(f"stream_benchmark: {label} failed (status {status}, "
          f"{identity['script_errors']} script errors); see {out / (label + '.out')}", file=sys.stderr)
    return 1


# ---------------------------------------------------------------------------------------------
# As-played and machine-profile rows (`RENDER-27`, analysis A-BG): where a row runs, and the
# main thread's scheduling (run-queue wait, CPU changes, the clock of the CPU it runs on).

def cpu_list(text):
    """The CPUs of a `taskset -c` list such as `0-11,16-23`."""
    cpus = set()
    for part in text.split(","):
        first, _, last = part.strip().partition("-")
        cpus.update(range(int(first), int(last or first) + 1))
    return cpus


def resolve_cpus(value, settings):
    """`--cpus`: a profile named in the local settings' `cpu_profiles` (CPU numbering is the
    machine's), or a CPU list. Returns `(list, profile name or "")`."""
    profiles = settings.get("cpu_profiles", {})
    if value in profiles:
        return profiles[value], value
    try:
        cpu_list(value)
    except ValueError:
        raise SystemExit(f"stream_benchmark: --cpus {value!r} is neither a CPU list nor a profile "
                         f"({', '.join(sorted(profiles)) or 'none in the local settings'})")
    return value, ""


def row_placement(args, settings):
    """A row's CPU list, worker count and its record of a non-default placement. The default (the
    rows of record) is pinned to the local settings' CPUs with eight workers in each pool, and
    records nothing new. `--as-played` leaves the pools to the game (one worker a logical CPU of
    the set) and the row unpinned unless `--cpus` gives a set; `--cpus` alone keeps eight workers."""
    if not args.as_played and not args.cpus:
        return settings.get("cpus", ""), WORKERS, {}
    cpus, profile = resolve_cpus(args.cpus, settings) if args.cpus else ("", "")
    workers = "auto" if args.as_played else WORKERS
    mode = {"as_played": args.as_played, "cpus": cpus, "cpu_profile": profile,
            "sched": sys.platform.startswith("linux")}
    mode.update(cpufreq_policy())
    return cpus, workers, mode


def row_mode_line(mode):
    where = "unpinned" if not mode.get("cpus") else f"CPUs {mode['cpus']}" + (
        f" (profile {mode['cpu_profile']})" if mode.get("cpu_profile") else "")
    pools = "the game's default pools" if mode.get("as_played") else f"{WORKERS} workers a pool"
    policy = " ".join(f"{key}={mode[key]}" for key in ("governor", "epp") if mode.get(key))
    return f"   {'as played' if mode.get('as_played') else 'CPU set'}: {where}, {pools}" + (
        f"; cpufreq {policy}" if policy else "")


def cpufreq_policy():
    """Linux: CPU 0's frequency driver, governor and energy preference (the clock follows them)."""
    base = Path("/sys/devices/system/cpu/cpu0/cpufreq")
    policy = {}
    for key, name in (("driver", "scaling_driver"), ("governor", "scaling_governor"),
                      ("epp", "energy_performance_preference")):
        try:
            policy[key] = (base / name).read_text().strip()
        except OSError:
            pass
    return policy


def efficiency_cpus():
    """Linux on a hybrid Intel CPU: the E-cores (`cpu_atom`), else an empty set."""
    try:
        return cpu_list(Path("/sys/devices/cpu_atom/cpus").read_text().strip())
    except (OSError, ValueError):
        return set()


def parse_stat(text):
    """`/proc/<pid>/task/<tid>/stat`: the state and the CPU it last ran on (field 39). The name
    field may hold spaces and parentheses, so fields count from the last `)`."""
    fields = text[text.rindex(")") + 2:].split()
    return fields[0], int(fields[36])


def parse_migrations(text):
    """`/proc/<pid>/task/<tid>/sched`: `se.nr_migrations`, or None where the kernel has no such line."""
    for line in text.splitlines():
        if line.startswith("se.nr_migrations"):
            return int(line.split(":")[1])
    return None


class MainThreadSampler:
    """Linux: samples Godot's main thread from outside the game every `period_s` (no work on the
    measured thread, no `perf`). Finds the Godot process by its `--log-file` (unique to a row) and
    records `[wall s, on-CPU ns, run-queue wait ns, timeslices, migrations, running, CPU, kHz]`:
    the thread's cumulative scheduler statistics, whether it was running or runnable, the CPU it
    last ran on and that CPU's `scaling_cur_freq` (the hardware's recent average on that CPU)."""

    def __init__(self, log_file, period_s=0.01):
        self.log_file = log_file
        self.period_s = period_s
        self.samples = []
        self.reason = ""
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._run, daemon=True)

    def start(self):
        self._thread.start()

    def stop(self):
        self._stop.set()
        self._thread.join()

    def record(self):
        return {"available": bool(self.samples), "reason": self.reason, "period_s": self.period_s,
                "fields": ["t_unix", "on_cpu_ns", "wait_ns", "timeslices", "migrations", "running", "cpu", "khz"],
                "samples": self.samples}

    def _find(self):
        for entry in Path("/proc").iterdir():
            if not entry.name.isdigit():
                continue
            try:
                argv = (entry / "cmdline").read_bytes().split(b"\0")
            except OSError:
                continue
            if Path(argv[0].decode(errors="replace")).name == "godot" and self.log_file.encode() in argv:
                return int(entry.name)
        return None

    def _run(self):
        pid = None
        while pid is None and not self._stop.wait(0.1):
            pid = self._find()
        if pid is None:
            self.reason = "the Godot process was not found"
            return
        task = Path(f"/proc/{pid}/task/{pid}")
        freq = {}
        try:
            files = [os.open(task / name, os.O_RDONLY) for name in ("schedstat", "sched", "stat")]
        except OSError as error:
            self.reason = f"unreadable: {error}"
            return
        try:
            while not self._stop.wait(self.period_s):
                try:
                    schedstat, sched, stat = (os.pread(fd, 8192, 0).decode() for fd in files)
                except OSError:
                    break  # the process ended
                if not schedstat:
                    break
                state, cpu = parse_stat(stat)
                if cpu not in freq:
                    try:
                        freq[cpu] = os.open(f"/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_cur_freq", os.O_RDONLY)
                    except OSError:
                        freq[cpu] = None
                khz = int(os.pread(freq[cpu], 64, 0)) if freq[cpu] is not None else 0
                on_cpu, wait, slices = (int(value) for value in schedstat.split()[:3])
                self.samples.append([round(time.time(), 4), on_cpu, wait, slices, parse_migrations(sched),
                                     int(state == "R"), cpu, khz])
        finally:
            for fd in files + [fd for fd in freq.values() if fd is not None]:
                os.close(fd)


def interpolate(samples, column, moment):
    """A cumulative counter at `moment`, linear between the samples around it (None outside)."""
    for before, after in zip(samples, samples[1:]):
        if before[0] <= moment <= after[0]:
            if before[column] is None or after[column] is None:
                return None
            span = after[0] - before[0]
            share = (moment - before[0]) / span if span > 0 else 0.0
            return before[column] + share * (after[column] - before[column])
    return None


def sched_phases(report, record, e_cores=None):
    """Per phase (sample) of a report, the main thread's scheduling from `MainThreadSampler`'s
    record joined by the rows' wall clock (`t_unix` at each frame's end): per frame its on-CPU
    and run-queue wait ms and timeslices, CPU changes a second, and over the samples where it was
    running or runnable the mean clock of its CPU (GHz) and the E-cores' share."""
    if not record.get("available"):
        return {"available": False, "reason": record.get("reason") or "no samples"}
    samples = record["samples"]
    e_cores = efficiency_cpus() if e_cores is None else e_cores
    phases = []
    for sample in report["samples"]:
        rows = sample["rows"]
        if not rows or "t_unix" not in rows[0]:
            continue
        start = rows[0]["t_unix"] - rows[0]["frame_ms"] / 1000.0
        end = rows[-1]["t_unix"]
        entry = {"phase": phase_label(sample), "frames": len(rows), "seconds": round(end - start, 3)}
        deltas = {}
        for column, name in ((1, "on_cpu"), (2, "wait"), (3, "timeslices"), (4, "migrations")):
            a, b = interpolate(samples, column, start), interpolate(samples, column, end)
            deltas[name] = None if a is None or b is None else b - a
        if deltas["on_cpu"] is not None:
            entry["on_cpu_ms"] = round(deltas["on_cpu"] / 1e6 / len(rows), 3)
            entry["wait_ms"] = round(deltas["wait"] / 1e6 / len(rows), 3)
            entry["timeslices"] = round(deltas["timeslices"] / len(rows), 2)
        if deltas["migrations"] is not None and end > start:
            entry["migrations_s"] = round(deltas["migrations"] / (end - start), 1)
        running = [s for s in samples if start <= s[0] <= end and s[5] and s[7]]
        entry["running_samples"] = len(running)
        if running:
            entry["clock_ghz"] = round(statistics.mean(s[7] for s in running) / 1e6, 3)
            if e_cores:
                entry["e_core_share"] = round(sum(s[6] in e_cores for s in running) / len(running), 3)
        phases.append(entry)
    return {"available": True, "period_s": record["period_s"], "phases": phases}


def sched_lines(report):
    """`summary`'s main-thread table of an as-played or machine-profile row (none on other rows)."""
    sched = report.get("main_thread_sched")
    if not sched:
        return []
    if not sched.get("available"):
        return [f"main thread: not measured ({sched.get('reason', '')})"]
    lines = [f"{'main thread':<22}{'cpu ms':>8}{'wait ms':>8}{'slices':>8}{'migr/s':>8}{'GHz':>7}{'E %':>6}  "
             "(a frame; GHz and E-cores over running samples)"]

    def cell(entry, key, width, digits):
        return f"{entry[key]:>{width}.{digits}f}" if key in entry else f"{'-':>{width}}"
    for entry in sched["phases"]:
        share = f"{entry['e_core_share'] * 100:>6.0f}" if "e_core_share" in entry else f"{'-':>6}"
        lines.append(f"{entry['phase']:<22}{cell(entry, 'on_cpu_ms', 8, 2)}{cell(entry, 'wait_ms', 8, 3)}"
                     f"{cell(entry, 'timeslices', 8, 1)}{cell(entry, 'migrations_s', 8, 0)}"
                     f"{cell(entry, 'clock_ghz', 7, 2)}{share}")
    return lines


def percentile(values, q):
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(len(ordered) * q))]


def phase_label(sample):
    return sample["label"] + (f" {sample['duration_s']:g}s" if sample.get("duration_s") else "")


def script_means(rows):
    """Mean milliseconds a frame per renderer over `rows`, from each row's own perf records (the
    benchmark joined them to the frame they were taken in), plus their details summed."""
    totals = {}
    details = {}
    for row in rows:
        for name, record in row.get("scripts", {}).items():
            totals[name] = totals.get(name, 0.0) + record["elapsed_ms"]
            for key, value in record.get("details", {}).items():
                if isinstance(value, (int, float)):
                    details[f"{name}.{key}"] = details.get(f"{name}.{key}", 0.0) + value
    count = max(1, len(rows))
    return ({name: value / count for name, value in totals.items()},
            {name: value / count for name, value in details.items()})


def print_summary(report, scripts=False):
    if report.get("row_mode"):
        print(row_mode_line(report["row_mode"]))
    print(f"{'phase':<22}{'n':>5}{'p50':>8}{'p95':>8}{'max':>8}{'>16.7':>6}{'>22':>5}{'>33':>5}"
          f"{'gpu50':>7}  levels")
    for sample in report["samples"]:
        rows = sample["rows"]
        times = [row["frame_ms"] for row in rows]
        gpu = [row["render_gpu_ms"] for row in rows]
        settle = f" settle_ms={sample['settle_ms']:.0f}" if "settle_ms" in sample else ""
        print(f"{phase_label(sample):<22}{len(rows):>5}{percentile(times, 0.5):>8.2f}"
              f"{percentile(times, 0.95):>8.2f}{max(times):>8.2f}{sample['over_16_7']:>6}"
              f"{sample['over_22']:>5}{sample['over_33_3']:>5}{percentile(gpu, 0.5):>7.2f}  "
              f"{rows[-1].get('reflection_levels')}{settle}")
    for line in sched_lines(report):
        print(line)
    for sample in report["samples"]:
        passes = sample.get("gpu_passes")
        if passes:
            # Passes under 5 us at p50 (the culls, marks and empty canvases) are left out.
            parts = " ".join(f"{name}={stat['p50']:.3f}" for name, stat in sorted(passes.items())
                             if stat["p50"] >= 0.005)
            print(f"{phase_label(sample):<22} gpu p50 by pass: {parts}")
    if not scripts:
        return
    for sample in report["samples"]:
        means, _ = script_means(sample["rows"])
        frame = statistics.mean(row["frame_ms"] for row in sample["rows"])
        parts = " ".join(f"{name}={value:.2f}" for name, value in sorted(means.items(), key=lambda kv: -kv[1]))
        print(f"{phase_label(sample):<22} frame {frame:.2f} timed {sum(means.values()):.2f}: {parts}")


# ---------------------------------------------------------------------------------------------
# `compare`: the C B B C verdict by same-binary spread.

def phase_stats(report):
    """Per phase (`phase_label`): frame p50 and max, frames over 33.3 ms, GPU p50."""
    return {phase_label(s): {"p50": percentile([r["frame_ms"] for r in s["rows"]], 0.5),
                             "max": max(r["frame_ms"] for r in s["rows"]), "over33": s["over_33_3"],
                             "gpu": percentile([r["render_gpu_ms"] for r in s["rows"]], 0.5)}
            for s in report["samples"]}


def phase_verdict(controls, branches):
    """A phase of C B B C: per metric (p50, max) the branch's mean minus the control's against the
    same-binary spread, the larger of the two sides' run-to-run differences. `worse` where the
    delta exceeds the spread, else `better` where its negative does, else `level`."""
    spread = {m: max(abs(controls[0][m] - controls[1][m]), abs(branches[0][m] - branches[1][m]))
              for m in ("p50", "max")}
    delta = {m: statistics.mean(x[m] for x in branches) - statistics.mean(x[m] for x in controls)
             for m in ("p50", "max")}
    worse = [m for m in delta if delta[m] > spread[m]]
    better = [m for m in delta if -delta[m] > spread[m]]
    verdict = ("worse " + ",".join(worse)) if worse else ("better " + ",".join(better)) if better else "level"
    return verdict, delta, spread


def compare_lines(name, runs):
    """The verdict table of one input: `runs` maps ctl1, br1, br2, ctl2 to their reports."""
    stats = {key: phase_stats(report) for key, report in runs.items()}
    lines = [f"## {name}: p50 / max ms (C1, C2 | B1, B2), over 33.3 ms, verdict (delta of means vs same-binary spread)",
             "| phase | ctl p50 | br p50 | ctl max | br max | >33.3 ctl / br | gpu p50 | verdict |",
             "|---|---|---|---|---|---|---|---|"]
    for phase in stats["ctl1"]:
        c = [stats[k][phase] for k in ("ctl1", "ctl2")]
        b = [stats[k][phase] for k in ("br1", "br2")]
        verdict, d, sp = phase_verdict(c, b)
        lines.append(
            f"| {phase} | {c[0]['p50']:.1f}, {c[1]['p50']:.1f} | {b[0]['p50']:.1f}, {b[1]['p50']:.1f} | "
            f"{c[0]['max']:.0f}, {c[1]['max']:.0f} | {b[0]['max']:.0f}, {b[1]['max']:.0f} | "
            f"{c[0]['over33']}, {c[1]['over33']} / {b[0]['over33']}, {b[1]['over33']} | "
            f"{statistics.mean([x['gpu'] for x in c + b]):.1f} | {verdict} (p50 {d['p50']:+.1f}, "
            f"max {d['max']:+.1f}; spread {sp['p50']:.1f} / {sp['max']:.1f}) |")
    return lines


# `drawn`: the drawn probe's table (METRUM_STREAM_DRAWN=1, `RENDER-32-N`).
def drawn_lines(report, bar=3):
    """Per sample and layer: chunks entered; entered-to-drawn p50 / p95 / max frames (drawn: the
    chunk itself; visible: it or a stand-in over its area), the share of the waiting frames whose
    product was ready (the install budget's), chunks still waiting at the end (`open`, their
    longest wait so far) or gone undrawn; in-view holes (runs of frames an installed-set chunk in
    the frustum was not drawn) and bare runs (nothing drew over it): count and longest. `pass`
    when the drawn p95 and every hole are at most `bar` frames and nothing stayed open."""
    probe = report.get("drawn_probe") or {}
    lines = ["| sample | layer | entered | drawn p50 / p95 / max | visible p50 / p95 / max | ready share | "
             "open (longest) / left | holes (longest) | bare (longest) | holes max a frame | verdict |",
             "|---|---|---|---|---|---|---|---|---|---|---|"]
    stat = lambda v: f"{percentile(v, 0.5)} / {percentile(v, 0.95)} / {max(v)}" if v else "-"
    for sample in [s["label"] for s in report["samples"]]:
        for layer, entry in sorted((probe.get(sample) or {}).items()):
            waits, visible, ready = entry["waits"], entry["waits_visible"], entry["ready_frames"]
            holes, bare, open_ = entry["holes"], entry["bare"], entry["open"]
            if not (entry["entered"] or holes or open_):
                continue
            share = f"{sum(ready) / sum(waits):.0%}" if sum(waits) else "-"
            ok = (not waits or percentile(waits, 0.95) <= bar) and all(h <= bar for h in holes) and not open_
            lines.append(
                f"| {sample} | {layer} | {entry['entered']} | {stat(waits)} | {stat(visible)} | {share} | "
                f"{len(open_)} ({max(open_) if open_ else 0}) / {entry['left_undrawn']} | "
                f"{len(holes)} ({max(holes) if holes else 0}) | {len(bare)} ({max(bare) if bare else 0}) | "
                f"{entry['holes_max']} | {'pass' if ok else 'MISS'} |")
    return lines


def drawn(args):
    status = 0
    for path in args.metrics:
        report = json.loads(Path(path).read_text())
        lines = drawn_lines(report, args.bar)
        print(f"## {Path(path).name}: frames observed {report.get('drawn_probe', {}).get('frames', 0)}")
        print("\n".join(lines))
        status = max(status, int(any(line.endswith("MISS |") for line in lines)))
    return status


# `flight`: a flight run's terrain streaming (`--phases flight_low,flight_high,flight_jump`,
# `TERRAIN-41`): per sample the rates, the backlog over time and, instrumented, the costs.

def _flight_detail(row, record, key):
    entry = (row.get("scripts") or {}).get(record) or {}
    return float((entry.get("details") or {}).get(key, 0.0))


def flight_lines(report, every_s):
    lines = []
    for sample in report["samples"]:
        rows = sample["rows"]
        if not rows:
            continue
        frame_ms = [row["frame_ms"] for row in rows]
        wall_s = sum(frame_ms) / 1000.0
        median = statistics.median(frame_ms)
        first, last = rows[0]["streams"]["terrain"], rows[-1]["streams"]["terrain"]
        builds = last.get("creates_total", 0) - first.get("creates_total", 0)
        missing = [row["streams"]["terrain"].get("terrain_missing", 0) for row in rows]
        lines.append(f"## {sample['label']}: {wall_s:.1f} s, {len(rows)} frames ({len(rows) / wall_s:.0f} fps), "
                     f"frame p50 {median:.1f} p95 {percentile(frame_ms, 0.95):.1f} ms; builds {builds} "
                     f"({builds / len(rows):.2f} a frame, {builds / wall_s:.0f}/s); missing max {max(missing)}, "
                     f"end {missing[-1]}; built {first.get('built')} -> {last.get('built')}")
        memory = sample.get("memory")
        if memory:
            lines.append("   memory at the end, MiB: " + ", ".join(
                f"{key} {value / 2**20:.0f}" for key, value in memory.items()))
        for key in ("patch_bytes", "water_patch_bytes"):
            if key in sample:
                lines.append(f"   {key}: {sample[key]}")
        built_parts = ["resources", "height", "node", "masks", "params", "land_cover"]
        count = sum(_flight_detail(row, "terrain", "build_count") for row in rows)
        if count:
            parts = ", ".join(f"{part} {1000 * sum(_flight_detail(row, 'terrain', f'build_{part}_ms') for row in rows) / count:.0f}"
                              for part in built_parts)
            total = 1000 * sum(_flight_detail(row, "terrain", "build_ms") for row in rows) / count
            lines.append(f"   one build {total:.0f} us: {parts}")
        payloads = sum(_flight_detail(row, "terrain", "worker_built") for row in rows)
        if payloads:
            run = sum(_flight_detail(row, "terrain", "worker_run_ms") for row in rows)
            source = sum(_flight_detail(row, "terrain", "worker_source_ms") for row in rows)
            lines.append(f"   payload workers: {run / payloads:.3f} ms a payload ({source / payloads:.3f} under the "
                         f"core lock), {run / 1000 / wall_s:.2f} workers busy, retried "
                         f"{sum(_flight_detail(row, 'terrain', 'worker_retried') for row in rows):.0f}")
        owners = ["terrain_commit", "graded", "water", "stream", "free"]
        if any(_flight_detail(row, "vegetation", "ledger_predicted_ms") for row in rows):
            lines.append("   install ledger, mean ms a frame: " + ", ".join(
                f"{owner} {statistics.mean(_flight_detail(row, 'vegetation', f'ledger_{owner}_ms') for row in rows):.2f}"
                for owner in owners) + f"; measured {statistics.mean(_flight_detail(row, 'vegetation', 'ledger_ms') for row in rows):.2f}"
                f", predicted {statistics.mean(_flight_detail(row, 'vegetation', 'ledger_predicted_ms') for row in rows):.2f}")
        lines.append("   t s | wanted | missing | in flight | ready | resident | built | oldest wait s | in-view holes")
        elapsed, due = 0.0, 0.0
        for row in rows:
            elapsed += row["frame_ms"] / 1000.0
            if elapsed < due and row is not rows[-1]:
                continue
            due += every_s
            terrain = row["streams"]["terrain"]
            wanted = (terrain.get("terrain_resident", 0) - terrain.get("terrain_leaving", 0)
                      + terrain.get("terrain_missing", 0))
            backlog = (row.get("backlog") or {}).get("terrain")
            oldest = f"{backlog[1] * median / 1000:.2f}" if backlog else "-"
            holes = backlog[3] if backlog else "-"
            lines.append(f"   {elapsed:5.1f} | {wanted} | {terrain.get('terrain_missing', 0)} | "
                         f"{terrain.get('payload_requested', 0)} | {terrain.get('payload_ready', 0)} | "
                         f"{terrain.get('terrain_resident', 0)} | {terrain.get('built', '-')} | {oldest} | {holes}")
    return lines


def flight(args):
    for path in args.metrics:
        print(f"# {Path(path).name}")
        print("\n".join(flight_lines(json.loads(Path(path).read_text()), args.every)))
    return 0


def split_blocks(report):
    """A shadow-sequence run's quartets (`RENDER-15`): per variant X whose blocks run `full, X, X,
    full`, the reports `ctl1`, `br1`, `br2`, `ctl2` holding those blocks' samples, as a series'
    rows would, for `compare_lines`. Returns `{variant: {row: report}}`."""
    sequence = report.get("shadows", [])
    blocks = [[s for s in report["samples"] if s.get("block") == i] for i in range(len(sequence))]
    out = {}
    for i in range(1, len(sequence) - 2):
        variant = sequence[i]
        if variant != "full" and sequence[i + 1] == variant and sequence[i - 1] == sequence[i + 2] == "full":
            rows = {"ctl1": blocks[i - 1], "br1": blocks[i], "br2": blocks[i + 1], "ctl2": blocks[i + 2]}
            out[variant] = {row: dict(report, samples=samples) for row, samples in rows.items()}
    return out


def split(args):
    """Writes `<label>.<variant>-ctl1` ... `-ctl2` metrics beside the run's report, so `compare
    <dir> <label>.<variant>` reads each variant's quartet."""
    path = Path(args.metrics)
    label = path.name.removesuffix(".metrics.json")
    quartets = split_blocks(json.loads(path.read_text()))
    for variant, rows in quartets.items():
        for row, report in rows.items():
            (path.parent / f"{label}.{variant}-{row}.metrics.json").write_text(json.dumps(report))
    print(" ".join(f"{label}.{variant}" for variant in quartets))
    return 0 if quartets else 1


def gpu_lines(name, runs):
    """`compare --gpu` (`RENDER-15`): per phase the GPU p50's branch-minus-control delta against
    its same-binary spread, read like `phase_verdict`'s, and the shadow passes' median primitives
    and draw calls a side (rows from before `shadow_pass` show none)."""
    stats = {key: phase_stats(report) for key, report in runs.items()}
    shadow = {key: {phase_label(s): [percentile([r["shadow_pass"][i] for r in s["rows"]], 0.5)
                                     for i in (2, 0)] if all("shadow_pass" in r for r in s["rows"]) else None
                    for s in report["samples"]} for key, report in runs.items()}
    lines = [f"## {name}: GPU p50 ms, delta vs same-binary spread; shadow-pass primitives (k) / draws, ctl / br",
             "| phase | ctl gpu | br gpu | delta (spread) | shadow ctl | shadow br |", "|---|---|---|---|---|---|"]
    for phase in stats["ctl1"]:
        c = [stats[k][phase]["gpu"] for k in ("ctl1", "ctl2")]
        b = [stats[k][phase]["gpu"] for k in ("br1", "br2")]
        spread = max(abs(c[0] - c[1]), abs(b[0] - b[1]))
        delta = statistics.mean(b) - statistics.mean(c)
        verdict = "worse" if delta > spread else "better" if -delta > spread else "level"

        def side(keys):
            values = [shadow[k].get(phase) for k in keys]
            if any(v is None for v in values):
                return "-"
            return f"{statistics.mean(v[0] for v in values) / 1000:.0f} / {statistics.mean(v[1] for v in values):.0f}"
        lines.append(f"| {phase} | {statistics.mean(c):.2f} | {statistics.mean(b):.2f} | "
                     f"{delta:+.2f} ({spread:.2f}) {verdict} | {side(('ctl1', 'ctl2'))} | {side(('br1', 'br2'))} |")
    return lines


def compare(args):
    directory = Path(args.dir)
    for name in args.inputs:
        runs = {f"{side}{n}": json.loads((directory / f"{name}-{side}{n}.metrics.json").read_text())
                for side, n in SERIES}
        print("\n" + "\n".join(compare_lines(name, runs)))
        if args.gpu:
            print("\n" + "\n".join(gpu_lines(name, runs)))
    return 0


# ---------------------------------------------------------------------------------------------
# `digests`: two runs' settle-point products (METRUM_STREAM_DIGESTS=1), per family and point.

def tile_fields(line):
    """A graded digest line: `x,z,level,step,token,behind,walls,copies`."""
    fields = line.split(",")
    return {"level": (int(fields[2]), int(fields[3])), "token": fields[4], "rest": fields[5:]}


def rule_level(distance, held, levels):
    """The level the graded rule gives at `distance` for a tile held at level code `held` (None:
    not held), from the report's `graded_levels`: a level's end moves to its held end for a tile
    held at that level or a finer one; `GL3`'s step ends have no hysteresis."""
    steps = [tuple(step) for step in levels["level_steps"]]
    for index, (free, held_end) in enumerate(zip(levels["level_end_m"], levels["held_level_end_m"])):
        if distance < (held_end if held is not None and held <= index else free):
            return steps[index]
    gl3 = [step for step in steps if step[0] == 3]
    for end, step in zip(list(levels["gl3_end_m"]) + [float("inf")], gl3):
        if distance < end:
            return step
    return gl3[-1]


def tile_distance(point, key, tile_m):
    """The camera's distance to tile `key`'s centre as the rule measures it (`(d, d)`), from the
    point's `level_camera` (XZ and the height above the ground under it). A report without it
    gives a bound: the ground distance up to the distance from the camera's height over y = 0
    (the ground under the camera between 0 and the camera)."""
    x, z = (int(v) for v in key.split(","))
    camera = point.get("level_camera")
    exact = camera is not None
    camera = camera or point["camera"]
    ground = math.hypot((x + 0.5) * tile_m - camera[0], (z + 0.5) * tile_m - camera[2])
    height = abs(camera[1])
    far = math.sqrt(ground * ground + height * height)
    return (far, far) if exact else (ground, far)


def in_band(levels, a, b, near, far):
    """Whether two held levels `a` and `b` are both ones the rule allows over the whole distance
    range [near, far]: the range lies inside one level end's hysteresis band [end, held end) and
    the pair is that band's finer (held) level and the rule's level without hysteresis."""
    steps = [tuple(step) for step in levels["level_steps"]]
    for index, (free, held_end) in enumerate(zip(levels["level_end_m"], levels["held_level_end_m"])):
        if free <= near and far < held_end:
            coarse = rule_level(near, None, levels)
            return coarse == rule_level(far, None, levels) and {a, b} == {steps[index], coarse}
    return False


def graded_point(a, b, levels):
    """`equal`, `band` or `different`, with a note. `band` only when every differing tile differs
    in level or step, lies within a level end's hysteresis band of the camera (the ends from the
    report) at levels the rule allows there, and every tile at equal level, step and token has
    equal bytes."""
    if a.get("point") != b.get("point") or a.get("sample") != b.get("sample"):
        return "different", "the settle points do not match"
    if a["digest"] == b["digest"]:
        return "equal", f"{a.get('tiles')} tiles"
    if levels is None:
        return "different", "no graded_levels in the report (or --levels)"
    if a.get("level_camera", a["camera"]) != b.get("level_camera", b["camera"]):
        return "different", "the cameras differ"
    band = []
    for key in sorted(set(a["by_tile"]) | set(b["by_tile"])):
        la, lb = a["by_tile"].get(key), b["by_tile"].get(key)
        if la == lb:
            continue
        if la is None or lb is None:
            return "different", f"tile {key} installed on one side only"
        fa, fb = tile_fields(la), tile_fields(lb)
        if fa["level"] == fb["level"]:
            same = "token" if fa["token"] == fb["token"] else "step, other token"
            return "different", f"tile {key} differs at equal level and {same}"
        near, far = tile_distance(a, key, levels["tile_m"])
        if not in_band(levels, fa["level"], fb["level"], near, far):
            return "different", (f"tile {key} at {near:.0f}-{far:.0f} m, levels {fa['level']} and "
                                 f"{fb['level']}, outside a hysteresis band")
        band.append((key, near, far, fa["level"], fb["level"]))
    bound = "" if "level_camera" in a else " (distance bound: no level camera recorded)"
    tiles = ", ".join(f"{key} {near:.0f}-{far:.0f} m {la}/{lb}" for key, near, far, la, lb in band)
    return "band", f"{len(band)} tiles in a hysteresis band: {tiles}{bound}"


def simple_point(a, b, fields):
    if a.get("point") != b.get("point") or a.get("sample") != b.get("sample"):
        return "different", "the settle points do not match"
    differing = [field for field in fields if a.get(field) != b.get(field)]
    if differing:
        return "different", "differs in " + ", ".join(differing)
    return "equal", ""


def compare_digests(a, b, levels=None):
    """Per family (graded, buildings, lamps, roads, parked cars, vegetation, land covers, water) a list of
    `(point, sample, verdict, note)`.
    `levels`: graded level ends for reports that predate their record (another report's)."""
    levels = a.get("graded_levels") or levels
    if a.get("graded_levels") and b.get("graded_levels") and a["graded_levels"] != b["graded_levels"]:
        levels = None
    out = {}
    for family, key, judge in (("graded", "graded_digests", lambda p, q: graded_point(p, q, levels)),
                               ("buildings", "building_digests", lambda p, q: simple_point(p, q, ("digest", "batches"))),
                               ("lamps", "lamp_digests", lambda p, q: simple_point(p, q, ("cells", "far"))),
                               ("roads", "road_digests", lambda p, q: simple_point(p, q, ("digest", "chunks"))),
                               ("parked", "parked_digests", lambda p, q: simple_point(p, q, ("digest", "batches"))),
                               ("vegetation", "vegetation_digests",
                                lambda p, q: simple_point(p, q, ("digest", "patches"))),
                               ("land_cover", "patch_digests",
                                lambda p, q: simple_point(p.get("land_cover", {}), q.get("land_cover", {}),
                                                          ("digest", "patches")))):
        x, y = a.get(key, []), b.get(key, [])
        rows = [(p.get("point"), p.get("sample", ""), *judge(p, q)) for p, q in zip(x, y)]
        if len(x) != len(y):
            rows.append((None, "", "different", f"{len(x)} settle points against {len(y)}"))
        out[family] = rows
    same = a.get("water_pose", []) == b.get("water_pose", [])
    out["water"] = [(None, "water_pose", "equal" if same else "different", str(a.get("water_pose")))]
    return out


def digests(args):
    a, b = (json.loads(Path(path).read_text()) for path in (args.a, args.b))
    levels = json.loads(Path(args.levels).read_text()).get("graded_levels") if args.levels else None
    families = compare_digests(a, b, levels)
    different = 0
    for family, rows in families.items():
        counts = {verdict: sum(1 for row in rows if row[2] == verdict) for verdict in ("equal", "band", "different")}
        print(f"{family}: {counts['equal']} equal, {counts['band']} band, {counts['different']} different "
              f"of {len(rows)}")
        for point, sample, verdict, note in rows:
            if verdict != "equal":
                print(f"  point {point} ({sample}): {verdict}: {note}")
        different += counts["different"]
    return 1 if different else 0


# ---------------------------------------------------------------------------------------------
# `stills`: captures through the isolated launcher, byte identity, and the systematic-pixel test.

def load_rgb(path):
    import numpy  # noqa: PLC0415  (only the stills comparisons need numpy and Pillow)
    from PIL import Image  # noqa: PLC0415
    return numpy.asarray(Image.open(path).convert("RGB"))


def image_difference(a, b):
    """Differing pixels, the largest channel delta and the worst 16 x 16 block's share."""
    import numpy  # noqa: PLC0415
    if a.shape != b.shape:
        return {"pixels": -1, "max_delta": -1, "worst_block": 1.0}
    delta = numpy.abs(a.astype(numpy.int16) - b.astype(numpy.int16)).max(axis=2)
    differs = delta > 0
    height, width = differs.shape
    blocks = numpy.zeros(((height + 15) // 16, (width + 15) // 16))
    numpy.add.at(blocks, (numpy.nonzero(differs)[0] // 16, numpy.nonzero(differs)[1] // 16), 1)
    return {"pixels": int(differs.sum()), "max_delta": int(delta.max()), "worst_block": float(blocks.max()) / 256}


def stills_compare(a_dir, b_dir):
    """Per PNG of `a_dir`: `identical` (bytes or pixels) or `different` / `missing`, with a note."""
    out = []
    for path in sorted(Path(a_dir).glob("*.png")):
        other = Path(b_dir) / path.name
        if not other.exists():
            out.append((path.name, "missing", ""))
        elif sha256(path) == sha256(other):
            out.append((path.name, "identical", "bytes"))
        else:
            a, b = load_rgb(path), load_rgb(other)
            if a.shape == b.shape and (a == b).all():
                out.append((path.name, "identical", "pixels"))
            else:
                d = image_difference(a, b)
                out.append((path.name, "different", f"{d['pixels']} px, max delta {d['max_delta']}, "
                                                    f"worst 16x16 block {d['worst_block']:.0%}"))
    return out


def systematic_counts(ctl1, ctl2, br1, br2):
    """The systematic-pixel test over two runs a side: pixels equal within each side's two runs
    and different between the sides (`by_side`, with its mask), against the same count for the
    two other pairings of the four runs (`null`: control and branch mixed, so a count there is
    run-to-run speckle), and the control's own run-to-run difference."""
    import numpy  # noqa: PLC0415

    def equal(x, y):
        return (x == y).all(axis=2)

    def count(a, b, c, d):
        mask = equal(a, b) & equal(c, d) & ~equal(a, c)
        return int(mask.sum()), mask
    by_side, mask = count(ctl1, ctl2, br1, br2)
    null = [count(ctl1, br1, ctl2, br2)[0], count(ctl1, br2, ctl2, br1)[0]]
    return {"by_side": by_side, "null": null, "ctl_vs_ctl": int((~equal(ctl1, ctl2)).sum()),
            "br_vs_br": int((~equal(br1, br2)).sum()), "mask": mask, "bbox": bounding_box(numpy, mask)}


def bounding_box(numpy, mask):
    rows, cols = numpy.nonzero(mask)
    return None if rows.size == 0 else (int(cols.min()), int(rows.min()), int(cols.max()) + 1, int(rows.max()) + 1)


def systematic_verdict(by_side, null):
    """`systematic` when the by-side count exceeds the larger null pairing by more than the null's
    own spread (their range) or three Poisson deviations of it, whichever is larger; else `none`
."""
    tolerance = max(max(null) - min(null), 3.0 * math.sqrt(max(null)))
    return ("systematic" if by_side > max(null) + tolerance else "none"), tolerance


def stills_systematic(dirs, masks=None):
    out = []
    for path in sorted(Path(dirs[0]).glob("*.png")):
        images = [load_rgb(Path(directory) / path.name) for directory in dirs]
        counts = systematic_counts(*images)
        verdict, tolerance = systematic_verdict(counts["by_side"], counts["null"])
        if masks is not None and counts["by_side"]:
            from PIL import Image  # noqa: PLC0415
            Path(masks).mkdir(parents=True, exist_ok=True)
            Image.fromarray(counts["mask"].astype("uint8") * 255).save(Path(masks) / f"systematic-{path.name}")
        out.append((path.name, verdict, counts, tolerance))
    return out


def capture(args):
    """One stills run: the idle-frame matrix's screenshots of `--poses` and `--views` with one
    library and Godot tree, in gamescope's headless backend, pinned, in an isolated profile, under
    the shared timing lock."""
    settings = timing_session.load_settings()
    godot_dir = Path(args.godot_dir).resolve()
    library = Path(args.library or godot_dir / "bin/libmetrum_rise.so").resolve()
    scripts = Path(args.scripts).resolve() if args.scripts else None
    out = Path(args.out).resolve() / args.label
    out.mkdir(parents=True, exist_ok=False)
    width, height = args.resolution.split("x")
    profile, project, _name, env = godot_isolated.create(
        out, "stills.", godot_dir=godot_dir, library=library, scripts=scripts, mods=player_mods(),
        name_prefix="MetrumStills", own_scripts=BENCH_SCRIPTS)
    try:
        env.update({
            "RAYON_NUM_THREADS": WORKERS, "METRUM_DEBUG": "0", "METRUM_CRASH_DIAGNOSTICS": "0",
            "METRUM_GAMEPLAY_BENCHMARK_MODE": "windowed", "METRUM_GAMEPLAY_BENCHMARK_MATRIX": "idle",
            "METRUM_GAMEPLAY_BENCHMARK_RUN_ID": args.label,
            "METRUM_GAMEPLAY_BENCHMARK_WORLD_PATH": str(Path(args.world).resolve()),
            "METRUM_GAMEPLAY_BENCHMARK_METRICS_PATH": str(out / "metrics.json"),
            "METRUM_GAMEPLAY_BENCHMARK_PROFILED": "0", "METRUM_GAMEPLAY_BENCHMARK_GPU_PROFILED": "0",
            "METRUM_GAMEPLAY_BENCHMARK_TIMEOUT_SEC": "180",
            "METRUM_IDLE_BENCH_SAVE_PATH": str(Path(args.save).resolve()) if args.save else "",
            "METRUM_IDLE_BENCH_POSES": args.poses, "METRUM_IDLE_BENCH_VIEWS": args.views,
            "METRUM_IDLE_BENCH_VARIANTS": args.variants, "METRUM_IDLE_BENCH_HOUR": str(args.hour),
            "METRUM_IDLE_BENCH_PATHS": "none", "METRUM_IDLE_BENCH_FRAMES": "10",
            "METRUM_IDLE_BENCH_WARMUP_FRAMES": str(args.warmup), "METRUM_IDLE_BENCH_SCREENSHOT_DIR": str(out),
        })
        lock = timing_session.main_target() / ".timing.lock"
        command = (["flock", "-s", str(lock)] if lock.exists() else [])
        if settings.get("cpus"):
            command += ["taskset", "-c", settings["cpus"]]
        command += ["timeout", "--kill-after=10s", str(args.timeout)] + godot_isolated.gamescope_command(
            ["--backend", "headless", "-W", width, "-H", height],
            ["godot", "--path", str(project), "--display-driver", "x11", "--windowed",
             "--resolution", args.resolution, "--", "--gameplay-road-benchmark"])
        with open(out / "godot.out", "w") as log:
            status = subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT).returncode
    finally:
        shutil.rmtree(profile, ignore_errors=True)
    errors = (out / "godot.out").read_text(errors="replace").count("SCRIPT ERROR")
    pngs = sorted(p.name for p in out.glob("*.png"))
    expected = (len([p for p in args.poses.split(",") if p]) + len([v for v in args.views.split(";") if v])) \
        * len([v for v in args.variants.split(",") if v])
    metrics = out / "metrics.json"
    ok = status == 0 and errors == 0 and len(pngs) == expected and metrics.exists() \
        and json.loads(metrics.read_text()).get("success")
    print(f"stills {args.label}: status {status}, {errors} script errors, {len(pngs)} of {expected} stills, "
          f"{'ok' if ok else 'FAILED'}")
    return 0 if ok else 1


def stills_variants(directory, crops=None, window=(480, 270)):
    """Per `<pose>-<variant>.png` of one capture, its difference from `<pose>-full.png`
    (`image_difference`). With `crops`, writes the window of the variant's most-changed region
    side by side: full, variant, and the difference amplified eight times."""
    import numpy  # noqa: PLC0415
    out = []
    for path in sorted(Path(directory).glob("*.png")):
        pose, _, variant = path.stem.rpartition("-")
        full = path.with_name(f"{pose}-full.png")
        if variant == "full" or not full.exists():
            continue
        a, b = load_rgb(full), load_rgb(path)
        d = image_difference(a, b)
        out.append((pose, variant, d))
        if crops is None or d["pixels"] <= 0:
            continue
        from PIL import Image  # noqa: PLC0415
        delta = numpy.abs(a.astype(numpy.int16) - b.astype(numpy.int16)).max(axis=2)
        # The window with the most differing pixels, on a grid of half-window steps.
        width, height = min(window[0], a.shape[1]), min(window[1], a.shape[0])
        best, corner = -1, (0, 0)
        for y in range(0, a.shape[0] - height + 1, height // 2):
            for x in range(0, a.shape[1] - width + 1, width // 2):
                count = int((delta[y:y + height, x:x + width] > 0).sum())
                if count > best:
                    best, corner = count, (x, y)
        x, y = corner
        diff = numpy.clip(delta[y:y + height, x:x + width].astype(numpy.int32) * 8, 0, 255).astype(numpy.uint8)
        strip = numpy.concatenate([a[y:y + height, x:x + width], b[y:y + height, x:x + width],
                                   numpy.repeat(diff[:, :, None], 3, axis=2)], axis=1)
        Path(crops).mkdir(parents=True, exist_ok=True)
        Image.fromarray(strip).save(Path(crops) / f"{pose}-{variant}-x{x}y{y}.png")
    return out


def stills(args):
    if args.stills_command == "capture":
        return capture(args)
    if args.stills_command == "variants":
        rows = stills_variants(args.dir, args.crops)
        for pose, variant, d in rows:
            print(f"{pose} {variant}: {d['pixels']} px against full, max delta {d['max_delta']}, "
                  f"worst 16x16 block {d['worst_block']:.0%}")
        return 0 if rows else 1
    if args.stills_command == "compare":
        rows = stills_compare(args.a, args.b)
        for name, verdict, note in rows:
            print(f"{name}: {verdict} {note}".rstrip())
        return 0 if rows and all(row[1] == "identical" for row in rows) else 1
    rows = stills_systematic([args.ctl1, args.ctl2, args.br1, args.br2], args.masks)
    for name, verdict, counts, tolerance in rows:
        print(f"{name}: {verdict}: by side {counts['by_side']}, other pairings {counts['null'][0]} / "
              f"{counts['null'][1]} (tolerance {tolerance:.0f}); control against control "
              f"{counts['ctl_vs_ctl']} px, branch against branch {counts['br_vs_br']} px; bbox {counts['bbox']}")
    return 1 if not rows or any(row[1] == "systematic" for row in rows) else 0


def add_run_options(parser, series):
    if series:
        parser.add_argument("--input-label", required=True, help="rows <input>-ctl1, -br1, -br2, -ctl2")
        parser.add_argument("--ctl-library", required=True, help="the control's library")
        parser.add_argument("--ctl-scripts", help="the control's godot/scripts")
    else:
        parser.add_argument("--label", required=True)
    parser.add_argument("--out", default=str(ROOT / "benchmark-results"))
    parser.add_argument("--godot-dir", default=str(ROOT / "godot"))
    parser.add_argument("--library", help="default: the godot directory's deployed library")
    parser.add_argument("--scripts", help="another build's godot/scripts (a control)")
    parser.add_argument("--save", help="a save to load; without it the world loads as a new game")
    parser.add_argument("--world", default=str(ROOT / "godot/bootstrap/worlds/kuopio_18432m_8m.sqlite"))
    parser.add_argument("--resolution", default="1920x1080")
    parser.add_argument("--speed", type=float, default=0.0)
    parser.add_argument("--phases", default="")
    parser.add_argument("--turns", default="")
    parser.add_argument("--still-s", type=float, default=0.0,
                        help="METRUM_STREAM_STILL_S: the still's seconds instead of its frames (0: frames)")
    parser.add_argument("--settle-cap-s", type=float, default=0.0,
                        help="METRUM_STREAM_SETTLE_CAP_S: start a phase unsettled after this long (0: wait)")
    parser.add_argument("--detail", action="store_true", help="perf channel on: shares, not times")
    parser.add_argument("--digests", action="store_true", help="settle-point digests (untimed: reads back)")
    parser.add_argument("--shadows", default="", help="METRUM_STREAM_SHADOWS: a run's (a series' branch rows') "
                        "sun-shadow variant sequence, e.g. full,none,none,full")
    parser.add_argument("--gpu-profile", action="store_true", help="Godot's render timestamps per pass (gpu_passes)")
    parser.add_argument("--drawn", action="store_true", help="the drawn probe (instrumented: entered-to-drawn, holes)")
    parser.add_argument("--shader-cache", help="keep the shader and pipeline caches in <dir>/<label or side> across "
                        "runs: seeded from it when present (warm), saved back after a passed run (RENDER-43)")
    parser.add_argument("--menu-dwell", type=float, help="METRUM_STREAM_MENU_DWELL_S: launch through the main menu, "
                        "which stays this many seconds from the launch before the benchmark's world (RENDER-43)")
    parser.add_argument("--as-played", action="store_true",
                        help="unpinned (or in --cpus) with the game's default pools, RAYON_NUM_THREADS unset; "
                        "samples the main thread's scheduling (RENDER-27)")
    parser.add_argument("--cpus", help="run the game inside this CPU set: a taskset list or a profile named in "
                        "the local timing settings' cpu_profiles (a machine profile); samples the main thread")
    parser.add_argument("--display-driver", choices=["x11", "wayland"], default="x11",
                        help="Godot's display driver (RENDER-62); wayland exposes gamescope's socket and fails "
                        "a row on an exit hang. Pinned acceptance rows stay x11")
    parser.add_argument("--settle-timeout", type=int, default=180)
    parser.add_argument("--timeout", type=int, default=600)


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    add_run_options(commands.add_parser("run"), False)
    add_run_options(commands.add_parser("series"), True)
    summary = commands.add_parser("summary")
    summary.add_argument("metrics")
    summary.add_argument("--scripts", action="store_true")
    rows = commands.add_parser("compare")
    rows.add_argument("dir", help="the directory of the series' metrics")
    rows.add_argument("inputs", nargs="+", help="input labels: <dir>/<input>-ctl1.metrics.json ...")
    rows.add_argument("--gpu", action="store_true",
                      help="also the GPU p50 delta and the shadow passes' primitives and draws (RENDER-15)")
    digest = commands.add_parser("digests")
    digest.add_argument("a")
    digest.add_argument("b")
    digest.add_argument("--levels", help="a report whose graded_levels apply (for reports without them)")
    probe = commands.add_parser("drawn")
    probe.add_argument("metrics", nargs="+", help="runs with --drawn")
    probe.add_argument("--bar", type=int, default=3, help="frames: the p95 and hole bar (RENDER-32-N: 3)")
    blocks = commands.add_parser("split")
    blocks.add_argument("metrics", help="a run with a `full,X,X,full` METRUM_STREAM_SHADOWS sequence")
    flown = commands.add_parser("flight", help="a flight run's terrain streaming (TERRAIN-41)")
    flown.add_argument("metrics", nargs="+", help="runs with --phases flight_low, flight_high or flight_jump")
    flown.add_argument("--every", type=float, default=1.0, help="seconds between backlog lines")
    still = commands.add_parser("stills").add_subparsers(dest="stills_command", required=True)
    shot = still.add_parser("capture")
    shot.add_argument("--label", required=True)
    shot.add_argument("--out", required=True)
    shot.add_argument("--godot-dir", default=str(ROOT / "godot"))
    shot.add_argument("--library")
    shot.add_argument("--scripts")
    shot.add_argument("--save")
    shot.add_argument("--world", default=str(ROOT / "godot/bootstrap/worlds/kuopio_18432m_8m.sqlite"))
    shot.add_argument("--poses", required=True, help="idle-matrix poses, comma separated")
    shot.add_argument("--views", default="", help="extra views, `name:x,z,yaw,pitch,distance;...`")
    shot.add_argument("--variants", default="full", help="idle-matrix variants, comma separated")
    shot.add_argument("--hour", type=int, default=10)
    shot.add_argument("--warmup", type=int, default=30)
    shot.add_argument("--resolution", default="1920x1080")
    shot.add_argument("--timeout", type=int, default=900)
    pair = still.add_parser("compare")
    pair.add_argument("a")
    pair.add_argument("b")
    kinds = still.add_parser("variants")
    kinds.add_argument("dir", help="one capture: each <pose>-<variant>.png against <pose>-full.png")
    kinds.add_argument("--crops", help="write the most-changed window side by side here")
    test = still.add_parser("systematic")
    for name in ("ctl1", "ctl2", "br1", "br2"):
        test.add_argument(name)
    test.add_argument("--masks", help="write each still's systematic pixels as a mask PNG here")
    census = commands.add_parser("census", help="a run's draw census (`--phases census`, RENDER-44)")
    census.add_argument("dir", help="the run's <label>.census directory")
    args = parser.parse_args(argv)
    args.argv = argv
    if args.command in ("run", "series"):
        return run(args)
    if args.command == "compare":
        return compare(args)
    if args.command == "digests":
        return digests(args)
    if args.command == "split":
        return split(args)
    if args.command == "drawn":
        return drawn(args)
    if args.command == "flight":
        return flight(args)
    if args.command == "stills":
        return stills(args)
    if args.command == "census":
        import draw_census
        return draw_census.report(args.dir)
    print_summary(json.loads(Path(args.metrics).read_text()), args.scripts)
    return 0


if __name__ == "__main__":
    sys.exit(main())
