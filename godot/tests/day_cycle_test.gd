# SPDX-License-Identifier: GPL-2.0-only

## Headless contract test for the day/night lighting curve.
## The palette is a pure function of the operational day fraction, so the whole cycle can be
## swept without an engine, a scene or a simulation. The properties asserted here are the ones
## a screenshot cannot check cheaply: that the curve is continuous everywhere including the
## sun-to-moon handover, that no key light ever shines up through the ground, and that the same
## clock reading always resolves to the same lighting.
extends SceneTree

const DayCycleConfig := preload("res://scripts/core/day_cycle.gd")

# One sample per authored minute: the finest step the simulation clock can actually produce.
const STEPS := 1440
const MAX_ENERGY_STEP := 0.05
const MAX_CHANNEL_STEP := 0.02

var _failures := 0

func _initialize() -> void:
	_test_direction_conversion()
	_test_solar_arc()
	_test_determinism()
	_test_key_light_never_shines_from_below()
	_test_curve_is_continuous()
	_test_night_is_dark_but_readable()
	_test_moon_keeps_a_month()
	_test_sky_turns_with_the_sun()
	quit(1 if _failures > 0 else 0)

func _test_direction_conversion() -> void:
	# North is -Z and east is +X, matching the hillshade convention in the ground shaders.
	var north := DayCycleConfig.direction_from_position(0.0, 0.0)
	_expect(north.is_equal_approx(Vector3(0.0, 0.0, -1.0)), "azimuth 0 must point north, got %s" % north)
	var east := DayCycleConfig.direction_from_position(0.0, 90.0)
	_expect(east.is_equal_approx(Vector3(1.0, 0.0, 0.0)), "azimuth 90 must point east, got %s" % east)
	var up := DayCycleConfig.direction_from_position(90.0, 0.0)
	_expect(up.is_equal_approx(Vector3.UP), "elevation 90 must point up, got %s" % up)

func _test_solar_arc() -> void:
	var noon := DayCycleConfig.solar_position_deg(0.5)
	var midnight := DayCycleConfig.solar_position_deg(0.0)
	_expect(
		absf(noon.y - 180.0) < 0.001,
		"the sun must be due south at noon, got azimuth %.3f" % noon.y
	)
	_expect(noon.x > 0.0, "the sun must be up at noon, got elevation %.3f" % noon.x)
	_expect(midnight.x < 0.0, "the sun must be down at midnight, got elevation %.3f" % midnight.x)

	# Noon is the daily maximum, and the arc is symmetric about it, which is what makes
	# sunrise and sunset share one palette.
	var highest := -90.0
	for step in range(STEPS):
		var elevation: float = DayCycleConfig.solar_position_deg(float(step) / STEPS).x
		highest = maxf(highest, elevation)
	_expect(
		absf(highest - noon.x) < 0.01,
		"noon must be the daily maximum: noon %.3f, highest %.3f" % [noon.x, highest]
	)
	for step in range(1, 12):
		var fraction := 0.5 - float(step) * 0.04
		var before: float = DayCycleConfig.solar_position_deg(fraction).x
		var after: float = DayCycleConfig.solar_position_deg(1.0 - fraction).x
		_expect(
			absf(before - after) < 0.001,
			"morning and evening elevations must mirror at %.3f: %.3f vs %.3f"
			% [fraction, before, after]
		)

	# Morning sun in the east, evening sun in the west.
	_expect(
		DayCycleConfig.solar_position_deg(0.3).y < 180.0,
		"the morning sun must sit east of south"
	)
	_expect(
		DayCycleConfig.solar_position_deg(0.7).y > 180.0,
		"the evening sun must sit west of south"
	)

func _test_determinism() -> void:
	var first := DayCycleConfig.create_sample()
	var second := DayCycleConfig.create_sample()
	for hour in [0.0, 4.2, 7.5, 12.0, 19.9, 21.3, 23.99]:
		var fraction: float = float(hour) / 24.0
		DayCycleConfig.sample_into(fraction, first)
		DayCycleConfig.sample_into(fraction, second)
		_expect(_digest(first) == _digest(second), "hour %s must resolve identically" % hour)

	# A reused sample must not carry state between calls: sampling noon then dusk then noon
	# again has to land back on the first result.
	DayCycleConfig.sample_into(0.5, first)
	var noon := _digest(first)
	DayCycleConfig.sample_into(0.86, first)
	DayCycleConfig.sample_into(0.5, first)
	_expect(_digest(first) == noon, "a reused sample must not accumulate state")

	# The fraction wraps rather than clamping, so midnight is one lighting state, not two.
	DayCycleConfig.sample_into(0.0, first)
	DayCycleConfig.sample_into(1.0, second)
	_expect(_digest(first) == _digest(second), "the day must wrap cleanly at midnight")

