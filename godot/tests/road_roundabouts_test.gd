# SPDX-License-Identifier: GPL-2.0-only

## The roundabout tool against the native bridge (`ROAD-62`, round R7b, PLAN-R7 3.7). On empty
## ground a 36 m ring: its verdict and overlay, the click, undo. A 1 + 1 crossing converted at
## 36 m (arms trimmed, zebras on the arms, none on the ring), refused at 12 m and beside a road
## with red and the reason, the keys' 2 m steps clamped at the range, 45 and 50 m both reachable
## on two lanes. Bulldozing a ring piece removes the ring and leaves four road ends, named on
## hover, with one undo. A bare ring takes an ordinary crossroads: four compass strokes.
## Ported from Veeti's road-chain snapshot `efd58129` (`road_roundabouts_test.gd`), adapted to
## the verdict and outline preview.
extends SceneTree

const RoundaboutTool := preload("res://scripts/tools/roundabout_tool.gd")
const R36_M := 16.25
var _failures := 0
var simulation: SimulationNode
var _sizes: Array = []

class OverlayStub extends Node3D:
	func set_tool_active(_enabled: bool) -> void:
		pass

class RoadToolStub extends Node3D:
	var fwd_lanes := 1
	var bkw_lanes := 1
	func mark_network_topology_dirty() -> void:
		pass

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _ground(x: float, z: float) -> Vector3:
	return Vector3(x, simulation.get_world_surface_height(Vector2(x, z)), z)

# Waits for the render generation to pass `generation`; false after 20 s.
func _await_generation(generation: int) -> bool:
	var deadline := Time.get_ticks_msec() + 20000
	while Time.get_ticks_msec() < deadline:
		if simulation.get_network_render_generation() > generation:
			return true
		await process_frame
	return false

func _commit(a: Vector3, b: Vector3, fwd: int = 1, bkw: int = 1) -> bool:
	var generation := simulation.get_network_render_generation()
	simulation.add_road_with_snap(PackedVector3Array([a, b]), fwd, bkw, true)
	var done := await _await_generation(generation)
	_expect(done, "the road from %s must commit" % a)
	return done

func _undo(what: String) -> void:
	var generation := simulation.get_network_render_generation()
	_expect(simulation.undo_action(), "%s: undo queues" % what)
	_expect(await _await_generation(generation), "%s: undo publishes" % what)
	await process_frame

func _key(tool: Node, keycode: Key) -> void:
	var event := InputEventKey.new()
	event.pressed = true
	event.keycode = keycode
	tool._unhandled_input(event)

# The tool at (x, z), as if the pointer moved there and the frame ran.
func _hover(tool: Node, x: float, z: float) -> void:
	tool.set_scripted_pointer(true, _ground(x, z))
	tool._update_preview()

func _click(tool: Node, what: String) -> void:
	var generation := simulation.get_network_render_generation()
	var event := InputEventMouseButton.new()
	event.button_index = MOUSE_BUTTON_LEFT
	event.pressed = true
	tool._unhandled_input(event)
	_expect(await _await_generation(generation), "%s publishes" % what)
	await process_frame

# What the bulldoze tool's hover names at `point`. The hover's own query answers `busy` while
# the simulation thread holds the core, as it often does just after an edit and for long runs of
# frames under load; this one waits for the core (`CODE-20`).
func _target_at(point: Vector3) -> Dictionary:
	return simulation.get_bulldoze_target_at(point.x, point.z, true)

func _ring_edge_at(point: Vector3) -> int:
	return int(_target_at(point).get("id", -1))

