#!/usr/bin/env python3
"""The rendered camera-move benchmark (`PERF-23`, docs/terrain.md "Streaming core").

`run` launches Godot's `stream` benchmark matrix (godot/scripts/benchmarks/stream_frame_benchmark.gd)
in gamescope's headless backend at 1000 Hz, in an isolated profile, pinned to the timing CPUs with
eight workers, under the timing locks and outside the quiet windows of the local timing settings
(`timing_session.py`'s). `--library` and `--scripts` run another build's library and scripts on
this tree's assets: a control is `--library <control .so> --scripts <its godot/scripts>`, with
this tree's benchmark scripts copied over its own. `summary` prints a report's per-phase table;
with `--scripts` (an instrumented run, `--detail`) also each phase's mean milliseconds per frame
by renderer, from the perf records the benchmark joined to each frame.

    ./run.sh --benchmark-stream                     # builds release, then `run` on Kuopio
    tools/stream_benchmark.py run --label c30k-br1 --save <populated_city_30000.sqlite> --out <dir>
    tools/stream_benchmark.py summary <dir>/c30k-br1.metrics.json [--scripts]
"""

import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
import timing_session  # noqa: E402  (shares the machine settings and window checks)

ROOT = Path(__file__).resolve().parent.parent
# The benchmark's own scripts: copied over another build's scripts so both sides measure alike.
BENCH_SCRIPTS = ["benchmarks/stream_frame_benchmark.gd", "benchmarks/gameplay_road_benchmark.gd",
                 "core/perf_debug.gd"]
WORKERS = "8"


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def player_mods():
    data = os.environ.get("XDG_DATA_HOME") or str(Path.home() / ".local/share")
    return Path(data) / "godot/app_userdata/Metrum Rise/mods"


def make_project(profile, godot_dir, library, scripts, mods):
    """An isolated copy of the project: every entry linked, scripts copied when overridden, and
    a copy of the packs at `mods` (if any) in the profile, so the run never writes the player's."""
    project = profile / "project"
    (project / "bin").mkdir(parents=True)
    for entry in list(godot_dir.iterdir()):
        if entry.name in ("project.godot", "override.cfg", "bin"):
            continue
        if entry.name == "scripts" and scripts is not None:
            shutil.copytree(scripts, project / "scripts", symlinks=True)
            for name in BENCH_SCRIPTS:
                shutil.copy2(godot_dir / "scripts" / name, project / "scripts" / name)
            continue
        (project / entry.name).symlink_to(entry)
    # Some scripts reach repo siblings through res://../<dir> (maps, economy).
    for entry in godot_dir.parent.iterdir():
        if entry != godot_dir:
            (profile / entry.name).symlink_to(entry)
    shutil.copy2(godot_dir / "project.godot", project / "project.godot")
    shutil.copy2(godot_dir / "bin/metrum_rise.gdextension", project / "bin/metrum_rise.gdextension")
    shutil.copy2(library, project / "bin/libmetrum_rise.so")
    name = f"MetrumStream-{profile.name}"
    (project / "override.cfg").write_text(
        f'[application]\nconfig/name="{name}"\nconfig/use_custom_user_dir=false\n')
    if mods is not None and mods.is_dir():
        shutil.copytree(mods, profile / f"data/godot/app_userdata/{name}/mods", symlinks=True)
    return project


def wait_for_quiet(settings, limit_s):
    """Waits while a quiet window is near or a busy process runs; the run is one row of at most
    `limit_s`."""
    names = sorted(set(["cargo", "rustc"] + settings.get("busy_processes", [])))
    while True:
        window = timing_session.in_windows(datetime.datetime.now(), settings.get("quiet_windows", []),
                                           limit_s)
        busy = timing_session.busy_processes(names)
        if not window and not busy:
            return
        print(f"stream_benchmark: waiting ({window or ''} {' '.join(busy)})", flush=True)
        time.sleep(30)


