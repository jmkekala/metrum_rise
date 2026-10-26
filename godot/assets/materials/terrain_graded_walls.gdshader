shader_type spatial;
render_mode blend_mix, depth_draw_opaque, cull_disabled, diffuse_burley, specular_schlick_ggx, skip_vertex_transform;
// The graded terrain's wall faces (TERRAIN-07-D round 4): conflict walls, abutments and headwalls
// (rust/src/simulation/terrain/graded/walls.rs), one MultiMesh instance of a unit quad per face,
// placed here in world space. VERTEX.x is 0 at the face's end a and 1 at b, VERTEX.y 0 at the
// bottom and 1 at the top. The instance transform carries the face's eight values raw, so
// MODEL_MATRIX (the patch node's translation times the instance) holds them in its basis
// columns exactly: column 0 (a.x, bottom at a, a.z) and column 1 (b.x, bottom at b, b.z), both
// relative to the patch centre, column 2 (top at a, top at b, 0); column 3 is the patch centre.
// The look of the exact cut's retaining walls (terrain.gd RETAINING_WALL_COLOR), lit through
// standard_light.gdshaderinc so the clouds dim the sun.

uniform vec3 albedo : source_color = vec3(0.54, 0.54, 0.50);
uniform float roughness = 0.88;
uniform float specular = 0.5;

#define STANDARD_LIGHT_DIFFUSE_BURLEY
#include "res://scripts/shaders/standard_light.gdshaderinc"
#include "res://scripts/shaders/street_lamp_light.gdshaderinc"

void vertex() {
	vec3 a = MODEL_MATRIX[0].xyz;
	vec3 b = MODEL_MATRIX[1].xyz;
	vec2 top = MODEL_MATRIX[2].xy;
	vec3 centre = MODEL_MATRIX[3].xyz;
	bool at_b = VERTEX.x > 0.5;
	vec3 end = at_b ? b : a;
	float y = VERTEX.y > 0.5 ? (at_b ? top.y : top.x) : end.y;
	vec3 world = vec3(centre.x + end.x, centre.y + y, centre.z + end.z);
	// The lower side lies on the left of a -> b (walls.rs WallFace): the face looks that way.
	vec2 along = b.xz - a.xz;
	float len = length(along);
	vec3 normal = len > 0.0 ? vec3(-along.y, 0.0, along.x) / len : vec3(0.0, 1.0, 0.0);
	VERTEX = (VIEW_MATRIX * vec4(world, 1.0)).xyz;
	NORMAL = normalize((VIEW_MATRIX * vec4(normal, 0.0)).xyz);
}

void fragment() {
	ALBEDO = albedo;
	ROUGHNESS = roughness;
	METALLIC = 0.0;
	SPECULAR = specular;
	standard_light_cloud_sun = standard_light_cloud(VERTEX, INV_VIEW_MATRIX);
	// A flat face with no normal map: the lamp light reads the face normal itself, and the quad
	// needs no tangents (a shader reading TANGENT warns on every mesh without them).
	EMISSION += ALBEDO * street_lamp_surface(VERTEX, NORMAL, vec3(0.0), vec3(0.0),
		vec3(0.5, 0.5, 1.0), 1.0, INV_VIEW_MATRIX, CAMERA_POSITION_WORLD);
}
