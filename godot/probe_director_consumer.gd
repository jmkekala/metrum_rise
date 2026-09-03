# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: probe_director_consumer.gd
#  script_path: probe_director_consumer.gd
#  module_name: probe_director_consumer
#  version: 0.1.0
#  author: [BantedHam]
#  description: Headless drill for the director consumer: the pacing
#           population multiplier reaches the sim through border
#           openness, the twelfth fiscal control, so a fade closes
#           the border and a build-up reopens it.
#  kind: spike
#  spec: none
#  internal_dependencies: [scripts/core/engine_director_source.gd,
#           scripts/core/spike_stats.gd]
#  external_dependencies: [Godot 4.x]
#  features: [director-pacing, border-openness]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-09-02
# =========================================================================

# Headless drill: the director's pacing reaches the sim through the
# game's own border-openness dial.
extends SceneTree

# =========================================================================
# THE DECLARATIONS
# =========================================================================

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
# OPENNESS
# =========================================================================
func _openness(sim: Node) -> float:
	var ov: Dictionary = sim.get_economy_overview()
	for c in ov.get("fiscal_policy_controls", []):
		var cd := c as Dictionary
		if String(cd.get("id", "")) == "border_openness":
			return float(cd.get("value", -1.0))
	return -1.0

# =========================================================================
# RUN
# =========================================================================
func _run() -> void:
	await process_frame
	var sim := SimulationNode.new()
	sim.name = "SimulationNode"
	get_root().add_child(sim)
	await process_frame
	sim.create_blank_world(500.0, 500.0, 10.0, 640.0, 0.0)
	var et := get_root().get_node_or_null("EngineTick")
	_check("the tick harness is live", et != null)
	var ids := PackedStringArray()
	for c in sim.get_economy_overview().get("fiscal_policy_controls", []):
		ids.append(String((c as Dictionary).get("id", "?")))
	print("  [live] policy controls: ", ids)
	print("  [live] direct set returns: ", sim.set_economy_policy_value("border_openness", 0.25))
	print("  [live] read after direct set: %.2f" % _openness(sim))
	var Director := load("res://scripts/core/engine_director_source.gd")
	Director.reset()

	# A fading peak closes the gate: population_for_state(peak_fade)=0.
	Director.state = "peak_fade"
	et.pacing = Director.status()
	et._deliver_engine_inputs()
	var closed := _openness(sim)
	print("  [live] peak_fade openness: %.2f" % closed)
	_check("the fade closes the border", closed == 0.0)

	# Build-up admits everyone again.
	Director.state = "build_up"
	et.pacing = Director.status()
	et._deliver_engine_inputs()
	var open := _openness(sim)
	print("  [live] build_up openness: %.2f" % open)
	_check("the build-up reopens the border", open == 1.0)

	Director.reset()
	print("=== %d passed, %d failed ===" % [passed, failed])
	load("res://scripts/core/spike_stats.gd").record(
		"probe_director_consumer", passed, failed)
	quit(1 if failed > 0 else 0)

# =========================================================================
# INIT
# =========================================================================
func _init() -> void:
	_run()
