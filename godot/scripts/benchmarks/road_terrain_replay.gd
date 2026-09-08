# SPDX-License-Identifier: GPL-2.0-only

## Fixed-location terrain diagnostics using the production gameplay benchmark's RoadTool and fences.
## Does not infer road geometry or choose easier sites. Offline Python audits the exported Rust profiles.
extends RefCounted

const MANIFEST := "res://../benchmarks/fixtures/kuopio-terrain/placements.json"
var _report: Dictionary
var _directory: String
var _label: String
var _preview: Dictionary
var _capture_images := false

func run(bench: Node) -> void:
	bench._metrics = {"phases": []}
	_report = {"schema_version": 1, "benchmark": "road_terrain_replay", "completed": false, "cases": []}
	_directory = bench.results_path.get_basename() + "-artifacts"
	if DirAccess.dir_exists_absolute(_directory):
		await _finish(bench, "artifact directory already exists")
		return
	if DirAccess.make_dir_recursive_absolute(_directory) != OK:
		await _finish(bench, "cannot create artifact directory")
		return
	var manifest_path: String = bench._environment_or("METRUM_GAMEPLAY_TERRAIN_MANIFEST", MANIFEST)
	var manifest = JSON.parse_string(FileAccess.get_file_as_string(manifest_path))
	if not manifest is Dictionary or manifest.get("schema_version") != 1 or not bench._nodes_are_ready():
		await _finish(bench, "invalid manifest or unavailable scene dependencies")
		return
	var source_world: String = ProjectSettings.globalize_path("res://../" + str(manifest.source_world))
	var reference: String = ProjectSettings.globalize_path("res://../" + str(manifest.reference_save))
	if FileAccess.get_sha256(source_world) != manifest.source_world_sha256 or FileAccess.get_sha256(reference) != manifest.reference_save_sha256:
		await _finish(bench, "fixture hash mismatch; refusing to replay a different map")
		return
	bench.world_path = source_world
	bench.input_manager.set_process(false)
	bench.input_manager.set_process_input(false)
	bench.input_manager.set_process_unhandled_input(false)
	if bench.mode == "headless":
		Engine.max_fps = 0
	_capture_images = bench.mode != "headless"
	_report.merge({
		"manifest_sha256": FileAccess.get_sha256(manifest_path),
		"replay_helper_sha256": FileAccess.get_sha256(get_script().resource_path),
		"reference_save_sha256": manifest.reference_save_sha256,
		"source_world_sha256": manifest.source_world_sha256,
		"input_contract": manifest.input_contract, "quality_limits": manifest.quality_limits,
		"runtime": bench._runtime_metadata(), "rendered_captures": _capture_images,
		"timing_contract": "diagnostic run; profile exports/screenshots outside operation timers but perturb caches and preview dwell; not accepted performance samples",
	})
	var selected: String = OS.get_environment("METRUM_GAMEPLAY_TERRAIN_CASE")
	var cases: Array = []
	for fixture in manifest.cases:
		if selected.is_empty() or fixture.case_id == selected:
			cases.append(fixture)
	if cases.is_empty():
		await _finish(bench, "unknown terrain case: " + selected)
		return
	_report["expected_case_ids"] = cases.map(func(fixture): return fixture.case_id)
	# First inspect the user's complete saved city. Loading is non-mutating; no save is overwritten.
	if not bench.input_manager.menu_load_game_from_path(reference):
		await _finish(bench, "reference save failed to load")
		return
	var reference_wait: Dictionary = await bench._wait_for_idle(bench.settle_timeout_sec)
	_report["reference"] = {"wait": reference_wait, "snapshot": _snapshot(bench)}
	_checkpoint(bench)
	for fixture in cases:
		bench._stop_scripted_tool()
		var result := {"case_id": fixture.case_id, "ok": false, "steps": [], "expected_steps": fixture.segments.size()}
		_report.cases.append(result)
		if not bench.input_manager.menu_load_world_definition(source_world):
			result["error"] = "source world failed to load"
			continue
		var reset: Dictionary = await bench._wait_for_idle(bench.settle_timeout_sec)
		if not reset.get("ok", false):
			result["error"] = "clean world did not settle"
			result["wait"] = reset
			continue
		bench.input_manager.current_tool = bench.input_manager.Tool.ROAD
		bench.input_manager._activate_tool_logic(bench.input_manager.Tool.ROAD, true)
		for stroke in fixture.segments:
			_label = "%s_%02d" % [fixture.case_id, int(stroke.attempt)]
			var step: Dictionary = await _run_stroke(bench, fixture, stroke)
			result.steps.append(step)
			_checkpoint(bench)
			if not step.get("ok", false):
				result["error"] = "placement failed; dependent steps not attempted"
				break
		result["ok"] = result.steps.size() == fixture.segments.size() and not result.has("error")
		print("[TERRAIN_REPLAY] %s steps=%d/%d placement_ok=%s" % [fixture.case_id, result.steps.size(), fixture.segments.size(), result.ok])
	_report["completed"] = true
	await _finish(bench)

