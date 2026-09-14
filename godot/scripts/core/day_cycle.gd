# SPDX-License-Identifier: GPL-2.0-only

## Deterministic day/night lighting palette sampled from the operational clock.
##
## [method sample_into] is a pure function of the day fraction: the same fraction always
## resolves to the same sun position and the same colours, with no engine state, no wall
## clock and no randomness. [code]scene_lighting.gd[/code] owns applying the result to the
## scene; this file owns only the curve, so the palette stays headlessly testable and can be
## retuned without touching the renderer.
##
## The sun follows a real solar-position solve for a fixed latitude and declination, so the
## arc, the day length and the length of the golden hour are consequences of one pair of
## constants rather than hand-placed keyframes. Colours are keyed to the resulting solar
## elevation, not to the clock, which makes sunrise and sunset symmetric for free and keeps
## the palette correct if the latitude is ever retuned.
extends RefCounted
class_name DayCycle

# Kuopio, the reference the art direction is being judged against, sits near 63 N. The
# declination is held at a late-spring value rather than swept seasonally: it buys a noon sun
# at 45 degrees, a long low golden hour, and a night that is a real night without eating the
# session. A seasonal sweep is a separate feature, not a prerequisite for this one.
const LATITUDE_DEG := 60.0
const SOLAR_DECLINATION_DEG := 15.0
const HOUR_ANGLE_PER_HOUR_DEG := 15.0

## Hour used by scenes that have no simulation clock to read, such as the asset editor.
const FALLBACK_HOUR := 10.5

# The moon is treated as full and antisolar: it rises as the sun sets. That is the cheapest
# believable model, it needs no second orbit, and it keeps night shadowed instead of flat.
const MOON_COLOR := Color(0.58, 0.70, 1.00, 1.0)
const MOON_ENERGY := 0.26
const MOON_INDIRECT_ENERGY := 0.10
const MOON_DISK_COLOR := Color(0.92, 0.94, 1.00, 1.0)
const MOON_DISK_INTENSITY := 0.85
const MOON_DISK_DEG := 0.62
const MOON_HALO_DEG := 3.20
const MOON_HALO_STRENGTH := 0.06

const SUN_ENERGY_PEAK := 1.65
const SUN_INDIRECT_ENERGY_PEAK := 0.35

# Palette stops keyed by true solar elevation in degrees. Anything outside the range clamps
# to the end stop; anything between blends linearly, so the curve is continuous everywhere.
const STOP_ELEVATION_DEG := [-16.0, -8.0, -4.0, -1.2, 0.0, 2.5, 6.0, 14.0, 26.0, 42.0]

# Stop spacing is a continuity budget, not decoration. The sun crosses the horizon at up to
# 0.107 degrees per authored minute, so a stop gap of G degrees can only carry a colour change
# of about 0.19 * G before the transition reads as a wipe rather than as dusk. The stops
# bunch up around the horizon because that is where the palette moves fastest.
#
# Stop 4 sits exactly on the horizon and is the last one with no sun energy, which is what
# guarantees the key light can never shine upward through the ground.

# Directional light tint. Neutral overhead so the warm low-sun stops read as a shift rather
# than as the scene's normal colour.
const KEY_COLOR := [
	Color(0.55, 0.66, 0.95),
	Color(0.58, 0.68, 0.95),
	Color(0.70, 0.66, 0.80),
	Color(0.90, 0.62, 0.46),
	Color(1.00, 0.66, 0.40),
	Color(1.00, 0.74, 0.48),
	Color(1.00, 0.86, 0.68),
	Color(1.00, 0.94, 0.85),
	Color(1.00, 0.945, 0.860),
	Color(1.00, 0.95, 0.87),
]

# Fraction of SUN_ENERGY_PEAK. Zero at and below the horizon: twilight is carried by the sky
# and the ambient term, never by a directional light shining up through the ground.
# Physically the sun at two degrees is attenuated to a few percent. That is not what this
# curve models: with no auto-exposure, an honestly attenuated low sun just makes the scene dark
# and the warm colour never lands. The ramp is steep enough that golden hour actually keys the
# scene, and the ambient stops below drop at the same time so the result reads as contrast
# rather than as an overall brightness change.
const KEY_ENERGY_SCALE := [0.0, 0.0, 0.0, 0.0, 0.0, 0.42, 0.72, 0.92, 0.98, 1.00]

