shader_type spatial;
render_mode cull_disabled, depth_draw_always;

uniform vec3 asphalt_color : source_color = vec3(0.0, 0.0, 0.0);
uniform vec3 line_color : source_color = vec3(0.9, 0.9, 0.9);
uniform float line_width = 0.05;
uniform float dash_length = 2.0;
uniform float dash_gap = 2.0;

varying vec3 world_pos;

void vertex() {
	world_pos = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
}

void fragment() {
	vec2 uv = UV;
	vec3 color = asphalt_color;
	float roughness = 0.8;
	
	// Extract explicit lane counts injected from Rust Geometry builder!
	float fwd_lanes = round(COLOR.r * 10.0);
	float bkw_lanes = round(COLOR.g * 10.0);
	float total_lanes = max(1.0, fwd_lanes + bkw_lanes);
	
	// Alpha fade for intersections
	float intersection_fade = COLOR.a;
	
	// UNIFORM DASH LOGIC (World-Space)
	float world_dist = length(world_pos.xz);
	float cycle_len = dash_length + dash_gap;
	float dash_pos = mod(world_dist, cycle_len);
	bool is_dash = dash_pos < dash_length;
	
	float local_x = uv.x;
	
	// 1. Solid Outer Edge Lines (0.05 lane thickness)
	if (local_x < 0.08 || local_x > total_lanes - 0.08) {
		color = mix(color, line_color, 0.85 * intersection_fade); // White edge marker
	} 
	// 2. Yellow Double Center Line (Splitting Forward vs Backward)
	else if (bkw_lanes > 0.0 && fwd_lanes > 0.0 && abs(local_x - bkw_lanes) < 0.15) {
		// Draw two thin yellow lines
		float yellow_dist = abs(local_x - bkw_lanes);
		if (yellow_dist > 0.03 && yellow_dist < 0.12) {
			color = mix(color, vec3(1.0, 0.75, 0.05), 0.9 * intersection_fade); // Distinct Yellow
		}
	}
	// 3. Dashed White Lane Dividers (Between identical directions)
	else {
		float lane_fract = fract(local_x);
		if (lane_fract < 0.04 || lane_fract > 0.96) {
			if (is_dash) {
				color = mix(color, line_color, 0.85 * intersection_fade); // Dashed white line
			}
		}
	}
	
	ALBEDO = color;
	ROUGHNESS = roughness;
	METALLIC = 0.15;
}
