# SPDX-License-Identifier: GPL-2.0-only

## Graded preview drag probe (`TERRAIN-07-E`; the gameplay harness's `graded_drag` matrix, meant
## for a rendered run on a real display): replays the pinned Kuopio strokes of the terrain
## replay's manifest through the production RoadTool as `benchmark_kuopio_graded_drags` drags them
## in Rust: eight poses growing from the stroke's start to its end, one pose a frame, clicked in
## the frame after the last pose and undone, then dragged again, caught up (the frames that stage
## the rest of the last pose's graded tiles) and clicked; the case goes on from that commit.
## Per frame it records the terrain renderer's install share: on a pose or catch-up frame
## show_preview (the slot uploads and the cell and wall instance swaps), on the frame that installs
## a commit's or undo's edit batch that frame's graded work (the staged tiles' uploads, the
## preview's release and the swaps); with the bytes uploaded, the patches drawing graded cells, the
## frame's wall time and the viewport's last measured GPU time. It checks the renderer's bookkeeping: no pool exhausted, no
## bake failed, nothing created by a drag or an edit beyond the pages and variants the renderer
## creates on demand, each batch's tiles installed with the preview released, and a catch-up of
## one share a frame.
## Env: METRUM_GRADED_DRAG_CASES (case ids, comma-separated; default every case),
## METRUM_GRADED_DRAG_DISTANCE (the camera's orbit distance from the stroke's end, m; default 40,
## so the tiles around the click take GL0, the dearest to upload), METRUM_GRADED_DRAG_CAPTURE_DIR
## (rendered runs only: a PNG of the converged preview before its click and of the settled commit,
## for T42's visual check; the capture adds a frame before the click, so a timing run leaves it unset).
## Every recorded frame is also checked for holes (`TERRAIN-12`): each drawing set's tiles draw what
## they show (`tile_draw_violations`), and no preview tile shows while the road tool draws no preview
## road (its bake's footprint, where the terrain discards, would show the sky). Check-only options:
## METRUM_GRADED_DRAG_SWEEP (degrees: before its drags each stroke is swept, its end swinging that
## far to each side of the stroke at the stroke's length, a pose a frame, as a player swings a
## stroke on a slope, then cancelled; with METRUM_GRADED_DRAG_SWEEP_CONVERGE each pose caught up
## before the next, a swing with pauses), METRUM_GRADED_DRAG_SAVE (a save to load instead of the
## manifest's world; its strokes from METRUM_GRADED_DRAG_STROKES, `x0,z0,x1,z1;...`) and
## METRUM_GRADED_DRAG_ATTEMPTS (the strokes' attempt numbers to run) and
## METRUM_GRADED_DRAG_HOLE_PIXELS (rendered runs: the background drawn magenta, the camera looking
## down, and every frame's magenta pixels counted: the sky through the ground).
extends RefCounted

const MANIFEST := "res://../benchmarks/fixtures/kuopio-terrain/placements.json"
const POSES := 8
# The Godot terrain install share of a pose, commit or undo frame (`terrain.md` budgets).
const SHARE_MS := 1.0
# Tiles a catch-up frame stages (`graded_stage.rs`'s CATCH_UP_FRAME_TILES, two since
# `TERRAIN-07-G` round 7).
const CATCH_UP_FRAME_TILES := 2
const CATCH_UP_LIMIT := 64
const EDIT_FRAME_LIMIT := 600
const SETTLE_FRAME_LIMIT := 6000
const CREATED := ["textures_created", "images_created", "meshes_created", "materials_created", "multimeshes_created"]
const HEALTH := ["pool_exhausted", "bake_failures", "walls_truncated"]
const KINDS := ["pose", "catch_up", "fast_commit", "undo", "commit"]

