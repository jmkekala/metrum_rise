## Deterministic scene lighting setup shared by gameplay and editor scenes.
##
## The node configures the existing WorldEnvironment and DirectionalLight3D
## once at scene startup, then exposes the same constants to terrain/water
## materials through static accessors.
extends Node
class_name SceneLighting

const SUN_DIRECTION := Vector3(-0.52, 0.58, -0.63)
const SUN_COLOR := Color(1.00, 0.92, 0.80, 1.0)
const SKY_COLOR := Color(0.58, 0.73, 0.86, 1.0)
const AMBIENT_COLOR := Color(0.54, 0.64, 0.70, 1.0)
const SUN_ENERGY := 1.65
const SUN_INDIRECT_ENERGY := 0.35
const AMBIENT_STRENGTH := 0.44
const SHADOW_MAX_DISTANCE_M := 520.0
const SHADOW_SPLIT_1 := 0.10
const SHADOW_SPLIT_2 := 0.28
const SHADOW_SPLIT_3 := 0.58
const SHADOW_FADE_START := 0.84
const SHADOW_BIAS := 0.035
const SHADOW_NORMAL_BIAS := 0.70
const SHADOW_BLUR := 0.85
const GROUND_SHADOW_AMBIENT := 0.64
const GROUND_SHADOW_SUN_STRENGTH := 0.36
const GROUND_SHADOW_MIN_VISIBILITY := 0.10
const STATIC_CASTER_EXTRA_CULL_MARGIN_M := 32.0
const DYNAMIC_CASTER_EXTRA_CULL_MARGIN_M := 12.0
const RECEIVER_EXTRA_CULL_MARGIN_M := 2.0

const SHADOW_STATIC_CASTER := "static_caster"
const SHADOW_DYNAMIC_CASTER := "dynamic_caster"
const SHADOW_TINY_DYNAMIC := "tiny_dynamic"
const SHADOW_RECEIVER_ONLY := "receiver_only"
const SHADOW_DEBUG_OVERLAY := "debug_overlay"

static func sun_direction() -> Vector3:
	return SUN_DIRECTION.normalized()

static func sun_color() -> Color:
	return SUN_COLOR

static func sky_color() -> Color:
	return SKY_COLOR

static func ambient_color() -> Color:
	return AMBIENT_COLOR

static func ambient_strength() -> float:
	return AMBIENT_STRENGTH

static func shadow_split_distances() -> Vector3:
	return Vector3(
		SHADOW_MAX_DISTANCE_M * SHADOW_SPLIT_1,
		SHADOW_MAX_DISTANCE_M * SHADOW_SPLIT_2,
		SHADOW_MAX_DISTANCE_M * SHADOW_SPLIT_3
	)

static func is_lighting_debug_enabled() -> bool:
	var visual_mode := OS.get_environment("METRUM_DEBUG_TERRAIN_VISUAL").strip_edges().to_lower()
	return visual_mode == "lighting" or visual_mode == "light" or visual_mode == "sun"

static func apply_ground_shadow_parameters(material: ShaderMaterial) -> void:
	if material == null:
		return
	material.set_shader_parameter("ground_shadow_ambient", GROUND_SHADOW_AMBIENT)
	material.set_shader_parameter("ground_shadow_sun_strength", GROUND_SHADOW_SUN_STRENGTH)
	material.set_shader_parameter("ground_shadow_min_visibility", GROUND_SHADOW_MIN_VISIBILITY)

static func apply_shadow_policy(
	instance: GeometryInstance3D,
	role: String,
	category: String = ""
) -> void:
	if instance == null:
		return
	match role:
		SHADOW_STATIC_CASTER, SHADOW_DYNAMIC_CASTER:
			instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_ON
		SHADOW_TINY_DYNAMIC, SHADOW_RECEIVER_ONLY, SHADOW_DEBUG_OVERLAY:
			instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
		_:
			instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	var extra_margin := _role_extra_cull_margin(role)
	if extra_margin > 0.0:
		instance.extra_cull_margin = max(instance.extra_cull_margin, extra_margin)
	instance.set_meta("shadow_policy_role", role)
	instance.set_meta("shadow_policy_category", category if not category.is_empty() else role)
	instance.set_meta("shadow_policy_casts", _role_casts_shadows(role))
	instance.set_meta("shadow_policy_receives", _role_receives_shadows(role))
	instance.set_meta("shadow_policy_extra_cull_margin", extra_margin)

