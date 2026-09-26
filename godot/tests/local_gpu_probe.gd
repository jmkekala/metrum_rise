# SPDX-License-Identifier: GPL-2.0-only

## Local, opt-in rendered GPU probe; writes measurements without saving gameplay state.
## Run from repo root: godot --path godot --windowed --resolution 1280x720 --gpu-profile --script res://tests/local_gpu_probe.gd
## METRUM_GPU_PROBE_OUTPUT selects a fresh output directory; METRUM_GPU_PROBE_WORLD selects a world SQLite.
## METRUM_GPU_PROBE_EXPERIMENT selects E01 (micro grass), E02 (shader cost attribution),
## E03 (long lateral panning), E04 (terrain texture import comparison), E11 (day cycle
## hours) or E12 (tree scatter from an eye-level horizon camera).
## METRUM_GPU_PROBE_VIEWS selects the camera radius sweep.
extends SceneTree

# Matches the renderers' idiom. A bare SceneLighting class_name reference needs
# Godot's global class cache, which a freshly cloned checkout has not built yet.
const SceneLightingConfig := preload("res://scripts/core/scene_lighting.gd")
const DayCycleConfig := preload("res://scripts/core/day_cycle.gd")

# Mid-morning: a raking sun that produces real shadow work, rather than the flattest or the
# darkest hour of the day.
const PROBE_HOUR := 9.0

var main: Node
var camera: Node
var terrain: Node
var output_dir: String
var results: Array = []
var pivot := Vector3.ZERO
var simple_material := StandardMaterial3D.new()
var overrides: Dictionary = {}
var micro_shader: Shader
var original_shaders: Dictionary = {}
var camera_radius := 1200.0
var active_material_count := 0
var variant_shaders: Dictionary = {}
var pan_route_m := 0.0
var pan_axis := Vector3.RIGHT
var pan_origin := Vector3.ZERO
# Slower machines need a longer window to collect a comparable number of samples.
# Applies to stationary trials only; pan trials carry their own durations.
var capture_seconds := 8.0
var vegetation: Node

func _initialize() -> void:
	call_deferred("run")

