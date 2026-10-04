# SPDX-License-Identifier: GPL-2.0-only

## Idle-frame matrix on the gameplay harness: loads the pinned world, holds the camera still at
## fixed poses and records steady-state frame cost with no edits, agents or input.
## Each pose is measured once per render variant so a feature's share of the frame is a matched
## difference, not a profile estimate. Capture windows are printed in engine microseconds so a
## native profile of the same run can be cut to the idle frames.
## Camera paths (METRUM_IDLE_BENCH_PATHS) then move the camera a fixed step per frame, so the
## streaming, LOD and vegetation work a moving view causes is measured on the same frames each run.
## METRUM_IDLE_BENCH_VSYNC=1 keeps vsync on (and METRUM_GAMEPLAY_BENCHMARK_MAX_FPS applies), so path
## frame pacing can be compared against the display refresh rather than measured as throughput.
## METRUM_IDLE_BENCH_CANOPY_DENSITY starts a new game on the pinned world at that many canopy stems
## per hectare instead of loading it with the default forest.
extends RefCounted

const TreeSpecies := preload("res://scripts/renderers/tree_species.gd")

# Offsets are from the world centre, so poses stay meaningful for any pinned world.
const POSES := [
	{"name": "overview", "offset": Vector2(0.0, 0.0), "radius": 900.0},
	{"name": "mid", "offset": Vector2(2000.0, -1500.0), "radius": 400.0},
	{"name": "close", "offset": Vector2(-1500.0, 2000.0), "radius": 120.0},
	{"name": "ground", "offset": Vector2(-1500.0, 2000.0), "radius": 12.0},
	# One of the densest canopy cells on Kuopio (about 510 trees per 400 m cell, the generator's top).
	{"name": "forest", "offset": Vector2(2400.0, 1600.0), "radius": 150.0},
	{"name": "forest_low", "offset": Vector2(2400.0, 1600.0), "radius": 30.0},
	# Among Kuopio's steepest ground (slopes above 1 over 10 m), for the cliff and relief shading.
	{"name": "cliff", "offset": Vector2(-2900.0, 2500.0), "radius": 150.0},
	{"name": "cliff_low", "offset": Vector2(-2900.0, 2500.0), "radius": 40.0},
	# Saved-city poses; pair them with METRUM_IDLE_BENCH_SAVE_PATH and METRUM_IDLE_BENCH_CENTRE.
	{"name": "city", "offset": Vector2(0.0, 0.0), "radius": 60.0},
	{"name": "city_low", "offset": Vector2(0.0, 0.0), "radius": 20.0},
]
# Per-frame camera paths from `from` to `to` (offsets from the world centre) with the orbit radius
# interpolated geometrically. `turns` orbits the view about the moving pivot by that many full
# turns. Steps are per frame, not per second, so every run visits the same poses.
const PATHS := [
	{"name": "pan_high", "from": Vector2(-3000.0, 0.0), "to": Vector2(3000.0, 0.0), "radius": [400.0, 400.0]},
	{"name": "pan_low", "from": Vector2(-2250.0, 2000.0), "to": Vector2(-750.0, 2000.0), "radius": [120.0, 120.0]},
	{"name": "zoom_in", "from": Vector2(0.0, 0.0), "to": Vector2(0.0, 0.0), "radius": [900.0, 30.0]},
	{"name": "orbit_high", "from": Vector2(0.0, 0.0), "to": Vector2(0.0, 0.0), "radius": [400.0, 400.0], "turns": 1.0},
	{"name": "orbit_low", "from": Vector2(-1500.0, 2000.0), "to": Vector2(-1500.0, 2000.0), "radius": [120.0, 120.0], "turns": 1.0},
]
const DEFAULT_PATH_FRAMES := 600
const SLOW_FRAME_MS := 33.3
const VARIANTS := ["full", "no_vegetation", "no_ssil_glow", "no_ssil", "no_glow", "no_shadows", "minimal", "no_water", "no_terrain"]
const DEFAULT_WARMUP_FRAMES := 120
const DEFAULT_CAPTURE_FRAMES := 600

var _report: Dictionary
var _environment: Environment
var _sun: DirectionalLight3D
var _vegetation: Node
var _water: Node3D
var _terrain: Node3D
var _lighting: Node
var _path_t := 0.0