var _bench: Node
var _graded: RefCounted
var _viewport: RID
var _distance := 40.0
var _capture_dir := ""
var _report: Dictionary
var _failures: Array[String] = []
# The frames of the drag or edit being recorded, and the kind of the frame now running.
var _frames: Array = []
var _kind := ""
var _last_us := 0
var _seen := {}
# Recorded frames with a resident patch drawing nothing.
var _holes := 0
# TERRAIN-12's checks: recorded frames with a set's tile drawing other than what it shows, with a
# preview tile shown and no preview road drawn, and (METRUM_GRADED_DRAG_HOLE_PIXELS) with the
# background showing through the ground; the sweep's swing (degrees, 0: none).
var _tile_holes := 0
var _unroaded := 0
var _pixel_holes := 0
var _sweep_deg := 0.0
var _sweep_converge := false
var _continue := not OS.get_environment("METRUM_GRADED_DRAG_CONTINUE").is_empty()
var _hole_pixels := false

func run(bench: Node) -> void:
	_bench = bench
	_report = {
		"schema_version": 1, "benchmark": "graded_drag", "success": false, "mode": bench.mode,
		"share_ms": SHARE_MS, "poses": POSES, "strokes": [], "loads": [], "failures": _failures,
		"timing_contract": "frame-start samples of the terrain renderer's counters; install ms are main-thread CPU time of show_preview and of the edit batch's frame (uploads queue on the rendering device); headless renders nothing",
	}
	bench._metrics = _report
	var manifest_path: String = bench._environment_or("METRUM_GAMEPLAY_TERRAIN_MANIFEST", MANIFEST)
	var manifest = JSON.parse_string(FileAccess.get_file_as_string(manifest_path))
	if not manifest is Dictionary or manifest.get("schema_version") != 1 or not bench._nodes_are_ready():
		_finish("invalid manifest or unavailable scene dependencies")
		return
	var world: String = ProjectSettings.globalize_path("res://../" + str(manifest.source_world))
	if FileAccess.get_sha256(world) != manifest.source_world_sha256:
		_finish("source world hash mismatch; refusing to replay a different map")
		return
	var distance_text := OS.get_environment("METRUM_GRADED_DRAG_DISTANCE")
	if distance_text.is_valid_float():
		_distance = float(distance_text)
	if OS.get_environment("METRUM_GRADED_DRAG_SWEEP").is_valid_float():
		_sweep_deg = float(OS.get_environment("METRUM_GRADED_DRAG_SWEEP"))
	_sweep_converge = not OS.get_environment("METRUM_GRADED_DRAG_SWEEP_CONVERGE").is_empty()
	if bench.mode != "headless":
		_capture_dir = OS.get_environment("METRUM_GRADED_DRAG_CAPTURE_DIR")
		_hole_pixels = not OS.get_environment("METRUM_GRADED_DRAG_HOLE_PIXELS").is_empty()
	var save := OS.get_environment("METRUM_GRADED_DRAG_SAVE")
	if not save.is_empty():
		manifest = {"source_world_sha256": FileAccess.get_sha256(save), "cases": [{"case_id": "save", "save": save, "segments": _save_strokes()}]}
	bench.input_manager.set_process(false)
	bench.input_manager.set_process_input(false)
	bench.input_manager.set_process_unhandled_input(false)
	Engine.max_fps = 0
	if bench.mode != "headless":
		DisplayServer.window_set_vsync_mode(DisplayServer.VSYNC_ENABLED)
		DisplayServer.window_set_title("Metrum Rise - graded drag probe")
	_viewport = bench.get_viewport().get_viewport_rid()
	RenderingServer.viewport_set_measure_render_time(_viewport, true)
	_report.merge({"runtime": bench._runtime_metadata(), "camera_distance_m": _distance,
		"manifest_sha256": FileAccess.get_sha256(manifest_path), "source_world_sha256": manifest.source_world_sha256,
		"helper_sha256": FileAccess.get_sha256(get_script().resource_path)})
	var selected := OS.get_environment("METRUM_GRADED_DRAG_CASES").split(",", false)
	for fixture in manifest.cases:
		if not selected.is_empty() and not fixture.case_id in selected:
			continue
		bench._stop_scripted_tool()
		var loads: bool = (bench.input_manager.menu_load_game_from_path(fixture.save) if fixture.has("save")
			else bench.input_manager.menu_load_world_definition(world))
		if not loads:
			_finish("the source world failed to load")
			return
		bench.input_manager.set_simulation_speed(0.0)
		var loaded := {}
		var load_us := Time.get_ticks_usec()
		if not await _settle(loaded):
			_finish("%s: the loaded world did not settle: %s" % [fixture.case_id, loaded])
			return
		_graded = bench.terrain.get("graded_tiles")
		# The settle includes every resident patch's cell set build (off the frame paths).
		_report.loads.append({"case": fixture.case_id, "settle_ms": float(Time.get_ticks_usec() - load_us) / 1000.0,
			"cell_builds": int(_graded.counters["cell_builds"]), "patches_with_cells": _graded.patch_cells.size()})
		if _graded == null:
			_finish("the terrain renderer draws no graded tiles")
			return
		# The view right after a load, where round 6's display run showed the holes.
		_report.loads[-1]["holes"] = _graded.hole_patches()
		if int(_report.loads[-1]["holes"]) > 0:
			_failures.append("%s: %d resident patches draw nothing after the load" % [fixture.case_id, int(_report.loads[-1]["holes"])])
			push_error("[GRADED_DRAG] " + _failures[-1])
		if _hole_pixels:
			_magenta_background()
		bench.input_manager.current_tool = bench.input_manager.Tool.ROAD
		bench.input_manager._activate_tool_logic(bench.input_manager.Tool.ROAD, true)
		var attempts := OS.get_environment("METRUM_GRADED_DRAG_ATTEMPTS").split(",", false)
		for stroke in fixture.segments:
			if not attempts.is_empty() and not str(int(stroke.attempt)) in attempts:
				continue
			var record := await _stroke(str(fixture.case_id), stroke)
			_report.strokes.append(record)
			bench._write_metrics()
			print("[GRADED_DRAG] %s" % JSON.stringify(_stroke_line(record)))
			# A stroke whose commit or edit failed leaves the case's city unknown; one that only
			# showed holes does not (METRUM_GRADED_DRAG_CONTINUE: go on, to check every stroke).
			if not record.ok and not (_continue and record.get("committed_both", false)):
				break
	_report["summary"] = _summary()
	print("[GRADED_DRAG_SUMMARY] %s" % JSON.stringify(_report.summary))
	_finish("")

