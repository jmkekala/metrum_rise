// Fixed per-building parameters: sun threshold, bedtime (22..26.5), wake hour, profile.
// Shift the day boundary to noon so sleeping crosses midnight without discontinuity.
float window_activity(vec4 schedule, float hour, float sun_elevation) {
	float daylight = 1.0 - smoothstep(schedule.x - 0.35, schedule.x + 0.35, sun_elevation);
	float phase = mod(hour + 12.0, 24.0);
	float asleep = smoothstep(schedule.y - 12.0, schedule.y - 11.9, phase)
		* (1.0 - smoothstep(schedule.z + 12.0, schedule.z + 12.1, phase));
	float awake = schedule.w > 1.5 ? 1.0 : 1.0 - asleep;
	return schedule.w < 0.5 ? 0.0 : daylight * awake;
}
