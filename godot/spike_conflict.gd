extends SceneTree

## Does signaling a junction stop conflicting movements from occupying it?
##
## The earlier collision count was measured with no signal installed, so both
## cross streets held permanent green and every route ran straight through the
## box. Two conflicting through-movements sent into one junction at once will
## overlap; that was an authored conflict, not an engine defect.
##
## This runs the same traffic twice on the same geometry and compares:
##
##   A. Uncontrolled. Every turn permitted, both streets green forever.
##   B. Two-phase signal. One street runs while the other is held.
##
## The claim under test is that B has materially fewer cars occupying the same
## point inside the junction than A. Only the junction box is measured, because
## that is the only place two streets can conflict.
##
## Run WITHOUT --headless:
##   godot --path godot --script spike_conflict.gd

const CARS := 120
const EXTENT := 260.0
const BOX_M := 30.0
const CAR_LEN_M := 2.6

var sim = null
var failures := 0
var checks := 0
var junction := -1
var arm_ids: Array = []

func check_true(label: String, ok: bool) -> void:
	checks += 1
	if ok:
		print("  ok    %s" % label)
	else:
		failures += 1
		print("  FAIL  %s" % label)

func _init() -> void:
	print("=== conflicting movements, signaled vs not ===")
	change_scene_to_file("res://scenes/Main.tscn")
	_run()

func _frames(n: int) -> void:
	for _i in range(n):
		await process_frame

func _shot(name: String) -> void:
	await process_frame
	var img: Image = get_root().get_texture().get_image()
	img.save_png(ProjectSettings.globalize_path("res://../shot_conflict_%s.png" % name))

## Car positions inside the junction box only.
func _in_box() -> Array:
	var out := []
	var by_type: Dictionary = sim.get_car_transforms()
	for key in by_type:
		var buf: PackedFloat32Array = by_type[key]
		for i in range(buf.size() / 12):
			var o := i * 12
			var p := Vector3(buf[o + 9], buf[o + 10], buf[o + 11])
			if p.length() <= BOX_M:
				out.append(p)
	return out

## Pairs of cars closer than one car length, sampled over a window.
##
## Reported as the worst single frame rather than a total, so a longer sample
## cannot inflate it.
func _worst_overlap(samples: int) -> Dictionary:
	var worst_pairs := 0
	var worst_dist := 999.0
	var peak_in_box := 0
	for _s in range(samples):
		var ps := _in_box()
		peak_in_box = max(peak_in_box, ps.size())
		var pairs := 0
		for i in range(ps.size()):
			for j in range(i + 1, ps.size()):
				var d: float = (ps[i] as Vector3).distance_to(ps[j] as Vector3)
				if d < worst_dist:
					worst_dist = d
				if d < CAR_LEN_M:
					pairs += 1
		worst_pairs = max(worst_pairs, pairs)
		await _frames(20)
	return {"pairs": worst_pairs, "dist": worst_dist, "peak": peak_in_box}

