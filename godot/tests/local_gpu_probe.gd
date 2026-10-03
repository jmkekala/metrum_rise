# SPDX-License-Identifier: GPL-2.0-only

## Local, opt-in rendered GPU probe; writes measurements without saving gameplay state.
## Run from repo root: godot --path godot --windowed --resolution 1280x720 --gpu-profile --script res://tests/local_gpu_probe.gd
## METRUM_GPU_PROBE_OUTPUT selects a fresh output directory; METRUM_GPU_PROBE_WORLD selects a world SQLite.
## METRUM_GPU_PROBE_EXPERIMENT selects E01 (micro grass), E02 (shader cost attribution),
## E03 (long lateral panning), E04 (terrain texture import comparison), E11 (day cycle
## hours), E12 (tree scatter from an eye-level horizon camera), E14 (a forest the brush
## painted, which is much denser than the one the generator makes), E15 (the vegetation
## patch grid and the near canopy band swept together over that same painted forest), E17
## (the near canopy band swept at the generator's own density instead) or E16
## (what is left in the frame once E15 has picked the grid: shadows, band and far range).
## E18 prices the near canopy's foliage card area in one painted stand, seen from inside it.
## E19 prices shadow work at that same pose: what the trees cost as casters and what
## shortening the directional shadow range buys.
## E20 prices the near canopy as drawn geometry at that pose: what the branched tree costs
## across its band in a painted stand, and what a finer near grid does there.
## METRUM_GPU_PROBE_VIEWS selects the camera radius sweep.
extends SceneTree

