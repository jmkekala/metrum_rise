#!/usr/bin/env python3
"""Record the native camera's response in an isolated, synthetic Godot project.

Uses an explicitly selected, copied GDExtension binary; never builds or deploys one.
Headless traces at 30/60/120 fixed FPS test response consistency. An optional GPU
capture produces a 60 FPS replay, not a performance benchmark or input-routing test.
"""

import argparse
import csv
import hashlib
import json
import math
from pathlib import Path
import shutil
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import godot_isolated  # noqa: E402


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def read_trace(path):
    with path.open() as stream:
        return list(csv.DictReader(stream))


def group_trace(rows):
    groups = {}
    for row in rows:
        groups.setdefault(row["trial"], []).append(row)
    return groups


def summarize(rows):
    summary = {}
    for name, samples in group_trace(rows).items():
        rate = round(1 / (float(samples[1]["time_s"]) - float(samples[0]["time_s"])))
        summary[name] = {
            "frames": len(samples),
            "max_eye_step_m": max(float(r["speed_m_s"]) / rate for r in samples),
            "peak_vertical_speed_m_s": max(abs(float(r["vertical_speed_m_s"])) for r in samples),
            "minimum_ground_clearance_m": min(float(r["clearance_m"]) for r in samples),
            "first_active_speed_m_s": next(float(r["speed_m_s"]) for r in samples if r["input"] != "idle"),
            "end_pivot": [float(samples[-1][f"pivot_{axis}"]) for axis in "xyz"],
            "end_distance_m": float(samples[-1]["distance"]),
        }
        events = []
        for before, after in zip(samples, samples[1:]):
            if after["input"].startswith("wheel"):
                events.append({
                    "time_s": float(after["time_s"]),
                    "distance_before_m": float(before["distance"]),
                    "distance_after_m": float(after["distance"]),
                    "landmark_shift_px": math.hypot(
                        float(after["landmark_x_px"]) - float(before["landmark_x_px"]),
                        float(after["landmark_y_px"]) - float(before["landmark_y_px"])),
                })
        if events:
            summary[name]["wheel_events"] = events
    return summary


