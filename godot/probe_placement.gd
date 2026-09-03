# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: probe_placement.gd
#  script_path: probe_placement.gd
#  module_name: probe_placement
#  version: 0.2.0
#  author: [BantedHam]
#  description: Three worlds, one placement each: flat zero, flat at a
#           nonzero base elevation, and the engine-filled world. The
#           discrimination that convicted the stored-height convention
#           and exposed the frontage failure at elevation.
#  kind: spike
#  spec: none
#  internal_dependencies: []
#  external_dependencies: [Godot 4.x]
#  features: [placement-discrimination, world-comparison]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-09-02
# =========================================================================

# Headless placement probe, round two: three worlds, one placement
# each. Flat zero is the world the game has always run on; flat at a
# nonzero base separates elevation from shape; the engine fill is the
# world that first exposed the rejection. Whichever worlds reject name
# the bug: shape, elevation, or only the fill.
extends SceneTree

# =========================================================================
# INIT
# =========================================================================
func _init() -> void:
	_run()

# =========================================================================
# FRAMES
# =========================================================================
func _frames(n: int) -> void:
	for _i in range(n):
		await process_frame

# =========================================================================
# ATTEMPT
# =========================================================================
func _attempt(sim: Node, label: String) -> void:
	var bx := 150.0
	var road_z := -280.0
	sim.add_road(PackedVector3Array([
		Vector3(bx - 150.0, 0.0, road_z), Vector3(bx + 150.0, 0.0, road_z)]), 2, 2)
	var last := {}
	for i in 30:
		await _frames(15)
		last = sim.place_industry_building("kenney:building.coal.mine", bx, road_z - 20.0)
		if bool(last.get("ok", false)):
			print("%s: PLACED after %d waits" % [label, i])
			return
	print("%s: NEVER PLACED, last error: %s (h at site %.2f)" % [
		label, String(last.get("error", "?")),
		sim.get_height_at(Vector2(bx, road_z - 20.0))])

# =========================================================================
# FRESH SIM
# =========================================================================
func _fresh_sim(old: Node) -> Node:
	if old != null:
		old.queue_free()
		await process_frame
	var sim := SimulationNode.new()
	get_root().add_child(sim)
	await process_frame
	sim.load_asset_packs(ProjectSettings.globalize_path("user://mods/"), "")
	return sim

# =========================================================================
# RUN
# =========================================================================
func _run() -> void:
	await process_frame
	var sim: Node = await _fresh_sim(null)
	print("flat zero: ", sim.create_blank_world(2000.0, 2000.0, 10.0, 640.0, 0.0))
	await _attempt(sim, "flat zero world")

	sim = await _fresh_sim(sim)
	print("flat base 5: ", sim.create_blank_world(2000.0, 2000.0, 10.0, 640.0, 5.0))
	await _attempt(sim, "flat world at base 5 m")

	sim = await _fresh_sim(sim)
	print("filled: ", sim.create_blank_world(2000.0, 2000.0, 10.0, 640.0, 0.0),
		" / ", sim.apply_engine_ground(0.5, 0.0, 0x2E5D, 8.0, 1000.0))
	await _attempt(sim, "engine-filled world")
	quit(0)
