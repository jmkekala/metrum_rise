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
## (`far200+no_trees`): what a class costs inside a shorter range. The game casts the moving cars
## and the walkers from the StreamNode's shadow-only twins holding only those that can shade the
## shadowed ground (`RENDER-15`, round R47); `agents_all` casts them from their whole bucket nodes
## instead, every agent in every split as before R47 (SH2's `full`): the twins' saving against
## it. On a tree without the twins `agents_all` is `full`. Benchmark-only: nothing in the game
## calls it.
extends RefCounted

const NAMES: Array[String] = ["full", "none", "filter2", "filter3", "no_pcss", "no_blend", "low",
	"medium", "medium_blend", "no_cloud_shadows", "no_casters", "no_buildings", "no_trees",
	"no_cars", "no_walkers", "no_parked", "no_furniture", "agents_all"]
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
# Whether `agents_all` applies.
static var _agents_all := false

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
	# The bucket nodes take the agents' casters over; a class switch on them would fight it.
	return qualities <= 1 and not (parts.has("agents_all") and (parts.has("no_cars") or parts.has("no_walkers")))

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
	if parts.has("agents_all") != _agents_all:
		_agents_all = parts.has("agents_all")
		_set_agents_all(main, _agents_all)

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

# `agents_all` on or off: the twins stop or start drawing and the agents' bucket nodes cast or not
# (`agents.gd` turned them off when the twins took over). Nothing on a tree without the twins.
static func _set_agents_all(main: Node, on: bool) -> void:
	var stream := main.get_node("StreamNode")
	if not stream.has_method("set_agent_casters_enabled"):
		return
	stream.set_agent_casters_enabled(not on)
	var agents := main.get_node("Agents")
	for buckets: Dictionary in [agents.walker_mmis, agents.car_mmis]:
		for mmi: MultiMeshInstance3D in buckets.values():
			mmi.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_ON if on else GeometryInstance3D.SHADOW_CASTING_SETTING_OFF

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
		"cars", "walkers":
			# The agents' twins (`RENDER-15`, R47); their bucket nodes cast nothing.
			var stream := main.get_node("StreamNode")
			if stream.has_method("set_agent_cast_shadows"):
				stream.set_agent_cast_shadows(0 if name == "walkers" else 1, cast)
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
