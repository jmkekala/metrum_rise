shader_type spatial;

uniform sampler2D heightmap;
uniform sampler2D overlay_texture;
uniform sampler2D parcel_texture;
uniform int overlay_mode = 0;
uniform float height_scale = 10.0;
uniform vec2 mesh_size = vec2(256.0, 256.0);
uniform vec2 heightmap_texture_size = vec2(256.0, 256.0);
uniform float terrain_cell_m = 10.0;
uniform float hillshade_azimuth_deg = 315.0;
uniform float hillshade_altitude_deg = 42.0;
uniform float hillshade_strength = 0.42;
uniform float hillshade_ambient = 0.36;
uniform float hillshade_contrast = 1.15;
uniform vec3 hillshade_shadow_tint : source_color = vec3(0.80, 0.86, 0.92);
uniform vec3 hillshade_light_tint : source_color = vec3(1.03, 1.02, 0.99);
uniform float contour_minor_interval_m = 10.0;
uniform float contour_major_interval_m = 50.0;
uniform float contour_minor_thickness = 1.0;
uniform float contour_major_thickness = 1.4;
uniform vec3 contour_minor_color : source_color = vec3(0.17, 0.24, 0.20);
uniform vec3 contour_major_color : source_color = vec3(0.10, 0.16, 0.14);
uniform float contour_minor_strength = 0.18;
uniform float contour_major_strength = 0.40;

varying float h;
varying vec2 var_centered_uv;

vec3 terrain_color_from_height(float world_elevation) {
	vec3 low_wet = vec3(0.16, 0.24, 0.14);
	vec3 shore = vec3(0.73, 0.69, 0.53);
	vec3 grass_low = vec3(0.34, 0.51, 0.23);
	vec3 grass_high = vec3(0.28, 0.42, 0.22);
	vec3 upland = vec3(0.49, 0.45, 0.30);
	vec3 rock = vec3(0.56, 0.56, 0.54);
	vec3 snow = vec3(0.93, 0.94, 0.96);

	vec3 color = low_wet;
	color = mix(color, grass_low, smoothstep(-30.0, 35.0, world_elevation));
	color = mix(color, shore, 1.0 - smoothstep(8.0, 42.0, abs(world_elevation)));
	color = mix(color, grass_high, smoothstep(80.0, 260.0, world_elevation));
	color = mix(color, upland, smoothstep(300.0, 700.0, world_elevation));
	color = mix(color, rock, smoothstep(800.0, 1400.0, world_elevation));
	color = mix(color, snow, smoothstep(1550.0, 2200.0, world_elevation));

	return color;
}

float sample_world_height(vec2 uv) {
	vec2 clamped_uv = clamp(uv, vec2(0.0), vec2(1.0));
	return texture(heightmap, clamped_uv).r * height_scale;
}

float hillshade_mask(vec2 uv) {
	vec2 texel = 1.0 / max(heightmap_texture_size, vec2(1.0));
	float h_left = sample_world_height(uv - vec2(texel.x, 0.0));
	float h_right = sample_world_height(uv + vec2(texel.x, 0.0));
	float h_up = sample_world_height(uv - vec2(0.0, texel.y));
	float h_down = sample_world_height(uv + vec2(0.0, texel.y));
	float safe_cell = max(terrain_cell_m, 0.001);
	float dzdx = (h_right - h_left) / (2.0 * safe_cell);
	float dzdz = (h_down - h_up) / (2.0 * safe_cell);
	vec3 normal = normalize(vec3(-dzdx, 1.0, -dzdz));

	float azimuth = radians(hillshade_azimuth_deg);
	float altitude = radians(hillshade_altitude_deg);
	vec3 light_dir = normalize(vec3(
		sin(azimuth) * cos(altitude),
		sin(altitude),
		-cos(azimuth) * cos(altitude)
	));

	float diffuse = clamp(dot(normal, light_dir), 0.0, 1.0);
	float shaped = pow(diffuse, hillshade_contrast);
	return clamp(mix(hillshade_ambient, 1.0, shaped), 0.0, 1.0);
}

float contour_mask(float world_elevation, float interval_m, float thickness) {
	float safe_interval = max(interval_m, 0.001);
	float scaled = world_elevation / safe_interval;
	float dist_to_line = abs(fract(scaled + 0.5) - 0.5);
	float aa_width = max(fwidth(scaled) * thickness, 0.0001);
	return 1.0 - smoothstep(0.0, aa_width, dist_to_line);
}

void vertex() {
// ...

	// TEXEL-CENTERED SAMPLING: Sample from the center of each float32 pixel
	// UV 0.0 should map to the center of the first pixel (0.5/256)
	vec2 centered_uv = (UV * (mesh_size - 1.0) + 0.5) / mesh_size;
	var_centered_uv = centered_uv;
	h = texture(heightmap, centered_uv).r;
	VERTEX.y += h * height_scale;
}

void fragment() {
	float world_elevation = h * height_scale;
	vec3 color = terrain_color_from_height(world_elevation);
	float hillshade = hillshade_mask(var_centered_uv);
	vec3 hillshade_tint = mix(hillshade_shadow_tint, hillshade_light_tint, hillshade);
	color *= mix(vec3(1.0), hillshade_tint, hillshade_strength);
	
	// Overlay Heatmaps (Pollution, Noise, Desirability)
	if (overlay_mode > 0) {
		vec4 heatmap_col = texture(overlay_texture, var_centered_uv);
		if (heatmap_col.a > 0.0) {
			color = mix(color, heatmap_col.rgb, 0.6);
		}
	}

	float major_contour = contour_mask(
		world_elevation,
		contour_major_interval_m,
		contour_major_thickness
	);
	float minor_contour = contour_mask(
		world_elevation,
		contour_minor_interval_m,
		contour_minor_thickness
	) * (1.0 - major_contour);
	color = mix(color, contour_minor_color, minor_contour * contour_minor_strength);
	color = mix(color, contour_major_color, major_contour * contour_major_strength);
	
	ALBEDO = color;
}
