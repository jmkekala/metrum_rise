// One world-space PBR texture contract for roads, authored yards and graded frontage.
// Direct highlights are disabled, but sky reflections still use this dielectric response.
const float PAVING_SPECULAR = 0.5;
struct PavingSample {
	vec3 albedo;
	vec3 normal_map;
	float roughness;
};

PavingSample sample_paving(
	sampler2D albedo_texture, sampler2D normal_texture, sampler2D roughness_texture,
	vec2 world_xz, float detail_scale, float macro_scale, float macro_blend,
	float brightness_scale, vec3 minimum_albedo, float minimum_blend
) {
	vec2 detail_uv = world_xz * detail_scale;
	vec2 macro_uv = world_xz * macro_scale;
	vec3 detail = texture(albedo_texture, detail_uv).rgb;
	vec3 macro = texture(albedo_texture, macro_uv).rgb;
	vec3 albedo = detail * mix(vec3(1.0), macro * 1.2, macro_blend) * brightness_scale;
	return PavingSample(
		mix(albedo, max(albedo, minimum_albedo), minimum_blend),
		texture(normal_texture, detail_uv).rgb,
		mix(texture(roughness_texture, detail_uv).r,
			texture(roughness_texture, macro_uv).r, macro_blend * 0.5)
	);
}

vec3 paving_tangent(vec3 surface_normal, mat4 world_to_view) {
	// Match world X/Z texture axes even when a mesh has no UVs/tangents or is rotated.
	vec3 world_x_view = (world_to_view * vec4(1.0, 0.0, 0.0, 0.0)).xyz;
	return normalize(world_x_view - surface_normal * dot(world_x_view, surface_normal));
}

vec3 paving_diffuse(vec3 surface_normal, vec3 light_direction, vec3 light_color, float attenuation) {
	return light_color * attenuation * max(dot(surface_normal, light_direction), 0.0) / PI;
}
