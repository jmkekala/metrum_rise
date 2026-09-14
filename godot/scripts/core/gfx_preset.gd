# SPDX-License-Identifier: GPL-2.0-only

## Runtime graphics preset switch for low-end playtesting.
##
## Applies the scene-configuration levers measured by tests/local_gpu_probe.gd
## (E05/E06) to a live session, so a preset can be judged for look and feel
## rather than only for frame time. It writes runtime properties only: no
## asset, shader or authored constant is modified.
##
## METRUM_GFX picks the startup preset and enables the node; without it the node
## does nothing. F9 cycles presets, F10 cycles the render scale, F11 toggles
## water, F8 hides the overlay.
extends Node

# Matches the renderers' idiom. A bare SceneLighting class_name reference needs
# Godot's global class cache, which a freshly cloned checkout has not built yet.
const SceneLightingConfig := preload("res://scripts/core/scene_lighting.gd")

const PRESETS := ["authored", "medium", "low", "potato"]
const SCALES := [1.0, 0.77, 0.67, 0.5]

# Camera far is derived per frame as distance * 4 + margin, floored at min_far.
# Margin 0 with a min_far floor caps the near-ground view at the floor value and
# still opens up when zoomed far out.
const AUTHORED_MIN_FAR_M := 9000.0
const AUTHORED_FAR_MARGIN_M := 1000.0
const NEAR_CLIP_M := 0.5

var _preset := "authored"
var _scale := 1.0
var _water := true
var _sun: DirectionalLight3D
var _camera: Node
var _water_node: Node
var _label: Label
var _fps_accum := 0.0
var _fps_frames := 0
var _fps := 0.0
# InputManager re-applies its own clip policy after a world loads, so the far
# override has to be re-asserted rather than set once.
var _reassert_timer := 0.0

# Inert unless METRUM_GFX names a preset. An always-on autoload would fight the
# lever trials in tests/local_gpu_probe.gd, which drive the same properties.
func _ready() -> void:
	var requested := OS.get_environment("METRUM_GFX").strip_edges().to_lower()
	if not PRESETS.has(requested):
		set_process(false)
		set_process_unhandled_input(false)
		return
	_preset = requested
	_build_overlay()

func _process(delta: float) -> void:
	_fps_accum += delta
	_fps_frames += 1
	if _fps_accum >= 0.5:
		_fps = _fps_frames / _fps_accum
		_fps_accum = 0.0
		_fps_frames = 0
	if not _resolve_scene():
		_label.text = "gfx: waiting for world"
		return
	_reassert_timer -= delta
	if _reassert_timer <= 0.0:
		_reassert_timer = 1.0
		_apply()
	_label.text = "%s  |  scale %.2f  |  water %s  |  %.0f fps\nF9 preset  F10 scale  F11 water  F8 hide" % [
		_preset, _scale, "on" if _water else "off", _fps
	]

func _unhandled_input(event: InputEvent) -> void:
	if not (event is InputEventKey and event.pressed and not event.echo):
		return
	match event.keycode:
		KEY_F9:
			_preset = PRESETS[(PRESETS.find(_preset) + 1) % PRESETS.size()]
			_sync_from_preset()
			_apply()
		KEY_F10:
			_scale = SCALES[(SCALES.find(_scale) + 1) % SCALES.size()]
			_apply()
		KEY_F11:
			_water = not _water
			_apply()
		KEY_F8:
			_label.visible = not _label.visible
		_:
			return
	get_viewport().set_input_as_handled()

# Preset changes reset the two independently cycled toggles to the preset's own
# values; F10/F11 then move away from them without being pulled back.
func _sync_from_preset() -> void:
	var config := preset_config(_preset)
	_scale = config.get("scale", 1.0)
	_water = config.get("water", true)

