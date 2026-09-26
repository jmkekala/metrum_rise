# SPDX-License-Identifier: GPL-2.0-only

## Deterministic scene lighting shared by gameplay and editor scenes.
##
## The node configures the existing WorldEnvironment and DirectionalLight3D at scene startup,
## then drives the day/night cycle every frame from the simulation clock. The palette itself
## lives in [code]day_cycle.gd[/code]; this node owns only the wiring, the shadow policy and
## the distance-fade ramp.
##
## Terrain, water and site ground read the sun through shader globals rather than through
## per-material parameters, so one write per frame relights every patch in the world instead
## of one write per patch per frame.
extends Node
class_name SceneLighting

const DayCycleConfig := preload("res://scripts/core/day_cycle.gd")
const SKY_CLOUD_SHADER := preload("res://scripts/shaders/sky_cloud_cover.gdshader")
const SKY_CLOUD_PANORAMA_PATH := (
	"res://assets/textures/general/sky/DaySkyHDRI059B_2K_TONEMAPPED.jpg"
)

const SKY_HEMISPHERE_CURVE := 0.20
# The panorama is a daylight capture used only as a cloud shape source; the day cycle supplies
# its colour through the shadow/light tints. The rotation is therefore fixed at the value the
# authored sun used to produce. Turning it with the sun would spin the entire cloud field
# around the sky once per day.
const SKY_CLOUD_PANORAMA_ROTATION := 0.3903
const SKY_CLOUD_STRENGTH := 0.48
const SKY_CLOUD_HORIZON_FADE_START := 0.04
const SKY_CLOUD_HORIZON_FADE_END := 0.16
const SKY_SUN_CURVE := 0.12
# Writing a sky material parameter marks the sky dirty, and a dirty sky regenerates its radiance
# cubemap. Rewriting it with identical values every frame therefore pays for a regeneration that
# changes nothing, and a paused game pays it forever. The gate is a thirtieth of the sun disk's
# own diameter, so the disk cannot be seen to step.
const SKY_UPDATE_MIN_DEG := 0.02
const FAR_FADE_END_GUARD_PATCHES := 0.50
# Aerial perspective and the far cull curtain are one depth ramp. The haze starts a few
# hundred metres out and reaches full sky colour exactly at the cull boundary, so distance
# reads as distance instead of the whole world sharing one saturation.
# The ramp is proportional to the far plane because CameraNode derives that from zoom.
# A fixed metric start would haze most of the view when zoomed in, where the far plane is
# only about 1.3 km. The curve keeps the near two thirds of the ramp faint, so it reads as
# atmosphere, and still closes to full sky colour at the cull boundary.
const AERIAL_HAZE_BEGIN_RATIO := 0.32
const FAR_FADE_CURVE := 2.60
# The sun subtends about 0.53 degrees. 1.15 doubled the penumbra, which read as noise on
# small casters such as tree crowns rather than as softness. The full moon subtends the same
# angle, so the cycle never has a reason to change this and the GPU probe keeps it as a lever.
const SUN_ANGULAR_DISTANCE_DEG := 0.55
const SHADOW_MAX_DISTANCE_M := 420.0
const SHADOW_SPLIT_1 := 0.10
const SHADOW_SPLIT_2 := 0.28
const SHADOW_SPLIT_3 := 0.58
const SHADOW_FADE_START := 0.78
const SHADOW_BIAS := 0.060
const SHADOW_NORMAL_BIAS := 1.25
const SHADOW_BLUR := 1.80
const GROUND_SHADOW_AMBIENT := 0.50
const GROUND_SHADOW_SUN_STRENGTH := 0.50
const GROUND_SHADOW_MIN_VISIBILITY := 0.02
# Share of its open-ground brightness a fully covered forest floor keeps once the shadow
# cascades stop reaching it. The crowns past the cascade edge already replace the shadow their
# neighbours stop casting; the ground they stand on did not, so a distant stand read as lit
# field with dark specks on it. This is the receiver half of the same term.
#
# The value is derived and not chosen. Terrain ground takes the key light at
# GROUND_SHADOW_SUN_STRENGTH and the sky at GROUND_SHADOW_AMBIENT, and a fragment the cascades
# put in shadow keeps GROUND_SHADOW_MIN_VISIBILITY of the key term. Where sun and sky are of
# comparable strength, that is the ratio below, so ground outside the cascades holds what the
# same ground held inside them.
const CANOPY_FLOOR_SHADE_FLOOR := (
	(GROUND_SHADOW_AMBIENT + GROUND_SHADOW_SUN_STRENGTH * GROUND_SHADOW_MIN_VISIBILITY)
	/ (GROUND_SHADOW_AMBIENT + GROUND_SHADOW_SUN_STRENGTH)
)
const STATIC_CASTER_EXTRA_CULL_MARGIN_M := 32.0
const DYNAMIC_CASTER_EXTRA_CULL_MARGIN_M := 12.0
const RECEIVER_EXTRA_CULL_MARGIN_M := 2.0

