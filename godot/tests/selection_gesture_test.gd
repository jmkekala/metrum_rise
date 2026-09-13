# SPDX-License-Identifier: GPL-2.0-only

## Exercises selection and lane gestures through real viewport input, including consumed releases.
## The simulation stub records commands; native lane mutation has separate Rust coverage.
extends SceneTree

const SelectTool = preload("res://scripts/tools/select_tool.gd")
var _failures := 0

class SimulationStub extends Node3D:
	var hit: Variant = Vector3.ZERO
	var rays := 0
	var self_loop := false
	var commands: Array = []
	func intersect_world_surface(_origin: Vector3, _direction: Vector3) -> Variant:
		rays += 1
		return hit
	func get_closest_node(_point: Vector3, _radius: float) -> int:
		return 7 if hit.x < 40.0 else -1
	func get_hovered_edge(x: float, _z: float) -> int:
		return (0 if x < 60.0 else 1) if x >= 40.0 else -1
	func get_node_lanes(_id: int) -> Array:
		return [
			{"edge_id": 10, "lane_id": 0, "is_incoming": true, "pos": Vector3(-5, 0.6, 0)},
			{"edge_id": 10 if self_loop else 20, "lane_id": 0, "is_incoming": false, "pos": Vector3(5, 0.6, 0)},
		]
	func get_node_pos(_id: int) -> Vector3:
		return Vector3.ZERO
	func get_lane_connections_array(_id: int) -> Array:
		return []
	func has_crosswalk(_node: int, _edge: int) -> bool:
		return false
	func set_crosswalk_override(node: int, edge: int, enabled: bool) -> void:
		commands.append(["crosswalk", node, edge, enabled])
	func set_lane_connection(node: int, source: int, lane: int, target: int, target_lane: int) -> void:
		commands.append(["connect", node, source, lane, target, target_lane])
	func clear_lane_source(node: int, edge: int, lane: int) -> void:
		commands.append(["source", node, edge, lane])
	func clear_lane_connections(node: int) -> void:
		commands.append(["clear", node])
	func get_edge_nodes(edge: int) -> Vector2i:
		return Vector2i(edge, edge + 1)
	func get_edge_geometry_3d(edge: int) -> PackedVector3Array:
		return PackedVector3Array([Vector3(edge * 20, 0, 0), Vector3((edge + 1) * 20, 0, 0)])
	func get_edge_width(_edge: int) -> float:
		return 5.0

class UiStub extends Node:
	func hide_road_properties() -> void:
		pass
	func show_road_properties_multi(_edges: Array[int], _point: Vector2) -> void:
		pass

class ReleaseConsumer extends Node:
	var consumed := 0
	func _unhandled_input(event: InputEvent) -> void:
		if event is InputEventMouseButton and not event.pressed:
			consumed += 1
			get_viewport().set_input_as_handled()

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _button(pressed: bool, button: MouseButton = MOUSE_BUTTON_LEFT) -> void:
	var event := InputEventMouseButton.new()
	event.button_index = button
	event.pressed = pressed
	event.position = Vector2(80, 80)
	root.push_input(event, true)

