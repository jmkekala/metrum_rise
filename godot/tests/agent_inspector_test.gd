# SPDX-License-Identifier: GPL-2.0-only

## A click on a moving car inspects it: a window beside the cursor shows its trip and can be
## dragged and resized without taking the keyboard, the route is drawn from the car to where it
## ends, and the camera follows its drawn position while zoom and orbit change. Panning with
## WASD, or a building link (which also opens the building's inspector), stops the follow but
## keeps the inspection until Follow. Esc or a click on empty ground ends the inspection with
## the camera left where it was. A Building Inspector window, clicked or not, leaves WASD to
## the camera too.
extends SceneTree

const InputManager := preload("res://scripts/core/input_manager.gd")
const InspectorScript := preload("res://scripts/renderers/agent_inspector.gd")
const BuildingInspector := preload("res://scripts/ui/building_inspector.gd")
const WORLD_M := 2048.0
const TIMEOUT_FRAMES := 600
const ROAD := [Vector3(-400, 0, 0), Vector3(400, 0, 0)]
const FOLLOW_S := 1.5
# Long enough for a speed command to reach the simulation thread.
const SETTLE_S := 0.3
# The window refreshes four times a second.
const PANEL_WAIT_S := 0.3
const UIStyle := preload("res://scripts/ui/ui_style.gd")

## Stands in for the building inspector, which a click that misses every agent reaches.
class BuildingInspectorStub extends Node:
	var clicks := 0
	var last := {}
	func try_inspect(world_pos: Vector3, screen_pos: Vector2, toggle := true) -> bool:
		clicks += 1
		last = {"world_pos": world_pos, "screen_pos": screen_pos, "toggle": toggle}
		return false