const SKY_ZENITH := [
	Color(0.018, 0.026, 0.058),
	Color(0.045, 0.070, 0.140),
	Color(0.095, 0.150, 0.275),
	Color(0.140, 0.210, 0.365),
	Color(0.170, 0.255, 0.420),
	Color(0.215, 0.345, 0.530),
	Color(0.245, 0.410, 0.615),
	Color(0.275, 0.475, 0.700),
	Color(0.295, 0.512, 0.750),
	Color(0.300, 0.520, 0.760),
]

const SKY_HORIZON := [
	Color(0.040, 0.055, 0.100),
	Color(0.090, 0.115, 0.195),
	Color(0.185, 0.185, 0.265),
	Color(0.360, 0.265, 0.300),
	Color(0.480, 0.325, 0.300),
	Color(0.680, 0.480, 0.375),
	Color(0.720, 0.575, 0.470),
	Color(0.680, 0.680, 0.680),
	Color(0.615, 0.720, 0.825),
	Color(0.580, 0.730, 0.860),
]

# Aerial perspective reads the haze colour, so it is kept deliberately less saturated than the
# horizon band above it. A fully orange horizon colour on the fog would tint the whole distant
# landscape at sunset instead of only the sky behind it.
const FOG_COLOR := [
	Color(0.030, 0.040, 0.075),
	Color(0.070, 0.090, 0.150),
	Color(0.140, 0.155, 0.215),
	Color(0.245, 0.230, 0.265),
	Color(0.440, 0.375, 0.350),
	Color(0.640, 0.535, 0.470),
	Color(0.665, 0.620, 0.575),
	Color(0.585, 0.620, 0.650),
	Color(0.578, 0.700, 0.815),
	Color(0.580, 0.730, 0.860),
]

const AMBIENT_COLOR := [
	Color(0.10, 0.14, 0.26),
	Color(0.14, 0.19, 0.33),
	Color(0.21, 0.26, 0.42),
	Color(0.28, 0.33, 0.48),
	Color(0.40, 0.38, 0.44),
	Color(0.50, 0.47, 0.50),
	Color(0.54, 0.54, 0.58),
	Color(0.54, 0.60, 0.68),
	Color(0.54, 0.63, 0.70),
	Color(0.54, 0.64, 0.70),
]

const AMBIENT_ENERGY := [0.16, 0.19, 0.24, 0.29, 0.30, 0.33, 0.37, 0.42, 0.44, 0.44]

# Multiplier for the shaders that bake their own ambient floor into EMISSION. Without it the
# ground keeps its full daytime brightness at midnight no matter what the environment says.
# Night bottoms out near a fifth of daylight rather than at a physical value: the city has no
# street lighting yet, and an unreadable board is not a look.
const AMBIENT_LIGHT_SCALE := [
	Color(0.115, 0.135, 0.215),
	Color(0.150, 0.180, 0.275),
	Color(0.245, 0.285, 0.400),
	Color(0.42, 0.45, 0.56),
	Color(0.50, 0.46, 0.48),
	Color(0.64, 0.59, 0.56),
	Color(0.78, 0.75, 0.75),
	Color(0.94, 0.94, 0.94),
	Color(1.00, 1.00, 1.00),
	Color(1.00, 1.00, 1.00),
]

# How far the ground albedo is pulled toward its own luminance. Night is desaturated, not just
# dark: the ambient tint is a multiplier, so on its own it leaves a vivid green ground reading
# as daylight with the brightness turned down.
const AMBIENT_DESATURATION := [0.65, 0.61, 0.50, 0.33, 0.21, 0.10, 0.02, 0.0, 0.0, 0.0]

const CLOUD_SHADOW := [
	Color(0.045, 0.055, 0.090),
	Color(0.080, 0.100, 0.155),
	Color(0.150, 0.165, 0.235),
	Color(0.245, 0.230, 0.275),
	Color(0.300, 0.265, 0.295),
	Color(0.400, 0.360, 0.375),
	Color(0.455, 0.440, 0.455),
	Color(0.505, 0.520, 0.555),
	Color(0.518, 0.572, 0.632),
	Color(0.520, 0.580, 0.640),
]

