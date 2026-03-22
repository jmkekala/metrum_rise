shader_type spatial;
render_mode cull_disabled, depth_draw_always;

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
	
	// Only draw stripes on road ribbons (Alpha = 0.0)
	if (COLOR.a < 0.1) {
		if (UV.y > 0.96 && COLOR.g > 0.5) {
			float dash = step(0.5, fract(UV.x * 0.2));
			stripe = dash;
		}
	}
	
	vec3 stripe_color = vec3(1.0, 1.0, 1.0);
	vec3 final_albedo = mix(albedo, stripe_color, stripe * 0.8);
	
	ALBEDO = final_albedo;
	
	NORMAL_MAP = normal;
	ROUGHNESS = roughness;
	METALLIC = 0.0;
}
