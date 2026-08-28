extends SceneTree

## Builds a signaled cross junction and measures whether crossing movements are
## admitted into the box at the same time.
##
## Two cars closer than one car length inside the junction means two conflicting
## movements were let in together. The signal is installed before anything is
## measured: an uncontrolled junction gives every arm permanent green, so cross
## traffic runs simultaneously by design and the result says nothing.
##
## Arms are grouped into streets by geometry rather than by edge id, because ids
## are not stable between runs and pairing sorted ids puts a crossing movement in
## every phase.
##
## Run WITHOUT --headless so the renderers are live:
##   godot --path godot --script spike_left_turn.gd

const CARS := 120
const EXTENT := 260.0
const BOX_M := 30.0
const CAR_LEN_M := 2.6

const SpikeRecord = preload("res://spike_record.gd")

var sim = null
var rec = null
var failures := 0
var checks := 0

func check_true(label: String, ok: bool) -> void:
	checks += 1
	if not ok:
		failures += 1
	rec.check(label, ok)

func _init() -> void:
	print("=== left turns and overlapping cars, in the rendered game ===")
	rec = SpikeRecord.new("left_turn")
	change_scene_to_file("res://scenes/Main.tscn")
	_run()

func _frames(n: int) -> void:
	for _i in range(n):
		await process_frame

func _shot(name: String) -> void:
	await process_frame
	var img: Image = get_root().get_texture().get_image()
	img.save_png(ProjectSettings.globalize_path("res://../shot_lt_%s.png" % name))
	print("  [shot] %s" % name)

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

## Worst overlap seen inside the junction across a sampling window.
func _worst(samples: int) -> Dictionary:
	var worst_pairs := 0
	var worst_dist := 999.0
	var peak := 0
	for _s in range(samples):
		var ps := _in_box()
		peak = max(peak, ps.size())
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
	return {"pairs": worst_pairs, "dist": worst_dist, "peak": peak}

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
	print("  nodes=%d" % sim.get_network_nodes().size())

	# Group the four arms by the street they belong to, from geometry. Edge ids
	# are not stable between runs, so sorting them and pairing [0,2] / [1,3]
	# puts a crossing movement in every phase.
	var nodes: PackedVector3Array = sim.get_network_nodes()
	var junction := -1
	for n in range(nodes.size()):
		if sim.get_node_connection_count(n) >= 3:
			junction = n
			break
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
	print("  junction=%d  east-west=%s  north-south=%s"
		% [junction, str(street_x), str(street_z)])

	print("\n-- %d cars, two-phase signal: one street at a time --" % CARS)
	sim.spawn_test_traffic(CARS)
	await _frames(180)
	# One street per phase, both of its directions together, which is what a
	# real light does. The conflict table is a yield rule and does not phase
	# anything on its own; without a signal both streets run at once.
	sim.add_junction_signal_phase(junction, PackedInt32Array(street_x), 12.0, 3.0)
	sim.add_junction_signal_phase(junction, PackedInt32Array(street_z), 12.0, 3.0)
	var ctl: Dictionary = sim.get_junction_control(junction)
	print("  kind=%s cycle=%.1fs" % [ctl.get("kind", "?"), ctl.get("cycle_s", -1.0)])
	sim.set_simulation_speed(3.0)
	await _frames(600)
	print("  agents=%d" % sim.get_agent_count())

	var r: Dictionary = await _worst(15)
	await _shot("signaled")
	print("  peak %d cars in the box, %d overlapping pairs, closest %.2f m"
		% [r["peak"], r["pairs"], r["dist"]])

	# The question this whole round has been about. Two cars inside a junction
	# closer than a car length means two crossing movements were admitted.
	#
	# Labels carry no measured numbers. A label is the identity of a check
	# across runs, so embedding this run's figures in it would make every run a
	# different check and the regression comparison would never match anything.
	check_true("no cars overlap inside the junction", r["pairs"] == 0)
	check_true("traffic still flows through the junction", r["peak"] > 0)

	# Numbers worth comparing to the last run. None of these is pass or fail on
	# its own; 69 cars through a junction is only meaningful against what it did
	# before.
	rec.measure("overlapping_pairs", r["pairs"])
	rec.measure("peak_cars_in_box", r["peak"])
	rec.measure("closest_pair_m", snappedf(r["dist"], 0.01))
	rec.measure("agents", sim.get_agent_count())
	rec.measure("cycle_s", ctl.get("cycle_s", -1.0))

	# Watch the junction cycle for a while, then exit so this is usable as a
	# check rather than a session that has to be closed by hand.
	print("\nwatching the signal cycle: %s green, then %s." % [str(street_x), str(street_z)])
	var worst_live := 0
	for _round in range(20):
		await _frames(120)
		var live: Dictionary = await _worst(1)
		worst_live = max(worst_live, int(live["pairs"]))
		print("  in the junction: %d cars, %d overlapping pairs" % [live["peak"], live["pairs"]])
	rec.measure("worst_overlap_while_cycling", worst_live)

	quit(rec.finish())
