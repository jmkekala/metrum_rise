# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: gate_check.gd
#  script_path: gate_check.gd
#  module_name: gate_check
#  version: 0.2.0
#  author: [BantedHam]
#  description: The cache-free parse gate: every adapter, spike, and
#           probe loaded as a fresh GDScript so a parse error shows
#           in seconds instead of after a twelve minute boot. Run it
#           before any long launch.
#  kind: spike
#  spec: none
#  internal_dependencies: []
#  external_dependencies: [Godot 4.x]
#  features: [parse-gate, cache-free]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-09-02
# =========================================================================

# Cache-free parse gate: loads changed scripts as fresh GDScript objects.
extends SceneTree

# =========================================================================
# INIT
# =========================================================================
func _init() -> void:
	var files := [
		"res://scripts/core/engine_tick.gd",
		"res://scripts/core/input_manager.gd",
		"res://scripts/core/engine_mind_source.gd",
		"res://scripts/core/engine_director_source.gd",
		"res://scripts/core/spike_tide_mind_water.gd",
		"res://scripts/core/spike_director.gd",
		"res://spike_engine_live.gd",
		"res://scripts/core/engine_terrain_source.gd",
		"res://scripts/core/engine_network_source.gd",
		"res://scripts/core/engine_social_source.gd",
		"res://scripts/renderers/terrain.gd",
		"res://scripts/core/spike_terrain_source.gd",
		"res://scripts/core/engine_water_source.gd",
		"res://scripts/core/spike_stats.gd",
		"res://scripts/core/spike_boundary.gd",
		"res://scripts/core/spike_gateway.gd",
		"res://scripts/core/spike_mesh_source.gd",
		"res://scripts/core/spike_sound_source.gd",
		"res://scripts/core/spike_system_sources.gd",
		"res://scripts/core/spike_weather_fire.gd",
		"res://scripts/core/spike_vehicle.gd",
		"res://scripts/core/spike_consumer_wave.gd",
		"res://scripts/core/spike_harness_wave.gd",
	]
	for f in files:
		var src := FileAccess.get_file_as_string(f)
		var s := GDScript.new()
		s.source_code = src
		var err := s.reload()
		print("GATE %s err=%d" % [str(f).get_file(), err])
	quit()