static func shadow_policy_label(instance: GeometryInstance3D) -> String:
	if instance == null:
		return "none"
	var role := str(instance.get_meta("shadow_policy_role", "unregistered"))
	var category := str(instance.get_meta("shadow_policy_category", "unregistered"))
	var casts := bool(instance.get_meta("shadow_policy_casts", false))
	var receives := bool(instance.get_meta("shadow_policy_receives", false))
	return "%s/%s cast=%s policy_casts=%s policy_receives=%s extra_cull=%.1f" % [
		category,
		role,
		_shadow_cast_label(instance.cast_shadow),
		str(casts),
		str(receives),
		instance.extra_cull_margin,
	]

static func _role_casts_shadows(role: String) -> bool:
	return role == SHADOW_STATIC_CASTER or role == SHADOW_DYNAMIC_CASTER

static func _role_receives_shadows(role: String) -> bool:
	return role != SHADOW_DEBUG_OVERLAY

static func _role_extra_cull_margin(role: String) -> float:
	match role:
		SHADOW_STATIC_CASTER:
			return STATIC_CASTER_EXTRA_CULL_MARGIN_M
		SHADOW_DYNAMIC_CASTER:
			return DYNAMIC_CASTER_EXTRA_CULL_MARGIN_M
		SHADOW_RECEIVER_ONLY:
			return RECEIVER_EXTRA_CULL_MARGIN_M
		_:
			return 0.0

static func _shadow_cast_label(value: int) -> String:
	match value:
		GeometryInstance3D.SHADOW_CASTING_SETTING_OFF:
			return "off"
		GeometryInstance3D.SHADOW_CASTING_SETTING_ON:
			return "on"
		GeometryInstance3D.SHADOW_CASTING_SETTING_DOUBLE_SIDED:
			return "double_sided"
		GeometryInstance3D.SHADOW_CASTING_SETTING_SHADOWS_ONLY:
			return "shadows_only"
		_:
			return str(value)

func _ready() -> void:
	var scene_root := get_parent()
	if scene_root == null:
		return
	_configure_environment(scene_root)
	_configure_sun(scene_root)
	call_deferred("_print_debug_if_requested", scene_root)

func _configure_environment(scene_root: Node) -> void:
	var world_environment := scene_root.get_node_or_null("WorldEnvironment") as WorldEnvironment
	if world_environment == null:
		return
	var environment := world_environment.environment
	if environment == null:
		environment = Environment.new()
	else:
		environment = environment.duplicate()
	world_environment.environment = environment
	environment.background_mode = Environment.BG_COLOR
	environment.background_color = SKY_COLOR
	environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	environment.ambient_light_color = AMBIENT_COLOR
	environment.ambient_light_energy = AMBIENT_STRENGTH

func _configure_sun(scene_root: Node) -> void:
	var sun := scene_root.get_node_or_null("DirectionalLight3D") as DirectionalLight3D
	if sun == null:
		return
	sun.light_color = SUN_COLOR
	sun.light_energy = SUN_ENERGY
	sun.light_indirect_energy = SUN_INDIRECT_ENERGY
	sun.shadow_enabled = true
	sun.shadow_bias = SHADOW_BIAS
	sun.shadow_normal_bias = SHADOW_NORMAL_BIAS
	sun.shadow_blur = SHADOW_BLUR
	sun.set("directional_shadow_mode", 2)
	sun.set("directional_shadow_max_distance", SHADOW_MAX_DISTANCE_M)
	sun.set("directional_shadow_split_1", SHADOW_SPLIT_1)
	sun.set("directional_shadow_split_2", SHADOW_SPLIT_2)
	sun.set("directional_shadow_split_3", SHADOW_SPLIT_3)
	sun.set("directional_shadow_blend_splits", true)
	sun.set("directional_shadow_fade_start", SHADOW_FADE_START)
	sun.global_transform = Transform3D(
		Basis.looking_at(-sun_direction(), Vector3.UP),
		sun.global_position
	)

