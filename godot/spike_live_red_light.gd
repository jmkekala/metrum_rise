extends SceneTree

## Windowed, rendered test: do cars actually stop at a red light?
##
## Loads the real Main scene so the terrain, road, and agent renderers are all
## live, builds a cross junction, puts cars on it, then turns every arm red and
## measures whether the fleet stops moving.
##
## Run WITHOUT --headless so the renderers actually draw:
##   godot --path godot --script spike_live_red_light.gd
##
## Writes shot_*.png next to the project at each stage, so the result is a
## picture rather than a claim.

const CARS := 300
const EXTENT := 260.0

var sim = null
var failures := 0
var checks := 0

func check_true(label: String, ok: bool) -> void:
	checks += 1
	if ok:
		print("  ok    %s" % label)
	else:
		failures += 1
		print("  FAIL  %s" % label)

func _init() -> void:
	print("=== live red light test (windowed) ===")
	change_scene_to_file("res://scenes/Main.tscn")
	_run()

func _frames(n: int) -> void:
	for _i in range(n):
		await process_frame

func _shot(name: String) -> void:
	await process_frame
	var img: Image = get_root().get_texture().get_image()
	var path := "res://../shot_%s.png" % name
	img.save_png(ProjectSettings.globalize_path(path))
	print("  [shot] %s" % name)

## Mean per-car movement between two position samples.
func _fleet_movement(a: Array, b: Array) -> float:
	var n: int = min(a.size(), b.size())
	if n == 0:
		return 0.0
	var total := 0.0
	for i in range(n):
		total += (a[i] as Vector3).distance_to(b[i] as Vector3)
	return total / float(n)

func _car_positions() -> Array:
	var out := []
	var by_type: Dictionary = sim.get_car_transforms()
	for key in by_type:
		var buf: PackedFloat32Array = by_type[key]
		for i in range(buf.size() / 12):
			var o := i * 12
			out.append(Vector3(buf[o + 9], buf[o + 10], buf[o + 11]))
	return out

## Samples fleet movement over a window.
func _measure() -> float:
	var p0 := _car_positions()
	await _frames(90)
	var p1 := _car_positions()
	return _fleet_movement(p0, p1)

func _run() -> void:
	# Let the scene, terrain, and renderers come up.
	await _frames(180)

	sim = get_root().get_node_or_null("Main/SimulationNode")
	if sim == null:
		print("FAIL: no SimulationNode under Main")
		quit(1)
		return
	print("-- Main scene is live, renderers up --")
	await _shot("01_world")

	print("\n-- building a cross junction --")
	sim.add_road(PackedVector3Array([
		Vector3(-EXTENT, 0, 0), Vector3(EXTENT, 0, 0),
	]), 2, 2)
	await _frames(400)
	sim.add_road(PackedVector3Array([
		Vector3(0, 0, -EXTENT), Vector3(0, 0, EXTENT),
	]), 2, 2)
	await _frames(600)

	var nodes: PackedVector3Array = sim.get_network_nodes()
	print("  nodes=%d" % nodes.size())
	check_true("the junction exists", nodes.size() >= 3)
	await _shot("02_roads")

	print("\n-- putting %d cars on the road --" % CARS)
	sim.spawn_test_traffic(CARS)
	await _frames(180)
	print("  agents=%d" % sim.get_agent_count())
	check_true("cars spawned", sim.get_agent_count() > 0)

	sim.set_simulation_speed(3.0)
	await _frames(400)
	await _shot("03_traffic")

	var free_flow := await _measure()
	print("  mean movement per car, no signals: %.3f m" % free_flow)
	check_true("cars are moving (%.3f m)" % free_flow, free_flow > 0.05)

	print("\n-- putting every junction on permanent red --")
	# A phase whose green set is empty holds every arm. Applied to every node,
	# nothing in the city may enter a junction.
	var held := 0
	for n in range(nodes.size()):
		if sim.get_node_connection_count(n) >= 3:
			sim.add_junction_signal_phase(n, PackedInt32Array([]), 600.0, 0.0)
			held += 1
	print("  junctions held: %d" % held)
	check_true("there were junctions to hold", held > 0)

	# Cars already inside a junction finish crossing, then queue at the line.
	await _frames(600)
	await _shot("04_red")

	var under_red := await _measure()
	print("  mean movement per car, all red: %.3f m" % under_red)

	check_true(
		"red slows the fleet (%.3f m -> %.3f m)" % [free_flow, under_red],
		under_red < free_flow * 0.5
	)

	print("\n-- releasing the signals --")
	for n in range(nodes.size()):
		if sim.get_node_connection_count(n) >= 3:
			sim.clear_junction_control(n)
	await _frames(600)
	await _shot("05_released")

	var released := await _measure()
	print("  mean movement per car, released: %.3f m" % released)
	check_true(
		"traffic recovers once the signals clear (%.3f m)" % released,
		released > under_red * 2.0
	)

	print("")
	if failures == 0:
		print("PASS: %d checks, 0 failures" % checks)
	else:
		print("FAIL: %d checks, %d failures" % [checks, failures])
	quit(0 if failures == 0 else 1)