func run() -> void:
	var seconds_override := OS.get_environment("METRUM_GPU_PROBE_SECONDS")
	if not seconds_override.is_empty():
		capture_seconds = maxf(float(seconds_override), 1.0)
	output_dir = OS.get_environment("METRUM_GPU_PROBE_OUTPUT")
	if output_dir.is_empty():
		output_dir = ProjectSettings.globalize_path("res://../benchmark-results/gpu-%d" % Time.get_unix_time_from_system())
	if DirAccess.dir_exists_absolute(output_dir):
		push_error("Output directory already exists: " + output_dir)
		quit(1)
		return
	DirAccess.make_dir_recursive_absolute(output_dir)
	main = load("res://scenes/Main.tscn").instantiate()
	# Own scene startup without attaching the road workload or changing menu launch state.
	main.set_script(null)
	root.add_child(main)
	current_scene = main
	await process_frame
	vegetation = main.get_node("Vegetation")
	# Keep all earlier experiments comparable: trees are opt-in for this harness.
	vegetation.enabled = false
	var input_manager: Node = main.get_node("InputManager")
	input_manager.set_process(false)
	input_manager.set_process_input(false)
	input_manager.set_process_unhandled_input(false)
	root.set_disable_input(true)
	var world := OS.get_environment("METRUM_GPU_PROBE_WORLD")
	if world.is_empty():
		world = ProjectSettings.globalize_path("res://bootstrap/worlds/kuopio_324km2_10m.sqlite")
	if not input_manager.menu_load_world_definition(world):
		push_error("Could not load probe world: " + world)
		quit(1)
		return
	input_manager.set_simulation_speed(0.0)
	# Trials must share one lighting state. A paused clock already holds the sun still, but the
	# authored start hour is not a measurement constant, so the probe pins its own unless
	# METRUM_TIME_OF_DAY names one.
	if DayCycleConfig.pinned_day_fraction() < 0.0:
		main.get_node("SceneLighting").pin_hour_of_day(PROBE_HOUR)
	camera = main.get_node("CameraNode")
	camera.set_process(false)
	camera.set_process_input(false)
	camera.set_process_unhandled_input(false)
	terrain = main.get_node("Terrain")
	pivot.y = main.get_node("SimulationNode").get_world_surface_height(Vector2.ZERO)
	camera.focus_on(pivot, 1200.0)
	DisplayServer.window_set_mode(DisplayServer.WINDOW_MODE_WINDOWED)
	DisplayServer.window_set_size(Vector2i(1280, 720))
	DisplayServer.window_set_vsync_mode(DisplayServer.VSYNC_DISABLED)
	Engine.max_fps = 0
	RenderingServer.viewport_set_measure_render_time(root.get_viewport_rid(), true)
	simple_material.albedo_color = Color(0.25, 0.45, 0.18)
	simple_material.roughness = 1.0
	var start := Time.get_ticks_msec()
	var stable := 0
	while stable < 30 and Time.get_ticks_msec() - start < 120000:
		await process_frame
		if not terrain.has_pending_render_work(false) and not main.get_node("Water").has_pending_render_work(false):
			stable += 1
		else:
			stable = 0
	if stable < 30:
		push_error("World did not settle within 120 seconds")
		quit(1)
		return
	var experiment := OS.get_environment("METRUM_GPU_PROBE_EXPERIMENT")
	var trials := ["baseline", "half_resolution", "no_shadows", "simple_terrain", "no_water", "baseline_repeat", "pan"]
	var radii := [1200.0]
	if experiment == "E07":
		trials = ["baseline", "trees_half", "trees_full", "baseline_repeat"]
		radii = [80.0, 300.0, 1200.0]
	elif experiment == "E08":
		trials = ["horizon_off", "horizon_trees", "pan_horizon_off",
			"pan_horizon_trees_cold", "pan_horizon_trees_warm", "pan_horizon_off_repeat"]
		radii = [300.0]
	elif experiment == "E10":
		# The tree budget inside the laptop playable configuration, where the 20% FPS
		# allowance applies. Every trial shares the "low" runtime preset levers.
		trials = ["play_off", "play_trees_half", "play_trees_full",
			"play_trees_shadows", "play_off_repeat"]
		radii = [300.0]
	elif experiment == "E09":
		# Cost of near-level shadow casting against the same placements. The LOD crossfade
		# lever was removed with the crossfade itself; captures before that commit carry
		# their own copy of this file and still describe what they measured.
		trials = ["baseline", "trees_plain", "trees_shadows", "baseline_repeat"]
		radii = [80.0, 300.0, 1200.0]
	elif experiment == "E13":
		# Which direction the eye-level camera faces, paired off against on. E12 aimed
		# north across open ground; the cost of the scatter depends on how much forest
		# the view actually contains, so the worst case has to be searched for.
		trials = []
		for yaw in ["000", "090", "180", "270"]:
			trials.append("eye_horizon_yaw%s_off" % yaw)
			trials.append("eye_horizon_yaw%s_full" % yaw)
		# The same pair again at 1.5x render scale, which is 1920x1080 of shaded
		# pixels from a 1280x720 window. The tree cost is mostly fill, so the
		# 720p delta alone does not predict what a 1080p player sees.
		trials.append("eye_horizon_yaw180_off_hi")
		trials.append("eye_horizon_yaw180_full_hi")
		radii = [300.0]
	elif experiment == "E12":
		# Cost of the tree scatter from a camera just above the canopy that looks level
		# at the horizon. E08 measured a horizon view from 120 m up. Only from eye level
		# does one 510 m patch fill the view, which is the case the per-patch LOD
		# selection gets wrong, so this is the pose that prices the scatter honestly.
		trials = ["eye_horizon_off", "eye_horizon_plain", "eye_horizon_shadows",
			"eye_horizon_shadows_near", "eye_horizon_off_repeat"]
		radii = [300.0]
	elif experiment == "E11":
		# Cost of the day cycle across the day. Every trial is the same scene at a
		# different pinned hour inside one process, so build differences and thermal
		# drift between runs cannot be read as a difference between hours. The trials
		# run in clock order and the last one repeats the first to bracket that drift.
		trials = ["hour_12_00", "hour_17_00", "hour_19_20", "hour_19_50",
			"hour_20_30", "hour_22_30", "hour_12_00_repeat"]
		radii = [300.0]
	elif experiment == "E01":
		prepare_micro_shader()
		trials = ["baseline", "skip_invisible_micro", "baseline_repeat"]
	elif experiment == "E02":
		build_ablation_variants()
		trials = ["baseline"]
		trials.append_array(variant_shaders.keys())
		trials.append("baseline_repeat")
	elif experiment == "E06":
		# One row per configuration at a representative gameplay view, with a
		# screenshot each, so desktop and laptop captures can be tabulated together.
		trials = [
			"baseline",
			"shadow_cheap", "no_shadows",
			"far_3000", "fsr_half", "water_off",
			"low_preset", "low_preset_water", "terrain_ceiling",
			"simple_terrain",
			"baseline_repeat",
		]
		radii = [300.0]
	elif experiment == "E05":
		# Scene-configuration levers outside the terrain shader. Every one is a
		# runtime property change; no source or asset is modified.
		trials = [
			"baseline",
			"shadow_cheap", "no_shadows",
			"far_3000", "far_1500",
			"water_off", "fsr_half",
			"combined_playable", "combined_aggressive",
			"baseline_repeat",
		]
		radii = [300.0]
	elif experiment == "E04":
		# Terrain texture import comparison: the asset state changes between runs,
		# so each configuration is a separate process launch measuring the same view.
		trials = ["baseline", "baseline_repeat"]
		radii = [80.0, 300.0, 1200.0]
	elif experiment == "E03":
		# Route length is expressed in patch spans so it always crosses boundaries.
		var span: float = maxf(terrain.get_render_patch_span_m(), 1.0)
		pan_route_m = span * 24.0
		pan_origin = pivot - pan_axis * (pan_route_m * 0.5)
		radii = [300.0]
		trials = [
			"static_pre",
			"pan_short_cold", "pan_short_warm",
			"pan_long_cold", "pan_long_warm", "pan_long_fast",
			"static_post",
		]
	if OS.get_environment("METRUM_GPU_PROBE_VIEWS") == "distance":
		radii = [80.0, 300.0, 1200.0]
	elif OS.get_environment("METRUM_GPU_PROBE_VIEWS") == "close":
		radii = [80.0]
	elif OS.get_environment("METRUM_GPU_PROBE_VIEWS") == "transition":
		assert(micro_shader != null)
		radii = [10.0]
		trials = ["baseline", "skip_invisible_micro", "baseline_repeat", "zoom_baseline", "zoom_skip_invisible_micro", "zoom_baseline_repeat"]
	for radius in radii:
		camera_radius = radius
		camera.focus_on(pivot, camera_radius)
		if not await settle_view():
			quit(1)
			return
		for trial in trials:
			if experiment == "E07":
				vegetation.enabled = trial.begins_with("trees_")
				vegetation.density_fraction = 0.5 if trial == "trees_half" else 1.0
				vegetation.rebuild_from_simulation_state()
			elif experiment == "E08":
				vegetation.enabled = trial.contains("trees")
				vegetation.density_fraction = 1.0
			elif experiment == "E10":
				vegetation.enabled = trial.contains("trees")
				vegetation.density_fraction = 0.5 if trial.contains("half") else 1.0
				vegetation.cast_shadows = trial.contains("shadows")
				vegetation.rebuild_from_simulation_state()
			elif experiment == "E12" or experiment == "E13":
				vegetation.enabled = not trial.contains("_off")
				vegetation.density_fraction = 1.0
				# "full" is the shipped configuration.
				vegetation.cast_shadows = trial.contains("shadows") or trial.contains("full")
				vegetation.far_range_override_m = 2500.0 if trial.contains("near") else 0.0
				vegetation.rebuild_from_simulation_state()
			elif experiment == "E11":
				main.get_node("SceneLighting").pin_hour_of_day(probe_hour(trial))
			elif experiment == "E09":
				vegetation.enabled = trial.begins_with("trees_")
				vegetation.density_fraction = 1.0
				vegetation.cast_shadows = trial.contains("shadows")
				# Both levers change instance setup, so every trial rebuilds its patches.
				vegetation.rebuild_from_simulation_state()
			root.scaling_3d_scale = 0.5 if trial == "half_resolution" else 1.0
			main.get_node("DirectionalLight3D").shadow_enabled = trial != "no_shadows"
			main.get_node("Water").visible = trial != "no_water"
			set_simple_terrain(terrain, trial == "simple_terrain")
			apply_scene_levers(trial)
			active_material_count = 0
			if micro_shader != null:
				set_micro_shader(terrain, trial.contains("skip_invisible_micro"))
			elif not variant_shaders.is_empty():
				apply_terrain_shader(terrain, variant_shaders.get(trial))
			camera.focus_on(trial_start_pivot(trial), camera_radius)
			apply_far_lever(trial)
			apply_horizon_view(trial)
			if experiment in ["E07", "E08", "E09", "E10", "E12", "E13"] and not await settle_view():
				quit(1)
				return
			await create_timer(4.0).timeout
			# Panning trials must start from a settled view so the capture measures
			# traversal work rather than leftover approach work.
			if trial.begins_with("pan_") and not await settle_view():
				quit(1)
				return
			await capture(trial)
	var metadata := {
		"godot": Engine.get_version_info(), "gpu": RenderingServer.get_video_adapter_name(),
		"cpu": OS.get_processor_name(), "world": world, "world_sha256": FileAccess.get_sha256(world),
		"binary_sha256": FileAccess.get_sha256("res://bin/libmetrum_rise.so"),
		"probe_sha256": FileAccess.get_sha256(get_script().resource_path),
		"experiment": OS.get_environment("METRUM_GPU_PROBE_EXPERIMENT"),
		"views": OS.get_environment("METRUM_GPU_PROBE_VIEWS"),
		"terrain_shader_sha256": FileAccess.get_sha256("res://assets/materials/terrain.gdshader"),
		"resolution": [1280, 720], "vsync": DisplayServer.window_get_vsync_mode(),
		"camera_pivot": [pivot.x, pivot.y, pivot.z], "camera_radii": radii,
		"capture_seconds": capture_seconds,
		"terrain_patch_span_m": terrain.get_render_patch_span_m(),
		"pan_route_m": pan_route_m,
		"pan_origin": [pan_origin.x, pan_origin.y, pan_origin.z],
		"note": "GPU viewport timings are asynchronous observations, not presentation latency. Trials are sequential; each includes 4 seconds warmup and a per-trial capture window. The legacy pan trial traverses 400m in 8 seconds; E03 pan trials use pan_plan().",
		"trials": results,
	}
	var file := FileAccess.open(output_dir.path_join("results.json"), FileAccess.WRITE)
	file.store_string(JSON.stringify(metadata, "\t"))
	file.close()
	print("GPU_PROBE_OUTPUT " + output_dir)
	quit()

