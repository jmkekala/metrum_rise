# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: probe_saveload.gd
#  script_path: probe_saveload.gd
#  module_name: probe_saveload
#  version: 0.1.0
#  author: [BantedHam]
#  description: Headless drill for the save round trip of a filled world:
#           the sculpted dig, the derived ground, the grown buildings,
#           and the parcel bounds must all return exactly, and the
#           fill-freeze caveat is measured rather than assumed.
#  kind: spike
#  spec: none
#  internal_dependencies: [scripts/core/spike_stats.gd]
#  external_dependencies: [Godot 4.x]
#  features: [save-round-trip, fill-freeze]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-09-02
# =========================================================================

# Headless save-and-load round trip of a filled, sculpted, grown world.
extends SceneTree

# =========================================================================
# THE DECLARATIONS
# =========================================================================

var passed := 0
var failed := 0

# =========================================================================
# CHECK
# =========================================================================
func _check(name: String, ok: bool) -> void:
	if ok:
		passed += 1
		print("  ok   %s" % name)
	else:
		failed += 1
		print("  FAIL %s" % name)

# =========================================================================
# FRAMES
# =========================================================================
func _frames(n: int) -> void:
	for _i in range(n):
		await process_frame

# =========================================================================
# RUN
# =========================================================================
func _run() -> void:
	await process_frame
	var sim := SimulationNode.new()
	sim.name = "SimulationNode"
	get_root().add_child(sim)
	await process_frame
	sim.load_asset_packs(ProjectSettings.globalize_path("user://mods/"), "")
	sim.create_blank_world(2000.0, 2000.0, 10.0, 640.0, 0.0)
	sim.apply_engine_ground(0.5, 0.0, 0x2E5D, 8.0, 1000.0)
	sim.add_road(PackedVector3Array([
		Vector3(0.0, 0.0, -280.0), Vector3(300.0, 0.0, -280.0)]), 2, 2)
	sim.add_road(PackedVector3Array([
		Vector3(-999.0, 0.0, 0.0), Vector3(-940.0, 0.0, 0.0)]), 2, 2)
	await _frames(150)
	var cand: int = sim.check_border_candidate(Vector3(-999.0, 0.0, 0.0))
	if cand >= 0:
		sim.set_border_connection(cand)
	# The store must hold a delivery before a reserve can bank engine
	# coal; the windowed drill forces one and so does this probe.
	var et := get_root().get_node_or_null("EngineTick")
	if et != null:
		et._deliver_engine_inputs()
	print("store: ", sim.engine_inputs_summary())
	var placed: Dictionary = sim.place_industry_building(
		"kenney:building.coal.mine", 150.0, -292.0)
	var reserve_before := 0.0
	if bool(placed.get("ok", false)):
		var c: Dictionary = sim.commit_extractor_polygon(int(placed["building_id"]),
			PackedVector2Array([Vector2(125, -325), Vector2(175, -325),
				Vector2(175, -275), Vector2(125, -275)]))
		reserve_before = float(c.get("total_reserve_units", 0.0))
	for i in 10:
		sim.apply_zoning_parcel_at(10.0 + float(i) * 28.0, -266.0, 1, 4, 4)
	# A sculpted dig, the measured override a save must carry.
	for i in 10:
		sim.sculpt_terrain(Vector2(-400.0, -400.0), 60.0, -0.5)
	sim.set_simulation_speed(10.0)
	await _frames(400)
	sim.set_simulation_speed(0.0)
	var grown_before := 0
	for i in 10:
		if sim.get_building_info_at(10.0 + float(i) * 28.0, -266.0).size() > 1:
			grown_before += 1
	var h_dig: float = sim.get_height_at(Vector2(-400.0, -400.0))
	var h_fill: float = sim.get_height_at(Vector2(500.0, 500.0))
	var bounds_before: Dictionary = sim.engine_parcel_bounds()
	print("pre-save: dig %.3f, fill %.3f, reserve %.1f, grown %d, parcels %d" % [
		h_dig, h_fill, reserve_before, grown_before, int(bounds_before.get("count", 0))])
	_check("the city grew before the save", grown_before > 0)
	_check("the reserve banked before the save", reserve_before > 0.0)

	var path := ProjectSettings.globalize_path("user://spike_saveload_test.sav")
	_check("save_game answers true", sim.save_game(path))

	# Scar the world, then load back over it: the load must restore.
	for i in 5:
		sim.sculpt_terrain(Vector2(-400.0, -400.0), 60.0, 1.0)
	_check("load_game answers true", sim.load_game(path))
	await _frames(60)

	var h_dig2: float = sim.get_height_at(Vector2(-400.0, -400.0))
	var h_fill2: float = sim.get_height_at(Vector2(500.0, 500.0))
	var grown_after := 0
	for i in 10:
		if sim.get_building_info_at(10.0 + float(i) * 28.0, -266.0).size() > 1:
			grown_after += 1
	var bounds_after: Dictionary = sim.engine_parcel_bounds()
	print("post-load: dig %.3f, fill %.3f, grown %d, parcels %d" % [
		h_dig2, h_fill2, grown_after, int(bounds_after.get("count", 0))])
	_check("the sculpted dig survives the round trip",
		absf(h_dig2 - h_dig) < 0.001)
	_check("the filled ground survives the round trip",
		absf(h_fill2 - h_fill) < 0.001)
	_check("grown buildings survive the round trip", grown_after == grown_before)
	_check("the parcel bounds survive the round trip",
		int(bounds_after.get("count", 0)) == int(bounds_before.get("count", 0)))

	# The fill-freeze caveat, measured: a loaded world's cells all read
	# as real values, so a re-apply finds nothing untouched.
	var refill: int = sim.apply_engine_ground(0.5, 0.0, 0x2E5D, 8.0, 1000.0)
	print("post-load re-apply filled: %d" % refill)
	_check("the fill-freeze caveat measures as recorded", refill == 0)

	DirAccess.remove_absolute(path)
	print("=== %d passed, %d failed ===" % [passed, failed])
	load("res://scripts/core/spike_stats.gd").record("probe_saveload", passed, failed)
	quit(1 if failed > 0 else 0)

# =========================================================================
# INIT
# =========================================================================
func _init() -> void:
	_run()