func run(bench: Node) -> void:
	var main := bench.get_parent()
	_environment = (main.get_node("WorldEnvironment") as WorldEnvironment).environment
	_sun = main.get_node("DirectionalLight3D")
	_vegetation = main.get_node("Vegetation")
	_water = main.get_node("Water")
	_terrain = main.get_node("Terrain")
	_lighting = main.get_node("SceneLighting")
	var vsync := OS.get_environment("METRUM_IDLE_BENCH_VSYNC") == "1"
	Engine.max_fps = bench._environment_int("METRUM_GAMEPLAY_BENCHMARK_MAX_FPS", 0, 0) if vsync else 0
	if bench.mode != "headless":
		DisplayServer.window_set_vsync_mode(DisplayServer.VSYNC_ENABLED if vsync else DisplayServer.VSYNC_DISABLED)
		DisplayServer.window_set_title("Metrum Rise - idle frame benchmark")
	var viewport_rid := bench.get_viewport().get_viewport_rid()
	RenderingServer.viewport_set_measure_render_time(viewport_rid, true)
	var warmup_frames: int = bench._environment_int("METRUM_IDLE_BENCH_WARMUP_FRAMES", DEFAULT_WARMUP_FRAMES, 0)
	var capture_frames: int = bench._environment_int("METRUM_IDLE_BENCH_FRAMES", DEFAULT_CAPTURE_FRAMES, 10)
	var variants := _selected(OS.get_environment("METRUM_IDLE_BENCH_VARIANTS"), VARIANTS)
	_report = {
		"schema_version": 1, "benchmark": "idle_frame", "success": false, "mode": bench.mode,
		"runtime": bench._runtime_metadata(), "warmup_frames": warmup_frames,
		"capture_frames": capture_frames, "variants": variants, "phases": [], "samples": [],
		"vsync": vsync, "max_fps": Engine.max_fps,
		"timing_contract": "steady-state frames with vsync and max_fps off unless vsync is set; frame_ms is process-frame wall time, render_cpu/gpu are RenderingServer viewport measurements",
	}
	bench._metrics = _report

	var load_start_us := Time.get_ticks_usec()
	print("[IDLE_BENCH] LOAD_BEGIN t_us=%d" % load_start_us)
	var save_path := OS.get_environment("METRUM_IDLE_BENCH_SAVE_PATH")
	# Canopy stems per hectare for a new game on the pinned world, as the new-game dialog sets it.
	var canopy_density := OS.get_environment("METRUM_IDLE_BENCH_CANOPY_DENSITY")
	var loaded: bool
	if not save_path.is_empty():
		loaded = bench.input_manager.menu_load_game_from_path(save_path)
	elif not canopy_density.is_empty():
		loaded = bench.input_manager.menu_start_new_game(bench.world_path,
			{"canopy_stems_per_ha": float(canopy_density)})
	else:
		loaded = bench._load_benchmark_world()
	_report["canopy_stems_per_ha"] = float(canopy_density) if not canopy_density.is_empty() else null
	if not loaded:
		bench._fail("failed to load %s" % (save_path if not save_path.is_empty() else bench.world_path))
		return
	# A saved city keeps its agents; hold the simulation so only rendering varies.
	bench.input_manager.set_simulation_speed(0.0)
	# Fixed daylight so screenshots of saved cities compare like with like.
	bench.get_parent().menu_set_time_of_day(float(bench._environment_int("METRUM_IDLE_BENCH_HOUR", 10, 0)))
	var load_call_ms: float = bench._elapsed_ms(load_start_us)
	var load_wait: Dictionary = await bench._wait_for_idle(bench.settle_timeout_sec)
	_report.phases.append({"phase": "world_load", "load_call_ms": load_call_ms,
		"settle_ms": load_wait.get("elapsed_ms", 0.0), "total_ms": bench._elapsed_ms(load_start_us)})
	print("[IDLE_BENCH] LOAD_END t_us=%d load_call_ms=%.1f total_ms=%.1f" % [
		Time.get_ticks_usec(), load_call_ms, bench._elapsed_ms(load_start_us)])
	if not bool(load_wait.get("ok", false)):
		bench._fail("world render work did not settle", load_wait)
		return

	var centre := Vector2.ZERO
	var centre_text := OS.get_environment("METRUM_IDLE_BENCH_CENTRE").split(",", false)
	if centre_text.size() == 2:
		centre = Vector2(float(centre_text[0]), float(centre_text[1]))
	for pose in _selected_poses(OS.get_environment("METRUM_IDLE_BENCH_POSES")):
		var xz: Vector2 = centre + pose.offset
		var y := float(bench.simulation_node.get_world_surface_height(xz))
		bench.camera.focus_on(Vector3(xz.x, y, xz.y), pose.radius)
		for variant in variants:
			_apply_variant(variant)
			var settle: Dictionary = await bench._wait_for_idle(bench.settle_timeout_sec)
			if not bool(settle.get("ok", false)):
				bench._fail("pose %s/%s did not settle" % [pose.name, variant], settle)
				return
			for i in range(warmup_frames):
				await bench.get_tree().process_frame
			var sample := await _capture(bench, viewport_rid, capture_frames)
			sample.merge({"pose": pose.name, "variant": variant, "settle_ms": settle.elapsed_ms})
			# Optional visual baseline: the last captured frame, for before/after image diffs.
			var screenshot_dir := OS.get_environment("METRUM_IDLE_BENCH_SCREENSHOT_DIR")
			if not screenshot_dir.is_empty():
				# Wind sway depends on wall-clock shader TIME, so it never matches between runs.
				_set_vegetation_wind(0.0)
				await bench.get_tree().process_frame
				await bench.get_tree().process_frame
				DirAccess.make_dir_recursive_absolute(screenshot_dir)
				var screenshot_path := screenshot_dir.path_join("%s-%s.png" % [pose.name, variant])
				bench.get_viewport().get_texture().get_image().save_png(screenshot_path)
				sample["screenshot"] = screenshot_path
				_set_vegetation_wind(1.0)
			_report.samples.append(sample)
			print("[IDLE_BENCH] %-8s %-14s frame p50=%.2f p95=%.2f ms  process=%.2f  render_cpu=%.2f gpu=%.2f  draws=%d objects=%d prims=%d  t_us=%d..%d" % [
				pose.name, variant, sample.frame_ms.p50, sample.frame_ms.p95, sample.process_ms.p50,
				sample.render_cpu_ms.p50, sample.render_gpu_ms.p50, sample.draw_calls,
				sample.objects, sample.primitives, sample.t_begin_us, sample.t_end_us])
	_apply_variant("full")
	var path_frames: int = bench._environment_int("METRUM_IDLE_BENCH_PATH_FRAMES", DEFAULT_PATH_FRAMES, 10)
	for path in _selected(OS.get_environment("METRUM_IDLE_BENCH_PATHS"), PATHS.map(func(p): return p.name)):
		var definition: Dictionary = PATHS.filter(func(p): return p.name == path)[0]
		_focus_path(bench, centre, definition, 0.0)
		var settle: Dictionary = await bench._wait_for_idle(bench.settle_timeout_sec)
		if not bool(settle.get("ok", false)):
			bench._fail("path %s did not settle at its start" % path, settle)
			return
		for i in range(warmup_frames):
			await bench.get_tree().process_frame
		var sample := await _capture(bench, viewport_rid, path_frames,
			func(frame: int): _focus_path(bench, centre, definition, float(frame + 1) / path_frames))
		var settle_after: Dictionary = await bench._wait_for_idle(bench.settle_timeout_sec)
		sample.merge({"path": path, "variant": "full", "settle_after_ms": settle_after.get("elapsed_ms", 0.0)})
		_report.samples.append(sample)
		print("[IDLE_BENCH] %-10s %-6s frame p50=%.2f p95=%.2f p99=%.2f max=%.2f ms  slow(>%.0fms)=%d  jitter p50=%.2f p95=%.2f  process p50=%.2f p95=%.2f  settle_after=%.0f ms  t_us=%d..%d" % [
			path, "full", sample.frame_ms.p50, sample.frame_ms.p95, sample.frame_ms.p99,
			sample.frame_ms.max, SLOW_FRAME_MS, sample.slow_frames, sample.jitter_ms.p50, sample.jitter_ms.p95, sample.process_ms.p50,
			sample.process_ms.p95, settle_after.get("elapsed_ms", 0.0), sample.t_begin_us, sample.t_end_us])
	_report.success = true
	var written: bool = bench._write_metrics()
	bench.get_tree().quit(0 if written else 1)

