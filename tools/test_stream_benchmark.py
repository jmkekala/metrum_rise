"""Stream benchmark runner: the per-frame join of perf records, the isolated project copy, the
series under one lock, and the verdict tools (`compare`, `digests`, `stills`), which reproduce
`RENDER-32-L`'s recorded verdicts from its reduced metrics (stream_benchmark_render32l.json)."""

import contextlib
import copy
import io
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import stream_benchmark as sb

FIXTURE = json.loads((Path(__file__).resolve().parent / "stream_benchmark_render32l.json").read_text())


def fake_godot(temp):
    """A minimal Godot tree and library in `temp`."""
    godot = temp / "repo/godot"
    for name in sb.BENCH_SCRIPTS + ["renderers/terrain.gd"]:
        (godot / "scripts" / name).parent.mkdir(parents=True, exist_ok=True)
        (godot / "scripts" / name).write_text("branch " + name)
    (godot / "bin").mkdir()
    (godot / "bin/metrum_rise.gdextension").write_text("ext")
    (godot / "project.godot").write_text("project")
    (godot / "assets").mkdir()
    library = temp / "lib.so"
    library.write_text("so")
    return godot, library


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


class SeriesTests(unittest.TestCase):
    def test_a_series_runs_its_quartet_in_one_hold_of_the_locks(self):
        with tempfile.TemporaryDirectory() as temp:
            temp = Path(temp)
            godot, library = fake_godot(temp)
            control = temp / "ctl.so"
            control.write_text("ctl")
            argv = ["series", "--input-label", "30k", "--ctl-library", str(control), "--library", str(library),
                    "--godot-dir", str(godot), "--out", str(temp / "out"), "--save", str(library)]
            wrapper = ["flock", "/locks/.run.lock", "flock", "-x", "/locks/.timing.lock"]
            settings = {"cpus": "0,2", "busy_processes": ["restic"], "quiet_windows": [], "timed_wrapper": wrapper}
            calls = []

            def fake_run(command, **kwargs):
                calls.append((command, kwargs.get("env", {})))
                return mock.Mock(returncode=0, stdout="abc\n")
            with mock.patch.object(sb.timing_session, "load_settings", return_value=settings), \
                    mock.patch.object(sb.subprocess, "run", side_effect=fake_run), \
                    mock.patch.dict(os.environ, {sb.LOCKED_ENV: ""}):
                sb.main(argv)
            # Outside the locks: the whole series runs itself again once, under the wrapper.
            self.assertEqual(len(calls), 1)
            self.assertEqual(calls[0][0][:len(wrapper)], wrapper)
            self.assertEqual(calls[0][0][-len(argv):], argv)
            self.assertEqual(calls[0][1][sb.LOCKED_ENV], "1")

            calls.clear()
            events = []
            with mock.patch.object(sb.timing_session, "load_settings", return_value=settings), \
                    mock.patch.object(sb.subprocess, "run", side_effect=fake_run), \
                    mock.patch.object(sb.timing_session, "busy_processes",
                                      side_effect=lambda names: events.append(("check", tuple(names))) or []), \
                    mock.patch.dict(os.environ, {sb.LOCKED_ENV: "1"}), \
                    contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                sb.main(argv)
            launches = [command for command, _env in calls if "gamescope" in command]
            self.assertEqual([c[c.index("--log-file") + 1].rsplit("/", 1)[1] for c in launches],
                             ["30k-ctl1.godot.log", "30k-br1.godot.log", "30k-br2.godot.log", "30k-ctl2.godot.log"])
            for command in launches:
                self.assertNotIn("flock", command)
            self.assertEqual(len([e for e in events if e[0] == "check"]), 4)
            self.assertIn("godot", events[0][1])
            libraries = [json.loads((temp / "out" / f"30k-{row}.identity.json").read_text())["library"]
                         for row in ("ctl1", "br1", "br2", "ctl2")]
            self.assertEqual(libraries, [str(control), str(library), str(library), str(control)])
            self.assertTrue(json.loads((temp / "out/30k-br1.identity.json").read_text())["locked"])

    def test_shadows_reach_the_branch_rows_only(self):
        with tempfile.TemporaryDirectory() as temp:
            temp = Path(temp)
            godot, library = fake_godot(temp)
            argv = ["series", "--input-label", "30k", "--ctl-library", str(library), "--library", str(library),
                    "--godot-dir", str(godot), "--out", str(temp / "out"), "--shadows", "full"]
            settings = {"cpus": "", "busy_processes": [], "quiet_windows": [], "timed_wrapper": []}
            calls = []

            def fake_run(command, **kwargs):
                calls.append((command, kwargs.get("env", {})))
                return mock.Mock(returncode=0, stdout="abc\n")
            with mock.patch.object(sb.timing_session, "load_settings", return_value=settings), \
                    mock.patch.object(sb.subprocess, "run", side_effect=fake_run), \
                    mock.patch.object(sb.timing_session, "busy_processes", return_value=[]), \
                    contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                sb.main(argv)
            shadows = [env["METRUM_STREAM_SHADOWS"] for command, env in calls if "gamescope" in command]
            self.assertEqual(shadows, ["", "full", "full", ""])

    def test_an_existing_label_refuses_before_any_lock(self):
        with tempfile.TemporaryDirectory() as temp:
            temp = Path(temp)
            (temp / "30k-br2.metrics.json").write_text("{}")
            with mock.patch.object(sb.subprocess, "run") as run, contextlib.redirect_stderr(io.StringIO()):
                status = sb.main(["series", "--input-label", "30k", "--ctl-library", "x", "--out", str(temp)])
            self.assertEqual(status, 2)
            run.assert_not_called()


