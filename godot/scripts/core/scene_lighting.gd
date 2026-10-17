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
const GameSettings := preload("res://scripts/core/game_settings.gd")
const SKY_CLOUD_SHADER := preload("res://scripts/shaders/sky_cloud_cover.gdshader")
const NightSkyBake := preload("res://scripts/renderers/night_sky_bake.gd")
const WaterReflection := preload("res://scripts/renderers/water_reflection.gd")
const WeatherGustMap := preload("res://scripts/renderers/weather_gust_map.gd")
const WeatherCloudShape := preload("res://scripts/renderers/weather_cloud_shape.gd")
const WeatherCloudEffect := preload("res://scripts/renderers/weather_cloud_effect.gd")
const WeatherMistEffect := preload("res://scripts/renderers/weather_mist_effect.gd")
# Weather for scenes with no simulation, such as the asset editor: settled fair cumulus, still.
const FALLBACK_WEATHER_CLOUDS := Vector4(0.0, 0.0, 0.45, 1.2)
# Fair weather's authored wind direction (weather.rs), for scenes with no simulation.
const FALLBACK_WIND_DIRECTION := Vector2(0.936329, 0.351123)
# Height a cloud's mass casts its shadow from, the base plus a third of the thickness, and the
# lowest sun that casts one; both must match weather_clouds.gdshaderinc.
const CLOUD_SHADOW_CAST_M := 1500.0 + 1400.0 * 0.3
const CLOUD_SHADOW_MIN_SUN_Y := 0.005
# The baked shadow window: 1024 texels of 24 m cover the 9 km of visible ground on every side,
# finer than the narrowest penumbra (about 70 m).
const CLOUD_SHADOW_TEXEL_M := 24.0
# Morning mist: extinction per metre at the water's surface over open water (about 190 m of
# visibility inside it), and the height over which it thins by e, so it hugs the water.
const MIST_EXTINCTION_PER_M := 0.016
const MIST_SCALE_HEIGHT_M := 14.0
# In-game hours: mist gathers from the first to the second, holds to the third and burns off by
# the fourth. An authored envelope, not a meteorological model.
const MIST_HOURS := [2.0, 5.0, 7.0, 10.0]
# Must match WIND_PERIOD_S in rust/src/simulation/core/weather.rs and weather_wind.gdshaderinc.
const WIND_PERIOD_S := 16384.0

const SKY_HEMISPHERE_CURVE := 0.20
const SKY_SUN_CURVE := 0.12
# Writing a sky material parameter marks the sky dirty, and a dirty sky regenerates its radiance
# cubemap. Rewriting it with identical values every frame therefore pays for a regeneration that
# changes nothing, and a paused game pays it forever. The gate is a thirtieth of the sun disk's
# own diameter, so the disk cannot be seen to step.
const SKY_UPDATE_MIN_DEG := 0.02
# Smallest change of cloud coverage that refreshes the sky's radiance cubemap.
const SKY_UPDATE_MIN_COVERAGE := 0.02
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
# SSIL carries window light spill (RENDER-10). Window thresholds in `window_lighting.rs` span
# 1-8 degrees of sun elevation with a 0.35 degree fade, so above this no window is lit. By day
# SSIL changed the measured image by under 0.3/255 and cost 1.5-1.9 ms (RENDER-15).
const WINDOW_LIGHT_MAX_SUN_ELEVATION_DEG := 8.35
const SHADOW_MAX_DISTANCE_M := 420.0
const SHADOW_SPLIT_1 := 0.10
const SHADOW_SPLIT_2 := 0.28
const SHADOW_SPLIT_3 := 0.58
const SHADOW_FADE_START := 0.78
const SHADOW_BIAS := 0.060
const SHADOW_NORMAL_BIAS := 1.25
# Multiplies the penumbra the sun's angular size gives. One is the real sun. 1.80 nearly doubled
# it, and with the low soft-shadow filter a trunk's shadow broke into dots and stopped short of
# its root. One, with the high filter in project.godot, keeps it whole from the root.
const SHADOW_BLUR := 1.0
const GROUND_SHADOW_AMBIENT := 0.50
const GROUND_SHADOW_SUN_STRENGTH := 0.50
const GROUND_SHADOW_MIN_VISIBILITY := 0.02
# Share of the sky light that still reaches a forest floor under closed crowns. The terrain
# takes the rest away in proportion to the sky the published crown coverage hides, so the
# floor of a stand in shade no longer keeps the sky of open ground in the shade of a house.
# Only the terrain takes this term: trunks and understory keep the full sky, and a floor
# darker than the trunks standing on it reads as burnt ground. The share is therefore set
# where the shaded floor of a 625 stems/ha pine stand renders just above its shaded trunks, at
# 57 against 52 in 8-bit luminance, where 0.20 put it at 40 and the old floor sat at 88.
const CANOPY_FLOOR_SKY_TRANSMISSION := 0.40
# Share of the sun the crowns of a closed stand pass to its floor, which the floor shade past
# the cascades keeps. An authored crown is open, so its cascade shadow is dappled and not solid.
# GROUND_SHADOW_MIN_VISIBILITY treated every crown as opaque and drew the far floor at half the
# light the cascades leave it, which was a dark step in the floor of every stand at the cascade
# edge. Fitted with the trees drawn as shadow only, over a 625 stems/ha pine stand seen
# straight down from 300 m, inside the cascades, and from 600 m, past them.
const CANOPY_FLOOR_SUN_VISIBILITY := 0.24
const STATIC_CASTER_EXTRA_CULL_MARGIN_M := 32.0
const DYNAMIC_CASTER_EXTRA_CULL_MARGIN_M := 12.0
const RECEIVER_EXTRA_CULL_MARGIN_M := 2.0

