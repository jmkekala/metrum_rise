#!/usr/bin/env python3
"""The graded drag probe's runner (`TERRAIN-23`, docs/terrain.md T60, `RENDER-32`): the gameplay
harness's `graded_drag` matrix (godot/scripts/benchmarks/graded_drag_probe.gd) in an isolated
profile with a copy of the player's packs (`godot_isolated.py`, `TOOLS-19`), rendered in
gamescope's headless backend at 1920x1080 (or `--headless`, which renders nothing: the flow only).

It fails loudly: the exit status is non-zero when Godot's own status is (gamescope exits 0 whatever
its child returns, so the status comes back through a file), when the run hit a `SCRIPT ERROR`
(stopped at once: a GDScript error does not end Godot), when the log shows an instance-uniform
overflow, or when the report is missing or not `success`. The probe's failures are printed.

`--library` and `--scripts` run another build (a control); `--env NAME=VALUE` passes the probe's
settings (`METRUM_GRADED_DRAG_CASES`, `_SWEEP`, `_SWEEP_CONVERGE`, `_HOLE_PIXELS`, `_SAVE`,
`_STROKES`, `_CONTINUE`, ...). Untimed: it takes the shared timing lock, and the run lock with
`--run-lock` (a 30k+ city).

    tools/graded_drag_probe.py --out <dir> --label br-kuopio --env METRUM_GRADED_DRAG_CASES=kuopio_08
"""

import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
import godot_isolated  # noqa: E402  (the isolated profile, TOOLS-19)

ROOT = Path(__file__).resolve().parent.parent
LOCKS = ROOT / "rust/target"
# A log line that fails the run: a cell drawn with failed instance uniforms renders wrong.
LOG_FAILURES = ["Too many instances using shader instance variables"]


def failures(report):
    """The probe's failures from its report, one line each."""
    out = []
    if report.get("error"):
        out.append(str(report["error"]))
    for stroke in report.get("strokes", []):
        for failure in stroke.get("failures", []):
            out.append(f"{stroke.get('case')} attempt {stroke.get('attempt')}: {failure}")
    out.extend(str(failure) for failure in report.get("failures", []))
    return out


def verdict(status, log_text, report):
    """The run's failures: Godot's status, script errors, log failures, the report's."""
    out = []
    if status != 0:
        out.append(f"Godot exited {status}")
    if "SCRIPT ERROR" in log_text:
        out.append("the run hit a SCRIPT ERROR")
    for pattern in LOG_FAILURES:
        if pattern in log_text:
            out.append(f"the log shows '{pattern}'")
    if report is None:
        out.append("no report was written")
    else:
        if not report.get("success", False):
            out.append("the report is not a success")
        out.extend(failures(report))
    return out