class CompareTests(unittest.TestCase):
    def test_reproduces_render_32_l_turn_rows(self):
        lines = sb.compare_lines("30k", FIXTURE["rows"])
        self.assertEqual(lines[3:], FIXTURE["rows_expected"])

    def test_verdict_against_the_same_binary_spread(self):
        def runs(*values):
            return [{"p50": p50, "max": peak} for p50, peak in values]
        self.assertEqual(sb.phase_verdict(runs((10, 20), (10.4, 21)), runs((10.2, 20), (10.4, 21)))[0], "level")
        self.assertEqual(sb.phase_verdict(runs((10, 20), (10.2, 20)), runs((11, 20), (11.1, 20)))[0], "worse p50")
        self.assertEqual(sb.phase_verdict(runs((10, 30), (10.1, 30)), runs((9, 25), (9.1, 26)))[0], "better p50,max")
        # Worse on one metric outranks better on the other.
        self.assertEqual(sb.phase_verdict(runs((10, 20), (10, 20)), runs((9, 25), (9, 25)))[0], "worse max")

    def test_gpu_lines_read_the_gpu_delta_and_the_shadow_pass(self):
        def report(gpu, shadow):
            rows = [{"frame_ms": 9.0, "render_gpu_ms": gpu, **({"shadow_pass": shadow} if shadow else {})}] * 3
            return {"samples": [{"label": "still", "rows": rows, "over_33_3": 0}]}
        runs = {"ctl1": report(8.8, [900, 300, 2_000_000]), "br1": report(7.5, [500, 200, 1_000_000]),
                "br2": report(7.6, [500, 200, 1_000_000]), "ctl2": report(8.7, [900, 300, 2_000_000])}
        self.assertEqual(sb.gpu_lines("30k", runs)[3],
                         "| still | 8.75 | 7.55 | -1.20 (0.10) better | 2000 / 900 | 1000 / 500 |")
        runs["ctl1"] = report(8.8, None)
        self.assertTrue(sb.gpu_lines("30k", runs)[3].endswith("| - | 1000 / 500 |"))