const SHADOW_STATIC_CASTER := "static_caster"
const SHADOW_DYNAMIC_CASTER := "dynamic_caster"
const SHADOW_TINY_DYNAMIC := "tiny_dynamic"
const SHADOW_RECEIVER_ONLY := "receiver_only"
const SHADOW_NON_RECEIVER := "non_receiver"
const SHADOW_DEBUG_OVERLAY := "debug_overlay"

## Sun elevation in degrees as last applied, for scripts that switch on at dusk (street lights).
static var current_sun_elevation_deg := 90.0

var _distance_fade_environment: Environment
## Benchmarks clear this to measure the frame without SSIL; gameplay never changes it.
var ssil_allowed := true
## Whether clouds may shade the sun. Off makes every receiver skip its cloud lookup.
var cloud_shadows_allowed := true
var _local_wind_clock_s := 0.0
var _distance_fade_terrain: Node
var _distance_fade_begin_m := -1.0
var _distance_fade_end_m := -1.0
var _sun: DirectionalLight3D
var _sky_material: Material
var _cloud_effect: WeatherCloudEffect
var _mist_effect: WeatherMistEffect
var _mist_map_generation := -1
# The shadow window the cloud effect baked last frame, as the receivers' global reads it.
var _baked_shadow_window := Vector4.ZERO
var _simulation: Node
# Written in place every frame, so the per-frame lighting path allocates nothing.
var _day_sample := DayCycleConfig.create_sample()
var _pinned_day_fraction := -1.0
var _sky_applied_elevation_deg := INF
var _sky_applied_azimuth_deg := INF
var _sky_applied_coverage := INF

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
	return MetrumDebug.is_enabled("lighting")

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
## reaches full strength where they end, so inside shadow range the floor loses only the sky
## its crowns hide, which holds at every distance.
static func apply_canopy_floor_shading(material: ShaderMaterial) -> void:
	if material == null:
		return
	var far_m := shadow_max_distance_m()
	material.set_shader_parameter("canopy_floor_shade_begin_m", far_m * SHADOW_FADE_START)
	material.set_shader_parameter("canopy_floor_shade_end_m", far_m)
	material.set_shader_parameter("canopy_floor_sky_transmission", CANOPY_FLOOR_SKY_TRANSMISSION)
	material.set_shader_parameter("canopy_floor_sun_visibility", CANOPY_FLOOR_SUN_VISIBILITY)
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
	var cloud_shape := WeatherCloudShape.new()
	cloud_shape.name = "WeatherCloudShape"
	add_child(cloud_shape)
	var night_sky_bake := NightSkyBake.new()
	night_sky_bake.name = "NightSkyBake"
	add_child(night_sky_bake)
	var gust_map := WeatherGustMap.new()
	gust_map.name = "WeatherGustMap"
	add_child(gust_map)
	var water_reflection := WaterReflection.new()
	water_reflection.name = "WaterReflection"
	water_reflection.setup(
		_simulation, _sun, _distance_fade_environment, scene_root.get_node_or_null("Water")
	)
	add_child(water_reflection)
	add_to_group("graphics_settings")
	_distance_fade_terrain = scene_root.get_node_or_null("Terrain")
	# Before the first frame is drawn, so nothing renders against the project-default palette.
	_apply_day_cycle()
	_update_distance_fade()
	call_deferred("_print_debug_if_requested", scene_root)