def run(args):
    settings = timing_session.load_settings()
    godot_dir = Path(args.godot_dir).resolve()
    library = Path(args.library or godot_dir / "bin/libmetrum_rise.so").resolve()
    scripts = Path(args.scripts).resolve() if args.scripts else None
    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    metrics = out / f"{args.label}.metrics.json"
    if metrics.exists():
        print(f"stream_benchmark: {metrics} exists; choose a new label", file=sys.stderr)
        return 2
    save = str(Path(args.save).resolve()) if args.save else ""
    world = str(Path(args.world).resolve())
    profile = Path(tempfile.mkdtemp(prefix="stream.", dir=out))
    try:
        project = make_project(profile, godot_dir, library, scripts, player_mods())
        identity = {
            "label": args.label, "library": str(library), "library_sha256": sha256(library),
            "scripts": str(scripts or godot_dir / "scripts"),
            "benchmark_sha256": sha256(godot_dir / "scripts" / BENCH_SCRIPTS[0]),
            "revision": subprocess.run(["git", "-C", str(godot_dir), "rev-parse", "HEAD"],
                                       capture_output=True, text=True).stdout.strip(),
            "input": save or world, "input_sha256": sha256(save)[:16] if save else "",
            "resolution": args.resolution, "speed": args.speed, "detail": args.detail,
            "phases": args.phases, "cpus": settings.get("cpus", ""), "workers": WORKERS,
        }
        width, height = args.resolution.split("x")
        env = dict(os.environ)
        for name in ("DISPLAY", "WAYLAND_DISPLAY"):
            env.pop(name, None)
        env.update({
            "XDG_DATA_HOME": str(profile / "data"), "XDG_CONFIG_HOME": str(profile / "config"),
            "RAYON_NUM_THREADS": WORKERS, "METRUM_DEBUG": "0", "METRUM_DEBUG_PERF": "1" if args.detail else "0",
            "METRUM_CRASH_DIAGNOSTICS": "0",
            "METRUM_GAMEPLAY_BENCHMARK_MODE": "windowed", "METRUM_GAMEPLAY_BENCHMARK_MATRIX": "stream",
            "METRUM_GAMEPLAY_BENCHMARK_RUN_ID": args.label, "METRUM_GAMEPLAY_BENCHMARK_WORLD_PATH": world,
            "METRUM_GAMEPLAY_BENCHMARK_METRICS_PATH": str(metrics),
            "METRUM_GAMEPLAY_BENCHMARK_PROFILED": "0", "METRUM_GAMEPLAY_BENCHMARK_GPU_PROFILED": "0",
            "METRUM_GAMEPLAY_BENCHMARK_GIT_REVISION": identity["revision"],
            "METRUM_GAMEPLAY_BENCHMARK_GIT_DIRTY": "0", "METRUM_GAMEPLAY_BENCHMARK_DIFF_SHA256": "none",
            "METRUM_GAMEPLAY_BENCHMARK_TIMEOUT_SEC": str(args.settle_timeout),
            "METRUM_IDLE_BENCH_SAVE_PATH": save, "METRUM_STREAM_SPEED": str(args.speed),
            "METRUM_STREAM_PHASES": args.phases, "METRUM_STREAM_TURNS": args.turns,
        })
        command = ["timeout", "--kill-after=10s", str(args.timeout)]
        if settings.get("cpus"):
            command = ["taskset", "-c", settings["cpus"]] + command
        command += ["gamescope", "--backend", "headless", "-r", "1000", "-W", width, "-H", height, "--",
                    "godot", "--path", str(project), "--log-file", str(out / f"{args.label}.godot.log"),
                    "--windowed", "--resolution", args.resolution, "--", "--gameplay-road-benchmark"]
        command = list(settings.get("timed_wrapper", [])) + command
        wait_for_quiet(settings, args.timeout)
        identity["start"] = datetime.datetime.now().isoformat(timespec="seconds")
        identity["loadavg"] = os.getloadavg()
        with open(out / f"{args.label}.out", "w") as log:
            status = subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT).returncode
        identity["end"] = datetime.datetime.now().isoformat(timespec="seconds")
        identity["status"] = status
        identity["script_errors"] = (out / f"{args.label}.out").read_text(errors="replace").count("SCRIPT ERROR")
        (out / f"{args.label}.identity.json").write_text(json.dumps(identity, indent=1) + "\n")
    finally:
        shutil.rmtree(profile, ignore_errors=True)
    if status == 0 and identity["script_errors"] == 0 and metrics.exists():
        report = json.loads(metrics.read_text())
        if report.get("success"):
            print_summary(report, args.detail)
            return 0
    print(f"stream_benchmark: {args.label} failed (status {status}, "
          f"{identity['script_errors']} script errors); see {out / (args.label + '.out')}", file=sys.stderr)
    return 1


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
    if not scripts:
        return
    for sample in report["samples"]:
        means, _ = script_means(sample["rows"])
        frame = statistics.mean(row["frame_ms"] for row in sample["rows"])
        parts = " ".join(f"{name}={value:.2f}" for name, value in sorted(means.items(), key=lambda kv: -kv[1]))
        print(f"{phase_label(sample):<22} frame {frame:.2f} timed {sum(means.values()):.2f}: {parts}")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    runner = commands.add_parser("run")
    runner.add_argument("--label", required=True)
    runner.add_argument("--out", default=str(ROOT / "benchmark-results"))
    runner.add_argument("--godot-dir", default=str(ROOT / "godot"))
    runner.add_argument("--library", help="default: the godot directory's deployed library")
    runner.add_argument("--scripts", help="another build's godot/scripts (a control)")
    runner.add_argument("--save", help="a save to load; without it the world loads as a new game")
    runner.add_argument("--world", default=str(ROOT / "maps/processed/Kuopio/kuopio_324km2_10m.sqlite"))
    runner.add_argument("--resolution", default="1920x1080")
    runner.add_argument("--speed", type=float, default=0.0)
    runner.add_argument("--phases", default="")
    runner.add_argument("--turns", default="")
    runner.add_argument("--detail", action="store_true", help="perf channel on: shares, not times")
    runner.add_argument("--settle-timeout", type=int, default=180)
    runner.add_argument("--timeout", type=int, default=600)
    summary = commands.add_parser("summary")
    summary.add_argument("metrics")
    summary.add_argument("--scripts", action="store_true")
    args = parser.parse_args(argv)
    if args.command == "run":
        return run(args)
    print_summary(json.loads(Path(args.metrics).read_text()), args.scripts)
    return 0


if __name__ == "__main__":
    sys.exit(main())