func _focus_path(bench: Node, centre: Vector2, path: Dictionary, t: float) -> void:
	var xz: Vector2 = centre + (path.from as Vector2).lerp(path.to, t)
	var radius: float = path.radius[0] * pow(path.radius[1] / path.radius[0], t)
	var y := float(bench.simulation_node.get_world_surface_height(xz))
	bench.camera.focus_on(Vector3(xz.x, y, xz.y), radius)
	# orbit() takes mouse motion, so the yaw step is divided back out of the sensitivity.
	if t > _path_t and path.has("turns"):
		bench.camera.orbit(Vector2(float(path.turns) * TAU * (t - _path_t) / bench.camera.sensitivity, 0.0))
	_path_t = t

## `step`, when valid, runs before each captured frame with the frame index (camera paths).
func _capture(bench: Node, viewport_rid: RID, frames: int, step := Callable()) -> Dictionary:
	var frame_ms := PackedFloat64Array()
	var process_ms := PackedFloat64Array()
	var render_cpu_ms := PackedFloat64Array()
	var render_gpu_ms := PackedFloat64Array()
	var draw_calls := 0
	var objects := 0
	var primitives := 0
	var begin_us := Time.get_ticks_usec()
	var last_us := begin_us
	var slow_frames := 0
	# Change between consecutive frame times: stutter that a percentile of frame time hides.
	var jitter_ms := PackedFloat64Array()
	for i in range(frames):
		if step.is_valid():
			step.call(i)
		await bench.get_tree().process_frame
		var now_us := Time.get_ticks_usec()
		if float(now_us - last_us) / 1000.0 > SLOW_FRAME_MS:
			slow_frames += 1
		if not frame_ms.is_empty():
			jitter_ms.append(absf(float(now_us - last_us) / 1000.0 - frame_ms[-1]))
		frame_ms.append(float(now_us - last_us) / 1000.0)
		last_us = now_us
		process_ms.append(Performance.get_monitor(Performance.TIME_PROCESS) * 1000.0)
		render_cpu_ms.append(RenderingServer.viewport_get_measured_render_time_cpu(viewport_rid)
			+ RenderingServer.get_frame_setup_time_cpu())
		render_gpu_ms.append(RenderingServer.viewport_get_measured_render_time_gpu(viewport_rid))
		draw_calls += RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_TOTAL_DRAW_CALLS_IN_FRAME)
		objects += RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_TOTAL_OBJECTS_IN_FRAME)
		primitives += RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_TOTAL_PRIMITIVES_IN_FRAME)
	return {
		"t_begin_us": begin_us, "t_end_us": last_us,
		"frame_ms": _stats(frame_ms), "process_ms": _stats(process_ms),
		"render_cpu_ms": _stats(render_cpu_ms), "render_gpu_ms": _stats(render_gpu_ms),
		"slow_frames": slow_frames, "jitter_ms": _stats(jitter_ms),
		"frame_ms_series": frame_ms if step.is_valid() else PackedFloat64Array(),
		"draw_calls": draw_calls / frames, "objects": objects / frames, "primitives": primitives / frames,
	}