class DigestTests(unittest.TestCase):
    def setUp(self):
        self.ctl = copy.deepcopy(FIXTURE["digests"]["ctl"])
        self.br = copy.deepcopy(FIXTURE["digests"]["br"])
        self.levels = FIXTURE["levels_report"]["graded_levels"]

    def counts(self, families):
        return {family: [row[2] for row in rows] for family, rows in families.items()}

    def test_reproduces_render_32_l_digest_verdicts(self):
        verdicts = self.counts(sb.compare_digests(self.ctl, self.br, self.levels))
        self.assertEqual(verdicts["buildings"], ["equal"] * 13)
        self.assertEqual(verdicts["graded"].count("equal"), 10)
        self.assertEqual([i + 1 for i, v in enumerate(verdicts["graded"]) if v == "band"], [4, 5, 11])
        self.assertEqual(verdicts["lamps"], ["equal"] * 4)
        self.assertEqual(verdicts["water"], ["equal"])

    def test_without_level_ends_a_differing_point_is_different(self):
        verdicts = self.counts(sb.compare_digests(self.ctl, self.br))
        self.assertEqual(verdicts["graded"].count("different"), 3)

    def test_the_report_level_ends_decide_the_band(self):
        narrow = dict(self.levels, held_level_end_m=[e + 10 for e in self.levels["level_end_m"]])
        verdicts = self.counts(sb.compare_digests(self.ctl, self.br, narrow))
        # 2,026-2,030 m tiles fall outside a 10 m band at 2 km.
        self.assertIn("different", verdicts["graded"])
        self.ctl["graded_levels"] = self.br["graded_levels"] = self.levels
        verdicts = self.counts(sb.compare_digests(self.ctl, self.br, narrow))
        self.assertEqual(verdicts["graded"].count("band"), 3)

    def differing_tile(self):
        point = self.ctl["graded_digests"][3]
        key = sorted(point["by_tile"])[0]
        return point, self.br["graded_digests"][3], key

    def test_equal_levels_with_other_bytes_are_different(self):
        point, other, key = self.differing_tile()
        fields = other["by_tile"][key].split(",")
        fields[2:5] = point["by_tile"][key].split(",")[2:5]
        other["by_tile"][key] = ",".join(fields)
        verdict, note = sb.graded_point(point, other, self.levels)
        self.assertEqual(verdict, "different")
        self.assertIn("equal level", note)

    def test_a_tile_on_one_side_only_is_different(self):
        point, other, key = self.differing_tile()
        del other["by_tile"][key]
        self.assertEqual(sb.graded_point(point, other, self.levels)[0], "different")

    def test_the_level_camera_gives_the_exact_distance(self):
        point, other, key = self.differing_tile()
        x, z = (int(v) for v in key.split(","))
        for side in (point, other):
            # The camera 2,016 m from the tile's centre along x, at ground height: inside the band.
            side["level_camera"] = [(x + 0.5) * 64 - 2016.0, 0.0, (z + 0.5) * 64]
        for k in list(point["by_tile"]):
            if k != key:
                del point["by_tile"][k], other["by_tile"][k]
        self.assertEqual(sb.graded_point(point, other, self.levels)[0], "band")
        for side in (point, other):
            side["level_camera"][0] = (x + 0.5) * 64 - 2040.0
        self.assertEqual(sb.graded_point(point, other, self.levels)[0], "different")

    def test_rule_level_holds_the_finer_level_inside_its_band(self):
        self.assertEqual(sb.rule_level(2010.0, None, self.levels), (3, 2))
        self.assertEqual(sb.rule_level(2010.0, 2, self.levels), (2, 0))
        self.assertEqual(sb.rule_level(2040.0, 2, self.levels), (3, 2))
        self.assertEqual(sb.rule_level(100.0, 0, self.levels), (1, 0))
        self.assertEqual(sb.rule_level(6000.0, None, self.levels), (3, 4))
        self.assertEqual(sb.rule_level(20000.0, None, self.levels), (3, 8))

    def test_main_exits_non_zero_on_different(self):
        with tempfile.TemporaryDirectory() as temp:
            a, b = Path(temp) / "a.json", Path(temp) / "b.json"
            a.write_text(json.dumps(dict(self.ctl, graded_levels=self.levels)))
            b.write_text(json.dumps(dict(self.br, graded_levels=self.levels)))
            with mock.patch("builtins.print"):
                self.assertEqual(sb.main(["digests", str(a), str(b)]), 0)
                self.br["building_digests"][0]["digest"] = "0" * 64
                b.write_text(json.dumps(dict(self.br, graded_levels=self.levels)))
                self.assertEqual(sb.main(["digests", str(a), str(b)]), 1)