# One stroke: the fast drag's click and its undo, then the converged drag's click.
func _stroke(case_id: String, stroke: Dictionary) -> Dictionary:
	var start := Vector2(float(stroke.start_xz[0]), float(stroke.start_xz[1]))
	var end := Vector2(float(stroke.end_xz[0]), float(stroke.end_xz[1]))
	var record := {"case": case_id, "attempt": int(stroke.attempt), "ok": false}
	_bench.camera.set_focus_padding(1.0)
	if _hole_pixels:
		# Looking down on the whole swing, so no frame shows the horizon.
		var reach := start.distance_to(end) * (1.2 if _sweep_deg > 0.0 else 0.6)
		_bench.camera.focus_on(_bench._surface_point(start.lerp(end, 0.0 if _sweep_deg > 0.0 else 0.5)), reach)
		_bench.camera.orbit(Vector2(0.0, 100000.0))
	else:
		_bench.camera.focus_on(_bench._surface_point(end), _distance)
	if not await _settle(record):
		return record
	var failures := _failures.size()
	var before: Dictionary = _graded.get_pool_counters()
	if _hole_pixels:
		_begin_frames()
		_kind = "settled"
		await _next_frame()
		_expect(int(_frames[-1].get("magenta", 0)) == 0, record, "the settled view shows no background: %d pixels" % int(_frames[-1].get("magenta", 0)))
	if _sweep_deg > 0.0:
		record["sweep"] = await _sweep(record, start, end, stroke)
		if not await _settle(record):
			return record
	if stroke.get("sweep_only", false):
		_check_frames(record)
		record.ok = _failures.size() == failures
		return record
	record["fast"] = await _drag(record, start, end, stroke, false)
	if not await _settle(record):
		return record
	if record.fast.committed:
		_begin_frames()
		_kind = "undo"
		_expect(_bench.simulation_node.undo_action(), record, "the fast commit undoes")
		record["undo"] = await _edit(record, "undo")
		record.undo["frames"] = _frames
		if not await _settle(record):
			return record
	record["converged"] = await _drag(record, start, end, stroke, true)
	if not await _settle(record):
		return record
	await _capture(record, "commit")
	var after: Dictionary = _graded.get_pool_counters()
	# The renderer creates two things on demand since `TERRAIN-07-G`: a page (its two textures)
	# when a block's page fills (T54), and a set's behind-the-road variant (four MultiMeshes and
	# four materials) with its patch's first flagged tile (round 10). Anything else fails.
	var pages := int(after["pages_created"]) - int(before["pages_created"])
	var variants := int(after["cell_variants_created"]) - int(before["cell_variants_created"])
	var on_demand := {"textures_created": 2 * pages, "materials_created": 4 * variants,
		"multimeshes_created": 4 * variants}
	for name: String in CREATED + HEALTH:
		var changed := int(after[name]) - int(before[name])
		_expect(changed == int(on_demand.get(name, 0)), record, "%s changed by %d (%d on demand)"
			% [name, changed, int(on_demand.get(name, 0))])
	record["pages_created"] = pages
	record["variants_created"] = variants
	# The preview's page-full lag (T27 (d)) is expected; a pool exhausted is a failure (HEALTH).
	record["preview_page_waits"] = int(after["preview_page_waits"]) - int(before["preview_page_waits"])
	_expect(record.fast.committed and record.converged.committed, record, "both clicks commit")
	record["committed_both"] = record.fast.committed and record.converged.committed
	_check_frames(record)
	_expect(_graded.hole_patches() == 0, record, "after the stroke a resident patch draws nothing")
	record.ok = _failures.size() == failures
	return record

