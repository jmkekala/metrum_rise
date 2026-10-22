shader_type sky;

#include "res://scripts/shaders/weather_clouds.gdshaderinc"
#include "res://scripts/shaders/night_sky.gdshaderinc"

uniform vec4 sky_zenith_color : source_color;
uniform vec4 sky_horizon_color : source_color;
uniform vec4 sky_nadir_color : source_color;
uniform float hemisphere_curve = 0.2;
// The day cycle's authored cloud colours: the shaded body and the sunlit side.
uniform vec4 cloud_shadow_color : source_color = vec4(0.52, 0.58, 0.64, 1.0);
uniform vec4 cloud_light_color : source_color = vec4(0.88, 0.91, 0.94, 1.0);
uniform float sun_curve = 0.12;
// The weather's coverage as SceneLighting last applied it. A material parameter rather than the
// weather_clouds global, because only a material change refreshes the radiance cubemap.
uniform float cloud_cover = 0.45;
// The disks are placed explicitly rather than read from LIGHT0 so that disk brightness and
// key-light energy can disagree. A sun on the horizon has to stay visible long after it has
// stopped being a useful key light, and the moon is not a Godot light at all.
uniform vec3 sun_disk_direction = vec3(0.0, 1.0, 0.0);
uniform vec4 sun_disk_color : source_color = vec4(1.0, 0.97, 0.92, 1.0);
uniform float sun_disk_intensity = 1.45;
uniform float sun_disk_deg = 0.62;
uniform float sun_halo_deg = 7.5;
uniform float sun_halo_strength = 0.20;
uniform vec3 moon_disk_direction = vec3(0.0, -1.0, 0.0);
uniform vec4 moon_disk_color : source_color = vec4(0.92, 0.94, 1.0, 1.0);
uniform float moon_disk_intensity = 0.0;
uniform float moon_disk_deg = 0.62;
uniform float moon_halo_deg = 3.2;
uniform float moon_halo_strength = 0.06;
// Share of the night side the earth lights faintly: earthshine.
const float MOON_EARTHSHINE = 0.015;
// Fades a disk out as it passes below the dome's horizon. It starts below zero so a setting
// sun survives all the way down to the horizon line instead of vanishing several degrees up.
uniform float sun_horizon_fade_start = -0.060;
uniform float sun_horizon_fade_end = -0.010;

vec3 hemisphere_gradient(vec3 direction) {
	float elevation = pow(abs(direction.y), 1.0 / max(hemisphere_curve, 0.001));
	vec3 upper = mix(sky_horizon_color.rgb, sky_zenith_color.rgb, elevation);
	vec3 lower = mix(sky_horizon_color.rgb, sky_nadir_color.rgb, elevation);
	return mix(lower, upper, step(0.0, direction.y));
}

vec3 add_disk(
	vec3 color,
	vec3 eye,
	vec3 body_direction,
	vec3 tint,
	float intensity,
	float diameter_deg,
	float halo_deg,
	float halo_strength,
	vec3 lit_from
) {
	if (intensity <= 0.0) {
		return color;
	}
	vec3 body = normalize(body_direction);
	float angle = acos(clamp(dot(body, eye), -1.0, 1.0));
	float radius = max(radians(diameter_deg) * 0.5, 0.0001);
	float disk = 1.0 - smoothstep(radius * 0.82, radius * 1.18, angle);
	// A body lit from `lit_from` shows its phase: the near hemisphere's normal under this pixel,
	// shaded by the light with a soft terminator; the halo follows the lit fraction. A zero
	// `lit_from` is a self-lit body.
	if (dot(lit_from, lit_from) > 0.0 && disk > 0.0) {
		vec3 across = (eye - body * dot(eye, body)) / sin(radius);
		vec3 normal = across - body * sqrt(max(1.0 - dot(across, across), 0.0));
		float lit = smoothstep(-0.08, 0.08, dot(normal, lit_from));
		float illumination = 0.5 * (1.0 - dot(body, lit_from));
		disk *= mix(MOON_EARTHSHINE, 1.0, lit);
		halo_strength *= illumination * illumination;
	}
	float halo_range = radians(max(halo_deg, 0.01));
	float halo_base = clamp(1.0 - angle / halo_range, 0.0, 1.0);
	float halo = pow(halo_base, 1.0 / max(sun_curve, 0.02));
	// Masked by the sample direction, not by the body's own elevation, so a body sitting on
	// the horizon keeps the half of its disk that is still above the line.
	float horizon_mask = smoothstep(sun_horizon_fade_start, sun_horizon_fade_end, eye.y);
	return color + tint * intensity * (disk * 1.25 + halo * halo_strength) * horizon_mask;
}

// The visible clouds are laid over the sky by weather_cloud_effect.gd. Radiance sees the deck
// only as its mean cover. It reads no texture and no drift, so moving clouds never dirty the
// radiance map; it changes only with the gated palette and coverage (`cloud_cover`).
vec4 cloud_mean_cover(vec3 eye) {
	float above = smoothstep(0.02, 0.25, eye.y);
	float coverage = cloud_cover;
	vec3 colour = mix(cloud_shadow_color.rgb, cloud_light_color.rgb, 0.45);
	return vec4(colour, coverage * mix(0.35, 0.85, coverage) * above);
}

void sky() {
	vec3 base_color = hemisphere_gradient(EYEDIR);
	if (!AT_CUBEMAP_PASS) {
		base_color += night_sky(EYEDIR, length(fwidth(EYEDIR)));
	}
	base_color = add_disk(
		base_color,
		EYEDIR,
		sun_disk_direction,
		sun_disk_color.rgb,
		sun_disk_intensity,
		sun_disk_deg,
		sun_halo_deg,
		sun_halo_strength,
		vec3(0.0)
	);
	base_color = add_disk(
		base_color,
		EYEDIR,
		moon_disk_direction,
		moon_disk_color.rgb,
		moon_disk_intensity,
		moon_disk_deg,
		moon_halo_deg,
		moon_halo_strength,
		normalize(sun_disk_direction)
	);
	if (AT_CUBEMAP_PASS) {
		vec4 clouds = cloud_mean_cover(EYEDIR);
		base_color = mix(base_color, clouds.rgb, clouds.a);
	}
	COLOR = base_color;
}
