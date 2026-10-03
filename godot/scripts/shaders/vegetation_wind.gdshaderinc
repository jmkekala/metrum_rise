// Handover from the branched tree to its impostor. Each tree dissolves from one into the other
// by its own distance, so no patch changes level as a block. Set from tree_species.gd.
uniform float tree_crossfade_begin_m = 150.0;
uniform float tree_crossfade_end_m = 200.0;

// Share of one tree drawn as its impostor. Measured to the main camera and not the rendering
// one, so the shadow pass hands over the same tree at the same moment as the colour pass.
float vegetation_crossfade(vec3 tree_origin, vec3 main_camera) {
	return smoothstep(tree_crossfade_begin_m, tree_crossfade_end_m, distance(tree_origin, main_camera));
}

// Whether this vertex is drawn for a shadow map. Only the colour pass renders from the main
// camera; a directional shadow camera stands far off along the light.
bool vegetation_shadow_pass(vec3 camera, vec3 main_camera) {
	return distance(camera, main_camera) > 0.01;
}

// Dither threshold of one pixel, in [0, 1). Screen space is the only space the cards and the quad
// share. Interleaved gradient noise spreads the threshold evenly over small pixel blocks.
float vegetation_dither_threshold(vec2 pixel) {
	return fract(52.9829189 * fract(dot(pixel, vec2(0.06711056, 0.00583715))));
}

// Whether one pixel of one representation survives the handover. The branched level keeps the
// pixels whose threshold is at or above the impostor share and the impostor keeps the rest, so
// together they cover each pixel once.
bool vegetation_crossfade_keeps(vec2 pixel, float impostor_share, bool impostor) {
	float threshold = vegetation_dither_threshold(pixel);
	return impostor ? threshold < impostor_share : threshold >= impostor_share;
}

// Handover from the full branched tree to its reduced level, per tree like the one above. The
// patch used to swap the mesh of all its trees at once when its centre crossed one distance, so
// trees 15 m away changed together with trees 100 m away. Set from tree_species.gd.
uniform float tree_detail_begin_m = 45.0;
uniform float tree_detail_end_m = 55.0;
// Whether this material draws the reduced level. Its surfaces sit beside the full level's in the
// mesh a patch inside the band draws, so each tree keeps its own share of both.
uniform bool reduced_level = false;

// Share of one tree drawn from its reduced level. A tree that takes no handover draws whichever
// level it was given whole.
float vegetation_reduced_share(vec3 tree_origin, vec3 main_camera, bool hands_over) {
	return hands_over
		? smoothstep(tree_detail_begin_m, tree_detail_end_m, distance(tree_origin, main_camera))
		: (reduced_level ? 1.0 : 0.0);
}

// Dither thresholds one branched level keeps, as [x, y). The impostor keeps [0, impostor share),
// the reduced level the thresholds after it up to its own share, and the full level the rest.
// The three ranges meet, so every pixel of a tree is drawn by exactly one of them.
vec2 vegetation_level_keeps(float impostor_share, float reduced_share) {
	float full_from = max(impostor_share, reduced_share);
	return reduced_level ? vec2(impostor_share, full_from) : vec2(full_from, 2.0);
}

// World XZ direction; zero direction disables displacement without a normalization NaN.
uniform vec2 wind_direction = vec2(1.0, 0.35);
uniform float wind_strength : hint_range(0.0, 2.0) = 1.0;
// Radians per second; wavelength is in world metres.
uniform float wind_speed : hint_range(0.0, 4.0) = 0.8;
uniform float gust_wavelength : hint_range(1.0, 500.0) = 80.0;

// The largest lateral offset the term below can produce, at weight 1 and default strength.
const float WIND_MAX_OFFSET_M = 0.50;
// Screen-space bounds of the gate, in pixels of that offset, and the shape of the ramp between
// them. The exponent biases the curve toward the near end: a linear smoothstep spends most of
// its range on distances where the sway is visible without reading as sway.
const float WIND_FADE_MIN_PX = 1.4;
const float WIND_FADE_FULL_PX = 4.4;
const float WIND_FADE_EXPONENT = 1.30;

