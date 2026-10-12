# SPDX-License-Identifier: GPL-2.0-only

## In-game debug console: a left-anchored drop-down over every scene, toggled by the key left
## of "1" (§ on Nordic layouts, ` on US layouts).
##
## An autoload controller over `MetrumDebug`, which owns channel state and the shared log in
## Rust. While open it reads new log lines each frame, refreshes the Perf tab twice a second and
## runs console commands; while closed it does no work beyond watching for its key. The events
## logger is installed once here so warnings and errors are kept even with the console closed.
extends CanvasLayer

const DebugConsoleView := preload("res://scripts/ui/debug_console/debug_console_view.gd")
const DebugEventLogger := preload("res://scripts/ui/debug_console/debug_event_logger.gd")

const SLIDE_TIME_S := 0.12
const PERF_REFRESH_S := 0.5
# Lines read from Rust per frame; a burst larger than this catches up over the next frames.
const READ_BATCH := 512
# Lines kept for re-filtering, matching the Rust ring capacity.
const MAX_MODEL_LINES := 4096
const DUMP_DIR := "user://debug"

const HELP_TEXT := "help | channel | channel <name> on|off | dump | clear"

var _view: DebugConsoleView
var _open := false
var _tween: Tween
var _last_seq := 0
var _lines: Array[Dictionary] = []
var _hidden_channels := {}
var _filter_text := ""
var _perf_elapsed_s := 0.0
var _event_logger: Logger

func _ready() -> void:
	layer = 120
	process_mode = Node.PROCESS_MODE_ALWAYS
	_view = DebugConsoleView.new()
	_view.visible = false
	add_child(_view)
	var locked := {}
	for channel in MetrumDebug.channel_names():
		locked[channel] = MetrumDebug.is_locked(channel)
	_view.set_channels(MetrumDebug.channel_names(), locked)
	_view.set_startup_modes(MetrumDebug.startup_modes())
	_view.channel_toggled.connect(_on_channel_toggled)
	_view.chip_toggled.connect(_on_chip_toggled)
	_view.filter_changed.connect(_on_filter_changed)
	_view.command_submitted.connect(_run_command)
	_view.copy_pressed.connect(_copy_visible)
	_view.dump_pressed.connect(_dump)
	_view.clear_pressed.connect(_clear)
	_view.tab_changed.connect(func(_tab: int) -> void: _perf_elapsed_s = PERF_REFRESH_S)
	get_viewport().size_changed.connect(_layout)
	_layout()
	set_process(false)
	_event_logger = DebugEventLogger.new()
	OS.add_logger(_event_logger)

func _exit_tree() -> void:
	if _event_logger != null:
		OS.remove_logger(_event_logger)

func _input(event: InputEvent) -> void:
	var key := event as InputEventKey
	if key == null or not key.pressed:
		return
	if key.physical_keycode == KEY_QUOTELEFT:
		# Consumed even on echo so the key never types into a text field.
		get_viewport().set_input_as_handled()
		if not key.echo:
			set_open(not _open)
	elif key.keycode == KEY_ESCAPE and _open and _view.has_text_focus():
		_view.release_text_focus()
		get_viewport().set_input_as_handled()

## Opens or closes the console with a short slide from the top edge.
func set_open(open: bool) -> void:
	if open == _open:
		return
	_open = open
	if _tween != null:
		_tween.kill()
	_tween = create_tween()
	var height := get_viewport().get_visible_rect().size.y
	if open:
		_sync_channel_states()
		_perf_elapsed_s = PERF_REFRESH_S
		_view.visible = true
		_view.position.y = -height
		_tween.tween_property(_view, "position:y", 0.0, SLIDE_TIME_S)
		set_process(true)
	else:
		_view.release_text_focus()
		_tween.tween_property(_view, "position:y", -height, SLIDE_TIME_S)
		_tween.tween_callback(func() -> void: _view.visible = false)
		set_process(false)