def run_logged(command, log, env, timeout=300):
    with log.open("w") as stream:
        subprocess.run(command, env=env, stdout=stream, stderr=subprocess.STDOUT,
                       check=True, timeout=timeout)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--library", required=True, type=Path, help="GDExtension .so to snapshot")
    parser.add_argument("--output", required=True, type=Path, help="Fresh artifact directory")
    parser.add_argument("--video", action="store_true", help="Render using gamescope headless and encode with ffmpeg")
    parser.add_argument("--build-description", default="unknown: explicit binary, not rebuilt by this harness")
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[1]
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    project = output / "project"
    (project / "bin").mkdir(parents=True)
    (project / ".godot").mkdir()
    library = project / "bin/libmetrum_rise.so"
    shutil.copy2(args.library, library)
    shutil.copy2(repo / "godot/bin/metrum_rise.gdextension", library.with_name("metrum_rise.gdextension"))
    shutil.copy2(repo / "godot/tests/local_camera_playtest.gd", project / "playtest.gd")
    (project / ".godot/extension_list.cfg").write_text("res://bin/metrum_rise.gdextension\n")
    (project / "project.godot").write_text('''config_version=5
[application]
config/name="Metrum Camera Playtest"
[display]
window/size/viewport_width=960
window/size/viewport_height=540
[rendering]
renderer/rendering_method="gl_compatibility"
''')
    # Absolute XDG homes in the output (`TOOLS-19`): the run never touches the home profile.
    env = godot_isolated.isolated_env(output, display=True)
    env.update({"METRUM_DEBUG": "0", "RAYON_NUM_THREADS": "4"})
    metadata = {
        "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip(),
        "camera_source_sha256": digest(repo / "rust/src/nodes/camera_node.rs"),
        "motion_source_sha256": digest(repo / "rust/src/nodes/camera_node/motion.rs"),
        "library_sha256": digest(library),
        "library_origin": str(args.library.resolve()),
        "library_build_description": args.build_description,
        "godot": subprocess.check_output(["godot", "--version"], text=True).strip(),
        "fixture_sha256": digest(project / "playtest.gd"),
        "resolution": [960, 540], "rayon_workers": 4,
        "scope": "Native camera API, synthetic terrain; no UI routing, city workload or wall-time performance acceptance",
        "commands": [],
    }
    traces = {}
    for tag, rate in [("30", 30), ("60", 60), ("120", 120), ("60-repeat", 60)]:
        trial = output / tag
        trial.mkdir()
        env.update({"METRUM_CAMERA_OUTPUT": str(trial), "METRUM_CAMERA_FPS": str(rate), "METRUM_CAMERA_CAPTURE": "0"})
        command = ["godot", "--headless", "--path", str(project), "--fixed-fps", str(rate), "--script", "res://playtest.gd"]
        metadata["commands"].append(command)
        run_logged(command, trial / "godot.log", env)
        rows = read_trace(trial / "trace.csv")
        if len(rows) != 22 * rate:
            raise RuntimeError(f"Incomplete trace: {tag}: {len(rows)} frames")
        traces[tag] = rows
    if traces["60"] != traces["60-repeat"]:
        raise RuntimeError("Repeated 60 FPS camera traces differ")
    # Endpoints should be stable even though first-step displacement depends on frame duration.
    fields = ["pivot_x", "pivot_y", "pivot_z", "yaw", "pitch", "distance", "eye_x", "eye_y", "eye_z"]
    reference = group_trace(traces["60"])
    max_endpoint_difference = 0.0
    for tag in ["30", "120"]:
        for name, samples in group_trace(traces[tag]).items():
            for field in fields:
                difference = abs(float(samples[-1][field]) - float(reference[name][-1][field]))
                max_endpoint_difference = max(max_endpoint_difference, difference)
                if difference > 0.002:
                    raise RuntimeError(f"Frame-rate-dependent endpoint: {tag} {name} {field}: {difference}")
    if args.video:
        trial = output / "rendered-60"
        (trial / "frames").mkdir(parents=True)
        env.update({"METRUM_CAMERA_OUTPUT": str(trial), "METRUM_CAMERA_FPS": "60", "METRUM_CAMERA_CAPTURE": "1"})
        env.pop("DISPLAY", None)
        env.pop("WAYLAND_DISPLAY", None)
        status = trial / "exit-status"
        command = ["gamescope", "--backend", "headless", "-W", "960", "-H", "540", "--",
                   "sh", "-c", '"$@"; result=$?; echo "$result" > "$0"; exit "$result"', str(status),
                   "godot", "--path", str(project), "--display-driver", "x11", "--audio-driver", "Dummy",
                   "--fixed-fps", "60", "--script", "res://playtest.gd"]
        metadata["commands"].append(command)
        run_logged(command, trial / "godot.log", env, timeout=600)
        if not status.exists() or status.read_text().strip() != "0":
            raise RuntimeError("Rendered Godot failed; see rendered-60/godot.log")
        if read_trace(trial / "trace.csv") != traces["60"]:
            raise RuntimeError("Rendered and headless motion traces differ")
        if len(list((trial / "frames").glob("*.png"))) != 1320:
            raise RuntimeError("Missing rendered frames")
        command = ["ffmpeg", "-v", "error", "-framerate", "60", "-i", str(trial / "frames/%05d.png"),
                   "-c:v", "libx264", "-crf", "18", "-pix_fmt", "yuv420p", "-movflags", "+faststart",
                   str(output / "camera-replay.mp4")]
        metadata["commands"].append(command)
        run_logged(command, trial / "encode.log", env)
    metadata["checks"] = {"repeat_60_bitwise_equal": True, "max_endpoint_difference": max_endpoint_difference,
                          "rendered_matches_headless": True if args.video else None}
    (output / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
    (output / "summary.json").write_text(json.dumps(summarize(traces["60"]), indent=2) + "\n")
    print(f"Camera playtest passed. Artifacts: {output}")


if __name__ == "__main__":
    main()
