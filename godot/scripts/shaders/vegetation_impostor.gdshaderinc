// Shared by the impostor and its shadow caster: the hemi-octahedral view grid of the bake
// (tools/bake_tree_impostors.py) and the crop of each camera-facing quad to its baked views.

// Pixels per baked view.
const float FRAME_PX = 64.0;

vec3 hemi_decode(vec2 uv) {
	vec2 p = vec2(uv.x + uv.y, uv.x - uv.y) * 0.5;
	return normalize(vec3(p.x, 1.0 - abs(p.x) - abs(p.y), p.y));
}

vec2 hemi_encode(vec3 d) {
	d.y = max(d.y, 0.0);
	vec3 a = d / (abs(d.x) + abs(d.y) + abs(d.z));
	return vec2(a.x + a.z, a.x - a.z);
}

vec2 project_frame(vec3 offset, vec2 cell) {
	vec3 d = hemi_decode((cell + 0.5) / 8.0 * 2.0 - 1.0);
	vec3 right = normalize(cross(vec3(0.0, 1.0, 0.0), d));
	vec3 up = cross(d, right);
	return vec2(dot(offset, right), -dot(offset, up)) + 0.5;
}

// The quad is square and as wide as the form's bounds, but a pine fills about a twentieth of
// its view: every other pixel ran the alpha-tested fragment in the depth pre-pass, the colour
// pass and the shadow maps, and a painted stand spent 9 ms of pre-pass and 5 ms of shadow on
// impostor pixels it then discarded. Each baked view records the rectangle that holds its
// coverage (frame_bounds in tree_impostors.json), and the quad shrinks to what its views can
// draw.

// Margin in frame UV that a view sampled at this footprint can draw past its level-zero
// rectangle. A coarser mip only scales texels the rectangle holds, but the texel at mip k
// spans 2^k level-zero texels and the bilinear filter reaches half of one further.
// `uv_per_pixel` is the frame UV one screen or shadow-map pixel spans on the quad.
float impostor_crop_pad(float uv_per_pixel) {
	// The margin over the fragment's own estimate covers a frame tilted against the quad.
	float lod = clamp(log2(max(uv_per_pixel * FRAME_PX * 1.5, 1.0)), 0.0, log2(FRAME_PX));
	return 1.5 * exp2(ceil(lod)) / FRAME_PX;
}

// Grows `crop` (a0, b0, a1, b1), in quad units where the whole quad spans -0.5..0.5, to cover
// what one view draws. A view's UV is linear in the quad offset right * a + up * b, so its
// rectangle maps back to a parallelogram on the quad and four corners bound it.
vec4 impostor_crop(vec4 crop, vec3 right, vec3 up, vec2 cell, vec4 rect, float pad) {
	vec3 d = hemi_decode((cell + 0.5) / 8.0 * 2.0 - 1.0);
	vec3 frame_right = normalize(cross(vec3(0.0, 1.0, 0.0), d));
	vec3 frame_up = cross(d, frame_right);
	mat2 to_frame = mat2(vec2(dot(frame_right, right), -dot(frame_up, right)),
		vec2(dot(frame_right, up), -dot(frame_up, up)));
	// A view this far from the quad's own direction is never blended; keep the whole quad.
	if (abs(determinant(to_frame)) < 0.25) {
		return vec4(-0.5, -0.5, 0.5, 0.5);
	}
	mat2 to_quad = inverse(to_frame);
	rect += vec4(-pad, -pad, pad, pad);
	vec2 p0 = to_quad * (rect.xy - 0.5);
	vec2 p1 = to_quad * (rect.zy - 0.5);
	vec2 p2 = to_quad * (rect.xw - 0.5);
	vec2 p3 = to_quad * (rect.zw - 0.5);
	return vec4(min(crop.xy, min(min(p0, p1), min(p2, p3))),
		max(crop.zw, max(max(p0, p1), max(p2, p3))));
}

// The quad offset of this vertex once the quad is cropped. UV (0, 0) is the top left corner.
vec3 impostor_crop_offset(vec4 crop, vec3 right, vec3 up, vec2 uv) {
	crop = clamp(crop, vec4(-0.5), vec4(0.5));
	return right * mix(crop.x, crop.z, uv.x) + up * mix(crop.w, crop.y, uv.y);
}
