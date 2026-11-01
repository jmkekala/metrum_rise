# SPDX-License-Identifier: GPL-2.0-only

## Road tunnels through the player's road tool (`ROAD-61`): on a sculpted hill the tool's tunnel
## toggle previews and commits a straight stroke between two ends outside the hill as a tunnel
## between two approaches, the T key flips the toggle and its checkbox, a stroke that never gets
## the bore's cover is refused with its reason, the surface over the bore stays the hill, and a
## debug car drives in at one portal and out at the other, reported inside the bore.
## METRUM_ROAD61_SCREENSHOT, when set on a rendered run, saves the west portal
## (METRUM_ROAD61_VIEW picks another view, METRUM_ROAD61_INSIDE=1 lets the camera into the bore,
## METRUM_ROAD61_DRIVE_S how long the car drives first). METRUM_ROAD61_SKY_TEST=1 on a rendered
## run keys the sky to one colour and counts it from the middle of the bore looking east: no sky
## pixel may show outside the far mouth's opening (`ROAD-61` round R8, the closed lining).
extends SceneTree

const TIMEOUT_MS := 60000
const HILL_TOP_M := 25.0
const WEST := Vector2(-150.0, 0.0)
const EAST := Vector2(150.0, 0.0)

var _failures := 0
var _main: Node
var _simulation: Node
# The hill's lowest height over the middle 100 m of the bore, read before the road exists.
var _bore_cover_ground := 0.0
# The east portal row's x (the last 4 m knot with the bore's 5 m of cover) and the deck height.
var _east_portal_x := 0.0
var _deck_y := 0.0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	_main = load("res://scenes/Main.tscn").instantiate()
	# Own scene startup without the menu launch flow.
	_main.set_script(null)
	root.add_child(_main)
	current_scene = _main
	await process_frame
	await process_frame
	_simulation = _main.get_node("SimulationNode")
	var input_manager: Node = _main.get_node("InputManager")
	_expect(_simulation.create_blank_world(640.0, 640.0, 4.0, 128.0, 0.0), "the blank world must load")
	for step in 80:
		if _simulation.get_world_surface_height(Vector2.ZERO) > HILL_TOP_M:
			break
		_simulation.sculpt_terrain(Vector2.ZERO, 100.0, 1.0)
	_expect(_simulation.get_world_surface_height(Vector2.ZERO) > HILL_TOP_M, "the sculpt tool must raise a hill")
	input_manager._refresh_after_world_load()
	input_manager.set_simulation_speed(0.0)
	# Keys reach the tools once the terrain near the camera is ready.
	var ready_deadline := Time.get_ticks_msec() + TIMEOUT_MS
	while not input_manager._world_ready and Time.get_ticks_msec() < ready_deadline:
		await process_frame
	_expect(input_manager._world_ready, "the world must become ready for input")
	var tool: Node = _main.get_node("RoadTool")
	var ui: Node = _main.get_node_or_null("MainUI")
	tool.active = true
	tool.fwd_lanes = 1
	tool.bkw_lanes = 1
	input_manager.current_tool = input_manager.Tool.ROAD
	_press_t(input_manager)
	_expect(tool.tunnel_enabled, "T must turn the tunnel toggle on")
	if ui != null and ui.tunnel_check != null:
		_expect(ui.tunnel_check.button_pressed, "the checkbox must follow the T key")

	# The stroke that stays above 5 m of cover nowhere: a refusal with its reason, no road.
	var shallow := PackedVector3Array([_ground(Vector2(-150.0, 120.0)), _ground(Vector2(150.0, 120.0))])
	var refused := _preview(tool, shallow)
	_expect(not refused.get("is_valid", true) and refused.get("invalid_reason", "") == "tunnel_cover", "a stroke past the hill must be refused for its cover, got %s" % refused.get("invalid_reason", "missing"))
	tool.cancel_road()

	_bore_cover_ground = minf(_ground(Vector2(-50.0, 0.0)).y, _ground(Vector2(50.0, 0.0)).y)
	var points := PackedVector3Array([_ground(WEST), _ground(EAST)])
	_deck_y = points[0].y
	# The stroke's knots lie 4 m apart from its west end; the bore ends at the last covered one.
	for knot in 76:
		var x := WEST.x + 4.0 * knot
		if _ground(Vector2(x, 0.0)).y >= _deck_y + 5.0:
			_east_portal_x = x
	print("hill profile at z = 0: %s" % [range(-100, 1, 10).map(func(x): return "%d:%.1f" % [x, _ground(Vector2(x, 0.0)).y])])
	var preview := _preview(tool, points)
	_expect(preview.get("is_valid", false), "the tunnel stroke must preview: %s" % preview.get("invalid_reason", "missing"))
	_expect(preview.get("tunnel", false), "the preview must carry the tunnel intent")
	var prepared: PackedVector3Array = preview.get("prepared_points", PackedVector3Array())
	for point in prepared:
		_expect(absf(point.y - points[0].y) < 0.01, "a straight tunnel keeps the chord, got %s" % point)
	_expect(tool._commit_segment(points[-1]), "the click must queue the tunnel")
	var deadline := Time.get_ticks_msec() + TIMEOUT_MS
	while tool._commit_request_id > 0 and Time.get_ticks_msec() < deadline:
		await process_frame
	tool.cancel_road()
	tool.active = false
	var classes := []
	for edge in int(_simulation.get_road_benchmark_state()["edge_slots"]):
		classes.append(_simulation.get_edge_class(edge))
	classes.sort()
	_expect(classes == [0, 0, 2], "the stroke must commit as two approaches and a tunnel, got %s" % [classes])
	# Over the bore the visible surface is the hill, not the buried deck.
	var over_bore: float = _simulation.get_world_surface_height(Vector2.ZERO)
	_expect(over_bore > points[0].y + 5.0, "the surface over the bore must be the hill, got %.2f over a deck at %.2f" % [over_bore, points[0].y])

	await _drive_through()
	_press_t(input_manager)
	_expect(not tool.tunnel_enabled, "T must turn the tunnel toggle off")

	var screenshot := OS.get_environment("METRUM_ROAD61_SCREENSHOT")
	if OS.get_environment("METRUM_ROAD61_SKY_TEST") == "1":
		await _count_sky(screenshot)
	elif screenshot != "":
		await _capture(screenshot)
	_finish()

