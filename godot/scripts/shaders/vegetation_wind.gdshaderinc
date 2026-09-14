// World XZ direction; zero direction disables displacement without a normalization NaN.
uniform vec2 wind_direction = vec2(1.0, 0.35);
uniform float wind_strength : hint_range(0.0, 2.0) = 1.0;
// Radians per second; wavelength is in world metres.
uniform float wind_speed : hint_range(0.0, 4.0) = 0.8;
uniform float gust_wavelength : hint_range(1.0, 500.0) = 80.0;

// O(1) per near vertex: three sine terms, no textures or inverse transforms.
// Only instance origin and authored weight participate, so cores and cards stay together.
vec3 vegetation_wind_offset(vec2 instance_xz, float weight, float time) {
	vec2 direction = wind_direction / max(length(wind_direction), 0.0001);
	vec2 across = vec2(-direction.y, direction.x);
	float phase = time * wind_speed - dot(instance_xz, direction) * TAU / max(gust_wavelength, 1.0);
	float bend = 0.36 * sin(phase) + 0.08 * sin(phase * 0.5);
	float flutter = 0.06 * sin(phase * 4.0) * weight;
	// At weight 1 and default strength, the offset is bounded by 0.50 m.
	vec2 offset = (direction * bend + across * flutter) * weight * wind_strength;
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