# Matches the renderers' idiom. A bare SceneLighting class_name reference needs
# Godot's global class cache, which a freshly cloned checkout has not built yet.
const SceneLightingConfig := preload("res://scripts/core/scene_lighting.gd")
const TreeSpecies := preload("res://scripts/renderers/tree_species.gd")
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
# Pivot the current pose is built around: `pivot` while the camera stands still, and the
# lerped route position during a pan. A pose that reads `pivot` directly snaps back to the
# centre of the route on every frame, which makes a panning trial measure nothing.
var view_pivot := Vector3.ZERO
# Slower machines need a longer window to collect a comparable number of samples.
# Applies to stationary trials only; pan trials carry their own durations.
var capture_seconds := 8.0
var vegetation: Node
# E14 brush fixture. Preset 8 is the managed stand: 0.85 of a 4 m lattice, about 531
# stems per hectare, inside the real Finnish 400-700 for a managed stand.
const PAINT_PRESET := 8
var painted_plants := 0
var painted_extent_m := 1600.0
var painted_ms := 0.0
var card_catalogues: Dictionary = {}
var card_probe_scripts: Array[GDScript] = []

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
	elif experiment == "E24":
		# Impostors against the lathe, over the painted stand. Run once per build: the lathe
		# build is the baseline and the impostor build the candidate, both at their shipped
		# bands (800 m and 250 m). The aerial pose is the view the lathe failed in: from
		# altitude the whole stand past the handover was one flat colour.
		paint_dense_forest()
		trials = ["nearsweep_a", "aerial_a", "nearsweep_b", "aerial_b", "nearsweep_c", "aerial_c"]
		radii = [30.0]
	elif experiment == "E23":
		# What the reduced near level is actually worth. It keeps every foliage card and cuts
		# only interior wood, and E18 found the near cost follows card area rather than plane
		# count, so the saving is not obvious and has to be priced before the handover is
		# tuned. `d` is the handover distance in metres: 800 draws the whole near band from
		# the branched tree, which is what shipped, and 1 draws all of it from the reduced
		# level, which is the ceiling this change can reach.
		paint_dense_forest()
		trials = ["nearsweep_near800_f4_d800", "nearsweep_near800_f4_d180",
			"nearsweep_near800_f4_d800_b", "nearsweep_near800_f4_d45",
			"nearsweep_near800_f4_d800_c", "nearsweep_near800_f4_d1",
			"nearsweep_near800_f4_d800_d"]
		radii = [30.0]
	elif experiment == "E22":
		# Whether the finer grid's hitch is the grid or the budget it derives. E21 found
		# subdivision 8 worth 10.2 ms of GPU time while panning and 4 to 5 ms WORSE at frame
		# p99, with the frame running 8 to 10 ms past its own GPU time at that percentile.
		# The per-frame upload budget is the subdivision squared, so a finer grid also lets
		# one frame build four times as many sub-patches. That is a budget choice, not a
		# property of the grid. These trials hold the grid at 8 and cap the budget instead.
		paint_dense_forest()
		trials = ["pan_nearsweep_near800_f4", "pan_nearsweep_near800_f8_b16",
			"pan_nearsweep_near800_f4_b", "pan_nearsweep_near800_f8_b8",
			"pan_nearsweep_near800_f4_c", "pan_nearsweep_near800_f8"]
		radii = [30.0]
	elif experiment == "E21":
		# Whether the finer near grid survives motion. E20 priced subdivision 8 at 13.73 ms
		# cheaper from a standing camera, but a standing camera never crosses a patch
		# boundary, and patch churn while moving is the cost the two-tier grid was built to
		# contain. Subdividing raises the resident instance count by the square, so every
		# crossing builds four times the instances it used to. The route runs 800 m through
		# the middle of the 1600 m painted square, so the whole traversal stays inside the
		# stand. Frame p99 and the vegetation generation peak answer the question; the GPU
		# median only confirms the E20 saving reproduces in this process.
		paint_dense_forest()
		trials = ["nearsweep_near800_f4", "nearsweep_near800_f8",
			"pan_nearsweep_near800_f4", "pan_nearsweep_near800_f8",
			"pan_nearsweep_near800_f4_b", "pan_nearsweep_near800_f8_b",
			"pan_nearsweep_near800_f4_c"]
		radii = [30.0]
	elif experiment == "E20":
		# What the near canopy costs to DRAW in a painted stand, which is the quantity a level
		# between the branched tree and the lathe would compete for. E17 swept the same band at
		# the generator's own density and found 200 m to 800 m worth 0.94 ms; the painted stand
		# is about seventeen times denser, and nobody has priced it there. The subdivision trial
		# rides along because the grid is a free override and the question is open.
		paint_dense_forest()
		trials = ["nearsweep_near800_f4", "nearsweep_near400_f4", "nearsweep_near800_f4_b",
			"nearsweep_near200_f4", "nearsweep_near800_f4_c", "nearsweep_near800_f8",
			"nearsweep_near800_f4_d"]
		radii = [30.0]
	elif experiment == "E19":
		# Restoring the near band to 800 m grew the shadow casting set from the 200 m disc it
		# had been tuned against to the full SHADOW_MAX_DISTANCE_M disc, which is 4.4 times the
		# ground. Shadows re-render the same alpha scissored canopy into four cascades, so the
		# cost follows covered area exactly as the colour pass does. Trials share one process
		# and one pose; the sun's split distances are fractions of its range and rescale with it.
		paint_dense_forest()
		# This pose runs near 90 ms, which heats the card enough that trial order alone moves
		# the result by several percent. The authored range is repeated between every
		# measurement so the drift can be fitted and removed rather than assumed away.
		trials = ["shadowsweep_far420", "shadowsweep_veg_off", "shadowsweep_far420_b",
			"shadowsweep_far250", "shadowsweep_far420_c", "shadowsweep_far150",
			"shadowsweep_far420_d"]
		radii = [30.0]
	elif experiment == "E18":
		# Card fill, measured from inside the stand rather than above it. The cards are
		# alpha scissored into the opaque pass, so their cost follows the area they cover,
		# not the number of planes stacked behind one another.
		paint_dense_forest()
		prepare_card_trials()
		trials = ["cards_original", "cards_trimmed", "cards_original_repeat"]
		radii = [30.0]
	elif experiment == "E17":
		# The near band swept at the density the generator makes, not the one the brush
		# paints. Every earlier band number came from a painted stand of about 531 stems/ha,
		# which is 17 times the shipped 30.47 and is the worst case by construction. The
		# band has to be chosen for the forest the player normally flies over, so it is
		# priced here too. No paint call: this is the generator's own scatter.
		trials = ["gen_off", "gen_f4_near200_full", "gen_f4_near400_full",
			"gen_f4_near600_full", "gen_f4_near800_full", "gen_f4_near200_full_repeat"]
		radii = [300.0]
	elif experiment == "E16":
		# Attribution at the grid E15 selected. Every trial holds the subdivision at 4 and
		# changes one lever, so each difference is that lever. "plain" drops shadow casting,
		# which measured as more than half the tree cost on the coarse grid. far2500 clamps
		# the canopy range, which bought nothing on the coarse grid and is re-asked here.
		paint_dense_forest()
		trials = ["abl_off", "abl_f4_near200_plain", "abl_f4_near200_full",
			"abl_f4_near100_full", "abl_f4_near400_full",
			"abl_f4_near200_far2500_full", "abl_f4_near200_plain_repeat"]
		radii = [300.0]
	elif experiment == "E15":
		# What the vegetation patch grid costs and what it buys. A patch is one instance, so
		# the near band cannot be narrower than the patch diagonal; the grid was the 510 m
		# terrain grid, which is what held the near cards out to 800 m. Each trial names a
		# subdivision and a near range. f1_near800 is the shipped geometry, f4_near800
		# isolates what subdividing costs on its own, and f4_near200 is the pair of them.
		paint_dense_forest()
		trials = ["grid_off", "grid_f1_near800_full", "grid_f4_near800_full",
			"grid_f2_near400_full", "grid_f4_near200_full", "grid_f8_near200_full",
			"grid_f1_near800_full_repeat"]
		radii = [300.0]
	elif experiment == "E14":
		# Cost of a forest the brush painted rather than the one the generator makes.
		# The brush plants on a fixed 4 m lattice, so its managed-stand preset is about
		# 531 stems/ha where the shipped generator is 30.47. That is the case the player
		# reports as unplayable, and no generator density can reach it: the generator
		# ceiling is 121.875 stems/ha. The view is the E12 pose, so the two are comparable.
		paint_dense_forest()
		trials = ["dense_eye_horizon_off", "dense_eye_horizon_plain",
			"dense_eye_horizon_full", "dense_eye_horizon_near",
			"dense_eye_horizon_off_repeat"]
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
			elif experiment == "E24":
				# The shipped configuration of whichever build runs this.
				vegetation.enabled = true
				vegetation.density_fraction = 1.0
				vegetation.cast_shadows = true
				vegetation.patch_subdivision_override = 0
				vegetation.near_range_override_m = 0.0
				vegetation.rebuild_from_simulation_state()
			elif experiment in ["E20", "E21", "E22", "E23"]:
				vegetation.enabled = true
				vegetation.density_fraction = 1.0
				# Shipped shadow configuration throughout, so the only thing that moves is the
				# band the branched tree is drawn in and the grid it is carried on.
				vegetation.cast_shadows = true
				vegetation.patch_subdivision_override = trial_grid_value(trial, "f")
				vegetation.near_range_override_m = float(trial_grid_value(trial, "near"))
				vegetation.upload_budget_override = trial_grid_value(trial, "b")
				# Zero would mean "draw everything from the reduced level", so absence is -1.
				var detail := trial_grid_value(trial, "d")
				vegetation.near_detail_override_m = float(detail) if detail > 0 else -1.0
				vegetation.rebuild_from_simulation_state()
			elif experiment == "E19":
				vegetation.enabled = true
				vegetation.density_fraction = 1.0
				# The authored band and grid: this prices the shipped configuration.
				vegetation.patch_subdivision_override = 0
				vegetation.near_range_override_m = 0.0
				vegetation.cast_shadows = not trial.contains("veg_off")
				var sun: DirectionalLight3D = main.get_node("DirectionalLight3D")
				var shadow_far := trial_grid_value(trial, "far")
				sun.set("directional_shadow_max_distance", float(shadow_far)
					if shadow_far > 0 else SceneLightingConfig.shadow_max_distance_m())
				vegetation.rebuild_from_simulation_state()
			elif experiment == "E18":
				vegetation.enabled = true
				vegetation.density_fraction = 1.0
				vegetation.cast_shadows = true
				vegetation.patch_subdivision_override = 4
				vegetation.near_range_override_m = 200.0
				vegetation.meshes = card_catalogues[trial.trim_suffix("_repeat")]
				vegetation.rebuild_from_simulation_state()
			elif experiment in ["E15", "E16", "E17"]:
				vegetation.enabled = not trial.contains("_off")
				vegetation.density_fraction = 1.0
				vegetation.cast_shadows = trial.contains("full")
				vegetation.patch_subdivision_override = trial_grid_value(trial, "f")
				vegetation.near_range_override_m = float(trial_grid_value(trial, "near"))
				vegetation.far_range_override_m = float(trial_grid_value(trial, "far"))
				vegetation.rebuild_from_simulation_state()
			elif experiment in ["E12", "E13", "E14"]:
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
			view_pivot = trial_start_pivot(trial)
			camera.focus_on(view_pivot, camera_radius)
			apply_far_lever(trial)
			apply_horizon_view(trial)
			if experiment in ["E07", "E08", "E09", "E10", "E12", "E13", "E14", "E15", "E16", "E18", "E19"] and not await settle_view():
				quit(1)
				return
			# Terrain streaming never quiets after a dense paint, so E24 waits on the one system
			# it compares: both builds must have uploaded the pose's whole canopy.
			if experiment == "E24" and not await settle_vegetation():
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
		"card_shader_sha256": FileAccess.get_sha256("res://scripts/shaders/vegetation_wind_cards.gdshader"),
		"tree_species_sha256": FileAccess.get_sha256("res://scripts/renderers/tree_species.gd"),
		"depth_prepass_enabled": ProjectSettings.get_setting("rendering/driver/depth_prepass/enable"),
		"probe_sha256": FileAccess.get_sha256(get_script().resource_path),
		"experiment": OS.get_environment("METRUM_GPU_PROBE_EXPERIMENT"),
		"views": OS.get_environment("METRUM_GPU_PROBE_VIEWS"),
		"terrain_shader_sha256": FileAccess.get_sha256("res://assets/materials/terrain.gdshader"),
		"resolution": [1280, 720], "vsync": DisplayServer.window_get_vsync_mode(),
		"camera_pivot": [pivot.x, pivot.y, pivot.z], "camera_radii": radii,
		"capture_seconds": capture_seconds,
		"terrain_patch_span_m": terrain.get_render_patch_span_m(),
		"painted_plants": painted_plants,
		"painted_extent_m": painted_extent_m,
		"painted_preset": PAINT_PRESET,
		"paint_ms": painted_ms,
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

