# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: engine_tick.gd
#  script_path: scripts/core/engine_tick.gd
#  module_name: engine_tick
#  version: 0.9.0
#  author: [BantedHam]
#  description: The harness that makes the sources play: world time
#           advances here, the tide breathes into the water level, the
#           bus pump runs so gateway traffic delivers, and the ambient
#           conditions at the listener are published each tick for
#           whoever consumes them. Autoloaded; the sources it drives
#           are the ones the drills already proved.
#  kind: module
#  spec: none
#  internal_dependencies: [scripts/core/engine_water_source.gd,
#           scripts/core/engine_weather_source.gd,
#           scripts/core/rust_gateway.gd]
#  external_dependencies: [Godot 4.x]
#  features: [world-time, tide-breathing, bus-pump, listener-conditions]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## The heartbeat wiring sources into the running game.
extends Node

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const EngineWaterSource := preload("res://scripts/core/engine_water_source.gd")
const EngineWeatherSource := preload("res://scripts/core/engine_weather_source.gd")
const EngineDirectorSource := preload("res://scripts/core/engine_director_source.gd")
const RustGateway := preload("res://scripts/core/rust_gateway.gd")

## Seconds of world time per real second; tides need hours to breathe.
const TIME_SCALE := 600.0

var world_t := 0.0
var listener := Vector3.ZERO
var conditions: Dictionary = {}
var gateway: Node = null
## The last drained batch, held for the Rust intake when it lands: the
## director has observed it; the core consumes it next.
var last_batch: Array = []
## The director's current read on the city, refreshed every tick.
var pacing: Dictionary = {}

const EngineSocialSource := preload("res://scripts/core/engine_social_source.gd")
const EngineMineralSource := preload("res://scripts/core/engine_mineral_source.gd")
const EngineMindSource := preload("res://scripts/core/engine_mind_source.gd")
const EngineTerrainSource := preload("res://scripts/core/engine_terrain_source.gd")
const EnginePixelate := preload("res://addons/2.5D_engine/evaluator/pixelate_node.gd")
const EngineBoundary := preload("res://scripts/core/engine_boundary.gd")

## Ticks between deliveries into the core; the evaluated ground moves
## slowly and habitability is real work per parcel.
const DELIVERY_INTERVAL_TICKS := 120
## The probe grid: sized to the city's parcel bounds when a city
## exists, centred on the listener before one does.
const PROBE_SIDE := 5
const PROBE_SPACING_M := 200.0
## Coverage beyond the outermost parcel, so growth at the edge still
## lands inside delivered coverage instead of the flat fallback.
const PROBE_MARGIN_M := 200.0
## A one-parcel city has a zero-size bounding box; the grid never
## collapses below this spacing.
const PROBE_MIN_SPACING_M := 50.0

var _delivery_countdown := 1
## The core's last confirmed intake revision, for anyone who asks.
var intake_revision := 0
## Ground reconciliation is once per world: the sim derives untouched
## terrain from the twin, sculpts stay measured, then this holds true
## until a world load resets it through reapply_ground().
var _ground_applied := false

## The game scene; ground derivation never runs in the editors, where a
## filled cell would export as authored world data.
const GAME_SCENE := "res://scenes/Main.tscn"


# =========================================================================
# REAPPLY GROUND
# =========================================================================
## A world load calls this so the fresh world's untouched cells derive.
func reapply_ground() -> void:
	_ground_applied = false

# =========================================================================
# IN GAME SCENE
# =========================================================================
func _in_game_scene() -> bool:
	var scene := get_tree().current_scene
	return scene != null and scene.scene_file_path == GAME_SCENE


# =========================================================================
# APPLY ENGINE GROUND
# =========================================================================
## Reconcile the sim's ground with the drawn field: untouched samples
## derive from the bit-exact twin using the renderer's own parameters,
## so slopes, placement, and clipping agree with what the player sees.
func _apply_engine_ground() -> void:
	var sim := get_tree().root.find_child("SimulationNode", true, false)
	if sim == null or not sim.has_method("apply_engine_ground"):
		return
	var filled := int(sim.call("apply_engine_ground",
		EngineTerrainSource.FOOTPRINT, 0.0,
		EngineTerrainSource.FIELD_SEED, EngineTerrainSource.AMPLITUDE_M,
		EngineTerrainSource.WORLD_SCALE_M))
	_ground_applied = true
	print("EngineTick: engine ground applied, %d samples filled" % filled)

