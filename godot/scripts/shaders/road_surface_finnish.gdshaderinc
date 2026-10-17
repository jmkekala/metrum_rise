// SPDX-License-Identifier: GPL-2.0-only
// Finnish road surface: stone-mastic asphalt with studded-tyre ruts on busy roads, sealed cracks,
// trench and cut patches, manholes and drain grates, and a crushed-rock gravel variant. From the look-dev in docs/roads.md (`ROAD-56`).
//
// road_m is (metres along the road, signed metres across it from the centreline) on road
// stretches. Junction asphalt has no such coordinates (road_space false): it keeps the base and
// patches and drops everything laid out along lanes. The detail texture is sampled in world XZ
// on both, so a stretch and its junction share one aggregate without a seam.
//
// detail_map: linear RGBA8, repeating: R aggregate, GB micro slopes, A relief.
// Two texture reads, no loops: O(1) per fragment.

float fr_hash(float x) {
	float p = fract(x * 0.1031);
	p *= p + 33.33;
	return fract(p * (p + p));
}

float fr_noise(float x) {
	float i = floor(x);
	float f = fract(x);
	return mix(fr_hash(i), fr_hash(i + 1.0), f * f * (3.0 - 2.0 * f));
}

// 2D value noise in 0..1 for blotches anchored in the world.
float fr_noise2(vec2 p) {
	vec2 i = floor(p);
	vec2 f = fract(p);
	vec2 u = f * f * (3.0 - 2.0 * f);
	float h00 = fr_hash(i.x * 57.0 + i.y * 113.0);
	float h10 = fr_hash(i.x * 57.0 + 57.0 + i.y * 113.0);
	float h01 = fr_hash(i.x * 57.0 + i.y * 113.0 + 113.0);
	float h11 = fr_hash(i.x * 57.0 + 57.0 + i.y * 113.0 + 113.0);
	return mix(mix(h00, h10, u.x), mix(h01, h11, u.x), u.y);
}

// GGX sun specular for the road materials' light(): their diffuse is paving_diffuse, and without
// this asphalt has no sheen toward a low sun.
vec3 fr_sun_specular(vec3 n, vec3 l, vec3 v, float roughness, vec3 light_color, float attenuation) {
	vec3 h = normalize(l + v);
	float nl = max(dot(n, l), 0.0);
	float nv = max(dot(n, v), 1.0e-4);
	float nh = max(dot(n, h), 0.0);
	float lh = max(dot(l, h), 0.0);
	float alpha = max(roughness * roughness, 0.002);
	float a2 = alpha * alpha;
	float denom = nh * nh * (a2 - 1.0) + 1.0;
	float k = alpha * 0.5;
	float visibility = 0.25 / ((nl * (1.0 - k) + k) * (nv * (1.0 - k) + k));
	float fresnel = 0.04 + 0.96 * pow(1.0 - lh, 5.0);
	return light_color * attenuation * nl * a2 / (PI * denom * denom) * visibility * fresnel;
}

float fr_line(float d, float half_width, float aa) {
	return 1.0 - smoothstep(half_width - aa, half_width + aa, abs(d));
}

