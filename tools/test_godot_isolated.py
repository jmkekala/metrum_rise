"""The isolated Godot launcher (`TOOLS-19`): its environment is absolute and outside the home
profile, a relative or home XDG path is refused, and its profile holds a uniquely named project."""

import contextlib
import io
import json
import os
from pathlib import Path
import shlex
import sys
import tempfile
import unittest
from unittest import mock

import godot_isolated as gi


def fake_godot(temp):
    godot = temp / "repo/godot"
    (godot / "bin").mkdir(parents=True)
    (godot / "bin/metrum_rise.gdextension").write_text("ext")
    (godot / "bin/libmetrum_rise.so").write_text("deployed")
    (godot / "project.godot").write_text("project")
    (godot / "override.cfg").write_text("the player's override")
    (godot / ".godot").mkdir()
    (godot / "scripts").mkdir()
    (temp / "repo/maps").mkdir()
    return godot


class EnvironmentTests(unittest.TestCase):
    def test_the_environment_is_absolute_and_outside_the_home_profile(self):
        with tempfile.TemporaryDirectory() as temp:
            profile = Path(temp) / "profile"
            env = gi.isolated_env(profile, base={"HOME": "/home/player", "DISPLAY": ":0", "XDG_DATA_HOME": "rel"})
            for name, sub in gi.XDG_HOMES.items():
                path = Path(env[name])
                self.assertTrue(path.is_absolute())
                self.assertEqual(path, profile / sub)
                for real in gi.home_godot_dirs().values():
                    self.assertFalse(path.resolve().is_relative_to(real.resolve()))
                    self.assertNotEqual((path / "godot").resolve(), real.resolve())
            self.assertNotIn("DISPLAY", env)
            self.assertIn("DISPLAY", gi.isolated_env(profile, base={"DISPLAY": ":0"}, display=True))

    def test_a_relative_unset_or_home_xdg_path_is_refused(self):
        with tempfile.TemporaryDirectory() as home:
            good = {name: f"/srv/profile/{sub}" for name, sub in gi.XDG_HOMES.items()}
            gi.check_isolated(good, home)
            for name in gi.XDG_HOMES:
                for value in ("profile/data", "", str(Path(home) / gi.HOME_DEFAULTS[name]),
                              str(Path(home) / gi.HOME_DEFAULTS[name] / "godot/inner")):
                    env = dict(good, **{name: value})
                    with self.assertRaises(gi.NotIsolated, msg=f"{name}={value!r}"):
                        gi.check_isolated(env, home)
            with self.assertRaises(gi.NotIsolated):
                gi.isolated_env("relative/profile", base={})


class ProfileTests(unittest.TestCase):
    def test_a_profile_holds_a_uniquely_named_project_and_the_packs_copy(self):
        with tempfile.TemporaryDirectory() as temp:
            temp = Path(temp)
            godot = fake_godot(temp)
            mods = temp / "player-mods/pack"
            mods.mkdir(parents=True)
            (mods / "asset.toml").write_text("pack")
            profile, project, name, env = gi.create(temp, "probe.", godot_dir=godot, mods=mods.parent,
                                                    name_prefix="MetrumProbe")
            self.assertEqual(profile.parent, temp.resolve())
            self.assertEqual(name, f"MetrumProbe-{profile.name}")
            self.assertIn(f'config/name="{name}"', (project / "override.cfg").read_text())
            self.assertTrue((project / "bin").is_symlink())
            self.assertTrue((project / ".godot").is_symlink())
            self.assertFalse((project / "project.godot").is_symlink())
            self.assertTrue((profile / "maps").is_symlink())
            self.assertEqual((gi.user_dir(profile, name) / "mods/pack/asset.toml").read_text(), "pack")
            self.assertTrue(Path(env["XDG_DATA_HOME"]).is_relative_to(profile))
            # Another build's library gets a bin/ of its own; the tree's stays untouched.
            library = temp / "other.so"
            library.write_text("other")
            _profile, project, _name, _env = gi.create(temp, "probe.", godot_dir=godot, library=library)
            self.assertEqual((project / "bin/libmetrum_rise.so").read_text(), "other")
            self.assertEqual((godot / "bin/libmetrum_rise.so").read_text(), "deployed")

    def test_profile_prints_shell_assignments(self):
        with tempfile.TemporaryDirectory() as temp:
            temp = Path(temp)
            godot = fake_godot(temp)
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                self.assertEqual(gi.main(["profile", "--root", str(temp), "--godot-dir", str(godot)]), 0)
            values = dict(line.split("=", 1) for line in out.getvalue().splitlines())
            values = {key: shlex.split(value)[0] for key, value in values.items()}
            gi.check_isolated(values)
            self.assertTrue(Path(values["METRUM_GODOT_PROJECT"], "override.cfg").exists())

    def test_run_substitutes_the_project_and_keeps_only_a_failed_profile(self):
        with tempfile.TemporaryDirectory() as temp:
            temp = Path(temp)
            godot = fake_godot(temp)
            record = temp / "env.json"
            probe = ("import json, os, sys; json.dump({'project': sys.argv[1], 'env': {k: os.environ.get(k) "
                     "for k in ('XDG_DATA_HOME', 'XDG_CONFIG_HOME', 'XDG_CACHE_HOME', 'DISPLAY')}}, "
                     f"open({str(record)!r}, 'w')); sys.exit(int(sys.argv[2]))")
            root = temp / "profiles"
            root.mkdir()
            with mock.patch.dict(os.environ, {"DISPLAY": ":0"}):
                status = gi.main(["run", "--root", str(root), "--godot-dir", str(godot), "--",
                                  sys.executable, "-c", probe, "{project}", "0"])
            self.assertEqual(status, 0)
            seen = json.loads(record.read_text())
            self.assertTrue(seen["project"].startswith(str(root.resolve())))
            gi.check_isolated(seen["env"])
            self.assertIsNone(seen["env"]["DISPLAY"])
            self.assertEqual(list(root.iterdir()), [])
            with contextlib.redirect_stderr(io.StringIO()):
                status = gi.main(["run", "--root", str(root), "--godot-dir", str(godot), "--",
                                  sys.executable, "-c", probe, "{project}", "3"])
            self.assertEqual(status, 3)
            self.assertEqual(len(list(root.iterdir())), 1)


if __name__ == "__main__":
    unittest.main()
