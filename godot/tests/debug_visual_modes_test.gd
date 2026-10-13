# SPDX-License-Identifier: GPL-2.0-only

## Visual debug modes switch at runtime: console command and Visual pickers, shader globals,
## the walker VAT material, building-site materials and terrain/water residency and LODs.
extends SceneTree

const DebugConsoleScript := preload("res://scripts/ui/debug_console/debug_console.gd")
const TerrainScript := preload("res://scripts/renderers/terrain.gd")
const WaterScript := preload("res://scripts/renderers/water.gd")
const AgentsScript := preload("res://scripts/renderers/agents.gd")
const BuildingsScript := preload("res://scripts/renderers/buildings.gd")
const MODES := [
	"terrain_visual", "terrain_grass", "building_sites_visual", "pedestrian_vat",
	"terrain_force_lod1", "terrain_force_full_world", "junction_connectors",
]
# Writes both shader globals into one pixel, since global values cannot be read back at runtime.
const GLOBAL_PROBE_SHADER := """
shader_type canvas_item;
global uniform int terrain_visual_debug_mode;
global uniform int terrain_grass_visual_debug_mode;
void fragment() {
	COLOR = vec4(float(terrain_visual_debug_mode) / 10.0, float(terrain_grass_visual_debug_mode) / 10.0, 0.0, 1.0);
}
"""
# Wide enough that some patches sit past both the residency cull distance and the first LOD
# distance from a camera in one corner.
const WORLD_M := 8000.0
const CAMERA_FAR_M := 3000.0
const SETTLE_FRAMES := 1200