func settle_vegetation() -> bool:
	var started := Time.get_ticks_msec()
	var stable := 0
	while Time.get_ticks_msec() - started < 120000:
		await process_frame
		stable = 0 if vegetation.has_pending_work() else stable + 1
		if stable >= 30:
			return true
	push_error("Vegetation did not settle")
	return false

func settle_view() -> bool:
	var started := Time.get_ticks_msec()
	var stable := 0
	# Five minutes, not two. Terrain streaming after a dense paint sits close to the old
	# budget and fails it about as often as it clears it, and a settle that times out costs a
	# whole run. The loop still leaves as soon as the scene is quiet, so a healthy run is
	# no slower for this.
	while Time.get_ticks_msec() - started < 300000:
		await process_frame
		if not terrain.has_pending_render_work(false) and not main.get_node("Water").has_pending_render_work(false) and not vegetation.has_pending_work():
			stable += 1
		else:
			stable = 0
		if stable >= 30:
			return true
	# Which of the three is still working, so a settle failure names its cause instead of
	# sending the reader back to the renderer to guess.
	push_error("Camera view did not settle: terrain=%s water=%s vegetation=%s\n  terrain queues %s\n  blocked %s\n  vegetation %s" % [
		str(terrain.has_pending_render_work(false)),
		str(main.get_node("Water").has_pending_render_work(false)),
		str(vegetation.has_pending_work()),
		JSON.stringify(terrain.get_pending_render_work_counts()),
		JSON.stringify(terrain.get_blocked_dirty_patch_failures()),
		JSON.stringify(vegetation.metrics())])
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
	var relief := [
		"float local_relief = terrain_mask.r * terrain_mask.r * TERRAIN_MASK_RELIEF_MAX_M;",
		"float local_relief = 0.0;",
	]
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
	if trial.begins_with("pan_nearsweep_"):
		# Half the painted extent, so both ends and everything between them carry painted
		# stems. Twenty seconds over 800 m is 40 m/s, which is a fast drag rather than a
		# teleport: the grid has to keep up rather than being handed a settled view.
		return {"from": pivot - pan_axis * 400.0, "to": pivot + pan_axis * 400.0, "seconds": 20.0}
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
## Paints a square of brushed managed stand centred on the probe pivot, and records what
## it planted. The brush is the only way to reach this density: it writes a fixed 4 m
## planting lattice, where the generator's own ceiling is an 8 m canopy cell.
##
## Stamps are discs, so they are spaced below their own radius and overlap. An overlap
## plants nothing twice: a lattice point that already carries a plant is not clear, so the
## painted population is a property of the square and not of how it was covered.
func paint_dense_forest() -> void:
	var simulation: Node = main.get_node("SimulationNode")
	var start := Time.get_ticks_msec()
	var radius := 200.0
	var step := 140.0
	var half := painted_extent_m * 0.5
	var steps := int(half / step)
	for column in range(-steps, steps + 1):
		for row in range(-steps, steps + 1):
			var at := Vector2(pivot.x + column * step, pivot.z + row * step)
			# Each stamp adds a thin share, so repeat until the stand is at the preset density.
			var added := 1
			while added > 0:
				added = int(simulation.paint_vegetation(at, radius, PAINT_PRESET, 1))
				painted_plants += added
	painted_ms = float(Time.get_ticks_msec() - start)
	print("GPU_PROBE_PAINTED plants=%d extent_m=%.0f ms=%.0f" % [
		painted_plants, painted_extent_m, painted_ms,
	])
	vegetation.rebuild_from_simulation_state()