// 1 / tan(fov_y / 2) of the player camera. PROJECTION_MATRIX reads as a zero matrix in the vertex
// stage, so the one term the gate cannot take from a built-in is supplied here instead.
// VIEWPORT_SIZE is populated, so window size still needs no plumbing and a resize needs no
// update. See CAMERA_FOV_DEG in tree_species.gd; the default is the engine's own 75 degrees.
uniform float camera_projection_scale = 1.303;

// Share of the wind a tree at this depth is allowed to take.
//
// Sway is authored in metres and drawn in pixels, and nothing connected the two. At 800 m, the
// far edge of the near level, one pixel covers 1.14 m of world and the whole 0.50 m offset is
// under half of one. Motion that small cannot move a crown's outline. It can only flip
// alpha-scissor pixels on and off along every hard cutout edge in the stand, frame after frame,
// which is why a forest seen from altitude boils rather than sways.
//
// One pixel of offset is where that failure begins, not where it ends. Between one and two
// pixels the sway moves a crown by a fraction of its own width, which a single tree reads as
// motion and a closed stand reads as the same shimmer with a longer period. So the band is
// authored past the resolution floor rather than at it: wind is a close-up effect, and the
// exponent keeps the tail short instead of trailing a few percent of sway across the middle
// distance.
//
// Measured on a 625 stems/ha conifer stand at 1080p and 75 degrees, as the share of crown
// pixels one frame of sway moves by more than 0.02 luminance: 49 percent at 100 m, 31 percent
// at 150 m, 8 percent at 200 m, 0.5 percent at 250 m and none past 251 m, where the offset
// falls below WIND_FADE_MIN_PX. Both terms are screen-space, so a different resolution or
// field of view moves those distances rather than changing what is drawn.
float vegetation_wind_visibility(float distance_m, vec2 viewport) {
	// Pixels one world metre covers at this depth.
	float px_per_m = camera_projection_scale * viewport.y * 0.5 / max(distance_m, 0.001);
	float ramp = smoothstep(WIND_FADE_MIN_PX, WIND_FADE_FULL_PX, WIND_MAX_OFFSET_M * px_per_m);
	// The impostor does not sway, so the sway is gone before a tree starts to become one.
	float before_handover = 1.0 - smoothstep(tree_crossfade_begin_m - 50.0, tree_crossfade_begin_m,
		distance_m);
	return pow(ramp, WIND_FADE_EXPONENT) * before_handover;
}

// O(1) per near vertex: three sine terms, no textures or inverse transforms.
// Only instance origin and authored weight participate, so cores and cards stay together.
vec3 vegetation_wind_offset(vec2 instance_xz, float weight, float time, float visibility) {
	vec2 direction = wind_direction / max(length(wind_direction), 0.0001);
	vec2 across = vec2(-direction.y, direction.x);
	float phase = time * wind_speed - dot(instance_xz, direction) * TAU / max(gust_wavelength, 1.0);
	float bend = 0.36 * sin(phase) + 0.08 * sin(phase * 0.5);
	float flutter = 0.06 * sin(phase * 4.0) * weight;
	// At weight 1, default strength and full visibility, the offset is WIND_MAX_OFFSET_M.
	vec2 offset = (direction * bend + across * flutter) * weight * wind_strength * visibility;
	return vec3(offset.x, 0.0, offset.y);
}

// Transmission tint for light passing through a leaf, not reflecting off it.
uniform vec3 foliage_backlight : source_color = vec3(0.080, 0.150, 0.040);

// Foliage transmits light; bark does not. One material carries both, and splitting them
// would cost draw calls on a scatter that is already draw-call bound, so the authored
// palette's green bias is the discriminator: every foliage colour has g well above r,
// every bark and rock colour does not. No per-vertex tag and no extra attribute.
vec3 vegetation_backlight(vec3 albedo) {
	return foliage_backlight * smoothstep(0.0, 0.05, albedo.g - albedo.r);
}

// Distance over which a tree stops receiving cast shadows, just inside the sun's shadow range.
// A distant tree casts its own baked silhouette, set back along the light, so a stand shadows
// itself tree by tree out to here. Past the fade the canopy shade term below stands in for those
// shadows. Set from tree_species.gd.
uniform float tree_receive_begin_m = 90.0;
uniform float tree_receive_end_m = 120.0;

// Share of the cast shadow a tree at this distance receives.
float vegetation_shadow_keep(float distance_m) {
	return 1.0 - smoothstep(tree_receive_begin_m, tree_receive_end_m, distance_m);
}