func settle_view() -> bool:
	var started := Time.get_ticks_msec()
	var stable := 0
	while Time.get_ticks_msec() - started < 120000:
		await process_frame
		if not terrain.has_pending_render_work(false) and not main.get_node("Water").has_pending_render_work(false) and not vegetation.has_pending_work():
			stable += 1
		else:
			stable = 0
		if stable >= 30:
			return true
	push_error("Camera view did not settle")
	return false

func prepare_micro_shader() -> void:
	var source := FileAccess.get_file_as_string("res://assets/materials/terrain.gdshader")
	var begin := source.find("vec3 apply_grass_detail(")
	var end := source.find("\nfloat sample_local_relief", begin)
	assert(begin >= 0 and end > begin)
	var original := source.substr(begin, end - begin)
	var body := original
	var samples := "\tvec3 micro_grass = sample_grass_albedo_cross(world_pos.xz, terrain_grass_detail_scale);\n\tfloat height_detail = sample_grass_height_cross(world_pos.xz, terrain_grass_detail_scale);\n"
	assert(body.contains(samples))
	body = body.replace(samples, "")
	var marker := "\tcolor = mix(\n\t\tcolor,\n\t\tgrass_material_layer(color, micro_grass"
	assert(body.contains(marker))
	# Existing fade reaches exactly zero; only then bypass all micro contribution.
	body = body.replace(marker, "\tif (micro_visibility <= 0.0) { return color; }\n" + samples + marker)
	micro_shader = Shader.new()
	micro_shader.code = source.replace(original, body)
	var file := FileAccess.open(output_dir.path_join("E01-terrain.gdshader"), FileAccess.WRITE)
	file.store_string(micro_shader.code)
	file.close()

