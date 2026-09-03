# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: spike_engine_live.gd
#  script_path: spike_engine_live.gd
#  module_name: spike_engine_live
#  version: 0.4.0
#  author: [BantedHam]
#  description: Every engine insertion point drilled in the RENDERED
#           game: the dig drawn on the composite ground, the coal
#           loop banking the delivered channel, the border opened,
#           parcels zoned and grown on engine land value, a derived
#           strike through the game audio, and the frame price under
#           time. Screenshots land in a dated spikes folder.
#  kind: spike
#  spec: none
#  internal_dependencies: [scripts/core/engine_mesh_source.gd,
#           scripts/core/engine_mind_source.gd,
#           scripts/core/engine_mineral_source.gd]
#  external_dependencies: [Godot 4.x]
#  features: [windowed-drill, screenshots, coal-loop, growth]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-09-02
# =========================================================================

extends SceneTree

## Windowed, rendered: every engine insertion point live in the real
## Main scene, the way the traffic spikes ran. The workflow toggle goes
## on, the scene boots with the engine drawing, and each hook answers
## through the same objects the game uses.
##
## Run WITHOUT --headless so the renderers actually draw:
##   godot --path godot --script spike_engine_live.gd
##
## Writes shot_engine_*.png next to the project, so the result is a
## picture rather than a claim.

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const EngineMeshSource := preload("res://scripts/core/engine_mesh_source.gd")
const MindSource := preload("res://scripts/core/engine_mind_source.gd")
const MineralSource := preload("res://scripts/core/engine_mineral_source.gd")

var failures := 0
var checks := 0
## The folder stamp: day.month.year, matching the dated spike folders.
var _run_stamp := "%d.%d.%s" % [
	Time.get_datetime_dict_from_system(true)["day"],
	Time.get_datetime_dict_from_system(true)["month"],
	str(Time.get_datetime_dict_from_system(true)["year"]).substr(2, 2)]
var _shot_index := 1

# =========================================================================
# CHECK TRUE
# =========================================================================
func check_true(label: String, ok: bool) -> void:
	checks += 1
	if ok:
		print("  ok    %s" % label)
	else:
		failures += 1
		print("  FAIL  %s" % label)

# =========================================================================
# INIT
# =========================================================================
func _init() -> void:
	print("=== engine live spike (windowed) ===")
	EngineMeshSource.set_enabled(true)
	change_scene_to_file("res://scenes/Main.tscn")
	_run()

# =========================================================================
# FRAMES
# =========================================================================
func _frames(n: int) -> void:
	for _i in range(n):
		await process_frame


# =========================================================================
# SHOT
# =========================================================================
## Bob's screenshot convention, matched exactly:
##   spikeNN_<subject>[_pixelart_filter]_<W/100>x<H/100>.png
## in a dated screenshots/spikes_<range>/ folder. The resolution is
## the real pixel size with two zeros collapsed, the filter is named
## only when it is actually on, and the spike number plus subject say
## which run and which proof. Nothing overwrites anything: a fixed
## filename cost a dozen runs of evidence once already.
func _shot(name: String) -> void:
	await process_frame
	var img: Image = get_root().get_texture().get_image()
	var w := img.get_width()
	var h := img.get_height()
	var filter := ""
	if absf(get_root().scaling_3d_scale - 1.0) > 0.001:
		filter = "_pixelart_filter"
	var dir := "res://../screenshots/spikes_%s" % _run_stamp
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(dir))
	var path := "%s/spike%02d_%s%s_%dx%d.png" % [
		dir, _shot_index, name, filter, int(w / 100.0), int(h / 100.0)]
	_shot_index += 1
	img.save_png(ProjectSettings.globalize_path(path))
	print("  [shot] %s" % path.get_file())

