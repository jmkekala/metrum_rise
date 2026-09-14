shader_type sky;
render_mode use_half_res_pass;

uniform sampler2D cloud_panorama : source_color, filter_linear_mipmap, repeat_enable;
uniform vec4 sky_zenith_color : source_color;
uniform vec4 sky_horizon_color : source_color;
uniform vec4 sky_nadir_color : source_color;
uniform float hemisphere_curve = 0.2;
uniform float panorama_rotation = 0.0;
uniform float cloud_strength = 0.48;
uniform float cloud_horizon_fade_start = 0.04;
uniform float cloud_horizon_fade_end = 0.16;
uniform vec4 cloud_shadow_color : source_color = vec4(0.52, 0.58, 0.64, 1.0);
uniform vec4 cloud_light_color : source_color = vec4(0.88, 0.91, 0.94, 1.0);
uniform float sun_curve = 0.12;
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

vec2 panorama_uv(vec3 direction) {
	float longitude = atan(direction.x, direction.z) / TAU + 0.5;
	float latitude = acos(clamp(direction.y, -1.0, 1.0)) / PI;
	return vec2(fract(longitude + panorama_rotation), latitude);
}

vec4 extract_clouds(vec3 direction) {
	vec3 source = texture(cloud_panorama, panorama_uv(direction)).rgb;
	float source_high = max(max(source.r, source.g), source.b);
	float source_low = min(min(source.r, source.g), source.b);
	float neutrality = source_low / max(source_high, 0.001);
	float luminance = dot(source, vec3(0.2126, 0.7152, 0.0722));
	float cloud_shape = smoothstep(0.55, 0.91, neutrality)
		* smoothstep(0.14, 0.72, luminance);
	float upper_hemisphere = smoothstep(
		cloud_horizon_fade_start,
		cloud_horizon_fade_end,
		direction.y
	);
	float cloud_alpha = clamp(cloud_shape * upper_hemisphere * cloud_strength, 0.0, 0.72);
	float cloud_light = smoothstep(0.22, 0.78, luminance);
	vec3 cloud_color = mix(cloud_shadow_color.rgb, cloud_light_color.rgb, cloud_light);
	return vec4(cloud_color, cloud_alpha);
}

vec3 add_disk(
	vec3 color,
	vec3 eye,
	vec3 body_direction,
	vec3 tint,
	float intensity,
	float diameter_deg,
	float halo_deg,
	float halo_strength
) {
	if (intensity <= 0.0) {
		return color;
	}
	float angle = acos(clamp(dot(normalize(body_direction), eye), -1.0, 1.0));
	float radius = max(radians(diameter_deg) * 0.5, 0.0001);
	float disk = 1.0 - smoothstep(radius * 0.82, radius * 1.18, angle);
	float halo_range = radians(max(halo_deg, 0.01));
	float halo_base = clamp(1.0 - angle / halo_range, 0.0, 1.0);
	float halo = pow(halo_base, 1.0 / max(sun_curve, 0.02));
	// Masked by the sample direction, not by the body's own elevation, so a body sitting on
	// the horizon keeps the half of its disk that is still above the line.
	float horizon_mask = smoothstep(sun_horizon_fade_start, sun_horizon_fade_end, eye.y);
	return color + tint * intensity * (disk * 1.25 + halo * halo_strength) * horizon_mask;
}

void sky() {
	if (AT_HALF_RES_PASS) {
		vec4 clouds = extract_clouds(EYEDIR);
		COLOR = clouds.rgb;
		ALPHA = clouds.a;
	} else {
		vec3 base_color = hemisphere_gradient(EYEDIR);
		base_color = add_disk(
			base_color,
			EYEDIR,
			sun_disk_direction,
			sun_disk_color.rgb,
			sun_disk_intensity,
			sun_disk_deg,
			sun_halo_deg,
			sun_halo_strength
		);
		base_color = add_disk(
			base_color,
			EYEDIR,
			moon_disk_direction,
			moon_disk_color.rgb,
			moon_disk_intensity,
			moon_disk_deg,
			moon_halo_deg,
			moon_halo_strength
		);
		COLOR = mix(base_color, HALF_RES_COLOR.rgb, HALF_RES_COLOR.a);
	}
}