const CLOUD_LIGHT := [
	Color(0.100, 0.120, 0.180),
	Color(0.160, 0.190, 0.270),
	Color(0.290, 0.290, 0.370),
	Color(0.480, 0.400, 0.410),
	Color(0.590, 0.455, 0.420),
	Color(0.800, 0.610, 0.490),
	Color(0.880, 0.720, 0.580),
	Color(0.930, 0.860, 0.790),
	Color(0.895, 0.905, 0.925),
	Color(0.880, 0.910, 0.940),
]

# Disk brightness is independent of how much the sun lights the ground, so a setting sun can
# stay visible as a disk after it has stopped being a useful key light.
const SUN_DISK_INTENSITY := [0.0, 0.0, 0.0, 0.22, 0.55, 1.05, 1.25, 1.38, 1.44, 1.45]

# A low sun reads larger and hazier than an overhead one. One factor drives all three of the
# disk size, the halo width and the halo strength, which keeps the sunset tuned in one place.
const SUN_DISK_DEG_LOW := 1.05
const SUN_DISK_DEG_HIGH := 0.62
const SUN_HALO_DEG_LOW := 15.0
const SUN_HALO_DEG_HIGH := 7.5
const SUN_HALO_STRENGTH_LOW := 0.55
const SUN_HALO_STRENGTH_HIGH := 0.20
const LOW_SUN_FADE_END_DEG := 18.0

# How much of the key light's colour the depth fog scatters back toward the viewer. Uniform
# haze reads as a flat wall at the horizon; scattering makes the air bright toward the sun and
# cool away from it, which is most of what makes a low sun read as golden rather than as smog.
const FOG_SUN_SCATTER_LOW := 0.55
const FOG_SUN_SCATTER_HIGH := 0.06

## One sampled lighting state.
##
## Reused across frames: [method DayCycle.sample_into] writes every field in place, so the
## per-frame path allocates nothing.
class Sample:
	extends RefCounted

	## Day fraction this sample was taken at, in `0.0..1.0`.
	var day_fraction := 0.0
	## True solar elevation in degrees. Negative when the sun is below the horizon.
	var sun_elevation_deg := 0.0
	## Solar azimuth in degrees, measured from north and increasing eastward.
	var sun_azimuth_deg := 180.0
	## Unit vector pointing from the world toward the sun, whether or not it is up.
	var sun_direction := Vector3.UP
	## Unit vector pointing toward whichever body is the current key light.
	var key_direction := Vector3.UP
	## Key light tint.
	var key_color := Color.WHITE
	## Key light energy. Zero through the dead band between sunset and moonrise.
	var key_energy := 0.0
	## Key light indirect energy.
	var key_indirect_energy := 0.0
	## True while the moon rather than the sun is the key light.
	var is_moonlit := false
	## Sky dome colour straight up.
	var sky_zenith := Color.WHITE
	## Sky dome colour at the horizon. Also the base for the nadir band.
	var sky_horizon := Color.WHITE
	## Sky dome colour straight down, below the horizon band.
	var sky_nadir := Color.WHITE
	## Unlit side of the cloud layer.
	var cloud_shadow := Color.WHITE
	## Lit side of the cloud layer.
	var cloud_light := Color.WHITE
	## Environment ambient colour.
	var ambient_color := Color.WHITE
	## Environment ambient energy.
	var ambient_energy := 0.0
	## Multiplier for shaders that bake their own ambient floor rather than reading the
	## environment. White at noon, dim blue at midnight.
	var ambient_light_scale := Color.WHITE
	## How far the ground albedo is pulled toward luminance. Zero by day.
	var ambient_desaturation := 0.0
	## Depth-fog colour, which is what carries aerial perspective.
	var fog_color := Color.WHITE
	## How strongly the depth fog scatters the key light's colour toward the viewer.
	var fog_sun_scatter := FOG_SUN_SCATTER_HIGH
	## Direction toward the sun disk drawn in the sky shader.
	var sun_disk_direction := Vector3.UP
	## Sun disk tint.
	var sun_disk_color := Color.WHITE
	## Sun disk brightness. Zero once the sun is well below the horizon.
	var sun_disk_intensity := 0.0
	## Sun disk angular diameter in degrees.
	var sun_disk_deg := SUN_DISK_DEG_HIGH
	## Sun halo angular radius in degrees.
	var sun_halo_deg := SUN_HALO_DEG_HIGH
	## Sun halo strength.
	var sun_halo_strength := SUN_HALO_STRENGTH_HIGH
	## Direction toward the moon disk drawn in the sky shader.
	var moon_disk_direction := Vector3.DOWN
	## Moon disk brightness. Zero while the moon is below the horizon.
	var moon_disk_intensity := 0.0