func _press_t(input_manager: Node) -> void:
	var key := InputEventKey.new()
	key.keycode = KEY_T
	key.pressed = true
	input_manager._unhandled_input(key)

func _ground(at: Vector2) -> Vector3:
	return Vector3(at.x, _simulation.get_world_surface_height(at), at.y)

# One frame samples the stroke and previews it synchronously, as the pointer would.
func _preview(tool: Node, points: PackedVector3Array) -> Dictionary:
	tool._begin_stroke(points[0], 0, Vector3.ZERO)
	tool.set_scripted_pointer(true, points[-1])
	tool._process(1.0 / 60.0)
	return tool._preview

# A debug car from the west approach to the east one passes the bore under the hill's top.
func _drive_through() -> void:
	var car: int = _simulation.spawn_debug_car(_ground(WEST), _ground(EAST))
	_expect(car >= 0, "a car route must run through the tunnel")
	if car < 0:
		return
	_simulation.set_simulation_speed(2.0)
	var picked := false
	var inside := false
	var out := false
	var drawn_in_bore := 0
	var deadline := Time.get_ticks_msec() + TIMEOUT_MS
	while not out and Time.get_ticks_msec() < deadline:
		await process_frame
		# Inspection follows the picked agent; the lanes list the car once it has moved.
		if not picked:
			picked = _simulation.pick_debug_agent(_ground(WEST), 60.0) == car
			continue
		var details: Dictionary = _simulation.get_agent_inspection(car)
		if not details.has("position"):
			continue
		var position: Vector3 = details["position"]
		var edge := int(details.get("lane_edge", -1))
		if edge >= 0 and _simulation.get_edge_class(edge) == 2:
			inside = true
			# The inspector reports the car on its lane in the bore, not on the hill above it.
			if absf(position.x) < 50.0:
				_expect(position.y < _bore_cover_ground - 5.0, "the inspector reports the car inside the bore, got %s under ground of at least %.1f" % [position, _bore_cover_ground])
			# The drawn car stands on the buried deck, never on the hill above it.
			if details.has("drawn_position"):
				var drawn: Vector3 = details["drawn_position"]
				if absf(drawn.x) < 50.0:
					drawn_in_bore += 1
					_expect(drawn.y < _bore_cover_ground - 5.0, "the car is drawn inside the bore, got %s under ground of at least %.1f" % [drawn, _bore_cover_ground])
		out = inside and position.x > 100.0
	_simulation.set_simulation_speed(0.0)
	_expect(picked, "the debug car must be picked")
	_expect(inside and out, "the car must pass under the hill and leave the east portal (inside %s, out %s)" % [inside, out])
	print("car frames drawn in the bore: %d" % drawn_in_bore)