func _run() -> void:
	await _frames(180)
	sim = get_root().get_node_or_null("Main/SimulationNode")
	if sim == null:
		print("FAIL: no SimulationNode")
		quit(1)
		return

	print("-- building a cross junction --")
	sim.add_road(PackedVector3Array([
		Vector3(-EXTENT, 0, 0), Vector3(EXTENT, 0, 0),
	]), 2, 2)
	await _frames(400)
	sim.add_road(PackedVector3Array([
		Vector3(0, 0, -EXTENT), Vector3(0, 0, EXTENT),
	]), 2, 2)
	await _frames(600)

	var nodes: PackedVector3Array = sim.get_network_nodes()
	for n in range(nodes.size()):
		if sim.get_node_connection_count(n) >= 3:
			junction = n
			break
	check_true("found the junction", junction >= 0)
	if junction < 0:
		quit(1)
		return

	# Group arms by the street they belong to, from real geometry. Sorted edge
	# ids do NOT alternate around the junction: on this cross, edge 0 runs west,
	# 1 north, 2 south, 3 east. Pairing [0,2] and [1,3] puts a crossing movement
	# in every phase, so the signal would cycle between two conflicting sets and
	# send cross traffic through together.
	var seen := {}
	var street_x: Array = []
	var street_z: Array = []
	for entry in sim.get_node_lanes(junction):
		if not (entry is Dictionary and entry.has("edge_id")):
			continue
		var e: int = int(entry["edge_id"])
		if seen.has(e):
			continue
		seen[e] = true
		var pair: Vector2i = sim.get_edge_nodes(e)
		var other: int = pair.y if pair.x == junction else pair.x
		var dir: Vector3 = (nodes[other] - nodes[junction]).normalized()
		if abs(dir.x) > abs(dir.z):
			street_x.append(e)
		else:
			street_z.append(e)
	arm_ids = street_x + street_z
	print("  junction=%d  east-west=%s  north-south=%s"
		% [junction, str(street_x), str(street_z)])
	check_true("four arms meet here", arm_ids.size() == 4)
	check_true("each street has two opposing arms",
		street_x.size() == 2 and street_z.size() == 2)
	if arm_ids.size() < 4:
		quit(1)
		return

	sim.spawn_test_traffic(CARS)
	await _frames(180)
	sim.set_simulation_speed(3.0)
	await _frames(400)
	print("  %d cars running" % sim.get_agent_count())

	# A. Uncontrolled: both streets green, every turn permitted.
	print("\n-- A: uncontrolled, both streets run at once --")
	sim.clear_junction_control(junction)
	await _frames(400)
	var a: Dictionary = await _worst_overlap(12)
	await _shot("a_uncontrolled")
	print("  peak %d cars in the box, %d overlapping pairs, closest %.2f m"
		% [a["peak"], a["pairs"], a["dist"]])

	# B. Two-phase signal: opposite arms together, the conflicting pair held.
	print("\n-- B: two-phase signal, one street at a time --")
	# One street per phase. Both arms of a street run together because they do
	# not conflict; the crossing street is held.
	sim.add_junction_signal_phase(junction, PackedInt32Array(street_x), 12.0, 3.0)
	sim.add_junction_signal_phase(junction, PackedInt32Array(street_z), 12.0, 3.0)
	var ctl: Dictionary = sim.get_junction_control(junction)
	print("  kind=%s cycle=%.1fs" % [ctl.get("kind", "?"), ctl.get("cycle_s", -1.0)])
	await _frames(600)
	var b: Dictionary = await _worst_overlap(12)
	await _shot("b_signaled")
	print("  peak %d cars in the box, %d overlapping pairs, closest %.2f m"
		% [b["peak"], b["pairs"], b["dist"]])

	# The direct question: while a street is red, does anything from it enter
	# the junction? Counting occupancy is indirect. This watches the held arms
	# specifically and reports any car that got inside the box anyway.
	print("\n-- C: does a red arm leak cars into the junction? --")
	var leaks := 0
	var samples := 0
	for _s in range(40):
		# The sim's own clock. Passing a made-up t asks what the light would
		# show at some other moment, not what it is showing now.
		var t: float = sim.get_sim_time()
		# Whichever street is red right now is the one that must stay out.
		var x_red: bool = sim.get_junction_signal_aspect(junction, street_x[0], t) == 2
		var held: Array = street_x if x_red else street_z
		var held_axis_is_x := x_red
		for p in _in_box():
			# A car inside the box that belongs to the held street is a leak.
			var on_x: bool = abs(p.x) >= abs(p.z)
			if on_x == held_axis_is_x and p.length() < BOX_M * 0.5:
				leaks += 1
		samples += 1
		await _frames(15)
	print("  %d samples, %d held-street cars found inside the junction" % [samples, leaks])
	check_true("a red street does not put cars in the junction (%d)" % leaks, leaks == 0)

	print("")
	check_true(
		"signaling reduces conflicting occupancy (%d -> %d pairs)"
			% [a["pairs"], b["pairs"]],
		b["pairs"] < a["pairs"]
	)
	check_true(
		"the junction is not simply emptied (%d cars still in the box)" % b["peak"],
		b["peak"] > 0
	)

	print("")
	if failures == 0:
		print("PASS: %d checks, 0 failures" % checks)
	else:
		print("FAIL: %d checks, %d failures" % [checks, failures])
	quit(0 if failures == 0 else 1)
