# SPDX-License-Identifier: GPL-2.0-only

## The menu's pipeline prewarm (`RENDER-43` round R43b, docs/terrain.md "First-use pipelines at
## the load"): the templates of every layer that first draws after a load (`pipeline_prewarm.gd`)
## sit hidden in a world of their own, and this small view of it renders every frame, so Godot
## creates their surfaces and compiles their pipelines in the background while the menu shows,
## for whichever city comes next. Godot creates surfaces only while it renders a 3D scene, and the
## pipeline variants it compiles follow the 3D features rendered so far, so the view renders with
## the game's: the root viewport's scaling, anti-aliasing and MSAA, the default environment's
## screen-space effects and a shadowed sun. The view is never shown and its instances are hidden,
## so the menu looks as before. Freed with the menu; compilations already started finish anyway
## and stay with their shaders and the caches.
extends SubViewport

const PipelinePrewarm := preload("res://scripts/renderers/pipeline_prewarm.gd")
const DEFAULT_ENVIRONMENT := preload("res://default_environment.tres")
# Smallest view that renders every pass; its size selects no pipeline.
const VIEW_SIZE := Vector2i(64, 64)

var _prewarm := PipelinePrewarm.new()

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
	_prewarm.begin(find_world_3d().scenario, PipelinePrewarm.templates())

func _exit_tree() -> void:
	_prewarm.end()
