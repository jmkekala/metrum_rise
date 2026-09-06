shader_type spatial;
render_mode unshaded, cull_disabled, depth_draw_never;

uniform sampler2D asphalt_tex : source_color, filter_linear_mipmap_anisotropic, repeat_enable;
uniform sampler2D sidewalk_tex : source_color, filter_linear_mipmap_anisotropic, repeat_enable;
uniform float lane_width_m = 3.5;
uniform float sidewalk_width_m = 1.5;
uniform int forward_lanes = 1;
uniform int backward_lanes = 1;
// 0 = valid, 1 = checking, 2 = invalid. Only display state; Rust owns validity.
uniform int placement_state = 0;

varying vec2 world_xz;

void vertex() {
	world_xz = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xz;
}

void fragment() {
	float lanes = float(forward_lanes + backward_lanes);
	bool walkway = lanes < 0.5;
	float half_carriageway = max(lanes * lane_width_m, 2.0) * 0.5;
	float half_width = half_carriageway + (walkway ? 0.0 : sidewalk_width_m);
	vec3 asphalt = texture(asphalt_tex, world_xz * 0.05).rgb;
	vec3 sidewalk = texture(sidewalk_tex, world_xz * 0.12).rgb * 1.16;
	vec3 base = COLOR.r < 0.25 ? asphalt : (COLOR.r < 0.75 ? sidewalk : vec3(0.50, 0.49, 0.46));
	float stripe = 0.0;
	if (COLOR.r < 0.25 && lanes > 1.0) {
		// UV is longitudinal metres / signed lateral metres, so bends retain lane spacing.
		float lane_position = (UV.y + half_carriageway) / lane_width_m;
		float divider = floor(lane_position + 0.5);
		if (divider > 0.5 && divider < lanes - 0.5) {
			float distance_to_line = abs(lane_position - divider) * lane_width_m;
			float aa = max(fwidth(UV.y), 0.005);
			stripe = 1.0 - smoothstep(0.08, 0.08 + aa, distance_to_line);
			bool center = forward_lanes > 0 && backward_lanes > 0 && abs(divider - float(backward_lanes)) < 0.1;
			if (!center) {
				float phase = fract(UV.x / 6.0);
				float dash_aa = min(fwidth(UV.x / 6.0), 0.25);
				stripe *= smoothstep(0.0, dash_aa + 0.001, phase) * (1.0 - smoothstep(0.5 - dash_aa, 0.5 + dash_aa, phase));
			}
		}
	}
	base = mix(base, vec3(0.82), stripe * 0.85);
	ALBEDO = base;
	if (placement_state != 0) {
		vec3 status = placement_state == 2 ? vec3(1.0, 0.16, 0.10) : vec3(1.0, 0.68, 0.12);
		float tint_strength = placement_state == 2 ? 0.48 : 0.22;
		float outline = 1.0 - smoothstep(0.10, 0.10 + max(fwidth(UV.y), 0.01), half_width - abs(UV.y));
		ALBEDO = mix(mix(base, status, tint_strength), status, outline * 0.85);
	}
	ALPHA = 0.94;
}