## Builds one shader per isolated terrain feature by forcing an early return at the
## top of the owning function. Variants are diagnostic: they change appearance and
## exist only to attribute opaque-pass cost, never to be adopted as-is.
func build_ablation_variants() -> void:
	var source := FileAccess.get_file_as_string("res://assets/materials/terrain.gdshader")
	# Each entry maps a variant name to the edits applied to the original source.
	# An edit is [exact_source_fragment, replacement]; every fragment must be unique.
	var cliff := ["vec3 cliff_masks(vec2 uv) {", "vec3 cliff_masks(vec2 uv) {\n\tif (true) { return vec3(0.0); }"]
	var relief := ["float sample_local_relief(vec2 uv) {", "float sample_local_relief(vec2 uv) {\n\tif (true) { return 0.0; }"]
	var shore := ["float shoreline_mask(vec2 uv) {", "float shoreline_mask(vec2 uv) {\n\tif (true) { return 0.0; }"]
	var grass := [
		"vec3 apply_grass_detail(vec3 color, vec3 world_pos, float grass_mask) {",
		"vec3 apply_grass_detail(vec3 color, vec3 world_pos, float grass_mask) {\n\tif (true) { return color; }",
	]
	var land := [
		"\tfloat broad = warped_noise(world_xz, 0.0010, vec2(5.0, 71.0));",
		"\tif (true) { return color; }\n\tfloat broad = warped_noise(world_xz, 0.0010, vec2(5.0, 71.0));",
	]
	var contour := [
		"float contour_mask(float world_elevation, float interval_m, float thickness) {",
		"float contour_mask(float world_elevation, float interval_m, float thickness) {\n\tif (true) { return 0.0; }",
	]
	var macro := ["float macro_variation(vec2 world_xz) {", "float macro_variation(vec2 world_xz) {\n\tif (true) { return 0.5; }"]
	var stochastic := [
		"vec3 sample_grass_albedo_stochastic(vec2 uv, float salt) {",
		"vec3 sample_grass_albedo_stochastic(vec2 uv, float salt) {\n\tif (true) { return texture(terrain_grass_albedo, uv).rgb; }",
	]
	var cull := ["render_mode cull_disabled;", "render_mode cull_back;"]
	var specs := {
		"A_no_cliff_masks": [cliff],
		"B_no_local_relief": [relief],
		"C_no_shoreline": [shore],
		"D_no_grass_detail": [grass],
		"E_no_land_variation": [land],
		"F_no_contours": [contour],
		"G_no_macro_variation": [macro],
		"H_no_stochastic_tiling": [stochastic],
		"I_cull_back": [cull],
		"J_terrain_only_lit": [cliff, relief, shore, grass, land, contour, macro],
	}
	for name_variant in specs:
		var name := str(name_variant)
		var code := source
		for edit_variant in specs[name_variant]:
			var edit: Array = edit_variant
			var find: String = edit[0]
			assert(code.count(find) == 1)
			code = code.replace(find, edit[1])
		var shader := Shader.new()
		shader.code = code
		variant_shaders[name] = shader
		var file := FileAccess.open(output_dir.path_join("E02-%s.gdshader" % name), FileAccess.WRITE)
		file.store_string(code)
		file.close()