// Manhole covers in the lanes and drain grates in the gutters of road stretches, one candidate
// of each per cell along the road.
void fr_street_fittings(bool road_space, float s, float l, float half_width, float lane_width,
		float aa, inout vec3 albedo, inout float roughness, inout float height_m) {
	if (!road_space) {
		return;
	}
	// Manholes: a 0.62 m cast-iron cover in a lighter frame, on the centreline or a lane centre.
	float mc = floor(s / 43.0);
	float m0 = fr_hash(mc * 7.7 + 3.0);
	float m1 = fr_hash(mc * 7.7 + 41.0);
	float ml = m1 < 0.35 ? 0.0 : (m1 < 0.68 ? 1.0 : -1.0) * min(lane_width * 0.5, half_width * 0.5);
	vec2 mq = vec2(s - (mc * 43.0 + 6.0 + m0 * 31.0), l - ml);
	float mr = length(mq);
	float present = step(m0, 0.55);
	float cover = (1.0 - smoothstep(0.31 - aa, 0.31 + aa, mr)) * present;
	float frame = (1.0 - smoothstep(0.37 - aa, 0.37 + aa, mr)) * present - cover;
	// Raised diamond tread on the cover, faded where tyres polish it.
	vec2 tread = abs(fract(vec2(mq.x + mq.y, mq.x - mq.y) * 7.0) - 0.5);
	float boss = step(0.32, max(tread.x, tread.y)) * (1.0 - smoothstep(0.012, 0.03, aa));
	vec3 iron = vec3(0.045, 0.042, 0.040) * (1.0 + 0.35 * boss);
	albedo = mix(albedo, iron, cover);
	albedo = mix(albedo, vec3(0.075, 0.072, 0.068), frame);
	roughness = mix(roughness, 0.55 - 0.15 * boss, cover);
	height_m += -0.008 * (cover + frame) + 0.002 * boss * cover;
	// Drain grates: 0.5 m x 0.4 m in the gutter, bars across the flow.
	float gc = floor(s / 37.0);
	float g0 = fr_hash(gc * 9.1 + 11.0);
	float g1 = fr_hash(gc * 9.1 + 83.0);
	float gl = (g1 > 0.5 ? 1.0 : -1.0) * (half_width - 0.24);
	vec2 gq = vec2(s - (gc * 37.0 + 3.0 + g0 * 31.0), l - gl);
	float grate_box = max(abs(gq.x) - 0.25, abs(gq.y) - 0.20);
	float grate = (1.0 - smoothstep(-aa, aa, grate_box)) * step(g0, 0.65) * step(2.5, half_width);
	float slot = (1.0 - smoothstep(-aa, aa, max(abs(gq.x) - 0.21, abs(gq.y) - 0.16)))
		* step(0.45, fract(gq.x / 0.05)) * (1.0 - smoothstep(0.008, 0.02, aa));
	albedo = mix(albedo, mix(vec3(0.05, 0.047, 0.044), vec3(0.008), slot), grate);
	roughness = mix(roughness, 0.6, grate);
	height_m += -0.015 * grate - 0.02 * slot * grate;
}

// Signed distance to a rectangle of half size `half_size` with corners rounded by `radius`.
float fr_rounded_box(vec2 q, vec2 half_size, float radius) {
	vec2 e = abs(q) - half_size + radius;
	return length(max(e, 0.0)) + min(max(e.x, e.y), 0.0) - radius;
}

// L3 zebra paint over the already sampled stones. Paint is applied after patches, seals and
// fittings, as a crossing is repainted over them; only cracks break through it. Look-dev targets
// (local Commons and street photos): Jyväskylä's exposed pinholes, Okkerinkatu's and
// Mustinlammenkatu's streaky tyre wear, Nummikatu's fresh paint, doubled bars where a repaint
// missed the old one. Optical values, not photometric measurements.
void fr_zebra_paint(vec4 crossing, vec2 world_xz, float lane_width, float aa, vec4 aggregate,
		vec4 coarse, float crack, inout vec3 albedo, inout float roughness, inout vec2 micro_slope) {
	float age = clamp(crossing.z - 1.0, 0.0, 1.0);
	float h = fract(age * 13.7 + 0.37);
	// Fade the 1/32 m chip cells toward their mean before they become subpixel (aa is half
	// a pixel footprint). Aggregate itself already has mip filtering.
	float chips = mix(0.5, fr_noise2(world_xz * 32.0), 1.0 - smoothstep(1.0 / 128.0, 1.0 / 64.0, aa));
	// Bars 0.5 x 2.5 m on a 1 m pitch, centred at 0.25 m: x is folded to the nearest bar, so the
	// fold falls mid-gap. Sprayed paint has corners rounded by about 4 cm and fades over about a
	// centimetre of overspray, kept inside the nominal bar so the crossing's mesh edge never cuts it.
	vec2 q = vec2(mod(crossing.x + 0.25, 1.0) - 0.5, crossing.y);
	float soft = 0.008 + aa;
	float erosion = (0.002 + 0.012 * age) * chips;
	float bar = 1.0 - smoothstep(-2.0 * soft, 0.0, fr_rounded_box(q, vec2(0.25, 1.25), 0.04) + erosion);
	// Some repaints miss the old bars by a few centimetres, either way and per bar, across and
	// along, leaving the old paint's edge beside the new one: faint, and worn further than the bar
	// over it. A constant offset read as an embossed drop shadow, not as old paint.
	float ghost = 0.0;
	if (h > 0.62) {
		float k = floor(crossing.x + 0.25);
		float shift = fr_hash(k * 3.17 + h * 41.0) - 0.5;
		float slide = fr_hash(k * 5.31 + h * 17.0) - 0.5;
		vec2 g = vec2(crossing.x - 0.25 - k - shift * 0.14, crossing.y - slide * 0.30);
		ghost = 1.0 - smoothstep(-2.0 * soft, 0.0, fr_rounded_box(g, vec2(0.25, 1.25), 0.05) + erosion);
	}
	// Studded tyres thin the paint in two bands per lane, 1.8 m track width, in streaks that run
	// with the traffic: a worn bar fades and speckles down to the stones, never to a clean cut.
	float lane_l = mod(crossing.w, lane_width) - lane_width * 0.5;
	float wheels = 1.0 - smoothstep(0.18, 0.46, abs(abs(lane_l) - 0.90));
	float streak = fr_noise2(vec2(crossing.x * 14.0, crossing.y * 0.9) + h * 31.0);
	float wear = age * (0.15 + 0.75 * wheels) * (0.50 + 1.00 * streak);
	float grain = aggregate.r * 0.65 + coarse.r * 0.35;
	float intact = mix(0.2, 1.0, smoothstep(wear - 0.18, wear + 0.18, grain + chips * 0.2));
	float old_wear = wear + 0.45;
	float old_paint = 0.30 * ghost * smoothstep(old_wear - 0.18, old_wear + 0.18, grain + chips * 0.2);
	float paint = max(bar * intact, old_paint) * (1.0 - crack);
	// The new film smooths the stones' micro relief. It adds no height: a 2 mm step over the
	// centimetre of soft edge became a slope in fr_relief_normal and shaded one side of every bar
	// like an embossed bevel.
	float film = bar * intact * (1.0 - crack);
	// Linear off-white, about 2 mm optical film over most of the substrate micro relief; the
	// stones under it give the sprayed speckle.
	vec3 white = vec3(0.68, 0.67, 0.63) * (1.0 - 0.20 * age) * (0.92 + 0.16 * grain);
	albedo = mix(albedo, white, paint);
	roughness = mix(roughness, 0.56 + 0.16 * age, paint);
	micro_slope *= 1.0 - 0.35 * film;
}