func _run_stroke(bench: Node, fixture: Dictionary, stroke: Dictionary) -> Dictionary:
	var start := Vector2(float(stroke.start_xz[0]), float(stroke.start_xz[1]))
	var end := Vector2(float(stroke.end_xz[0]), float(stroke.end_xz[1]))
	var midpoint := (start + end) * 0.5
	# Fixed orthographic framing exposes road-edge defects without a distant horizon dominating.
	bench.camera.set("orthogonal", true)
	bench.camera.projection = Camera3D.PROJECTION_ORTHOGONAL
	bench.camera.set_focus_padding(2.0)
	bench.camera.focus_on(bench._surface_point(midpoint), maxf(60.0, start.distance_to(end) * 0.55 + 10.0))
	var settled: Dictionary = await bench._wait_for_idle(bench.settle_timeout_sec)
	if not settled.get("ok", false):
		return {"attempt": stroke.attempt, "ok": false, "wait": settled}
	await _capture(bench, "before")
	var segment := {
		"start_offset": start, "end_offset": end, "draw_mode": 0,
		"fwd_lanes": int(stroke.fwd_lanes), "bkw_lanes": int(stroke.bkw_lanes),
	}
	_preview = {}
	var operation: Dictionary = await bench._run_segment(
		{"case_id": fixture.case_id, "topology": "logged_terrain"}, 0, false,
		int(stroke.attempt), Vector2.ZERO, segment
	)
	bench._capture_frames = false
	var step := {
		"attempt": stroke.attempt, "original_accepted": stroke.original_accepted,
		"operation": operation, "preview": _preview, "snapshot": _snapshot(bench),
		"expected_live_edges": stroke.expected_live_edges,
		"ok": operation.get("ok", false),
	}
	if step.snapshot is Dictionary and step.snapshot.has("edges"):
		step["ok"] = step.ok and step.snapshot.edges.size() == int(stroke.expected_live_edges)
	else:
		step["ok"] = false
	await _capture(bench, "committed" if step.ok else "failed")
	bench.road_tool.cancel_road()
	return step

func capture_preview(bench: Node) -> void:
	var validation: Dictionary = bench.road_tool._preview_cache_surface
	_preview = {}
	for key in ["is_valid", "invalid_reason", "max_grade", "allowed_grade", "request_id", "surface_generation"]:
		if validation.has(key):
			_preview[key] = validation[key]
	var points: PackedVector3Array = validation.get("prepared_points", PackedVector3Array())
	var profile: Array = []
	for point in points:
		profile.append([point.x, point.y, point.z, bench.simulation_node.get_height_at(Vector2(point.x, point.z))])
	_preview["prepared_profile"] = profile
	await _capture(bench, "preview")

func _snapshot(bench: Node) -> Variant:
	return JSON.parse_string(bench.simulation_node.get_road_terrain_benchmark_snapshot())

func _capture(bench: Node, phase: String) -> void:
	if not _capture_images:
		return
	await RenderingServer.frame_post_draw
	var path := _directory.path_join(_label + "_" + phase + ".png")
	var error := bench.get_viewport().get_texture().get_image().save_png(path)
	if error != OK:
		_report["capture_error"] = "failed to save " + path

func _checkpoint(bench: Node) -> void:
	var phases: Array = bench._metrics.get("phases", [])
	bench._metrics = _report.duplicate(false)
	bench._metrics["phases"] = phases
	if not bench._write_metrics():
		_report["write_error"] = "could not write metrics"

func _finish(bench: Node, error: String = "") -> void:
	bench._stop_scripted_tool()
	if not error.is_empty():
		_report["error"] = error
		push_error("[TERRAIN_REPLAY] " + error)
	_checkpoint(bench)
	# Geometry audit runs in the launcher even when placement/rendering failed.
	bench.get_tree().quit(0 if _report.completed and not _report.has("write_error") else 1)
