# SPDX-License-Identifier: GPL-2.0-only

## The sun-shadow variants the idle and stream benchmarks measure (`RENDER-15`). Every variant
## starts from the High setting as `SceneLighting.apply_shadow_quality` sets it, then removes or
## cheapens one part: `none` the sun's shadow, `filter2` / `filter3` the soft filter at quality 2
## or 3 (High is the project's 4), `no_pcss` the variable penumbra (angular distance 0, a plain
## PCF lookup), `no_blend` the blending between splits, `low` and `medium` the game's Low and
## Medium settings, `medium_blend` Medium with split blending, `no_cloud_shadows` the
## receivers' cloud lookup, and `no_casters` every caster (the light's `shadow_caster_mask` 0): the
## shadow maps draw nothing while the receivers still read them, so against `full` it is the
## drawing plus the penumbra filter the blocker search then skips.
##
## The casters by class and distance (SH2): `no_<class>` stops one class casting while it still
## draws (`buildings`: the building batches; `trees`: every tree, rock and plant caster, the
## vegetation's own toggle; `cars`: the moving cars; `walkers`; `parked`: the parked cars;
## `furniture`: road signs and street lights). `far<m>` ends the sun's shadow at that many metres
## instead of SceneLighting's 420, its four splits at the same shares of it: the casters and the
## receivers beyond it go. Parts join with `+` into one variant, at most one quality among them
## (`far200+no_trees`): what a class costs inside a shorter range. `agents_near` casts the moving
## cars and the walkers from shadow-only twins holding only those that can shade the visible
## ground (`CasterTwins`), instead of the whole city's agents in every split: the same shadows
## for less drawing. Benchmark-only: nothing in the game calls it.
extends RefCounted

const NAMES: Array[String] = ["full", "none", "filter2", "filter3", "no_pcss", "no_blend", "low",
	"medium", "medium_blend", "no_cloud_shadows", "no_casters", "no_buildings", "no_trees",
	"no_cars", "no_walkers", "no_parked", "no_furniture", "agents_near"]
const CLASSES: Array[String] = ["buildings", "trees", "cars", "walkers", "parked", "furniture"]
# The node renderers' shadow policy categories (`SceneLighting.apply_shadow_policy`) per class;
# the building batches, the parked cars and the vegetation draw through Rust and have toggles.
const NODE_CATEGORIES := {
	"buildings": ["buildings"],
	"cars": ["cars"],
	"walkers": ["walkers"],
	"furniture": ["road_signs", "street_lights"],
}
# `GameSettings.SHADOW_QUALITY_*`, by value: this script is copied over older trees' scripts.
const QUALITY_HIGH := 0
const QUALITY_LOW := 1
const QUALITY_MEDIUM := 2
# Light3D's default caster mask: every layer.
const ALL_LAYERS := 0xFFFFFFFF
# The node meta keeping a node caster's own setting while its class is off.
const SAVED_CAST_META := &"shadow_variant_cast"

# The classes off as the last variant left them, so a variant changes only what differs.
static var _classes_off: Array[String] = []
# `agents_near`'s twins while it applies.
static var _twins: CasterTwins = null

## Whether `variant` names a variant: parts of NAMES or `far<metres>` joined by `+`.
static func is_known(variant: String) -> bool:
	var qualities := 0
	for part in variant.split("+"):
		if part.begins_with("far"):
			if not part.trim_prefix("far").is_valid_int() or part.trim_prefix("far").to_int() <= 0:
				return false
			continue
		if not NAMES.has(part):
			return false
		qualities += int(part in ["low", "medium", "medium_blend"])
	var parts := variant.split("+")
	# The twins take the agents' casters over; a class switch on them would fight it.
	return qualities <= 1 and not (parts.has("agents_near") and (parts.has("no_cars") or parts.has("no_walkers")))