## Applies `shader` to every resident terrain patch material, or restores the
## original terrain shader when `shader` is null.
func apply_terrain_shader(node: Node, shader: Shader) -> void:
	if node is MeshInstance3D and node.material_override is ShaderMaterial:
		var material: ShaderMaterial = node.material_override
		if material.shader != null and material.shader.resource_path == "res://assets/materials/terrain.gdshader":
			original_shaders[material] = material.shader
		if original_shaders.has(material):
			material.shader = original_shaders[material] if shader == null else shader
			active_material_count += 1
	for child in node.get_children():
		apply_terrain_shader(child, shader)

## Camera pivot the trial starts from, before its warmup and settle.
func trial_start_pivot(trial: String) -> Vector3:
	var plan := pan_plan(trial)
	if plan.has("from"):
		return plan["from"] as Vector3
	if trial == "static_post":
		return pan_origin + pan_axis * pan_route_m
	if trial == "static_pre":
		return pan_origin
	return pivot

## Lateral traversal definition for E03. Short routes stay inside the resident
## patch ring; long routes deliberately outrun it so streaming cost is separable.
func pan_plan(trial: String) -> Dictionary:
	var span: float = maxf(terrain.get_render_patch_span_m(), 1.0)
	if trial.begins_with("pan_horizon_"):
		return {"from": pivot - Vector3(span * 2, 0, 0), "to": pivot + Vector3(span * 2, 0, 0), "seconds": 20.0}
	var short_route := span * 4.0
	var short_from := pivot - pan_axis * (short_route * 0.5)
	var long_to := pan_origin + pan_axis * pan_route_m
	match trial:
		"pan_short_cold":
			return {"from": short_from, "to": short_from + pan_axis * short_route, "seconds": 12.0}
		"pan_short_warm":
			return {"from": short_from + pan_axis * short_route, "to": short_from, "seconds": 12.0}
		"pan_long_cold":
			return {"from": pan_origin, "to": long_to, "seconds": 24.0}
		"pan_long_warm":
			return {"from": long_to, "to": pan_origin, "seconds": 24.0}
		"pan_long_fast":
			return {"from": pan_origin, "to": long_to, "seconds": 8.0}
	return {}

