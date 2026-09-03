# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: spike_director.gd
#  script_path: scripts/core/spike_director.gd
#  module_name: spike_director
#  version: 0.2.1
#  author: [BantedHam]
#  description: Drills the director: events excite intensity and quiet
#           relaxes it, states move on the node's own machine with the
#           seeded variety walk, the multipliers answer per state
#           (peak_fade's zero population is the design, not a fault),
#           and everything resets clean including the walk's clock.
#  kind: spike
#  spec: none
#  internal_dependencies: [scripts/core/engine_director_source.gd]
#  external_dependencies: [Godot 4.x]
#  features: [director-drill]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## Run: godot --headless --path godot --script scripts/core/spike_director.gd
extends SceneTree

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const DirectorSource := preload("res://scripts/core/engine_director_source.gd")

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
	DirectorSource.reset()
	var start := DirectorSource.status()
	_check("the director wakes relaxed", str(start["state"]) == "relax")

	# A storm of events.
	var busy: Array = []
	for i in 8:
		busy.append({"name": "rust/shock_%d" % i})
	var before := DirectorSource.intensity
	for i in 30:
		DirectorSource.step(busy, 1.0)
	var excited := DirectorSource.intensity
	_check("events excite the director", excited > before)
	print("     after the storm: state=%s intensity=%.3f" % [
		DirectorSource.state, excited])

	# Quiet.
	for i in 120:
		DirectorSource.step([], 1.0)
	_check("quiet relaxes the director", DirectorSource.intensity < excited)

	var s := DirectorSource.status(100, 1.0)
	print("     state=%s population=%d strength=%.2f" % [
		str(s["state"]), int(s["population"]), float(s["threat_strength"])])
	# peak_fade proposes zero population by design: the director changes
	# frequency, never amplitude, and the fade IS the frequency dropping.
	_check("population multiplier answers per state",
		int(s["population"]) == 0 if str(s["state"]) == "peak_fade"
		else int(s["population"]) > 0)
	_check("threat strength answers", float(s["threat_strength"]) > 0.0)

	# Determinism: the same history gives the same drama.
	DirectorSource.reset()
	for i in 30:
		DirectorSource.step(busy, 1.0)
	var replay := DirectorSource.intensity
	_check("the same history plays the same drama", replay == excited)

	DirectorSource.reset()
	_check("reset returns to relax",
		str(DirectorSource.status()["state"]) == "relax"
		and DirectorSource.intensity == 0.0
		and DirectorSource.tick_index == 0)

	print("=== %d passed, %d failed ===" % [passed, failed])
	load("res://scripts/core/spike_stats.gd").record("spike_director", passed, failed)
	quit(1 if failed > 0 else 0)
