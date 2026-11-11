"""Stream benchmark runner: the per-frame join of perf records and the isolated project copy."""

import tempfile
import unittest
from pathlib import Path

import stream_benchmark as sb


class ScriptMeansTests(unittest.TestCase):
    def test_means_are_per_frame_of_the_rows_own_records(self):
        rows = [
            {"frame_ms": 30.0, "scripts": {"vegetation": {"elapsed_ms": 6.0, "details": {"uploads": 2}},
                                           "water_reflection": {"elapsed_ms": 0.5, "details": {}}}},
            {"frame_ms": 10.0, "scripts": {"vegetation": {"elapsed_ms": 2.0, "details": {"uploads": 0}}}},
            {"frame_ms": 12.0, "scripts": {}},
        ]
        means, details = sb.script_means(rows)
        self.assertAlmostEqual(means["vegetation"], 8.0 / 3)
        self.assertAlmostEqual(means["water_reflection"], 0.5 / 3)
        self.assertAlmostEqual(details["vegetation.uploads"], 2.0 / 3)

    def test_percentile_is_nearest_rank(self):
        self.assertEqual(sb.percentile([4.0, 1.0, 3.0, 2.0], 0.5), 3.0)
        self.assertEqual(sb.percentile([1.0], 0.95), 1.0)


class ProjectTests(unittest.TestCase):
    def test_control_scripts_take_this_trees_benchmark_scripts(self):
        with tempfile.TemporaryDirectory() as temp:
            temp = Path(temp)
            godot = temp / "repo/godot"
            for name in sb.BENCH_SCRIPTS + ["renderers/terrain.gd"]:
                (godot / "scripts" / name).parent.mkdir(parents=True, exist_ok=True)
                (godot / "scripts" / name).write_text("branch " + name)
            (godot / "bin").mkdir()
            (godot / "bin/metrum_rise.gdextension").write_text("ext")
            (godot / "project.godot").write_text("project")
            (godot / "assets").mkdir()
            control = temp / "control/scripts"
            for name in sb.BENCH_SCRIPTS + ["renderers/terrain.gd"]:
                (control / name).parent.mkdir(parents=True, exist_ok=True)
                (control / name).write_text("control " + name)
            library = temp / "lib.so"
            library.write_text("so")
            profile = temp / "profile"
            profile.mkdir()
            project = sb.make_project(profile, godot, library, control, None)
            self.assertEqual((project / "scripts/renderers/terrain.gd").read_text(), "control renderers/terrain.gd")
            for name in sb.BENCH_SCRIPTS:
                self.assertEqual((project / "scripts" / name).read_text(), "branch " + name)
            self.assertTrue((project / "assets").is_symlink())
            self.assertEqual((project / "bin/libmetrum_rise.so").read_text(), "so")


if __name__ == "__main__":
    unittest.main()
