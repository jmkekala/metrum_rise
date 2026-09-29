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
        "commit_ms": 5 * scale, "settle_tail_ms": scale,
    }
    return {
        "schema_version": 4, "success": True, "mode": "headless", "runtime": runtime,
        "repetitions": 1, "warmup_repetitions": 0,
        "matrix_cases": [{"case_id": "t", "segments": [{}]}],
        "fixtures": [{
            "case_id": "t", "ok": True, "warmup": False, "repetition": 0,
            "anchor_x": 0, "anchor_z": 0, "state_before": state, "segments": [segment],
        }],
    }


class RoadBenchmarkReportTest(unittest.TestCase):
    def test_preview_correlation_modes_and_boundaries(self):
        def row(request):
            return dict(request_id=request, input_us=100, dispatch_us=200,
                        received_us=300, installed_us=400, frame_us=500,
                        input_to_frame_ms=0.4, exact_at_frame=True, native={},
                        full_terrain=False, frame_boundary="frame_post_draw")

        state = dict.fromkeys(("generation", "live_edges", "edge_slots", "nodes", "lanes", "agents", "buildings"), 0)
        case = dict(case_id="flat_t", mode=0, ok=True, dropped=0,
                    stationary=[row(1)], moving=[row(2)], final=row(3),
                    state_before=state, state_after=state, scheduled_input_count=120,
                    scheduled_input_hz=60, moving_interval_ms=2000, frame_ms={"max": 17})
        full = copy.deepcopy(case)
        full["mode"] = 1
        for sample in full["stationary"] + full["moving"] + [full["final"]]:
            sample["full_terrain"] = True
        data = dict(benchmark="road_preview_latency", schema_version=1,
                    success=True, samples=1, expected_cases=["flat_t"],
                    mode="windowed", cases=[case, full])
        self.assertIs(validate_capture(data), data)
        for change in ("missing_mode", "wrong_boundary", "duplicate", "stale_final", "bad_order", "cadence", "missing_native"):
            bad = copy.deepcopy(data)
            candidate = bad["cases"][0]
            if change == "missing_mode":
                bad["cases"].pop()
            elif change == "wrong_boundary":
                candidate["final"]["frame_boundary"] = "headless_process_frame"
            elif change == "duplicate":
                candidate["final"]["request_id"] = 1
            elif change == "stale_final":
                candidate["final"]["exact_at_frame"] = False
            elif change == "missing_native":
                bad["runtime"] = {"diagnostic_environment": {"METRUM_DEBUG_PERF": "1"}}
            elif change == "cadence":
                candidate["moving_interval_ms"] = 4000
            else:
                candidate["final"]["installed_us"] = 600
            with self.subTest(change=change), self.assertRaises(ValueError):
                validate_capture(bad)

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

    def test_old_global_guide_schema_is_rejected(self):
        data = capture()
        data["schema_version"] = 3
        with self.assertRaises(ValueError):
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
        repeat["segments"][0]["state_after"]["lanes"] += 2
        with self.assertRaises(ValueError):
            validate_capture(data)


if __name__ == "__main__":
    unittest.main()
