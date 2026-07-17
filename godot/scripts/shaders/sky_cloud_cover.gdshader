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
uniform float sun_angle_max = 7.5;
uniform float sun_curve = 0.12;

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

vec3 add_directional_sun(vec3 color, vec3 direction) {
	if (!LIGHT0_ENABLED) {
		return color;
	}
	vec3 sun_direction = normalize(-LIGHT0_DIRECTION);
	float sun_angle = acos(clamp(dot(sun_direction, direction), -1.0, 1.0));
	float sun_radius = max(LIGHT0_SIZE * 0.5, 0.0001);
	float sun_disk = 1.0 - smoothstep(sun_radius * 0.82, sun_radius * 1.18, sun_angle);
	float halo_range = radians(max(sun_angle_max, 0.01));
	float halo_base = clamp(1.0 - sun_angle / halo_range, 0.0, 1.0);
	float halo = pow(halo_base, 1.0 / max(sun_curve, 0.02));
	vec3 sun_color = LIGHT0_COLOR * LIGHT0_ENERGY;
	return color + sun_color * (sun_disk * 1.25 + halo * 0.22);
}

void sky() {
	if (AT_HALF_RES_PASS) {
		vec4 clouds = extract_clouds(EYEDIR);
		COLOR = clouds.rgb;
		ALPHA = clouds.a;
	} else {
		vec3 base_color = add_directional_sun(hemisphere_gradient(EYEDIR), EYEDIR);
		COLOR = mix(base_color, HALF_RES_COLOR.rgb, HALF_RES_COLOR.a);
	}
}
