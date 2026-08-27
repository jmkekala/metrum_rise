extends SceneTree

## Headless check that junction control and lane cross-sections are reachable
## from Godot and behave the way the Rust unit tests say they do.
##
## Run:
##   godot --headless --path godot --script spike_junction_control.gd
##
## The Rust tests prove the model in isolation. This proves the GDExtension
## boundary: that the methods exist, that the types marshal, and that a signal
## set from GDScript reports the aspects the program implies.

var failures := 0
var checks := 0

func check(label: String, actual: Variant, expected: Variant) -> void:
	checks += 1
	if typeof(actual) == typeof(expected) and actual == expected:
		print("  ok    %s = %s" % [label, str(actual)])
	else:
		failures += 1
		print("  FAIL  %s: got %s, want %s" % [label, str(actual), str(expected)])

func check_true(label: String, actual: bool) -> void:
	check(label, actual, true)

func _init() -> void:
	print("=== junction control over the GDExtension boundary ===")

	if not ClassDB.class_exists("SimulationNode"):
		print("FAIL: SimulationNode is not registered. The DLL did not load.")
		quit(1)
		return

	var sim = ClassDB.instantiate("SimulationNode")
	if sim == null:
		print("FAIL: could not instantiate SimulationNode")
		quit(1)
		return

	# Road placement is queued to the sim thread, and that thread is only started
	# by _ready(). A loose instance silently drops every command.
	sim.name = "SimulationNode"
	root.add_child(sim)

	_check_methods_exist(sim)
	await _build_and_settle(sim)
	_check_uncontrolled_default(sim)
	_check_priority(sim)
	_check_signal(sim)
	await _check_cross_section(sim)

	sim.queue_free()

	print("")
	if failures == 0:
		print("PASS: %d checks, 0 failures" % checks)
		quit(0)
	else:
		print("FAIL: %d checks, %d failures" % [checks, failures])
		quit(1)

func _check_methods_exist(sim) -> void:
	print("\n-- the API is bound --")
	for m in [
		"set_junction_priority",
		"add_junction_signal_phase",
		"set_junction_signal_offset",
		"clear_junction_control",
		"get_junction_control",
		"get_junction_signal_aspect",
		"add_road_with_cross_section",
	]:
		check_true("has %s" % m, sim.has_method(m))

## Waits for the sim thread to drain queued road commands.
##
## Road placement is asynchronous, so a read taken too early sees the graph
## mid-edit. This polls for the node count to stop moving rather than guessing a
## frame budget.
func _settle(min_nodes: int = 0, max_frames: int = 900) -> void:
	var sim = root.get_node_or_null("SimulationNode")
	var stable := 0
	var last := -1
	for _i in range(max_frames):
		await process_frame
		if sim == null:
			continue
		var n: int = sim.get_network_nodes().size()
		if n == last:
			stable += 1
		else:
			stable = 0
			last = n
		if stable >= 20 and n >= min_nodes:
			return

func _build_and_settle(sim) -> void:
	print("\n-- building a cross junction --")
	# Two roads meeting at the origin, so a node there has four arms.
	var ns := PackedVector3Array([
		Vector3(-100, 0, 0), Vector3(0, 0, 0), Vector3(100, 0, 0),
	])
	var ew := PackedVector3Array([
		Vector3(0, 0, -100), Vector3(0, 0, 0), Vector3(0, 0, 100),
	])
	sim.add_road(ns, 1, 1)
	await _settle(2)
	sim.add_road(ew, 1, 1)
	await _settle(5)
	# Two crossing roads split at the shared point, so the graph holds more nodes
	# than the four endpoints: the junction itself is one of them.
	var node_count: int = sim.get_network_nodes().size()
	check_true("the cross junction exists (got %d nodes)" % node_count, node_count >= 5)

func _check_uncontrolled_default(sim) -> void:
	print("\n-- a fresh junction is uncontrolled --")
	var c: Dictionary = sim.get_junction_control(0)
	check("kind", c.get("kind", "<missing>"), "uncontrolled")
	# An uncontrolled junction shows no aspect, so it must never report red.
	check("aspect", sim.get_junction_signal_aspect(0, 0, 0.0), 0)