## Returns `true` while the console is shown.
func is_open() -> bool:
	return _open

func _process(delta: float) -> void:
	_read_new_lines()
	_perf_elapsed_s += delta
	if _perf_elapsed_s >= PERF_REFRESH_S:
		_perf_elapsed_s = 0.0
		_sync_channel_states()
		if _view.current_tab() == DebugConsoleView.TAB_PERF:
			_view.set_perf_text(_perf_text())

func _layout() -> void:
	var viewport_size := get_viewport().get_visible_rect().size
	_view.position = Vector2(0.0, 0.0 if _open else -viewport_size.y)
	_view.size = Vector2(floorf(viewport_size.x / 3.0), viewport_size.y)

func _read_new_lines() -> void:
	var batch: Dictionary = MetrumDebug.read_since(_last_seq, READ_BATCH)
	var seqs: PackedInt64Array = batch["seq"]
	if seqs.is_empty():
		return
	var times: PackedInt64Array = batch["time_ms"]
	var channels: PackedStringArray = batch["channel"]
	var sources: PackedStringArray = batch["source"]
	var messages: PackedStringArray = batch["message"]
	for i in seqs.size():
		var line := {
			"seq": seqs[i], "time_ms": times[i], "channel": channels[i],
			"source": sources[i], "message": messages[i],
		}
		_lines.append(line)
		if _is_visible(line):
			_view.append_line(line.time_ms, line.channel, line.message)
	_last_seq = seqs[seqs.size() - 1]
	if _lines.size() > MAX_MODEL_LINES:
		_lines = _lines.slice(_lines.size() - MAX_MODEL_LINES)

func _is_visible(line: Dictionary) -> bool:
	if _hidden_channels.has(line.channel):
		return false
	if _filter_text.is_empty():
		return true
	return line.message.containsn(_filter_text) or line.channel.containsn(_filter_text)

func _rebuild_view() -> void:
	_view.clear_log()
	for line in _lines:
		if _is_visible(line):
			_view.append_line(line.time_ms, line.channel, line.message)

func _sync_channel_states() -> void:
	for channel in MetrumDebug.channel_names():
		_view.set_channel_state(channel, MetrumDebug.is_enabled(channel))

func _on_channel_toggled(channel: String, on: bool) -> void:
	MetrumDebug.set_enabled(channel, on)
	_view.set_channel_state(channel, MetrumDebug.is_enabled(channel))
	_view.set_status("%s %s" % [channel, "on" if MetrumDebug.is_enabled(channel) else "off"])

func _on_chip_toggled(channel: String, shown: bool) -> void:
	if shown:
		_hidden_channels.erase(channel)
	else:
		_hidden_channels[channel] = true
	_rebuild_view()

func _on_filter_changed(text: String) -> void:
	_filter_text = text.strip_edges()
	_rebuild_view()

func _run_command(text: String) -> void:
	var words := text.strip_edges().split(" ", false)
	if words.is_empty():
		return
	match words[0].to_lower():
		"help":
			_view.set_status(HELP_TEXT)
		"channel", "ch":
			_channel_command(words)
		"dump":
			_dump()
		"clear":
			_clear()
		_:
			_view.set_status("Unknown command '%s'. %s" % [words[0], HELP_TEXT])

func _channel_command(words: PackedStringArray) -> void:
	if words.size() == 1:
		var states := PackedStringArray()
		for channel in MetrumDebug.channel_names():
			states.append("%s=%s" % [channel, "on" if MetrumDebug.is_enabled(channel) else "off"])
		_view.set_status(" ".join(states))
		return
	var channel := words[1].to_lower()
	if not MetrumDebug.channel_names().has(channel):
		_view.set_status("Unknown channel '%s'." % channel)
		return
	if words.size() < 3 or not words[2].to_lower() in ["on", "off"]:
		_view.set_status("Usage: channel %s on|off" % channel)
		return
	var on := words[2].to_lower() == "on"
	if not MetrumDebug.set_enabled(channel, on):
		_view.set_status("%s cannot be switched off." % channel)
		return
	_view.set_channel_state(channel, on)
	_view.set_status("%s %s" % [channel, words[2].to_lower()])