// Cast shadow a tree receives from one light. Directional attenuation is the shadow alone; other
// lights keep theirs.
float vegetation_shadow(float attenuation, bool directional, float shadow_keep) {
	return directional ? mix(1.0, attenuation, shadow_keep) : attenuation;
}

global uniform vec3 scene_sun_direction;
// Sun a crown surface keeps inside a stand, from its neighbours' crowns. A sun ray from a surface
// `d` below the canopy top crosses the crown layer over `d * cot(elevation)` of ground, and in a
// stand of closure `c` it meets crowns at STAND_SUN_DENSITY * c per metre, so the share that
// passes falls as exp(-density * c * path). It falls to STAND_SUN_FLOOR, not zero: that is the
// share of sun the open crowns of a closed stand pass to its floor, CANOPY_FLOOR_SUN_VISIBILITY
// in scene_lighting.gd, which a ray at the floor has crossed the whole layer to reach. A lone
// tree has closure zero and keeps its sun. The cast shadows inside their range already hold
// neighbours' crowns, so the two combine as the darker of them, not as a product.
// STAND_SUN_DENSITY is fitted: a painted mixed stand at 13:00 goes from 0.62 and 0.53 of the
// meadow beside it, from 180 m and 140 m up, to 0.44 and 0.37, inside the 0.30-0.44 the
// forest-edge photographs measure over green ground.
const float STAND_CROWN_TOP_M = 20.0;
const float STAND_SUN_FLOOR = 0.24;
const float STAND_SUN_DENSITY = 0.3;
float vegetation_stand_sun(float height_m, float closure) {
	vec3 sun = scene_sun_direction;
	float path = max(STAND_CROWN_TOP_M - height_m, 0.0) * length(sun.xz) / max(sun.y, 0.02);
	return mix(STAND_SUN_FLOOR, 1.0, exp(-STAND_SUN_DENSITY * closure * path));
}

// Diffuse light on a tree: the engine's Lambert, wrap and backlight terms, restated so that the
// shadow can fade out. `wrap` zero is plain Lambert.
vec3 vegetation_diffuse_light(vec3 normal, vec3 light, vec3 light_color, float shadow,
		vec3 backlight, float wrap) {
	float wrap_scale = 1.0 + wrap;
	float n_dot_l = max(0.0, (dot(normal, light) + wrap) / (wrap_scale * wrap_scale));
	return light_color / PI * (n_dot_l + (1.0 - n_dot_l) * backlight) * shadow;
}

// Specular light on a tree: the engine's GGX term at the roughness of one every tree material
// uses, where the distribution is 1 / PI and the visibility term is 0.5 / (NdotL + NdotV). The
// reflectance is the engine's 0.04 for its default specular of 0.5.
vec3 vegetation_specular_light(vec3 normal, vec3 light, vec3 view, vec3 light_color,
		float shadow, float specular_amount) {
	float n_dot_l = max(dot(normal, light), 0.0);
	float n_dot_v = max(dot(normal, view), 0.0001);
	float l_dot_h = max(dot(light, normalize(light + view)), 0.0);
	float fresnel = 0.04 + 0.96 * pow(1.0 - l_dot_h, 5.0);
	return light_color * n_dot_l / PI * fresnel * 0.5 / (n_dot_l + n_dot_v) * shadow * specular_amount;
}

// Canopy shading. Past the fade above no tree receives a cast shadow, so a stand loses the one
// thing that makes a forest darker than a single tree: its members shadow each other. The
// replacement is one multiplier per instance that fades in over the same distances the cast
// shadow fades out over. It is keyed on the instance origin, which the near and the distant mesh
// of one tree share, so a tree keeps its shade when its mesh changes.
uniform float canopy_shade : hint_range(0.0, 2.0) = 1.0;

// Share of the key light a shadowed crown keeps. A crown in a neighbour's shadow still collects
// sky light and light scattered off the stand around it, so it is not black. This value leaves
// the multiplier with a mean of 0.70, which is what a canopy keeps when about two fifths of its
// visible surface sits in shadow at ambient level.
const float CANOPY_SHADE_FLOOR = 0.40;
// Metres per noise cell. Canopy shadow falls in clumps of crowns rather than tree by tree, and
// an uncorrelated value per instance reads as static at the distances this applies to: nothing
// here filters it over time. Default canopy density is 30.5 stems per hectare, so mean spacing
// is 18 m and one cell covers a group of roughly seven trees.
const float CANOPY_SHADE_CELL_M = 48.0;