## E08 keeps the camera at fixed altitude and low pitch, looking north toward the horizon.
## Override focus_on's automatic pose identically for tree-on and tree-off captures.
func apply_horizon_view(trial: String) -> void:
	if not trial.contains("horizon"):
		return
	# E12 sits just above the canopy and looks level, so a single scatter patch spans
	# the screen from the camera to the horizon instead of covering a thin strip.
	if trial.begins_with("eye_"):
		camera.position.y = pivot.y + 30.0
		camera.rotation = Vector3(-0.04, deg_to_rad(probe_yaw(trial)), 0.0)
		return
	camera.position.y = pivot.y + 120.0
	camera.rotation = Vector3(-0.12, 0.0, 0.0)

## Applies the E05 scene-configuration levers for `trial`, restoring the authored
## SceneLighting/viewport configuration for every trial that does not name them.
## These are runtime property writes only; no source file or asset is touched.
func apply_scene_levers(trial: String) -> void:
	var config := lever_config(trial)
	var cheap_shadows: bool = config.get("shadows", "") == "cheap"
	var sun: DirectionalLight3D = main.get_node("DirectionalLight3D")
	if cheap_shadows:
		# Angular distance drives Godot's variable-penumbra path; zero makes the
		# directional shadow a plain PCF lookup. Two splits instead of four, and
		# no split blending, halve the cascade sampling again.
		sun.light_angular_distance = 0.0
		sun.shadow_blur = 0.0
		sun.set("directional_shadow_mode", 1)
		sun.set("directional_shadow_blend_splits", false)
		RenderingServer.directional_soft_shadow_filter_set_quality(
			RenderingServer.SHADOW_QUALITY_HARD
		)
	else:
		sun.light_angular_distance = SceneLightingConfig.SUN_ANGULAR_DISTANCE_DEG
		sun.shadow_blur = SceneLightingConfig.SHADOW_BLUR
		sun.set("directional_shadow_mode", 2)
		sun.set("directional_shadow_blend_splits", true)
		RenderingServer.directional_soft_shadow_filter_set_quality(
			RenderingServer.SHADOW_QUALITY_SOFT_MEDIUM
		)
	if config.has("water"):
		main.get_node("Water").visible = config["water"]
	if config.has("fsr"):
		root.scaling_3d_mode = Viewport.SCALING_3D_MODE_FSR2
		root.scaling_3d_scale = config["fsr"]
	elif trial != "half_resolution":
		root.scaling_3d_mode = Viewport.SCALING_3D_MODE_BILINEAR
		root.scaling_3d_scale = 1.0
	if config.has("render_scale"):
		root.scaling_3d_mode = Viewport.SCALING_3D_MODE_BILINEAR
		root.scaling_3d_scale = config["render_scale"]
	if config.get("terrain", "") == "simple":
		set_simple_terrain(terrain, true)

## Overrides the camera far plane, which CameraNode otherwise derives as
## distance * 4 + margin (13 km at radius 1200). Must run after focus_on.
func apply_far_lever(trial: String) -> void:
	var config := lever_config(trial)
	if config.has("far"):
		camera.far = config["far"]

