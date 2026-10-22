// Planar water reflection pass (water_reflection.gd). Up to two cameras, each mirrored below
// the level of one lake in view, draw the scene into `water_reflection_map` and
// `water_reflection_map_2`, which water on that level samples where screen-space reflection has
// nothing to read: behind a tree standing between the camera and the lake.
//
// `water_reflection_plane`: x = the first mirror's plane height, y = 1 while it renders; z and
// w the same for the second.

global uniform vec4 water_reflection_plane;

// Render layer 20, seen only by the reflection cameras. Must match REFLECTION_LAYER in
// water_reflection.gd.
const uint WATER_REFLECTION_LAYER = 1u << 19u;
// Render layer 17, which holds nothing and is in the second mirror camera's mask only, so a
// shader can tell which mirror it is drawing for. Must match SECOND_MIRROR_LAYER.
const uint WATER_REFLECTION_SECOND_LAYER = 1u << 16u;

// Height of the water plane the camera with `camera_layers` mirrors, or -1e30 for a camera that
// is not a reflection camera.
float water_reflection_mirror_plane(uint camera_layers) {
	if ((camera_layers & WATER_REFLECTION_LAYER) == 0u) {
		return -1.0e30;
	}
	return (camera_layers & WATER_REFLECTION_SECOND_LAYER) != 0u
		? water_reflection_plane.z : water_reflection_plane.x;
}

// True for a fragment a reflection camera must not draw: one below its water plane, such as
// the lake bed, which from below the plane would hide everything the water reflects. Godot's
// camera has no oblique near plane, so the clip is done here.
bool water_reflection_clipped(vec3 fragment_world, uint camera_layers) {
	return fragment_world.y < water_reflection_mirror_plane(camera_layers);
}
