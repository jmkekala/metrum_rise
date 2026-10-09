# SPDX-License-Identifier: GPL-2.0-only

## Checks the cached growth bridge, exact project counts and HUD bar geometry.
extends SceneTree

const DemandMeter = preload("res://scripts/ui/demand_meter.gd")
var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	var simulation := SimulationNode.new()
	var overview: Dictionary = simulation.get_growth_overview()
	for key in ["residential", "commercial", "industrial"]:
		_expect(overview.has(key), "Growth bridge must return " + key)
		_expect(int(overview[key].projects) == 0, "An unassessed world must not invent projects")
		_expect(str(overview[key].detail).contains("Waiting"), "An unassessed world explains its missing assessment")
	simulation.free()

	var meter := DemandMeter.new()
	meter.position = Vector2(24, 24)
	meter.size = Vector2(110, 64)
	root.add_child(meter)
	meter.set_growth({
		"residential": {"projects": 6, "detail": "7 household places needed."},
		"commercial": {"projects": 3, "detail": "Insufficient operating income on two other sites."},
		"industrial": {"projects": 0, "detail": "No free zoned lots."},
	})
	await process_frame
	await process_frame
	_expect(meter._counts.residential.text == "R 6", "Housing count stays exact")
	_expect(meter._counts.commercial.text == "C 3", "Business count stays exact")
	_expect(not meter._bars.industrial.visible, "Zero projects have no bar")
	_expect(is_equal_approx(meter._bars.residential.size.y, meter._bars.commercial.size.y * 2.0), "Bars share a count scale")
	_expect(meter.tooltip_text.contains("No free zoned lots."), "Rust constraints reach the tooltip")
	_expect(not meter.tooltip_text.contains("%"), "Growth counts must not masquerade as pressure percentages")
	_expect(meter.get_combined_minimum_size().y <= 70.0, "Meter fits its compact HUD strip")
	if OS.get_environment("METRUM_GROWTH_METER_SCREENSHOT") != "":
		await RenderingServer.frame_post_draw
		root.get_texture().get_image().save_png(OS.get_environment("METRUM_GROWTH_METER_SCREENSHOT"))
	meter.free()
	print("Growth meter tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)