# The stroke's per-frame hole counts (patches drawing nothing; TERRAIN-12's tiles, preview tiles
# without their road, background pixels), each a failure, then reset for the next stroke.
func _check_frames(record: Dictionary) -> void:
	_expect(_holes == 0, record, "%d frames drew a resident patch with nothing (a hole)" % _holes)
	_expect(_tile_holes == 0, record, "%d frames drew a set's tile other than it shows" % _tile_holes)
	_expect(_unroaded == 0, record, "%d frames showed preview terrain without its preview road" % _unroaded)
	_expect(_pixel_holes == 0, record, "%d frames showed the background through the ground" % _pixel_holes)
	record["frame_holes"] = {"patches": _holes, "tiles": _tile_holes, "unroaded": _unroaded, "pixels": _pixel_holes}
	_holes = 0
	_tile_holes = 0
	_unroaded = 0
	_pixel_holes = 0

# Swings the stroke's end from `_sweep_deg` on one side of the stroke to as far on the other and
# back, at the stroke's length, a pose a frame (a player swinging a preview over a slope), then
# cancels it: a moving preview whose earlier poses' tiles lie where its road no longer is.
func _sweep(record: Dictionary, start: Vector2, end: Vector2, stroke: Dictionary) -> Dictionary:
	var tool: Node = _bench.road_tool
	var segment := {"draw_mode": 0, "fwd_lanes": int(stroke.fwd_lanes), "bkw_lanes": int(stroke.bkw_lanes)}
	var arm := end - start
	var poses := 4 * maxi(1, int(_sweep_deg / 5.0))
	var swing := func(pose: int) -> Vector3:
		# A triangle wave: 0 -> +sweep -> -sweep -> 0 over the poses.
		var t := float(pose) / poses
		var phase := 4.0 * t if t < 0.25 else (2.0 - 4.0 * t if t < 0.75 else 4.0 * t - 4.0)
		return _bench._surface_point(start + arm.rotated(deg_to_rad(_sweep_deg) * phase))
	_begin_frames()
	_kind = "sweep"
	_bench._begin_scripted_road(Vector2.ZERO, segment, _bench._surface_point(start), swing.call(0))
	for pose in range(1, poses + 1):
		await _next_frame()
		if _sweep_converge:
			# A pause in the swing: the pose's tiles all staged before the next pose moves them.
			_kind = "catch_up"
			var catch_up := 0
			while tool._graded_pending > 0 and catch_up < CATCH_UP_LIMIT:
				await _next_frame()
				catch_up += 1
			_kind = "sweep"
		tool.set_scripted_pointer(true, swing.call(pose))
		tool._queue_preview_update()
	await _next_frame()
	var invalid := 0
	for row in _frames:
		invalid += 0 if bool(row.get("valid", true)) else 1
	tool.cancel_road()
	await _next_frame()
	return {"poses": poses, "invalid_frames": invalid, "frames": _frames}

