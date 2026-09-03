# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: spike_mesh_source.gd
#  script_path: scripts/core/spike_mesh_source.gd
#  module_name: spike_mesh_source
#  version: 0.1.1
#  author: [BantedHam]
#  description: Drills the engine mesh source: frozen geometry is real,
#           deterministic per key, distinct between keys, cache-hit on
#           the second ask, and the toggle round-trips with packs as the
#           documented fallback when off.
#  kind: spike
#  spec: none
#  internal_dependencies: [scripts/core/engine_mesh_source.gd]
#  external_dependencies: [Godot 4.x]
#  features: [frozen-mesh-drill, toggle-drill]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## Run: godot --headless --path godot --script scripts/core/spike_mesh_source.gd
extends SceneTree

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const Source := preload("res://scripts/core/engine_mesh_source.gd")

var passed := 0
var failed := 0

# =========================================================================
# CHECK
# =========================================================================
func _check(name: String, ok: bool) -> void:
	if ok:
		passed += 1
		print("  ok   %s" % name)
	else:
		failed += 1
		print("  FAIL %s" % name)

# =========================================================================
# INIT
# =========================================================================
func _init() -> void:
	var m1 := Source.frozen_mesh_for("mill", 0)
	_check("freeze produces a mesh", m1 != null)
	_check("frozen mesh has geometry",
		m1 != null and m1.get_surface_count() > 0
		and m1.surface_get_array_len(0) > 0)

	var m2 := Source.frozen_mesh_for("mill", 0)
	_check("same key shares one frozen mesh", m2 == m1)

	var other := Source.frozen_mesh_for("granary", 2)
	_check("different key freezes different geometry", other != m1)
	if m1 != null and other != null:
		var a := (m1.surface_get_arrays(0)[Mesh.ARRAY_VERTEX] as PackedVector3Array)
		var b := (other.surface_get_arrays(0)[Mesh.ARRAY_VERTEX] as PackedVector3Array)
		_check("the field made them differ", a != b)

	Source.set_enabled(true)
	_check("toggle turns on", Source.enabled())
	Source.set_enabled(false)
	_check("toggle turns off, packs are the path again", not Source.enabled())

	print("=== %d passed, %d failed ===" % [passed, failed])
	load("res://scripts/core/spike_stats.gd").record("spike_mesh_source", passed, failed)
	quit(1 if failed > 0 else 0)
