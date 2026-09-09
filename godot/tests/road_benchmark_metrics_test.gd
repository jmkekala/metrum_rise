# SPDX-License-Identifier: GPL-2.0-only

## Regression checks for benchmark aggregation and honest small-sample reporting.
extends SceneTree

const Metrics = preload("res://scripts/benchmarks/road_benchmark_metrics.gd")
const Harness = preload("res://scripts/benchmarks/gameplay_road_benchmark.gd")

func _initialize() -> void:
	var small := Metrics.distribution([1.0, 2.0, 100.0, 101.0])
	assert(small.p50 == 51.0)
	assert(small.p95 == null and small.p99 == null)
	assert(Metrics.distribution([]).p50 == null)
	var many: Array = []
	for index in range(1, 1001):
		many.append(float(index))
	var large := Metrics.distribution(many)
	assert(large.p95 == 950.0 and large.p99 == 990.0)
	var fixtures: Array = [
		{"case_id": "t", "ok": true, "warmup": true, "total_ms": 9999.0,
			"segments": [{"commit_ms": 9999.0}]},
		{"case_id": "t", "ok": true, "warmup": false, "total_ms": 110.0,
			"segments": [{"commit_ms": 10.0}, {"commit_ms": 100.0}]},
		{"case_id": "t", "ok": true, "warmup": false, "total_ms": 220.0,
			"segments": [{"commit_ms": 20.0}, {"commit_ms": 200.0}]},
	]
	var summary := Metrics.summarize(fixtures)
	assert(summary.cases.t.fixture_total_ms.count == 2)
	assert(summary.cases.t.segments.initial_road.commit_ms.p50 == 15.0)
	assert(summary.cases.t.segments.edit_1.commit_ms.p50 == 150.0)
	var harness := Harness.new()
	harness.repetitions = 2
	harness.warmup_repetitions = 1
	var definitions := harness._fixture_definitions()
	assert(definitions.size() == 8 and harness._resets_each_fixture())
	var workload := harness._fixture_workload(definitions)
	assert(workload.size() == 24 and workload[0].warmup)
	assert(workload[8].fixture.case_id == definitions.front().case_id)
	assert(workload[16].fixture.case_id == definitions.back().case_id)
	harness.selected_case_ids = PackedStringArray(["double_t_close_2l", "four_way_mixed_8l_2l"])
	var targeted := harness._fixture_definitions()
	assert(targeted.size() == 2)
	assert(targeted[0] == definitions[4] and targeted[1] == definitions[6])
	harness.selected_case_ids = PackedStringArray(["double_t_close_2l", "unknown"])
	assert(harness._fixture_definitions().is_empty())
	harness.selected_case_ids = PackedStringArray(["double_t_close_2l", "double_t_close_2l"])
	assert(harness._fixture_definitions().is_empty())
	harness.selected_case_ids = PackedStringArray()
	harness.matrix_name = "interaction"
	var interactions := harness._fixture_definitions()
	assert(interactions.size() == 3)
	assert(interactions[2].segments[1].preview_mode == "immediate")
	assert(not interactions[2].segments[0].has("preview_mode"))
	harness.matrix_name = "road08"
	var road08 := harness._fixture_definitions()[0]
	assert(not road08.segments[1].has("expected_preview_invalid_reason"))
	assert(road08.junctions[0].expected_degree == 2)
	harness.free()
	print("road_benchmark_metrics_test: PASS")
	quit(0)
