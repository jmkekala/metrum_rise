# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: engine_sound_player.gd
#  script_path: scripts/core/engine_sound_player.gd
#  module_name: engine_sound_player
#  version: 0.1.0
#  author: [BantedHam]
#  description: The audible end of derived sound: a node any scene mounts
#           that strikes a composition and plays the rendered frames
#           through an AudioStreamGenerator. strike() takes what the
#           thing is made of and how it is shaped, and the room hears
#           the engine's acoustics.
#  kind: module
#  spec: none
#  internal_dependencies: [scripts/core/engine_sound_source.gd]
#  external_dependencies: [Godot 4.x]
#  features: [derived-sound-playback]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## Mount, call strike(), hear the material.
extends Node

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const SoundSource := preload("res://scripts/core/engine_sound_source.gd")

var _player: AudioStreamPlayer

# =========================================================================
# READY
# =========================================================================
func _ready() -> void:
	_player = AudioStreamPlayer.new()
	var stream := AudioStreamGenerator.new()
	stream.mix_rate = SoundSource.SAMPLE_RATE
	stream.buffer_length = 2.0
	_player.stream = stream
	add_child(_player)


# =========================================================================
# STRIKE
# =========================================================================
## Strike a thing: its composition and geometry become its voice.
func strike(composition: Dictionary, geometry: Dictionary, wetness: float = 0.0) -> void:
	var profile := SoundSource.profile_for(composition, geometry, wetness)
	var frames := SoundSource.render_strike(profile)
	if frames.is_empty():
		return
	_player.play()
	var playback := _player.get_stream_playback() as AudioStreamGeneratorPlayback
	if playback == null:
		return
	for i in mini(frames.size(), playback.get_frames_available()):
		playback.push_frame(Vector2(frames[i], frames[i]))