func _run() -> void:
	var scene := Node3D.new()
	root.add_child(scene)
	simulation = SimulationNode.new()
	simulation.name = "SimulationNode"
	scene.add_child(simulation)
	_expect(simulation.create_blank_world(1024.0, 1024.0, 8.0, 128.0, 0.0), "fixture world must load")
	simulation.set_simulation_speed(0.0)
	# The crossing at the origin, 150 m arms.
	if not await _commit(_ground(-150.0, 0.0), _ground(150.0, 0.0)) \
			or not await _commit(_ground(0.0, -150.0), _ground(0.0, 150.0)):
		_finish(scene)
		return
	var terrain := Node3D.new()
	terrain.name = "Terrain"
	scene.add_child(terrain)
	var road := RoadToolStub.new()
	road.name = "RoadTool"
	scene.add_child(road)
	var tool: Node3D = RoundaboutTool.new()
	tool.name = "RoundaboutTool"
	scene.add_child(tool)
	tool.set_process(false)
	tool.size_changed.connect(func(d: float, lo: float, hi: float, lanes: int): _sizes = [d, lo, hi, lanes])
	tool.set_tool_active(true)

	await _ground_ring(tool)
	await _sizes_and_refusals(tool, road)
	await _conversion_and_bulldoze(tool)
	await _crossroads_on_a_bare_ring(tool)
	tool.set_tool_active(false)
	_finish(scene)

func _ground_ring(tool: Node) -> void:
	_hover(tool, 300.0, 300.0)
	_expect(bool(tool._preview.get("is_valid", false)), "a 36 m ring on empty ground: %s" % tool._preview)
	_expect(tool.blueprint_mesh.mesh != null, "the overlay shows the ring")
	_expect(tool.blueprint_mat.albedo_color == tool.blue_color, "blue when it would build")
	_expect(_sizes == [36.0, 30.0, 60.0, 1], "the row reads one lane, 36 of 30-60 m: %s" % [_sizes])
	_expect(tool._label.text.begins_with("Roundabout 36 m, 1 lane"), tool._label.text)
	var ring: PackedVector3Array = tool._preview.ring
	_expect(ring[0] == ring[ring.size() - 1], "the outline closes")
	await _click(tool, "the ring")
	var on_ring := _ground(300.0 + R36_M, 300.0)
	var target: Dictionary = _target_at(on_ring)
	_expect(target.get("label", "") == "Roundabout, 0 arms", "the hover names it: %s" % target)
	await _undo("the ring")
	target = _target_at(on_ring)
	_expect(target.get("kind", "") != "road", "undo takes the ring away: %s" % target)

func _sizes_and_refusals(tool: Node, road: Node) -> void:
	# The keys move 2 m and hold at the ends.
	_hover(tool, 300.0, 300.0)
	_key(tool, KEY_BRACKETRIGHT)
	_hover(tool, 300.0, 300.0)
	_expect(_sizes[0] == 38.0, "] adds 2 m: %s" % [_sizes])
	for i in 20:
		_key(tool, KEY_BRACKETRIGHT)
		tool._update_preview()
	_expect(_sizes[0] == 60.0, "] holds at the end: %s" % [_sizes])
	for i in 20:
		_key(tool, KEY_BRACKETLEFT)
		tool._update_preview()
	_expect(_sizes[0] == 30.0, "[ holds at the end: %s" % [_sizes])
	tool.set_diameter(36.0)
	# A two-lane type: 45 to 80 m. The keys and the 1 m row reach both 45 and 50.
	road.fwd_lanes = 2
	_hover(tool, 300.0, 300.0)
	_expect(_sizes == [50.0, 45.0, 80.0, 2], "two lanes read 50 of 45-80 m: %s" % [_sizes])
	tool.set_diameter(45.0)
	tool._update_preview()
	_expect(_sizes[0] == 45.0 and tool.is_valid, "45 m is reachable: %s" % [_sizes])
	_key(tool, KEY_BRACKETLEFT)
	tool._update_preview()
	_expect(_sizes[0] == 45.0, "[ holds at 45: %s" % [_sizes])
	tool.set_diameter(50.0)
	road.fwd_lanes = 1
	# Too close to the crossing's arm: red, with Rust's reason in words.
	_hover(tool, 24.0, 24.0)
	_expect(not tool.is_valid and tool._preview.get("invalid_reason", "") == "roundabout_overlaps_road", "beside a road: %s" % tool._preview)
	tool._update_blueprint_visuals()
	_expect(tool.blueprint_mat.albedo_color == tool.red_color, "red when refused")
	_expect(tool._label.text.contains("Another road occupies this site"), tool._label.text)
	_expect(tool.blueprint_mesh.mesh != null, "the refused outline still shows")
	# The API refuses what the tool's range never offers.
	var small: Dictionary = simulation.preview_roundabout(_ground(0.0, 0.0), 12.0, 1)
	_expect(small.get("invalid_reason", "") == "invalid_roundabout_dimensions", "12 m: %s" % small)
	_expect(not simulation.place_roundabout(_ground(0.0, 0.0), 12.0, 1), "12 m does not build")

