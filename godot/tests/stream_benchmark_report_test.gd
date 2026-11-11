# SPDX-License-Identifier: GPL-2.0-only

## The stream benchmark's report schema (`PERF-23`): a sample built by the benchmark's own
## finishing step from synthetic rows passes the schema check; one missing a row key, a statistic
## or a frame fails it; and the frame statistics and slow-frame counts are what the rows say.
extends SceneTree

const Stream := preload("res://scripts/benchmarks/stream_frame_benchmark.gd")

var _failures := 0

func _initialize() -> void:
	var bench := Stream.new()
	var rows := []
	for index in 4:
		var row := {}
		for key: String in Stream.ROW_KEYS:
			row[key] = {} if key in ["streams", "stream_delta", "scripts"] else 0
		row["frame"] = index
		row["frame_ms"] = [10.0, 20.0, 40.0, 30.0][index]
		row["reflection_levels"] = [null, null]
		rows.append(row)
	var sample: Dictionary = bench._finish("turn_out", 1.0, rows)
	_expect(sample.frames == 4 and sample.over_16_7 == 3 and sample.over_22 == 2 and sample.over_33_3 == 1,
		"slow-frame counts follow the rows")
	_expect(sample.frame_ms.p50 == 30.0 and sample.frame_ms.max == 40.0 and sample.frame_ms.mean == 25.0,
		"frame statistics follow the rows")
	var report := {"schema_version": Stream.SCHEMA_VERSION, "benchmark": "stream", "runtime": {},
		"phases": ["turns"], "load": {}, "samples": [sample]}
	_expect(Stream.report_errors(report).is_empty(), "a finished sample is well formed: %s" % str(Stream.report_errors(report)))
	var no_levels: Dictionary = report.duplicate(true)
	(no_levels.samples[0].rows[2] as Dictionary).erase("reflection_levels")
	_expect(not Stream.report_errors(no_levels).is_empty(), "a row without its levels fails")
	var no_p99: Dictionary = report.duplicate(true)
	(no_p99.samples[0].frame_ms as Dictionary).erase("p99")
	_expect(not Stream.report_errors(no_p99).is_empty(), "a sample without p99 fails")
	var short: Dictionary = report.duplicate(true)
	(short.samples[0].rows as Array).pop_back()
	_expect(not Stream.report_errors(short).is_empty(), "frames must equal rows")
	var other: Dictionary = report.duplicate(true)
	other.benchmark = "idle_frame"
	_expect(not Stream.report_errors(other).is_empty(), "another benchmark's report fails")
	_expect(Stream.stats(PackedFloat64Array()).p50 == 0.0, "empty statistics are zeros")
	print("stream_benchmark_report_test: %s" % ("PASS" if _failures == 0 else "FAIL"))
	quit(_failures)

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error("stream_benchmark_report_test: " + message)
