# SPDX-License-Identifier: GPL-2.0-only

## Short-lived feedback for optional road-type pockets Rust refused on a built road (`ROAD-128`):
## one small label over each junction with the reasons, which fades out after `HOLD_S`. Uses the
## shared road reason translations.
extends Label

const UIStyle = preload("res://scripts/ui/ui_style.gd")
const RoadReasonText = preload("res://scripts/ui/road_reason_text.gd")
## Seconds the notice stays fully visible before it fades.
const HOLD_S := 4.0
const FADE_S := 0.6

var _world_pos := Vector3.ZERO

## One notice per junction in `refusals` (Rust's `pocket_refusals`), added to `layer`.
static func show_refusals(layer: CanvasLayer, refusals: Array) -> Array:
	var reasons_at := {}
	for refusal: Dictionary in refusals:
		var at: Vector3 = refusal.get("position", Vector3.ZERO)
		var reasons: PackedStringArray = reasons_at.get(at, PackedStringArray())
		var text := RoadReasonText.text(refusal)
		if not reasons.has(text):
			reasons.append(text)
		reasons_at[at] = reasons
	var notices := []
	for at: Vector3 in reasons_at:
		var notice := new()
		notice._world_pos = at + Vector3.UP * 3.0
		notice.text = "No turn pocket: " + "\n".join(reasons_at[at])
		layer.add_child(notice)
		notices.append(notice)
	return notices

func _ready() -> void:
	mouse_filter = Control.MOUSE_FILTER_IGNORE
	horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	UIStyle.set_font_size(self, 14)
	add_theme_color_override("font_color", Color(1.0, 0.76, 0.18, 0.98))
	add_theme_color_override("font_shadow_color", Color(0.0, 0.0, 0.0, 0.8))
	add_theme_constant_override("shadow_offset_x", 1)
	add_theme_constant_override("shadow_offset_y", 1)
	_follow()
	var tween := create_tween()
	tween.tween_interval(HOLD_S)
	tween.tween_property(self, "modulate:a", 0.0, FADE_S)
	tween.tween_callback(queue_free)

func _process(_delta: float) -> void:
	_follow()

# Centred over the junction on screen; hidden while it is behind the camera.
func _follow() -> void:
	var camera := get_viewport().get_camera_3d()
	visible = camera != null and not camera.is_position_behind(_world_pos)
	if visible:
		reset_size()
		position = (camera.unproject_position(_world_pos) - Vector2(size.x * 0.5, size.y)).round()