# The save's strokes: METRUM_GRADED_DRAG_STROKES, `x0,z0,x1,z1` each, `;`-separated, two-lane.
func _save_strokes() -> Array:
	var strokes := []
	for text: String in OS.get_environment("METRUM_GRADED_DRAG_STROKES").split(";", false):
		var v := text.split_floats(",", false)
		if v.size() == 4:
			strokes.append({"attempt": strokes.size(), "start_xz": [v[0], v[1]], "end_xz": [v[2], v[3]],
				"fwd_lanes": 1, "bkw_lanes": 1, "sweep_only": _sweep_deg > 0.0})
	return strokes

# The background a plain magenta and the ambient light a plain grey (no sky, fog or reflections),
# so a pixel of exactly that colour is the background showing through the ground: the camera's own
# environment, which overrides the world's (the day cycle keeps setting that one's sky).
func _magenta_background() -> void:
	var environment := Environment.new()
	_bench.get_viewport().get_camera_3d().environment = environment
	environment.background_mode = Environment.BG_COLOR
	environment.background_color = Color(1.0, 0.0, 1.0)
	environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	environment.ambient_light_color = Color(0.5, 0.5, 0.5)
	environment.reflected_light_source = Environment.REFLECTION_SOURCE_DISABLED
	environment.fog_enabled = false
	environment.volumetric_fog_enabled = false

# Pixels of the last drawn frame that are the magenta background, every second pixel each way;
# with a capture directory the first frames showing any are saved as hole_<n>.png.
var _hole_captures := 0
var _capture_all := not OS.get_environment("METRUM_GRADED_DRAG_CAPTURE_ALL").is_empty()
func _magenta_pixels() -> int:
	var image := _bench.get_viewport().get_texture().get_image()
	image.convert(Image.FORMAT_RGBA8)
	var data := image.get_data()
	var width := image.get_width()
	var count := 0
	for y in range(0, image.get_height(), 2):
		var row := y * width * 4
		for x in range(0, width, 2):
			var at := row + x * 4
			if data[at] >= 250 and data[at + 1] <= 5 and data[at + 2] >= 250:
				count += 1
	if (count > 0 or _capture_all) and not _capture_dir.is_empty() and _hole_captures < 12 and _kind != "settled":
		DirAccess.make_dir_recursive_absolute(_capture_dir)
		image.save_png(_capture_dir.path_join("hole_%02d_%s_%d.png" % [_hole_captures, _kind, _frames.size()]))
		_hole_captures += 1
	return count

