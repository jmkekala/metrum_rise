# SPDX-License-Identifier: GPL-2.0-only

## Verifies cell gestures, independent Erase, cancellation, native selection and chunk reuse.
extends "res://tests/road_junction_preview_test.gd"

class CellSimulationStub extends Node3D:
	var hit := Vector3(5, 0, 20)
	var revision := 1
	var previews := 0
	var commits: Array = []
	var paths: Array = []
	func get_zone_profiles() -> Array:
		return [{"runtime_id": 1}, {"runtime_id": 2}]
	func move_network_node(_node: int, _position: Vector3) -> bool:
		return false
	func get_node_pos(_node: int) -> Vector3:
		return hit
	func intersect_world_surface(_origin: Vector3, _direction: Vector3) -> Vector3:
		return hit
	func get_zoning_cell_dependencies() -> PackedInt64Array:
		return PackedInt64Array([revision])
	func get_zoning_cells_preview_packed(shape: int, path: PackedVector2Array, _radius: float, profile: int) -> Dictionary:
		previews += 1
		paths.append([shape, path.duplicate(), profile])
		return {"valid": true, "cells": PackedInt64Array([1, 0, 0]), "dependencies": get_zoning_cell_dependencies(), "corners": PackedVector3Array([Vector3(0, 0, 10), Vector3(10, 0, 10), Vector3(10, 0, 20), Vector3(0, 0, 20)]), "colors": PackedColorArray([Color.GREEN])}
	func apply_zoning_cells_preview(cells: PackedInt64Array, dependencies: PackedInt64Array, profile: int) -> bool:
		commits.append([cells, dependencies, profile])
		revision += 1
		return true

func _run() -> void:
	_test_cell_controller()
	simulation = SimulationNode.new()
	root.add_child(simulation)
	await _test_cell_bridge()
	simulation.free()
	if _failures == 0:
		print("zoning_cells_tool_test: PASS")
	quit(0 if _failures == 0 else 1)

func _test_cell_controller() -> void:
	var scene := Node3D.new()
	root.add_child(scene)
	var sim := CellSimulationStub.new()
	sim.name = "SimulationNode"
	scene.add_child(sim)
	var overlay := Node3D.new()
	overlay.name = "ZoningOverlay"
	scene.add_child(overlay)
	var camera := Camera3D.new()
	scene.add_child(camera)
	camera.position = Vector3(0, 100, 100)
	camera.look_at(Vector3.ZERO)
	camera.current = true
	var tool := ZoningToolScript.new()
	scene.add_child(tool)
	tool.active = true
	tool._process(0.0)
	var calls := sim.previews
	tool._process(0.0)
	_expect(sim.previews == calls, "Idle cell hover reuses its preview")
	var press := InputEventMouseButton.new()
	press.button_index = MOUSE_BUTTON_LEFT
	for shape in range(4):
		tool.cell_shape = shape
		tool._process(0.0)
		press.pressed = true
		tool._unhandled_input(press)
		sim.hit.x += 100.0
		tool._cell_tool._input(InputEventMouseMotion.new())
		tool._process(0.0)
		press.pressed = false
		tool._cell_tool._input(press)
		_expect(sim.commits.size() == shape + 1, "Each selector commits one completed gesture")
		_expect(sim.paths[-1][0] == shape, "Rust receives the chosen selector")
		_expect(sim.paths[-1][1].size() == (1 if shape == 2 else 2), "Fill keeps its seed; other tools retain the swept path")
	tool.cell_erase = true
	tool._process(0.0)
	press.pressed = true
	tool._unhandled_input(press)
	press.pressed = false
	tool._cell_tool._input(press)
	_expect(sim.commits[-1][2] == 0 and tool.cell_shape == 3, "Erase reuses the selected brush")
	tool.select_profile(2)
	_expect(not tool.cell_erase and tool.current_profile_runtime_id == 2, "Selecting a profile leaves Erase")
	tool._process(0.0)
	var committed := sim.commits.size()
	press.pressed = true
	tool._unhandled_input(press)
	sim.revision += 1
	tool._process(0.0)
	press.pressed = false
	tool._cell_tool._input(press)
	_expect(sim.commits.size() == committed, "A changed dependency cancels a pending gesture")
	press.pressed = true
	tool._unhandled_input(press)
	tool.select_profile(1)
	press.pressed = false
	tool._cell_tool._input(press)
	_expect(sim.commits.size() == committed, "A settings change before the next frame cannot commit a different operation")
	press.pressed = true
	tool._unhandled_input(press)
	tool.set_cell_workflow(false)
	press.pressed = false
	tool._cell_tool._input(press)
	_expect(sim.commits.size() == committed, "Switching workflows cancels the pending gesture")
	tool.set_cell_workflow(true)
	var controls := preload("res://scripts/ui/zoning_cell_controls.gd").new()
	controls.zoning_tool = tool
	scene.add_child(controls)
	controls._shape.item_selected.emit(1)
	controls._erase.button_pressed = true
	_expect(tool.cell_shape == 1 and tool.cell_erase, "Toolbar combines Marquee with the separate Erase operation")
	controls._radius.value = 35.0
	_expect(tool.cell_brush_radius_m == 35.0, "Brush radius is expressed in world metres")
	var move := preload("res://scripts/tools/move_tool.gd").new()
	move.simulation_node = sim
	move.selected_node_id = 1
	move.selected_node_pos = Vector3.ONE
	move._apply_move(Vector3.ONE)
	_expect(move.selected_node_pos == sim.hit, "Rejected move restores the cursor without asking renderers to rebuild")
	move.free()
	scene.free()

