// SPDX-License-Identifier: GPL-2.0-only
// Finnish paths (`ROAD-56`): light-traffic asphalt, stone dust (kivituhka) and forest trail.
// From Codex look-dev; outputs follow fr_surface in road_surface_finnish.gdshaderinc.
// detail_map is the existing linear RGBA aggregate_packed.png, repeat + mipmaps.
// world_xz anchors grain; road_m=(along metres, signed transverse metres); centreline y=0.
// half_width_m > 0. age/wetness clamped to 0..1; footprint_m is metres per pixel.
// Optical relief only: geometry owns thickness, crown and drainage. No tyre ruts.
// Call in uniform fragment control flow (screen derivatives rotate world grain slopes).
// Fixed work per fragment, two texture reads; no loops, allocations or scene-size dependence.

float fp_hash(vec2 v) {
	vec3 p = fract(vec3(v.x, v.y, v.x) * 0.1031);
	p += dot(p, p.yzx + 33.33);
	return fract((p.x + p.y) * p.z);
}
float fp_noise(vec2 p) {
	vec2 i = floor(p);
	vec2 f = fract(p);
	vec2 u = f * f * (3.0 - 2.0 * f);
	return mix(mix(fp_hash(i), fp_hash(i + vec2(1.0, 0.0)), u.x),
		mix(fp_hash(i + vec2(0.0, 1.0)), fp_hash(i + vec2(1.0)), u.x), u.y);
}
float fp_band(float d, float radius_m, float footprint_m) {
	// Box integration preserves thin-feature energy instead of widening distant cracks.
	float px = max(footprint_m, 0.0002);
	return clamp((min(d + px * 0.5, radius_m) - max(d - px * 0.5, -radius_m)) / px, 0.0, 1.0);
}
void fp_surface(sampler2D detail_map, vec2 world_xz, vec2 road_m,
		float half_width_m, float age, float wetness, float footprint_m, float kind,
		out vec3 albedo, out float roughness, out float height_m,
		out vec2 micro_slope, out float water) {
	float a = clamp(age, 0.0, 1.0);
	float w = clamp(wetness, 0.0, 1.0);
	float px = max(footprint_m, 0.0002);
	float s = road_m.x;
	float l = road_m.y;
	float hw = max(half_width_m, 0.10);
	float macro = fp_noise(world_xz * 0.63 + vec2(19.0, 4.0));
	float broad = fp_noise(world_xz * 0.19 + vec2(2.0, 31.0));
	float grain_scale = kind < 0.5 ? 0.24 : 0.40;
	vec4 grain = texture(detail_map, world_xz / grain_scale);
	vec4 grit = texture(detail_map, (world_xz + vec2(7.3, 3.1)) / 1.6);
	float verge_dist = abs(l) - hw + (macro - 0.5) * 0.22;
	float margin = smoothstep(-0.40, 0.04, verge_dist);
	float crumble = smoothstep(-0.16, 0.06, verge_dist + (grit.r - 0.34) * 0.16);
	float grass = smoothstep(-0.035 - px, 0.035 + px, verge_dist + (macro - 0.5) * 0.19 + (grit.a - 0.3) * 0.14);
	float centre = 1.0 - smoothstep(0.18 * hw, 0.73 * hw, abs(l + (broad - 0.5) * 0.24));
	float cell = floor(s / 7.0);
	float h0 = fp_hash(vec2(cell, 17.0));
	float h1 = fp_hash(vec2(cell, 51.0));
	float h2 = fp_hash(vec2(cell, 92.0));
	float local_s = mod(s, 7.0);
	float jitter = (fp_noise(vec2(l * 3.8, cell)) - 0.5) * 0.20;
	float fissure_d = local_s - (1.1 + h0 * 4.7) - l * (h1 - 0.5) * 0.35 - jitter;
	float fissure = fp_band(fissure_d, 0.003 + a * 0.006, px) * a * step(0.30, h2);
	float root_d = local_s - (1.3 + h1 * 4.3) - 0.30 * sin(l * 2.7 + h0 * 5.0);
	float root_envelope = smoothstep(0.10, 0.70, abs(l) / hw) * step(0.56, h0);
	float heave = exp(-root_d * root_d / 0.022) * root_envelope * a;
	vec2 dip_q = vec2((local_s - (1.6 + h2 * 3.8)) / (0.30 + h0 * 0.70),
		(l - (h1 - 0.5) * hw) / (0.20 + h1 * 0.35));
	float dip = (1.0 - smoothstep(0.12, 1.0, dot(dip_q, dip_q) + (macro - 0.5) * 0.25)) * step(0.28, h1);
	float puddle = smoothstep(0.48, 0.87, w) * smoothstep(0.20, 0.80, dip) * (1.0 - margin);
	float fine_fade = 1.0 - smoothstep(0.008, 0.035, px);
	if (kind < 0.5) {
		vec2 repair_q = vec2(local_s - (2.0 + h0 * 2.9), l - (h2 - 0.5) * hw);
		float repair_d = max(abs(repair_q.x) - (0.35 + h1 * 0.48), abs(repair_q.y) - (0.24 + h0 * 0.22));
		float repair = (1.0 - smoothstep(-px, px, repair_d)) * step(0.68, h1) * smoothstep(0.25, 0.85, a);
		float base = 0.075 + a * 0.031 + (grain.r - 0.34) * 0.10 + (macro - 0.5) * 0.016;
		albedo = vec3(base * 1.02, base * 1.015, base);
		albedo = mix(albedo, albedo * 0.60, repair);
		albedo = mix(albedo, vec3(0.024), fissure * (1.0 - repair));
		height_m = 0.010 * heave - 0.0015 * fissure - 0.004 * dip + 0.001 * repair;
		roughness = 0.79 - repair * 0.07;
		micro_slope = (grain.gb - 0.5) * 0.17;
		water = puddle * 0.55;
	} else if (kind < 1.5) {
		float loose = 0.25 + margin * 0.75;
		float base = 0.29 + (broad - 0.5) * 0.04 - centre * 0.024;
		base += (grain.r - 0.34) * (0.21 + 0.22 * loose) + (grit.r - 0.34) * margin * 0.12;
		albedo = base * vec3(1.05, 1.00, 0.88);
		height_m = (grain.a - 0.30) * (0.001 + margin * 0.004) - 0.013 * dip + 0.003 * heave;
		roughness = 0.96 - centre * 0.10;
		micro_slope = (grain.gb - 0.5) * (0.24 + 0.42 * margin);
		water = puddle;
	} else {
		float base = 0.10 + centre * 0.035 + (broad - 0.5) * 0.034 + (grain.r - 0.34) * 0.12;
		albedo = base * vec3(1.18, 0.88, 0.59);
		// Sparse original procedural leaf/needle silhouettes; no photographic atlas.
		vec2 litter_cell = floor(world_xz / 0.19);
		float lh = fp_hash(litter_cell + vec2(8.0));
		vec2 litter_q = fract(world_xz / 0.19) - vec2(0.22 + lh * 0.56, 0.22 + fp_hash(litter_cell) * 0.56);
		float ang = lh * 6.2831853;
		vec2 leaf_q = vec2(litter_q.x * cos(ang) - litter_q.y * sin(ang), litter_q.x * sin(ang) + litter_q.y * cos(ang));
		float leaf = (1.0 - smoothstep(0.65, 1.0, length(leaf_q / vec2(0.20, 0.075)))) * step(0.63, lh);
		float needle = fp_band(leaf_q.y * 0.19, 0.0008, px) * (1.0 - smoothstep(0.13, 0.19, abs(leaf_q.x))) * step(lh, 0.46);
		float litter = max(leaf * fine_fade, needle) * smoothstep(0.20, 0.80, abs(l) / hw);
		albedo = mix(albedo, vec3(0.24, 0.12, 0.035) * (0.65 + lh * 0.6), litter);
		float root_extent = (0.35 + 0.65 * smoothstep(0.0, 0.55, abs(l) / hw)) * step(0.56, h0);
		float branch_d = root_d - 0.12 - 0.19 * l;
		float root = max(fp_band(root_d, 0.024, px), fp_band(branch_d, 0.011, px) * 0.75) * root_extent;
		float root_rise = (exp(-root_d * root_d / 0.0012) * 0.022
			+ exp(-branch_d * branch_d / 0.0004) * 0.009) * root_extent;
		albedo = mix(albedo, vec3(0.20, 0.145, 0.080), root);
		height_m = -0.018 * dip + root_rise + (grain.a - 0.30) * 0.002;
		roughness = 0.93 - centre * 0.08;
		micro_slope = (grain.gb - 0.5) * 0.30;
		water = puddle * 0.88;
		albedo *= 1.0 - w * centre * 0.17;
	}
	vec3 edge_colour = kind > 1.5 ? vec3(0.090, 0.075, 0.035) : vec3(0.26, 0.25, 0.21);
	albedo = mix(albedo, edge_colour * (0.88 + grit.r * 0.36), crumble);
	albedo = mix(albedo, vec3(0.054, 0.078, 0.025) * (0.8 + macro * 0.4 + (grit.r - 0.34) * 0.6), grass);
	roughness = mix(roughness, 0.98, max(crumble, grass));
	water *= 1.0 - max(crumble, grass);
	albedo *= 1.0 - w * 0.34 - water * 0.12;
	roughness = mix(roughness, 0.46, w * (1.0 - grass) * 0.70);
	roughness = mix(roughness, 0.09, water);
	// Convert the world-XZ texture slopes to the caller's road-space tangent frame.
	vec2 rdx = dFdx(road_m);
	vec2 rdy = dFdy(road_m);
	vec2 wdx = dFdx(world_xz);
	vec2 wdy = dFdy(world_xz);
	float det = rdx.x * rdy.y - rdy.x * rdx.y;
	if (abs(det) > 1e-10) {
		vec2 world_s = (wdx * rdy.y - wdy * rdx.y) / det;
		vec2 world_l = (wdy * rdx.x - wdx * rdy.x) / det;
		micro_slope = vec2(dot(micro_slope, world_s), dot(micro_slope, world_l));
	}
	height_m *= 1.0 - grass;
	micro_slope *= (1.0 - 0.97 * water) * (1.0 - grass);
}
void fp_asphalt_surface(sampler2D detail_map, vec2 world_xz, vec2 road_m, float half_width_m,
		float age, float wetness, float footprint_m, out vec3 albedo, out float roughness,
		out float height_m, out vec2 micro_slope, out float water) {
	fp_surface(detail_map, world_xz, road_m, half_width_m, age, wetness, footprint_m, 0.0,
		albedo, roughness, height_m, micro_slope, water);
}
void fp_stone_dust_surface(sampler2D detail_map, vec2 world_xz, vec2 road_m, float half_width_m,
		float age, float wetness, float footprint_m, out vec3 albedo, out float roughness,
		out float height_m, out vec2 micro_slope, out float water) {
	fp_surface(detail_map, world_xz, road_m, half_width_m, age, wetness, footprint_m, 1.0,
		albedo, roughness, height_m, micro_slope, water);
}
void fp_forest_surface(sampler2D detail_map, vec2 world_xz, vec2 road_m, float half_width_m,
		float age, float wetness, float footprint_m, out vec3 albedo, out float roughness,
		out float height_m, out vec2 micro_slope, out float water) {
	fp_surface(detail_map, world_xz, road_m, half_width_m, age, wetness, footprint_m, 2.0,
		albedo, roughness, height_m, micro_slope, water);
}
