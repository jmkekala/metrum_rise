"""Timing session runner: path rule, plans, control cache, calibration and tolerance verdicts."""

import datetime
import os
import sqlite3
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import timing_session as ts


def day(mean=2.2, p99=3.9, p999=4.5, preempted=5000, **extra):
    return dict(mean_ms=mean, p99_ms=p99, p999_ms=p999, max_unpreempted_ms=6.2,
                max_preempted_less_wait_ms=5.0, preempted_frames=preempted, **extra)


class PathRuleTests(unittest.TestCase):
    def test_docs_tools_and_tests_need_no_timing(self):
        level, _, edit, save = ts.classify(
            ["docs/simulation.md", "tools/timing_session.py", "CLAUDE.md", "rust/tests/a.rs",
             "godot/scripts/x.gd", "run.sh"])
        self.assertEqual((level, edit, save), (None, False, False))

    def test_slot_and_edit_paths_are_r(self):
        for path in ("rust/src/simulation/zoning/system/cell_lots/derivation.rs",
                     "rust/src/simulation/economy/demand.rs",
                     "rust/src/simulation/economy/households/employment.rs",
                     "rust/src/nodes/sim/core/schedule.rs", "rust/src/nodes/sim/zoning.rs"):
            self.assertEqual(ts.classify([path])[0], "R", path)
        level, _, edit, _ = ts.classify(["rust/src/simulation/network/roadbed/compile.rs"])
        self.assertEqual((level, edit), ("R", True))
        self.assertTrue(ts.classify(["rust/src/nodes/sim/core/road_commit.rs"])[2])
        self.assertTrue(ts.classify(["rust/src/simulation/save/schema.rs"])[3])

    def test_per_frame_and_unlisted_paths_fail_closed_to_f(self):
        for path in ("rust/src/nodes/sim/core/frame.rs", "rust/src/simulation/economy/agents/tick.rs",
                     "rust/src/thread_pools.rs", "rust/Cargo.toml", "rust/src/new_system.rs",
                     "rust/src/nodes/sim/core/populated_city/frame_budget.rs", "rust/build.rs"):
            level, reasons, *_ = ts.classify([path])
            self.assertEqual(level, "F", path)
        self.assertIn("fails closed", ts.classify(["rust/src/new_system.rs"])[1][0])
        self.assertIn("per-frame", ts.classify(["rust/src/nodes/sim/core/frame.rs"])[1][0])

    def test_one_f_path_makes_the_diff_f(self):
        level, _, edit, _ = ts.classify(["rust/src/simulation/zoning/a.rs",
                                            "rust/src/nodes/sim/core/thread.rs",
                                            "rust/src/nodes/sim/editing.rs"])
        self.assertEqual((level, edit), ("F", True))


