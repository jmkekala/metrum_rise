#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-only
"""Run the resident parking capture in a uniquely named, isolated Godot profile."""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "captures/parking")
    parser.add_argument("--headless", action="store_true", help="Validate setup without rendered PNGs")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    profile = Path(tempfile.mkdtemp(prefix="metrum-parking-capture-"))
    project = profile / "godot"
    project.mkdir()
    for entry in (ROOT / "godot").iterdir():
        if entry.name not in {"project.godot", "override.cfg"}:
            (project / entry.name).symlink_to(entry)
    shutil.copy2(ROOT / "godot/project.godot", project / "project.godot")
    name = "MetrumParkingCapture-" + profile.name
    (project / "override.cfg").write_text(
        f'[application]\nconfig/name="{name}"\nconfig/use_custom_user_dir=false\n'
    )
    for entry in ROOT.iterdir():
        if not entry.name.startswith(".") and entry.name != "godot":
            (profile / entry.name).symlink_to(entry)
    if sys.platform == "darwin":
        data = Path.home() / "Library/Application Support/Godot/app_userdata"
    else:
        data = Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local/share")) / "godot/app_userdata"
    source = data / "Metrum Rise/mods"
    user = data / name
    mods = user / "mods"
    mods.mkdir(parents=True)
    # Read the player's requested packs only; all fixture edits stay in these private copies.
    for pack in ("neukku", "kuopio"):
        shutil.copytree(source / pack, mods / pack)
    shutil.copytree(ROOT / "godot/bootstrap/mods/parking", mods / "parking")
    selected = {
        "building.residential.lamelli_8", "building.residential.pistetalo_12",
        "building.residential.house_1", "building.residential.house_2",
        "building.residential.house_3", "building.residential.house_4",
    }
    for manifest in mods.rglob("asset.toml"):
        if manifest.parent.name not in selected:
            continue
        text = manifest.read_text()
        text = text.replace('placement_mode = "zoned_private"', 'placement_mode = "explicit"\nservice_class = "parking_capture"')
        text = re.sub(r"^(?:zone_type|density|economy_profile)\s*=.*\n", "", text, flags=re.M)
        text = re.sub(r"^worker_capacity\s*=.*\n", "", text, flags=re.M)
        if not re.search(r"^household_capacity\s*=", text, flags=re.M):
            text = text.replace('[building]', '[building]\nhousehold_capacity = 8')
        manifest.write_text(text)
    (user / "active_packs.cfg").write_text('[packs]\nenabled=["parking", "neukku", "kuopio"]\n')
    env = os.environ.copy()
    env["METRUM_PARKING_CAPTURE_DIR"] = str(output)
    command = ["perl", "-e", "alarm 900; exec @ARGV", "godot", "--path", str(project)]
    command += ["--headless"] if args.headless else ["--windowed", "--resolution", "1600x900", "--position", "80,80"]
    command += ["--script", "res://tests/parking_capture.gd"]
    (output / "profile.json").write_text(json.dumps({"project": str(project), "user": str(user)}, indent=2))
    with (output / "capture.log").open("w") as log:
        result = subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT)
    log_text = (output / "capture.log").read_text()
    if result.returncode or "SCRIPT ERROR:" in log_text or "ERROR:" in log_text:
        print(log_text[-12000:], file=sys.stderr)
        return result.returncode or 1
    print(f"Parking capture: {output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
