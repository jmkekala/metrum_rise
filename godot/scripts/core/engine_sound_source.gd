# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: engine_sound_source.gd
#  script_path: scripts/core/engine_sound_source.gd
#  module_name: engine_sound_source
#  version: 0.1.0
#  author: [BantedHam]
#  description: Derived procedural sound: the 2.5D engine's acoustic
#           profile (modal frequencies and decay times computed from
#           material composition, geometry, and wetness) rendered into
#           audio frames by damped modal synthesis. Nothing is authored
#           and nothing is sampled; what a thing sounds like follows
#           from what the game says it is made of.
#  kind: module
#  spec: damped modal synthesis over acoustic_node profiles
#  internal_dependencies: [addons/2.5D_engine/evaluator/acoustic_node.gd]
#  external_dependencies: [Godot 4.x]
#  features: [modal-synthesis, derived-sound, deterministic]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## Modal profiles in, audio frames out.
@tool
extends RefCounted

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const Acoustic := preload("res://addons/2.5D_engine/evaluator/acoustic_node.gd")

const SAMPLE_RATE := 44100


# =========================================================================
# PROFILE FOR
# =========================================================================
## The engine's own profile for a struck thing.
static func profile_for(composition: Dictionary, geometry: Dictionary,
		wetness: float = 0.0) -> Dictionary:
	return Acoustic.profile(composition, geometry, wetness)


# =========================================================================
# RENDER STRIKE
# =========================================================================
## Render one strike of a profile: every mode rings at its own frequency
## and dies at its own decay, summed and peak-normalised. Deterministic:
## the same profile renders the same frames forever.
static func render_strike(profile: Dictionary, seconds: float = 1.2) -> PackedFloat32Array:
	var frames := PackedFloat32Array()
	var count := int(seconds * SAMPLE_RATE)
	if count <= 0:
		return frames
	frames.resize(count)
	var modes: Array = profile.get("modes", [])
	if modes.is_empty():
		return frames
	var peak := 0.0
	for i in count:
		var t := float(i) / float(SAMPLE_RATE)
		var v := 0.0
		for m in modes:
			var mode := m as Dictionary
			var f := float(mode.get("f", 0.0))
			if f <= 0.0 or f >= SAMPLE_RATE * 0.5:
				continue
			var tau := maxf(float(mode.get("tau", 0.01)), 1.0e-4)
			v += sin(TAU * f * t) * exp(-t / tau)
		frames[i] = v
		peak = maxf(peak, absf(v))
	if peak > 0.0:
		for i in count:
			frames[i] /= peak
	return frames


# =========================================================================
# RMS
# =========================================================================
## Root-mean-square of a slice, which is how the drill hears decay.
static func rms(frames: PackedFloat32Array, from: int, to: int) -> float:
	var n := mini(to, frames.size()) - from
	if n <= 0:
		return 0.0
	var acc := 0.0
	for i in range(from, from + n):
		acc += frames[i] * frames[i]
	return sqrt(acc / float(n))
