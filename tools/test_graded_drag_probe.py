"""The graded drag probe's runner fails loudly (`TERRAIN-23`): any failure of Godot's status, the
log or the report makes its verdict non-empty."""

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


if __name__ == "__main__":
    unittest.main()
