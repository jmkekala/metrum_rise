# SPDX-License-Identifier: GPL-3.0-or-later

## The menu's pipeline prewarm (`RENDER-43` rounds R43b and R43c, docs/terrain.md "First-use
## pipelines at the load"): the templates of every layer that first draws after a load
## (`pipeline_prewarm.gd`) sit hidden in a world of their own, and this small view of it renders
## every frame, so Godot creates their surfaces and compiles their pipelines in the background
## while the menu shows, for whichever city comes next. Godot creates surfaces only while it
## renders a 3D scene, and the pipeline variants it compiles follow the 3D features rendered so
## far, so the view renders with the game's: the root viewport's scaling, anti-aliasing and MSAA,
## the default environment's screen-space effects and a shadowed sun. The view is never shown and
## its instances are hidden, so the menu looks as before.
##
## Paced (R43c): Godot's background compilations run a few at a time in one first-in first-out
## queue, and a draw that needs a queued pipeline waits for everything queued before it, so a
## player who left the menu with everything queued waited for all of it at the transition. The
## groups (`PipelinePrewarm.groups()`, in value order) are added one at a time: a group's
## templates are built and added, the view draws (their compilations are queued), then a marker
## task joins the same queue (`PipelinePrewarm.queue_marker`); when it has run, the queue has
## taken up everything before it, the group's instances are freed and the next group is added.
## A shader's first draw compiles its variants on the main thread (0.1-0.5 s each on a first run:
## a frozen menu frame, which a click waits for), so a group's templates go in slices, each
## ending at a shader the view has not drawn yet, and the marker follows each slice: leaving
## costs at most one slice's compilations, a click at most one slice's frame. The first group
## is added whole in its build frame, as before R43c.
## `group_finished` and `finished` say how far it is (a loading screen could wait on them). Freed
## with the menu; compilations already queued finish anyway and stay with their shaders and the
## caches.
##
## The water mirror draws at its own MSAA (`WaterReflection.MIRROR_MSAA`), and Godot precompiles a
## surface's pipelines for one sample count, the project setting's
## (`rendering/anti_aliasing/quality/msaa_3d`, read by the precompilation every frame and by
## nothing else after the launch: the root viewport took its MSAA at the start). So while a mirror
## group's templates get their surfaces the view holds that setting at the mirror's: its first
## draw queues their mirror sets, the setting is restored, the next draw queues their game sets
## (the earlier groups' instances are already freed, so nothing else is recompiled). Nothing
## drawn changes. `_exit_tree` restores the setting whatever the stage. If a later Godot stops
## reading the setting there, the mirror's first draws are merely cold again.
extends SubViewport

## Emitted when the queue has taken up group `index`'s compilations (`PipelinePrewarm.groups()`
## order).
signal group_finished(index: int, name: String)
## Emitted after the last group's `group_finished`.
signal finished

const PipelinePrewarm := preload("res://scripts/renderers/pipeline_prewarm.gd")
const WaterReflection := preload("res://scripts/renderers/water_reflection.gd")
const DEFAULT_ENVIRONMENT := preload("res://default_environment.tres")
# Smallest view that renders every pass; its size selects no pipeline.
const VIEW_SIZE := Vector2i(64, 64)
const MSAA_SETTING := "rendering/anti_aliasing/quality/msaa_3d"

# The group in flight: START builds the first in the view's first frame, ADD adds a slice, DRAW
# waits for the view's draw that queues the slice's compilations, MARKER for the queue to take
# them up (then the next slice, or the next group); DONE after the last group.
enum Stage { START, ADD, DRAW, MARKER, DONE }

var _prewarm := PipelinePrewarm.new()
var _groups: Array[Dictionary] = PipelinePrewarm.groups()
var _group := 0
var _stage := Stage.START
var _scenario := RID()
var _marker := -1
# The next group's pairs, built while the queue takes up the group in flight.
var _ahead: Array = []
var _ahead_built := false
var _ahead_ms := 0.0
# The pairs of the group in flight not yet in the world.
var _pending: Array = []
# The shaders the view has drawn templates of (or draws this frame).
var _seen := {}
# When this frame's slice was added (the slice frames' longest, `report`), 0 when none was.
var _slice_us := 0
# The project MSAA setting's value while a mirror group holds it at the mirror's, else null.
var _swung_from: Variant = null
# Per group `{name, pairs, build_ms, added_ms, slices, slice_ms, finished_ms}`: its build's
# main-thread time, when it was started (from the launch), its slice frames and the longest one
# from the slice's add to the end of its draw, and when its marker ran (`report`).
var _log: Array[Dictionary] = []

func _ready() -> void:
	size = VIEW_SIZE
	own_world_3d = true
	render_target_update_mode = SubViewport.UPDATE_ALWAYS
	var root := get_tree().root
	scaling_3d_mode = root.scaling_3d_mode
	scaling_3d_scale = root.scaling_3d_scale
	msaa_3d = root.msaa_3d
	use_taa = root.use_taa
	screen_space_aa = root.screen_space_aa
	var camera := Camera3D.new()
	add_child(camera)
	camera.make_current()
	var environment := WorldEnvironment.new()
	environment.environment = DEFAULT_ENVIRONMENT
	add_child(environment)
	var sun := DirectionalLight3D.new()
	sun.shadow_enabled = true
	add_child(sun)
	_scenario = find_world_3d().scenario
	RenderingServer.frame_post_draw.connect(_on_post_draw)

