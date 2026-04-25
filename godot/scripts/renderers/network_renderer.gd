## Coordinates visual refresh after async network mutations (roads, rails, etc.).
##
## The sim thread adds roads/rails asynchronously and sets the `network_dirty` flag
## on `SimCore`. This node polls `is_network_dirty()` once per frame and triggers
## the correct sequence: network-surface terrain rebuild → terrain visual update → network mesh rebuild.
## Centralising the refresh here means adding a new transport mode (rail, etc.) only
## requires wiring that tool's mesh update call in one place.
##
## Rust methods called: is_network_dirty(), rebuild_network_surface_terrain(),
##   clear_terrain_dirty()
extends Node

@onready var simulation_node = $"../SimulationNode"
@onready var terrain = $"../Terrain"
@onready var water = $"../Water"
@onready var road_tool = $"../RoadTool"
@onready var zoning_overlay = $"../ZoningOverlay"
# @onready var rail_tool = $"../RailTool"  # uncomment when RailTool exists

var _road_debug_enabled: bool = false

func _ready() -> void:
	_road_debug_enabled = _is_road_debug_enabled()

func _process(_delta: float) -> void:
	if not simulation_node.is_network_dirty():
		return

	var total_start_us := Time.get_ticks_usec()

	# 1. Rebuild road-surface-driven visual terrain to match the edited network.
	var terrain_rebuild_start_us := Time.get_ticks_usec()
	simulation_node.rebuild_network_surface_terrain()
	var terrain_rebuild_ms := float(Time.get_ticks_usec() - terrain_rebuild_start_us) / 1000.0
	var dirty_terrain_patch_keys: PackedInt32Array = simulation_node.get_dirty_terrain_patches()
	var dirty_patch_pairs := int(dirty_terrain_patch_keys.size() / 2)

	# 2. Redraw the terrain mesh (rebuild_network_surface_terrain sets terrain_dirty, but
	#    we update it eagerly here and clear the flag so terrain.gd._process skips it
	#    this frame rather than running a redundant second pass).
	var terrain_visuals_start_us := Time.get_ticks_usec()
	terrain.update_terrain_visuals()
	var terrain_visuals_ms := float(Time.get_ticks_usec() - terrain_visuals_start_us) / 1000.0
	simulation_node.clear_terrain_dirty()

	var water_visuals_start_us := Time.get_ticks_usec()
	if water and water.has_method("refresh_road_clipped_patches"):
		water.refresh_road_clipped_patches(dirty_terrain_patch_keys)
	var water_visuals_ms := float(Time.get_ticks_usec() - water_visuals_start_us) / 1000.0

	# 3. Rebuild each network's visual mesh.
	var road_mesh_start_us := Time.get_ticks_usec()
	road_tool.update_main_mesh()
	var road_mesh_ms := float(Time.get_ticks_usec() - road_mesh_start_us) / 1000.0
	# rail_tool.update_main_mesh()  # add when RailTool exists

	# 4. Check whether any queued road endpoints are border connections.
	# Must run after the road is in the graph so check_border_candidate() finds the node.
	var border_checks_start_us := Time.get_ticks_usec()
	road_tool.drain_pending_border_checks()
	var border_checks_ms := float(Time.get_ticks_usec() - border_checks_start_us) / 1000.0

	# 5. Road geometry changed → distance_to_road was recomputed by Rust; re-upload the texture.
	if zoning_overlay: zoning_overlay.mark_distance_dirty()

	# 6. Clear the flag now that the refresh is done — same pattern as clear_terrain_dirty().
	simulation_node.clear_network_dirty()

	if _road_debug_enabled:
		var total_ms := float(Time.get_ticks_usec() - total_start_us) / 1000.0
		print(
			"[DEBUG:road] refresh terrain_rebuild_ms=%.3f dirty_patches=%d terrain_visuals_ms=%.3f water_visuals_ms=%.3f road_mesh_ms=%.3f border_checks_ms=%.3f total_ms=%.3f"
			% [
				terrain_rebuild_ms,
				dirty_patch_pairs,
				terrain_visuals_ms,
				water_visuals_ms,
				road_mesh_ms,
				border_checks_ms,
				total_ms,
			]
		)

func _is_road_debug_enabled() -> bool:
	var debug_value := OS.get_environment("METRUM_DEBUG").strip_edges()
	if debug_value != "1":
		return false
	var filter := OS.get_environment("METRUM_DEBUG_FILTER").strip_edges().to_lower()
	return filter == "road" or filter == "road-geometry"