# Drags the stroke in POSES poses, a pose a frame; `converge` stages the catch-up frames before
# the click, else the click follows the last pose's frame at once. Then waits for the commit's
# edit batch.
func _drag(record: Dictionary, start: Vector2, end: Vector2, stroke: Dictionary, converge: bool) -> Dictionary:
	var tool: Node = _bench.road_tool
	var segment := {"draw_mode": 0, "fwd_lanes": int(stroke.fwd_lanes), "bkw_lanes": int(stroke.bkw_lanes)}
	var end_pos: Vector3 = _bench._surface_point(end)
	_begin_frames()
	# A pose's pointer is set at the start of the frame that previews it; the last pose is the end.
	_kind = "pose"
	_bench._begin_scripted_road(Vector2.ZERO, segment, _bench._surface_point(start),
		_bench._surface_point(start.lerp(end, 1.0 / POSES)))
	for pose in range(2, POSES + 1):
		await _next_frame()
		tool.set_scripted_pointer(true, end_pos if pose == POSES else _bench._surface_point(start.lerp(end, float(pose) / POSES)))
		tool._queue_preview_update()
	await _next_frame()
	var pending := int(tool._graded_pending)
	var catch_up := 0
	if converge:
		_kind = "catch_up"
		while tool._graded_pending > 0 and catch_up < CATCH_UP_LIMIT:
			await _next_frame()
			catch_up += 1
		_expect(tool._graded_pending == 0, record, "the preview converges")
		_expect(catch_up == ceili(float(pending) / CATCH_UP_FRAME_TILES), record,
			"one share a catch-up frame: %d frames for %d tiles" % [catch_up, pending])
		await _capture(record, "preview")
	var out := {"valid": bool(tool._preview.get("is_valid", false)), "pending_after_poses": pending,
		"catch_up_frames": catch_up, "committed": false}
	# The click, before the road tool's processing of this frame; no chained stroke follows it.
	var kind := "commit" if converge else "fast_commit"
	_kind = kind
	var queued: bool = tool._commit_segment(end_pos)
	tool.active = false
	_expect(queued, record, "the %s queues" % kind)
	if queued:
		var edit := await _edit(record, kind)
		out["committed"] = bool(edit.get("installed", false))
		out["edit"] = edit
	out["frames"] = _frames
	return out

# Waits, from the frame of a commit's click or an undo (`_kind`), for the renderer to apply the
# edit and install its batch; the frames after the first are recorded as `wait`, the batch's frame
# as `kind`.
func _edit(record: Dictionary, kind: String) -> Dictionary:
	var edits := int(_graded.counters["edits"])
	var frames := 0
	await _next_frame()
	_kind = "wait"
	while frames < EDIT_FRAME_LIMIT and (int(_graded.counters["edits"]) == edits or not _graded._edit_batch.is_empty() or _graded._edit_release >= 0):
		await _next_frame()
		frames += 1
	var batch: Array = _frames.filter(func(row): return row.has("batch_ms"))
	var installed: bool = frames < EDIT_FRAME_LIMIT
	_expect(installed, record, "the %s's edit installs" % kind)
	_expect(batch.size() == 1, record, "the %s installs in one batch frame: %d" % [kind, batch.size()])
	# Read in the frame after the batch: its tiles drawn, the preview released.
	for key in _graded.last_edit_batch:
		_expect(_graded.installed.has(key), record, "the %s's tile %s is installed" % [kind, str(key)])
	_expect(_graded.preview.is_empty(), record, "the %s releases the preview" % kind)
	for row in batch:
		row["kind"] = kind
	return {"installed": installed, "wait_frames": frames}

func _begin_frames() -> void:
	_frames = []
	_last_us = Time.get_ticks_usec()
	_remember()

func _remember() -> void:
	for name in ["uploads", "upload_bytes", "preview_frames", "edit_batches", "edit_adopted", "cell_builds"]:
		_seen[name] = int(_graded.counters[name])

