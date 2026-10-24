# SPDX-License-Identifier: GPL-2.0-only

## Bounded, opt-in road preview timing collector. Never participates in placement decisions.
## Godot ticks and Rust Instant durations are kept separate; frame_post_draw is not scanout.
extends RefCounted

const CAPACITY := 4096
const STRICT_DEADLINE_US := 16_700
var rows: Array[Dictionary] = []
var dropped := 0
var dropped_frames := 0
var dropped_lifecycle := 0
var input_id := 0
var input_us := 0
var _points := PackedVector3Array()
var _requests: Dictionary = {}
var pending_frame: Dictionary = {}
var last_rendered: Dictionary = {}
var frame_ms: Array[float] = []
var display_age_samples: Array[Dictionary] = []
var _last_frame_us := 0
var worker_service_ms := 0.0
var worker_wait_ms := 0.0
var worker_results := 0
var viewport := RID()
var viewport_cpu_ms: Array[float] = []
var viewport_gpu_ms: Array[float] = []
var _scheduled_inputs: Array[Dictionary] = []
var _scheduled_input_count := 0
var _next_unsampled_schedule_index := 0
var _latest_sampled_schedule_index := -1
var _input_lifecycle: Dictionary = {}
var _scheduled_inputs_finished := false

func _init() -> void:
	# The collector is opt-in. Reserve the bounded outer array once; individual diagnostic records
	# still allocate dictionaries, so acceptance runs must quantify the revised harness overhead.
	_scheduled_inputs.resize(CAPACITY)

func schedule(scheduled_us: int, emitted_us: int) -> void:
	if _scheduled_input_count >= CAPACITY:
		dropped_lifecycle += 1
		return
	_scheduled_inputs[_scheduled_input_count] = {
		"schedule_id": _scheduled_input_count + 1,
		"scheduled_us": scheduled_us,
		"emitted_us": emitted_us,
		"schedule_lateness_ms": float(emitted_us - scheduled_us) / 1000.0,
		"disposition": "scheduled",
	}
	_scheduled_input_count += 1

func sample(points: PackedVector3Array, sampled_us: int) -> void:
	if points != _points:
		_supersede_latest_sample(sampled_us)
		_points = points
		input_id += 1
		input_us = sampled_us
		if _next_unsampled_schedule_index < _scheduled_input_count:
			for index in range(_next_unsampled_schedule_index, _scheduled_input_count - 1):
				_scheduled_inputs[index]["disposition"] = "coalesced_before_sampling"
			var sampled_index := _scheduled_input_count - 1
			var lifecycle: Dictionary = _scheduled_inputs[sampled_index]
			lifecycle["disposition"] = "sampled_pending"
			lifecycle["sampled_input_id"] = input_id
			lifecycle["sampled_us"] = sampled_us
			lifecycle["request_count"] = 0
			lifecycle["forgotten_request_count"] = 0
			lifecycle["stale_display_count"] = 0
			_input_lifecycle[input_id] = sampled_index
			_latest_sampled_schedule_index = sampled_index
			_next_unsampled_schedule_index = _scheduled_input_count

func dispatch(id: int, start_us: int, end_us: int, full: bool) -> void:
	# Production keeps at most three outstanding requests plus a cached and a displayed result,
	# and forgets displaced ones. Retire older entries even when cancellation prevents installation.
	if _requests.size() >= 8:
		forget(int(_requests.keys().front()))
	_requests[id] = {
		"request_id": id, "input_id": input_id, "input_us": input_us,
		"dispatch_us": start_us, "full_terrain": full,
		"input_to_dispatch_ms": float(start_us - input_us) / 1000.0,
		"dispatch_ms": float(end_us - start_us) / 1000.0,
		"poll_calls": 0, "poll_ms": 0.0, "terrain_retries": 0,
	}
	var lifecycle := _lifecycle_for_input(input_id)
	if not lifecycle.is_empty():
		lifecycle.request_count += 1
		lifecycle["latest_request_id"] = id

func poll(id: int, elapsed_us: int, payload: Variant) -> void:
	if not _requests.has(id):
		return
	var row: Dictionary = _requests[id]
	row.poll_calls += 1
	row.poll_ms += float(elapsed_us) / 1000.0
	if payload is Dictionary:
		row["received_us"] = Time.get_ticks_usec()
		row["native"] = payload.get("preview_timing", {})
		row["packed_bytes"] = packed_bytes(payload)
		# The synchronous preview's native stages (`docs/roads.md` "Stage budgets").
		if payload.has("plan_ms"):
			var stages := {}
			for stage in ["plan_ms", "roadbed_ms", "payload_ms", "terrain_ms", "packing_ms"]:
				stages[stage] = float(payload.get(stage, 0.0))
			row["stages"] = stages
		if not row.native.is_empty():
			worker_results += 1
			worker_service_ms += row.native.worker_ms - row.native.core_wait_ms
			worker_wait_ms += row.native.core_wait_ms

func forget(id: int) -> void:
	# A displaced or retired request will never be installed.
	if _requests.has(id):
		var lifecycle := _lifecycle_for_input(int(_requests[id].input_id))
		if not lifecycle.is_empty():
			lifecycle.forgotten_request_count += 1
	_requests.erase(id)

func request(id: int) -> Dictionary:
	return _requests.get(id, {})

func installed(id: int) -> void:
	if not _requests.has(id):
		return
	pending_frame = _requests[id]
	pending_frame["installed_us"] = Time.get_ticks_usec()
	pending_frame["input_to_install_ms"] = float(pending_frame.installed_us - pending_frame.input_us) / 1000.0