# =========================================================================
# RUN
# =========================================================================
func _run() -> void:
	# Let the scene, terrain, and renderers come up; the ground
	# reconciliation runs inside these frames and is priced below.
	var boot0 := Time.get_ticks_msec()
	await _frames(180)
	print("  [price] 180 boot frames took %d ms" % (Time.get_ticks_msec() - boot0))

	var sim = get_root().get_node_or_null("Main/SimulationNode")
	check_true("SimulationNode is live under Main", sim != null)
	if sim == null:
		quit(1)
		return
	var tick = get_root().get_node_or_null("EngineTick")
	check_true("EngineTick autoload is live", tick != null)
	if tick == null:
		quit(1)
		return

	# The pixel posture: one art pixel per four screen pixels, the
	# knob read from the pixelate node's own config.
	check_true("pixel art is all the way on",
		absf(get_root().scaling_3d_scale - 0.25) < 0.001)

	await _shot("boot")

	# Ground reconciliation ran at boot (toggle on, game scene), and a
	# second apply finds nothing untouched, which is idempotence and
	# the sculpt guarantee in one number.
	check_true("ground reconciliation ran at boot", tick._ground_applied)
	var t0 := Time.get_ticks_msec()
	var refill: int = sim.apply_engine_ground(0.5, 0.0, 0x2E5D, 8.0, 1000.0)
	print("  [price] second apply answered in %d ms" % (Time.get_ticks_msec() - t0))
	check_true("a second apply finds nothing untouched", refill == 0)

	# The intake, through the same call the harness makes on its
	# interval: a delivery lands, the summary reads back whole.
	tick._deliver_engine_inputs()
	var summary: Dictionary = sim.engine_inputs_summary()
	check_true("a delivery landed a revision", int(summary["revision"]) >= 1)
	check_true("the probe grid delivered whole", int(summary["parcels"]) == 25)
	var dmean := float(summary["desirability_mean"])
	check_true("desirability rides in its 0..1 band",
		dmean >= 0.0 and dmean <= 1.0)
	print("  [live] desirability mean %.4f, coal mean %.6f, revision %d" % [
		dmean, float(summary["coal_mean"]), int(summary["revision"])])

	# Parcel bounds: no city yet, so the count is zero and the
	# listener-centred grid is the honest fallback in force.
	var bounds: Dictionary = sim.engine_parcel_bounds()
	check_true("no city means no parcel bounds", int(bounds["count"]) == 0)

	# The tick harness's live surfaces: sky, drama, and the tide clock.
	check_true("weather publishes at the listener", not tick.conditions.is_empty())
	check_true("the director paces", tick.pacing.has("state"))
	var w0: float = tick.world_t
	await _frames(30)
	check_true("world time breathes the tide clock", tick.world_t > w0)

	# A mind lives in the running game: spawned into the roster the
	# harness already ticks, walking within a second of real time.
	MindSource.spawn(99, 7, Vector2(4.0, 4.0))
	await _frames(60)
	var ms: Dictionary = MindSource.state(99)
	check_true("a spawned mind lives and answers",
		not ms.is_empty() and String(ms["behavior"]) != "")
	if not ms.is_empty():
		print("  [live] mind 99: %s at %s" % [String(ms["behavior"]), str(ms["pos"])])
	MindSource.despawn(99)

	# THE SCULPT DRILL: deform, and the drawn ground must move. It runs
	# here, before any sim step, because terrain's process loop must be
	# awake to draw the dig and task 12's deadlock owns everything after
	# the first step. The sculpt lands at the world origin, screen
	# centre in the boot camera, so the pixels around centre witness.
	var h_before: float = sim.get_height_at(Vector2(0.0, 0.0))
	await process_frame
	var img_before: Image = get_root().get_texture().get_image()
	# A dig, not a mound: mining-shaped deformation goes down.
	for i in 10:
		sim.sculpt_terrain(Vector2(0.0, 0.0), 60.0, -0.5)
	await _frames(45)
	var h_after: float = sim.get_height_at(Vector2(0.0, 0.0))
	print("  [live] sculpt: sim height %.3f -> %.3f" % [h_before, h_after])
	check_true("the sim holds the dig", h_after < h_before - 0.1)
	var img_after: Image = get_root().get_texture().get_image()
	var scx := img_before.get_width() / 2
	var scy := img_before.get_height() / 2
	var moved := 0.0
	for dy in range(-60, 61, 6):
		for dx in range(-60, 61, 6):
			var a := img_before.get_pixel(scx + dx, scy + dy)
			var b := img_after.get_pixel(scx + dx, scy + dy)
			moved += absf(a.r - b.r) + absf(a.g - b.g) + absf(a.b - b.b)
	print("  [live] sculpt: centre pixels moved by %.3f" % moved)
	check_true("the drawn ground moves with the sculpt", moved > 1.0)

	# THE COAL LOOP LIVE: scout the delivered channel for a rich spot,
	# place a mine there, commit its polygon, and the reserve must bank
	# engine coal, painted deposits nowhere in sight. Every rejection
	# prints whole, so a failed placement teaches instead of hiding.
	var best := Vector3.ZERO
	var best_r := 0.0
	var candidates := PackedVector3Array()
	for cz in range(-300, 301, 150):
		for cx in range(-300, 301, 150):
			candidates.append(Vector3(float(cx), 0.0, float(cz)))
	var coal_r := MineralSource.richness("coal", candidates)
	for i in candidates.size():
		if coal_r[i] > best_r:
			best_r = coal_r[i]
			best = candidates[i]
	print("  [live] coal scout: best %.4f at %s" % [best_r, str(best)])
	check_true("the map holds minable coal", best_r > 0.01)
	# The registry names its own extractor: mine-like ids first, then
	# anything, each attempt printed, first committed polygon wins.
	var ids: PackedStringArray = sim.get_registered_asset_ids()
	var try_ids := PackedStringArray()
	for id in ids:
		var l := String(id).to_lower()
		if l.contains("coal") or l.contains("mine") or l.contains("extract"):
			try_ids.append(id)
	print("  [live] registry: %d assets, mine-like: %s" % [ids.size(), str(try_ids)])
	if try_ids.is_empty():
		try_ids = ids
	# Raw ground, road, mine: no leveling, because the stored-height
	# convention fix made the fill's slopes their authored gentle
	# selves. The road runs from the map border past the coal, and its
	# border endpoint becomes the connection immigrants arrive through:
	# without one, demand sits at (0, -1, -1) forever, which a headless
	# probe measured across twenty thousand frames.
	var road_z := best.z + 20.0
	# Both roads up front, before anything places: the city's own short
	# road, and a sixty metre stub touching the map's real edge. The
	# demand gate counts a Border node with any road edge attached, and
	# the headless probe proved arrivals need no route to town: demand
	# lifted and five buildings grew off a disconnected stub. Roads
	# added late wedge the scene (task 12's deadlock family).
	var hm: Vector2 = sim.get_heightmap_size()
	var border_x := -((hm.x - 1.0) * 10.0 * 0.5 - 1.0)
	print("  [live] world edge at x=%.0f" % border_x)
	sim.add_road(PackedVector3Array([
		Vector3(best.x - 150.0, 0.0, road_z),
		Vector3(best.x + 150.0, 0.0, road_z)]), 2, 2)
	sim.add_road(PackedVector3Array([
		Vector3(border_x, 0.0, 0.0),
		Vector3(border_x + 60.0, 0.0, 0.0)]), 2, 2)
	await _frames(300)
	var border_cand: int = sim.check_border_candidate(Vector3(border_x, 0.0, 0.0))
	print("  [live] border candidate: %d" % border_cand)
	check_true("the border stub opens the outside world", border_cand >= 0)
	if border_cand >= 0:
		sim.set_border_connection(border_cand)
	var poly := PackedVector2Array([
		Vector2(best.x - 25.0, best.z - 25.0),
		Vector2(best.x + 25.0, best.z - 25.0),
		Vector2(best.x + 25.0, best.z + 25.0),
		Vector2(best.x - 25.0, best.z + 25.0)])
	var committed: Dictionary = {}
	for attempt in 20:
		if not committed.is_empty():
			break
		for id in try_ids:
			if not committed.is_empty():
				break
			for off: float in [12.0, 20.0, 32.0]:
				var pz := road_z - off
				var placed: Dictionary = sim.place_industry_building(
					String(id), best.x, pz)
				if not bool(placed.get("ok", false)):
					if attempt == 0 or attempt == 19:
						print("  [live] %s at %.0f m (try %d): %s" % [
							String(id), off, attempt,
							String(placed.get("error", "?"))])
					continue
				var c: Dictionary = sim.commit_extractor_polygon(
					int(placed["building_id"]), poly)
				print("  [live] %s placed %.0f m off road (try %d), polygon: %s" % [
					String(id), off, attempt, str(c)])
				if bool(c.get("ok", false)):
					committed = c
					break
		if committed.is_empty():
			await _frames(60)
	check_true("an extractor places and its polygon commits",
		not committed.is_empty())
	check_true("the reserve banks engine coal",
		float(committed.get("total_reserve_units", 0.0)) > 0.0)

	# THE CITY GROWS: parcels zoned along the coal road, the bounds
	# export re-aims the probe grid, and demand grows buildings on the
	# delivered land value. Frozen shells render when they do.
	var profiles: Array = sim.get_zone_profiles()
	var res_id := -1
	for p in profiles:
		var pd := p as Dictionary
		if String(pd.get("zone_type", "")) == "residential":
			res_id = int(pd.get("runtime_id", -1))
			break
	print("  [live] %d zone profiles, residential runtime id %d" % [
		profiles.size(), res_id])
	check_true("a residential zone profile exists", res_id >= 0)
	var zoned := 0
	for i in 10:
		var zx := best.x - 140.0 + float(i) * 28.0
		if sim.apply_zoning_parcel_at(zx, road_z + 14.0, res_id, 4, 4):
			zoned += 1
	print("  [live] zoned %d/10 parcels along the road" % zoned)
	check_true("parcels zone along the road", zoned >= 3)
	var b2: Dictionary = sim.engine_parcel_bounds()
	print("  [live] parcel bounds: %s" % str(b2))
	check_true("the city exports its bounds", int(b2.get("count", 0)) > 0)
	tick._deliver_engine_inputs()
	var s2: Dictionary = sim.engine_inputs_summary()
	check_true("the re-aimed grid delivers whole",
		int(s2.get("parcels", 0)) == 25)

	# Growth under time. Terrain's process loop deadlocks on the state
	# a stepped sim leaves behind (task 12 holds six reproductions:
	# even waking it AFTER speed returns to zero wedges), so it goes
	# dark before the window and STAYS dark to the end of the run; the
	# boot-built terrain keeps rendering, and the freeze stays an open
	# finding, not a hidden one.
	var main_node := get_root().get_node("Main")
	var terrain_node := main_node.get_node_or_null("Terrain")
	if terrain_node != null:
		terrain_node.set_process(false)
	print("  [live] growth window: terrain dark, speed 5.0")
	sim.set_simulation_speed(5.0)
	var ft0 := Time.get_ticks_msec()
	await _frames(300)
	var ms_frame := float(Time.get_ticks_msec() - ft0) / 300.0
	print("  [live] sustained: %.1f ms/frame over 300 frames at speed 5" % ms_frame)
	check_true("the running city sustains frames", ms_frame < 10000.0)
	sim.set_simulation_speed(0.0)
	print("  [live] terrain stays dark; task 12 owns its wake")
	var grown := 0
	var sample_info := {}
	for i in 10:
		var zx := best.x - 140.0 + float(i) * 28.0
		var info: Dictionary = sim.get_building_info_at(zx, road_z + 14.0)
		if info.size() > 1:
			grown += 1
			if sample_info.is_empty():
				sample_info = info
	print("  [live] buildings on zoned parcels: %d/10; sample: %s" % [
		grown, str(sample_info)])
	check_true("demand grows on the delivered land value", grown > 0)
	await _shot("city")

	# A strike through the game's own audio: the engine's derived voice
	# in the running scene.
	var Acoustic := preload("res://addons/2.5D_engine/evaluator/acoustic_node.gd")
	var SoundPlayer := preload("res://scripts/core/engine_sound_player.gd")
	var mats: Dictionary = Acoustic.materials()
	var mat_names := mats.keys()
	mat_names.sort()
	var sp := SoundPlayer.new()
	get_root().add_child(sp)
	await process_frame
	sp.strike({mat_names[0]: 1.0}, {"form": "rod", "length": 0.8, "radius": 0.03})
	await _frames(10)
	check_true("a strike plays through the game's audio", sp._player.playing)

	await _shot("live")
	print("=== %d checks, %d failures ===" % [checks, failures])
	load("res://scripts/core/spike_stats.gd").record("spike_engine_live",
		checks - failures, failures, {"note": "windowed, real Main scene"})
	quit(1 if failures > 0 else 0)