# The reflection global outlives this scene; left pointing at the freed panorama, every
# material that reads globals would fail to bind until the next scene published a new one.
func _exit_tree() -> void:
	if _cloud_effect != null:
		RenderingServer.global_shader_parameter_set("weather_cloud_reflection", null)
		RenderingServer.global_shader_parameter_set("weather_cloud_shadow_map", null)
		RenderingServer.global_shader_parameter_set("weather_cloud_shadow_window", Vector4.ZERO)

func _process(delta: float) -> void:
	# A scene with no simulation, such as the asset editor, keeps its own wind clock on wall
	# time so its trees still sway; a simulated world's wind follows the simulation clock.
	_local_wind_clock_s = fposmod(_local_wind_clock_s + delta, WIND_PERIOD_S)
	_apply_day_cycle()
	_update_distance_fade()

# Every write here is a single RenderingServer call that covers the whole world, so the cost
# is a constant handful of calls per frame regardless of how many patches are resident.
func _apply_day_cycle() -> void:
	DayCycleConfig.sample_into(_current_day_fraction(), _day_sample, _current_day())
	_apply_key_light(_day_sample)
	_apply_environment_palette(_day_sample)
	_apply_shader_globals(_day_sample)
	_apply_weather_globals(_day_sample)
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

# The moon's month counts simulation days; a scene with no simulation keeps day zero.
func _current_day() -> int:
	if _simulation != null and _simulation.has_method("get_current_day"):
		return int(_simulation.get_current_day())
	return 0

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
	_distance_fade_environment.ssil_enabled = (
		ssil_allowed and sample.sun_elevation_deg < WINDOW_LIGHT_MAX_SUN_ELEVATION_DEG
	)

func _apply_shader_globals(sample: DayCycleConfig.Sample) -> void:
	current_sun_elevation_deg = sample.sun_elevation_deg
	RenderingServer.global_shader_parameter_set("scene_window_clock", Vector2(sample.day_fraction * 24.0, sample.sun_elevation_deg))
	RenderingServer.global_shader_parameter_set("scene_sun_direction", sample.key_direction)
	RenderingServer.global_shader_parameter_set("scene_celestial_basis", sample.celestial_basis)
	RenderingServer.global_shader_parameter_set("scene_sun_color", sample.key_color)
	RenderingServer.global_shader_parameter_set("scene_sky_color", sample.sky_horizon)
	RenderingServer.global_shader_parameter_set("scene_ambient_strength", sample.ambient_energy)
	RenderingServer.global_shader_parameter_set(
		"scene_ambient_light", sample.ambient_light_scale
	)
	RenderingServer.global_shader_parameter_set(
		"scene_ambient_desaturation", sample.ambient_desaturation
	)

