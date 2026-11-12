// The agents' draw distance fade (`RENDER-28`, terrain.md "Agent draw distances"): a walker or a
// moving car is drawn whole within `fade_m.x` metres of the main camera, loses pixels over the
// band to `fade_m.y` and draws nothing beyond, where the render snapshot stops posing it a little
// further out (config.rs `WALKER_DRAW_DISTANCE_M`, `CAR_DRAW_DISTANCE_M`). The distance is the
// instance origin's from the main camera, so the shadow passes fade an agent with its body. No
// alpha blending: a fading agent discards a share of its pixels by a screen-space dither.

// The share of the agent drawn: exactly 1 within `fade_m.x`, so nothing nearer changes, falling
// smoothly to 0 at `fade_m.y`. Call from vertex(), where MODEL_MATRIX includes the MultiMesh
// instance's transform.
float agent_draw_fade(mat4 model, mat4 main_camera, vec2 fade_m) {
	return 1.0 - smoothstep(fade_m.x, fade_m.y, distance(model[3].xyz, main_camera[3].xyz));
}

// Whether the pixel at `fragcoord` is one a fading agent drops: interleaved gradient noise against
// the share drawn, as Godot's own distance-fade dither, so the kept pixels follow `fade` without a
// regular pattern. Never at a share of 1.
bool agent_draw_dropped(float fade, vec2 fragcoord) {
	const vec3 magic = vec3(0.06711056, 0.00583715, 52.9829189);
	return fade < 1.0 && fade <= fract(magic.z * fract(dot(fragcoord, magic.xy)));
}
