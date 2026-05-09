shader_type spatial;
render_mode cull_back, depth_draw_opaque;

uniform sampler2D albedo_tex : source_color, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D normal_tex : hint_normal, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D roughness_tex : hint_roughness_r, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D displacement_tex : hint_default_black, filter_linear_mipmap_anisotropic, repeat_enable;

uniform float uv_scale = 0.05; // Primary detail scaling (20m per tile)
uniform float macro_uv_scale = 0.007; // Macro variation scaling (~140m per tile)
uniform float macro_influence = 0.4; // Influence of macro variation

varying vec3 world_pos;

void vertex() {
	world_pos = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
}

void fragment() {
	// 1. World-Space UVs for seamless asphalt tiling
	vec2 asphalt_uv = world_pos.xz * uv_scale;
	vec2 macro_uv = world_pos.xz * macro_uv_scale;
	
	vec4 albedo_micro = texture(albedo_tex, asphalt_uv);
	vec4 albedo_macro = texture(albedo_tex, macro_uv);
	
	// Blend macro variation into albedo for large-scale interest
	vec3 albedo = albedo_micro.rgb * mix(vec3(1.0), albedo_macro.rgb * 1.2, macro_influence);
	
	vec3 normal = texture(normal_tex, asphalt_uv).rgb;
	float roughness_micro = texture(roughness_tex, asphalt_uv).r;
	float roughness_macro = texture(roughness_tex, macro_uv).r;
	float roughness = mix(roughness_micro, roughness_macro, macro_influence * 0.5);
	// 2. Road-Specific Detail (Stripes)
	float stripe = 0.0;
	float dist = length(VERTEX);
	
	// Only draw stripes on road ribbons (Alpha = 0.0)
	if (COLOR.a < 0.1) {
		// Antialiased Stripe Mask (Vertical across the lane)
		float fw_y = fwidth(UV.y);
		float thickness = 0.025; // Even thinner lines
		float stripe_v = smoothstep(1.0 - thickness - fw_y, 1.0 - thickness, UV.y);
		
		if (COLOR.g > 0.5) {
			// Horizontal Pattern (Along the road)
			float fw_x = fwidth(UV.x * 0.2);
			float dash_val = fract(UV.x * 0.2);
			
			// Dash pattern with antialiasing
			float dash_pattern = smoothstep(0.5 - fw_x, 0.5 + fw_x, dash_val);
			
			// Center lines (COLOR.b) are solid, others are dashed
			float is_center = COLOR.b;
			float base_pattern = mix(dash_pattern, 1.0, is_center);
			
			stripe = stripe_v * base_pattern;
		}
	}
	
	// 3. Sidewalk Detail (Tiles & Lightness)
	if (COLOR.a > 0.9) {
		// Concrete Tile Grout (Using World UVs for seamlessness across junctions)
		vec2 world_uv = world_pos.xz;
		vec2 grid = abs(fract(world_uv * 2.0) - 0.5); // 0.5m tiles
		float grout = smoothstep(0.45, 0.5, grid.x) + smoothstep(0.45, 0.5, grid.y);
		
		// Lighten to concrete color and subtract grout lines
		albedo = mix(albedo, vec3(0.85, 0.84, 0.82), 0.5);
		albedo *= mix(1.0, 0.6, grout);
		
		// Curb/sidewalk boundaries are real geometry now. Do not derive a curb gap from mesh UVs:
		// node-piece CDT triangles use arbitrary UVs, which would paint dark bars through junctions.
	}
	
	// Darker grey-white for better integration
	vec3 stripe_color = vec3(0.7, 0.7, 0.7);
	// Drastically reduce opacity and add distance-based fading
	// Markings start at 30% strength and fade to 5% at 500m
	float marking_strength = mix(0.3, 0.05, smoothstep(50.0, 500.0, dist));
	vec3 final_albedo = mix(albedo, stripe_color, stripe * marking_strength);
	
	ALBEDO = final_albedo;
	
	NORMAL_MAP = normal;
	ROUGHNESS = roughness;
	METALLIC = 0.0;
}