# =========================================================================
# READY
# =========================================================================
func _ready() -> void:
	gateway = RustGateway.new()
	add_child(gateway)
	# Pixel art all the way on, applied as render scale until SPEC
	# 13.9's dial is built: one art pixel per pixel_size screen pixels,
	# the knob read from the pixelate node's own config. Also the
	# largest performance lever this box has, which is why it is on now.
	var vp := get_viewport()
	if vp != null:
		vp.scaling_3d_mode = Viewport.SCALING_3D_MODE_BILINEAR
		vp.scaling_3d_scale = 1.0 / float(
			EnginePixelate.default_config()["pixel_size"])


# =========================================================================
# DELIVER ENGINE INPUTS
# =========================================================================
## Hand the core everything evaluated, one call: desirability and the
## mineral channels on the probe grid.
func _deliver_engine_inputs() -> void:
	var sim := get_tree().root.find_child("SimulationNode", true, false)
	if sim == null or not sim.has_method("set_engine_inputs"):
		return
	# The grid covers the city when parcels exist, the listener's
	# surroundings before any city does.
	var half := float(PROBE_SIDE - 1) * 0.5
	var origin := listener + Vector3(-half * PROBE_SPACING_M, 0.0,
		-half * PROBE_SPACING_M)
	var spacing := PROBE_SPACING_M
	if sim.has_method("engine_parcel_bounds"):
		var bounds: Dictionary = sim.call("engine_parcel_bounds")
		if int(bounds.get("count", 0)) > 0:
			var min_x := float(bounds["min_x"]) - PROBE_MARGIN_M
			var min_z := float(bounds["min_z"]) - PROBE_MARGIN_M
			var span := maxf(
				float(bounds["max_x"]) + PROBE_MARGIN_M - min_x,
				float(bounds["max_z"]) + PROBE_MARGIN_M - min_z)
			spacing = maxf(span / float(PROBE_SIDE - 1), PROBE_MIN_SPACING_M)
			origin = Vector3(min_x, 0.0, min_z)
	var positions := PackedVector3Array()
	for row in PROBE_SIDE:
		for col in PROBE_SIDE:
			positions.append(origin + Vector3(
				float(col) * spacing, 0.0, float(row) * spacing))
	var desirability := EngineSocialSource.desirability(positions,
		fmod(world_t / 31557600.0, 1.0))
	var iron := EngineMineralSource.richness("iron", positions)
	var coal := EngineMineralSource.richness("coal", positions)
	var stone := EngineMineralSource.richness("stone", positions)
	# The probe grid's world geometry rides with the values, so the core
	# can place every sample and refuse coverage it was never given. The
	# policy channel is gone with the layer that produced it: minds are
	# living instances now, and the actor path consumes them directly.
	intake_revision = int(sim.call("set_engine_inputs",
		PackedFloat64Array(desirability), PackedFloat64Array(iron),
		PackedFloat64Array(coal), PackedFloat64Array(stone),
		origin.x, origin.z, spacing, PROBE_SIDE))
	# The upward half of the boundary: every extraction site's depletion
	# aggregates into a deposit grid the engine opens as measured rows.
	if sim.has_method("get_extractor_sites"):
		_write_mining_deposit(sim.call("get_extractor_sites"))
	# The director paces arrivals through the game's own dial: the
	# population multiplier becomes border openness, a frequency of
	# arrivals and never their strength, which is the design law the
	# director node itself states. Build-up admits everyone, the peak
	# saturates the cap, the fade closes the gate, relax reopens it.
	if sim.has_method("set_economy_policy_value") and not pacing.is_empty():
		var openness := clampf(
			float(pacing.get("population", 100)) / 100.0, 0.0, 1.0)
		sim.set_economy_policy_value("border_openness", openness)


