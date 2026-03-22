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
	// UV.y is 0 to 1 across the width. Center line is around 0.5.
	float stripe = 0.0;
	if (UV.y > 0.48 && UV.y < 0.52) {
		float dash = step(0.5, fract(UV.x * 0.2));
		stripe = dash;
	}
	
	vec3 final_albedo = mix(albedo.rgb, vec3(1.0, 0.9, 0.0), stripe * 0.8) * COLOR.rgb;
	
	ALBEDO = final_albedo;
	if (!FRONT_FACING) {
		ALBEDO = mix(final_albedo, vec3(1.0, 0.0, 0.0), 0.5); // Tint red if backface
	}
	
	// Force normal to be UP in world space (transformed to view space)
	NORMAL = (VIEW_MATRIX * vec4(0.0, 1.0, 0.0, 0.0)).xyz;
	
	NORMAL_MAP = normal;
	ROUGHNESS = roughness;
	METALLIC = 0.0;
}
