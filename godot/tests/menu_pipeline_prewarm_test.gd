# SPDX-License-Identifier: GPL-3.0-or-later

## Rendered (gamescope headless) coverage of the menu's paced pipeline prewarm (`RENDER-43`
## round R43c, `menu_pipeline_prewarm.gd`): the groups finish in `PipelinePrewarm.groups()` order with one in
## flight, the project MSAA setting holds the mirror's only while a mirror group is in its first
## draw and is back at its launch value after the prewarm and after a menu left in the middle of
## a mirror group, the root viewport's MSAA never changes, and the templates of static meshes
## (the street lights, the signs) have their real meshes' vertex formats.
extends SceneTree

const MenuPipelinePrewarm := preload("res://scripts/core/menu_pipeline_prewarm.gd")
const PipelinePrewarm := preload("res://scripts/renderers/pipeline_prewarm.gd")
const WaterReflection := preload("res://scripts/renderers/water_reflection.gd")
const StreetLights := preload("res://scripts/renderers/street_lights.gd")
const RoadSigns := preload("res://scripts/renderers/road_signs.gd")
const PipelineInventory := preload("res://scripts/benchmarks/pipeline_inventory.gd")
const MSAA_SETTING := "rendering/anti_aliasing/quality/msaa_3d"
const MAX_FRAMES := 6000
var _failures := 0

func _initialize() -> void:
	# The root enters the tree after `_initialize`: the view needs its world.
	await process_frame
	var setting: Variant = ProjectSettings.get_setting(MSAA_SETTING)
	var root_msaa := root.msaa_3d
	_expect(setting != WaterReflection.MIRROR_MSAA, "the game's MSAA differs from the mirror's")
	await _test_order_and_swing(setting, root_msaa)
	await _test_exit_in_mirror(setting, root_msaa)
	_test_formats()
	quit(1 if _failures > 0 else 0)

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _test_order_and_swing(setting: Variant, root_msaa: Viewport.MSAA) -> void:
	var groups := PipelinePrewarm.groups()
	var view := MenuPipelinePrewarm.new()
	var finished: Array[int] = []
	view.group_finished.connect(func(index: int, name: String) -> void:
		_expect(name == groups[index][PipelinePrewarm.GROUP_NAME], "group %d is %s" % [index, name])
		finished.append(index))
	root.add_child(view)
	var swung := false
	for _frame in MAX_FRAMES:
		# Between the view's process and the frame's draw, where a mirror group holds the setting.
		await RenderingServer.frame_pre_draw
		var log: Array = view.report().groups
		if log.is_empty():
			continue
		var group: Dictionary = groups[log.size() - 1]
		# One group in flight: every added group but the last has finished, in order.
		_expect(finished.size() >= log.size() - 1, "one group in flight")
		_expect(view._prewarm.instance_count() <= int(log.back().pairs), "only the group in flight is in the world")
		var held: Variant = ProjectSettings.get_setting(MSAA_SETTING)
		if held != setting:
			_expect(group[PipelinePrewarm.GROUP_MIRROR] and held == WaterReflection.MIRROR_MSAA,
				"the setting is the mirror's only in a mirror group (%s)" % group[PipelinePrewarm.GROUP_NAME])
			swung = true
		_expect(root.msaa_3d == root_msaa, "the root viewport's MSAA is untouched")
		if view.is_finished():
			break
	_expect(view.is_finished(), "the prewarm finished within %d frames" % MAX_FRAMES)
	var order: Array[int] = []
	for index in groups.size():
		order.append(index)
	_expect(finished == order, "the groups finished in order: %s" % str(finished))
	_expect(swung, "a mirror group swung the setting")
	_expect(ProjectSettings.get_setting(MSAA_SETTING) == setting, "the setting is back after the prewarm")
	var report := view.report()
	_expect(int(report.finished_ms) >= int(report.groups.back().added_ms), "the report has the prewarm's end")
	root.remove_child(view)
	view.free()
	await process_frame

func _test_exit_in_mirror(setting: Variant, root_msaa: Viewport.MSAA) -> void:
	var view := MenuPipelinePrewarm.new()
	root.add_child(view)
	var left := false
	for _frame in MAX_FRAMES:
		await RenderingServer.frame_pre_draw
		if ProjectSettings.get_setting(MSAA_SETTING) != setting:
			# Leave in the middle of the mirror group, before the draw that would restore it.
			root.remove_child(view)
			left = true
			break
	_expect(left, "the mirror group was reached")
	_expect(ProjectSettings.get_setting(MSAA_SETTING) == setting, "the setting is back after leaving in the mirror group")
	_expect(root.msaa_3d == root_msaa, "the root viewport's MSAA is untouched after leaving")
	view.free()
	await process_frame

# The street lights' merged meshes and the signs' meshes are static: their templates must have
# their vertex formats, or the templates compile pipelines nothing draws.
func _test_formats() -> void:
	StreetLights._ensure_meshes()
	var lamp_template := _formats(StreetLights.pipeline_prewarm_surfaces()[0][0] as Mesh)
	for lods: Array in StreetLights._meshes:
		for mesh: Mesh in lods:
			_expect(_formats(mesh) == lamp_template, "a street light's format %s is its template's %s" % [
				str(_formats(mesh)), str(lamp_template)])
	RoadSigns._ensure_meshes()
	var sign_templates: Array[int] = []
	for pair: Array in RoadSigns.pipeline_prewarm_surfaces():
		sign_templates.append_array(_formats(pair[0] as Mesh))
	_expect(not RoadSigns._meshes.is_empty(), "the signs have meshes")
	for mesh: Mesh in RoadSigns._meshes:
		if mesh == null:
			continue
		for format: int in _formats(mesh):
			_expect(format in sign_templates, "a sign's format %d is a template's %s" % [format, str(sign_templates)])

# Each surface's vertex format as a pipeline sees it (`PipelineInventory.FORMAT_MASK`).
func _formats(mesh: Mesh) -> Array[int]:
	var out: Array[int] = []
	for surface in mesh.get_surface_count():
		out.append(int(RenderingServer.mesh_get_surface(mesh.get_rid(), surface).get("format", 0))
			& PipelineInventory.FORMAT_MASK)
	return out
