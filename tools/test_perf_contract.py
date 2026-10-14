"""Contract gate and conservative path-selection regressions."""

import copy
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import perf_contract
from perf_contract import (BOUNDED, PHASES, check_fiscal_coverage, check_limits,
                           run_test, selected_phases)


class ContractTests(unittest.TestCase):
    def rows(self):
        rows = []
        for tier, scale in ((10000, 1), (30000, 3)):
            counts = {name: scale * 10 for name in BOUNDED}
            counts.update(cch_point_searches=scale * 4, cch_bucket_sources=scale * 6)
            rows.append(dict(residents=tier, hourly_ms=scale * 20, daily_ms=scale * 5,
                             households=scale * 5000, counts=counts,
                             building_churn=[[1, 0] for _ in range(24)]))
        return rows

    def fiscal_rows(self):
        rows = self.rows()
        for row in rows:
            row.update(workload="fiscal_stress", day=16)
        return rows

    def test_shared_and_unknown_paths_run_every_phase(self):
        for path in ("rust/src/simulation/save/mod.rs", "rust/src/simulation/economy/customers/business.rs",
                     "rust/src/simulation/buildings/allocator.rs", "tools/perf_contract.py",
                     "rust/src/new_shared_system.rs"):
            self.assertEqual(selected_phases([path]), list(PHASES))

    def test_employment_changes_include_route_diagnostic(self):
        self.assertIn("routing", selected_phases([
            "rust/src/simulation/economy/households/employment.rs"
        ]))

    def test_documentation_only_skips_runtime_checks(self):
        self.assertEqual(selected_phases(["docs/economy.md", "CLAUDE.md"]), [])

    def test_scoped_selection_unions_paths(self):
        self.assertEqual(selected_phases(["rust/src/simulation/zoning/cells.rs"]),
                         ["planning", "execution", "lots"])
        self.assertIn("logistics", selected_phases(["rust/src/simulation/zoning/cells.rs",
                                                   "rust/src/simulation/economy/logistics/tick.rs"]))

    def test_time_budget_is_fixed_not_small_city_timing_ratio(self):
        rows = self.rows()
        rows[0]["hourly_ms"] = 1
        self.assertEqual(check_limits(rows), [])
        rows[1]["daily_ms"] = 30.001
        self.assertTrue(check_limits(rows))

    def test_every_bounded_count_and_combined_cch_is_enforced(self):
        original = self.rows()
        for counter in (*BOUNDED, "cch_bucket_sources", "cch_point_searches"):
            rows = copy.deepcopy(original)
            rows[1]["counts"][counter] = 1000
            self.assertTrue(check_limits(rows), counter)

    def test_zero_small_count_does_not_hide_new_work(self):
        rows = self.rows()
        rows[0]["counts"]["reservation_rows_copied"] = 0
        self.assertTrue(check_limits(rows))
        rows[1]["counts"]["reservation_rows_copied"] = 0
        self.assertEqual(check_limits(rows), [])

    def test_missing_tier_is_failure(self):
        self.assertTrue(check_limits(self.rows()[:1]))

    def test_hourly_churn_requires_coverage_not_daily_total(self):
        rows = self.rows()
        rows[0]["building_churn"][8] = [0, 0]
        rows[0]["building_churn"][9] = [50, 0]
        self.assertIn("hour intervals [8]", check_limits(rows, require_churn=True)[0])
        rows[0]["building_churn"][8] = [0, 1]
        self.assertEqual(check_limits(rows, require_churn=True), [])
        del rows[0]["building_churn"]
        self.assertIn("missing 24-hour", check_limits(rows, require_churn=True)[0])

    def test_baseline_churn_is_reported_without_being_a_gate(self):
        rows = self.rows()
        rows[0]["building_churn"] = [[0, 0] for _ in range(24)]
        self.assertEqual(check_limits(rows), [])
        self.assertTrue(check_limits(rows, require_churn=True))

    def test_fiscal_coverage_requires_two_day16_stress_rows(self):
        rows = self.fiscal_rows()
        self.assertEqual(check_fiscal_coverage(rows), [])
        rows[0]["building_churn"][4] = [0, 0]
        self.assertIn("inactive hour intervals [4]", check_fiscal_coverage(rows)[0])
        rows[0]["building_churn"][4] = [0, 1]
        rows[1]["workload"] = "baseline"
        self.assertIn("wrong workload", " ".join(check_fiscal_coverage(rows)))
        rows[1]["workload"] = "fiscal_stress"
        rows[1]["day"] = 15
        self.assertIn("not the day-16", " ".join(check_fiscal_coverage(rows)))

    def test_fiscal_workload_uses_time_and_work_limits(self):
        rows = self.fiscal_rows()
        rows[0]["hourly_ms"] = 40.001
        self.assertIn("10000 hourly_ms", " ".join(check_limits(rows, require_churn=True)))
        rows = self.fiscal_rows()
        rows[1]["counts"]["staffing_trials"] = 1000
        self.assertIn("staffing_trials", " ".join(check_limits(rows, require_churn=True)))

    def test_fiscal_coverage_reports_missing_tier(self):
        self.assertIn("exactly one fiscal workload", check_fiscal_coverage(self.fiscal_rows()[:1])[0])

    def test_baseline_failures_are_not_masked_by_fiscal_rows(self):
        baseline = self.rows()
        baseline[0]["daily_ms"] = 11
        self.assertTrue(check_limits(baseline))
        self.assertEqual(check_limits(self.fiscal_rows()), [])

    def test_runner_requires_expected_report_cardinality(self):
        def fake_run(_command, **kwargs):
            kwargs["stdout"].write("test fake ... ok\n1 passed\n")
            return subprocess.CompletedProcess([], 0)

        with tempfile.TemporaryDirectory() as directory, patch("perf_contract.subprocess.run", fake_run):
            with self.assertRaisesRegex(RuntimeError, "emitted 0 PERF_PHASE reports"):
                run_test(
                    Path("/tmp/fake-tests"), "fake", {}, Path(directory),
                    "phase", "PERF_PHASE ", 1,
                )

    def test_runner_accepts_six_agent_reports(self):
        reports = "".join(
            f"PERF_AGENT {{\"agents\": {count}, \"mode\": \"network\"}}\n"
            for count in (10000, 30000, 10000, 30000, 10000, 30000)
        )

        def fake_run(_command, **kwargs):
            kwargs["stdout"].write(reports + "1 passed\n")
            return subprocess.CompletedProcess([], 0)

        with tempfile.TemporaryDirectory() as directory, patch("perf_contract.subprocess.run", fake_run):
            result = run_test(
                Path("/tmp/fake-tests"), "fake", {}, Path(directory),
                "agents", "PERF_AGENT ", 6,
            )
        self.assertEqual(len(result), 6)

    def run_main_with_mocked_binary(self, arguments, run_test_side_effect):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "contract-tests"
            binary.write_bytes(b"test binary")
            artifacts = root / "artifacts"
            with patch.object(perf_contract, "run_test", side_effect=run_test_side_effect), \
                    patch.object(perf_contract.subprocess, "check_output",
                                 side_effect=["HEAD\\n", b""]), \
                    patch.object(perf_contract.sys, "argv",
                                 ["perf_contract.py", *arguments, "--binary", str(binary),
                                  "--artifacts", str(artifacts)]):
                try:
                    result = perf_contract.main()
                except Exception as error:
                    error.report = json.loads((artifacts / "report.json").read_text())
                    raise
            return result, json.loads((artifacts / "report.json").read_text())

    def test_prepare_and_agents_reports_finish_passed(self):
        def one_record(*_args):
            return [{"phase": "prepare"}]

        result, report = self.run_main_with_mocked_binary(["--prepare"], one_record)
        self.assertEqual(result, 0)
        self.assertEqual(report["status"], "passed")
        self.assertEqual(len(report["records"]), 2)

        def six_records(*_args):
            return [{"agents": count} for count in (10000, 30000, 10000, 30000, 10000, 30000)]

        result, report = self.run_main_with_mocked_binary(["--agents"], six_records)
        self.assertEqual(result, 0)
        self.assertEqual(report["status"], "passed")
        self.assertEqual(len(report["records"]), 6)

    def test_failure_report_retains_completed_records(self):
        def fail_after_first(*args):
            if args[4] == "prepare-10000":
                return [{"residents": 10000}]
            raise RuntimeError("synthetic timeout")

        with self.assertRaisesRegex(RuntimeError, "synthetic timeout") as raised:
            self.run_main_with_mocked_binary([], fail_after_first)
        report = raised.exception.report
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["records"], [{"residents": 10000}])
        self.assertIn("synthetic timeout", report["error"])

    def test_locality_failure_retains_completed_day_limit_failures(self):
        rows = self.rows()
        rows[1]["daily_ms"] = 31.0
        rows[1]["counts"]["staffing_trials"] = 1000
        fiscal_rows = self.fiscal_rows()
        fiscal_rows[1]["counts"]["staffing_trials"] = 1000
        called = []

        def fail_after_days(*args):
            label = args[4]
            called.append(label)
            if label.startswith("prepare-"):
                return [{"phase": "prepare"}]
            if label.startswith("fiscal-prepare-"):
                return [{"phase": "fiscal_prepare"}]
            if label == "day-10000":
                return [rows[0]]
            if label == "day-30000":
                return [rows[1]]
            if label == "fiscal-day-10000":
                return [fiscal_rows[0]]
            if label == "fiscal-day-30000":
                return [fiscal_rows[1]]
            if label == "perf_contract_queued_locality":
                raise RuntimeError("locality failed")
            if label == "agents":
                return [{"agents": count} for count in (10000, 30000, 10000, 30000, 10000, 30000)]
            if label.endswith(("-10000", "-30000")):
                return [{"phase": label}]
            return []

        result, report = self.run_main_with_mocked_binary([], fail_after_days)
        self.assertEqual(result, 1)
        self.assertEqual(report["status"], "failed")
        self.assertEqual(
            report["failures"],
            ["locality failed", *check_limits(rows, workload="baseline"),
             *check_limits(fiscal_rows, workload="fiscal_stress")],
        )
        self.assertEqual([row for row in report["records"] if "hourly_ms" in row], rows)
        self.assertEqual(report["fiscal_records"], fiscal_rows)
        self.assertEqual(called[-2:], ["perf_contract_financing_locality", "agents"])


if __name__ == "__main__":
    unittest.main()