## Single source of truth for what each lever trial changes. Keys: shadows
## ("cheap"), far (metres), fsr (viewport scale), render_scale (bilinear viewport
## scale), water (bool), terrain ("simple").
## Trials absent here run the authored configuration.
func lever_config(trial: String) -> Dictionary:
	match trial:
		"shadow_cheap":
			return {"shadows": "cheap"}
		"far_3000":
			return {"far": 3000.0}
		"far_1500":
			return {"far": 1500.0}
		"water_off":
			return {"water": false}
		"fsr_half":
			return {"fsr": 0.5}
		"eye_horizon_yaw180_off_hi", "eye_horizon_yaw180_full_hi":
			return {"render_scale": 1.5}
		"combined_playable", "low_preset":
			return {"shadows": "cheap", "far": 3000.0, "fsr": 0.5}
		"combined_aggressive":
			return {"shadows": "cheap", "far": 1500.0, "fsr": 0.5}
		"low_preset_water":
			return {"shadows": "cheap", "far": 3000.0, "fsr": 0.5, "water": false}
		"terrain_ceiling":
			return {"shadows": "cheap", "far": 3000.0, "fsr": 0.5, "terrain": "simple"}
	if trial.begins_with("play_"):
		# The "low" runtime preset: cheap shadows, a 3 km far plane and FSR2 at 0.67.
		return {"shadows": "cheap", "far": 3000.0, "fsr": 0.67}
	return {}

func set_micro_shader(node: Node, enabled: bool) -> void:
	if node is MeshInstance3D and node.material_override is ShaderMaterial:
		var material: ShaderMaterial = node.material_override
		if material.shader != null and material.shader.resource_path == "res://assets/materials/terrain.gdshader":
			original_shaders[material] = material.shader
		if original_shaders.has(material):
			material.shader = micro_shader if enabled else original_shaders[material]
			active_material_count += 1
	for child in node.get_children():
		set_micro_shader(child, enabled)

func set_simple_terrain(node: Node, enabled: bool) -> void:
	if node is MeshInstance3D:
		if enabled:
			# Idempotent: a second enable must not record the simple material as
			# the thing to restore, which would leak it into later trials.
			if not overrides.has(node):
				overrides[node] = node.material_override
			node.material_override = simple_material
		elif overrides.has(node):
			node.material_override = overrides[node]
	for child in node.get_children():
		set_simple_terrain(child, enabled)

