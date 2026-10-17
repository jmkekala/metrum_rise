# SPDX-License-Identifier: GPL-2.0-only

## Real headlights for the cars nearest the camera at night.
##
## Rust method called: nearest_car_poses() (from agents.gd's drawn car buffers each frame).
## A fixed pool of spot lights follows the `POOL_SIZE` drawn cars nearest the camera within
## `POOL_RADIUS_M`, so near traffic lights trees, walls, walkers and the road by their own colour.
## Cars beyond fade over to the cheap light volume of vehicle_headlight_beam.gdshader, which
## fades its headlight term in over the same band. Like the street lights, Clustered Forward+ makes
## the pool's cost nearly flat in its size once any light is on screen.
extends Node3D

const SceneLighting := preload("res://scripts/core/scene_lighting.gd")
const VehicleLamps := preload("res://scripts/renderers/vehicle_lamps.gd")

const POOL_SIZE := 12
const POOL_RADIUS_M := 100.0
# Last stretch of the radius over which a light fades out (vehicle_headlight_beam.gdshader fades
# its headlight term in over the same band).
const POOL_FADE_M := 30.0
# The `SHADOWED` nearest lights (slots 0..3: nearest_car_poses() is nearest first) cast
# shadows, which is what lets Godot apply a projector texture at all (`SpotLight3D` warns
# "Projector texture only works with shadows active"; storage_rd/light_storage.cpp only builds
# the projector matrix for a shadowed light). Their projector is the far beam's own low-beam
# pattern (hot spot, broad foreground, cut-off just above the horizon), baked by
# tools/bake_headlight_projector.py in the spot's angular space. The rest are a plain cone.
# Off for now: on the M2 Pro (release, no-headlights.sqlite at 23:00, three views, 120 frames,
# alternated twice) four shadowed lights cost 1.9-2.9 ms a frame and two 1.3-2.1 ms, and hard
# positional shadows or an empty caster mask save little, so the cost is per-pixel shading, not
# drawing casters. The shaped beam looks clearly better close up; screenshots in
# screenshots/2026-10-09-roads/headlights-*.png.
const SHADOWED := 0
const PROJECTOR := preload("res://assets/textures/vehicles/headlight_projector.png")
# Calibrated against vehicle_headlight_beam.gdshader (tools/bake_headlight_projector.py, linear,
# flat road, albedo 0.25, both far lamps): a shadowed light at energy 3460, inverse-square decay
# and 60 m range adds within -4%/+4% of the far beam 10 to 25 m ahead and 0 to 8 m aside,
# 80% at 35 m; Godot's range window halves it by 45 m.
#   ahead/aside m   beam    spot*0.25
#   15 / 0          0.108   0.112
#   20 / 4          0.045   0.045
#   25 / 8          0.019   0.018
const RANGE_M := 60.0
const DECAY := 2.0
const BEAM_ENERGY := 3460.0
const BEAM_ANGLE_DEG := 60.0
const BEAM_ANGLE_ATTENUATION := 0.02
# The unshadowed cone has no cut-off or foreground falloff, so it is aimed level, its rim softened
# by a steep angle falloff, and run at about a third of the beam's peak: it matches the beam's hot
# spot around 20 m and overlights the first few metres.
const CONE_ENERGY := 1200.0
const CONE_ANGLE_DEG := 35.0
const CONE_ANGLE_ATTENUATION := 3.0
const COLOR := Color(1.0, 0.93, 0.8)
# Sun elevations in degrees over which the lamps come on (matches the street lights).
const LAMPS_ON_ELEVATION_DEG := Vector2(4.0, -2.0)

# Key: car bucket key (vehicle_type * 10 + variant), Value: headlight centre in the car's space.
var head_points: Dictionary = {}
var _pool: Array[SpotLight3D] = []
var _lit := false


func _ready() -> void:
	for i in POOL_SIZE:
		var light := SpotLight3D.new()
		light.name = "Headlight%d" % i
		light.visible = false
		var shadowed := i < SHADOWED
		light.shadow_enabled = shadowed
		# Godot feeds every light into volumetric fog by default, a fixed cost per frame.
		light.light_volumetric_fog_energy = 0.0
		light.light_color = COLOR
		light.light_specular = 0.3
		light.spot_range = RANGE_M
		light.spot_attenuation = DECAY
		light.spot_angle = BEAM_ANGLE_DEG if shadowed else CONE_ANGLE_DEG
		light.spot_angle_attenuation = BEAM_ANGLE_ATTENUATION if shadowed else CONE_ANGLE_ATTENUATION
		if shadowed:
			light.light_projector = PROJECTOR
		# Fade from the car origin below, as the far beam does; the engine's built-in
		# distance fade measures the displaced lamp origin instead.
		light.distance_fade_enabled = false
		add_child(light)
		_pool.append(light)


## Registers where a car model's headlights sit, from its mesh (vehicle_lamps.gd metadata).
func register_model(bucket_key: int, mesh: Mesh) -> void:
	if mesh.has_meta(VehicleLamps.HEAD_LAMP_META):
		var lamp: Vector3 = mesh.get_meta(VehicleLamps.HEAD_LAMP_META)
		# Centred between the lamps, just ahead of the lens so the light misses the car's own nose.
		head_points[bucket_key] = Vector3(0.0, lamp.y, lamp.z - 0.15)


func update(simulation_node: Object, car_buffers: Dictionary) -> void:
	var elevation := SceneLighting.current_sun_elevation_deg
	var night := smoothstep(LAMPS_ON_ELEVATION_DEG.x, LAMPS_ON_ELEVATION_DEG.y, elevation)
	var camera := get_viewport().get_camera_3d() if is_inside_tree() else null
	if night <= 0.0 or camera == null:
		if _lit:
			for light in _pool:
				light.visible = false
			_lit = false
		return
	_lit = true
	# 13 floats per car: the row-major 3x4 MultiMesh transform, then its bucket key.
	var cars: PackedFloat32Array = simulation_node.nearest_car_poses(
		car_buffers, camera.global_position, POOL_RADIUS_M, POOL_SIZE
	)
	var count := cars.size() / 13
	for i in _pool.size():
		var light := _pool[i]
		var key := int(cars[i * 13 + 12]) if i < count else -1
		if not head_points.has(key):
			light.visible = false
			continue
		var b := i * 13
		var car := Transform3D(
			Vector3(cars[b], cars[b + 4], cars[b + 8]),
			Vector3(cars[b + 1], cars[b + 5], cars[b + 9]),
			Vector3(cars[b + 2], cars[b + 6], cars[b + 10]),
			Vector3(cars[b + 3], cars[b + 7], cars[b + 11])
		)
		# Spot lights shine along their -Z, as cars face. The projector carries the beam's dip.
		light.global_transform = car * Transform3D(Basis.IDENTITY, head_points[key])
		var distance_m := camera.global_position.distance_to(car.origin)
		var near_strength := 1.0 - smoothstep(POOL_RADIUS_M - POOL_FADE_M, POOL_RADIUS_M, distance_m)
		var energy := BEAM_ENERGY if i < SHADOWED else CONE_ENERGY
		light.light_energy = energy * night * near_strength
		light.visible = true