def lock(path, exclusive):
    handle = open(path, "a")
    fcntl.flock(handle, fcntl.LOCK_EX if exclusive else fcntl.LOCK_SH)
    return handle


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", required=True, help="the directory of the run's report and logs")
    parser.add_argument("--label", required=True)
    parser.add_argument("--godot-dir", default=str(ROOT / "godot"))
    parser.add_argument("--library", help="default: the godot directory's deployed library")
    parser.add_argument("--scripts", help="another build's godot/scripts (a control)")
    parser.add_argument("--headless", action="store_true", help="no rendering: the flow only")
    parser.add_argument("--env", action="append", default=[], help="NAME=VALUE for the probe")
    parser.add_argument("--run-lock", action="store_true", help="also hold the run lock (a 30k+ city)")
    parser.add_argument("--timeout", type=int, default=1800)
    args = parser.parse_args(argv)
    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%Y%m%dT%H%M%S")
    run_dir = out / f"drag-{args.label}-{stamp}"
    run_dir.mkdir()
    metrics = run_dir / "metrics.json"
    log = run_dir / "godot.log"
    status_file = run_dir / "status"
    try:
        profile, project, _name, env = godot_isolated.create(
            out, prefix="drag.", godot_dir=Path(args.godot_dir).resolve(), name_prefix="MetrumGradedDrag",
            mods=godot_isolated.player_mods(),
            library=Path(args.library).resolve() if args.library else None,
            # A control keeps its own probe: the probe reads its renderer's internals.
            scripts=Path(args.scripts).resolve() if args.scripts else None)
    except godot_isolated.NotIsolated as error:
        print(f"graded_drag_probe: {error}", file=sys.stderr)
        return 2
    # The terrain manifest names its files from the repository root (res://..), the source world
    # under godot/, which the project copy's profile does not link (it links godot's siblings).
    if not (profile / "godot").exists():
        (profile / "godot").symlink_to(Path(args.godot_dir).resolve())
    revision = subprocess.run(["git", "-C", str(ROOT), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    env.update({
        "METRUM_GAMEPLAY_BENCHMARK_MODE": "headless" if args.headless else "windowed",
        "METRUM_GAMEPLAY_BENCHMARK_MATRIX": "graded_drag",
        "METRUM_GAMEPLAY_BENCHMARK_RUN_ID": f"graded-drag-{args.label}",
        "METRUM_GAMEPLAY_BENCHMARK_METRICS_PATH": str(metrics),
        "METRUM_GAMEPLAY_BENCHMARK_PROFILED": "0", "METRUM_GAMEPLAY_BENCHMARK_GPU_PROFILED": "0",
        "METRUM_GAMEPLAY_BENCHMARK_GIT_REVISION": revision, "METRUM_DEBUG": "0",
    })
    for item in args.env:
        name, _, value = item.partition("=")
        env[name] = value
    godot = ["godot", "--path", str(project), "--log-file", str(log)]
    godot += ["--headless"] if args.headless else ["--windowed", "--resolution", "1920x1080"]
    godot += ["--", "--gameplay-road-benchmark"]
    command = ["sh", "-c", '"$@"; echo $? > "$0"', str(status_file)] + godot
    if not args.headless:
        command = ["gamescope", "--backend", "headless", "-W", "1920", "-H", "1080", "--"] + command
    command = ["taskset", "-c", "0,2,4,6,8,10,12,14", "timeout", "--kill-after=10s", str(args.timeout)] + command
    (run_dir / "identity.txt").write_text(
        f"tree {revision}\nlibrary {args.library or 'the godot directory'}\nscripts {args.scripts or 'the tree'}\n"
        f"env {' '.join(args.env)}\n")
    held = [lock(LOCKS / ".run.lock", True)] if args.run_lock else []
    held.append(lock(LOCKS / ".timing.lock", False))
    stdout = open(run_dir / "out.txt", "w")
    process = subprocess.Popen(command, env=env, stdout=stdout, stderr=subprocess.STDOUT, start_new_session=True)
    stopped = False
    while process.poll() is None:
        time.sleep(2)
        text = (run_dir / "out.txt").read_text(errors="replace")
        if "SCRIPT ERROR" in text:
            os.killpg(process.pid, 15)
            stopped = True
            break
    process.wait()
    stdout.close()
    for handle in held:
        handle.close()
    status = int(status_file.read_text().strip()) if status_file.exists() and status_file.read_text().strip() else -1
    text = (run_dir / "out.txt").read_text(errors="replace") + (log.read_text(errors="replace") if log.exists() else "")
    report = json.loads(metrics.read_text()) if metrics.exists() else None
    problems = verdict(status, text, report)
    if stopped:
        problems.insert(0, "stopped at its first SCRIPT ERROR")
    summary = report.get("summary") if report else None
    lines = [f"graded_drag_probe {args.label}: {'PASS' if not problems else 'FAIL'}",
             f"summary {json.dumps(summary)}"] + [f"  {problem}" for problem in problems]
    (run_dir / "verdict.txt").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))
    if not problems:
        shutil.rmtree(profile, ignore_errors=True)
    else:
        print(f"graded_drag_probe: profile kept: {profile}", file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
