"""Regression checks for road capture validation and matched process comparisons."""

import copy
import unittest

from road_benchmark_report import compare_pairs, validate_capture


def capture(scale=1.0):
    runtime = dict.fromkeys((
        "godot", "cpu", "logical_cpus", "video_adapter", "max_fps", "vsync",
        "rayon_num_threads_env", "world_sha256", "world_contract", "fixture_isolation",
        "simulation_speed", "input_contract", "harness_sha256", "viewport_size", "configured_renderer",
        "metrics_helper_sha256", "diagnostic_environment",
    ), "fixed")
    runtime["profiled"] = False
    state = dict.fromkeys(("live_edges", "edge_slots", "nodes", "lanes", "agents", "buildings", "rayon_threads"), 0)
    segment = {
        "ok": True, "generation_before": 1, "generation_expected": 2, "generation_after": 2,
        "start": [0, 0, 0], "end": [90, 0, 0],
        "state_after": {**state, "command": {"generation": 2, "committed": True, "routing_ms": scale}},
        "preview_ready_ms": 2 * scale, "preview_ms": 3 * scale,
        "commit_dispatch_ms": scale, "generation_ready_ms": 2 * scale,
        "render_ack_ms": 3 * scale, "first_idle_ms": 4 * scale,
        "ghost_generation": 2, "ghost_vertex_count": 100, "ghost_visible": True,
        "ghost_ready_ms": 3 * scale,
        "commit_ms": 5 * scale, "settle_tail_ms": scale,
    }
    return {
        "schema_version": 3, "success": True, "mode": "headless", "runtime": runtime,
        "repetitions": 1, "warmup_repetitions": 0,
        "matrix_cases": [{"case_id": "t", "segments": [{}]}],
        "fixtures": [{
            "case_id": "t", "ok": True, "warmup": False, "repetition": 0,
            "anchor_x": 0, "anchor_z": 0, "state_before": state, "segments": [segment],
        }],
    }


class RoadBenchmarkReportTest(unittest.TestCase):
    def test_valid_capture_and_paired_process_delta(self):
        self.assertIsNotNone(validate_capture(capture()))
        result = compare_pairs([capture(), capture()], [capture(0.8), capture(1.0)], "render_ack_ms")
        row = result["rows"][0]
        self.assertEqual(row["process_pairs"], 2)
        self.assertAlmostEqual(row["paired_delta_percent_median"], -10)
        self.assertEqual(len(result["warnings"]), 2)

    def test_command_metric(self):
        result = compare_pairs([capture()], [capture(0.5)], "command.routing_ms")
        self.assertEqual(result["rows"][0]["paired_delta_percent_median"], -50)
        with self.assertRaises(ValueError):
            compare_pairs([capture()], [capture()], "misspelled_metric_ms")

    def test_only_declared_rejections_are_accepted(self):
        data = capture()
        segment = data["fixtures"][0]["segments"][0]
        segment.update(expected_preview_rejection=True, committed=False, invalid_reason="too_close")
        with self.assertRaises(ValueError):
            validate_capture(data)
        data["matrix_cases"][0]["segments"][0]["expected_preview_invalid_reason"] = "too_close"
        self.assertIsNotNone(validate_capture(data))
        segment["expected_preview_rejection"] = False
        with self.assertRaises(ValueError):
            validate_capture(data)

    def test_missing_duplicate_and_failed_fixtures_are_rejected(self):
        for change in ("missing", "duplicate", "failed"):
            data = capture()
            if change == "missing":
                data["fixtures"].clear()
            elif change == "duplicate":
                data["fixtures"].append(copy.deepcopy(data["fixtures"][0]))
            else:
                data["fixtures"][0]["ok"] = False
            with self.subTest(change=change), self.assertRaises(ValueError):
                validate_capture(data)

    def test_stale_command_and_reversed_milestones_are_rejected(self):
        data = capture()
        data["fixtures"][0]["segments"][0]["state_after"]["command"]["generation"] = 1
        with self.assertRaises(ValueError):
            validate_capture(data)
        data = capture()
        data["fixtures"][0]["segments"][0]["render_ack_ms"] = 100
        with self.assertRaises(ValueError):
            validate_capture(data)

    def test_missing_or_stale_ghosts_are_rejected(self):
        for field, value in [("ghost_generation", 1), ("ghost_vertex_count", 0), ("ghost_visible", False), ("ghost_ready_ms", 100)]:
            data = capture()
            data["fixtures"][0]["segments"][0][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                validate_capture(data)

    def test_profiled_and_mismatched_inputs_are_rejected(self):
        for change in ("profiled", "anchor", "population", "cadence", "output"):
            candidate = capture()
            if change == "profiled":
                candidate["runtime"]["profiled"] = True
            elif change == "anchor":
                candidate["fixtures"][0]["anchor_x"] = 90
            elif change == "population":
                candidate["fixtures"][0]["state_before"]["agents"] = 100
            elif change == "output":
                candidate["fixtures"][0]["segments"][0]["state_after"]["lanes"] = 100
            else:
                candidate["runtime"]["max_fps"] = 60
            with self.subTest(change=change), self.assertRaises(ValueError):
                compare_pairs([capture()], [candidate], "render_ack_ms")

    def test_reset_fixture_output_must_repeat(self):
        data = capture()
        data["runtime"]["fixture_isolation"] = "reset_each_fixture"
        data["repetitions"] = 2
        repeat = copy.deepcopy(data["fixtures"][0])
        repeat["repetition"] = 1
        data["fixtures"].append(repeat)
        self.assertIsNotNone(validate_capture(data))
        repeat["segments"][0]["ghost_vertex_count"] += 2
        with self.assertRaises(ValueError):
            validate_capture(data)


if __name__ == "__main__":
    unittest.main()