func _copy_visible() -> void:
	var text := PackedStringArray()
	for line in _lines:
		if _is_visible(line):
			text.append("%7.3f %s [DEBUG:%s] %s" % [line.time_ms / 1000.0, line.source, line.channel, line.message])
	DisplayServer.clipboard_set("\n".join(text))
	_view.set_status("Copied %d lines." % text.size())

func _dump() -> void:
	var stamp := Time.get_datetime_string_from_system(false, true).replace(":", "-").replace(" ", "_")
	var base := "%s/dump-%s" % [DUMP_DIR, stamp]
	var path: String = MetrumDebug.write_dump(base + ".txt")
	if path.is_empty():
		_view.set_status("Dump failed; see the events channel.")
		return
	var lines := PackedStringArray(["Dump written to %s" % path])
	var save := _save_game(base + ".sqlite")
	if not save.is_empty():
		lines.append("Save %s" % save)
	_view.set_status("\n".join(lines))
	var shot := await _save_screenshot(base + ".png")
	if not shot.is_empty():
		lines.append("Screenshot %s" % shot)
		_view.set_status("\n".join(lines))

## Saves the running game, with its camera, as an ordinary save file beside the text dump so the
## same state can be loaded later. Returns the absolute path, or "" outside gameplay (editors
## and menus have no game to save) or when the save fails.
func _save_game(path: String) -> String:
	var sim := _simulation_node()
	if sim == null or sim.is_asset_editor_mode() or sim.is_world_editor_mode():
		return ""
	var absolute := ProjectSettings.globalize_path(path)
	# The save goes through the normal save path, which reports its own failures to events.
	return absolute if sim.save_game(absolute) else ""

## Saves the game view without the console as a PNG beside the text dump. Returns the absolute
## path, or "" where no rendered image exists (headless runs).
func _save_screenshot(path: String) -> String:
	if DisplayServer.get_name() == "headless":
		return ""
	# Hide the console for one drawn frame so the image shows the world it covers.
	_view.visible = false
	await RenderingServer.frame_post_draw
	var image := get_viewport().get_texture().get_image()
	_view.visible = _open
	if image == null or image.save_png(path) != OK:
		push_warning("Debug screenshot to %s failed" % path)
		return ""
	return ProjectSettings.globalize_path(path)

func _clear() -> void:
	MetrumDebug.clear()
	_lines.clear()
	_view.clear_log()
	_view.set_status("Log cleared.")

func _perf_text() -> String:
	var fps := Engine.get_frames_per_second()
	var text := "Frame: %.0f FPS  %.1f ms\n" % [fps, 1000.0 / maxf(fps, 1.0)]
	var sim := _simulation_node()
	if sim != null:
		var stats: Dictionary = sim.get_perf_stats()
		text += "Simulation tick: %.2f ms\nAgents: %d\nPathfind calls: %d\n" % [
			float(stats.get("last_tick_ms", 0.0)),
			int(stats.get("agent_count", 0)),
			int(stats.get("pathfind_calls", 0)),
		]
	else:
		text += "No simulation in this scene.\n"
	text += "\nRenderer CPU (perf channel):\n"
	if not MetrumDebug.is_enabled("perf"):
		text += "Turn on the perf channel to collect renderer summaries."
	elif PerfDebug.last_summary.is_empty():
		text += "Waiting for the first summary."
	else:
		text += PerfDebug.last_summary.replace(" ", "\n")
	return text

func _simulation_node() -> Node:
	var scene := get_tree().current_scene
	return scene.get_node_or_null("SimulationNode") if scene != null else null
