# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: engine_vehicle_source.gd
#  script_path: scripts/core/engine_vehicle_source.gd
#  module_name: engine_vehicle_source
#  version: 0.1.0
#  author: [BantedHam]
#  description: Traffic's vehicle dynamics from the vehicle node's own
#           laws: Pacejka tires, the friction circle, the powertrain
#           curve, and the rollover check, wrapped for what the traffic
#           sim asks: how hard can this car corner here, how fast does
#           it pull away, does this bend at this speed roll it.
#  kind: module
#  spec: Pacejka magic formula and friction circle via vehicle_node
#  internal_dependencies: [addons/2.5D_engine/evaluator/vehicle_node.gd]
#  external_dependencies: [Godot 4.x]
#  features: [tires, powertrain, rollover, corner-speed]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## What a car can actually do.
@tool
extends RefCounted

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const Vehicle := preload("res://addons/2.5D_engine/evaluator/vehicle_node.gd")

static var _cfg: Dictionary = {}

# =========================================================================
# CONFIG
# =========================================================================
static func _config() -> Dictionary:
	if _cfg.is_empty():
		_cfg = Vehicle.default_config()
	return _cfg


# =========================================================================
# PULL FORCE
# =========================================================================
## The drive force at a throttle and gear, on the node's powertrain.
static func pull_force(throttle: float, gear: int, rpm: float) -> float:
	var torque := Vehicle.engine_torque(rpm, _config()) * clampf(throttle, 0.0, 1.0)
	return Vehicle.drive_force(torque, gear, _config())


# =========================================================================
# TIRE BUDGET
# =========================================================================
## Whether combined braking and cornering demand exceeds the tires, and
## what the tires actually deliver: the friction circle, verbatim.
static func tire_budget(fx: float, fy: float, mu_fz: float) -> Dictionary:
	return Vehicle.friction_circle(fx, fy, mu_fz)


# =========================================================================
# CORNER DEMAND
# =========================================================================
## The lateral acceleration a bend demands at a speed.
static func corner_demand(speed: float, radius: float) -> float:
	return Vehicle.corner_lateral_accel(speed, radius)


# =========================================================================
# ROLLS
# =========================================================================
## Does this bend at this speed roll this body. The node's own verdict:
## an untripped car slides at the grip ceiling before it tips, so tall
## bodies roll where low ones scrub wide, which is the physics traffic
## accidents actually have.
static func rolls(track_width: float, cog_height: float, speed: float,
		radius: float, mu: float = 0.9, tripped: bool = false) -> bool:
	var verdict := Vehicle.rollover(track_width, cog_height, speed, radius,
		mu, tripped)
	return bool(verdict.get("rolls", false))