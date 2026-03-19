shader_type spatial;

uniform sampler2D heightmap;
uniform sampler2D zoning_texture;
uniform sampler2D parcel_texture;
uniform int overlay_mode = 0;
uniform float height_scale = 10.0;
uniform vec2 mesh_size = vec2(256.0, 256.0);

varying float h;
varying vec2 var_centered_uv;

uniform bool show_global_zoning = false;

// ... other uniforms remain unchanged ...

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
	// Simple color based on height
	vec3 color = mix(vec3(0.1, 0.4, 0.1), vec3(0.5, 0.5, 0.5), h);
	if (h > 0.8) color = vec3(1.0, 1.0, 1.0); // Snow
	
	// Overlay Zoning & Parcel Grid
	if (overlay_mode == 0) {
        // Modernized Vector Geometry system parses procedural Zoning meshes globally native to the Godot `SurfaceTool` API bypassing raster matrices!
	} else {
		// Rendering for Heatmaps (Pollution, Noise, Desirability)
		vec4 heatmap_col = texture(zoning_texture, var_centered_uv);
		if (heatmap_col.a > 0.0) {
			color = mix(color, heatmap_col.rgb, 0.6);
		}
	}
	
	ALBEDO = color;
}