## The untrimmed card generator, rebuilt from the shipped source with the crop removed. The
## comparison lives only here: production carries one card geometry, not a switch between two.
func prepare_card_trials() -> void:
	var source := FileAccess.get_file_as_string("res://scripts/renderers/tree_species.gd")
	var crop_line := "var trim: Rect2 = FOLIAGE_ALPHA_RECTS[quadrant]"
	assert(source.contains(crop_line))
	card_catalogues["cards_trimmed"] = vegetation.meshes
	var script := GDScript.new()
	script.source_code = source.replace(crop_line, "var trim := Rect2(0, 0, 1, 1)")
	assert(script.reload() == OK)
	card_probe_scripts.append(script)
	var meshes: Array = script.build_meshes()
	# Hold the distant meshes fixed: the experiment isolates the near cards only.
	for species in [TreeSpecies.CONIFER, TreeSpecies.BROADLEAF]:
		for variant in range(meshes[species].size()):
			for level in [1, 2]:
				meshes[species][variant][level] = vegetation.meshes[species][variant][level]
	card_catalogues["cards_original"] = meshes

## Integer a grid trial carries after `prefix`, as in "f4" or "near200". Returns zero when
## the trial names none, which is what both renderer overrides read as "keep the authored
## value" -- so `grid_off` and every non-E15 trial leave the shipped geometry alone.
func trial_grid_value(trial: String, prefix: String) -> int:
	for part in trial.split("_"):
		if part.begins_with(prefix) and part.substr(prefix.length()).is_valid_int():
			return part.substr(prefix.length()).to_int()
	return 0

