# SPDX-License-Identifier: GPL-2.0-only

## One-shot input while the simulation thread holds the core (`CODE-25`): the bulldoze tool's
## hover keeps its target while its own query answers busy, and its click waits for the core and
## deletes the road under the pointer. `debug_hold_core` makes the simulation thread hold the
## core, as a long frame would, until a call waits for it, so the hover is checked while the core
## is held and the click's wait is what ends the hold. The road tool's one-shot stroke checks
## answer with the core held too (`ROAD-102`), and the roundabout tool's hover answers without
## the core (`ZONE-10`).
extends SceneTree

const BulldozeTool := preload("res://scripts/tools/bulldoze_tool.gd")
var _failures := 0
var simulation: SimulationNode

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

func _run() -> void:
	var scene := Node3D.new()
	root.add_child(scene)
	simulation = SimulationNode.new()
	simulation.name = "SimulationNode"
	scene.add_child(simulation)
	_expect(simulation.create_blank_world(512.0, 512.0, 8.0, 128.0, 0.0), "fixture world must load")
	simulation.set_simulation_speed(0.0)
	# Straight down on the world: every screen point lands on the ground.
	var camera := Camera3D.new()
	camera.projection = Camera3D.PROJECTION_ORTHOGONAL
	camera.size = 200.0
	camera.position = Vector3(0.0, 300.0, 0.0)
	camera.rotation_degrees = Vector3(-90.0, 0.0, 0.0)
	scene.add_child(camera)
	camera.current = true
	# The road goes under wherever the tool reads the pointer.
	var mouse := root.get_mouse_position()
	var under = simulation.intersect_world_surface(camera.project_ray_origin(mouse), camera.project_ray_normal(mouse), true)
	_expect(under != null, "the pointer is over the ground")
	if under == null:
		_finish(scene)
		return
	var point: Vector3 = under
	var generation := simulation.get_network_render_generation()
	simulation.add_road_with_snap(PackedVector3Array([_ground(point.x - 60.0, point.z), _ground(point.x + 60.0, point.z)]), 1, 1, true)
	_expect(await _await_generation(generation), "the road commits")

	var tool: Node3D = BulldozeTool.new()
	tool.name = "BulldozeTool"
	scene.add_child(tool)
	tool.set_process(false)
	tool.active = true
	tool._update_hover()
	_expect(tool.hover_mesh.visible, "the hover finds the road")

	_expect(simulation.debug_hold_core(20000), "the simulation thread holds the core")
	tool._update_hover()
	_expect(tool.hover_mesh.visible and tool.hover_mesh.mesh != null, "the hover keeps its target while the core is held")
	_expect(bool(simulation.get_bulldoze_target_at(point.x, point.z).get("busy", false)), "the core was held through the hover")

	generation = simulation.get_network_render_generation()
	var click := InputEventMouseButton.new()
	click.button_index = MOUSE_BUTTON_LEFT
	click.pressed = true
	click.position = mouse
	root.push_input(click, true)
	_expect(await _await_generation(generation), "the click waits for the core and deletes the road")
	var after: Dictionary = simulation.get_bulldoze_target_at(point.x, point.z, true)
	_expect(after.get("kind", "") != "road", "no road is left under the pointer: %s" % after)
	_expect(not tool.hover_mesh.visible, "the click clears the hover")

	# The one-shot stroke checks wait for the core for their corridor clearance (`ROAD-102`):
	# with it held they still answer, never "pending" without a reason.
	var stroke := PackedVector3Array([_ground(point.x - 60.0, point.z + 40.0), _ground(point.x + 60.0, point.z + 40.0)])
	for check in ["validate_road_candidate_with_snap", "validate_road_candidate_for_commit_with_snap"]:
		_expect(simulation.debug_hold_core(20000), "the simulation thread holds the core for %s" % check)
		var answer: Dictionary = simulation.call(check, stroke, 1, 1, true)
		_expect(bool(answer.get("is_valid", false)) and not answer.has("is_pending"), "%s waits for the core: %s" % [check, answer])

	# The roundabout tool's hover reads the road tool's context (`ZONE-10`, round RZ10b): with the
	# core held it answers the ring at once, and the core is still held after it.
	var centre := _ground(point.x, point.z - 60.0)
	_expect(simulation.debug_hold_core(20000), "the simulation thread holds the core for the ring hover")
	var site: Dictionary = simulation.get_roundabout_site(centre, 1)
	var ring: Dictionary = simulation.preview_roundabout(centre, float(site.get("default_diameter_m", 36.0)), 1)
	_expect(int(site.get("node", 0)) == -1 and int(site.get("lanes", 0)) == 1, "the ring site on empty ground: %s" % site)
	_expect(bool(ring.get("is_valid", false)) and ring.get("ring", PackedVector3Array()).size() > 8, "the ring hover answers: %s" % ring)
	_expect(int(ring.get("demolish_count", -1)) == 0, "a ring on empty ground demolishes nothing")
	_expect(bool(simulation.get_bulldoze_target_at(centre.x, centre.z).get("busy", false)), "the ring hover left the core held")
	# A waiting call ends the hold.
	simulation.get_bulldoze_target_at(centre.x, centre.z, true)
	tool.active = false
	_finish(scene)

func _finish(scene: Node) -> void:
	scene.free()
	if _failures == 0:
		print("core_busy_input_test: PASS")
	quit(_failures)