# Awaits the next frame and records the one that just ran (`_kind`): resumed at the frame's start,
# before any node processes it.
func _next_frame() -> void:
	await _bench.get_tree().process_frame
	var now := Time.get_ticks_usec()
	var counters: Dictionary = _graded.counters
	var row := {
		"kind": _kind, "frame_ms": float(now - _last_us) / 1000.0,
		# The frame's end on Godot's clock, to place a profile's samples (`ROAD-68-H` round 2).
		"end_us": now,
		"gpu_ms": RenderingServer.viewport_get_measured_render_time_gpu(_viewport),
		"terrain_ms": float(counters["install_ms_last"]),
		"uploads": int(counters["uploads"]) - int(_seen["uploads"]),
		"upload_bytes": int(counters["upload_bytes"]) - int(_seen["upload_bytes"]),
		"pending": int(_bench.road_tool._graded_pending),
		"cell_patches": _graded.patch_cells.size(),
		"cell_builds": int(counters["cell_builds"]) - int(_seen["cell_builds"]),
		# Resident patches drawing nothing (sky through the ground): a failure in any frame.
		"holes": _graded.hole_patches(),
		"tile_draw": _graded.tile_draw_violations(),
		"valid": bool(_bench.road_tool._preview.get("is_valid", false)),
		"preview_shown": _graded.preview.size(),
	}
	if int(row["holes"]) > 0:
		_holes += 1
	if int(row["tile_draw"]["holes"]) > 0:
		_tile_holes += 1
	# A preview tile discards its bake's road footprint: it shows only beside its road (the
	# preview's display, or the held one until the commit's chunks install), outside an edit.
	var tool: Node = _bench.road_tool
	var road_drawn: bool = tool._preview_display.sequence > 0 or tool._swap_display != null
	if _kind in ["pose", "catch_up", "sweep"] and int(row["preview_shown"]) > 0 and not road_drawn:
		_unroaded += 1
		row["unroaded"] = true
	if _hole_pixels:
		row["magenta"] = _magenta_pixels()
		if int(row["magenta"]) > 0 and _kind != "settled":
			_pixel_holes += 1
	if int(counters["preview_frames"]) != int(_seen["preview_frames"]):
		row["preview_ms"] = float(counters["preview_install_ms_last"])
		row["preview_tiles"] = int(counters["preview_tiles_last"])
		row["preview_parts"] = counters["preview_parts"]
	if int(counters["edit_batches"]) != int(_seen["edit_batches"]):
		row["batch_ms"] = float(counters["edit_install_ms_last"])
		row["batch_tiles"] = int(counters["edit_tiles_last"])
		row["batch_adopted"] = int(counters["edit_adopted"]) - int(_seen["edit_adopted"])
		row["batch_parts"] = (counters["edit_frame_parts"] as Dictionary).duplicate()
	_frames.append(row)
	_last_us = now
	_remember()

# The harness's idle wait, then the graded tiles idle (no tile or list in flight, no count queued,
# no cell set to build) for three frames in a row.
func _settle(record: Dictionary) -> bool:
	var wait: Dictionary = await _bench._wait_for_idle(_bench.settle_timeout_sec)
	var graded = _bench.terrain.get("graded_tiles")
	var stable := 0
	var frames := 0
	while bool(wait.get("ok", false)) and graded != null and stable < 3 and frames < SETTLE_FRAME_LIMIT:
		await _bench.get_tree().process_frame
		frames += 1
		var idle: bool = (graded.requested.is_empty() and graded.patch_lists_requested.is_empty() and graded._counts.is_empty()
			and graded._builds.is_empty() and graded._builds_urgent.is_empty())
		stable = stable + 1 if idle else 0
	if stable < 3:
		record["error"] = "the terrain did not settle"
		record["wait"] = wait
	return stable >= 3

# Saves the frame now drawn as <case>_<attempt>_<label>.png when a capture directory is set.
func _capture(record: Dictionary, label: String) -> void:
	if _capture_dir.is_empty():
		return
	DirAccess.make_dir_recursive_absolute(_capture_dir)
	await RenderingServer.frame_post_draw
	var path := _capture_dir.path_join("%s_%02d_%s.png" % [record["case"], int(record["attempt"]), label])
	_expect(_bench.get_viewport().get_texture().get_image().save_png(path) == OK, record, "capture " + path)