func _print_debug_if_requested(scene_root: Node) -> void:
	if not is_lighting_debug_enabled():
		return
	var splits := shadow_split_distances()
	print(
		"[DEBUG:lighting] sun_direction=%s sun_color=%s sky_color=%s ambient=%.3f shadow_max=%.1f splits=(%.1f,%.1f,%.1f) ground_shadow=(ambient=%.2f sun=%.2f min=%.2f)"
		% [
			str(sun_direction()),
			str(SUN_COLOR),
			str(SKY_COLOR),
			AMBIENT_STRENGTH,
			SHADOW_MAX_DISTANCE_M,
			splits.x,
			splits.y,
			splits.z,
			GROUND_SHADOW_AMBIENT,
			GROUND_SHADOW_SUN_STRENGTH,
			GROUND_SHADOW_MIN_VISIBILITY,
		]
	)
	_print_shadow_policy_summary(scene_root)

func _print_shadow_policy_summary(scene_root: Node) -> void:
	var stats := {}
	_collect_shadow_policy_stats(scene_root, stats)
	var keys := stats.keys()
	keys.sort()
	for key_variant in keys:
		var key := str(key_variant)
		var counts: Dictionary = stats[key]
		print(
			"[DEBUG:lighting] shadow_policy %s total=%d visible=%d policy_casts=%d policy_receives=%d off=%d on=%d double_sided=%d shadows_only=%d multimeshes=%d nonempty_multimeshes=%d multimesh_instances=%d max_extra_cull=%.1f"
			% [
				key,
				int(counts.get("total", 0)),
				int(counts.get("visible", 0)),
				int(counts.get("policy_casts", 0)),
				int(counts.get("policy_receives", 0)),
				int(counts.get("off", 0)),
				int(counts.get("on", 0)),
				int(counts.get("double_sided", 0)),
				int(counts.get("shadows_only", 0)),
				int(counts.get("multimeshes", 0)),
				int(counts.get("nonempty_multimeshes", 0)),
				int(counts.get("multimesh_instances", 0)),
				float(counts.get("max_extra_cull", 0.0)),
			]
		)

func _collect_shadow_policy_stats(node: Node, stats: Dictionary) -> void:
	if node is GeometryInstance3D:
		var geometry := node as GeometryInstance3D
		if geometry.has_meta("shadow_policy_role"):
			var category := str(geometry.get_meta("shadow_policy_category", "unregistered"))
			var role := str(geometry.get_meta("shadow_policy_role", "unregistered"))
			var key := "%s/%s" % [category, role]
			if not stats.has(key):
				stats[key] = {
					"total": 0,
					"visible": 0,
					"policy_casts": 0,
					"policy_receives": 0,
					"off": 0,
					"on": 0,
					"double_sided": 0,
					"shadows_only": 0,
					"multimeshes": 0,
					"nonempty_multimeshes": 0,
					"multimesh_instances": 0,
					"max_extra_cull": 0.0,
				}
			var counts: Dictionary = stats[key]
			counts["total"] = int(counts["total"]) + 1
			if geometry.visible:
				counts["visible"] = int(counts["visible"]) + 1
			if bool(geometry.get_meta("shadow_policy_casts", false)):
				counts["policy_casts"] = int(counts["policy_casts"]) + 1
			if bool(geometry.get_meta("shadow_policy_receives", false)):
				counts["policy_receives"] = int(counts["policy_receives"]) + 1
			var cast_label := _shadow_cast_label(geometry.cast_shadow)
			counts[cast_label] = int(counts.get(cast_label, 0)) + 1
			counts["max_extra_cull"] = max(float(counts["max_extra_cull"]), geometry.extra_cull_margin)
			if geometry is MultiMeshInstance3D:
				var mmi := geometry as MultiMeshInstance3D
				counts["multimeshes"] = int(counts["multimeshes"]) + 1
				if mmi.multimesh != null:
					var instance_count := mmi.multimesh.instance_count
					counts["multimesh_instances"] = int(counts["multimesh_instances"]) + instance_count
					if instance_count > 0:
						counts["nonempty_multimeshes"] = int(counts["nonempty_multimeshes"]) + 1
	for child in node.get_children():
		_collect_shadow_policy_stats(child, stats)
