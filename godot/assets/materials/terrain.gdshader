shader_type spatial;

uniform sampler2D heightmap;
uniform sampler2D overlay_texture;
uniform sampler2D parcel_texture;
uniform int overlay_mode = 0;
uniform float height_scale = 10.0;
uniform vec2 mesh_size = vec2(256.0, 256.0);

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
	
	// Overlay Heatmaps (Pollution, Noise, Desirability)
	if (overlay_mode > 0) {
		vec4 heatmap_col = texture(overlay_texture, var_centered_uv);
		if (heatmap_col.a > 0.0) {
			color = mix(color, heatmap_col.rgb, 0.6);
		}
	}
	
	ALBEDO = color;
}