## Levers per preset. Keys: shadows ("cheap"), far (metres), scale (FSR2 render
## scale, 1.0 disables upscaling), water (bool). Measured on a ThinkPad T490
## (UHD 620) at 1280x720, camera radius 300 m, with the mipmapped grass import.
func preset_config(preset: String) -> Dictionary:
	match preset:
		"medium":
			return {"shadows": "cheap", "far": 3000.0}
		"low":
			return {"shadows": "cheap", "far": 3000.0, "scale": 0.67}
		"potato":
			return {"shadows": "cheap", "far": 3000.0, "scale": 0.5, "water": false}
	return {}

func _apply() -> void:
	var config := preset_config(_preset)
	_apply_shadows(config.get("shadows", "") == "cheap")
	_water_node.visible = _water
	if is_equal_approx(_scale, 1.0):
		get_viewport().scaling_3d_mode = Viewport.SCALING_3D_MODE_BILINEAR
		get_viewport().scaling_3d_scale = 1.0
		# One source of truth for the mode: the project setting.
		get_viewport().screen_space_aa = ProjectSettings.get_setting(
			"rendering/anti_aliasing/quality/screen_space_aa")
	else:
		get_viewport().scaling_3d_mode = Viewport.SCALING_3D_MODE_FSR2
		get_viewport().scaling_3d_scale = _scale
		# FSR2 resolves its own edges from temporal samples. Blurring its output again with a
		# post-process filter costs a pass and softens what the upscaler just reconstructed.
		get_viewport().screen_space_aa = Viewport.SCREEN_SPACE_AA_DISABLED
	if _camera.has_method("set_clip_policy"):
		var far_m: float = config.get("far", AUTHORED_MIN_FAR_M)
		var margin_m := AUTHORED_FAR_MARGIN_M if not config.has("far") else 0.0
		_camera.set_clip_policy(NEAR_CLIP_M, far_m, margin_m)

func _apply_shadows(cheap: bool) -> void:
	if cheap:
		# Angular distance drives Godot's variable-penumbra path; zero makes the
		# directional shadow a plain PCF lookup. Two splits instead of four, and
		# no split blending, halve the cascade sampling again.
		_sun.light_angular_distance = 0.0
		_sun.shadow_blur = 0.0
		_sun.set("directional_shadow_mode", 1)
		_sun.set("directional_shadow_blend_splits", false)
		RenderingServer.directional_soft_shadow_filter_set_quality(
			RenderingServer.SHADOW_QUALITY_HARD
		)
	else:
		_sun.light_angular_distance = SceneLightingConfig.SUN_ANGULAR_DISTANCE_DEG
		_sun.shadow_blur = SceneLightingConfig.SHADOW_BLUR
		_sun.set("directional_shadow_mode", 2)
		_sun.set("directional_shadow_blend_splits", true)
		RenderingServer.directional_soft_shadow_filter_set_quality(
			RenderingServer.SHADOW_QUALITY_SOFT_MEDIUM
		)

# The Router swaps scenes, so cached nodes are re-resolved whenever they go away.
func _resolve_scene() -> bool:
	if (
		is_instance_valid(_sun)
		and is_instance_valid(_camera)
		and is_instance_valid(_water_node)
	):
		return true
	_sun = null
	_camera = null
	_water_node = null
	var scene := get_tree().current_scene
	if scene == null:
		return false
	var sun := scene.find_child("DirectionalLight3D", true, false)
	var camera := scene.find_child("CameraNode", true, false)
	var water := scene.find_child("Water", true, false)
	if sun == null or camera == null or water == null:
		return false
	_sun = sun
	_camera = camera
	_water_node = water
	_sync_from_preset()
	return true

func _build_overlay() -> void:
	var layer := CanvasLayer.new()
	layer.layer = 128
	add_child(layer)
	_label = Label.new()
	_label.position = Vector2(12, 12)
	_label.add_theme_color_override("font_color", Color.WHITE)
	_label.add_theme_color_override("font_outline_color", Color.BLACK)
	_label.add_theme_constant_override("outline_size", 6)
	layer.add_child(_label)
