# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: engine_ambience_source.gd
#  script_path: scripts/core/engine_ambience_source.gd
#  module_name: engine_ambience_source
#  version: 0.1.0
#  author: [BantedHam]
#  description: The ambient bed, derived: the soundscape node's own wind
#           noise law turns the weather source's wind into decibels, and
#           the bed renders as filtered noise at that level. Wind picks
#           up and the world audibly picks up with it; nothing plays
#           from a file.
#  kind: module
#  spec: soundscape_node wind noise over weather_node wind
#  internal_dependencies: [addons/2.5D_engine/evaluator/soundscape_node.gd,
#           scripts/core/engine_weather_source.gd]
#  external_dependencies: [Godot 4.x]
#  features: [derived-ambience, wind-bed]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## What the air sounds like here, now.
@tool
extends RefCounted

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const Soundscape := preload("res://addons/2.5D_engine/evaluator/soundscape_node.gd")
const EngineWeatherSource := preload("res://scripts/core/engine_weather_source.gd")

const SAMPLE_RATE := 44100


## Where full gain sits on the node's decibel scale.
const REF_MAX_DB := 70.0


# =========================================================================
# WIND DB
# =========================================================================
## The wind's own level at a spot and moment, in decibels, from the
## soundscape node's law. Cover is VEGETATION: leaves are what a wind
## sounds like, so more cover is louder and bare rock is nearly silent,
## which is the node's law and the opposite of shelter.
static func wind_db(x: float, z: float, t: float, cover: float = 0.0) -> float:
	var w := EngineWeatherSource.conditions(x, z, t)
	var speed := (w["wind"] as Vector2).length()
	return Soundscape.wind_noise_db(speed, cover)


# =========================================================================
# RENDER BED
# =========================================================================
## A seconds-long bed of wind at that level: deterministic filtered noise
## whose gain is the derived decibels. The hash walk keeps it repeatable;
## the one-pole lowpass keeps it wind rather than hiss.
static func render_bed(db: float, seconds: float = 2.0, noise_seed: int = 7) -> PackedFloat32Array:
	var frames := PackedFloat32Array()
	var count := int(seconds * SAMPLE_RATE)
	if count <= 0:
		return frames
	frames.resize(count)
	# Decibels are a power law; gain is the intensity RELATIVE to the
	# ceiling, or every audible level saturates to one.
	var gain := clampf(Soundscape.db_to_intensity(db - REF_MAX_DB), 0.0, 1.0)
	var state := 0.0
	var h := noise_seed
	for i in count:
		h = int((h ^ (h >> 16)) * 0x45d9f3b) & 0x7FFFFFFF
		var white := float(h % 65536) / 32768.0 - 1.0
		state += 0.04 * (white - state)
		frames[i] = state * gain
	return frames
