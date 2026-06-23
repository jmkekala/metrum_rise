## Opt-in renderer frame CPU diagnostics for `--debug perf`.
##
## Renderers record their main-thread work here when `METRUM_DEBUG_PERF=1`.
## The summary is intentionally coarse and stable: one rolling line every half
## second with total frame renderer CPU and per-renderer costs.
extends RefCounted

class_name PerfDebug

const LOG_INTERVAL_S := 0.5
const FRAME_SPIKE_THRESHOLD_MS := 16.0

static var _enabled_cache_valid: bool = false
static var _enabled_cached: bool = false
static var _current_frame: int = -1
static var _current_frame_total_ms: float = 0.0
static var _last_log_us: int = 0
static var _frame_count: int = 0
static var _frame_total_ms: float = 0.0
static var _frame_max_ms: float = 0.0
static var _frame_spikes: int = 0
static var _renderer_stats: Dictionary = {}
static var _detail_stats: Dictionary = {}

static func is_enabled() -> bool:
	if not _enabled_cache_valid:
		var perf_value := OS.get_environment("METRUM_DEBUG_PERF").strip_edges().to_lower()
		var debug_value := OS.get_environment("METRUM_DEBUG").strip_edges()
		var debug_filter := OS.get_environment("METRUM_DEBUG_FILTER").strip_edges().to_lower()
		_enabled_cached = (
			perf_value == "1"
			or perf_value == "true"
			or perf_value == "yes"
			or perf_value == "on"
			or (debug_value == "1" and debug_filter == "perf")
		)
		_enabled_cache_valid = true
	return _enabled_cached

static func record(renderer_name: String, elapsed_ms: float, details: Dictionary = {}) -> void:
	if not is_enabled():
		return
	var now_us := Time.get_ticks_usec()
	if _last_log_us == 0:
		_last_log_us = now_us
	_advance_frame(Engine.get_process_frames())
	if float(now_us - _last_log_us) / 1000000.0 >= LOG_INTERVAL_S:
		_print_summary(now_us)
	var clamped_elapsed_ms := maxf(0.0, elapsed_ms)
	_current_frame_total_ms += clamped_elapsed_ms
	_add_stat(_renderer_stats, renderer_name, clamped_elapsed_ms)
	for key_variant in details.keys():
		var value = details[key_variant]
		if value is int or value is float:
			_add_stat(_detail_stats, "%s.%s" % [renderer_name, str(key_variant)], float(value))

static func _advance_frame(frame: int) -> void:
	if _current_frame < 0:
		_current_frame = frame
		return
	if frame == _current_frame:
		return
	_finish_frame()
	_current_frame = frame
	_current_frame_total_ms = 0.0

static func _finish_frame() -> void:
	_frame_count += 1
	_frame_total_ms += _current_frame_total_ms
	_frame_max_ms = maxf(_frame_max_ms, _current_frame_total_ms)
	if _current_frame_total_ms >= FRAME_SPIKE_THRESHOLD_MS:
		_frame_spikes += 1

static func _add_stat(stats: Dictionary, key: String, elapsed_ms: float) -> void:
	var entry: Dictionary = stats.get(key, {})
	if entry.is_empty():
		entry = {
			"calls": 0,
			"total_ms": 0.0,
			"max_ms": 0.0,
		}
	entry["calls"] = int(entry["calls"]) + 1
	entry["total_ms"] = float(entry["total_ms"]) + elapsed_ms
	entry["max_ms"] = maxf(float(entry["max_ms"]), elapsed_ms)
	stats[key] = entry

static func _print_summary(now_us: int) -> void:
	var frame_avg_ms := 0.0
	if _frame_count > 0:
		frame_avg_ms = _frame_total_ms / float(_frame_count)
	print(
		"[DEBUG:perf] fps=%.1f frames=%d frame_cpu_avg_ms=%.3f frame_cpu_max_ms=%.3f frame_spikes=%d renderer_ms=%s detail_ms=%s"
		% [
			Engine.get_frames_per_second(),
			_frame_count,
			frame_avg_ms,
			_frame_max_ms,
			_frame_spikes,
			_format_stats(_renderer_stats),
			_format_stats(_detail_stats),
		]
	)
	_last_log_us = now_us
	_frame_count = 0
	_frame_total_ms = 0.0
	_frame_max_ms = 0.0
	_frame_spikes = 0
	_renderer_stats.clear()
	_detail_stats.clear()

static func _format_stats(stats: Dictionary) -> String:
	if stats.is_empty():
		return "none"
	var keys: Array = stats.keys()
	keys.sort()
	var parts: Array[String] = []
	for key_variant in keys:
		var key: String = str(key_variant)
		var entry: Dictionary = stats[key_variant]
		var calls: int = max(1, int(entry.get("calls", 0)))
		var avg_ms: float = float(entry.get("total_ms", 0.0)) / float(calls)
		var max_ms: float = float(entry.get("max_ms", 0.0))
		parts.append("%s(avg=%.3f,max=%.3f,calls=%d)" % [key, avg_ms, max_ms, calls])
	return ",".join(parts)
