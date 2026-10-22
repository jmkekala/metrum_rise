// SPDX-License-Identifier: GPL-2.0-only

// Godot's built-in lighting, reproduced so the sun can pass through the cloud shadow.
//
// Godot gives a directional light no projector texture and a material no hook into its light
// short of replacing light() whole, so a material whose sun the clouds should dim includes this
// in place of the engine's own. It is light_compute() from Godot 4.7.2
// (servers/rendering/renderer_rd/shaders/scene_forward_lights_inc.glsl) for
// specular_schlick_ggx and the Lambert or Burley diffuse, without the clearcoat, anisotropy,
// rim, backlight, transmittance and area-light branches, which no material that includes this
// uses. Two terms the engine does not expose to light() are left at their neutral values: the
// sun's angular size, which only softens N.L by about half a degree, and the multiple-scattering
// energy compensation, which only brightens smooth metals.
//
// Godot defaults a shader that names no diffuse mode to Lambert; BaseMaterial3D names Burley.
// light() cannot see the render mode, so a shader with `diffuse_burley` also defines
// STANDARD_LIGHT_DIFFUSE_BURLEY before including this, and every other one gets Lambert.
//
// The including shader declares `uniform float specular`, as BaseMaterial3D does, and sets
// `standard_light_cloud_sun = standard_light_cloud(VERTEX, INV_VIEW_MATRIX);` in fragment():
// Godot lets only the stage functions write a varying.
#ifndef STANDARD_LIGHT_INCLUDED
#define STANDARD_LIGHT_INCLUDED

#include "res://scripts/shaders/weather_clouds.gdshaderinc"

// Share of the sun the clouds pass to this fragment.
varying float standard_light_cloud_sun;

float standard_light_cloud(vec3 view_vertex, mat4 inv_view) {
	return weather_cloud_sun((inv_view * vec4(view_vertex, 1.0)).xyz);
}

float standard_light_schlick(float u) {
	float m = 1.0 - u;
	float m2 = m * m;
	return m2 * m2 * m;
}

void light() {
	// Clouds dim the sun only; weather_sun.w lets the moon pass untouched.
	float attenuation = ATTENUATION * (LIGHT_IS_DIRECTIONAL ? standard_light_cloud_sun : 1.0);
	float n_dot_l = min(dot(NORMAL, LIGHT), 1.0);
	float c_n_dot_v = max(dot(NORMAL, VIEW), 1e-4);
	float c_n_dot_l = max(n_dot_l, 0.0);
	vec3 half_vector = normalize(VIEW + LIGHT);
	float c_l_dot_h = clamp(dot(LIGHT, half_vector), 0.0, 1.0);
	if (METALLIC < 1.0) {
#ifdef STANDARD_LIGHT_GRASS_TRANSMISSION
		// Thin grass transmits a little back-facing sun, with the same cloud/shadow attenuation.
		DIFFUSE_LIGHT += LIGHT_COLOR * vec3(0.12, 0.16, 0.07) * max(-n_dot_l, 0.0) / PI * attenuation;
#endif
#ifdef STANDARD_LIGHT_DIFFUSE_BURLEY
		float fd90_minus_1 = 2.0 * c_l_dot_h * c_l_dot_h * ROUGHNESS - 0.5;
		float fd_v = 1.0 + fd90_minus_1 * standard_light_schlick(c_n_dot_v);
		float fd_l = 1.0 + fd90_minus_1 * standard_light_schlick(c_n_dot_l);
		DIFFUSE_LIGHT += LIGHT_COLOR * (fd_v * fd_l * c_n_dot_l / PI) * attenuation;
#else
		DIFFUSE_LIGHT += LIGHT_COLOR * (c_n_dot_l / PI) * attenuation;
#endif
	}
	if (ROUGHNESS > 0.0) {
		// Schlick-GGX specular.
		float c_n_dot_h = clamp(dot(NORMAL, half_vector), 0.0, 1.0);
		float alpha_ggx = ROUGHNESS * ROUGHNESS;
		float a = c_n_dot_h * alpha_ggx;
		float k = alpha_ggx / (1.0 - c_n_dot_h * c_n_dot_h + a * a);
		float d = clamp(k * k / PI, 0.0, 65504.0);
		float g = clamp(0.5 / mix(2.0 * c_n_dot_l * c_n_dot_v, c_n_dot_l + c_n_dot_v, alpha_ggx),
			0.0, 65504.0);
		vec3 f0 = mix(vec3(0.16 * specular * specular), ALBEDO, METALLIC);
		float f90 = clamp(dot(f0, vec3(50.0 * 0.33)), METALLIC, 1.0);
		vec3 fresnel = f0 + (f90 - f0) * standard_light_schlick(c_l_dot_h);
		SPECULAR_LIGHT += c_n_dot_l * d * fresnel * g * LIGHT_COLOR * attenuation * SPECULAR_AMOUNT;
	}
}
#endif