func _test_key_light_never_shines_from_below() -> void:
	var sample := DayCycleConfig.create_sample()
	for step in range(STEPS):
		DayCycleConfig.sample_into(float(step) / STEPS, sample)
		if sample.key_energy <= 0.0:
			continue
		_expect(
			sample.key_direction.y > 0.0,
			(
				"a lit key must stay above the horizon: step %d elevation %.3f key.y %.4f"
				% [step, sample.sun_elevation_deg, sample.key_direction.y]
			)
		)
		_expect(
			sample.is_moonlit == sample.key_direction.is_equal_approx(sample.moon_disk_direction),
			"the moonlit flag must match which body the key is following at step %d" % step
		)

func _test_curve_is_continuous() -> void:
	var previous := DayCycleConfig.create_sample()
	var current := DayCycleConfig.create_sample()
	DayCycleConfig.sample_into(0.0, previous)
	# Across midnight into the next day, as the clock carries the day number over.
	for step in range(1, STEPS + 1):
		DayCycleConfig.sample_into(float(step) / STEPS, current, int(step == STEPS))
		_expect(
			absf(current.key_energy - previous.key_energy) <= MAX_ENERGY_STEP,
			(
				"key energy must not jump: step %d %.4f -> %.4f"
				% [step, previous.key_energy, current.key_energy]
			)
		)
		_expect(
			absf(current.ambient_desaturation - previous.ambient_desaturation) <= MAX_CHANNEL_STEP,
			(
				"desaturation must not jump: step %d %.4f -> %.4f"
				% [step, previous.ambient_desaturation, current.ambient_desaturation]
			)
		)
		for pair in [
			["sky_horizon", previous.sky_horizon, current.sky_horizon],
			["fog", previous.fog_color, current.fog_color],
			["ambient", previous.ambient_color, current.ambient_color],
			["ambient_scale", previous.ambient_light_scale, current.ambient_light_scale],
		]:
			var before: Color = pair[1]
			var after: Color = pair[2]
			var delta := maxf(
				maxf(absf(after.r - before.r), absf(after.g - before.g)),
				absf(after.b - before.b)
			)
			_expect(
				delta <= MAX_CHANNEL_STEP,
				"%s must not jump: step %d delta %.4f" % [pair[0], step, delta]
			)
		# The key light may swap bodies only while it is contributing nothing, which is what
		# makes the 180 degree direction change invisible.
		if previous.key_energy > 0.0 and current.key_energy > 0.0:
			_expect(
				previous.key_direction.dot(current.key_direction) > 0.99,
				(
					"the key direction must not jump while lit: step %d dot %.4f"
					% [step, previous.key_direction.dot(current.key_direction)]
				)
			)
		DayCycleConfig.sample_into(float(step) / STEPS, previous, int(step == STEPS))

func _test_night_is_dark_but_readable() -> void:
	var night := DayCycleConfig.create_sample()
	var noon := DayCycleConfig.create_sample()
	DayCycleConfig.sample_into(0.0, night)
	DayCycleConfig.sample_into(0.5, noon)
	_expect(night.is_moonlit, "midnight must be moonlit")
	_expect(not noon.is_moonlit, "noon must be sunlit")
	_expect(
		night.ambient_light_scale.g < noon.ambient_light_scale.g * 0.5,
		"night must be materially darker than noon"
	)
	# The city has no street lighting yet, so a physically dark night would be an unreadable
	# board rather than a look.
	_expect(
		night.ambient_light_scale.g > 0.05,
		"night must stay readable, got %.3f" % night.ambient_light_scale.g
	)
	_expect(night.key_energy > 0.0, "moonlight must still shape the ground at midnight")
	_expect(
		night.ambient_desaturation > 0.5 and noon.ambient_desaturation == 0.0,
		"night must wash colour out while noon leaves it alone"
	)
	_expect(
		night.fog_color.get_luminance() < noon.fog_color.get_luminance(),
		"the far haze must darken at night instead of leaving a daytime curtain"
	)
	_expect(
		night.sky_horizon.b > night.sky_horizon.r,
		"the night sky must be cool, got %s" % night.sky_horizon
	)

	# Golden hour, then blue hour. The probes are found by solar elevation rather than by clock
	# time so they keep testing the same moment of the arc if the latitude is ever retuned.
	var sunset := DayCycleConfig.create_sample()
	DayCycleConfig.sample_into(_evening_fraction_at_elevation(0.0), sunset)
	_expect(
		sunset.sky_horizon.r > sunset.sky_horizon.b,
		"the horizon must be warm at sunset, got %s" % sunset.sky_horizon
	)
	_expect(
		sunset.key_energy < 0.10,
		"a sun on the horizon must have stopped keying the scene, got %.3f" % sunset.key_energy
	)
	_expect(
		sunset.sun_disk_intensity > 0.0,
		"the sun disk must still be drawn once the sun has stopped lighting the ground"
	)
	_expect(
		sunset.fog_color.r > sunset.fog_color.b,
		"the aerial haze must warm with the horizon, got %s" % sunset.fog_color
	)

	var blue_hour := DayCycleConfig.create_sample()
	DayCycleConfig.sample_into(_evening_fraction_at_elevation(-4.0), blue_hour)
	_expect(
		blue_hour.sky_horizon.b > blue_hour.sky_horizon.r,
		"the blue hour horizon must turn cool, got %s" % blue_hour.sky_horizon
	)
	_expect(
		blue_hour.sky_horizon.get_luminance() < sunset.sky_horizon.get_luminance(),
		"the blue hour must be darker than the sunset that precedes it"
	)

