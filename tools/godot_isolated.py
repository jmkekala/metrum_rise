#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-only
"""Runs a Godot test, benchmark, probe or import in a throwaway profile (`TOOLS-19`, docs/debug.md
"Godot Test Guard").

Godot ignores a relative `XDG_DATA_HOME` / `XDG_CONFIG_HOME` and falls back to the home profile,
where a test rewrites the player's logs, editor settings or packs. This launcher makes an
absolute profile directory with its own data, config and cache homes and a project copy of its
own name (every entry of `godot/` linked, `project.godot` copied, an `override.cfg` naming it), so
user:// is the profile's on every platform. It refuses an environment whose XDG homes are
relative or are the home profile's. `run` deletes a passed run's profile and keeps a failed
one's; `profile` only makes one and prints its paths for a shell to `eval`.

    tools/godot_isolated.py run -- godot --headless --path {project} --script res://tests/x.gd
    tools/godot_isolated.py run --library <.so> -- godot --headless --path {project} --import
    tools/godot_isolated.py run --mods player -- gamescope --backend headless -- godot --path {project}
    eval "$(tools/godot_isolated.py profile)"   # METRUM_GODOT_PROJECT, XDG_* for a shell's runs

`{project}` and `{profile}` in the command are replaced by the project copy and profile paths.
The display variables are unset unless `--display` is given.
"""

import argparse
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
XDG_HOMES = {"XDG_DATA_HOME": "data", "XDG_CONFIG_HOME": "config", "XDG_CACHE_HOME": "cache"}
# The home profile's Godot directories, by the XDG variable that would move them.
HOME_DEFAULTS = {"XDG_DATA_HOME": ".local/share", "XDG_CONFIG_HOME": ".config", "XDG_CACHE_HOME": ".cache"}


class NotIsolated(Exception):
    """An environment whose Godot run would read or write the home profile."""


def home_godot_dirs(home=None):
    home = Path(home or Path.home())
    return {name: home / default / "godot" for name, default in HOME_DEFAULTS.items()}


def check_isolated(env, home=None):
    """Raises `NotIsolated` unless every XDG home in `env` is set, absolute, and neither the home
    profile's nor inside one of its Godot directories."""
    real = [path.resolve() for path in home_godot_dirs(home).values()]
    for name in XDG_HOMES:
        value = env.get(name, "")
        if not value:
            raise NotIsolated(f"{name} is unset: Godot would use the home profile")
        path = Path(value)
        if not path.is_absolute():
            raise NotIsolated(f"{name}={value} refused: not absolute, so Godot would ignore it "
                              "and use the home profile")
        godot = (path / "godot").resolve()
        for directory in real:
            if godot == directory or path.resolve().is_relative_to(directory):
                raise NotIsolated(f"{name}={value} refused: it is the home profile's ({directory})")


def isolated_env(profile, base=None, display=False, home=None):
    """`base` (default the process environment) with the XDG homes in `profile`, checked; the
    display variables removed unless `display`."""
    env = dict(os.environ if base is None else base)
    for name, sub in XDG_HOMES.items():
        env[name] = str(Path(profile) / sub)
    if not display:
        for name in ("DISPLAY", "WAYLAND_DISPLAY"):
            env.pop(name, None)
    check_isolated(env, home)
    return env


def player_mods():
    """The player's installed packs (read only, to copy them into a profile)."""
    if sys.platform == "darwin":
        return Path.home() / "Library/Application Support/Godot/app_userdata/Metrum Rise/mods"
    # Godot itself ignores a relative XDG_DATA_HOME, so the packs it loads are the home's then.
    data = os.environ.get("XDG_DATA_HOME", "")
    if not data or not Path(data).is_absolute():
        data = str(Path.home() / ".local/share")
    return Path(data) / "godot/app_userdata/Metrum Rise/mods"