func _apply_variant(variant: String) -> void:
	var minimal := variant == "minimal"
	_vegetation.enabled = not (minimal or variant == "no_vegetation")
	var effects := not (minimal or variant == "no_ssil_glow")
	# The day cycle writes ssil_enabled every frame (on only while windows can be lit).
	_lighting.ssil_allowed = effects and variant != "no_ssil"
	_environment.glow_enabled = effects and variant != "no_glow"
	_sun.shadow_enabled = not (minimal or variant == "no_shadows")
	_water.visible = variant != "no_water"
	_terrain.visible = variant != "no_terrain"

# Screenshot-only: TreeSpecies caches every vegetation ShaderMaterial in static containers.
func _set_vegetation_wind(strength: float) -> void:
	var materials: Array = [TreeSpecies._wind_material, TreeSpecies._card_material]
	materials.append_array(TreeSpecies._canopy_materials)
	materials.append_array(TreeSpecies._impostor_materials)
	materials.append_array(TreeSpecies._impostor_shadow_materials)
	for pair in TreeSpecies._form_materials.values():
		materials.append_array(pair)
	for material in materials:
		if material is ShaderMaterial:
			material.set_shader_parameter("wind_strength", strength)

func _stats(values: PackedFloat64Array) -> Dictionary:
	var sorted := values.duplicate()
	sorted.sort()
	var total := 0.0
	for value in sorted:
		total += value
	var count := sorted.size()
	return {
		"mean": total / count, "p50": sorted[count / 2],
		"p95": sorted[mini(count - 1, int(count * 0.95))],
		"p99": sorted[mini(count - 1, int(count * 0.99))], "max": sorted[count - 1],
	}

func _selected(text: String, all: Array) -> Array:
	var wanted := text.split(",", false)
	return all.filter(func(name): return wanted.is_empty() or name in wanted)

func _selected_poses(text: String) -> Array:
	var wanted := text.split(",", false)
	return POSES.filter(func(pose): return wanted.is_empty() or pose.name in wanted)