# Returns the evening fraction whose solar elevation is closest to a target.
func _evening_fraction_at_elevation(target_deg: float) -> float:
	var best := 0.5
	var best_error := INF
	for step in range(STEPS / 2, STEPS):
		var fraction := float(step) / STEPS
		var error: float = absf(DayCycleConfig.solar_position_deg(fraction).x - target_deg)
		if error < best_error:
			best_error = error
			best = fraction
	return best

func _test_moon_keeps_a_month() -> void:
	var sample := DayCycleConfig.create_sample()
	var month := DayCycleConfig.SYNODIC_MONTH_DAYS
	# Through a month of minutes a lit moon stays above the horizon, never lights the day, and
	# the key never jumps from one body to the other while lit.
	for step in range(int(month) * 96):
		var days := float(step) / 96.0
		DayCycleConfig.sample_into(fposmod(days, 1.0), sample, int(days))
		if sample.is_moonlit and sample.key_energy > 0.0:
			_expect(
				sample.moon_elevation_deg > 0.0,
				"moonlight must come from above the horizon at day %.3f" % days
			)
		if sample.sun_elevation_deg > 1.0:
			_expect(not sample.is_moonlit, "the moon must not light the day at day %.3f" % days)
	# It drifts east among the stars by a twelfth of a turn a day, so it rises later each day.
	var start := DayCycleConfig.moon_celestial_direction(0.0)
	var next := DayCycleConfig.moon_celestial_direction(1.0)
	var drift := rad_to_deg(Vector2(next.x, next.y).angle() - Vector2(start.x, start.y).angle())
	_expect(
		drift > 10.0 and drift < 15.0,
		"the moon must move east about 12 degrees a day among the stars, got %.2f" % drift
	)
	# Full when it stands opposite the sun, new beside it; never a different moon for the same
	# time.
	var phases := []
	for quarter in range(4):
		var days := month * (float(quarter) * 0.25) - DayCycleConfig.MOON_AGE_AT_DAY_ZERO
		DayCycleConfig.sample_into(fposmod(days, 1.0), sample, int(floor(days)))
		phases.append(sample.moon_illumination)
	_expect(phases[0] < 0.01, "a new moon must be dark, got %.3f" % phases[0])
	_expect(absf(phases[1] - 0.5) < 0.05, "a first quarter must be half lit, got %.3f" % phases[1])
	_expect(phases[2] > 0.99, "a full moon must be fully lit, got %.3f" % phases[2])
	_expect(absf(phases[3] - 0.5) < 0.05, "a last quarter must be half lit, got %.3f" % phases[3])

func _test_sky_turns_with_the_sun() -> void:
	# The night sky turns with the sun's hour angle, so the sun keeps one place among the stars
	# (right ascension zero at its declination) all day, and the pole stands still.
	var sample := DayCycleConfig.create_sample()
	var declination := deg_to_rad(DayCycleConfig.SOLAR_DECLINATION_DEG)
	var sun_fixed := Vector3(cos(declination), 0.0, sin(declination))
	var pole := DayCycleConfig.direction_from_position(DayCycleConfig.LATITUDE_DEG, 0.0)
	for step in range(STEPS):
		DayCycleConfig.sample_into(float(step) / STEPS, sample)
		var m := sample.celestial_basis
		var sun := Vector3(m.x.dot(sample.sun_direction), m.y.dot(sample.sun_direction), m.z.dot(sample.sun_direction))
		_expect(sun.is_equal_approx(sun_fixed), "the sun must hold still among the stars at step %d, got %s" % [step, sun])
		_expect(m.z.is_equal_approx(pole), "the celestial pole must stand due north at the latitude, step %d" % step)
		_expect(is_equal_approx(m.determinant(), 1.0), "the celestial basis must be a rotation, step %d" % step)

func _digest(sample: DayCycleConfig.Sample) -> String:
	return "%.6f|%.6f|%s|%s|%.6f|%s|%s|%s|%.6f|%s|%s|%.6f|%.6f" % [
		sample.sun_elevation_deg,
		sample.sun_azimuth_deg,
		str(sample.key_direction),
		str(sample.key_color),
		sample.key_energy,
		str(sample.sky_zenith),
		str(sample.sky_horizon),
		str(sample.fog_color),
		sample.ambient_energy,
		str(sample.ambient_color),
		str(sample.ambient_light_scale),
		sample.sun_disk_intensity,
		sample.moon_disk_intensity,
	]

func _expect(condition: bool, message: String) -> void:
	if condition:
		return
	_failures += 1
	push_error(message)