class PlanTests(unittest.TestCase):
    def labels(self, rows):
        return [row["label"] for row in rows]

    def test_bench_spec(self):
        bench = ts.parse_bench("a::b::bench_x;RAYON_NUM_THREADS=1;cpus=one")
        self.assertEqual((bench["test"], bench["name"], bench["env"], bench["cpus"]),
                         ("a::b::bench_x", "bench_x", {"RAYON_NUM_THREADS": "1"}, "one"))
        for bad in ("", "A=1", "name;oops", "name;cpus=two"):
            with self.assertRaises(ValueError):
                ts.parse_bench(bad)

    def test_r_alternates_benchmarks_and_32x(self):
        rows = ts.plan_rows("R", benches=[ts.parse_bench("m::bench_a")])
        self.assertEqual(self.labels(rows), [
            "bench-bench_a-1-ctl", "bench-bench_a-1-br", "bench-bench_a-2-br", "bench-bench_a-2-ctl",
            "bench-bench_a-3-ctl", "bench-bench_a-3-br",
            "p07-1-ctl", "p07-2-br", "p07-3-br", "p07-4-ctl"])
        self.assertTrue(all(row["speed"] == 32 and row["tier"] == 30000
                            for row in rows if row["kind"] == "perf07"))

    def test_f_reuses_the_control_day(self):
        rows = ts.plan_rows("F")
        days = [row for row in rows if row["kind"] == "perf07" and row["speed"] == 1]
        self.assertEqual([(row["side"], bool(row.get("reuse")), bool(row.get("second")))
                          for row in days], [("ctl", True, False), ("br", False, False),
                                             ("br", False, True)])

    def test_l_landing_and_merge(self):
        landing = ts.plan_rows("L")
        kinds = [(row["kind"], row["side"], row.get("tier"), row.get("speed"), row.get("reuse"),
                  bool(row.get("second"))) for row in landing]
        # Timing has no 100k tier; K22 at 30k; K22 and contract head only.
        self.assertFalse(any(row.get("tier") == 100000 for row in landing))
        self.assertEqual({row["tiers"] for row in landing if row["kind"] == "k22"}, {"30000"})
        self.assertIn(("k22", "ctl", None, None, "only", False), kinds)
        self.assertIn(("contract", "ctl", None, None, "only", False), kinds)
        self.assertEqual(sum(1 for kind in kinds if kind[:4] == ("perf07", "br", 30000, 1)), 2)
        merge = ts.plan_rows("L", merge=True)
        ctl_days = [row for row in merge if row["kind"] == "perf07" and row["side"] == "ctl"
                    and row["tier"] == 30000 and row["speed"] == 1]
        self.assertEqual([row.get("reuse") for row in ctl_days], [None, None])
        self.assertFalse(any(row.get("reuse") for row in merge if row["kind"] in ("k22", "contract")))

    def test_k22_option(self):
        rows = ts.plan_rows("R", k22=True)
        self.assertEqual([row["tiers"] for row in rows if row["kind"] == "k22"], ["30000"] * 2)

    def test_prepare_needs_skip_cache_only_rows(self):
        rows = ts.plan_rows("L")
        self.assertEqual(ts.prepare_needs(rows, "ctl"),
                         {"baseline": [30000], "fiscal": [], "commit": []})
        self.assertEqual(ts.prepare_needs(rows, "br"),
                         {"baseline": [10000, 30000], "fiscal": [10000, 30000], "commit": [30000]})


    def test_prepare_chains_keep_fiscal_after_its_baseline(self):
        rows = ts.plan_rows("L")
        session = {"binaries": {"ctl": "c", "br": "b"}, "caches": {"ctl": "/c", "br": "/b"},
                   "workers": 8, "needs": {"br": ts.prepare_needs(rows, "br")}}
        chains = ts.prepare_chains(session, "br", Path("."))
        # Per tier the baseline then its fiscal city; the commit city in its own run.
        self.assertEqual([len(chain) for chain in chains], [2, 2, 1])