func _expect(condition: bool, record: Dictionary, message: String) -> void:
	if condition:
		return
	var text := "%s %s: %s" % [record.get("case", ""), record.get("attempt", ""), message]
	_failures.append(text)
	push_error("[GRADED_DRAG] " + text)

# The install share of a recorded frame: show_preview's, or the edit batch's on its frame.
static func _share(row: Dictionary) -> float:
	return float(row.get("batch_ms", row.get("preview_ms", 0.0)))

func _stroke_rows(record: Dictionary) -> Array:
	var rows: Array = []
	for part in ["fast", "undo", "converged"]:
		if record.has(part) and record[part].has("frames"):
			rows.append_array(record[part].frames)
	return rows

func _stroke_line(record: Dictionary) -> Dictionary:
	var line := {"case": record.case, "attempt": record.attempt, "ok": record.ok}
	for kind in KINDS:
		var shares: Array = _stroke_rows(record).filter(func(row): return row.kind == kind).map(_share)
		line[kind + "_max_ms"] = shares.max() if not shares.is_empty() else 0.0
	if record.has("converged"):
		line["catch_up_frames"] = record.converged.catch_up_frames
	return line

# Per kind of frame: the count, the install share's p50, p95 and max, the frames over SHARE_MS,
# and the most tiles and bytes one frame uploaded.
func _summary() -> Dictionary:
	var rows: Array = []
	for record in _report.strokes:
		rows.append_array(_stroke_rows(record))
	var out := {"strokes": _report.strokes.size(), "failures": _failures.size()}
	for kind in KINDS:
		var of_kind: Array = rows.filter(func(row): return row.kind == kind and (row.has("preview_ms") or row.has("batch_ms")))
		var shares: Array = of_kind.map(_share)
		shares.sort()
		var bytes: Array = of_kind.map(func(row): return row.upload_bytes)
		out[kind] = {
			"frames": shares.size(),
			"p50_ms": shares[floori(shares.size() * 0.5)] if not shares.is_empty() else 0.0,
			"p95_ms": shares[mini(shares.size() - 1, int(shares.size() * 0.95))] if not shares.is_empty() else 0.0,
			"max_ms": shares.max() if not shares.is_empty() else 0.0,
			"over_share": shares.filter(func(ms): return ms > SHARE_MS).size(),
			"max_uploads": of_kind.map(func(row): return row.uploads).max() if not of_kind.is_empty() else 0,
			"max_bytes": bytes.max() if not bytes.is_empty() else 0,
		}
	var gpu: Array = rows.map(func(row): return row.gpu_ms)
	gpu.sort()
	out["gpu_ms"] = {"p50": gpu[floori(gpu.size() * 0.5)], "max": gpu.max()} if not gpu.is_empty() else {}
	return out

# Engine errors the probe fails on, counted in its own log (--log-file): a cell drawn with
# failed instance uniforms renders wrong and its timings are not the share (`TERRAIN-07-D` round 7).
const LOG_FAILURES := ["Too many instances using shader instance variables"]

func _log_failures() -> Dictionary:
	var out := {}
	var args := OS.get_cmdline_args()
	var at := args.find("--log-file")
	if at < 0 or at + 1 >= args.size():
		return {"log": "no --log-file"}
	var text := FileAccess.get_file_as_string(args[at + 1])
	for pattern: String in LOG_FAILURES:
		out[pattern] = text.count(pattern)
	return out

func _finish(error: String) -> void:
	_bench._stop_scripted_tool()
	if error.is_empty():
		var logged := _log_failures()
		_report["log_failures"] = logged
		for pattern: String in LOG_FAILURES:
			if int(logged.get(pattern, 0)) > 0:
				error = "the log shows %d x '%s'" % [int(logged[pattern]), pattern]
	if not error.is_empty():
		_report["error"] = error
		push_error("[GRADED_DRAG] " + error)
	_report.success = error.is_empty() and _failures.is_empty() and not _report.strokes.is_empty()
	_bench._metrics = _report
	var written: bool = _bench._write_metrics()
	_bench.get_tree().quit(0 if written and _report.success else 1)