// Deterministic value in [0,1) for one integer cell. Unsigned arithmetic only; neither TIME nor
// screen coordinates participates, so two runs of one frame shade the stand identically.
float canopy_shade_cell(vec2 cell) {
	uvec2 c = uvec2(ivec2(cell));
	uint h = c.x * 374761393u + c.y * 668265263u;
	h = (h ^ (h >> 13u)) * 1274126177u;
	return float((h ^ (h >> 16u)) & 16777215u) / 16777216.0;
}

// Smoothly interpolated value noise on the ground plane. Four hashes, and every vertex of one
// instance gets the same answer because the input is the instance origin rather than the vertex.
float canopy_shade_noise(vec2 instance_xz) {
	vec2 p = instance_xz / CANOPY_SHADE_CELL_M;
	vec2 cell = floor(p);
	vec2 f = p - cell;
	f = f * f * (3.0 - 2.0 * f);
	float a = canopy_shade_cell(cell);
	float b = canopy_shade_cell(cell + vec2(1.0, 0.0));
	float c = canopy_shade_cell(cell + vec2(0.0, 1.0));
	float d = canopy_shade_cell(cell + vec2(1.0, 1.0));
	return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// Share of its light the base of a trunk keeps past the fade, and the height at unit scale
// where the trunk zone ends and the crown begins. Crowns shade the space under them from the sun
// and most of the sky, so a stand seen from the side is bright at the top and dark toward the
// ground. Without it the whole trunk zone of a far stand was lit like the crown tops, and the
// stand read as a canopy floating on a band of lit trunks.
// The term fades in over CANOPY_BASE_FADE_M from canopy_base_begin_m, which is the end of the
// shipped handover, so the branched tree and its impostor never disagree about it while both are
// drawn. It has its own uniform rather than following tree_crossfade_end_m, because a probe that
// moves the handover must still compare the two levels without it.
uniform float canopy_base_begin_m = 200.0;
const float CANOPY_BASE_SHADE = 0.30;
const float CANOPY_BASE_TOP_M = 8.0;
const float CANOPY_BASE_FADE_M = 100.0;

// How far the base term has faded in at one distance.
float vegetation_canopy_base_ramp(float distance_m) {
	return smoothstep(canopy_base_begin_m, canopy_base_begin_m + CANOPY_BASE_FADE_M, distance_m);
}

// Share of its light a surface `height_m` above the instance origin at unit scale keeps.
float vegetation_canopy_base(float height_m, float base_ramp) {
	return mix(1.0, mix(CANOPY_BASE_SHADE, 1.0, smoothstep(0.0, CANOPY_BASE_TOP_M, height_m)),
		base_ramp);
}

// Multiplier on reflected and transmitted colour for one instance at one distance, before its
// base term, and how far that distance has taken the term in. The impostor needs them apart,
// because its base term varies across one card and must be evaluated per fragment.
vec2 vegetation_canopy_shade_parts(vec2 instance_xz, float distance_m) {
	float shade = mix(CANOPY_SHADE_FLOOR, 1.0, canopy_shade_noise(instance_xz));
	float ramp = smoothstep(tree_receive_begin_m, tree_receive_end_m, distance_m) * canopy_shade;
	return vec2(shade, ramp);
}

// The clamp is what lets `canopy_shade` run past one. A strength above one extrapolates past the
// floor, which is how a probe asks for more darkening than the derived default.
float vegetation_canopy_combine(vec2 parts, float base) {
	return clamp(mix(1.0, parts.x * base, parts.y), 0.0, 1.0);
}

// Multiplier on reflected and transmitted colour for one instance at one distance, for a surface
// `height_m` above the instance origin at unit scale.
float vegetation_canopy_shade(vec2 instance_xz, float distance_m, float height_m) {
	return vegetation_canopy_combine(vegetation_canopy_shade_parts(instance_xz, distance_m),
		vegetation_canopy_base(height_m, vegetation_canopy_base_ramp(distance_m)));
}