const SHADOW_STATIC_CASTER := "static_caster"
const SHADOW_DYNAMIC_CASTER := "dynamic_caster"
const SHADOW_TINY_DYNAMIC := "tiny_dynamic"
const SHADOW_RECEIVER_ONLY := "receiver_only"
const SHADOW_NON_RECEIVER := "non_receiver"
const SHADOW_DEBUG_OVERLAY := "debug_overlay"

static var _cloud_panorama_cache: Texture2D

var _distance_fade_environment: Environment
var _distance_fade_terrain: Node
var _distance_fade_begin_m := -1.0
var _distance_fade_end_m := -1.0
var _sun: DirectionalLight3D
var _sky_material: Material
var _simulation: Node
# Written in place every frame, so the per-frame lighting path allocates nothing.
var _day_sample := DayCycleConfig.create_sample()
var _pinned_day_fraction := -1.0
var _sky_applied_elevation_deg := INF
var _sky_applied_azimuth_deg := INF

## Shadow range in effect, in metres. `METRUM_SHADOW_FAR` replaces the authored range, so a
## probe can separate the two discontinuities that sit near each other in a forest view: the
## end of shadow at SHADOW_MAX_DISTANCE_M, and the vegetation mesh change further out. Values
## at or below zero keep the authored range.
static func shadow_max_distance_m() -> float:
	var override := OS.get_environment("METRUM_SHADOW_FAR").strip_edges().to_float()
	return override if override > 0.0 else SHADOW_MAX_DISTANCE_M