# The west portal from the approach, once the terrain and roads settle.
func _capture(path: String) -> void:
	var camera: Node = root.get_viewport().get_camera_3d()
	var top: float = _simulation.get_world_surface_height(Vector2(-75.0, 0.0))
	var view := PackedFloat32Array([-80.0, top * 0.5, 0.0, -1.2, -0.42, 85.0, 0.0])
	# METRUM_ROAD61_VIEW: another orbit view, `pivot x, y, z, yaw, pitch, distance`.
	var custom := OS.get_environment("METRUM_ROAD61_VIEW").split_floats(",")
	if custom.size() == 6:
		view = PackedFloat32Array(Array(custom) + [0.0])
	if OS.get_environment("METRUM_ROAD61_INSIDE") == "1":
		camera.terrain_clearance_enabled = false
	camera.set_view_state(view)
	var car: int = _simulation.spawn_debug_car(_ground(WEST), _ground(EAST))
	_simulation.set_simulation_speed(1.0)
	var drive := OS.get_environment("METRUM_ROAD61_DRIVE_S")
	await create_timer(float(drive) if drive != "" else 6.0).timeout
	_simulation.set_simulation_speed(0.0)
	await create_timer(4.0).timeout
	await RenderingServer.frame_post_draw
	_expect(car >= 0 and root.get_texture().get_image().save_png(path) == OK, "the screenshot must save")

# From the middle of the bore looking east along it, with the sky drawn in one key colour: no key
# pixel outside the far mouth's opening (its lining's mouth, grown by half a metre and 2 px).
func _count_sky(path: String) -> void:
	var camera: Camera3D = root.get_viewport().get_camera_3d()
	camera.terrain_clearance_enabled = false
	# The camera's own environment replaces the world's: no sky shader, weather compositor,
	# distance fog or per-frame palette paints over the key colour.
	var environment := Environment.new()
	environment.background_mode = Environment.BG_COLOR
	environment.background_color = Color(1.0, 0.0, 1.0)
	environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	environment.ambient_light_color = Color(0.5, 0.5, 0.5)
	environment.reflected_light_source = Environment.REFLECTION_SOURCE_DISABLED
	camera.environment = environment
	# The control: from the west approach, looking up the hill, the key shows.
	camera.set_view_state(PackedFloat32Array([-120.0, _deck_y + 2.0, 0.0, -PI / 2.0, 0.25, 4.0, 0.0]))
	await create_timer(2.0).timeout
	await RenderingServer.frame_post_draw
	var control := root.get_texture().get_image()
	if path != "":
		control.save_png(path.get_basename() + "-control.png")
	var outside := _key_pixels(control, Rect2())
	_expect(outside.x > 1000, "the keyed sky must show outside the tunnel, got %d pixels" % outside.x)
	camera.set_view_state(PackedFloat32Array([0.0, _deck_y + 2.0, 0.0, -PI / 2.0, -0.02, 4.0, 0.0]))
	await create_timer(4.0).timeout
	await RenderingServer.frame_post_draw
	var image := root.get_texture().get_image()
	if path != "":
		image.save_png(path)
	var corners := [Vector3(_east_portal_x, _deck_y - 0.5, -5.5), Vector3(_east_portal_x, _deck_y - 0.5, 5.5), Vector3(_east_portal_x, _deck_y + 5.0, -5.5), Vector3(_east_portal_x, _deck_y + 5.0, 5.5)]
	var opening := Rect2(camera.unproject_position(corners[0]), Vector2.ZERO)
	for corner in corners:
		opening = opening.expand(camera.unproject_position(corner))
	var counts := _key_pixels(image, opening.grow(2.0))
	print("sky pixels: %d outside the tunnel (control), %d from inside the bore outside the far mouth %s, %d through it" % [outside.x, counts.x, opening, counts.y])
	_expect(counts.x == 0, "no sky may show through the lining: %d key pixels outside the far mouth" % counts.x)

# Key-coloured pixels of `image`: x outside `opening`, y inside it.
func _key_pixels(image: Image, opening: Rect2) -> Vector2i:
	var counts := Vector2i.ZERO
	for y in image.get_height():
		for x in image.get_width():
			var pixel := image.get_pixel(x, y)
			if pixel.r > 0.8 and pixel.g < 0.2 and pixel.b > 0.8:
				if opening.has_point(Vector2(x, y)):
					counts.y += 1
				else:
					counts.x += 1
	return counts

func _finish() -> void:
	print("Road tunnel tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)
