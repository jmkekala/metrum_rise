# SPDX-License-Identifier: GPL-2.0-only

## Coordinates visual refresh after async network mutations (roads, rails, etc.).
##
## The sim thread adds roads/rails asynchronously and sets the `network_dirty` flag
## on `SimCore`. This node polls `is_network_dirty()` once per frame and triggers
## the visual sequence: terrain visual update → network mesh rebuild.
## Centralising the refresh here means adding a new transport mode (rail, etc.) only
## requires wiring that tool's mesh update call in one place.
##
## Rust methods called: is_network_dirty(), get_network_render_generation(),
##   get_dirty_terrain_patches(), acknowledge_network_render()
extends Node

const PerfDebug := preload("res://scripts/core/perf_debug.gd")

@onready var simulation_node = $"../SimulationNode"
@onready var terrain = $"../Terrain"
@onready var water = $"../Water"
@onready var road_tool = $"../RoadTool"
@onready var zoning_overlay = $"../ZoningOverlay"
# @onready var rail_tool = $"../RailTool"  # uncomment when RailTool exists

var _road_debug_enabled: bool = false
var _road_geometry_debug_enabled: bool = false
var _road_mesh_refreshed_surface_generation: int = -1

const ROAD_PATCH_DEBUG_MAX_DIRTY_PAIRS := 64

func _ready() -> void:
	_road_debug_enabled = _is_road_debug_enabled()
	_road_geometry_debug_enabled = _is_road_geometry_debug_enabled()

func _process(_delta: float) -> void:
	var perf_enabled := PerfDebug.is_enabled()
	var total_start_us := Time.get_ticks_usec() if perf_enabled else 0
	if not simulation_node.is_network_dirty():
		if road_tool.needs_main_mesh_hydration():
			var generation := _current_road_surface_generation()
			if road_tool.update_main_mesh(generation) == generation:
				road_tool.mark_network_topology_dirty()
		_road_mesh_refreshed_surface_generation = -1
		if perf_enabled:
			PerfDebug.record(
				"network",
				float(Time.get_ticks_usec() - total_start_us) / 1000.0
			)
		return

	if total_start_us == 0:
		total_start_us = Time.get_ticks_usec()

	# 1. Capture one network generation before its terrain state. A changed generation aborts
	# before either renderer stages or commits resources.
	var terrain_rebuild_ms := 0.0
	var surface_generation := _current_road_surface_generation()
	var dirty_terrain_patch_keys: PackedInt32Array = simulation_node.get_dirty_terrain_patches()
	var dirty_patch_pairs := int(dirty_terrain_patch_keys.size() / 2)
	if _current_road_surface_generation() != surface_generation:
		return

	# 2. Validate the complete terrain batch without mutating scene resources. A bad engineered CDT
	# keeps both the previous terrain batch and previous road generation visible.
	var road_mesh_ms := 0.0
	var terrain_visuals_ms := 0.0
	var prepared_terrain_update: Dictionary = {}
	if dirty_patch_pairs > 0:
		var terrain_visuals_start_us := Time.get_ticks_usec()
		prepared_terrain_update = terrain.prepare_terrain_visual_update()
		terrain_visuals_ms = float(Time.get_ticks_usec() - terrain_visuals_start_us) / 1000.0
	if dirty_patch_pairs > 0 and prepared_terrain_update.is_empty():
		var pending_total_ms := float(Time.get_ticks_usec() - total_start_us) / 1000.0
		if perf_enabled:
			PerfDebug.record(
				"network",
				pending_total_ms,
				{
					"terrain_visuals": terrain_visuals_ms,
					"water_visuals": 0.0,
					"road_mesh": road_mesh_ms,
					"border_checks": 0.0,
				}
			)
		return
	if _current_road_surface_generation() != surface_generation:
		return

	# 3. Build and validate changed road chunks without replacing resident instances. Staging also
	# rechecks the generation after potentially expensive ArrayMesh construction.
	var road_mesh_staged := false
	if _road_mesh_refreshed_surface_generation != surface_generation:
		var road_mesh_start_us := Time.get_ticks_usec()
		var staged_generation: int = road_tool.update_main_mesh(surface_generation, true)
		if staged_generation != surface_generation:
			return
		road_mesh_ms = float(Time.get_ticks_usec() - road_mesh_start_us) / 1000.0
		road_mesh_staged = true
	if _current_road_surface_generation() != surface_generation:
		if road_mesh_staged:
			road_tool.discard_staged_main_mesh_update()
		return

	# 4. Commit the already-validated terrain and road batches back-to-back. Godot cannot draw
	# between these calls, so only complete generation pairs become visible.
	if dirty_patch_pairs > 0:
		var terrain_commit_start_us := Time.get_ticks_usec()
		if not terrain.commit_prepared_terrain_visual_update(prepared_terrain_update):
			if road_mesh_staged:
				road_tool.discard_staged_main_mesh_update()
			return
		terrain_visuals_ms += float(Time.get_ticks_usec() - terrain_commit_start_us) / 1000.0
	if road_mesh_staged:
		var road_commit_start_us := Time.get_ticks_usec()
		var rendered_generation: int = road_tool.commit_staged_main_mesh_update(surface_generation)
		if rendered_generation != surface_generation:
			return
		road_mesh_ms += float(Time.get_ticks_usec() - road_commit_start_us) / 1000.0
		_road_mesh_refreshed_surface_generation = surface_generation
	# rail_tool.update_main_mesh()  # add when RailTool exists

	# 5. An exact acknowledgement is the commit fence for consumers that read live sim state. If a
	# newer revision arrived, the complete older visual pair may remain visible while the new pair
	# stays dirty, but dependent water/zoning work waits for that new revision.
	if not simulation_node.acknowledge_network_render(surface_generation):
		return
	_road_mesh_refreshed_surface_generation = -1
	road_tool.mark_network_topology_dirty()

	var water_visuals_start_us := Time.get_ticks_usec()
	if water and water.has_method("refresh_road_clipped_patches"):
		water.refresh_road_clipped_patches(dirty_terrain_patch_keys)
	var water_visuals_ms := float(Time.get_ticks_usec() - water_visuals_start_us) / 1000.0

	# 6. Check whether any queued road endpoints are border connections.
	# Must run after the road is in the graph so check_border_candidate() finds the node.
	var border_checks_start_us := Time.get_ticks_usec()
	road_tool.drain_pending_border_checks()
	var border_checks_ms := float(Time.get_ticks_usec() - border_checks_start_us) / 1000.0

	# 7. Road geometry changed → refresh no-build edge overlay geometry.
	if zoning_overlay: zoning_overlay.mark_no_build_dirty()

	var total_ms := float(Time.get_ticks_usec() - total_start_us) / 1000.0
	if perf_enabled:
		PerfDebug.record(
			"network",
			total_ms,
			{
				"terrain_visuals": terrain_visuals_ms,
				"water_visuals": water_visuals_ms,
				"road_mesh": road_mesh_ms,
				"border_checks": border_checks_ms,
			}
		)
	if _road_debug_enabled:
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
		if _road_geometry_debug_enabled:
			_print_road_geometry_patch_debug(dirty_terrain_patch_keys)

