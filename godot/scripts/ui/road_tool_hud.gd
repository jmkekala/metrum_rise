# SPDX-License-Identifier: GPL-2.0-only

## The road tool's HUD (`ROAD-112` round D): one label anchored beside the stroke's end, the
## same pixel size at every zoom, worded from Rust's numbers only. The session's `hud` record
## gives the length, the angle to the road the stroke starts on or the bearing, the smallest arc
## radius with its design speed and the type's illegal radius, the build cost and the upkeep; the
## preview gives the class-priced cost, the steepest grade, the demolitions and the refusal, which
## `road_reason_text.gd` words. Plain defaults: the font, size and colours of the label it
## replaces (white, amber for a warning or a pending stroke, red for a refusal).
extends CanvasLayer

const RoadReasonText := preload("res://scripts/ui/road_reason_text.gd")
const COLOR_NORMAL := Color(1.0, 1.0, 1.0, 0.95)
const COLOR_WARNING := Color(1.0, 0.76, 0.18, 0.98)
const COLOR_REFUSED := Color(1.0, 0.28, 0.18, 0.98)

## The label the HUD shows.
var label: Label

func _init() -> void:
	layer = 10
	label = Label.new()
	label.add_theme_font_size_override("font_size", 18)
	label.add_theme_color_override("font_color", COLOR_NORMAL)
	label.add_theme_color_override("font_shadow_color", Color(0.0, 0.0, 0.0, 0.8))
	label.add_theme_constant_override("shadow_offset_x", 1)
	label.add_theme_constant_override("shadow_offset_y", 1)
	label.visible = false
	add_child(label)

## Puts the label above-right of the world point `anchor` as `camera` sees it this frame.
func place(camera: Camera3D, anchor: Vector3) -> void:
	if label.visible and camera != null:
		label.position = camera.unproject_position(anchor) + Vector2(8.0, -24.0)

## Writes the HUD for a stroke into `target`: its text and colour from the session's `hud`
## numbers and the stroke's `preview` (`is_pending` for a sent stroke), with the snap marker for
## `snap` (what took the cursor) and the zoning-grid toggle. A refused preview shows only its
## reason, as `RoadReasonText` words it.
static func write_label(target: Label, hud: Dictionary, preview: Dictionary, snap: String, zoning_snap: bool) -> void:
	var color := COLOR_NORMAL
	var marker := " [zoning grid]" if zoning_snap else (" [snapped]" if snap.begins_with("angle") or snap.begins_with("guide") else "")
	var length_m := float(hud.get("length_m", 0.0))
	var cost := float(preview.get("build_cost", hud.get("build_cost", 0.0)))
	var text := ""
	if bool(preview.get("is_pending", false)):
		color = COLOR_WARNING
		text = "Checking... %.1f m  cost %.0f%s" % [length_m, cost, marker]
	elif not bool(preview.get("is_valid", true)):
		color = COLOR_REFUSED
		text = RoadReasonText.text(preview)
	else:
		text = "%.1f m  %s  cost %.0f  upkeep %.1f/day%s" % [
			length_m, _angle_text(hud), cost, float(hud.get("upkeep_per_day", 0.0)), marker,
		]
		if preview.has("max_grade"):
			text += "\ngrade %.1f%%" % (float(preview["max_grade"]) * 100.0)
		var radius := float(hud.get("radius_m", -1.0))
		if radius >= 0.0:
			text += ("\n" if not preview.has("max_grade") else "  ") + "radius %.1f m  design %.0f km/h" % [
				radius, float(hud.get("design_speed_kmh", 0.0)),
			]
			if bool(hud.get("tight_radius", false)):
				color = COLOR_WARNING
				text += "  under %.1f m: too tight" % float(hud.get("illegal_radius_m", 0.0))
			elif bool(hud.get("design_speed_warning", false)):
				color = COLOR_WARNING
				text += "  below the speed limit"
		var demolish := int(preview.get("demolish_count", 0))
		if demolish > 0:
			text += "\ndemolishes %d building%s" % [demolish, "" if demolish == 1 else "s"]
		# A grade above the design tier is shown, not refused (ROAD-68 question 37).
		if String(preview.get("warning_reason", "")) == "road_steep":
			color = COLOR_WARNING
			text += "\nsteep %.0f%% (design %.0f%%)" % [
				float(preview.get("max_grade", 0.0)) * 100.0,
				float(preview.get("warning_limit", 0.0)) * 100.0,
			]
		var state := String(preview.get("plan_state", ""))
		if not state.is_empty() and state != "ready":
			color = COLOR_WARNING
			text += " [checking road]"
	target.add_theme_color_override("font_color", color)
	target.text = text
	target.visible = true

# The angle to the road the stroke starts on, folded to 0..90 degrees (a road and its reverse
# are one), else the stroke's compass bearing.
static func _angle_text(hud: Dictionary) -> String:
	var angle := float(hud.get("start_angle_deg", -1.0))
	if angle >= 0.0:
		return "%.0f° to road" % (180.0 - angle if angle > 90.0 else angle)
	return "bearing %.0f°" % float(hud.get("bearing_deg", 0.0))
