# SPDX-License-Identifier: GPL-2.0-only

## Environmental overlay cadence, payload upload, invalidation and retry regressions.
extends SceneTree

const TerrainRenderer := preload("res://scripts/renderers/terrain.gd")
var _failures := 0

class ObservedRenderer extends TerrainRenderer:
	var uploads := 0
	var applications := 0
	var reject_next := false
	func _update_overlay_texture() -> bool:
		uploads += 1
		if reject_next:
			reject_next = false
			return false
		if overlay_texture == null:
			overlay_texture = ImageTexture.new()
		return true

	func _apply_overlay_mode() -> void:
		applications += 1

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	var renderer := ObservedRenderer.new()
	for mode in [1, 2, 3]:
		var before := renderer.uploads
		renderer.overlay_mode = mode
		renderer._refresh_overlay_texture(10)
		_expect(renderer.uploads == before + 1, "selecting environmental mode %d must upload" % mode)
		renderer._refresh_overlay_texture(10)
		_expect(renderer.uploads == before + 1, "unchanged day and mode must reuse the image")
		var applications_before := renderer.applications
		renderer._refresh_overlay_texture(11)
		_expect(renderer.uploads == before + 2, "daily settlement must refresh environmental mode %d" % mode)
		_expect(renderer.applications == applications_before, "daily pixel updates must reuse material bindings")
	for mode in [0, 4]:
		var before := renderer.uploads
		renderer.overlay_mode = mode
		renderer._refresh_overlay_texture(12)
		renderer._refresh_overlay_texture(13)
		_expect(renderer.uploads == before + 1, "non-environmental modes must ignore day changes")
	var before := renderer.uploads
	renderer.mark_overlay_dirty()
	renderer._refresh_overlay_texture(13)
	_expect(renderer.uploads == before + 1, "world-editor invalidation must refresh an unchanged mode")
	renderer.overlay_mode = 2
	renderer.reject_next = true
	var applications_before := renderer.applications
	renderer._refresh_overlay_texture(14)
	_expect(renderer.applications == applications_before, "failed uploads must not apply a stale image")
	renderer._refresh_overlay_texture(14)
	_expect(renderer.applications == applications_before + 1, "failed uploads must retry on the next refresh")
	before = renderer.uploads
	renderer._refresh_overlay_texture(14)
	_expect(renderer.uploads == before, "successful retry must restore caching")
	renderer.mark_overlay_dirty()
	renderer._refresh_overlay_texture(14)
	_expect(renderer.uploads == before + 1, "world replacement must refresh even at the same day and mode")
	renderer.free()
	var simulation := SimulationNode.new()
	root.add_child(simulation)
	var actual := TerrainRenderer.new()
	actual.simulation_node = simulation
	for world_size in [Vector2(80.0, 40.0), Vector2(40.0, 80.0), Vector2(85.0, 45.0)]:
		_expect(simulation.create_blank_world(world_size.x, world_size.y, 10.0, 40.0, 0.0), "payload fixture must create a blank world")
		for mode in [1, 2, 3]:
			actual.overlay_mode = mode
			_expect(actual._update_overlay_texture(), "native RGBA payload must upload for every environmental mode")
			var image: Image = actual.overlay_image
			_expect(image.get_width() == roundi(world_size.x / 10.0) + 1 and image.get_height() == roundi(world_size.y / 10.0) + 1, "world replacement must resize the uploaded image")
			var bytes := image.get_data()
			_expect(bytes.size() == image.get_width() * image.get_height() * 4 and bytes.count(0) == bytes.size(), "blank world must upload transparent RGBA pixels")
	actual.free()
	simulation.queue_free()
	await process_frame
	print("Environment overlay tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)