class CacheTests(unittest.TestCase):
    machine = {"boot_id": "b1", "kernel": "k", "power": {}, "tools": "t"}

    def key(self, **change):
        args = dict(machine=self.machine, binary_sha="s1", snapshot_key="k1",
                    benchmark="perf07", tier=30000, speed=1, workers=8, cpus=[2, 0],
                    env={"METRUM_DEBUG": "0", "PATH": "/bin", "METRUM_PERF07_MINUTES": "480"})
        args.update(change)
        return ts.cache_key(**args)

    def test_key_covers_identity_and_ignores_unrelated_env(self):
        base = self.key()
        self.assertEqual(base["cpus"], [0, 2])
        self.assertNotIn("PATH", base["env"])
        self.assertIn("METRUM_PERF07_MINUTES", base["env"])
        for change in (dict(binary_sha="s2"), dict(snapshot_key="k2"), dict(speed=32),
                       dict(workers=4), dict(cpus=[0]), dict(machine={**self.machine, "boot_id": "b2"}),
                       dict(env={"METRUM_DEBUG": "0"})):
            self.assertNotEqual(ts.key_hash(self.key(**change)), ts.key_hash(base), change)
        self.assertEqual(ts.key_hash(self.key(env={"METRUM_DEBUG": "0", "PATH": "/x",
                                                   "METRUM_PERF07_MINUTES": "480"})),
                         ts.key_hash(base))

    def test_round_trip_stale_and_expiry(self):
        with tempfile.TemporaryDirectory() as root:
            cache = ts.ControlCache(root, "b1")
            cache.put(self.key(), [{"day": day()}], "session-a", ["17:00:00"])
            self.assertEqual(cache.get(self.key())["session"], "session-a")
            self.assertIsNone(cache.get(self.key(binary_sha="s2")))
            other_boot = ts.ControlCache(root, "b2")
            self.assertIsNone(other_boot.get(self.key()))
            self.assertEqual(other_boot.expire_other_boots(), ["b1"])
            self.assertFalse((Path(root) / "b1").exists())

    def test_windows(self):
        at = datetime.datetime(2026, 11, 9, 17, 50)
        self.assertIsNone(ts.in_windows(at, ["18:00-18:15"], 5 * 60))
        self.assertEqual(ts.in_windows(at, ["18:00-18:15"], 11 * 60), "18:00-18:15")
        self.assertEqual(ts.in_windows(at.replace(hour=18, minute=10), ["18:00-18:15"]), "18:00-18:15")
        self.assertIsNone(ts.in_windows(at.replace(hour=18, minute=16), ["18:00-18:15"], 600))


class ToleranceTests(unittest.TestCase):
    def test_review_table(self):
        for n, mean, p99, p999, maxu in ((1, 2.0, 4.8, 11.3, 8.5), (2, 1.4, 3.4, 8.0, 6.0),
                                         (3, 1.14, 2.78, 6.5, 4.9)):
            self.assertAlmostEqual(ts.tolerance_pct("mean_ms", n, n), mean, delta=0.06)
            self.assertAlmostEqual(ts.tolerance_pct("p99_ms", n, n), p99, delta=0.06)
            self.assertAlmostEqual(ts.tolerance_pct("p999_ms", n, n), p999, delta=0.1)
            self.assertAlmostEqual(ts.tolerance_pct("max_unpreempted_ms", n, n), maxu, delta=0.06)
        self.assertAlmostEqual(ts.tolerance_pct("mean_ms", 2, 2, noisy=True), 2.8, delta=0.01)

    def test_verdicts(self):
        self.assertEqual(ts.verdict("mean_ms", [2.20, 2.21], [2.22, 2.23])[2], "level")
        self.assertEqual(ts.verdict("mean_ms", [2.20, 2.21], [2.30, 2.31])[2], "worse")
        self.assertEqual(ts.verdict("mean_ms", [2.20, 2.21], [2.10, 2.11])[2], "better")
        self.assertEqual(ts.verdict("mean_ms", [2.20, 2.21], [2.25, 2.26], noisy=True)[2], "level")
        # A component this small is level within its absolute floor.
        self.assertEqual(ts.verdict("mean_pathing_ms", [0.0004], [0.0008])[2], "level")

    def test_noisy_rows(self):
        rows = [{"label": "a", "day": day(preempted=100)}, {"label": "b", "day": day(preempted=150)},
                {"label": "c", "day": day(preempted=450)}]
        self.assertEqual(ts.noisy_rows(rows), ["c"])
        self.assertEqual(ts.noisy_rows(rows[:2]), [])

    def test_calibration(self):
        cached = [day(mean=4.40, p99=7.0, p999=8.1, preempted=300),
                  day(mean=4.32, p99=6.9, p999=8.0, preempted=100)]
        passed, deltas, _ = ts.calibrate(day(mean=4.40, p99=7.1, p999=8.5, preempted=250), cached)
        self.assertTrue(passed)
        self.assertAlmostEqual(deltas["mean_ms"], 100 * (4.40 - 4.36) / 4.36)
        for drifted in (day(mean=4.55, p99=7.0, p999=8.0, preempted=200),
                        day(mean=4.36, p99=7.6, p999=8.0, preempted=200),
                        day(mean=4.36, p99=6.95, p999=8.0, preempted=900)):
            self.assertFalse(ts.calibrate(drifted, cached)[0])

    def test_second_run(self):
        self.assertIsNone(ts.wants_second_run(30000, 1, day(), [day()]))
        self.assertIn("limit", ts.wants_second_run(30000, 1, day(p999=7.7), [day()]))
        self.assertIsNone(ts.wants_second_run(30000, 1, day(p999=7.5), []))
        with patch.dict(ts.KNOWN_EXCEPTIONS, clear=True):
            self.assertIn("limit", ts.wants_second_run(30000, 32, day(p999=8.1), []))
        self.assertIn("limit", ts.wants_second_run(30000, 1, day(p999=8.3), [day()]))
        # A clear miss needs no second day, and a known exception (PERF-17) none near its limit.
        self.assertIsNone(ts.wants_second_run(30000, 1, day(p999=10.4) | {"max_unpreempted_ms": 12.1}, []))
        self.assertIsNone(ts.wants_second_run(30000, 32, day(p999=8.1), []))

    def test_known_exceptions(self):
        over = day(p999=8.4) | {"max_unpreempted_ms": 8.6, "residents": 30000, "speed": 32.0}
        self.assertEqual(ts.acceptance_misses(over), ([], "PERF-17"))
        self.assertEqual(ts.acceptance_misses(over | {"speed": 1.0})[0],
                         ["max_unpreempted_ms", "p999_ms"])
        self.assertEqual(ts.acceptance_misses(day() | {"residents": 30000, "speed": 1.0}), ([], None))
        self.assertIn("mean_ms", ts.wants_second_run(30000, 1, day(mean=2.3), [day(mean=2.2)]))


