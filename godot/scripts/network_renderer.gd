## Coordinates visual refresh after async network mutations (roads, rails, etc.).
##
## The sim thread adds roads/rails asynchronously and sets the `network_dirty` flag
## on `SimCore`. This node polls `is_network_dirty()` once per frame and triggers
## the correct sequence: terrain flatten → terrain visual update → network mesh rebuild.
## Centralising the refresh here means adding a new transport mode (rail, etc.) only
## requires wiring that tool's mesh update call in one place.
##
## Rust methods called: is_network_dirty(), flatten_terrain_for_roads(),
##   clear_terrain_dirty()
extends Node

@onready var simulation_node = $"../SimulationNode"
@onready var terrain = $"../Terrain"
@onready var road_tool = $"../RoadTool"
@onready var zoning_overlay = $"../ZoningOverlay"
# @onready var rail_tool = $"../RailTool"  # uncomment when RailTool exists

func _process(_delta: float) -> void:
	if not simulation_node.is_network_dirty():
		return

	# 1. Flatten terrain to match new road/rail geometry.
	simulation_node.flatten_terrain_for_roads()

	# 2. Redraw the terrain mesh (flatten_terrain_for_roads sets terrain_dirty, but
	#    we update it eagerly here and clear the flag so terrain.gd._process skips it
	#    this frame rather than running a redundant second pass).
	terrain.update_terrain_visuals()
	simulation_node.clear_terrain_dirty()

	# 3. Rebuild each network's visual mesh.
	road_tool.update_main_mesh()
	# rail_tool.update_main_mesh()  # add when RailTool exists

	# 4. Check whether any queued road endpoints are border connections.
	# Must run after the road is in the graph so check_border_candidate() finds the node.
	road_tool.drain_pending_border_checks()

	# 5. Road geometry changed → distance_to_road was recomputed by Rust; re-upload the texture.
	if zoning_overlay: zoning_overlay.mark_distance_dirty()

	# 6. Clear the flag now that the refresh is done — same pattern as clear_terrain_dirty().
	simulation_node.clear_network_dirty()
