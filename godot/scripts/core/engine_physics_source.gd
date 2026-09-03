# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: engine_physics_source.gd
#  script_path: scripts/core/engine_physics_source.gd
#  module_name: engine_physics_source
#  version: 0.1.0
#  author: [BantedHam]
#  description: Ground contact from the evaluated field, no collision
#           meshes anywhere: height is the field, the normal is its
#           gradient by central differences, and slope falls out of the
#           normal. The same ground the renderers draw is the ground
#           things stand on, which is the engine's no-collision-mesh
#           doctrine consumed at the boundary.
#  kind: module
#  spec: none
#  internal_dependencies: [scripts/core/engine_network_source.gd]
#  external_dependencies: [Godot 4.x]
#  features: [ground-contact, field-normals]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## Contact with the evaluated ground.
@tool
extends RefCounted

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const EngineNetworkSource := preload("res://scripts/core/engine_network_source.gd")

## Half the step of the central difference, in world metres.
const EPS := 0.05

# =========================================================================
# HEIGHT
# =========================================================================
static func height(wx: float, wz: float) -> float:
	return EngineNetworkSource.ground_height(wx, wz)


# =========================================================================
# NORMAL
# =========================================================================
## The surface normal by central differences on the field itself.
static func normal(wx: float, wz: float) -> Vector3:
	var hx := height(wx + EPS, wz) - height(wx - EPS, wz)
	var hz := height(wx, wz + EPS) - height(wx, wz - EPS)
	return Vector3(-hx, 2.0 * EPS, -hz).normalized()


# =========================================================================
# CONTACT
# =========================================================================
## Everything a body needs to stand: where the ground is, which way it
## faces, and how steep it is in degrees.
static func contact(wx: float, wz: float) -> Dictionary:
	var n := normal(wx, wz)
	return {
		"height": height(wx, wz),
		"normal": n,
		"slope_deg": rad_to_deg(acos(clampf(n.y, -1.0, 1.0))),
	}