func frame(displayed_id: int, rendered: bool) -> void:
	var now := Time.get_ticks_usec()
	if _last_frame_us > 0:
		if frame_ms.size() < CAPACITY:
			frame_ms.append(float(now - _last_frame_us) / 1000.0)
		else:
			dropped_frames += 1
	_last_frame_us = now
	if rendered and viewport.is_valid() and viewport_gpu_ms.size() < CAPACITY:
		viewport_cpu_ms.append(RenderingServer.viewport_get_measured_render_time_cpu(viewport))
		viewport_gpu_ms.append(RenderingServer.viewport_get_measured_render_time_gpu(viewport))
	if pending_frame.is_empty():
		_observe_display_age(displayed_id, now)
		return
	if pending_frame.request_id != displayed_id:
		pending_frame = {}
		return
	var row := pending_frame.duplicate(true)
	row["frame_us"] = now
	row["frame_boundary"] = "frame_post_draw" if rendered else "headless_process_frame"
	row["input_to_frame_ms"] = float(now - row.input_us) / 1000.0
	row["install_to_frame_ms"] = float(now - row.installed_us) / 1000.0
	row["latest_input_id_at_frame"] = input_id
	row["exact_at_frame"] = row.input_id == input_id
	_observe_input_lifecycle(row, now)
	last_rendered = row
	if rows.size() < CAPACITY:
		rows.append(row)
	else:
		dropped += 1
	pending_frame = {}
	_observe_display_age(displayed_id, now)

func _observe_display_age(displayed_id: int, now: int) -> void:
	if not last_rendered.is_empty() and last_rendered.request_id == displayed_id:
		if display_age_samples.size() >= CAPACITY:
			dropped_frames += 1
			return
		display_age_samples.append({"frame_us": now, "request_id": displayed_id,
			"age_ms": float(now - last_rendered.input_us) / 1000.0})

func finish_scheduled_inputs(finished_us: int) -> void:
	for index in range(_next_unsampled_schedule_index, _scheduled_input_count):
		if _scheduled_inputs[index].disposition == "scheduled":
			_scheduled_inputs[index]["disposition"] = "coalesced_before_sampling"
	_next_unsampled_schedule_index = _scheduled_input_count
	_supersede_latest_sample(finished_us)
	_scheduled_inputs_finished = true

func scheduled_input_lifecycle() -> Array:
	return _scheduled_inputs.slice(0, _scheduled_input_count).duplicate(true)

func _lifecycle_for_input(sampled_input_id: int) -> Dictionary:
	var index: int = int(_input_lifecycle.get(sampled_input_id, -1))
	return _scheduled_inputs[index] if index >= 0 else {}

func _supersede_latest_sample(resolved_us: int) -> void:
	if _latest_sampled_schedule_index < 0:
		return
	var lifecycle: Dictionary = _scheduled_inputs[_latest_sampled_schedule_index]
	if lifecycle.disposition != "sampled_pending":
		return
	lifecycle["disposition"] = "sampled_but_superseded"
	lifecycle["superseded_us"] = resolved_us
	lifecycle["sample_to_resolution_ms"] = float(resolved_us - int(lifecycle.sampled_us)) / 1000.0
	lifecycle["strict_16_7_met"] = false

func _observe_input_lifecycle(row: Dictionary, frame_us: int) -> void:
	if _scheduled_inputs_finished:
		return
	var lifecycle := _lifecycle_for_input(int(row.input_id))
	if lifecycle.is_empty():
		return
	if lifecycle.disposition in ["exact_on_time", "late_exact"]:
		lifecycle["additional_display_count"] = int(lifecycle.get("additional_display_count", 0)) + 1
		return
	var latency_ms := float(frame_us - int(lifecycle.sampled_us)) / 1000.0
	if bool(row.exact_at_frame):
		lifecycle["frame_us"] = frame_us
		lifecycle["exact_request_id"] = int(row.request_id)
		lifecycle["sample_to_frame_ms"] = latency_ms
		lifecycle["sample_to_resolution_ms"] = latency_ms
		lifecycle["strict_16_7_met"] = meets_strict_deadline_us(frame_us - int(lifecycle.sampled_us))
		lifecycle["disposition"] = "exact_on_time" if lifecycle.strict_16_7_met else "late_exact"
		return
	lifecycle.stale_display_count += 1
	if lifecycle.stale_display_count == 1:
		lifecycle["first_stale_frame_us"] = frame_us
		lifecycle["first_stale_request_id"] = int(row.request_id)
		lifecycle["sample_to_stale_frame_ms"] = latency_ms
		lifecycle["strict_16_7_met"] = false
		lifecycle["disposition"] = "stale_displayed"

static func meets_strict_deadline_us(elapsed_us: int) -> bool:
	return elapsed_us < STRICT_DEADLINE_US

static func packed_bytes(value: Variant) -> int:
	# Count packed payload lengths, never walk vertices or copy packed buffers.
	match typeof(value):
		TYPE_PACKED_BYTE_ARRAY: return value.size()
		TYPE_PACKED_INT32_ARRAY, TYPE_PACKED_FLOAT32_ARRAY: return value.size() * 4
		TYPE_PACKED_INT64_ARRAY, TYPE_PACKED_FLOAT64_ARRAY: return value.size() * 8
		TYPE_PACKED_VECTOR2_ARRAY: return value.size() * 8
		TYPE_PACKED_VECTOR3_ARRAY: return value.size() * 12
		TYPE_PACKED_COLOR_ARRAY, TYPE_PACKED_VECTOR4_ARRAY: return value.size() * 16
		TYPE_DICTIONARY:
			var size := 0
			for key in value:
				size += packed_bytes(value[key])
			return size
		TYPE_ARRAY:
			var size := 0
			for item in value:
				size += packed_bytes(item)
			return size
	return 0