static func shadow_split_distances() -> Vector3:
	var far_m := shadow_max_distance_m()
	return Vector3(
		far_m * SHADOW_SPLIT_1,
		far_m * SHADOW_SPLIT_2,
		far_m * SHADOW_SPLIT_3
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

## Probe override for the canopy shading strength, which scales both halves of the term: the
## shade on the crowns and the shade on the ground under them. Zero removes it without an edit
## to a shader, which is what a paired look at the same stand needs; one is the derived default,
## and values above it extrapolate past the floor for a stand that still reads too bright. An
## unset or unparsable value keeps the default.
static func canopy_shade_strength() -> float:
	var raw := OS.get_environment("METRUM_CANOPY_SHADE").strip_edges()
	return clampf(raw.to_float(), 0.0, 2.0) if raw.is_valid_float() else 1.0

## Ties the forest-floor shading ramp to the shadow cascades, exactly as the canopy half in
## `tree_species.gd` is tied to them. The term begins where the cascades begin to fade and
## reaches full strength where they end, so no ground inside shadow range changes at all.
static func apply_canopy_floor_shading(material: ShaderMaterial) -> void:
	if material == null:
		return
	var far_m := shadow_max_distance_m()
	material.set_shader_parameter("canopy_floor_shade_begin_m", far_m * SHADOW_FADE_START)
	material.set_shader_parameter("canopy_floor_shade_end_m", far_m)
	material.set_shader_parameter("canopy_floor_shade_floor", CANOPY_FLOOR_SHADE_FLOOR)
	material.set_shader_parameter("canopy_floor_shade", canopy_shade_strength())

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
		SHADOW_TINY_DYNAMIC, SHADOW_RECEIVER_ONLY, SHADOW_NON_RECEIVER, SHADOW_DEBUG_OVERLAY:
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
	return role != SHADOW_DEBUG_OVERLAY and role != SHADOW_NON_RECEIVER

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
	_pinned_day_fraction = DayCycleConfig.pinned_day_fraction()
	_simulation = scene_root.get_node_or_null("SimulationNode")
	_configure_sun(scene_root)
	_configure_environment(scene_root)
	_distance_fade_terrain = scene_root.get_node_or_null("Terrain")
	# Before the first frame is drawn, so nothing renders against the project-default palette.
	_apply_day_cycle()
	_update_distance_fade()
	call_deferred("_print_debug_if_requested", scene_root)

func _process(_delta: float) -> void:
	_apply_day_cycle()
	_update_distance_fade()

# Every write here is a single RenderingServer call that covers the whole world, so the cost
# is a constant handful of calls per frame regardless of how many patches are resident.
func _apply_day_cycle() -> void:
	DayCycleConfig.sample_into(_current_day_fraction(), _day_sample)
	_apply_key_light(_day_sample)
	_apply_environment_palette(_day_sample)
	_apply_shader_globals(_day_sample)
	_apply_sky_palette(_day_sample)

## Pins the rendered clock to an hour of day, or releases it back to the simulation clock
## when given a negative hour.
##
## This is the same override `METRUM_TIME_OF_DAY` sets at startup, exposed so local capture
## tooling can hold one lighting state without reloading the world for every hour.
func pin_hour_of_day(hour: float) -> void:
	_pinned_day_fraction = -1.0 if hour < 0.0 else fposmod(hour / 24.0, 1.0)
	_apply_day_cycle()

## Returns the lighting state the last applied frame resolved to. Read-only: the node rewrites
## it every frame.
func current_sample() -> DayCycleConfig.Sample:
	return _day_sample

# A paused simulation holds the fraction still, which parks the sun rather than freezing it
# mid-slew. Scenes with no simulation, such as the asset editor, get a fixed authored hour.
func _current_day_fraction() -> float:
	if _pinned_day_fraction >= 0.0:
		return _pinned_day_fraction
	if _simulation != null and _simulation.has_method("get_day_fraction"):
		return float(_simulation.get_day_fraction())
	return DayCycleConfig.FALLBACK_HOUR / 24.0

func _apply_key_light(sample: DayCycleConfig.Sample) -> void:
	if _sun == null:
		return
	# Hidden through the dead band between sunset and moonrise. The energy is already zero
	# there, so this only skips the shadow map the engine would otherwise still render.
	var lit: bool = sample.key_energy > 0.0
	_sun.visible = lit
	if not lit:
		return
	_sun.light_color = sample.key_color
	_sun.light_energy = sample.key_energy
	_sun.light_indirect_energy = sample.key_indirect_energy
	# looking_at needs an up vector that is not parallel to the target. The authored latitude
	# keeps the key light well away from the zenith, but the guard costs nothing and stops a
	# retuned latitude from producing a broken basis instead of a warning.
	var direction: Vector3 = sample.key_direction
	var up := Vector3.UP if absf(direction.y) < 0.999 else Vector3.FORWARD
	_sun.global_transform = Transform3D(
		Basis.looking_at(-direction, up),
		_sun.global_position
	)

func _apply_environment_palette(sample: DayCycleConfig.Sample) -> void:
	if _distance_fade_environment == null:
		return
	_distance_fade_environment.background_color = sample.sky_horizon
	_distance_fade_environment.ambient_light_color = sample.ambient_color
	_distance_fade_environment.ambient_light_energy = sample.ambient_energy
	# Aerial perspective and the far cull curtain share one ramp, so the haze colour is also
	# what the horizon dissolves into. Tracking the sky keeps distance reading as distance at
	# every hour instead of leaving a daytime blue curtain standing at midnight.
	_distance_fade_environment.fog_light_color = sample.fog_color
	_distance_fade_environment.fog_sun_scatter = sample.fog_sun_scatter

func _apply_shader_globals(sample: DayCycleConfig.Sample) -> void:
	RenderingServer.global_shader_parameter_set("scene_window_clock", Vector2(sample.day_fraction * 24.0, sample.sun_elevation_deg))
	RenderingServer.global_shader_parameter_set("scene_sun_direction", sample.key_direction)
	RenderingServer.global_shader_parameter_set("scene_sun_color", sample.key_color)
	RenderingServer.global_shader_parameter_set("scene_sky_color", sample.sky_horizon)
	RenderingServer.global_shader_parameter_set("scene_ambient_strength", sample.ambient_energy)
	RenderingServer.global_shader_parameter_set(
		"scene_ambient_light", sample.ambient_light_scale
	)
	RenderingServer.global_shader_parameter_set(
		"scene_ambient_desaturation", sample.ambient_desaturation
	)

func _apply_sky_palette(sample: DayCycleConfig.Sample) -> void:
	if (
		absf(sample.sun_elevation_deg - _sky_applied_elevation_deg) < SKY_UPDATE_MIN_DEG
		and absf(
			angle_difference(
				deg_to_rad(sample.sun_azimuth_deg), deg_to_rad(_sky_applied_azimuth_deg)
			)
		) < deg_to_rad(SKY_UPDATE_MIN_DEG)
	):
		return
	_sky_applied_elevation_deg = sample.sun_elevation_deg
	_sky_applied_azimuth_deg = sample.sun_azimuth_deg
	if _sky_material is ShaderMaterial:
		var shader_sky := _sky_material as ShaderMaterial
		shader_sky.set_shader_parameter("sky_zenith_color", sample.sky_zenith)
		shader_sky.set_shader_parameter("sky_horizon_color", sample.sky_horizon)
		shader_sky.set_shader_parameter("sky_nadir_color", sample.sky_nadir)
		shader_sky.set_shader_parameter("cloud_shadow_color", sample.cloud_shadow)
		shader_sky.set_shader_parameter("cloud_light_color", sample.cloud_light)
		shader_sky.set_shader_parameter("sun_disk_direction", sample.sun_disk_direction)
		shader_sky.set_shader_parameter("sun_disk_color", sample.sun_disk_color)
		shader_sky.set_shader_parameter("sun_disk_intensity", sample.sun_disk_intensity)
		shader_sky.set_shader_parameter("sun_disk_deg", sample.sun_disk_deg)
		shader_sky.set_shader_parameter("sun_halo_deg", sample.sun_halo_deg)
		shader_sky.set_shader_parameter("sun_halo_strength", sample.sun_halo_strength)
		shader_sky.set_shader_parameter("moon_disk_direction", sample.moon_disk_direction)
		shader_sky.set_shader_parameter("moon_disk_intensity", sample.moon_disk_intensity)
	elif _sky_material is ProceduralSkyMaterial:
		# Fallback path when the cloud panorama is missing. It has no cloud layer and no disk
		# controls, so it carries the gradient only.
		var procedural_sky := _sky_material as ProceduralSkyMaterial
		procedural_sky.sky_top_color = sample.sky_zenith
		procedural_sky.sky_horizon_color = sample.sky_horizon
		procedural_sky.ground_horizon_color = sample.sky_horizon
		procedural_sky.ground_bottom_color = sample.sky_nadir

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
	environment.background_mode = Environment.BG_SKY
	environment.sky = _create_hemisphere_sky()
	environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	environment.fog_enabled = false
	environment.fog_mode = Environment.FOG_MODE_DEPTH
	environment.fog_density = 1.0
	environment.fog_depth_curve = FAR_FADE_CURVE
	environment.fog_height_density = 0.0
	environment.fog_light_energy = 1.0
	environment.fog_aerial_perspective = 0.0
	environment.fog_sky_affect = 0.0
	_distance_fade_environment = environment

func _update_distance_fade() -> void:
	if _distance_fade_environment == null or _distance_fade_terrain == null:
		return
	if not is_inside_tree():
		return
	var viewport := get_viewport()
	if viewport == null:
		return
	var camera := viewport.get_camera_3d()
	if camera == null:
		_distance_fade_environment.fog_enabled = false
		return
	if not _distance_fade_terrain.has_method("get_visibility_cull_far_m"):
		_distance_fade_environment.fog_enabled = false
		return

	var cull_far_m := float(_distance_fade_terrain.get_visibility_cull_far_m(camera))
	var patch_span_m := 1.0
	if _distance_fade_terrain.has_method("get_render_patch_span_m"):
		patch_span_m = maxf(
			float(_distance_fade_terrain.get_render_patch_span_m()),
			1.0
		)
	# Finish inside the cull boundary so desired/resident/prewarm patches stay
	# fully hidden while they activate behind the atmospheric transition.
	var fade_end_m := maxf(
		cull_far_m - patch_span_m * FAR_FADE_END_GUARD_PATCHES,
		1.0
	)
	var fade_begin_m := fade_end_m * AERIAL_HAZE_BEGIN_RATIO
	if (
		is_equal_approx(fade_begin_m, _distance_fade_begin_m)
		and is_equal_approx(fade_end_m, _distance_fade_end_m)
	):
		if not _distance_fade_environment.fog_enabled:
			_distance_fade_environment.fog_enabled = true
		return

	_distance_fade_begin_m = fade_begin_m
	_distance_fade_end_m = fade_end_m
	_distance_fade_environment.fog_depth_begin = fade_begin_m
	_distance_fade_environment.fog_depth_end = fade_end_m
	_distance_fade_environment.fog_enabled = true

func _create_hemisphere_sky() -> Sky:
	var cloud_panorama := _load_cloud_panorama()
	if cloud_panorama == null:
		return _create_procedural_hemisphere_sky()

	var material := ShaderMaterial.new()
	material.shader = SKY_CLOUD_SHADER
	material.set_shader_parameter("cloud_panorama", cloud_panorama)
	material.set_shader_parameter("hemisphere_curve", SKY_HEMISPHERE_CURVE)
	material.set_shader_parameter("panorama_rotation", SKY_CLOUD_PANORAMA_ROTATION)
	material.set_shader_parameter("cloud_strength", SKY_CLOUD_STRENGTH)
	material.set_shader_parameter(
		"cloud_horizon_fade_start",
		SKY_CLOUD_HORIZON_FADE_START
	)
	material.set_shader_parameter("cloud_horizon_fade_end", SKY_CLOUD_HORIZON_FADE_END)
	material.set_shader_parameter("sun_curve", SKY_SUN_CURVE)
	# The moon's appearance is fixed, so it is set here rather than rewritten every frame. Only
	# its direction and brightness change with the cycle.
	material.set_shader_parameter("moon_disk_color", DayCycleConfig.MOON_DISK_COLOR)
	material.set_shader_parameter("moon_disk_deg", DayCycleConfig.MOON_DISK_DEG)
	material.set_shader_parameter("moon_halo_deg", DayCycleConfig.MOON_HALO_DEG)
	material.set_shader_parameter("moon_halo_strength", DayCycleConfig.MOON_HALO_STRENGTH)
	_sky_material = material
	return _build_sky(material)

func _load_cloud_panorama() -> Texture2D:
	if _cloud_panorama_cache != null:
		return _cloud_panorama_cache
	if not FileAccess.file_exists(SKY_CLOUD_PANORAMA_PATH):
		push_warning("Cloud panorama is missing: %s" % SKY_CLOUD_PANORAMA_PATH)
		return null
	var image := Image.new()
	var decode_error := image.load_jpg_from_buffer(
		FileAccess.get_file_as_bytes(SKY_CLOUD_PANORAMA_PATH)
	)
	if decode_error != OK:
		push_warning(
			"Cloud panorama could not be decoded (%s): %s"
			% [decode_error, SKY_CLOUD_PANORAMA_PATH]
		)
		return null
	var mipmap_error := image.generate_mipmaps()
	if mipmap_error != OK:
		push_warning("Cloud panorama mipmaps could not be generated: %s" % mipmap_error)
	_cloud_panorama_cache = ImageTexture.create_from_image(image)
	return _cloud_panorama_cache

func _create_procedural_hemisphere_sky() -> Sky:
	var material := ProceduralSkyMaterial.new()
	material.sky_curve = SKY_HEMISPHERE_CURVE
	material.ground_curve = SKY_HEMISPHERE_CURVE
	material.sun_curve = SKY_SUN_CURVE
	material.use_debanding = true
	_sky_material = material
	return _build_sky(material)

func _build_sky(material: Material) -> Sky:
	var sky := Sky.new()
	# The sky palette changes every frame now. INCREMENTAL spreads the radiance cubemap
	# refresh across frames, which is the mode Godot provides for exactly this case; QUALITY
	# would pay the full regeneration cost on every one of those changes.
	sky.process_mode = Sky.PROCESS_MODE_INCREMENTAL
	sky.radiance_size = Sky.RADIANCE_SIZE_128
	sky.sky_material = material
	return sky

func _configure_sun(scene_root: Node) -> void:
	var sun := scene_root.get_node_or_null("DirectionalLight3D") as DirectionalLight3D
	if sun == null:
		return
	_sun = sun
	sun.light_angular_distance = SUN_ANGULAR_DISTANCE_DEG
	# The sky shader places its own sun and moon disks so their brightness can differ from the
	# key light's energy. Letting the engine draw a third disk from this light would double it.
	sun.sky_mode = DirectionalLight3D.SKY_MODE_LIGHT_ONLY
	sun.shadow_enabled = true
	sun.shadow_bias = SHADOW_BIAS
	sun.shadow_normal_bias = SHADOW_NORMAL_BIAS
	sun.shadow_blur = SHADOW_BLUR
	sun.set("directional_shadow_mode", 2)
	sun.set("directional_shadow_max_distance", shadow_max_distance_m())
	sun.set("directional_shadow_split_1", SHADOW_SPLIT_1)
	sun.set("directional_shadow_split_2", SHADOW_SPLIT_2)
	sun.set("directional_shadow_split_3", SHADOW_SPLIT_3)
	sun.set("directional_shadow_blend_splits", true)
	sun.set("directional_shadow_fade_start", SHADOW_FADE_START)

func _print_debug_if_requested(scene_root: Node) -> void:
	if not is_lighting_debug_enabled():
		return
	await get_tree().process_frame
	if not is_inside_tree():
		return
	_update_distance_fade()
	var splits := shadow_split_distances()
	var hour := _day_sample.day_fraction * 24.0
	print(
		"[DEBUG:lighting] time=%02d:%02d sun_elevation=%.2f sun_azimuth=%.2f moonlit=%s key_color=%s key_energy=%.3f sky=%s fog=%s ambient=%.3f ambient_scale=%s shadow_max=%.1f splits=(%.1f,%.1f,%.1f) far_fade=(%.1f,%.1f) ground_shadow=(ambient=%.2f sun=%.2f min=%.2f)"
		% [
			int(hour),
			int(fposmod(hour, 1.0) * 60.0),
			_day_sample.sun_elevation_deg,
			_day_sample.sun_azimuth_deg,
			str(_day_sample.is_moonlit),
			str(_day_sample.key_color),
			_day_sample.key_energy,
			str(_day_sample.sky_horizon),
			str(_day_sample.fog_color),
			_day_sample.ambient_energy,
			str(_day_sample.ambient_light_scale),
			shadow_max_distance_m(),
			splits.x,
			splits.y,
			splits.z,
			_distance_fade_begin_m,
			_distance_fade_end_m,
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
