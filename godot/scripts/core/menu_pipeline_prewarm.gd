# SPDX-License-Identifier: GPL-2.0-only

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
## templates are built and added, the view draws once (their compilations are queued), then a
## marker task joins the same queue (`PipelinePrewarm.queue_marker`); when it has run, the queue
## has taken up everything before it, the group's instances are freed and the next group is
## added. Leaving costs at most the group in flight. `group_finished` and `finished` say how far
## it is (a loading screen could wait on them). Freed with the menu; compilations already queued
## finish anyway and stay with their shaders and the caches.
extends SubViewport

## Emitted when the queue has taken up group `index`'s compilations (`PipelinePrewarm.groups()`
## order).
signal group_finished(index: int, name: String)
## Emitted after the last group's `group_finished`.
signal finished

const PipelinePrewarm := preload("res://scripts/renderers/pipeline_prewarm.gd")
const DEFAULT_ENVIRONMENT := preload("res://default_environment.tres")
# Smallest view that renders every pass; its size selects no pipeline.
const VIEW_SIZE := Vector2i(64, 64)

# The group in flight: ADD builds and adds it, DRAW waits for the view's draw that queues its
# compilations, MARKER for the queue to take them up; DONE after the last group.
enum Stage { ADD, DRAW, MARKER, DONE }

var _prewarm := PipelinePrewarm.new()
var _groups: Array[Dictionary] = PipelinePrewarm.groups()
var _group := 0
var _stage := Stage.ADD
var _scenario := RID()
var _marker := -1
# The next group's pairs, built while the queue takes up the group in flight.
var _ahead: Array = []
var _ahead_built := false
# Per group `{name, pairs, build_ms, added_ms, finished_ms}`: the build's main-thread time and the
# times from the launch it was added and its marker ran (`report`).
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
		Stage.ADD:
			_add_group()
		Stage.MARKER:
			if WorkerThreadPool.is_task_completed(_marker):
				WorkerThreadPool.wait_for_task_completion(_marker)
				_marker = -1
				_finish_group()
			elif not _ahead_built and _group + 1 < _groups.size():
				_ahead = _build(_group + 1)
				_ahead_built = true

func _exit_tree() -> void:
	if RenderingServer.frame_post_draw.is_connected(_on_post_draw):
		RenderingServer.frame_post_draw.disconnect(_on_post_draw)
	# A marker not yet run is left to the queue: waiting for it here would hold the transition
	# for the group in flight, which the world's first frames would otherwise overlap.
	if _marker >= 0 and WorkerThreadPool.is_task_completed(_marker):
		WorkerThreadPool.wait_for_task_completion(_marker)
	_prewarm.end()

## Whether every group's compilations have been taken up by the queue.
func is_finished() -> bool:
	return _stage == Stage.DONE

## The groups added so far, each `{name, pairs, build_ms, added_ms, finished_ms}` (milliseconds
## from the launch; finished_ms -1 while in flight), and the last group's finish (-1 before it).
func report() -> Dictionary:
	return {"groups": _log.duplicate(true), "finished_ms": _log.back().finished_ms if is_finished() else -1}

func _add_group() -> void:
	var build_us := Time.get_ticks_usec()
	var pairs := _ahead if _ahead_built else _build(_group)
	_ahead = []
	_ahead_built = false
	_prewarm.add(_scenario, pairs)
	_log.append({"name": _groups[_group][PipelinePrewarm.GROUP_NAME], "pairs": pairs.size(),
		"build_ms": float(Time.get_ticks_usec() - build_us) / 1000.0, "added_ms": Time.get_ticks_msec(),
		"finished_ms": -1})
	_stage = Stage.DRAW

func _build(index: int) -> Array:
	return (_groups[index][PipelinePrewarm.GROUP_PAIRS] as Callable).call()

# The view drew with the group's instances: their surfaces exist and their compilations are queued.
func _on_post_draw() -> void:
	if _stage != Stage.DRAW:
		return
	_marker = PipelinePrewarm.queue_marker()
	_stage = Stage.MARKER

func _finish_group() -> void:
	_prewarm.release()
	_log.back().finished_ms = Time.get_ticks_msec()
	group_finished.emit(_group, _groups[_group][PipelinePrewarm.GROUP_NAME])
	_group += 1
	if _group < _groups.size():
		_add_group()
		return
	_stage = Stage.DONE
	RenderingServer.frame_post_draw.disconnect(_on_post_draw)
	finished.emit()
