# SPDX-License-Identifier: GPL-2.0-only

## Correlation, bounded storage, payload accounting and cancellation regression for ROAD-30.
extends SceneTree

const Trace := preload("res://scripts/benchmarks/road_preview_metrics.gd")

func _initialize() -> void:
	var trace := Trace.new()
	var points := PackedVector3Array([Vector3.ZERO, Vector3.ONE])
	var now := Time.get_ticks_usec()
	trace.sample(points, now)
	trace.sample(points, now + 1)
	assert(trace.input_id == 1 and trace.input_us == now)
	trace.dispatch(10, now + 2, now + 3, true)
	trace.poll(10, 50, {"vertices": points, "indices": PackedInt32Array([0, 1]), "nested": [{"bytes": PackedByteArray([1, 2, 3])}]})
	assert(trace.request(10).packed_bytes == 35)
	trace.installed(10)
	trace.sample(PackedVector3Array([Vector3.ZERO, Vector3(2, 0, 0)]), Time.get_ticks_usec())
	trace.frame(10, true)
	assert(trace.rows.size() == 1 and not trace.last_rendered.exact_at_frame)
	assert(trace.last_rendered.request_id == 10 and trace.last_rendered.input_id == 1)
	assert(trace.last_rendered.input_to_frame_ms >= trace.last_rendered.input_to_install_ms)
	trace.frame(10, true)
	assert(trace.rows.size() == 1, "idle frames must not duplicate a displayed request")
	assert(trace.display_age_samples.size() == 2)
	assert(trace.display_age_samples[1].age_ms >= trace.display_age_samples[0].age_ms)
	trace.dispatch(11, now, now, false)
	trace.installed(11)
	trace.frame(0, true)
	assert(trace.rows.size() == 1, "cancelled installations must not count as rendered")
	for id in range(12, 30):
		trace.dispatch(id, now, now, false)
	assert(trace._requests.size() <= 8)
	trace.rows.resize(Trace.CAPACITY)
	trace.installed(29)
	trace.frame(29, false)
	assert(trace.dropped == 1 and trace.rows.size() == Trace.CAPACITY)
	assert(trace.last_rendered.frame_boundary == "headless_process_frame")
	trace.frame_ms.resize(Trace.CAPACITY)
	trace.frame(29, false)
	assert(trace.dropped_frames > 0, "frame saturation must not silently truncate acceptance")
	print("road_preview_metrics_test: PASS")
	quit()
