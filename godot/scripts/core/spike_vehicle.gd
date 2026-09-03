# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: spike_vehicle.gd
#  script_path: scripts/core/spike_vehicle.gd
#  module_name: spike_vehicle
#  version: 0.1.1
#  author: [BantedHam]
#  description: Drills the vehicle source: throttle pulls and more
#           throttle pulls harder, the friction circle clips excessive
#           demand to the tire's own budget, corner demand grows with
#           speed and shrinks with radius, and the rollover check trips
#           on the tall-and-fast case, all on the node's laws.
#  kind: spike
#  spec: none
#  internal_dependencies: [scripts/core/engine_vehicle_source.gd]
#  external_dependencies: [Godot 4.x]
#  features: [vehicle-drill]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## Run: godot --headless --path godot --script scripts/core/spike_vehicle.gd
extends SceneTree

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const VehicleSource := preload("res://scripts/core/engine_vehicle_source.gd")

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
# INIT
# =========================================================================
func _init() -> void:
	var half := VehicleSource.pull_force(0.5, 1, 3000.0)
	var full := VehicleSource.pull_force(1.0, 1, 3000.0)
	_check("throttle pulls", full > 0.0)
	_check("more throttle pulls harder", full > half)
	_check("pull is deterministic",
		VehicleSource.pull_force(1.0, 1, 3000.0) == full)

	var inside := VehicleSource.tire_budget(1000.0, 1000.0, 5000.0)
	_check("demand inside the circle passes untouched",
		not bool(inside["clipped"]) and float(inside["fx"]) == 1000.0)
	var outside := VehicleSource.tire_budget(4000.0, 4000.0, 5000.0)
	_check("excess demand clips to the budget", bool(outside["clipped"]))
	var mag := sqrt(pow(float(outside["fx"]), 2) + pow(float(outside["fy"]), 2))
	_check("the clipped force sits ON the circle", absf(mag - 5000.0) < 0.01)

	var slow := VehicleSource.corner_demand(10.0, 50.0)
	var fast := VehicleSource.corner_demand(30.0, 50.0)
	var wide := VehicleSource.corner_demand(30.0, 200.0)
	_check("speed raises corner demand", fast > slow)
	_check("a wider bend asks less", wide < fast)

	var sedan_ok := VehicleSource.rolls(1.6, 0.5, 15.0, 30.0)
	var truck_fast := VehicleSource.rolls(1.6, 1.4, 30.0, 15.0)
	_check("a sedan takes the bend", not sedan_ok)
	_check("tall and fast into a tight bend rolls", truck_fast)

	print("=== %d passed, %d failed ===" % [passed, failed])
	load("res://scripts/core/spike_stats.gd").record("spike_vehicle", passed, failed)
	quit(1 if failed > 0 else 0)