# Numeric globals only: writing them never dirties the sky's radiance map, so the clouds can
# drift every frame for the cost of a few calls.
func _apply_weather_globals(sample: DayCycleConfig.Sample) -> void:
	var clouds := FALLBACK_WEATHER_CLOUDS
	var simulated := _simulation != null and _simulation.has_method("get_weather_clouds")
	if simulated:
		clouds = _simulation.get_weather_clouds()
	RenderingServer.global_shader_parameter_set("weather_clouds", clouds)
	var wind := Vector4(_local_wind_clock_s, 1.0, 0.0, 0.0)
	if simulated:
		wind = _simulation.get_weather_wind()
	RenderingServer.global_shader_parameter_set("weather_wind", wind)
	var wind_direction := FALLBACK_WIND_DIRECTION
	var high_clouds := Vector4.ZERO
	var rain := 0.0
	if simulated:
		wind_direction = _simulation.get_weather_wind_direction()
		high_clouds = _simulation.get_weather_high_clouds()
		rain = _simulation.get_weather_rain()
	RenderingServer.global_shader_parameter_set("weather_wind_direction", wind_direction)
	RenderingServer.global_shader_parameter_set("weather_rain", Vector4(rain, 0.0, 0.0, 0.0))
	var sun_lit := 0.0 if sample.is_moonlit else 1.0
	var sun: Vector3 = sample.sun_direction
	# A scene with no simulation, such as the asset editor, keeps its sky but casts no cloud
	# shadow: a fixed cloud would sit over its preview for good, and there only the building
	# shader takes the shadow, not the editor ground.
	RenderingServer.global_shader_parameter_set(
		"weather_sun",
		Vector4(sun.x, sun.y, sun.z, sun_lit if simulated and cloud_shadows_allowed else 0.0)
	)
	_apply_cloud_shadow_window(clouds, sun, simulated)
	_apply_mist(sample, clouds, wind, simulated)
	if _cloud_effect != null:
		_cloud_effect.clouds = clouds
		_cloud_effect.high_clouds = high_clouds
		_cloud_effect.rain = rain
		_cloud_effect.sun = Vector4(sun.x, sun.y, sun.z, sun_lit)
		# The palette is sRGB, as the sky's source_color uniforms and the lights take it; the
		# compute passes blend into the linear frame, so they get it converted.
		_cloud_effect.shade = sample.cloud_shadow.srgb_to_linear()
		_cloud_effect.light = sample.cloud_light.srgb_to_linear()
		_cloud_effect.horizon = sample.sky_horizon.srgb_to_linear()

# Centres the baked shadow window where the camera's ground casts its shadow on the deck and
# snaps it to whole texels, so the bake does not crawl as the camera moves. The cloud effect bakes
# it after this frame's opaque pass, so receivers read it next frame: the global published now is
# last frame's window, the one the texture holds. Receivers outside it, or every receiver while
# it is off, evaluate the weather map themselves.
func _apply_cloud_shadow_window(clouds: Vector4, sun: Vector3, simulated: bool) -> void:
	if _cloud_effect == null:
		return
	RenderingServer.global_shader_parameter_set("weather_cloud_shadow_window", _baked_shadow_window)
	var window := Vector4.ZERO
	_cloud_effect.shadow_window = Vector4.ZERO
	var camera := get_viewport().get_camera_3d() if is_inside_tree() else null
	var direction := sun.normalized()
	if (camera != null and simulated and cloud_shadows_allowed and clouds.z > 0.0
			and direction.y > CLOUD_SHADOW_MIN_SUN_Y):
		var ground: Vector3 = (camera.get_focus_position() if camera.has_method("get_focus_position")
			else camera.global_position)
		var separation := CLOUD_SHADOW_CAST_M - ground.y
		if separation > 0.0:
			var side := WeatherCloudEffect.SHADOW_TEXELS * CLOUD_SHADOW_TEXEL_M
			var centre := Vector2(ground.x, ground.z) + Vector2(direction.x, direction.z) * separation / direction.y
			var corner := ((centre - Vector2(side, side) * 0.5) / CLOUD_SHADOW_TEXEL_M).floor() * CLOUD_SHADOW_TEXEL_M
			_cloud_effect.shadow_window = Vector4(corner.x, corner.y, CLOUD_SHADOW_TEXEL_M, separation)
			window = Vector4(corner.x, corner.y, 1.0 / side, 1.0)
	_baked_shadow_window = window

