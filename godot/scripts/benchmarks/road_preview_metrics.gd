# SPDX-License-Identifier: GPL-2.0-only

## Bounded, opt-in road preview timing collector. Never participates in placement decisions.
## Godot ticks and Rust Instant durations are kept separate; frame_post_draw is not scanout.
extends RefCounted

const CAPACITY := 4096
var rows: Array[Dictionary] = []
var dropped := 0
var dropped_frames := 0
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

func sample(points: PackedVector3Array, sampled_us: int) -> void:
	if points != _points:
		_points = points
		input_id += 1
		input_us = sampled_us

func dispatch(id: int, start_us: int, end_us: int, full: bool) -> void:
	# At most one running request, one cached result and one displayed result in production.
	# Retire older entries even when cancellation prevents an installation.
	if _requests.size() >= 8:
		_requests.erase(_requests.keys().front())
	_requests[id] = {
		"request_id": id, "input_id": input_id, "input_us": input_us,
		"dispatch_us": start_us, "full_terrain": full,
		"input_to_dispatch_ms": float(start_us - input_us) / 1000.0,
		"dispatch_ms": float(end_us - start_us) / 1000.0,
		"poll_calls": 0, "poll_ms": 0.0, "terrain_retries": 0,
	}

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
		if not row.native.is_empty():
			worker_results += 1
			worker_service_ms += row.native.worker_ms - row.native.core_wait_ms
			worker_wait_ms += row.native.core_wait_ms

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
