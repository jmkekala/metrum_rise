shader_type spatial;
render_mode blend_mix, depth_draw_opaque, cull_back, diffuse_burley, specular_schlick_ggx, shadows_disabled;

// One vehicle body: every tier of every model, moving and parked (`RENDER-65`, docs/vehicles.md
// "Renderer"). The mesh carries its ids in CUSTOM0.x, one whole number holding three bytes
// (bits 0-7 part, 8-15 surface, 16-23 paint), not in COLOR: the Compatibility renderer multiplies COLOR by the instance colour even for a
// MultiMesh without colours, and that colour is then unwritten memory (`RENDER-70`). Each car's
// data comes in INSTANCE_CUSTOM: x the front wheels' steer angle, y the wheels' roll angle (both
// radians), z the 8 lamp bits, w the look word (6 paint bits, then 4 blink bits). Compatibility
// keeps custom data as half floats, so each whole number stays below 2048 and exact. The model
// was turned half a turn about Y at load (cars drive along their -Z), so the wheel centres are in
// that frame.
// The steer, roll, brake, indicator and reverse values come from Rust (`RENDER-66`,
// nodes/sim/render/vehicle_signals.rs and car_visual.rs).

global uniform vec2 scene_window_clock; // hour, true solar elevation

uniform sampler2D albedo_texture : source_color, filter_linear_mipmap_anisotropic, repeat_disable;
uniform sampler2D orm_texture : hint_default_white, filter_linear_mipmap_anisotropic, repeat_disable;
uniform sampler2D normal_texture : hint_normal, filter_linear_mipmap_anisotropic, repeat_disable;
uniform sampler2D emission_texture : source_color, hint_default_black, filter_linear_mipmap, repeat_disable;
uniform bool has_normal = false;
// False for a livery that paints its panels itself: painted faces stay as the albedo has them.
uniform bool paint_enabled = true;
// The model's palette in linear colour, by paint index.
uniform vec3 palette[64];
// Wheel parts 1 to 8: centre, then 1 for a wheel that steers.
uniform vec4 wheels[8];
uniform float lamp_energy = 4.0;
uniform vec2 lamps_on_elevation_deg = vec2(4.0, -2.0);
// The cars' draw distance fade band from the camera, metres (`RENDER-28`, `RENDER-48`).
uniform vec2 draw_fade_m = vec2(1.0e9, 2.0e9);
// A parked tier's band of each car's own distance from the main camera, metres, `[x, y)`
// (`RENDER-69`): the parked layer draws a chunk's cars in all four tiers' batches, and each tier
// keeps only the cars in its band, so exactly one tier draws a car. The moving cars, already
// bucketed by tier, keep the default, every distance.
uniform vec2 tier_band_m = vec2(0.0, 1.0e30);
uniform float specular = 0.5;

// The ids, decoded once a vertex: a triangle's corners are identical, so flat is exact.
varying flat float paint_face;
varying flat vec3 paint_colour;
varying flat float lamp_gain;
varying flat float draw_fade;

#define STANDARD_LIGHT_DIFFUSE_BURLEY
#include "res://scripts/shaders/standard_light.gdshaderinc"
#include "res://scripts/shaders/weather_rain.gdshaderinc"
#include "res://scripts/shaders/weather_wind.gdshaderinc"
#include "res://scripts/shaders/agent_draw_fade.gdshaderinc"

// Lamp bits of INSTANCE_CUSTOM.z (docs/vehicles.md "Per-car data").
const int LIGHTS_ON = 1;
const int BRAKE = 2;
const int INDICATOR_LEFT = 4;
const int INDICATOR_RIGHT = 8;
const int REVERSE = 16;
const int HIGH_BEAM = 32;

// The lit share of lamp surface `surface` (1 to 15; glass and plain faces stay dark) for the
// lamp bits, the night value and the blink.
float lamp_share(int surface, int lamps, float night, float blink) {
	bool on = (lamps & LIGHTS_ON) != 0;
	bool brake = (lamps & BRAKE) != 0;
	if (surface == 1 || surface == 15) { return on ? night : 0.0; }        // tail, marker
	if (surface == 2) { return brake ? 1.0 : 0.0; }                        // brake
	if (surface == 3) { return brake ? 1.0 : (on ? 0.5 * night : 0.0); }   // tail and brake
	if (surface == 4) { return (lamps & INDICATOR_LEFT) != 0 ? blink : 0.0; }
	if (surface == 5) { return (lamps & INDICATOR_RIGHT) != 0 ? blink : 0.0; }
	if (surface == 6) { return (lamps & REVERSE) != 0 ? 1.0 : 0.0; }
	if (surface == 7) { return on ? night : 0.0; }                         // low beam
	if (surface == 8) { return (lamps & HIGH_BEAM) != 0 ? night : 0.0; }
	return 0.0;
}