// age, wetness and grass in 0..1; lanes and lane_width describe the carriageway centred on
// road_m.y = 0. traffic in 0..1 scales the studded-tyre ruts: none on residential streets.
// height_m is optical relief for fr_relief_normal, not geometry.
void fr_surface_painted(sampler2D detail_map, vec2 world_xz, vec2 road_m, bool road_space,
		float lanes, float lane_width, float traffic, float age, float wetness, bool gravel, float grass,
		float footprint_m, vec4 crossing, out vec3 albedo, out float roughness, out float height_m,
		out vec2 micro_slope, out float water) {
	float s = road_m.x;
	float l = road_m.y;
	float a = clamp(age, 0.0, 1.0);
	float w = clamp(wetness, 0.0, 1.0);
	float along = road_space ? 1.0 : 0.0;
	float half_width = lanes * lane_width * 0.5;
	vec4 d = texture(detail_map, world_xz / (gravel ? 2.0 : 0.8));
	// Asphalt: the same aggregate at three times the size reads as ravelled patches of coarse
	// stone, which survive mipmapping and a low render scale where the fine grain blurs out.
	vec4 fine = texture(detail_map, (world_xz + vec2(5.3, 2.1)) / (gravel ? 0.31 : 2.6));
	float macro = fr_noise(s * 0.17 + l * 0.63 + 48.0) * 0.6 + fr_noise(s * 0.41 - l * 0.9 + 17.0) * 0.4;
	float aa = max(0.002, min(footprint_m * 0.5, 0.10));
	float wander = (fr_noise(s * 0.095 + 71.0) - 0.5) * 0.42 + (fr_noise(s * 0.37 + 27.0) - 0.5) * 0.10;
	float lc = gravel ? l - wander : mod(l + half_width, lane_width) - lane_width * 0.5;
	float tw = 0.23 + 0.10 * fr_noise(s * 0.24 + 2.0);
	// Wheel paths sit about 0.9 m either side of a lane centre. Only busy roads (traffic) show
	// them: studded tyres rut and polish them. A residential street wears evenly.
	float rut = along * (gravel ? 1.0 : traffic)
		* (1.0 - smoothstep(tw, tw + 0.24, abs(abs(lc) - (0.88 + 0.045 * fr_noise(s * 0.19)))));
	// Blotches from uneven binder, moisture and wear at about 1.8 and 6 m, anchored in the world;
	// the coarse grain from `fine` carries the smaller scale.
	float mottle = fr_noise2(world_xz * 0.55 + 7.0) * 0.55 + fr_noise2(world_xz * 0.16 + 19.0) * 0.45;
	float edge = along * smoothstep(half_width - 0.30, half_width + 0.16, abs(l) + (macro - 0.5) * 0.28);
	float cell = floor(s / 11.0);
	float local_s = mod(s, 11.0);
	float h0 = fr_hash(cell + 17.0);
	float h1 = fr_hash(cell + 53.0);
	float h2 = fr_hash(cell + 93.0);
	if (gravel) {
		float loose = clamp(0.40 + 0.44 * (1.0 - rut) + 0.16 * smoothstep(half_width - 1.0, half_width, abs(l)), 0.0, 1.0);
		float dust = 0.21 + 0.055 * macro + 0.055 * (fine.r - 0.35);
		float base = dust + (d.r - 0.34) * (0.34 * loose + 0.07) - rut * (0.025 + 0.025 * macro);
		float tint = fr_noise(s * 0.029 + 8.0) * 0.65 + macro * 0.35;
		albedo = base * vec3(1.04 + 0.13 * tint, 1.00 + 0.015 * tint, 0.91 - 0.075 * tint);
		float pit_l = (h2 > 0.5 ? 0.88 : -0.88) + wander;
		vec2 q = vec2((local_s - (2.0 + h0 * 6.0)) / (0.30 + h1 * 0.62), (l - pit_l) / (0.24 + h0 * 0.19));
		float pothole = along * (1.0 - smoothstep(0.28, 1.0, dot(q, q) + (fr_noise(s * 6.0 + l * 9.0) - 0.5) * 0.19)) * smoothstep(0.18, 0.30, h1);
		height_m = -0.033 * abs(l) * along - (0.017 + 0.018 * macro) * rut - 0.070 * pothole + 0.009 * (d.a - 0.3) * loose;
		height_m += 0.003 * sin(s * 9.8 + fr_noise(s * 0.3)) * smoothstep(0.55, 0.75, macro) * (1.0 - rut) * along;
		albedo *= 1.0 - 0.18 * pothole;
		float ridge = along * (1.0 - smoothstep(0.18, 0.45, abs(l - wander)));
		albedo = mix(albedo, vec3(0.075, 0.105, 0.027), ridge * grass * smoothstep(0.36, 0.58, macro));
		roughness = 0.95 - 0.15 * rut;
		vec2 pool = vec2((local_s - (3.0 + h2 * 5.0)) / (0.35 + h0 * 0.9), (l + pit_l) / (0.19 + h1 * 0.16));
		float pools = along * (1.0 - smoothstep(0.35, 1.0, dot(pool, pool) + (fr_noise(s * 5.0 + l * 8.0) - 0.5) * 0.14)) * 0.85;
		water = w * max(pothole, pools);
		micro_slope = (d.gb - 0.5) * 2.0 * (0.30 + loose * 0.60) + (fine.gb - 0.5) * 0.13;
	} else {
		// Utility trenches and resurfacing leave long machine-cut patches: bands across the
		// whole road or one lane, and strips along it. One candidate of each per cell along the
		// road, as signed box distances in metres (negative inside).
		float band_cell = floor(s / 31.0);
		float b0 = fr_hash(band_cell * 3.1 + 5.0);
		float b1 = fr_hash(band_cell * 3.1 + 61.0);
		float b2 = fr_hash(band_cell * 3.1 + 113.0);
		float band_half_l = b1 > 0.55 ? half_width * 0.5 + 0.12 : half_width + 0.4;
		float band_l = b1 > 0.55 ? (b1 > 0.78 ? 1.0 : -1.0) * half_width * 0.5 : 0.0;
		float band = max(abs(s - (band_cell * 31.0 + 4.0 + b2 * 23.0)) - (0.6 + b0 * 3.2),
			abs(l - band_l) - band_half_l);
		band = b0 < 0.40 ? band : 1.0e3;
		float strip_cell = floor(s / 59.0);
		float t0 = fr_hash(strip_cell * 5.3 + 17.0);
		float t1 = fr_hash(strip_cell * 5.3 + 71.0);
		float t2 = fr_hash(strip_cell * 5.3 + 131.0);
		float strip_half_w = 0.30 + t1 * 0.45;
		float strip_l = (t2 > 0.5 ? 1.0 : -1.0) * max(half_width - 0.6 - t0 * 1.8 - strip_half_w, 0.0);
		float strip_half_s = 4.5 + t2 * 18.0;
		float strip = max(abs(s - (strip_cell * 59.0 + 2.0 + strip_half_s + t1 * (52.0 - 2.0 * strip_half_s))) - strip_half_s,
			abs(l - strip_l) - strip_half_w);
		strip = t0 < 0.45 ? strip : 1.0e3;
		float trench = road_space ? min(band, strip) : 1.0e3;
		// Each trench patch has its own age: fresh ones are dark, old ones faded nearly to the road.
		float trench_age = band < strip ? fr_hash(band_cell + 7.0) : fr_hash(strip_cell + 29.0);
		float trenched = 1.0 - smoothstep(-aa, aa, trench);
		float trench_seal = (1.0 - smoothstep(0.015, 0.05, abs(trench))) * (1.0 - trench_age) * 0.8;
		// Small cut rectangles or ragged hand patches, one candidate per 11 m cell.
		float ps = local_s - (2.4 + h0 * 5.5);
		float pl = l - (h2 - 0.5) * (2.0 * max(half_width, 1.5) - 2.0);
		float x = ps + pl * (h2 - 0.5) * 0.45;
		float y = pl + ps * (h1 - 0.5) * 0.12;
		float box = max(abs(x) / (0.32 + 2.0 * h1), abs(y) / (0.23 + 0.70 * h0));
		vec2 pr = vec2(x / (0.28 + 1.9 * h1), y / (0.24 + 0.72 * h0));
		float rag = length(pr) + (fr_noise(s * 3.0 + l * 5.0 + cell) - 0.5) * 0.31 + (fr_noise(l * 11.0 - s * 7.0) - 0.5) * 0.10;
		float dist = (h0 > 0.5 ? box : rag) - 1.0;
		float active = smoothstep(0.35 + h1 * 0.60, 0.45 + h1 * 0.60, a);
		float patched = (1.0 - smoothstep(-aa, aa, dist)) * active;
		float seal = max((1.0 - smoothstep(0.022, 0.075, abs(dist))) * active, trench_seal);
		// Lengthwise cracks follow the centre joint and the edges; transverse ones cross a cell.
		float wc = (fr_noise(s * 0.7 + 7.0) - 0.5) * 0.30 + (fr_noise(s * 4.2) - 0.5) * 0.05;
		float ld = min(abs(l - 0.11 - wc), abs(l - (half_width - 0.4) + wc));
		float cw = 0.003 + 0.019 * pow(fr_noise(s * 0.49 + l * 0.8 + 4.0), 3.0);
		float crack = along * fr_line(ld, cw, aa) * smoothstep(0.15 + 0.40 * fr_noise(s * 0.13), 0.35 + 0.4 * fr_noise(s * 0.13), a);
		float cd = local_s - (1.0 + h2 * 8.5) - (fr_noise(l * 1.6 + cell * 7.0) - 0.5) * 0.50;
		float cc = fr_line(cd, 0.003 + 0.014 * fr_noise(l * 2.0 + cell), aa) * smoothstep(h0 * 0.65 + 0.2, h0 * 0.65 + 0.35, a);
		crack = max(crack, along * cc);
		float joint = along * fr_line(ld, 0.027 + 0.02 * fr_noise(s * 0.3), aa) * smoothstep(0.60, 0.80, fr_noise(s * 0.23 + 6.0)) * a;
		seal = max(seal, joint);
		// Asphalt oxidises from near black to the light grey of its granite aggregate: linear
		// albedo about 0.07 when fresh, 0.17 on an old residential street, with the stones
		// standing out more as the binder wears off them. Polished wheel paths are lighter.
		float level = mix(0.07, 0.17, a) + 0.012 * (fr_noise(s * 0.05 + l * 0.2 + 3.0) - 0.5);
		// Contrast relative to the level: light stones in dark binder, and ravelled coarse patches.
		float stones = (d.r - 0.34) * (1.0 + 1.4 * a) + (fine.r - 0.34) * (0.8 + 1.2 * a);
		float granite = level * (1.0 + stones) + 0.012 * macro + rut * a * 0.03;
		albedo = granite * vec3(1.015, 1.01, 1.0) * (0.72 + 0.56 * smoothstep(0.2, 0.8, mottle));
		// Grime gathers along the kerb, under the sand the winter gritting leaves.
		albedo *= 1.0 - 0.22 * along * smoothstep(half_width - 0.9, half_width - 0.2, abs(l) + (mottle - 0.5) * 0.4);
		albedo = mix(albedo, albedo * (0.43 + 0.26 * h2), patched);
		// A trench patch is newer asphalt with fewer exposed stones, darker the younger it is.
		float trench_level = mix(0.07, level * 0.96, trench_age) * (1.0 + stones * (0.3 + 0.4 * trench_age));
		albedo = mix(albedo, trench_level * vec3(1.0, 1.0, 0.99) * (0.8 + 0.4 * smoothstep(0.2, 0.8, mottle)),
			trenched);
		albedo = mix(albedo, vec3(0.037, 0.034, 0.031), seal * 0.80);
		albedo = mix(albedo, vec3(0.014, 0.014, 0.013), crack * (0.50 + 0.40 * fr_noise(s * 1.7 + l)));
		height_m = -0.02 * abs(l) * along - (0.002 + 0.016 * a) * rut - 0.0015 * crack + 0.001 * patched;
		// Frost heave and settlement: a few millimetres over metres, which a low sun picks out.
		height_m += 0.002 * trenched + 0.006 * (mottle - 0.5) + 0.0015 * (fine.a - 0.3) * (0.5 + a);
		roughness = 0.84 - 0.2 * rut * a - 0.10 * patched - 0.2 * seal
			- 0.05 * trenched * (1.0 - trench_age) + 0.08 * (d.r - 0.34) * a;
		water = w * smoothstep(0.3, 0.85, rut * a * (0.8 + 0.3 * macro));
		micro_slope = ((d.gb - 0.5) * (0.45 + 0.35 * a) + (fine.gb - 0.5) * (0.15 + 0.25 * a))
			* mix(1.0, 0.6, trenched);
		fr_street_fittings(road_space, s, l, half_width, lane_width, aa, albedo, roughness, height_m);
		// Ordinary asphalt takes one cheap branch, with no extra texture samples or paint noise.
		if (crossing.z > 0.5) {
			fr_zebra_paint(crossing, world_xz, lane_width, aa, d, fine, crack,
				albedo, roughness, micro_slope);
		}
	}
	// Crumbling edge into a gravel shoulder; kerbed streets hide it under the kerb.
	vec3 shoulder = (0.22 + d.r * 0.14) * vec3(1.08, 1.02, 0.89);
	albedo = mix(albedo, shoulder, edge);
	roughness = mix(roughness, 0.98, edge);
	water *= 1.0 - edge;
	albedo *= 1.0 - w * 0.35 - water * 0.15;
	roughness = mix(roughness, gravel ? 0.40 : 0.31, w * 0.65);
	roughness = mix(roughness, 0.085, water);
	micro_slope *= 1.0 - 0.95 * water;
}