class SplitTests(unittest.TestCase):
    def test_a_chain_splits_into_one_quartet_per_variant(self):
        sequence = ["full", "none", "none", "full", "low", "low", "full"]
        samples = [{"label": "still", "block": i, "shadows": v} for i, v in enumerate(sequence)]
        quartets = sb.split_blocks({"shadows": sequence, "samples": samples})
        self.assertEqual(sorted(quartets), ["low", "none"])
        blocks = {row: report["samples"][0]["block"] for row, report in quartets["low"].items()}
        self.assertEqual(blocks, {"ctl1": 3, "br1": 4, "br2": 5, "ctl2": 6})
        # A variant without its own pair between two full blocks makes no quartet.
        self.assertEqual(sb.split_blocks({"shadows": ["full", "none", "full"], "samples": samples[:3]}), {})


class StillsTests(unittest.TestCase):
    def setUp(self):
        try:
            import numpy  # noqa: F401
            from PIL import Image  # noqa: F401
        except ImportError:
            self.skipTest("numpy and Pillow are needed for the stills comparisons")

    def write(self, directory, name, array):
        from PIL import Image
        directory.mkdir(parents=True, exist_ok=True)
        Image.fromarray(array).save(directory / name)

    def test_byte_identity_and_a_difference(self):
        import numpy
        with tempfile.TemporaryDirectory() as temp:
            temp = Path(temp)
            image = numpy.zeros((32, 48, 3), numpy.uint8)
            self.write(temp / "a", "pose-full.png", image)
            self.write(temp / "b", "pose-full.png", image)
            self.assertEqual(sb.stills_compare(temp / "a", temp / "b"), [("pose-full.png", "identical", "bytes")])
            changed = image.copy()
            changed[3, 5] = (9, 0, 0)
            self.write(temp / "b", "pose-full.png", changed)
            name, verdict, note = sb.stills_compare(temp / "a", temp / "b")[0]
            self.assertEqual(verdict, "different")
            self.assertIn("1 px, max delta 9", note)

    def test_variants_compare_against_their_poses_full(self):
        import numpy
        with tempfile.TemporaryDirectory() as temp:
            temp = Path(temp)
            image = numpy.zeros((40, 64, 3), numpy.uint8)
            self.write(temp / "a", "city-full.png", image)
            self.write(temp / "a", "city-no_pcss.png", image)
            changed = image.copy()
            changed[30:34, 50:60] = (0, 20, 0)
            self.write(temp / "a", "city-low.png", changed)
            rows = sb.stills_variants(temp / "a", temp / "crops", window=(16, 8))
            self.assertEqual([(pose, variant, d["pixels"]) for pose, variant, d in rows],
                             [("city", "low", 40), ("city", "no_pcss", 0)])
            crops = list((temp / "crops").glob("*.png"))
            self.assertEqual(len(crops), 1)
            self.assertTrue(crops[0].name.startswith("city-low-"))

    def test_speckle_is_not_systematic_and_a_stable_change_is(self):
        import numpy
        rng = numpy.random.default_rng(7)
        base = numpy.full((64, 64, 3), 100, numpy.uint8)

        edge = (numpy.arange(64), numpy.arange(64))

        def speckled():
            # Run-to-run speckle along an edge: each run draws each edge pixel one of three values.
            image = base.copy()
            image[edge] = (numpy.array([[99], [100], [101]], numpy.uint8)[rng.integers(0, 3, 64)]).repeat(3, 1)
            return image
        runs = [speckled() for _ in range(4)]
        counts = sb.systematic_counts(*runs)
        self.assertEqual(sb.systematic_verdict(counts["by_side"], counts["null"])[0], "none")
        for image in runs[2:]:
            image[10:20, 10:20] = (200, 0, 0)
        counts = sb.systematic_counts(*runs)
        self.assertGreaterEqual(counts["by_side"], 90)
        self.assertEqual(sb.systematic_verdict(counts["by_side"], counts["null"])[0], "systematic")
        self.assertEqual(counts["bbox"][:2], (10, 10))

    def test_render_32_l_counts_read_no_systematic_difference(self):
        # The recorded by-side counts and other pairings of the 30k city and the pond (RENDER-32-L).
        for by_side, null in [(1702, [1804, 1719]), (595, [504, 571]), (311, [509, 326]),
                              (1644, [1750, 1816]), (32503, [7882, 37624])]:
            self.assertEqual(sb.systematic_verdict(by_side, null)[0], "none")