func apply_horizon_view(trial: String) -> void:
	if (trial.begins_with("cards_") or trial.begins_with("shadowsweep_")
		or trial.contains("nearsweep_")):
		# In the painted stand, at crown height, looking into the nearest trees.
		camera.position = view_pivot + Vector3(0.0, 12.0, 30.0)
		camera.rotation = Vector3(-0.04, 0.0, 0.0)
		return
	if trial.begins_with("aerial_"):
		# Over the painted stand from about the height of the reference captures, looking
		# 35 degrees down across it, so the view holds both bands and the handover between.
		camera.position = view_pivot + Vector3(0.0, 350.0, 450.0)
		camera.rotation = Vector3(deg_to_rad(-35.0), 0.0, 0.0)
		return
	if (
		not trial.contains("horizon")
		and not trial.begins_with("grid_")
		and not trial.begins_with("abl_")
	):
		return
	# E15 shares the E12 pose: the honest case for a per-patch level choice is the one
	# where a single patch fills the view from the camera to the horizon.
	if trial.begins_with("grid_") or trial.begins_with("abl_"):
		camera.position.y = pivot.y + 30.0
		camera.rotation = Vector3(-0.04, 0.0, 0.0)
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
			view_pivot = (plan["from"] as Vector3).lerp(plan["to"] as Vector3, progress)
			camera.focus_on(view_pivot, camera_radius)
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
		"patch_subdivision_override": vegetation.patch_subdivision_override,
		"upload_budget_override": vegetation.upload_budget_override,
		"near_detail_m": vegetation.near_detail_m(),
		"near_range_override_m": vegetation.near_range_override_m,
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
	print("GPU_PROBE_RESULT %s frame=%s gpu=%s draws=%d primitives=%d pending_frames=%d/%d resident=%d" % [
		trial, entry.frame_ms, entry.gpu_ms, entry.draw_calls, entry.primitives,
		pending_frames, frames.size(), entry.resident_patches_at_end,
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
