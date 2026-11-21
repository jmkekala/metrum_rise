#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Drafts a zoned building asset from a `metrum_models` facts sidecar (`TOOLS-33`,
docs/asset_editor.md "Draft command").

    tools/model_draft.py --sidecar <model>/export/<name>.model.json --role <role.json> \\
        --mods <packs folder> [--write] [--report <report.json>]

Godot runs headless in a throwaway profile (tools/godot_isolated.py), never in the player's. The
role's pack is copied from `--mods` into that profile, and the draft is published and checked
there by `AssetAuthoringFiles.draft_from_model`, where every rule lives. Without `--write` nothing
outside the profile is written (besides `--report`). With `--write` the draft is published once
more into `<mods>/<pack_id>`, and only when the copy passed the acceptance check. An asset that
already exists is never replaced: the stale-geometry report comes back instead.

A role without `pack_id` takes the only pack in `--mods`; with none or several it is refused.
Paths inside the profile's pack copy are reported as their `--mods` paths, since the profile is
deleted after the run, except the report's `copy` paths: they named the deleted copy, so only
`copy.passed` is kept (all of them with `--keep-profile`). A dry run's report says `dry_run: true`,
and `would_write` names where `--write` would publish.

Exit status: 0 drafted (and written), or an existing asset with no stale line (information lines
may be reported); 1 refused, failed or not run; 2 an existing asset with stale lines reported.
"""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

import godot_isolated

SCRIPT = "res://scripts/editors/asset_editor/model_draft.gd"
# What the game's `is_valid_pack_id` accepts; the Rust role reader says the same.
PACK_ID_RULE = ("a pack id is the name of the pack's folder in --mods, made of lowercase letters, digits "
                "and hyphens, not starting or ending with a hyphen")


def valid_pack_id(pack_id):
    return (isinstance(pack_id, str) and pack_id != "" and not pack_id.startswith("-")
            and not pack_id.endswith("-") and all(c in "abcdefghijklmnopqrstuvwxyz0123456789-" for c in pack_id))


def label(role):
    """`<pack_id>:<asset_id>` of role inputs that may be incomplete, as the draft names them."""
    field = lambda key: role.get(key) if isinstance(role.get(key), str) else f"(no {key})"
    return f"{field('pack_id')}:{field('asset_id')}"


def with_pack_id(role, mods):
    """The role with `pack_id` filled in when it gives none: the only pack in `mods` (a folder,
    not a link and not a dot-name, holding `pack.toml`). None or several packs is a refusal."""
    if "pack_id" in role:
        return role
    packs = sorted(path.name for path in Path(mods).iterdir()
                   if not path.name.startswith(".") and path.is_dir() and not path.is_symlink()
                   and (path / "pack.toml").is_file())
    if len(packs) != 1:
        raise ValueError(f"the role gives no pack_id and --mods holds {len(packs)} packs "
                         f"({', '.join(packs) or 'none'}); name one in pack_id")
    return dict(role, pack_id=packs[0])


def unprofile(value, copy, mods):
    """`value` with every path inside the profile's pack copy `copy` named by its path in `mods`:
    the copy is the pack in `mods`, and the profile is deleted after the run."""
    if isinstance(value, str):
        for prefix in dict.fromkeys((copy, os.path.realpath(copy))):
            value = mods if value == prefix else value.replace(prefix + os.sep, mods + os.sep)
        return value
    if isinstance(value, list):
        return [unprofile(item, copy, mods) for item in value]
    if isinstance(value, dict):
        return {key: unprofile(item, copy, mods) for key, item in value.items()}
    return value


def copy_pack(mods, pack_id, destination):
    """Copies `<mods>/<pack_id>` into `destination` (links kept as links, which the draft then
    refuses), without the pack's own `.git` entry (`godot_isolated.without_git`). A missing pack
    copies nothing: the draft refuses it by name."""
    if not valid_pack_id(pack_id):
        raise ValueError(f"role pack_id {pack_id!r} is not a pack id: {PACK_ID_RULE}")
    destination.mkdir(parents=True, exist_ok=True)
    source = Path(mods) / pack_id
    if source.is_dir() and not source.is_symlink():
        shutil.copytree(source, destination / pack_id, symlinks=True,
                        ignore=godot_isolated.without_git(source, 0))


def request(args, role, profile):
    """The draft request: native absolute paths, the role inline, the real packs folder only
    with `--write`."""
    return {
        "sidecar": os.path.abspath(args.sidecar),
        "role": role,
        "mods": str(profile / "draft-mods"),
        "work": str(profile / "draft-work"),
        "write_mods": os.path.abspath(args.mods) if args.write else None,
        "target_mods": os.path.abspath(args.mods),
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--sidecar", required=True, help="the model's <name>.model.json")
    parser.add_argument("--role", required=True, help="the wanted asset's role inputs (JSON)")
    parser.add_argument("--mods", required=True, help="the packs folder holding the role's pack")
    parser.add_argument("--write", action="store_true",
                        help="publish into <mods>/<pack_id> after the copy passes the acceptance check")
    parser.add_argument("--report", help="also write the JSON report here")
    parser.add_argument("--godot", default="godot", help="the Godot binary")
    parser.add_argument("--profile-root", help="where the throwaway profile is made (default: the temporary directory)")
    parser.add_argument("--keep-profile", action="store_true", help="keep the profile after the run")
    args = parser.parse_args(argv)
    try:
        role = json.loads(Path(args.role).read_text(encoding="utf-8"))
        if not isinstance(role, dict):
            raise ValueError("not a JSON object")
    except (OSError, ValueError) as error:
        print(f"model_draft: cannot read the role inputs {args.role}: {error}", file=sys.stderr)
        return 1
    try:
        role = with_pack_id(role, args.mods)
    except (OSError, ValueError) as error:
        print(f"model_draft: {label(role)}: refused: {error}", file=sys.stderr)
        return 1
    try:
        profile, project, _, env = godot_isolated.create(
            Path(args.profile_root).resolve() if args.profile_root else None, "metrum-model-draft.")
    except godot_isolated.NotIsolated as error:
        print(f"model_draft: {error}", file=sys.stderr)
        return 1
    keep = args.keep_profile
    try:
        copy_pack(args.mods, role["pack_id"], profile / "draft-mods")
        request_path = profile / "draft-request.json"
        report_path = profile / "draft-report.json"
        request_path.write_text(json.dumps(request(args, role, profile), indent=2), encoding="utf-8")
        run = subprocess.run(
            [args.godot, "--headless", "--path", str(project), "--script", SCRIPT, "--",
             str(request_path), str(report_path)],
            env=env, stdin=subprocess.DEVNULL, capture_output=True, text=True)
        if not report_path.is_file():
            print(run.stdout + run.stderr, file=sys.stderr)
            print(f"model_draft: {label(role)}: Godot exited {run.returncode} without a report", file=sys.stderr)
            keep = True
            return 1
        report = json.loads(report_path.read_text(encoding="utf-8"))
        copy_paths = report.get("copy")
        report = unprofile(report, str(profile / "draft-mods"), os.path.abspath(args.mods))
        if isinstance(copy_paths, dict):
            # Unprofiled, the copy's paths would name the real pack, which a dry run never wrote.
            report["copy"] = (dict(copy_paths, kept=True) if keep
                              else {"passed": copy_paths.get("passed"), "kept": False})
        print(report.get("summary", ""))
        if args.report:
            Path(args.report).write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        return run.returncode
    except ValueError as error:
        print(f"model_draft: {label(role)}: refused: {error}", file=sys.stderr)
        return 1
    finally:
        if keep:
            print(f"model_draft: profile kept: {profile}", file=sys.stderr)
        else:
            shutil.rmtree(profile, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