if __name__ == "__main__":
    unittest.main()


class CensusTests(unittest.TestCase):
    """`census` (`RENDER-44`, tools/draw_census.py): Godot's culling and counting mirrored."""

    def write(self, temp, rows, count, lods=()):
        import base64
        import struct
        census = {
            "label": "t",
            # A camera at the origin looking down -Z, 90 degrees high, 2:1, planes 0.1 and 1000 m.
            "camera": {"transform": [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0], "fov": 90.0, "keep_aspect": 1,
                       "near": 0.1, "far": 1000.0, "size": [200, 100], "lod_threshold": 1.0},
            "godot": {"visible": [1, 1, 3 * count], "shadow": [0, 0, 0], "gpu_ms": 1.0},
            "meshes": {"7": [{"aabb": [-0.5, -0.5, -0.5, 1, 1, 1], "primitive": 3, "vertices": 8,
                              "indices": 36, "lods": list(lods), "material": 0}]},
            "items": [{"layer": "rs:cars", "kind": "multimesh", "mesh": 7, "count": count, "instance": 1,
                       "transform": [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0], "aabb": [-1, -1, -400, 2, 2, 400],
                       "cast": 1, "range": [0, 0],
                       "rows": base64.b64encode(struct.pack(f"<{12 * count}f", *rows)).decode()}],
            "layers": {},
        }
        path = Path(temp) / "t.census.json"
        path.write_text(json.dumps(census))
        return path

    def test_a_drawn_batch_counts_every_instance_and_its_off_screen_ones(self):
        import draw_census
        on = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, -60]
        # Behind the camera: off-screen, but drawn with its batch.
        behind = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 30]
        far = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, -350]
        with tempfile.TemporaryDirectory() as temp:
            _, layers = draw_census.analyse(self.write(temp, on + behind + far, 3))
        stats = layers["rs:cars"]
        self.assertEqual((stats["batches_drawn"], stats["drawn"], stats["offscreen"]), (1, 3, 1))
        self.assertEqual(stats["prims"].sum(), 36)
        # Bands: 50-100 m, 0-50 m (behind), 300-500 m.
        self.assertEqual(list(stats["prims"][:5]), [12, 12, 0, 0, 12])

    def test_a_batch_takes_the_level_of_detail_of_its_nearest_point(self):
        import draw_census
        near = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, -2]
        far = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, -350]
        # One level of 6 indices (2 triangles) for edges up to 0.1 m: a pixel at about 100 m here.
        with tempfile.TemporaryDirectory() as temp:
            _, layers = draw_census.analyse(self.write(temp, near + far, 2, [[0.1, 6]]))
        stats = layers["rs:cars"]
        # The batch's box holds the camera: the full mesh for both; the far one alone would draw 2.
        self.assertEqual(stats["prims"].sum(), 24)
        self.assertEqual(stats["own_prims"].sum(), 14)
