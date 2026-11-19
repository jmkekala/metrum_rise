"""The graded drag probe's runner fails loudly (`TERRAIN-23`): any failure of Godot's status, the
log or the report makes its verdict non-empty."""

import argparse
import fcntl
from pathlib import Path
import tempfile
import unittest

import graded_drag_probe as probe

PASSED = {"success": True, "strokes": [{"case": "kuopio_08", "attempt": 21}], "failures": []}


class VerdictTest(unittest.TestCase):
    def test_a_passed_run_has_no_problem(self):
        self.assertEqual(probe.verdict(0, "", PASSED), [])

    def test_a_failed_check_fails(self):
        report = {"success": False, "strokes": [], "failures": ["kuopio_08 21: the fast_commit installs in one batch frame: 0"]}
        problems = probe.verdict(1, "", report)
        self.assertIn("Godot exited 1", problems)
        self.assertTrue(any("fast_commit" in problem for problem in problems))

    def test_gamescope_status_zero_does_not_hide_a_failed_report(self):
        report = dict(PASSED, success=False)
        self.assertIn("the report is not a success", probe.verdict(0, "", report))

    def test_script_errors_log_failures_and_missing_reports_fail(self):
        self.assertIn("the run hit a SCRIPT ERROR", probe.verdict(0, "SCRIPT ERROR: x", PASSED))
        overflow = "Too many instances using shader instance variables"
        self.assertTrue(any(overflow in problem for problem in probe.verdict(0, overflow, PASSED)))
        self.assertIn("no report was written", probe.verdict(0, "", None))



def options(**given):
    """The runner's options as parsed, defaults but `given`."""
    out = dict(env=[], timing=False, exclusive=False, prewarm=False, as_played=False, run_lock=False)
    out.update(given)
    return argparse.Namespace(**out)


class TimedRowTest(unittest.TestCase):
    """The timing mode and the prewarmed launch (`TERRAIN-14` round T14Cf)."""

    def test_the_timing_mode_and_the_prewarm_reach_the_probe(self):
        self.assertEqual(probe.probe_env(options()), {})
        env = probe.probe_env(options(timing=True, prewarm=True, env=["METRUM_GRADED_DRAG_SWEEP=30"]))
        self.assertEqual(env, {"METRUM_GRADED_DRAG_TIMING": "1", "METRUM_GRADED_DRAG_PREWARM": "1",
                               "METRUM_GRADED_DRAG_SWEEP": "30"})

    def test_a_timed_row_holds_both_locks_exclusive_and_as_played_is_unpinned(self):
        self.assertEqual(probe.held_locks(options()), [(".timing.lock", False)])
        self.assertEqual(probe.held_locks(options(run_lock=True)), [(".run.lock", True), (".timing.lock", False)])
        both = [(".run.lock", True), (".timing.lock", True)]
        self.assertEqual(probe.held_locks(options(timing=True)), both)
        self.assertEqual(probe.held_locks(options(exclusive=True)), both)
        self.assertEqual(probe.pinning(options())[:2], ["taskset", "-c"])
        self.assertEqual(probe.pinning(options(as_played=True)), [])

    def test_the_first_rows_pipelines_and_the_late_frames_are_reported(self):
        # The first row's compilations are counted from the settled frame before its drag.
        frames = [
            {"kind": "sweep", "frame_ms": 1223.0, "pre_ms": 9.0, "draw_ms": 1210.0, "pipelines": [0, 0, 6, 0, 0]},
            {"kind": "sweep", "frame_ms": 19.5, "pre_ms": 2.1, "draw_ms": 16.8, "pipelines": [0, 0, 3, 0, 2]},
            {"kind": "sweep", "frame_ms": 16.6, "pipelines": [0, 0, 0, 0, 0]},
        ]
        report = {"strokes": [{"case": "kuopio_08", "attempt": 21, "sweep": {"frames": frames},
                               "fast": {"frames": [{"kind": "pose", "frame_ms": 16.7, "pipelines": [0, 0, 0, 0, 0]}]}}]}
        lines = probe.frame_summary(report)
        self.assertIn("sweep frames 3, late 1", lines)
        self.assertIn("  late kuopio_08:21:sweep[1] 19.5 ms: scripts 2.10, draw 16.80", lines)
        self.assertIn("long kuopio_08:21:sweep[0] sweep 1223.0 ms: pipelines {'surface': 6}", lines)
        self.assertIn("pose frames 1, late 0", lines)
        self.assertEqual(lines[-1], "pipelines over the rows {'canvas': 0, 'mesh': 0, 'surface': 9, 'draw': 0, 'specialization': 2}")


    def test_locks_are_taken_together_or_not_at_all(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            wanted = [(".run.lock", True), (".timing.lock", True)]
            other = open(directory / ".timing.lock", "a")
            fcntl.flock(other, fcntl.LOCK_SH)
            self.assertIsNone(probe.try_locks(directory, wanted))
            # The run lock was given back while the timing lock was busy.
            free = open(directory / ".run.lock", "a")
            fcntl.flock(free, fcntl.LOCK_EX | fcntl.LOCK_NB)
            free.close()
            other.close()
            held = probe.try_locks(directory, wanted)
            self.assertEqual(len(held), 2)
            for handle in held:
                handle.close()


if __name__ == "__main__":
    unittest.main()