func _cell_chunk(x: int, z: int, version := PackedInt64Array()) -> Dictionary:
	for attempt in range(100):
		var payload := simulation.try_get_zoning_cell_chunk_packed(x, z, version)
		if not payload.get("busy", true):
			return payload
		await create_timer(0.01).timeout
	_expect(false, "Cell chunk must become available")
	return {}

func _test_cell_bridge() -> void:
	_expect(simulation.create_blank_world(512.0, 512.0, 8.0, 128.0, 0.0), "Cell fixture loads")
	simulation.set_simulation_speed(0.0)
	var outside := await _cell_chunk(1, 0)
	_expect(not outside.get("busy", true) and outside.get("outside", false), "Outside chunks need no preparation")
	var cold := simulation.try_get_zoning_cell_chunk_packed(0, 0, PackedInt64Array())
	_expect(cold.get("busy", false) and not cold.has("vertices"), "Cold chunks defer generation and never publish partial geometry")
	var empty := await _cell_chunk(0, 0)
	_expect(empty.get("vertices", PackedVector3Array()).is_empty(), "Queued cell preparation completes while paused")
	if not await _commit(PackedVector3Array([Vector3(-96, 0, 0), Vector3(96, 0, 0)])):
		return
	var initial := await _cell_chunk(0, 0)
	var remote := await _cell_chunk(0, -1)
	var unchanged := await _cell_chunk(0, 0, initial.version)
	_expect(unchanged.get("unchanged", false) and not unchanged.has("vertices"), "Unchanged chunk uploads no geometry")
	var preview := simulation.get_zoning_cells_preview_packed(2, PackedVector2Array([Vector2(0, 20)]), 20.0, 1)
	_expect(preview.get("cell_count", 0) >= 100, "Fill covers six rows along the complete road side")
	_expect(simulation.apply_zoning_cells_preview(preview.cells, preview.dependencies, 1), "Native paint commits the preview")
	_expect(not simulation.apply_zoning_cells_preview(preview.cells, preview.dependencies, 2), "Consumed preview cannot repaint stale cells")
	var painted := await _cell_chunk(0, 0, initial.version)
	_expect(not painted.get("unchanged", true) and painted.colors != initial.colors, "Paint changes only the affected chunk payload")
	var endpoint := simulation.get_closest_node(Vector3(96, 0, 0), 1.0)
	_expect(endpoint >= 0, "Move fixture resolves the road endpoint")
	if endpoint >= 0:
		_expect(not simulation.move_network_node(endpoint, Vector3(96, 0, 40)), "Node movement cannot cross painted reservations")
		_expect(simulation.get_node_pos(endpoint) == Vector3(96, 0, 0), "Rejected movement retains the accepted position")
		var after_rejection := await _cell_chunk(0, 0, painted.version)
		_expect(after_rejection.get("unchanged", false), "Rejected movement does not invalidate the zoning mesh")
	var remote_after := await _cell_chunk(0, -1, remote.version)
	_expect(remote_after.get("unchanged", false), "Opposite road side retains its chunk mesh")
	_expect(simulation.apply_zoning_parcel_at(0.0, -16.0, 0, 2, 2), "Manual parcels remain usable beside painted cells")
	var suppressed := simulation.get_zoning_cells_preview_packed(0, PackedVector2Array([Vector2(0, -16)]), 20.0, 1)
	_expect(not suppressed.get("valid", true), "A manual reservation suppresses cached empty cells")
	_expect(simulation.undo_action(), "Completed cell gesture queues a local undo")
	var restored := false
	for attempt in range(100):
		var payload := await _cell_chunk(0, 0)
		if payload.get("colors", PackedColorArray()) == initial.colors:
			restored = true
			break
		await create_timer(0.01).timeout
	_expect(restored, "Undo restores the original cell paint after an unrelated manual placement")
	_expect(simulation.get_zoning_parcels_overlay().size() == 1, "Cell undo preserves the authored parcel")
	if endpoint >= 0:
		_expect(simulation.move_network_node(endpoint, Vector3(120, 0, 0)), "Clear road extension returns successful movement")
		_expect(simulation.get_node_pos(endpoint) == Vector3(120, 0, 0), "Accepted movement publishes the new position")