## Returns the solar elevation and azimuth in degrees for a day fraction.
##
## `x` is elevation above the horizon, `y` is azimuth measured from north and increasing
## eastward, so the sun runs east at dawn, south at noon and west at dusk.
static func solar_position_deg(day_fraction: float) -> Vector2:
	var hour_angle := deg_to_rad(
		(fposmod(day_fraction, 1.0) * 24.0 - 12.0) * HOUR_ANGLE_PER_HOUR_DEG
	)
	var latitude := deg_to_rad(LATITUDE_DEG)
	var declination := deg_to_rad(SOLAR_DECLINATION_DEG)
	var sin_elevation := (
		sin(latitude) * sin(declination)
		+ cos(latitude) * cos(declination) * cos(hour_angle)
	)
	var elevation := asin(clampf(sin_elevation, -1.0, 1.0))
	var azimuth := atan2(
		-cos(declination) * sin(hour_angle),
		cos(latitude) * sin(declination) - sin(latitude) * cos(declination) * cos(hour_angle)
	)
	# atan2 returns (-180, 180]; the documented convention is a compass bearing, so due south
	# reads as 180 rather than as -180 and the afternoon reads as 180..360 rather than negative.
	return Vector2(rad_to_deg(elevation), fposmod(rad_to_deg(azimuth), 360.0))

## Converts an elevation/azimuth pair in degrees to a world direction.
##
## North is -Z and east is +X, matching the hillshade convention already used by the terrain
## and site-ground shaders.
static func direction_from_position(elevation_deg: float, azimuth_deg: float) -> Vector3:
	var elevation := deg_to_rad(elevation_deg)
	var azimuth := deg_to_rad(azimuth_deg)
	var horizontal := cos(elevation)
	return Vector3(
		sin(azimuth) * horizontal,
		sin(elevation),
		-cos(azimuth) * horizontal
	).normalized()

## Returns a fresh reusable sample. Callers keep one and pass it to [method sample_into].
static func create_sample() -> Sample:
	return Sample.new()