func capture(trial: String) -> void:
	var label := "%s-r%d" % [trial, camera_radius]
	print("GPU_PROBE_BEGIN " + label)
	var frames: Array = []
	var gpu: Array = []
	var cpu: Array = []
	var raw: Array = []
	var plan := pan_plan(trial)
	var window_us := int(float(plan.get("seconds", capture_seconds)) * 1000000.0)
	var pending_frames := 0
	var vegetation_pending_frames := 0
	var vegetation_before: Dictionary = vegetation.metrics()
	vegetation.generation_ms_max = 0.0
	var start := Time.get_ticks_usec()
	var previous := start
	while Time.get_ticks_usec() - start < window_us:
		var progress := clampf(float(Time.get_ticks_usec() - start) / float(window_us), 0.0, 1.0)
		if trial.begins_with("zoom_"):
			camera.focus_on(pivot, lerpf(10.0, 1200.0, progress))
			# Apply the same traversal overhead in controls; new resident patches must
			# receive the selected shader too. This is measurement tooling only.
			active_material_count = 0
			set_micro_shader(terrain, trial.contains("skip_invisible_micro"))
		if trial == "pan":
			camera.focus_on(pivot + Vector3(progress * 400.0, 0, 0), camera_radius)
		elif plan.has("from"):
			camera.focus_on((plan["from"] as Vector3).lerp(plan["to"] as Vector3, progress), camera_radius)
			apply_horizon_view(trial)
			if not variant_shaders.is_empty():
				active_material_count = 0
				apply_terrain_shader(terrain, variant_shaders.get(trial))
		await process_frame
		var now := Time.get_ticks_usec()
		var frame_ms := float(now - previous) / 1000.0
		previous = now
		var gpu_ms := RenderingServer.viewport_get_measured_render_time_gpu(root.get_viewport_rid())
		var cpu_ms := RenderingServer.viewport_get_measured_render_time_cpu(root.get_viewport_rid())
		# Cheap boolean only: per-frame dictionary queries would distort frame tails.
		var vegetation_pending: bool = vegetation.has_pending_work()
		var pending: bool = terrain.has_pending_render_work(false) or vegetation_pending
		if vegetation_pending:
			vegetation_pending_frames += 1
		if pending:
			pending_frames += 1
		frames.append(frame_ms)
		gpu.append(gpu_ms)
		cpu.append(cpu_ms)
		raw.append([frame_ms, gpu_ms, cpu_ms, 1 if pending else 0])
	if trial.begins_with("zoom_"):
		# Pin the final screenshot pose independently of sampled frame cadence.
		camera.focus_on(pivot, 1200.0)
		await settle_view()
		await create_timer(1.0).timeout
	await RenderingServer.frame_post_draw
	root.get_texture().get_image().save_png(output_dir.path_join(label + ".png"))
	# Recorded for every experiment: the palette is elevation-keyed, so the hour alone
	# does not identify the lighting a trial actually ran under.
	var sun_state: DayCycleConfig.Sample = main.get_node("SceneLighting").current_sample()
	var entry := {"trial": trial, "camera_radius": camera_radius, "terrain_material_instances": active_material_count, "frame_ms": summarize(frames), "gpu_ms": summarize(gpu), "render_cpu_ms": summarize(cpu), "samples_frame_gpu_cpu_pending": raw,
		"vegetation": vegetation.metrics(),
		"vegetation_at_start": vegetation_before,
		"vegetation_pending_frames": vegetation_pending_frames,
		"capture_seconds": float(plan.get("seconds", capture_seconds)),
		"pan_seconds": plan.get("seconds", 0.0),
		"pan_distance_m": 0.0 if not plan.has("from") else (plan["to"] as Vector3).distance_to(plan["from"] as Vector3),
		"pending_render_frames": pending_frames,
		"pending_render_counts_at_end": terrain.get_pending_render_work_counts(),
		"camera_far_m": camera.far,
		"camera_position": [camera.position.x, camera.position.y, camera.position.z],
		"camera_rotation": [camera.rotation.x, camera.rotation.y, camera.rotation.z],
		"scaling_3d_mode": root.scaling_3d_mode,
		"scaling_3d_scale": root.scaling_3d_scale,
		"pinned_hour": probe_hour(trial),
		"sun_elevation_deg": sun_state.sun_elevation_deg,
		"key_energy": sun_state.key_energy,
		"key_is_moonlit": sun_state.is_moonlit,
		"sun_angular_distance_deg": (main.get_node("DirectionalLight3D") as DirectionalLight3D).light_angular_distance,
		"shadow_enabled": (main.get_node("DirectionalLight3D") as DirectionalLight3D).shadow_enabled,
		"resident_patches_at_end": terrain.get_resident_patch_keys().size(),
		"draw_calls": RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_TOTAL_DRAW_CALLS_IN_FRAME),
		"primitives": RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_TOTAL_PRIMITIVES_IN_FRAME),
		"video_memory_bytes": RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_VIDEO_MEM_USED)}
	results.append(entry)
	print("GPU_PROBE_RESULT %s frame=%s gpu=%s pending_frames=%d/%d resident=%d" % [
		trial, entry.frame_ms, entry.gpu_ms, pending_frames, frames.size(), entry.resident_patches_at_end,
	])

## Compass yaw in degrees from a trial that names one as "yaw090". Trials without a
## yaw segment face north, which is what every earlier horizon capture used.
func probe_yaw(trial: String) -> float:
	for part in trial.split("_"):
		if part.begins_with("yaw"):
			return float(part.substr(3).to_int())
	return 0.0

# "hour_19_20" is 19:20. A trailing "_repeat" is ignored so a bracket trial can name the
# same hour twice. Returns -1 for any trial that does not pin an hour.
func probe_hour(trial: String) -> float:
	var parts := trial.split("_")
	if parts.size() < 3 or parts[0] != "hour":
		return -1.0
	return float(parts[1].to_int()) + float(parts[2].to_int()) / 60.0

func summarize(values: Array) -> Dictionary:
	var sorted := values.duplicate()
	sorted.sort()
	var total := 0.0
	for value in values:
		total += value
	return {"count": values.size(), "mean": total / values.size(), "p50": sorted[int((sorted.size() - 1) * 0.5)], "p95": sorted[int((sorted.size() - 1) * 0.95)], "p99": sorted[int((sorted.size() - 1) * 0.99)], "p999": sorted[int((sorted.size() - 1) * 0.999)], "max": sorted.back()}