void vertex() {
	// One float, exact below 2^24, read per vertex: no interpolation touches it here.
	int packed = int(CUSTOM0.x + 0.5);
	vec3 ids = vec3(float(packed & 255), float((packed >> 8) & 255), float((packed >> 16) & 255));
	int part = int(ids.r);
	if (part >= 1 && part <= 8) {
		vec4 wheel = wheels[part - 1];
		// Roll is right-handed about the model's +X, which is -X after the half turn; steer is
		// right-handed about +Y in either frame (docs/vehicles.md "Wheels").
		float roll = -INSTANCE_CUSTOM.y;
		float steer = INSTANCE_CUSTOM.x * wheel.w;
		mat3 spin = mat3(vec3(1.0, 0.0, 0.0), vec3(0.0, cos(roll), sin(roll)), vec3(0.0, -sin(roll), cos(roll)));
		mat3 turn = mat3(vec3(cos(steer), 0.0, -sin(steer)), vec3(0.0, 1.0, 0.0), vec3(sin(steer), 0.0, cos(steer)));
		mat3 rotation = turn * spin;
		VERTEX = wheel.xyz + rotation * (VERTEX - wheel.xyz);
		NORMAL = rotation * NORMAL;
		TANGENT = rotation * TANGENT;
		BINORMAL = rotation * BINORMAL;
	}
	int lamps = int(INSTANCE_CUSTOM.z + 0.5) & 255;
	int look = int(INSTANCE_CUSTOM.w + 0.5);
	int paint = look & 63;
	float blink_offset = float((look >> 6) & 15) / 16.0;
	paint_face = ids.b;
	paint_colour = paint_enabled ? palette[paint] : vec3(1.0);
	float night = smoothstep(lamps_on_elevation_deg.x, lamps_on_elevation_deg.y, scene_window_clock.y);
	// 90 flashes a minute on the wind clock (`RENDER-66`): it stops while the game is paused and
	// keeps normal pace above normal speed. 1.5 flashes a second complete a whole number of
	// flashes in the clock's wrap, so the blink runs on across it.
	float blink = step(0.5, fract(weather_wind.x * 1.5 + blink_offset));
	int surface = int(ids.g);
	lamp_gain = surface >= 1 && surface <= 15 ? lamp_share(surface, lamps, night, blink) : 0.0;
	draw_fade = agent_draw_fade(MODEL_MATRIX, MAIN_CAM_INV_VIEW_MATRIX, draw_fade_m);
	// The fade's distance, so the shadow passes keep the tier the main view draws.
	float eye = distance(MODEL_MATRIX[3].xyz, MAIN_CAM_INV_VIEW_MATRIX[3].xyz);
	// Faded out, or another tier's car: every triangle collapses to the origin and rasterizes
	// nothing.
	if (draw_fade <= 0.0 || eye < tier_band_m.x || eye >= tier_band_m.y) {
		VERTEX = vec3(0.0);
	}
}

#define SCENE_WINDOW_CLOCK_DECLARED
#include "res://scripts/shaders/street_lamp_light.gdshaderinc"

void fragment() {
	if (agent_draw_dropped(draw_fade, FRAGCOORD.xy)) {
		discard;
	}
	vec3 albedo = texture(albedo_texture, UV).rgb;
	// B is 0 or 255; painted panels are neutral grey in the albedo and take the paint.
	if (paint_face > 127.0) {
		albedo *= paint_colour;
	}
	vec3 orm = texture(orm_texture, UV).rgb;
	ALBEDO = albedo;
	AO = orm.r;
	AO_LIGHT_AFFECT = 0.5;
	ROUGHNESS = orm.g;
	METALLIC = orm.b;
	SPECULAR = specular;
	vec3 surface_albedo = ALBEDO;
	float surface_roughness = ROUGHNESS;
	weather_wet_surface(weather_wetness((INV_VIEW_MATRIX * vec4(NORMAL, 0.0)).y),
		0.2 * (1.0 - METALLIC), surface_albedo, surface_roughness);
	ALBEDO = surface_albedo;
	ROUGHNESS = surface_roughness;
	if (has_normal) {
		NORMAL_MAP = texture(normal_texture, UV).rgb;
	}
	// Lit lenses only: glass and the cabin picture stay dark until lit cabins are built.
	EMISSION = texture(emission_texture, UV).rgb * lamp_gain * lamp_energy;
	standard_light_cloud_sun = standard_light_cloud(VERTEX, INV_VIEW_MATRIX);
	EMISSION += ALBEDO * (1.0 - METALLIC) * mix(1.0, AO, AO_LIGHT_AFFECT) * street_lamp_surface(VERTEX, NORMAL, TANGENT,
		BINORMAL, NORMAL_MAP, NORMAL_MAP_DEPTH, INV_VIEW_MATRIX, CAMERA_POSITION_WORLD);
}