# Mist gathers on calm, clear mornings, when the ground cools under an open sky and the air is
# still: a fair-weather breeze halves it, an overcast or stormy wind clears it, and a closed deck
# that keeps the night warm thins it further.
func _apply_mist(sample: DayCycleConfig.Sample, clouds: Vector4, wind: Vector4, simulated: bool) -> void:
	if _mist_effect == null:
		return
	if simulated:
		_refresh_mist_map()
	var hour := fposmod(_current_day_fraction(), 1.0) * 24.0
	var envelope := (smoothstep(MIST_HOURS[0], MIST_HOURS[1], hour)
		* (1.0 - smoothstep(MIST_HOURS[2], MIST_HOURS[3], hour)))
	var calm := clampf(1.5 - wind.y, 0.0, 1.0) * (1.0 - 0.6 * smoothstep(0.6, 1.0, clouds.z))
	var density := MIST_EXTINCTION_PER_M * envelope * calm if simulated else 0.0
	_mist_effect.density = density
	_mist_effect.enabled = density > 0.0 and _mist_effect.map_texture != null
	_mist_effect.scale_height = MIST_SCALE_HEIGHT_M
	_mist_effect.wind_offset = Vector2(wind.z, wind.w)
	_mist_effect.color = sample.fog_color.srgb_to_linear()
	_mist_effect.sun = sample.sun_direction
	_mist_effect.sun_color = sample.key_color.srgb_to_linear() if not sample.is_moonlit else Color.BLACK
	# The glow towards a low sun is what makes mist read as mist and not as haze.
	_mist_effect.sun_scatter = 0.6 * (1.0 - smoothstep(0.0, 25.0, sample.sun_elevation_deg))

# Uploads the Rust mist map when the water or terrain it was built from has changed. Its level
# comes multiplied by presence, so filtering and mipmaps weight it by where mist can lie and the
# shader divides it back out, instead of averaging shore levels towards zero.
func _refresh_mist_map() -> void:
	var generation: int = _simulation.get_mist_map_generation()
	if generation == _mist_map_generation:
		return
	_mist_map_generation = generation
	var map: Dictionary = _simulation.get_mist_map()
	var width: int = map.get("width", 0)
	var height: int = map.get("height", 0)
	var data: PackedFloat32Array = map.get("data", PackedFloat32Array())
	var top: float = map.get("top_level", -INF)
	if width <= 0 or height <= 0 or data.size() != width * height * 2 or top == -INF:
		_mist_effect.map_texture = null
		return
	var image := Image.create_from_data(width, height, false, Image.FORMAT_RGF, data.to_byte_array())
	image.generate_mipmaps()
	_mist_effect.map_texture = ImageTexture.create_from_image(image)
	_mist_effect.map_origin = Vector2(map.origin_x, map.origin_z)
	_mist_effect.map_cell_m = map.cell_m
	_mist_effect.map_size = Vector2i(width, height)
	# Five scale heights above the highest lake, where the mist has thinned below a hundredth.
	_mist_effect.ceiling = top + 5.0 * MIST_SCALE_HEIGHT_M

func _applied_cloud_coverage() -> float:
	if _simulation != null and _simulation.has_method("get_weather_clouds"):
		return (_simulation.get_weather_clouds() as Vector4).z
	return FALLBACK_WEATHER_CLOUDS.z