## Writes the lighting state for `day_fraction` into `out`.
##
## Pure and allocation-free: the same fraction always produces the same values, and nothing
## outside `out` is touched.
static func sample_into(day_fraction: float, out: Sample) -> void:
	var fraction := fposmod(day_fraction, 1.0)
	var position := solar_position_deg(fraction)
	var elevation: float = position.x
	var azimuth: float = position.y
	var at := _locate(elevation)

	out.day_fraction = fraction
	out.sun_elevation_deg = elevation
	out.sun_azimuth_deg = azimuth
	out.sun_direction = direction_from_position(elevation, azimuth)

	out.sky_zenith = _color_at(SKY_ZENITH, at)
	out.sky_horizon = _color_at(SKY_HORIZON, at)
	# The nadir band tracks the horizon rather than carrying its own table. The ratio is the
	# one the authored daytime palette already used.
	out.sky_nadir = Color(
		out.sky_horizon.r * 0.76, out.sky_horizon.g * 0.76, out.sky_horizon.b * 0.76
	)
	out.cloud_shadow = _color_at(CLOUD_SHADOW, at)
	out.cloud_light = _color_at(CLOUD_LIGHT, at)
	out.ambient_color = _color_at(AMBIENT_COLOR, at)
	out.ambient_energy = _float_at(AMBIENT_ENERGY, at)
	out.ambient_light_scale = _color_at(AMBIENT_LIGHT_SCALE, at)
	out.ambient_desaturation = _float_at(AMBIENT_DESATURATION, at)
	out.fog_color = _color_at(FOG_COLOR, at)

	var low_sun := 1.0 - smoothstep(0.0, LOW_SUN_FADE_END_DEG, elevation)
	out.sun_disk_direction = out.sun_direction
	out.sun_disk_color = _color_at(KEY_COLOR, at)
	out.sun_disk_intensity = _float_at(SUN_DISK_INTENSITY, at)
	out.fog_sun_scatter = lerpf(FOG_SUN_SCATTER_HIGH, FOG_SUN_SCATTER_LOW, low_sun)
	out.sun_disk_deg = lerpf(SUN_DISK_DEG_HIGH, SUN_DISK_DEG_LOW, low_sun)
	out.sun_halo_deg = lerpf(SUN_HALO_DEG_HIGH, SUN_HALO_DEG_LOW, low_sun)
	out.sun_halo_strength = lerpf(SUN_HALO_STRENGTH_HIGH, SUN_HALO_STRENGTH_LOW, low_sun)

	# The moon is antisolar, so its elevation is the sun's mirrored. It fades in over the
	# window where the sun has already stopped contributing, which means the key light can be
	# swapped while both bodies are contributing nothing and the swap cannot be seen.
	var moon_gate := smoothstep(-1.0, -7.0, elevation)
	out.moon_disk_direction = direction_from_position(-elevation, azimuth + 180.0)
	out.moon_disk_intensity = MOON_DISK_INTENSITY * moon_gate

	var sun_energy := _float_at(KEY_ENERGY_SCALE, at) * SUN_ENERGY_PEAK
	if sun_energy > 0.0:
		out.is_moonlit = false
		out.key_direction = out.sun_direction
		out.key_color = _color_at(KEY_COLOR, at)
		out.key_energy = sun_energy
		out.key_indirect_energy = (
			_float_at(KEY_ENERGY_SCALE, at) * SUN_INDIRECT_ENERGY_PEAK
		)
	else:
		out.is_moonlit = true
		out.key_direction = out.moon_disk_direction
		out.key_color = MOON_COLOR
		out.key_energy = MOON_ENERGY * moon_gate
		out.key_indirect_energy = MOON_INDIRECT_ENERGY * moon_gate

## Returns the day fraction a pinned clock override requests, or `-1.0` when none is set.
##
## `METRUM_TIME_OF_DAY` accepts an hour of day such as `20.5`. It exists so screenshots and
## GPU probe trials can hold one lighting state instead of drifting through the cycle.
static func pinned_day_fraction() -> float:
	var requested := OS.get_environment("METRUM_TIME_OF_DAY").strip_edges()
	if requested.is_empty() or not requested.is_valid_float():
		return -1.0
	return fposmod(requested.to_float() / 24.0, 1.0)

# Returns the lower stop index in `x` and the blend weight toward the next stop in `y`.
static func _locate(elevation_deg: float) -> Vector2:
	var last := STOP_ELEVATION_DEG.size() - 1
	if elevation_deg <= float(STOP_ELEVATION_DEG[0]):
		return Vector2(0.0, 0.0)
	if elevation_deg >= float(STOP_ELEVATION_DEG[last]):
		return Vector2(float(last), 0.0)
	for index in range(last):
		var upper := float(STOP_ELEVATION_DEG[index + 1])
		if elevation_deg < upper:
			var lower := float(STOP_ELEVATION_DEG[index])
			return Vector2(float(index), (elevation_deg - lower) / (upper - lower))
	return Vector2(float(last), 0.0)

static func _color_at(stops: Array, at: Vector2) -> Color:
	var index := int(at.x)
	if at.y <= 0.0:
		return stops[index]
	return (stops[index] as Color).lerp(stops[index + 1], at.y)

static func _float_at(stops: Array, at: Vector2) -> float:
	var index := int(at.x)
	if at.y <= 0.0:
		return float(stops[index])
	return lerpf(float(stops[index]), float(stops[index + 1]), at.y)