var _failures := 0
var _changes: Array[String] = []

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	# `--script` runs load autoloads; fall back to a local console if this one did not.
	var console: Node = root.get_node_or_null("DebugConsole")
	if console == null:
		console = DebugConsoleScript.new()
		console.name = "DebugConsole"
		root.add_child(console)
	console.visual_mode_changed.connect(func(mode: String) -> void: _changes.append(mode))
	_check_registry()
	_check_switching(console)
	await _check_shader_globals(console)
	await _check_renderers(console)
	_expect(console.set_visual_mode("terrain_visual", "off"), "cleanup must succeed")
	print("Debug visual mode tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)

func _check_registry() -> void:
	_expect(Array(MetrumDebug.visual_mode_names()) == MODES, "visual mode registry must list the visual modes in order")
	for mode in MODES:
		var values := MetrumDebug.visual_mode_values(mode)
		_expect(values.size() >= 2 and values[0] == "off", "%s must start with off" % mode)
		_expect(MetrumDebug.visual_mode(mode) == "off", "%s must start off without launch variables" % mode)
	_expect(MetrumDebug.visual_mode_values("no_such_mode").is_empty(), "unknown modes have no values")
	_expect(not MetrumDebug.startup_modes().has("terrain_visual"), "visual modes must leave the startup-only list")

func _check_switching(console: Node) -> void:
	for mode in MODES:
		var values := MetrumDebug.visual_mode_values(mode)
		var last := values[values.size() - 1]
		_changes.clear()
		console._run_command("visual %s %s" % [mode, last])
		_expect(MetrumDebug.visual_mode(mode) == last, "command must switch %s to %s" % [mode, last])
		_expect(_changes == [mode], "a switch must notify renderers once for %s" % mode)
		_changes.clear()
		console._run_command("visual %s %s" % [mode, last])
		_expect(_changes.is_empty(), "reselecting the current value must not notify renderers")
		var picker: OptionButton = console._view._visual_pickers[mode]
		_expect(picker.get_item_text(picker.selected) == last, "the %s picker must follow the command" % mode)
		picker.select(0)
		picker.item_selected.emit(0)
		_expect(MetrumDebug.visual_mode(mode) == "off", "the %s picker must switch it off" % mode)

	var before: int = MetrumDebug.read_since(0, 0)["last_seq"]
	console._run_command("visual terrain_visual nonsense")
	_expect(MetrumDebug.visual_mode("terrain_visual") == "off", "an unknown value must be refused")
	var events: Dictionary = MetrumDebug.read_since(before, 64)
	_expect(Array(events["message"]).any(func(line: String) -> bool: return line.contains("refused")), "a refusal must be logged on events")
	_expect(not MetrumDebug.set_visual_mode("no_such_mode", "on"), "an unknown mode must be refused")

	console._run_command("visual terrain_visual sun")
	_expect(MetrumDebug.visual_mode("terrain_visual") == "lighting", "aliases must resolve in commands")
	console._run_command("visual terrain_grass 9")
	_expect(MetrumDebug.visual_mode("terrain_grass") == "luminance", "indices must resolve in commands")
	console._run_command("visual off")
	for mode in MODES:
		_expect(MetrumDebug.visual_mode(mode) == "off", "visual off must clear %s" % mode)

# Headless runs use the dummy renderer, which draws nothing; run.sh also runs this test on a
# rendered display, where a probe pixel shows the values the shaders receive.
func _check_shader_globals(console: Node) -> void:
	if DisplayServer.get_name() == "headless":
		print("Shader global probe skipped: headless renderer")
		return
	var viewport := SubViewport.new()
	viewport.size = Vector2i(4, 4)
	viewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	var rect := ColorRect.new()
	rect.size = Vector2(4, 4)
	var material := ShaderMaterial.new()
	material.shader = Shader.new()
	material.shader.code = GLOBAL_PROBE_SHADER
	rect.material = material
	viewport.add_child(rect)
	root.add_child(viewport)
	for values in [["relief", "footprint"], ["lighting", "raw"], ["off", "off"]]:
		console._run_command("visual terrain_visual %s" % values[0])
		console._run_command("visual terrain_grass %s" % values[1])
		for frame in 3:
			await RenderingServer.frame_post_draw
		var pixel := viewport.get_texture().get_image().get_pixel(1, 1)
		var expected := Vector2(MetrumDebug.visual_mode_index("terrain_visual"), MetrumDebug.visual_mode_index("terrain_grass"))
		var drawn := Vector2(pixel.r, pixel.g) * 10.0
		_expect(drawn.distance_to(expected) < 0.3, "shaders must receive %s, drew %s" % [expected, drawn])
	viewport.queue_free()

func _check_renderers(console: Node) -> void:
	_check_building_sites(console)
	var host := Node3D.new()
	root.add_child(host)
	var simulation := SimulationNode.new()
	simulation.name = "SimulationNode"
	host.add_child(simulation)
	_expect(simulation.create_blank_world(WORLD_M, WORLD_M, 20.0, 1000.0, 0.0), "fixture world must be created")
	# Flooding the flat world gives the water renderer a patch under every terrain patch.
	simulation.begin_world_open_water_fill_preview(Vector2.ZERO, 5.0)
	_expect(simulation.commit_world_open_water_fill_preview(), "fixture water must commit")
	# No network renderer here: acknowledge the fixture's road surface generation as it would,
	# or terrain residency keeps waiting for that publication.
	for frame in SETTLE_FRAMES:
		if not simulation.is_network_dirty():
			break
		simulation.acknowledge_network_render(simulation.get_road_tool_surface_generation())
		await process_frame
	_expect(not simulation.is_network_dirty(), "the fixture network must be acknowledged")
	var camera := Camera3D.new()
	camera.far = CAMERA_FAR_M
	host.add_child(camera)
	camera.global_position = Vector3(-WORLD_M * 0.5 + 200.0, 300.0, -WORLD_M * 0.5 + 200.0)
	camera.look_at(Vector3.ZERO)
	camera.current = true
	var agents := AgentsScript.new()
	agents.name = "Agents"
	host.add_child(agents)
	_check_walkers(console, agents)
	# In the game scene both renderers enter the tree together. Added one by one, water first
	# resolves a placeholder (its uses check for terrain methods), then terrain resolves the real
	# water and water is pointed at the real terrain before it builds its patches.
	var placeholder := Node.new()
	placeholder.name = "Terrain"
	host.add_child(placeholder)
	var water := WaterScript.new()
	water.name = "Water"
	host.add_child(water)
	host.remove_child(placeholder)
	placeholder.free()
	var terrain := TerrainScript.new()
	terrain.name = "Terrain"
	host.add_child(terrain)
	water.terrain_node = terrain
	water.rebuild_from_simulation_state()
	await _check_residency(console, terrain, water)
	host.queue_free()
	await process_frame

func _check_walkers(console: Node, agents: Node) -> void:
	_expect(not agents.walker_mmis.is_empty(), "the walker renderer must build its VAT materials")
	_expect(console.set_visual_mode("pedestrian_vat", "offsets"), "pedestrian_vat must switch")
	for mmi in agents.walker_mmis.values():
		_expect(int(mmi.material_override.get_shader_parameter("debug_mode")) == 3, "walker materials must show the offsets view")
	_expect(console.set_visual_mode("pedestrian_vat", "off"), "pedestrian_vat must switch off")
	for mmi in agents.walker_mmis.values():
		_expect(int(mmi.material_override.get_shader_parameter("debug_mode")) == 0, "walker materials must return to normal")

# The site renderer is not added to the tree: its packs and frames need a running city. The
# switch handler and the mesh apply path are the parts this mode changes.
func _check_building_sites(console: Node) -> void:
	var buildings := BuildingsScript.new()
	buildings._setup_building_site_surfaces()
	console.visual_mode_changed.connect(buildings._on_visual_mode_changed)
	var normal_ground: Material = buildings.building_site_ground_instance.material_override
	var triangle := PackedVector3Array([Vector3.ZERO, Vector3(1, 0, 0), Vector3(0, 0, 1)])
	var data := {"revision": 7, "ground_vertices": triangle, "asphalt_vertices": triangle}
	buildings.building_site_revision = 7
	_expect(console.set_visual_mode("building_sites_visual", "material"), "building_sites_visual must switch")
	_expect(buildings.building_site_revision == -1, "a switch must request a fresh site frame")
	_expect(buildings.building_site_ground_instance.material_override != normal_ground, "the ground override must show the overlay")
	buildings._apply_building_site_mesh_data(data)
	var surface: Material = buildings.building_site_surface_instance.mesh.surface_get_material(0)
	_expect(surface == buildings.building_site_debug_materials.get("asphalt"), "rebuilt site surfaces must use overlay materials")
	_expect(console.set_visual_mode("building_sites_visual", "off"), "building_sites_visual must switch off")
	_expect(buildings.building_site_ground_instance.material_override == normal_ground, "the ground override must return to normal")
	buildings._apply_building_site_mesh_data(data)
	surface = buildings.building_site_surface_instance.mesh.surface_get_material(0)
	_expect(surface == WorldMaterials.site_surface_material(WorldMaterials.MATERIAL_ASPHALT), "rebuilt site surfaces must return to normal")
	console.visual_mode_changed.disconnect(buildings._on_visual_mode_changed)
	buildings.free()

func _check_residency(console: Node, terrain: Node, water: Node) -> void:
	_expect(await _settle(terrain, water), "camera-based residency must settle")
	var keys := _resident_keys(terrain)
	var terrain_lods := _lods(terrain)
	var water_lods := _lods(water)
	var capacity: int = terrain.patch_cols * terrain.patch_rows
	_expect(keys.size() > 0 and keys.size() < capacity, "the camera must load part of the world, got %d of %d" % [keys.size(), capacity])
	_expect(terrain_lods.values().any(func(step: int) -> bool: return step > 1), "the fixture must hold distant terrain at a coarser LOD")
	_expect(water_lods.values().any(func(step: int) -> bool: return step > 1), "the fixture must hold distant water at a coarser LOD")

	_expect(console.set_visual_mode("terrain_force_full_world", "on"), "force_full_world must switch on")
	_expect(await _settle(terrain, water), "full-world residency must settle")
	_expect(terrain.resident_patch_lookup.size() == capacity, "force_full_world must load every patch")
	_expect(console.set_visual_mode("terrain_force_full_world", "off"), "force_full_world must switch off")
	_expect(await _settle(terrain, water), "residency must settle back")
	_expect(_resident_keys(terrain) == keys, "switching off must return to the camera-based patch set")
	_expect(_lods(terrain) == terrain_lods, "switching off must return terrain to the camera-based LODs")

	_expect(console.set_visual_mode("terrain_force_lod1", "on"), "force_lod1 must switch on")
	_expect(await _settle(terrain, water), "forced LODs must settle")
	_expect(_lods(terrain).values().all(func(step: int) -> bool: return step == 1), "force_lod1 must refine every terrain patch")
	_expect(_lods(water).values().all(func(step: int) -> bool: return step == 1), "force_lod1 must refine every water patch")
	_expect(console.set_visual_mode("terrain_force_lod1", "off"), "force_lod1 must switch off")
	_expect(await _settle(terrain, water), "LODs must settle back")
	_expect(_lods(terrain) == terrain_lods, "switching off must return terrain to the camera-based LODs")
	_expect(_lods(water) == water_lods, "switching off must return water to the camera-based LODs")

func _resident_keys(renderer: Node) -> Array:
	var keys: Array = renderer.resident_patch_lookup.keys()
	keys.sort()
	return keys

func _lods(renderer: Node) -> Dictionary:
	var lods := {}
	for key in renderer.resident_patch_lookup:
		var patch: Dictionary = renderer.patches.get(key, {})
		if not patch.is_empty() and not bool(patch.get("height_is_baked", false)):
			lods[key] = int(patch.get("lod_step", 1))
	return lods

# Settled means no pending residency work, empty LOD queues and the same patch counts for a
# run of frames.
func _settle(terrain: Node, water: Node) -> bool:
	var stable := 0
	var last := Vector2i(-1, -1)
	for frame in SETTLE_FRAMES:
		await process_frame
		var counts := Vector2i(terrain.resident_patch_lookup.size(), water.resident_patch_lookup.size())
		var busy: bool = (
			terrain._terrain_residency_pending_mutations
			or water._water_residency_pending_mutations
			or not terrain.patch_lod_refresh_queue.is_empty()
			or not water.patch_lod_refresh_queue.is_empty()
			or not terrain.patch_payload_requested.is_empty()
		)
		stable = stable + 1 if not busy and counts == last else 0
		last = counts
		if stable >= 30:
			return true
	return false