## Applies `variant` (see `is_known`; unknown parts are ignored, so a name that is no variant is
## `full`). `lighting` is the SceneLighting node, untyped so a benchmark copied over an older tree's
## scripts still loads.
static func apply(lighting: Variant, sun: DirectionalLight3D, variant: String) -> void:
	var parts := variant.split("+")
	var quality := QUALITY_HIGH
	if parts.has("low"):
		quality = QUALITY_LOW
	elif parts.has("medium") or parts.has("medium_blend"):
		quality = QUALITY_MEDIUM
	# A tree from before Medium takes High or Low as a bool.
	var takes_bool := false
	for method: Dictionary in lighting.get_script().get_script_method_list():
		if method.name == "apply_shadow_quality":
			takes_bool = method.args[1].type == TYPE_BOOL
	lighting.apply_shadow_quality(sun, quality == QUALITY_LOW if takes_bool else quality)
	sun.shadow_enabled = not parts.has("none")
	sun.shadow_caster_mask = 0 if parts.has("no_casters") else ALL_LAYERS
	lighting.cloud_shadows_allowed = not parts.has("no_cloud_shadows")
	var far_m: float = lighting.shadow_max_distance_m()
	for part in parts:
		match part:
			"filter2":
				RenderingServer.directional_soft_shadow_filter_set_quality(RenderingServer.SHADOW_QUALITY_SOFT_LOW)
			"filter3":
				RenderingServer.directional_soft_shadow_filter_set_quality(RenderingServer.SHADOW_QUALITY_SOFT_MEDIUM)
			"no_pcss":
				sun.light_angular_distance = 0.0
			"no_blend":
				sun.directional_shadow_blend_splits = false
			"medium_blend":
				sun.directional_shadow_blend_splits = true
			_:
				if part.begins_with("far") and part.trim_prefix("far").is_valid_int():
					far_m = float(part.trim_prefix("far").to_int())
	sun.directional_shadow_max_distance = far_m
	var main: Node = lighting.get_parent()
	var off: Array[String] = []
	for name in CLASSES:
		if parts.has("no_" + name):
			off.append(name)
	if off != _classes_off:
		for name in CLASSES:
			if off.has(name) != _classes_off.has(name):
				_set_class_casts(main, name, not off.has(name))
		_classes_off = off
	if parts.has("agents_near") != (_twins != null):
		if _twins == null:
			_twins = CasterTwins.new(sun)
			main.add_child(_twins)
		else:
			_twins.restore()
			_twins.free()
			_twins = null

## The node casters still casting, by shadow policy category: `[nodes, instances, instances
## within `range_m` of `eye`]`, a MultiMesh's drawn instances counted from its buffer (the
## benchmark's caster census at the settled pose; the Rust layers' batches are not nodes).
static func node_casters(root: Node, eye: Vector3, range_m: float) -> Dictionary:
	var out := {}
	for node: Node in root.find_children("*", "GeometryInstance3D", true, false):
		var instance := node as GeometryInstance3D
		if instance.cast_shadow == GeometryInstance3D.SHADOW_CASTING_SETTING_OFF or not instance.is_visible_in_tree():
			continue
		var category := str(instance.get_meta("shadow_policy_category", "unregistered"))
		var counts: Array = out.get(category, [0, 0, 0])
		counts[0] += 1
		var multimesh: MultiMesh = (instance as MultiMeshInstance3D).multimesh if instance is MultiMeshInstance3D else null
		if multimesh == null:
			counts[1] += 1
			counts[2] += int(instance.global_position.distance_to(eye) <= range_m)
		elif multimesh.transform_format == MultiMesh.TRANSFORM_3D:
			var drawn := multimesh.instance_count if multimesh.visible_instance_count < 0 else multimesh.visible_instance_count
			# 12 floats of transform (origin at 3, 7, 11), then the colour and custom data.
			var stride := 12 + 4 * int(multimesh.use_colors) + 4 * int(multimesh.use_custom_data)
			var buffer := multimesh.buffer
			var to_world := instance.global_transform
			counts[1] += drawn
			for i in drawn:
				var at := i * stride
				var origin := to_world * Vector3(buffer[at + 3], buffer[at + 7], buffer[at + 11])
				counts[2] += int(origin.distance_to(eye) <= range_m)
		out[category] = counts
	return out

# Turns one class's casting off or back on: the Rust layers through their toggles, the node
# renderers' casters through their own setting, kept on the node while off.
static func _set_class_casts(main: Node, name: String, cast: bool) -> void:
	match name:
		"buildings":
			main.get_node("SimulationNode").set_building_cast_shadows(cast)
		"trees":
			main.get_node("Vegetation").set_cast_shadows(cast)
		"parked":
			main.get_node("StreamNode").set_parked_cast_shadows(cast)
	var categories: Array = NODE_CATEGORIES.get(name, [])
	if categories.is_empty():
		return
	for node: Node in main.find_children("*", "GeometryInstance3D", true, false):
		var instance := node as GeometryInstance3D
		if not categories.has(str(instance.get_meta("shadow_policy_category", ""))):
			continue
		if not cast and not instance.has_meta(SAVED_CAST_META):
			instance.set_meta(SAVED_CAST_META, instance.cast_shadow)
			instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
		elif cast and instance.has_meta(SAVED_CAST_META):
			instance.cast_shadow = instance.get_meta(SAVED_CAST_META)
			instance.remove_meta(SAVED_CAST_META)

