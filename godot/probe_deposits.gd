# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: probe_deposits.gd
#  script_path: probe_deposits.gd
#  module_name: probe_deposits
#  version: 0.1.0
#  author: [BantedHam]
#  description: Headless drill for the boundary upward half: the mine
#           extracts under time, the tick harness aggregates depletion
#           into deposit cells, and the engine heightmap node reads
#           the city mining back cell-exact.
#  kind: spike
#  spec: none
#  internal_dependencies: [scripts/core/engine_boundary.gd,
#           scripts/core/spike_stats.gd]
#  external_dependencies: [Godot 4.x]
#  features: [deposits-upward, measured-rows]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-09-02
# =========================================================================

# Headless drill for the boundary's upward half: the mine extracts, the
# tick harness writes the deposit grid, and the engine opens it back as
# measured rows, cell-exact.
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
	# One CONNECTED border road: buildings grow on demand pressure
	# alone, but households need a route in before anyone works a mine.
	sim.add_road(PackedVector3Array([
		Vector3(-999.0, 0.0, -280.0), Vector3(300.0, 0.0, -280.0)]), 2, 2)
	await _frames(150)
	var cand: int = sim.check_border_candidate(Vector3(-999.0, 0.0, -280.0))
	if cand >= 0:
		sim.set_border_connection(cand)
	var et := get_root().get_node_or_null("EngineTick")
	if et != null:
		et._deliver_engine_inputs()
	var placed: Dictionary = sim.place_industry_building(
		"kenney:building.coal.mine", 150.0, -292.0)
	_check("the mine places", bool(placed.get("ok", false)))
	if bool(placed.get("ok", false)):
		sim.commit_extractor_polygon(int(placed["building_id"]),
			PackedVector2Array([Vector2(125, -325), Vector2(175, -325),
				Vector2(175, -275), Vector2(125, -275)]))
	for i in 10:
		sim.apply_zoning_parcel_at(10.0 + float(i) * 28.0, -266.0, 1, 4, 4)

	# Run until the mine has actually extracted: staffing, commuting,
	# and production all take game time.
	sim.set_simulation_speed(50.0)
	var extracted := 0.0
	for round in 30:
		await _frames(300)
		var sites: Array = sim.get_extractor_sites()
		if not sites.is_empty():
			extracted = float((sites[0] as Dictionary).get("extracted_units", 0.0))
		if round % 5 == 0 or extracted > 0.0:
			var info: Dictionary = sim.get_building_info_at(150.0, -308.5)
			var house: Dictionary = sim.get_building_info_at(38.0, -261.5)
			print("round %d: house hh=%s adults=%s | extracted %.1f, mine uc=%s rem_h=%s workers=%s broken=%s" % [
				round, str(house.get("household_count", "?")),
				str(house.get("adult_count", "?")),
				extracted, str(info.get("under_construction", "?")),
				str(info.get("construction_remaining_hours", "?")),
				str(info.get("worker_count", "?")), str(info.get("economy_broken", "?"))])
		if extracted > 0.0:
			break
	sim.set_simulation_speed(0.0)
	_check("the mine extracts coal under time", extracted > 0.0)

	# The harness writes the deposit on its delivery, and the engine
	# opens it back as measured rows.
	if et != null:
		et._deliver_engine_inputs()
	var path := "" if et == null else String(et.last_deposit_path)
	print("deposit path: %s" % path)
	_check("the deposit grid was written", path != "" and FileAccess.file_exists(path))
	if path != "" and FileAccess.file_exists(path):
		var opened = load("res://scripts/core/engine_boundary.gd").open_deposit(path)
		_check("the engine opens the deposit", opened != null and opened.ok)
		if opened != null and opened.ok:
			# The query replicates the writer's mapping from the site's
			# own coordinates: same origin, same cell, same half-cell
			# centre, so the read-back is the write, not a guess.
			var site0 := (sim.get_extractor_sites()[0] as Dictionary)
			var latest := float(site0.get("extracted_units", 0.0))
			var sx0 := float(site0["x"])
			var sz0 := float(site0["z"])
			var dox := sx0 - 100.0
			var doz := sz0 - 100.0
			var dcx := int((sx0 - dox) / 100.0)
			var dcz := int((sz0 - doz) / 100.0)
			var lon := dox / 6000.0 + (float(dcx) + 0.5) / 60.0
			var lat := doz / 6000.0 - (float(dcz) + 0.5) / 60.0
			var v: float = opened.data_elevation_m(lat, lon)
			print("engine reads back: %.1f units (extracted %.1f)" % [v, latest])
			_check("the engine reads the city's mining as measured rows",
				absf(v - roundf(latest)) < 1.0)

	print("=== %d passed, %d failed ===" % [passed, failed])
	load("res://scripts/core/spike_stats.gd").record("probe_deposits", passed, failed)
	quit(1 if failed > 0 else 0)

# =========================================================================
# INIT
# =========================================================================
func _init() -> void:
	_run()