def make_project(profile, godot_dir, library=None, scripts=None, mods=None, name_prefix="MetrumIsolated",
                 own_scripts=()):
    """An isolated copy of the project in `profile`: every entry of `godot_dir` linked,
    `project.godot` copied, an `override.cfg` naming it `<name_prefix>-<profile name>`, and the
    repo's other entries linked beside it (scripts reach them through `res://../<dir>`).
    `library` copies another build's library into a `bin/` of its own (default: `bin/` linked);
    `scripts` copies another tree's scripts with `own_scripts` (paths under scripts/) taken from
    `godot_dir`; `mods` copies those packs into the profile's user://mods."""
    profile = Path(profile)
    godot_dir = Path(godot_dir)
    project = profile / "project"
    project.mkdir()
    for entry in list(godot_dir.iterdir()):
        if entry.name in ("project.godot", "override.cfg"):
            continue
        if entry.name == "bin" and library is not None:
            (project / "bin").mkdir()
            shutil.copy2(godot_dir / "bin/metrum_rise.gdextension", project / "bin/metrum_rise.gdextension")
            shutil.copy2(library, project / "bin/libmetrum_rise.so")
            continue
        if entry.name == "scripts" and scripts is not None:
            shutil.copytree(scripts, project / "scripts", symlinks=True)
            for name in own_scripts:
                shutil.copy2(godot_dir / "scripts" / name, project / "scripts" / name)
            continue
        (project / entry.name).symlink_to(entry)
    for entry in godot_dir.parent.iterdir():
        if entry != godot_dir:
            (profile / entry.name).symlink_to(entry)
    shutil.copy2(godot_dir / "project.godot", project / "project.godot")
    name = f"{name_prefix}-{profile.name}"
    (project / "override.cfg").write_text(
        f'[application]\nconfig/name="{name}"\nconfig/use_custom_user_dir=false\n')
    if mods is not None and Path(mods).is_dir():
        shutil.copytree(mods, user_dir(profile, name) / "mods", symlinks=True)
    return project


def user_dir(profile, name):
    """user:// of project `name` in `profile` (Linux and the BSDs: the data home's)."""
    return Path(profile) / XDG_HOMES["XDG_DATA_HOME"] / "godot/app_userdata" / name


def create(root=None, prefix="metrum-godot.", display=False, **project):
    """Makes a profile under `root` (default the temporary directory) with its project copy;
    returns `(profile, project, name, env)`, `env` as `isolated_env` gives it."""
    profile = Path(tempfile.mkdtemp(prefix=prefix, dir=root)).resolve()
    try:
        env = isolated_env(profile, display=display)
        path = make_project(profile, project.pop("godot_dir", ROOT / "godot"), **project)
    except BaseException:
        shutil.rmtree(profile, ignore_errors=True)
        raise
    return profile, path, f"{project.get('name_prefix', 'MetrumIsolated')}-{profile.name}", env


def substitute(command, profile, project):
    return [part.replace("{project}", str(project)).replace("{profile}", str(profile)) for part in command]


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("run", "profile"):
        sub = commands.add_parser(name)
        sub.add_argument("--root", help="where the profile is made (default: the temporary directory)")
        sub.add_argument("--prefix", default="metrum-godot.", help="the profile directory's name prefix")
        sub.add_argument("--name-prefix", default="MetrumIsolated", help="the project name's prefix")
        sub.add_argument("--godot-dir", default=str(ROOT / "godot"))
        sub.add_argument("--library", help="another build's library (default: the godot directory's bin/)")
        sub.add_argument("--scripts", help="another tree's godot/scripts")
        sub.add_argument("--mods", help="packs to copy into user://mods: a directory or `player`")
        if name == "run":
            sub.add_argument("--display", action="store_true", help="keep DISPLAY and WAYLAND_DISPLAY")
            sub.add_argument("--keep", action="store_true", help="keep the profile of a passed run too")
            sub.add_argument("cmd", nargs=argparse.REMAINDER, help="-- the command ({project}, {profile})")
    args = parser.parse_args(argv)
    mods = player_mods() if args.mods == "player" else Path(args.mods).resolve() if args.mods else None
    try:
        profile, project, name, env = create(
            Path(args.root).resolve() if args.root else None, args.prefix, getattr(args, "display", False),
            godot_dir=Path(args.godot_dir).resolve(), name_prefix=args.name_prefix, mods=mods,
            library=Path(args.library).resolve() if args.library else None,
            scripts=Path(args.scripts).resolve() if args.scripts else None)
    except NotIsolated as error:
        print(f"godot_isolated: {error}", file=sys.stderr)
        return 2
    if args.command == "profile":
        values = {"METRUM_GODOT_PROFILE": profile, "METRUM_GODOT_PROJECT": project,
                  "METRUM_GODOT_PROFILE_NAME": name, **{key: env[key] for key in XDG_HOMES}}
        print("\n".join(f"{key}={shlex.quote(str(value))}" for key, value in values.items()))
        return 0
    command = args.cmd[1:] if args.cmd[:1] == ["--"] else args.cmd
    if not command:
        print("godot_isolated: no command after --", file=sys.stderr)
        shutil.rmtree(profile, ignore_errors=True)
        return 2
    status = subprocess.run(substitute(command, profile, project), env=env).returncode
    if status == 0 and not args.keep:
        shutil.rmtree(profile, ignore_errors=True)
    else:
        print(f"godot_isolated: profile kept: {profile}", file=sys.stderr)
    return status


if __name__ == "__main__":
    sys.exit(main())