## `agents_near`: every moving car and walker caster (`agents.gd`'s buckets) casts nothing itself
## and gets a shadow-only twin sharing its mesh and material, refilled each frame from the
## frame's buffers `agents.gd` keeps (this node is the last child of Main, so it runs after them;
## reading a MultiMesh's buffer back stalls the GPU), with only the agents that can shade ground
## inside the sun's shadow range: view depth within the range plus TWIN_MARGIN_M either way, and
## distance within TWIN_RADIAL_FACTOR times the range plus the margin (a split's corners lie up to
## 1.86 times its depth away at 75 degrees of vertical field and 16:9). The margin covers the
## longest shadow a 2 m agent throws with the sun above 4 degrees. `O(agents)` a frame in
## GDScript: the frame time carries it, the GPU time does not.
class CasterTwins extends Node:
	const TWIN_MARGIN_M := 30.0
	const TWIN_RADIAL_FACTOR := 1.9
	var _sun: DirectionalLight3D
	var _agents: Node
	# [source MultiMeshInstance3D, its twin, "walker_buffers" or "car_buffers", bucket key, the
	# bucket's last rows] per bucket.
	var _pairs: Array = []

	func _init(sun: DirectionalLight3D) -> void:
		_sun = sun
		name = "ShadowCasterTwins"

	func _ready() -> void:
		_agents = get_parent().get_node("Agents")
		for maps: Array in [["walker_buffers", _agents.walker_mmis], ["car_buffers", _agents.car_mmis]]:
			for key in maps[1]:
				var source: MultiMeshInstance3D = maps[1][key]
				var multimesh := MultiMesh.new()
				multimesh.transform_format = source.multimesh.transform_format
				multimesh.use_colors = source.multimesh.use_colors
				multimesh.use_custom_data = source.multimesh.use_custom_data
				multimesh.mesh = source.multimesh.mesh
				var twin := MultiMeshInstance3D.new()
				twin.multimesh = multimesh
				twin.material_override = source.material_override
				twin.extra_cull_margin = source.extra_cull_margin
				twin.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_SHADOWS_ONLY
				source.add_sibling(twin)
				twin.global_transform = source.global_transform
				source.set_meta(SAVED_CAST_META, source.cast_shadow)
				source.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
				_pairs.append([source, twin, maps[0], key, PackedFloat32Array()])

	func _process(_delta: float) -> void:
		var camera := get_viewport().get_camera_3d()
		if camera == null:
			return
		var eye := camera.global_position
		var forward := -camera.global_basis.z
		var reach := _sun.directional_shadow_max_distance
		for pair: Array in _pairs:
			var buffers: Dictionary = _agents.get(pair[2])
			# A bucket the frame did not send keeps its rows; one its node draws none of is empty.
			if buffers.has(pair[3]):
				pair[4] = buffers[pair[3]]
			if (pair[0] as MultiMeshInstance3D).multimesh.instance_count == 0:
				pair[4] = PackedFloat32Array()
			_refill(pair[1], pair[4], eye, forward, reach)

	## Gives the casters back their own shadow and frees the twins.
	func restore() -> void:
		for pair: Array in _pairs:
			var source: MultiMeshInstance3D = pair[0]
			source.cast_shadow = source.get_meta(SAVED_CAST_META)
			source.remove_meta(SAVED_CAST_META)
			pair[1].free()
		_pairs.clear()

	func _refill(twin: MultiMeshInstance3D, rows: PackedFloat32Array, eye: Vector3, forward: Vector3,
			reach: float) -> void:
		var to := twin.multimesh
		var stride := 12 + 4 * int(to.use_colors) + 4 * int(to.use_custom_data)
		var count := rows.size() / stride
		if to.instance_count < count:
			to.instance_count = count
		var out := PackedFloat32Array()
		out.resize(to.instance_count * stride)
		var to_world := twin.global_transform
		var kept := 0
		for i in count:
			var at := i * stride
			var offset := to_world * Vector3(rows[at + 3], rows[at + 7], rows[at + 11]) - eye
			var depth := offset.dot(forward)
			if depth < -TWIN_MARGIN_M or depth > reach + TWIN_MARGIN_M:
				continue
			if offset.length() > reach * TWIN_RADIAL_FACTOR + TWIN_MARGIN_M:
				continue
			for k in stride:
				out[kept * stride + k] = rows[at + k]
			kept += 1
		if to.instance_count > 0:
			to.buffer = out
		to.visible_instance_count = kept
