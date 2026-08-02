shader_type spatial;
render_mode unshaded, cull_disabled, blend_mix, depth_draw_never;

uniform sampler2D albedo_tex : source_color, filter_linear_mipmap_anisotropic, repeat_enable;
uniform float uv_scale = 0.0833333333;
uniform float mix_strength = 0.38;
uniform float macro_strength = 0.12;
uniform float brightness = 1.10;

varying vec3 world_pos;

float hash21(vec2 p) {
	return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453123);
}

float value_noise(vec2 p) {
	vec2 i = floor(p);
	vec2 f = fract(p);
	f = f * f * (3.0 - 2.0 * f);
	float a = hash21(i);
	float b = hash21(i + vec2(1.0, 0.0));
	float c = hash21(i + vec2(0.0, 1.0));
	float d = hash21(i + vec2(1.0, 1.0));
	return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

vec2 rotate_uv(vec2 uv, float angle) {
	float s = sin(angle);
	float c = cos(angle);
	return vec2(c * uv.x - s * uv.y, s * uv.x + c * uv.y);
}

void vertex() {
	world_pos = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
}

void fragment() {
	vec2 world_uv = world_pos.xz * uv_scale;
	vec3 grain_a = texture(albedo_tex, world_uv).rgb;
	vec3 grain_b = texture(albedo_tex, rotate_uv(world_uv + vec2(19.17, 7.31), 1.5707963268)).rgb;
	vec3 grain_c = texture(albedo_tex, rotate_uv(world_uv * 0.87 + vec2(-11.43, 23.61), -0.7853981634)).rgb;

	float mid_mask = smoothstep(0.25, 0.75, value_noise(world_pos.xz * 0.050));
	float wide_mask = smoothstep(0.18, 0.82, value_noise(world_pos.xz * 0.016 + vec2(7.0, 2.0)));
	vec3 grain = mix(grain_a, grain_b, mid_mask * mix_strength);
	grain = mix(grain, grain_c, wide_mask * mix_strength * 0.55);

	float macro = value_noise(world_pos.xz * 0.010 + vec2(2.1, 6.4));
	grain *= mix(1.0 - macro_strength, 1.0 + macro_strength, macro);

	ALBEDO = grain * COLOR.rgb * brightness;
	ALPHA = COLOR.a;
}