func _check_priority(sim) -> void:
	print("\n-- priority signs --")
	sim.set_junction_priority(0, 0, 0)  # main
	sim.set_junction_priority(0, 1, 2)  # stop

	var c: Dictionary = sim.get_junction_control(0)
	check("kind", c.get("kind", "<missing>"), "priority")

	var signs: Dictionary = c.get("signs", {})
	check("edge 0 is main", signs.get(0, -1), 0)
	check("edge 1 is stop", signs.get(1, -1), 2)

	# Priority junctions show no aspect either.
	check("aspect stays green", sim.get_junction_signal_aspect(0, 1, 0.0), 0)

func _check_signal(sim) -> void:
	print("\n-- a timed signal --")
	# Two 20 s phases with 3 s amber: a 46 s cycle, matching the Rust test.
	sim.add_junction_signal_phase(0, PackedInt32Array([0, 2]), 20.0, 3.0)
	sim.add_junction_signal_phase(0, PackedInt32Array([1, 3]), 20.0, 3.0)

	var c: Dictionary = sim.get_junction_control(0)
	check("kind", c.get("kind", "<missing>"), "signal")
	check("cycle length", c.get("cycle_s", -1.0), 46.0)
	check("phase count", (c.get("phases", []) as Array).size(), 2)
	check("signs are gone", c.has("signs"), false)

	# 0 green, 1 amber, 2 red.
	check("arm 0 at t=0 is green", sim.get_junction_signal_aspect(0, 0, 0.0), 0)
	check("arm 1 at t=0 is red", sim.get_junction_signal_aspect(0, 1, 0.0), 2)
	check("arm 0 at t=21 is amber", sim.get_junction_signal_aspect(0, 0, 21.0), 1)
	check("arm 1 at t=24 is green", sim.get_junction_signal_aspect(0, 1, 24.0), 0)
	# The cycle repeats.
	check("t=46 matches t=0", sim.get_junction_signal_aspect(0, 0, 46.0), 0)

	print("\n-- the offset shifts the cycle, which is a green wave --")
	sim.set_junction_signal_offset(0, 23.0)
	check("arm 1 now green at t=0", sim.get_junction_signal_aspect(0, 1, 0.0), 0)
	sim.set_junction_signal_offset(0, 0.0)

	print("\n-- clearing returns it to uncontrolled --")
	sim.clear_junction_control(0)
	var after: Dictionary = sim.get_junction_control(0)
	check("kind", after.get("kind", "<missing>"), "uncontrolled")

func _check_cross_section(sim) -> void:
	print("\n-- a lane cross-section survives the boundary --")
	# Seven integers per band: kind, direction, width_mm, modes, marking, turns,
	# parking_angle.
	# kind:      0 travel, 1 median, 2 parking, 3 shoulder, 4 cycle, 5 reversible, 6 verge
	# direction: 0 forward, 1 backward, 2 none
	# One backward travel lane, a 1.5 m median, then two forward travel lanes.
	var bands := PackedInt32Array([
		0, 1, 3500, 2, 0, 0, 0,
		1, 2, 1500, 0, 0, 0, 0,
		0, 0, 3500, 2, 0, 0, 0,
		0, 0, 3500, 2, 0, 0, 0,
	])
	var pts := PackedVector3Array([Vector3(-100, 0, 300), Vector3(100, 0, 300)])
	var nodes_before: int = sim.get_network_nodes().size()
	sim.add_road_with_cross_section(pts, bands, false)
	await _settle(nodes_before + 2)
	var nodes_after: int = sim.get_network_nodes().size()
	check_true(
		"the road was added (%d -> %d nodes)" % [nodes_before, nodes_after],
		nodes_after > nodes_before
	)

	# Width must be the sum of the bands: 12.0 m of asphalt plus sidewalks. A
	# median takes real width rather than being a line painted on a 4-lane road.
	var edge_idx: int = sim.get_hovered_edge(0.0, 300.0)
	if edge_idx < 0:
		failures += 1
		checks += 1
		print("  FAIL  found the new edge: got %d" % edge_idx)
		return
	var w: float = sim.get_edge_width(edge_idx)
	check_true("width covers 12 m of asphalt (got %.2f)" % w, w >= 12.0)
