"""Focused importer and terrain-profile audit regressions; no Godot required."""

import copy
import json
from pathlib import Path
import tempfile
import unittest
import sqlite3

from road_terrain_replay import (
    DEFAULT_FIXTURE, DEFAULT_WORLD, audit_snapshot,
    profile_metrics, read_attempts, recover_stroke, report_capture, sha256, verify_source_terrain,
)


def edge(index, start, end, a, b):
    return {"edge_idx": index, "start_node": start, "end_node": end,
            "physical_geometry_world_precise": [a, b], "fwd_lanes": 1, "bkw_lanes": 1}


class TerrainReplayTest(unittest.TestCase):
    def test_split_t_recovers_branch_not_rebuilt_trunk(self):
        before = {0: edge(0, 0, 1, [0, 1, 0], [100, 2, 0])}
        staged = [edge(0, 0, 3, [0, 1, 0], [50, 4, 0]),
                  edge(1, 2, 3, [50, 10, 50], [50, 4, 0]),
                  edge(2, 3, 1, [50, 4, 0], [100, 2, 0])]
        stroke, _ = recover_stroke(before, {"edges": staged})
        self.assertEqual(stroke["start_xz"], [50, 50])
        self.assertEqual(stroke["end_xz"], [50, 0])
        self.assertNotIn("start_y", stroke)

    def test_crossing_recovers_both_new_ends(self):
        before = {0: edge(0, 0, 1, [0, 1, 0], [100, 2, 0])}
        staged = [edge(0, 0, 4, [0, 1, 0], [50, 4, 0]),
                  edge(1, 2, 4, [50, 10, 50], [50, 4, 0]),
                  edge(2, 4, 1, [50, 4, 0], [100, 2, 0]),
                  edge(3, 4, 3, [50, 4, 0], [50, 10, -50])]
        stroke, _ = recover_stroke(before, {"edges": staged})
        self.assertEqual(stroke["end_xz"], [50, -50])

    def test_logs_require_complete_supported_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "capture.log"
            for contents in ["", "[DEBUG:road] commit_segment_detail points=9 committed=true\n",
                             "[DEBUG:road] commit_segment_detail points=2 committed=true\nROAD_GEOMETRY_DUMP_BEGIN\n{"]:
                path.write_text(contents)
                with self.assertRaises(ValueError):
                    read_attempts(path)

    def test_severe_bump_has_pitch_and_source_evidence(self):
        metrics = profile_metrics([[0, 0, 0, 0], [1, 10, 0, 0], [2, 0, 0, 0]])
        self.assertEqual(metrics["max_grade"], 10)
        self.assertEqual(metrics["max_abs_source_delta_m"], 10)
        self.assertEqual(metrics["max_source_grade"], 0)
        self.assertGreater(metrics["max_pitch_change_deg"], 160)
        self.assertEqual(profile_metrics([[0, 1, 0, 1], [10, 2, 0, 2]])["max_abs_source_delta_m"], 0)

    def test_invalid_missing_and_vertical_profiles_fail(self):
        for points in [[], [[0, 0, 0, 0], [0, 1, 0, 0]], [[0, 0, 0, 0], [1, float("nan"), 0, 0]]]:
            with self.assertRaises(ValueError):
                profile_metrics(points)
        with self.assertRaises(ValueError):
            audit_snapshot({"edges": []}, {})

    def test_incomplete_or_failed_capture_cannot_pass(self):
        snapshot = {"edges": [{"edge_id": 0, "points": [[0, 0, 0, 0], [10, 1, 0, 1]]}]}
        data = {"benchmark": "road_terrain_replay", "schema_version": 1, "completed": True,
                "quality_limits": {"max_grade": 1}, "expected_case_ids": ["test"],
                "reference": {"wait": {"ok": True}, "snapshot": snapshot},
                "cases": [{"case_id": "test", "ok": True, "expected_steps": 1,
                           "steps": [{"attempt": 1, "ok": True, "snapshot": snapshot}]}]}
        self.assertTrue(report_capture(data)["success"])
        # The immutable broken save is historical evidence, not a target whose
        # stored Y values a future profile fix would have to mutate to turn green.
        baseline_bump = copy.deepcopy(data)
        baseline_bump["reference"]["snapshot"] = copy.deepcopy(snapshot)
        baseline_bump["reference"]["snapshot"]["edges"][0]["points"][1][1] = 100
        self.assertTrue(report_capture(baseline_bump)["success"])
        for mutation in ("missing", "duplicate", "failed", "incomplete", "screenshot"):
            altered = copy.deepcopy(data)
            if mutation == "missing":
                altered["cases"].clear()
            elif mutation == "duplicate":
                altered["cases"] *= 2
            elif mutation == "failed":
                altered["cases"][0]["steps"][0]["ok"] = False
            elif mutation == "incomplete":
                altered["completed"] = False
            else:
                altered["capture_error"] = "disk error"
            self.assertFalse(report_capture(altered)["success"], mutation)

    def test_pinned_fixture_integrity(self):
        manifest = json.loads((DEFAULT_FIXTURE / "placements.json").read_text())
        self.assertEqual(sha256(DEFAULT_FIXTURE / "kuopio-terrain-map.sqlite"), manifest["reference_save_sha256"])
        self.assertEqual(sha256(DEFAULT_WORLD), manifest["source_world_sha256"])
        attempts = {stroke["attempt"] for case in manifest["cases"] for stroke in case["segments"]}
        self.assertEqual(attempts, set(range(1, 38)))
        rejected = [stroke for case in manifest["cases"] for stroke in case["segments"] if not stroke["original_accepted"]]
        self.assertEqual([stroke["attempt"] for stroke in rejected], [36])
        self.assertEqual(manifest["reference_live_edges"], 70)
        with sqlite3.connect((DEFAULT_FIXTURE / "kuopio-terrain-map.sqlite").as_uri() + "?mode=ro", uri=True) as db:
            source = db.execute("SELECT height_blob_f32_le FROM terrain_state").fetchone()[0]
        verify_source_terrain(DEFAULT_WORLD, manifest["world_config"], source)


if __name__ == "__main__":
    unittest.main()