## Deposit grid geometry: extraction aggregates per hundred-metre cell,
## and world metres map to degrees at one arc-minute per cell, the same
## convention the boundary fixtures use; the mapping is authored until
## game worlds carry a real geographic anchor. The engine reads rasters
## north-up: latitude DECREASES with row, so a reader queries
## origin_lat minus (row + half) times the pixel, exactly as the
## boundary spike's fixture check does.
const DEPOSIT_CELL_M := 100.0
const DEPOSIT_DIR := "user://deposits"
## The last written deposit path, empty until mining has happened; the
## drills read it back through the engine's own heightmap node.
var last_deposit_path := ""

# =========================================================================
# WRITE MINING DEPOSIT
# =========================================================================
func _write_mining_deposit(sites: Array) -> void:
	var min_x := INF
	var min_z := INF
	var max_x := -INF
	var max_z := -INF
	var any := false
	for s in sites:
		var sd := s as Dictionary
		if float(sd.get("extracted_units", 0.0)) <= 0.0:
			continue
		any = true
		min_x = minf(min_x, float(sd["x"]))
		max_x = maxf(max_x, float(sd["x"]))
		min_z = minf(min_z, float(sd["z"]))
		max_z = maxf(max_z, float(sd["z"]))
	if not any:
		return
	var w := int(ceilf((max_x - min_x) / DEPOSIT_CELL_M)) + 3
	var h := int(ceilf((max_z - min_z) / DEPOSIT_CELL_M)) + 3
	var grid := PackedFloat32Array()
	grid.resize(w * h)
	grid.fill(0.0)
	var ox := min_x - DEPOSIT_CELL_M
	var oz := min_z - DEPOSIT_CELL_M
	for s in sites:
		var sd := s as Dictionary
		var units := float(sd.get("extracted_units", 0.0))
		if units <= 0.0:
			continue
		var cx := int((float(sd["x"]) - ox) / DEPOSIT_CELL_M)
		var cz := int((float(sd["z"]) - oz) / DEPOSIT_CELL_M)
		grid[clampi(cz, 0, h - 1) * w + clampi(cx, 0, w - 1)] += units
	DirAccess.make_dir_recursive_absolute(
		ProjectSettings.globalize_path(DEPOSIT_DIR))
	var path := ProjectSettings.globalize_path(
		DEPOSIT_DIR + "/mining_extraction.raw")
	if EngineBoundary.write_deposit(grid, w, h,
			ox / DEPOSIT_CELL_M / 60.0, oz / DEPOSIT_CELL_M / 60.0,
			1.0 / 60.0, path):
		last_deposit_path = path

# =========================================================================
# PROCESS
# =========================================================================
func _process(delta: float) -> void:
	world_t += delta * TIME_SCALE
	# The tide's clock is the water source's clock.
	EngineWaterSource.tide_time = world_t
	# The sky at the listener, published once per tick for ambience and UI.
	conditions = EngineWeatherSource.conditions(listener.x, listener.z,
		fmod(world_t / 31557600.0, 1.0))
	# Delivery is the pump's job; without it the bus queues forever.
	var bus := get_node_or_null("/root/GoatBusSystem")
	if bus != null and bus.has_method("process_queued_events"):
		bus.process_queued_events()
	# The director watches the same traffic the core will consume, and
	# paces on real seconds, because drama is felt in real time.
	last_batch = gateway.drain_inbound()
	pacing = EngineDirectorSource.step(last_batch, delta)
	# The mind roster advances on the node's own fixed step; empty until
	# the actor path spawns bodies, and free while it is.
	EngineMindSource.tick_all(delta)
	# Ground reconciliation, once per world, as soon as the core is
	# mounted, the workflow toggle says the field is what the player
	# sees, and this is the game, not an editor.
	if not _ground_applied and EngineTerrainSource.enabled() and _in_game_scene():
		_apply_engine_ground()
	# The intake: evaluated arrays into the core on an interval, because
	# habitability per parcel is real work and the ground shifts slowly.
	_delivery_countdown -= 1
	if _delivery_countdown <= 0:
		_delivery_countdown = DELIVERY_INTERVAL_TICKS
		_deliver_engine_inputs()