func _run() -> void:
	var scene := Node3D.new()
	root.add_child(scene)
	var sim := SimulationStub.new()
	sim.name = "SimulationNode"
	scene.add_child(sim)
	var ui := UiStub.new()
	ui.name = "MainUI"
	scene.add_child(ui)
	var overlay := Node3D.new()
	overlay.name = "ZoningOverlay"
	scene.add_child(overlay)
	var camera := Camera3D.new()
	scene.add_child(camera)
	camera.position = Vector3(0, 30, 20)
	camera.look_at(Vector3.ZERO)
	camera.current = true
	var tool_script: Script = SelectTool
	for argument in OS.get_cmdline_user_args():
		if argument.begins_with("--tool-path="):
			tool_script = load(argument.trim_prefix("--tool-path="))
	var tool = tool_script.new()
	scene.add_child(tool)
	tool.set_process(false)
	tool.active = true
	_button(true)
	_button(false)
	_expect(tool.selected_node == 7 and tool.lane_spheres.size() == 2, "Viewport click selects the junction and builds its controls")
	if "--benchmark-selection-drag" in OS.get_cmdline_user_args():
		_benchmark(tool, sim)
		tool.active = false
		scene.free()
		quit(_failures)
		return
	sim.hit = Vector3(-5, 0.6, 0)
	_button(true)
	sim.hit = Vector3(5, 0.6, 0)
	_button(false)
	_expect(sim.commands == [["connect", 7, 10, 0, 20, 0]], "Ordinary lane release commits exactly once")
	sim.commands.clear()
	sim.hit = Vector3(-5, 0.6, 0)
	_button(true)
	_button(false)
	_expect(sim.commands == [["source", 7, 10, 0]], "Release on the original incoming handle clears that source")
	sim.commands.clear()
	sim.hit = tool.crosswalk_toggles[0].pos
	_button(true)
	_button(false)
	_expect(sim.commands == [["crosswalk", 7, 10, true]], "Crosswalk controls retain precedence and toggle once")
	sim.commands.clear()
	# A self-loop can expose the same signed lane at two different road mouths.
	sim.self_loop = true
	tool._build_node_visuals(7)
	sim.hit = Vector3(-5, 0.6, 0)
	_button(true)
	sim.hit = Vector3(5, 0.6, 0)
	_button(false)
	_expect(sim.commands == [["connect", 7, 10, 0, 10, 0]], "An outgoing self-loop handle cannot be mistaken for its incoming source")
	sim.commands.clear()
	sim.self_loop = false
	tool._build_node_visuals(7)
	sim.hit = Vector3(-5, 0.6, 0)
	_button(true)
	tool.active = false
	tool.active = true
	sim.hit = Vector3(5, 0.6, 0)
	tool._process(0.016)
	_expect(tool.drag_line_mesh.mesh == null, "Reactivating a tool cannot resume its cancelled lane ribbon")
	_button(false)
	_expect(sim.commands.is_empty(), "Release after switching tools cannot mutate a lane")
	sim.hit = Vector3.ZERO
	_button(true)
	_button(false)
	sim.hit = Vector3(-5, 0.6, 0)
	_button(true)
	_button(true, MOUSE_BUTTON_RIGHT)
	sim.hit = Vector3(5, 0.6, 0)
	_button(false)
	_expect(sim.commands == [["clear", 7]], "Right-click clears connections and cancels the pending lane edit")
	sim.commands.clear()
	# A later input handler consumes release after the tool received the press.
	sim.hit = Vector3(-5, 0.6, 0)
	_button(true)
	var blocker := ReleaseConsumer.new()
	root.add_child(blocker)
	sim.hit = Vector3(5, 0.6, 0)
	_button(false)
	await process_frame
	_expect(blocker.consumed == 1, "The fixture must actually consume the release before the selection tool")
	blocker.free()
	tool._process(0.016)
	_expect(sim.commands.is_empty() and tool.drag_line_mesh.mesh == null, "Consumed release cancels without editing a road")
	sim.commands.clear()
	# Empty-space release also ends a gesture without a mutation.
	sim.hit = Vector3(-5, 0.6, 0)
	_button(true)
	sim.hit = null
	_button(false)
	_expect(sim.commands.is_empty() and tool.drag_line_mesh.mesh == null, "A missing world hit cancels the lane edit")
	# Deferred cancellation of the last release must not cancel a newer press.
	sim.hit = Vector3(-5, 0.6, 0)
	_button(true)
	await process_frame
	sim.hit = Vector3(5, 0.6, 0)
	tool._process(0.016)
	_expect(tool.drag_line_mesh.mesh != null, "A new press survives deferred cancellation from the previous release")
	_button(false)
	_expect(sim.commands == [["connect", 7, 10, 0, 20, 0]], "The newer gesture commits its own source and target")
	tool.active = false
	tool.active = true
	sim.hit = Vector3(50, 0, 0)
	_button(true)
	_expect(tool.selected_edges == [0], "An edge press starts a fresh selection")
	tool.active = false
	tool.active = true
	sim.hit = Vector3(70, 0, 0)
	tool._process(0.016)
	_expect(tool.selected_edges.is_empty(), "Reactivation cannot grow an old edge drag")
	_button(false)
	sim.hit = Vector3(50, 0, 0)
	_button(true)
	sim.hit = Vector3(70, 0, 0)
	tool._process(0.016)
	_expect(tool.selected_edges == [0, 1], "A fresh held gesture grows along connected edges")
	blocker = ReleaseConsumer.new()
	root.add_child(blocker)
	_button(false)
	await process_frame
	blocker.free()
	var replacement: Array[int] = [0]
	tool._set_selection(replacement)
	tool._process(0.016)
	_expect(tool.selected_edges == [0], "Consumed edge release cannot resume selection growth")
	tool.active = false
	scene.free()
	print("selection_gesture_test: %s" % ("PASS" if _failures == 0 else "FAIL"))
	quit(_failures)

func _benchmark(tool: Node3D, sim: SimulationStub) -> void:
	sim.hit = Vector3(-5, 0.6, 0)
	_button(true)
	sim.hit = Vector3(5, 0.6, 0)
	for warmup in 10:
		tool._process(0.016)
	var samples: Array[float] = []
	var query_counts: Array[int] = []
	for sample in 9:
		var before := sim.rays
		var start := Time.get_ticks_usec()
		for iteration in 256:
			tool._process(0.016)
		samples.append(float(Time.get_ticks_usec() - start) / 256.0)
		query_counts.append(sim.rays - before)
		var vertices: PackedVector3Array = tool.drag_line_mesh.mesh.surface_get_arrays(0)[Mesh.ARRAY_VERTEX]
		_expect(vertices.size() == 22, "Drag benchmark retains all ribbon vertices")
		_expect(((vertices[0] + vertices[1]) * 0.5).is_equal_approx(Vector3(-5, 0.6, 0)), "Ribbon starts at its incoming lane")
		_expect(((vertices[20] + vertices[21]) * 0.5).is_equal_approx(Vector3(5, 0.6, 0)), "Ribbon snaps to its outgoing lane")
	_button(false)
	_expect(sim.commands == [["connect", 7, 10, 0, 20, 0]], "Benchmark gesture commits the same connection")
	samples.sort()
	print("SELECTION_DRAG_BENCH " + JSON.stringify({"median_us": samples[4], "samples_us": samples, "queries_per_sample": query_counts, "updates_per_sample": 256}))