// Non-crossing callers retain the same surface contract; the zero mask removes paint work.
void fr_surface(sampler2D detail_map, vec2 world_xz, vec2 road_m, bool road_space,
		float lanes, float lane_width, float traffic, float age, float wetness, bool gravel, float grass,
		float footprint_m, out vec3 albedo, out float roughness, out float height_m,
		out vec2 micro_slope, out float water) {
	fr_surface_painted(detail_map, world_xz, road_m, road_space, lanes, lane_width, traffic, age,
		wetness, gravel, grass, footprint_m, vec4(0.0), albedo, roughness, height_m, micro_slope, water);
}

// Tangent-space normal (x along road_m.x, y along road_m.y) from the relief height and micro
// slopes. Call in uniform control flow: it takes screen derivatives.
vec3 fr_relief_normal(vec2 road_m, float height_m, vec2 micro_slope, float water) {
	vec2 dx = dFdx(road_m);
	vec2 dy = dFdy(road_m);
	float hx = dFdx(height_m);
	float hy = dFdy(height_m);
	float determinant = dx.x * dy.y - dy.x * dx.y;
	vec2 gradient = vec2(0.0);
	if (abs(determinant) > 1e-10) {
		gradient = vec2(hx * dy.y - hy * dx.y, dx.x * hy - dy.x * hx) / determinant;
	}
	vec3 n = normalize(vec3(-gradient - micro_slope, 1.0));
	return normalize(mix(n, vec3(0.0, 0.0, 1.0), water * 0.9));
}