func _current_road_surface_generation() -> int:
	if simulation_node.has_method("get_network_render_generation"):
		return int(simulation_node.get_network_render_generation())
	return 0

func _is_road_debug_enabled() -> bool:
	var debug_value := OS.get_environment("METRUM_DEBUG").strip_edges()
	if debug_value != "1":
		return false
	var filter := OS.get_environment("METRUM_DEBUG_FILTER").strip_edges().to_lower()
	return filter == "road"

func _is_road_geometry_debug_enabled() -> bool:
	if OS.get_environment("METRUM_DEBUG_ROAD_GEOMETRY_DUMP").strip_edges() == "1":
		return true
	var debug_value := OS.get_environment("METRUM_DEBUG").strip_edges()
	if debug_value != "1":
		return false
	var filter := OS.get_environment("METRUM_DEBUG_FILTER").strip_edges().to_lower()
	return filter == "road"

func _print_road_geometry_patch_debug(dirty_terrain_patch_keys: PackedInt32Array) -> void:
	var diagnostic_start_us := Time.get_ticks_usec()
	var dirty_pair_count := int(dirty_terrain_patch_keys.size() / 2)
	print(
		"[DEBUG:road] road_geometry_patch_debug dirty_patch_pairs=%d"
		% dirty_pair_count
	)
	if dirty_pair_count > ROAD_PATCH_DEBUG_MAX_DIRTY_PAIRS:
		print(
			"[DEBUG:road] road_patch_debug_skipped dirty_patch_pairs=%d limit=%d reason=too_many_dirty_patches"
			% [dirty_pair_count, ROAD_PATCH_DEBUG_MAX_DIRTY_PAIRS]
		)
		var skipped_ms := float(Time.get_ticks_usec() - diagnostic_start_us) / 1000.0
		print("[DEBUG:road] road_geometry_patch_debug_terrain_ms=0.000")
		print("[DEBUG:road] road_geometry_patch_debug_water_ms=0.000")
		print("[DEBUG:road] road_geometry_patch_debug_zoning_ms=0.000")
		print("[DEBUG:road] road_geometry_patch_debug_ms=%.3f" % skipped_ms)
		return
	if terrain and terrain.has_method("road_geometry_debug_patch_lines"):
		var terrain_start_us := Time.get_ticks_usec()
		var terrain_lines: Array = terrain.road_geometry_debug_patch_lines(dirty_terrain_patch_keys)
		for line_variant in terrain_lines:
			print("[DEBUG:road] %s" % String(line_variant))
		var terrain_ms := float(Time.get_ticks_usec() - terrain_start_us) / 1000.0
		print("[DEBUG:road] road_geometry_patch_debug_terrain_ms=%.3f" % terrain_ms)
	else:
		print("[DEBUG:road] road_geometry_patch_debug_terrain_ms=0.000")
	if water and water.has_method("road_geometry_debug_patch_lines"):
		var water_start_us := Time.get_ticks_usec()
		var water_lines: Array = water.road_geometry_debug_patch_lines(dirty_terrain_patch_keys)
		for line_variant in water_lines:
			print("[DEBUG:road] %s" % String(line_variant))
		var water_ms := float(Time.get_ticks_usec() - water_start_us) / 1000.0
		print("[DEBUG:road] road_geometry_patch_debug_water_ms=%.3f" % water_ms)
	else:
		print("[DEBUG:road] road_geometry_patch_debug_water_ms=0.000")
	if zoning_overlay and zoning_overlay.has_method("road_geometry_debug_patch_lines"):
		var zoning_start_us := Time.get_ticks_usec()
		var zoning_lines: Array = zoning_overlay.road_geometry_debug_patch_lines(dirty_terrain_patch_keys)
		for line_variant in zoning_lines:
			print("[DEBUG:road] %s" % String(line_variant))
		var zoning_ms := float(Time.get_ticks_usec() - zoning_start_us) / 1000.0
		print("[DEBUG:road] road_geometry_patch_debug_zoning_ms=%.3f" % zoning_ms)
	else:
		print("[DEBUG:road] road_geometry_patch_debug_zoning_ms=0.000")
	var diagnostic_ms := float(Time.get_ticks_usec() - diagnostic_start_us) / 1000.0
	print("[DEBUG:road] road_geometry_patch_debug_ms=%.3f" % diagnostic_ms)
