shader_type spatial;
render_mode cull_disabled, depth_draw_always;

uniform sampler2D albedo_tex : source_color, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D normal_tex : hint_normal, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D roughness_tex : hint_roughness_r, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D displacement_tex : hint_default_black, filter_linear_mipmap_anisotropic, repeat_enable;

uniform float uv_scale = 0.2; // Scaling for the 4k texture

varying vec3 world_pos;

void vertex() {
	world_pos = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
}

void fragment() {
	// 1. World-Space UVs for seamless asphalt tiling
	vec2 asphalt_uv = world_pos.xz * uv_scale;
	
	vec4 albedo = texture(albedo_tex, asphalt_uv);
	vec3 normal = texture(normal_tex, asphalt_uv).rgb;
	float roughness = texture(roughness_tex, asphalt_uv).r;
	// 2. Road-Specific Detail (Stripes)
	float stripe = 0.0;
	
	// Only draw stripes on road ribbons (Alpha = 0.0)
	if (COLOR.a < 0.1) {
		if (UV.y > 0.96 && COLOR.g > 0.5) {
			float dash = step(0.5, fract(UV.x * 0.2));
			stripe = dash;
		}
	}
	
	vec3 stripe_color = mix(vec3(1.0, 1.0, 1.0), vec3(1.0, 0.9, 0.0), COLOR.b);
	vec3 final_albedo = mix(albedo.rgb, stripe_color, stripe * 0.8);
	
	ALBEDO = final_albedo;
	
	NORMAL_MAP = normal;
	ROUGHNESS = roughness;
	METALLIC = 0.0;
}