func _conversion_and_bulldoze(tool: Node) -> void:
	_hover(tool, 3.0, 2.0)
	_expect(int(tool._site.node) >= 0, "within 8 m of the crossing: the site is the junction")
	_expect(_sizes == [36.0, 30.0, 60.0, 1], "a 1 + 1 crossing takes one lane: %s" % [_sizes])
	_expect(bool(tool._preview.get("is_valid", false)), "the crossing converts: %s" % tool._preview)
	var arms: Array = tool._preview.arms
	_expect(arms.size() == 4, "four arms: %s" % [arms])
	await _click(tool, "the conversion")
	_expect(simulation.get_closest_node(_ground(0.0, 0.0), 8.0) < 0, "the junction is gone")
	for arm: Dictionary in arms:
		var node: int = simulation.get_closest_node(arm.cut, 1.0)
		_expect(node >= 0, "a ring node at the cut")
		_expect(simulation.get_node_connection_count(node) == 3, "ring-in, ring-out and the arm")
		_expect(simulation.has_crosswalk(node, int(arm.edge)), "a zebra on the arm")
	# Between two arms, on the ring.
	var diagonal := R36_M * sqrt(0.5)
	var on_ring := _ground(diagonal, diagonal)
	var ring_edge := _ring_edge_at(on_ring)
	_expect(ring_edge >= 0, "the ring is under the pointer")
	for arm: Dictionary in arms:
		var node: int = simulation.get_closest_node(arm.cut, 1.0)
		_expect(not simulation.has_crosswalk(node, ring_edge), "no zebra on the ring")
	var target: Dictionary = _target_at(on_ring)
	_expect(target.get("label", "") == "Roundabout, 4 arms", "the hover: %s" % target)
	var generation := simulation.get_network_render_generation()
	# A click: it waits for the core instead of answering `busy`.
	var queued: Dictionary = simulation.bulldoze_at(on_ring.x, on_ring.z, true)
	_expect(bool(queued.get("queued", false)), "the bulldoze queues: %s" % queued)
	_expect(await _await_generation(generation), "the removal publishes")
	await process_frame
	for arm: Dictionary in arms:
		var end: int = simulation.get_closest_node(arm.cut, 1.0)
		_expect(end >= 0 and simulation.get_node_connection_count(end) == 1, "a road end at the cut")
	await _undo("the removal")
	for arm: Dictionary in arms:
		var ring_node: int = simulation.get_closest_node(arm.cut, 1.0)
		_expect(simulation.get_node_connection_count(ring_node) == 3, "undo brings the ring back")
	await _undo("the conversion")
	_expect(simulation.get_closest_node(_ground(0.0, 0.0), 8.0) >= 0, "undo brings the junction back")

func _crossroads_on_a_bare_ring(tool: Node) -> void:
	_hover(tool, -300.0, 300.0)
	await _click(tool, "the bare ring")
	var centre := Vector2(-300.0, 300.0)
	for direction: Vector2 in [Vector2.RIGHT, Vector2.DOWN, Vector2.LEFT, Vector2.UP]:
		var join := centre + direction * R36_M
		var far := centre + direction * (R36_M + 90.0)
		await _commit(_ground(far.x, far.y), _ground(join.x, join.y))
		var node: int = simulation.get_closest_node(_ground(join.x, join.y), 1.0)
		_expect(node >= 0 and simulation.get_node_connection_count(node) == 3, "the stroke joined at %s" % join)
	var on_ring := _ground(centre.x + R36_M * sqrt(0.5), centre.y + R36_M * sqrt(0.5))
	var target: Dictionary = _target_at(on_ring)
	_expect(target.get("label", "") == "Roundabout, 4 arms", "a crossroads on a bare ring: %s" % target)

func _finish(scene: Node) -> void:
	scene.free()
	if _failures == 0:
		print("road_roundabouts_test: PASS")
	quit(_failures)
