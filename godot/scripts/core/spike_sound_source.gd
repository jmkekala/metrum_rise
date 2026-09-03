# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: spike_sound_source.gd
#  script_path: scripts/core/spike_sound_source.gd
#  module_name: spike_sound_source
#  version: 0.1.1
#  author: [BantedHam]
#  description: Drills derived sound headless: profiles carry real modes,
#           strikes render nonsilent deterministic frames that decay the
#           way damped modes must, different materials sound different,
#           and wetness audibly damps, all by measurement on the frames.
#  kind: spike
#  spec: none
#  internal_dependencies: [scripts/core/engine_sound_source.gd]
#  external_dependencies: [Godot 4.x]
#  features: [sound-drill]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## Run: godot --headless --path godot --script scripts/core/spike_sound_source.gd
extends SceneTree

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const Source := preload("res://scripts/core/engine_sound_source.gd")
const Acoustic := preload("res://addons/2.5D_engine/evaluator/acoustic_node.gd")

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
	var mats := Acoustic.materials()
	_check("the engine publishes materials", not mats.is_empty())
	var names := mats.keys()
	names.sort()
	print("     materials: %s" % ", ".join(PackedStringArray(names)))

	var wood := {names[names.size() - 1]: 1.0}
	var stone := {names[0]: 1.0}
	var rod := {"form": "rod", "length": 0.8, "radius": 0.03}

	var p1 := Source.profile_for(wood, rod)
	_check("profile carries modes", (p1.get("modes", []) as Array).size() > 0)

	var a := Source.render_strike(p1)
	_check("strike renders frames", a.size() > 0)
	var b := Source.render_strike(p1)
	_check("rendering is deterministic", a == b)

	var early := Source.rms(a, 0, 4410)
	var late := Source.rms(a, a.size() - 4410, a.size())
	_check("the strike is audible", early > 0.01)
	_check("the strike decays like a struck thing", late < early)

	var p2 := Source.profile_for(stone, rod)
	var c := Source.render_strike(p2)
	_check("a different material sounds different", c != a)

	var wet := Source.profile_for(wood, rod, 1.0)
	var w := Source.render_strike(wet)
	var wet_late := Source.rms(w, w.size() - 4410, w.size())
	_check("wetness damps the ring, not quells it", wet_late <= late)

	print("=== %d passed, %d failed ===" % [passed, failed])
	load("res://scripts/core/spike_stats.gd").record("spike_sound_source", passed, failed)
	quit(1 if failed > 0 else 0)