func _process(_delta: float) -> void:
	match _stage:
		Stage.START:
			_start_group()
		Stage.ADD:
			_add_slice()
		Stage.MARKER:
			if WorkerThreadPool.is_task_completed(_marker):
				WorkerThreadPool.wait_for_task_completion(_marker)
				_marker = -1
				if _pending.is_empty():
					_finish_group()
				else:
					_add_slice()
			elif _pending.is_empty() and not _ahead_built and _group + 1 < _groups.size():
				var build_us := Time.get_ticks_usec()
				_ahead = _build(_group + 1)
				_ahead_built = true
				_ahead_ms = float(Time.get_ticks_usec() - build_us) / 1000.0

func _exit_tree() -> void:
	if RenderingServer.frame_post_draw.is_connected(_on_post_draw):
		RenderingServer.frame_post_draw.disconnect(_on_post_draw)
	_restore_msaa()
	# A marker not yet run is left to the queue: waiting for it here would hold the transition
	# for the slice in flight, which the world scene's start can overlap instead.
	if _marker >= 0 and WorkerThreadPool.is_task_completed(_marker):
		WorkerThreadPool.wait_for_task_completion(_marker)
	_prewarm.end()

## Whether every group's compilations have been taken up by the queue.
func is_finished() -> bool:
	return _stage == Stage.DONE

## The groups started so far, each `{name, pairs, build_ms, added_ms, slices, slice_ms,
## finished_ms}` (`_log`; finished_ms -1 while in flight), and the last group's finish (-1 before
## it).
func report() -> Dictionary:
	var end: int = _log.back().finished_ms if is_finished() else -1
	return {"groups": _log.duplicate(true), "finished_ms": end}

func _start_group() -> void:
	var build_us := Time.get_ticks_usec()
	var pairs := _ahead if _ahead_built else _build(_group)
	var build_ms := _ahead_ms if _ahead_built else float(Time.get_ticks_usec() - build_us) / 1000.0
	_ahead = []
	_ahead_built = false
	if _groups[_group][PipelinePrewarm.GROUP_MIRROR]:
		_swung_from = ProjectSettings.get_setting(MSAA_SETTING)
		ProjectSettings.set_setting(MSAA_SETTING, WaterReflection.MIRROR_MSAA)
	_log.append({"name": _groups[_group][PipelinePrewarm.GROUP_NAME], "pairs": pairs.size(),
		"build_ms": build_ms, "added_ms": Time.get_ticks_msec(), "slices": 0, "slice_ms": 0.0,
		"finished_ms": -1})
	_pending = pairs
	_stage = Stage.ADD
	if _group == 0:
		# The first group whole, in its build frame (what a click on the menu's first frames found
		# queued before R43c).
		for pair: Array in pairs:
			_is_new(pair)
		_pending = []
		_prewarm.add(_scenario, pairs)
		_slice_us = Time.get_ticks_usec()
		_log.back().slices = 1
		_stage = Stage.DRAW
		return
	_add_slice()

# Adds the pending pairs up to and including the first whose shader the view has not drawn yet.
func _add_slice() -> void:
	var slice := []
	while not _pending.is_empty():
		var pair: Array = _pending.pop_front()
		slice.append(pair)
		if _is_new(pair):
			break
	_prewarm.add(_scenario, slice)
	_slice_us = Time.get_ticks_usec()
	_log.back().slices += 1
	_stage = Stage.DRAW

# Whether `pair` draws a shader the view has not drawn yet (marking it drawn).
func _is_new(pair: Array) -> bool:
	var materials: Array = [pair[1]] if pair[1] != null else []
	if pair[1] == null:
		for surface in (pair[0] as Mesh).get_surface_count():
			materials.append((pair[0] as Mesh).surface_get_material(surface))
	var new := false
	for material: Material in materials:
		var shader: Variant = material
		if material is ShaderMaterial:
			shader = (material as ShaderMaterial).shader
		if not _seen.has(shader):
			_seen[shader] = true
			new = true
	return new

func _build(index: int) -> Array:
	return (_groups[index][PipelinePrewarm.GROUP_PAIRS] as Callable).call()

# The view drew with the slice's instances: their surfaces exist and their compilations are
# queued; the marker follows them. A mirror group's last slice draws once more with the setting
# restored, which queues the group's game sets.
func _on_post_draw() -> void:
	if _slice_us > 0:
		var slice_ms := float(Time.get_ticks_usec() - _slice_us) / 1000.0
		_log.back().slice_ms = maxf(_log.back().slice_ms, slice_ms)
		_slice_us = 0
	if _stage != Stage.DRAW:
		return
	if _swung_from != null and _pending.is_empty():
		# The mirror sets are queued; the next draw queues the game's.
		_restore_msaa()
		return
	_marker = PipelinePrewarm.queue_marker()
	_stage = Stage.MARKER

func _restore_msaa() -> void:
	if _swung_from != null:
		ProjectSettings.set_setting(MSAA_SETTING, _swung_from)
		_swung_from = null

func _finish_group() -> void:
	_prewarm.release()
	_log.back().finished_ms = Time.get_ticks_msec()
	group_finished.emit(_group, _groups[_group][PipelinePrewarm.GROUP_NAME])
	_group += 1
	if _group < _groups.size():
		_start_group()
		return
	_stage = Stage.DONE
	RenderingServer.frame_post_draw.disconnect(_on_post_draw)
	finished.emit()