class BenchTests(unittest.TestCase):
    def test_values(self):
        text = ("test x ... remote_roads=0 derive_retire_us=21.071\nremote_roads=1024 derive_retire_us=21.3\n"
                'CELL_PREPARATION_BENCH {"residents":10000,"samples_ms":[3.0,1.0,2.0]}\nok\n')
        self.assertEqual(ts.bench_values(text), {"derive_retire_us": [21.071, 21.3],
                                                 "CELL_PREPARATION_BENCH.median_ms": [2.0]})


class SnapshotTests(unittest.TestCase):
    def test_names(self):
        self.assertEqual(ts.snapshot_name("populated_city", 30000, None), "populated_city_30000.sqlite")
        self.assertEqual(ts.snapshot_name("populated_city", 30000, "0123456789abcdef0123"),
                         "populated_city_30000_0123456789abcdef.sqlite")

    def test_meta_and_sweep(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "populated_city_30000.sqlite"
            connection = sqlite3.connect(path)
            connection.executescript(
                "CREATE TABLE save_meta(version INTEGER); INSERT INTO save_meta VALUES (120);"
                "CREATE TABLE perf_fixture_meta(fixture_revision TEXT, residents INTEGER,"
                " day_index INTEGER, minute_of_day INTEGER, departures INTEGER);"
                "INSERT INTO perf_fixture_meta VALUES ('abc', 30000, 12, 0, 7);")
            connection.close()
            meta = ts.snapshot_meta(path)
            self.assertEqual((meta["fixture_revision"], meta["residents"], meta["save_version"]),
                             ("abc", 30000, 120))
            self.assertEqual(len(meta["key"]), 16)
            dead = Path(directory) / ".populated_city_30000.999999999.tmp.sqlite"
            live = Path(directory) / f".populated_city_30000.{os.getpid()}.tmp.sqlite"
            dead.write_text("")
            live.write_text("")
            self.assertEqual(ts.sweep_temporary(directory), [dead.name])
            self.assertTrue(live.exists())


if __name__ == "__main__":
    unittest.main()