var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	root.size = Vector2i(1600, 900)
	var host := Node3D.new()
	host.name = "Main"
	root.add_child(host)
	var simulation := SimulationNode.new()
	simulation.name = "SimulationNode"
	host.add_child(simulation)
	_expect(simulation.create_blank_world(WORLD_M, WORLD_M, 8.0, 512.0, 0.0), "fixture world must be created")
	var camera := CameraNode.new()
	camera.name = "CameraNode"
	host.add_child(camera)
	camera.current = true
	var inspector := InspectorScript.new()
	inspector.name = "AgentInspector"
	host.add_child(inspector)
	var buildings := BuildingInspectorStub.new()
	host.add_child(buildings)
	# Attach the script after entering the tree to skip gameplay construction in _ready().
	var input := Node.new()
	host.add_child(input)
	input.set_script(InputManager)
	input.simulation_node = simulation
	input.building_inspector = buildings
	input.agent_inspector = inspector
	# What input_manager's _ready() does: Godot enables input callbacks only at ready.
	inspector.building_requested.connect(input._on_agent_building_requested)
	input.set_process(true)
	input.set_process_input(true)
	input.set_process_unhandled_input(true)

	await _add_road(simulation, ROAD)
	var render_id: int = simulation.spawn_debug_car(ROAD[0], ROAD[1])
	_expect(render_id >= 0, "the debug car must find the road")
	simulation.set_simulation_speed(4.0)
	camera.focus_on(ROAD[0], 40.0)
	var first_seq: int = MetrumDebug.read_since(0, 0)["last_seq"]

	# Click where the car is drawn, through the same handler a mouse click reaches. Paused, so
	# the car is where it is drawn. A spawned car is drawn before the simulation thread has
	# ticked it onto its lane, where a click finds it, so the test waits until it has moved
	# (`DEBUG-07`).
	var spawned := await _drawn_car(simulation, render_id)
	_expect(spawned != Vector3.INF, "the car must be drawn")
	_expect(await _moved_car(simulation, render_id, spawned), "the car must drive")
	simulation.set_simulation_speed(0.0)
	await create_timer(SETTLE_S).timeout
	var car := await _drawn_car(simulation, render_id)
	# Off the screen centre, so the window placement depends on where the click was.
	camera.focus_on(car + Vector3(30.0, 0.0, 15.0), 40.0)
	var click := camera.unproject_position(car)
	await _click(simulation, camera, input, click, func() -> bool: return inspector.render_id == render_id)
	_expect(inspector.render_id == render_id, "a click on the car must inspect it, got %d" % inspector.render_id)
	_expect(buildings.clicks == 0, "a click on a car must not reach the building inspector")
	# The follow starts at the first refresh the core answers, the next frame if it is busy.
	_expect(await _eventually(func() -> bool: return camera.is_following()), "inspecting must start the camera follow")

	await _frames(5)
	var details: Dictionary = await _inspection(simulation, render_id)
	_expect(details.get("active", false) and details["mode"] == "car", "the inspection must describe a car: %s" % details)
	_expect(details.get("state") in ["on a road", "in a junction"], "the car must be on the road: %s" % details.get("state"))
	_expect(details["destination"].is_empty() and details["home"].is_empty(), "the debug car has no buildings")
	_expect(int(details["path_length"]) >= 2 and int(details["lane"]) >= 0, "the car must have a path and a lane: %s" % details)
	var panel = inspector.panel()
	_expect(panel is Window and panel.visible and panel.title_text() == "Agent %d" % render_id, "the window must name the agent: %s" % panel.title_text())
	var rect := Rect2(Vector2(panel.position), Vector2(panel.size))
	_expect(not rect.has_point(click) and rect.grow(UIStyle.CURSOR_WINDOW_GAP + 1.0).has_point(click), "the window must open beside the click %s: %s" % [click, rect])
	_expect(not panel.has_focus(), "the window must not take the keyboard")
	_expect(panel.value_text("Mode") == "car" and panel.value_text("Destination") == "none", "the panel must show the fields")
	# The route rows fill at the first panel refresh after a route rebuild the core answered (`DEBUG-08`).
	_expect(await _eventually(func() -> bool: return panel.value_text("Route end").begins_with("path ends at node")), "the panel must say where the route ends: %s" % panel.value_text("Route end"))
	_expect(details.get("last_failure", null) == "" and panel.value_text("Last failure") == "none", "a car that has not failed must show no failure: %s" % details.get("last_failure"))
	var failure := "trip failed: agent %d [row 0], resident, car, at node 3: no road path from node 3 to border node 1; retry in 5 s (failure 1 of 3)" % render_id
	panel.show_agent(details.merged({"last_failure": failure}, true))
	_expect(panel.value_text("Last failure") == failure, "the panel must show the agent's last trip failure")

	var route: Dictionary = await _route_lines(simulation, render_id)
	var points: PackedVector3Array = route.get("points", PackedVector3Array())
	_expect(inspector.vertex_count == points.size() * 3 and points.size() >= 2, "the overlay must draw the route as a ribbon, %d vertices for %d points" % [inspector.vertex_count, points.size()])
	if points.size() >= 2:
		var at: Vector3 = details["position"]
		_expect(Vector2(points[0].x, points[0].z).distance_to(Vector2(at.x, at.z)) < 0.01, "the route must start at the car")
		# Before the dead-end marker (six points) the last lane ends at the far end of the road.
		var last_lane := points[points.size() - 7]
		_expect(Vector2(last_lane.x, last_lane.z).distance_to(Vector2(ROAD[1].x, ROAD[1].z)) < 15.0, "the route must reach the far end: %s" % last_lane)
	var remaining := float(route.get("remaining_m", 0.0))
	_expect(remaining > 700.0 and remaining < 800.0, "the remaining route must be the rest of the road: %.1f m" % remaining)

	# The player drags the window by its title bar and resizes it from its corner.
	var moved_from: Vector2i = panel.position
	await _drag(Vector2(panel.position) + Vector2(100.0, -10.0), Vector2(-60.0, 40.0))
	_expect(panel.position == moved_from + Vector2i(-60, 40), "the title bar must move the window: %s -> %s" % [moved_from, panel.position])
	var sized_from: Vector2i = panel.size
	await _drag(Vector2(panel.position + panel.size) - Vector2(4.0, 4.0), Vector2(50.0, 30.0))
	_expect(panel.size.x > sized_from.x + 20 and panel.size.y > sized_from.y + 10, "the corner must resize the window: %s -> %s" % [sized_from, panel.size])
	_expect(not panel.has_focus(), "using the window must not take the keyboard")

	# Links: the debug car has no buildings, so show a home for it, as the refresh would.
	await create_timer(PANEL_WAIT_S).timeout
	_expect(panel.link("Home").disabled and panel.link("Work").text == "none", "a missing building must not be a link")
	var home := Vector3(120.0, 0.0, 80.0)
	panel.show_agent(details.merged({"home": {"id": 7, "kind": "residential", "profile": "house", "position": home}}, true))
	_expect(panel.link("Home").text == "7 residential (house)" and not panel.link("Home").disabled, "a home must be a link")
	panel.link("Home").pressed.emit()
	_expect(not camera.is_following() and panel.follow_offered(), "a link must stop the follow and offer Follow")
	_expect(Vector2(camera.get_focus_position().x, camera.get_focus_position().z) == Vector2(home.x, home.z), "a link must take the camera to the building")
	_expect(buildings.last.get("world_pos") == home and buildings.last.get("toggle") == false, "a link must open the building's inspector without toggling it: %s" % buildings.last)
	var building_window: Vector2 = buildings.last.get("screen_pos", Vector2.ZERO)
	_expect(is_equal_approx(building_window.x, panel.position.x + panel.size.x), "the building window must go beside the agent window")
	await create_timer(SETTLE_S).timeout
	_expect(Vector2(camera.get_focus_position().x, camera.get_focus_position().z) == Vector2(home.x, home.z), "the camera must stay on the building")
	_expect(inspector.is_active(), "a link must keep the inspection")
	panel.find_child("FollowButton", true, false).pressed.emit()
	_expect(await _eventually(func() -> bool: return camera.is_following() and not panel.follow_offered()), "Follow must resume following")
	_expect(camera.get_focus_position().distance_to(details["position"]) < 0.5, "Follow must bring the camera back to the car")

	# Moving: the pivot is on the car's drawn position each frame; zoom and orbit still work. At
	# 2x the car cannot reach the end of its route before the test pauses it again.
	simulation.set_simulation_speed(2.0)
	var start: Vector3 = camera.get_focus_position()
	var distance := camera.global_position.distance_to(start)
	var compared := 0
	var on_target := 0
	var worst := 0.0
	var steps := [false, false]
	var began_ms := Time.get_ticks_msec()
	var last_ms := began_ms
	while Time.get_ticks_msec() - began_ms < FOLLOW_S * 1000.0:
		# The agent renderer smooths and records each car's drawn pose once a frame.
		var now_ms := Time.get_ticks_msec()
		simulation.smooth_car_render_data(simulation.get_car_render_data(), (now_ms - last_ms) / 1000.0)
		last_ms = now_ms
		await process_frame
		# Empty while the core is busy; the inspector skips such frames too.
		var now: Dictionary = simulation.get_agent_inspection(render_id)
		if now.has("drawn_position"):
			compared += 1
			var off := camera.get_focus_position().distance_to(now["drawn_position"])
			worst = maxf(worst, off)
			on_target += 1 if off < 0.01 else 0
		var elapsed_s := (Time.get_ticks_msec() - began_ms) / 1000.0
		if elapsed_s > 0.3 and not steps[0]:
			steps[0] = true
			# A real wheel notch over the world, with the window open.
			for notch in 2:
				await _wheel_up(_outside(panel))
			_expect(camera.global_position.distance_to(camera.get_focus_position()) < distance - 1.0, "the wheel must still zoom while following")
		if elapsed_s > 0.6 and not steps[1]:
			steps[1] = true
			var before := camera.global_position - camera.get_focus_position()
			camera.orbit(Vector2(120.0, 0.0))
			var after := camera.global_position - camera.get_focus_position()
			_expect(before.angle_to(after) > 0.1, "orbit must still work while following")
	_expect(compared >= 10, "the car must be drawn while followed, %d frames" % compared)
	_expect(on_target >= compared * 0.8 and worst < 5.0, "the pivot must sit on the drawn car: %d of %d frames, worst %.3f m" % [on_target, compared, worst])
	_expect(camera.get_focus_position().distance_to(start) > 5.0, "the camera must move with the car")

	# WASD takes the camera back: the follow stops, the inspection, window and route stay.
	var pan_from := camera.get_focus_position()
	await _hold_key(KEY_D, 0.2)
	_expect(not camera.is_following() and panel.follow_offered(), "panning must stop the follow and offer Follow")
	_expect(inspector.is_active() and panel.visible and inspector.vertex_count > 0, "panning must keep the inspection")
	var panned_to := camera.get_focus_position()
	_expect(panned_to.distance_to(pan_from) > 1.0, "WASD must pan the camera")
	await create_timer(SETTLE_S).timeout
	_expect(camera.get_focus_position() == panned_to, "the camera must stay where it was panned while the car drives on")
	panel.find_child("FollowButton", true, false).pressed.emit()
	_expect(await _eventually(func() -> bool: return camera.is_following() and not panel.follow_offered()), "Follow must resume after panning")
	var following_now: Dictionary = await _inspection(simulation, render_id)
	_expect(camera.get_focus_position().distance_to(following_now.get("drawn_position", following_now.get("position", Vector3.INF))) < 5.0, "Follow must bring the camera back to the car")

	# Esc reaches the game with the window open and ends it; the camera stays where it is while
	# the car drives on.
	var esc := InputEventKey.new()
	esc.keycode = KEY_ESCAPE
	esc.pressed = true
	root.push_input(esc)
	await process_frame
	_expect(not inspector.is_active() and not camera.is_following(), "Esc must end the inspection and the follow")
	_expect(inspector.vertex_count == 0 and not panel.visible, "ending must clear the route and hide the panel")
	_expect(not (await _inspection(simulation, render_id)).get("active", true), "Rust must drop the selection")
	var resting := camera.get_focus_position()
	await create_timer(SETTLE_S).timeout
	_expect(camera.get_focus_position() == resting, "the camera must stay put after the inspection")
	camera.pan(Vector3(1.0, 0.0, 0.0), 1.0, 0.5)
	_expect(camera.get_focus_position() != resting, "pan must work again")

	# A click on empty ground ends an inspection and still reaches the building inspector.
	simulation.set_simulation_speed(0.0)
	await create_timer(SETTLE_S).timeout
	car = await _drawn_car(simulation, render_id)
	camera.focus_on(car, 40.0)
	await _click(simulation, camera, input, camera.unproject_position(car), func() -> bool: return inspector.is_active())
	_expect(inspector.is_active(), "the car must be inspected again")
	var building_clicks := buildings.clicks
	await _click(simulation, camera, input, camera.unproject_position(car + Vector3(0.0, 0.0, 30.0)), func() -> bool: return buildings.clicks > building_clicks)
	_expect(not inspector.is_active() and buildings.clicks == 2 and buildings.last.get("toggle") == true, "a click beside the road must end the inspection and inspect buildings")

	# A real Building Inspector window: open or clicked, it leaves the camera keys to the game.
	var real_buildings := BuildingInspector.new()
	host.add_child(real_buildings)
	var real_window: Window = real_buildings._create_window_entry("fixture")["window"]
	real_window.position = Vector2i(200, 200)
	real_window.show()
	await process_frame
	_expect(not real_window.has_focus(), "an open building window must not take the keyboard")
	await _drag(Vector2(real_window.position) + Vector2(40.0, 40.0), Vector2.ZERO)
	await process_frame
	_expect(not real_window.has_focus(), "a clicked building window must hand the keyboard back")
	var before_wasd := camera.get_focus_position()
	await _hold_key(KEY_D, 0.2)
	_expect(camera.get_focus_position().distance_to(before_wasd) > 1.0, "WASD must move the camera with a building window open")

	var events := _event_lines(first_seq)
	_expect(events.any(func(line: String) -> bool: return line.begins_with("agent inspection: agent %d, car" % render_id)), "starting must log the agent on events")

	simulation.set_simulation_speed(0.0)
	host.queue_free()
	await process_frame
	print("Agent inspector tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)

# The car's drawn position from the render snapshot, or INF if it never appears.
func _drawn_car(simulation: Node, render_id: int) -> Vector3:
	for frame in TIMEOUT_FRAMES:
		for bucket in simulation.get_car_render_data().values():
			var at := (bucket["ids"] as PackedInt64Array).find(render_id)
			if at >= 0:
				var t: PackedFloat32Array = bucket["transforms"]
				return Vector3(t[at * 12 + 3], t[at * 12 + 7], t[at * 12 + 11])
		await process_frame
	return Vector3.INF

# Whether `condition` holds within the frame limit.
func _eventually(condition: Callable) -> bool:
	for frame in TIMEOUT_FRAMES:
		if condition.call():
			return true
		await process_frame
	return false

# The inspection once the core answers: while the simulation thread holds the core it returns
# only the drawn position, which the follow loop accepts but a full check cannot (`DEBUG-07`).
func _inspection(simulation: Node, render_id: int) -> Dictionary:
	for frame in TIMEOUT_FRAMES:
		var details: Dictionary = simulation.get_agent_inspection(render_id)
		if details.has("active"):
			return details
		await process_frame
	return {}

# The inspected agent's route lines once the core answers: while the simulation thread holds
# the core they come back empty (`DEBUG-08`).
func _route_lines(simulation: Node, render_id: int) -> Dictionary:
	for frame in TIMEOUT_FRAMES:
		var route: Dictionary = simulation.get_agent_route_lines(render_id)
		if route.has("points"):
			return route
		await process_frame
	return {}

# An inspect click at `screen`, repeated until `done` holds: a click while the simulation thread
# holds the core finds no ground and is dropped without effect, as in the game (`DEBUG-07`).
func _click(simulation: Node, camera: Camera3D, input: Node, screen: Vector2, done: Callable) -> void:
	for frame in TIMEOUT_FRAMES:
		if simulation.intersect_terrain(camera.project_ray_origin(screen), camera.project_ray_normal(screen)) != null:
			input._handle_inspect_click(screen)
			if done.call():
				return
		await process_frame

# Whether the car is drawn away from `from` within the frame limit: the simulation has ticked it.
func _moved_car(simulation: Node, render_id: int, from: Vector3) -> bool:
	for frame in TIMEOUT_FRAMES:
		var at := await _drawn_car(simulation, render_id)
		if at != Vector3.INF and at.distance_to(from) > 0.5:
			return true
		await process_frame
	return false

# Presses the left button at `from`, moves by `by` and releases, through the root viewport.
func _drag(from: Vector2, by: Vector2) -> void:
	var press := InputEventMouseButton.new()
	press.button_index = MOUSE_BUTTON_LEFT
	press.pressed = true
	press.position = from
	root.push_input(press)
	await process_frame
	var motion := InputEventMouseMotion.new()
	motion.position = from + by
	motion.relative = by
	motion.button_mask = MOUSE_BUTTON_MASK_LEFT
	root.push_input(motion)
	await process_frame
	var release := press.duplicate()
	release.pressed = false
	release.position = from + by
	root.push_input(release)
	await process_frame

# Holds `key` for `seconds` of real time through the Input singleton, as a keyboard would.
func _hold_key(key: Key, seconds: float) -> void:
	var press := InputEventKey.new()
	press.keycode = key
	press.pressed = true
	Input.parse_input_event(press)
	await create_timer(seconds).timeout
	var release := press.duplicate()
	release.pressed = false
	Input.parse_input_event(release)
	await process_frame

func _wheel_up(at: Vector2) -> void:
	var wheel := InputEventMouseButton.new()
	wheel.button_index = MOUSE_BUTTON_WHEEL_UP
	wheel.pressed = true
	wheel.position = at
	root.push_input(wheel)
	await process_frame

# A screen point on the world, clear of `window`.
func _outside(window: Window) -> Vector2:
	var left := Vector2(20.0, root.size.y - 20.0)
	return left if not Rect2(Vector2(window.position), Vector2(window.size)).has_point(left) else Vector2(root.size.x - 20.0, root.size.y - 20.0)

func _frames(count: int) -> void:
	for frame in count:
		await process_frame

func _add_road(simulation: Node, points: Array) -> void:
	var request: int = simulation.add_road_with_snap(PackedVector3Array(points), 1, 1, true)
	for frame in TIMEOUT_FRAMES:
		var result: Variant = simulation.get_road_commit_result(request)
		if result != null:
			_expect(result.get("committed", false), "fixture road must commit: %s" % result)
			return
		await process_frame
	_expect(false, "fixture road commit timed out")

func _event_lines(after_seq: int) -> Array:
	var read: Dictionary = MetrumDebug.read_since(after_seq, 4096)
	var lines := []
	for i in read["message"].size():
		if read["channel"][i] == "events":
			lines.append(read["message"][i])
	return lines