func _apply_sky_palette(sample: DayCycleConfig.Sample) -> void:
	# The cubemap also carries the clouds' mean cover, so a weather change refreshes it too; drift
	# never does, and a transition does in a few steps.
	var coverage := _applied_cloud_coverage()
	if (
		absf(coverage - _sky_applied_coverage) < SKY_UPDATE_MIN_COVERAGE
		and absf(sample.sun_elevation_deg - _sky_applied_elevation_deg) < SKY_UPDATE_MIN_DEG
		and absf(
			angle_difference(
				deg_to_rad(sample.sun_azimuth_deg), deg_to_rad(_sky_applied_azimuth_deg)
			)
		) < deg_to_rad(SKY_UPDATE_MIN_DEG)
	):
		return
	_sky_applied_elevation_deg = sample.sun_elevation_deg
	_sky_applied_azimuth_deg = sample.sun_azimuth_deg
	_sky_applied_coverage = coverage
	if _sky_material is ShaderMaterial:
		var shader_sky := _sky_material as ShaderMaterial
		shader_sky.set_shader_parameter("cloud_cover", coverage)
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
	_cloud_effect = WeatherCloudEffect.new()
	_mist_effect = WeatherMistEffect.new()
	var compositor := Compositor.new()
	compositor.compositor_effects = [_cloud_effect, _mist_effect]
	world_environment.compositor = compositor
	RenderingServer.global_shader_parameter_set(
		"weather_cloud_reflection", _cloud_effect.reflection_texture
	)
	RenderingServer.global_shader_parameter_set(
		"weather_cloud_shadow_map", _cloud_effect.shadow_texture
	)

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
	var material := ShaderMaterial.new()
	material.shader = SKY_CLOUD_SHADER
	material.set_shader_parameter("hemisphere_curve", SKY_HEMISPHERE_CURVE)
	material.set_shader_parameter("sun_curve", SKY_SUN_CURVE)
	# The moon's appearance is fixed, so it is set here rather than rewritten every frame. Only
	# its direction and brightness change with the cycle.
	material.set_shader_parameter("moon_disk_color", DayCycleConfig.MOON_DISK_COLOR)
	material.set_shader_parameter("moon_disk_deg", DayCycleConfig.MOON_DISK_DEG)
	material.set_shader_parameter("moon_halo_deg", DayCycleConfig.MOON_HALO_DEG)
	material.set_shader_parameter("moon_halo_strength", DayCycleConfig.MOON_HALO_STRENGTH)
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
	# The sky shader places its own sun and moon disks so their brightness can differ from the
	# key light's energy. Letting the engine draw a third disk from this light would double it.
	sun.sky_mode = DirectionalLight3D.SKY_MODE_LIGHT_ONLY
	sun.shadow_enabled = true
	sun.shadow_bias = SHADOW_BIAS
	sun.shadow_normal_bias = SHADOW_NORMAL_BIAS
	sun.set("directional_shadow_max_distance", shadow_max_distance_m())
	sun.set("directional_shadow_split_1", SHADOW_SPLIT_1)
	sun.set("directional_shadow_split_2", SHADOW_SPLIT_2)
	sun.set("directional_shadow_split_3", SHADOW_SPLIT_3)
	sun.set("directional_shadow_fade_start", SHADOW_FADE_START)
	_apply_shadow_quality(sun)

## Re-reads the player's graphics settings after Options applies them.
func apply_graphics_settings() -> void:
	if is_instance_valid(_sun):
		_apply_shadow_quality(_sun)

func _apply_shadow_quality(sun: DirectionalLight3D) -> void:
	var low := GameSettings.get_shadow_quality() == GameSettings.SHADOW_QUALITY_LOW
	# Angular distance drives Godot's variable-penumbra path; zero makes the directional shadow a
	# plain PCF lookup. Two splits instead of four, and no split blending, halve the cascade
	# sampling again.
	sun.light_angular_distance = 0.0 if low else SUN_ANGULAR_DISTANCE_DEG
	sun.shadow_blur = 0.0 if low else SHADOW_BLUR
	sun.set("directional_shadow_mode", 1 if low else 2)
	sun.set("directional_shadow_blend_splits", not low)
	RenderingServer.directional_soft_shadow_filter_set_quality(
		RenderingServer.SHADOW_QUALITY_HARD if low
		else ProjectSettings.get_setting("rendering/lights_and_shadows/directional_shadow/soft_shadow_filter_quality"))

func _print_debug_if_requested(scene_root: Node) -> void:
	if not is_lighting_debug_enabled():
		return
	await get_tree().process_frame
	if not is_inside_tree():
		return
	_update_distance_fade()
	var splits := shadow_split_distances()
	var hour := _day_sample.day_fraction * 24.0
	MetrumDebug.log(
		"lighting",
		"time=%02d:%02d sun_elevation=%.2f sun_azimuth=%.2f moonlit=%s key_color=%s key_energy=%.3f sky=%s fog=%s ambient=%.3f ambient_scale=%s shadow_max=%.1f splits=(%.1f,%.1f,%.1f) far_fade=(%.1f,%.1f) ground_shadow=(ambient=%.2f sun=%.2f min=%.2f)"
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
		MetrumDebug.log(
			"lighting",
			"shadow_policy %s total=%d visible=%d policy_casts=%d policy_receives=%d off=%d on=%d double_sided=%d shadows_only=%d multimeshes=%d nonempty_multimeshes=%d multimesh_instances=%d max_extra_cull=%.1f"
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
