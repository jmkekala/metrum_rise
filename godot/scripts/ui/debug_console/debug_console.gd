# SPDX-License-Identifier: GPL-2.0-only

## In-game debug console: a left-anchored drop-down over every scene, toggled by the key left
## of "1" (§ on Nordic layouts, ` on US layouts).
##
## An autoload controller over `MetrumDebug`, which owns channel state, visual modes and the
## shared log in Rust. While open it reads new log lines each frame, refreshes the Perf tab twice
## a second and runs console commands; while closed it does no work beyond watching for its key.
## The events logger is installed once here so warnings and errors are kept even with the console
## closed. The panel's right and bottom edges resize it; its size is kept in the settings file
## across restarts. It stays between the HUD controls in `KEEP_CLEAR_GROUP`: below the top menu
## bar and above the bottom strips. Visual mode switches go through `set_visual_mode`, which applies shader globals and
## tells renderers through `visual_mode_changed`, so nothing polls for them per frame.
extends CanvasLayer

## Emitted after a visual debug mode takes a new value. Renderers that cache a mode re-read it.
signal visual_mode_changed(mode: String)

const DebugConsoleView := preload("res://scripts/ui/debug_console/debug_console_view.gd")
const DebugEventLogger := preload("res://scripts/ui/debug_console/debug_event_logger.gd")
const GameSettings := preload("res://scripts/core/game_settings.gd")

## HUD controls the console must not cover: the top menu bar and the gameplay clock, money and
## RCI boxes. Controls in the top half of the window push the console's top edge down; the rest
## pull its bottom edge up.
const KEEP_CLEAR_GROUP := "debug_console_keep_clear"
## Settings layout id under which the chosen size persists.
const LAYOUT_ID := "debug_console"
const MIN_SIZE := Vector2(360.0, 240.0)
# Space left between the console's bottom edge and a keep-clear control.
const KEEP_CLEAR_GAP_PX := 8.0

const SLIDE_TIME_S := 0.12
const PERF_REFRESH_S := 0.5
# Lines read from Rust per frame; a burst larger than this catches up over the next frames.
const READ_BATCH := 512
# Lines kept for re-filtering, matching the Rust ring capacity.
const MAX_MODEL_LINES := 4096
const DUMP_DIR := "user://debug"

const HELP_TEXT := "help | channel | channel <name> on|off | visual | visual <mode> <value> | visual off | parking | dump | clear"
# Visual modes read by shaders as global uniforms; the mode index is the shader's mode number.
const SHADER_GLOBAL_MODES := {
	"terrain_visual": "terrain_visual_debug_mode",
	"terrain_grass": "terrain_grass_visual_debug_mode",
}

var _view: DebugConsoleView
var _open := false
var _tween: Tween
var _last_seq := 0
var _lines: Array[Dictionary] = []
var _hidden_channels := {}
var _filter_text := ""
var _perf_elapsed_s := 0.0
var _event_logger: Logger
# The size the player chose by dragging; zero means the default (a third of the width, all the
# height the HUD leaves free).
var _chosen_size := Vector2.ZERO
# Where the open console's top edge sits, below any keep-clear control at the top.
var _top := 0.0

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
	var visual_values := {}
	for mode in MetrumDebug.visual_mode_names():
		visual_values[mode] = MetrumDebug.visual_mode_values(mode)
	_view.set_visual_modes(MetrumDebug.visual_mode_names(), visual_values)
	# Launch values reach the shaders here, before the game scene builds its materials.
	for mode in SHADER_GLOBAL_MODES:
		_apply_shader_global(mode)
	_view.channel_toggled.connect(_on_channel_toggled)
	_view.visual_mode_selected.connect(_on_visual_mode_selected)
	_view.chip_toggled.connect(_on_chip_toggled)
	_view.filter_changed.connect(_on_filter_changed)
	_view.command_submitted.connect(_run_command)
	_view.copy_pressed.connect(_copy_visible)
	_view.dump_pressed.connect(_dump)
	_view.clear_pressed.connect(_clear)
	_view.tab_changed.connect(func(_tab: int) -> void: _perf_elapsed_s = PERF_REFRESH_S)
	_view.resize_dragged.connect(_on_resize_dragged)
	_view.resize_finished.connect(_save_size)
	var saved := GameSettings.load_window_layout(LAYOUT_ID)
	if saved["has_size"]:
		_chosen_size = Vector2(saved["size"])
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
	if _is_toggle_key(key):
		# Consumed even on echo so the key never types into a text field.
		get_viewport().set_input_as_handled()
		if not key.echo:
			set_open(not _open)
	elif key.keycode == KEY_ESCAPE and _open and _view.has_text_focus():
		_view.release_text_focus()
		get_viewport().set_input_as_handled()

# The key left of 1. macOS reports it as § on ISO keyboards (the ` code there belongs to the <
# key beside left shift, which must keep typing) and as ` on ANSI ones.
static func _is_toggle_key(key: InputEventKey) -> bool:
	if key.physical_keycode == KEY_SECTION:
		return true
	return key.physical_keycode == KEY_QUOTELEFT and key.key_label != KEY_LESS

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
		# The HUD may have changed since the console last showed: a new scene or UI scale.
		_layout()
		_sync_channel_states()
		_sync_visual_states()
		_perf_elapsed_s = PERF_REFRESH_S
		_view.visible = true
		_view.position.y = -height
		_tween.tween_property(_view, "position:y", _top, SLIDE_TIME_S)
		set_process(true)
	else:
		_view.release_text_focus()
		_tween.tween_property(_view, "position:y", -height, SLIDE_TIME_S)
		_tween.tween_callback(func() -> void: _view.visible = false)
		set_process(false)

## Returns `true` while the console is shown.
func is_open() -> bool:
	return _open

## Switches a visual debug mode by value name, alias or index, then applies it: shader globals
## directly, everything else through `visual_mode_changed`. Returns `false` for an unknown mode
## or value, whose reason Rust logs on `events`. Selecting the current value changes nothing.
func set_visual_mode(mode: String, value: String) -> bool:
	var before := MetrumDebug.visual_mode_index(mode)
	if not MetrumDebug.set_visual_mode(mode, value):
		return false
	_view.set_visual_mode_state(mode, MetrumDebug.visual_mode(mode))
	if MetrumDebug.visual_mode_index(mode) == before:
		return true
	if SHADER_GLOBAL_MODES.has(mode):
		_apply_shader_global(mode)
	visual_mode_changed.emit(mode)
	return true

func _apply_shader_global(mode: String) -> void:
	RenderingServer.global_shader_parameter_set(SHADER_GLOBAL_MODES[mode], MetrumDebug.visual_mode_index(mode))

func _process(delta: float) -> void:
	_read_new_lines()
	_perf_elapsed_s += delta
	if _perf_elapsed_s >= PERF_REFRESH_S:
		_perf_elapsed_s = 0.0
		_layout()
		_sync_channel_states()
		if _view.current_tab() == DebugConsoleView.TAB_PERF:
			_view.set_perf_text(_perf_text())

func _layout() -> void:
	var viewport_size := get_viewport().get_visible_rect().size
	var wanted := _chosen_size if _chosen_size != Vector2.ZERO else Vector2(floorf(viewport_size.x / 3.0), INF)
	var width := clampf(wanted.x, minf(MIN_SIZE.x, viewport_size.x), viewport_size.x)
	var span := _free_span(width, viewport_size.y)
	_top = span.x
	var free_height := span.y - span.x
	var height := clampf(wanted.y, minf(MIN_SIZE.y, free_height), free_height)
	_view.size = Vector2(width, height)
	_view.position = Vector2(0.0, _top if _open else -viewport_size.y)

# The free vertical span (top, bottom) between the visible keep-clear controls the console's
# columns overlap: it starts right below the top ones and ends a gap above the bottom ones.
# O(group members); the group holds a few HUD bars and strips.
func _free_span(width: float, viewport_height: float) -> Vector2:
	var top := 0.0
	var bottom := viewport_height
	for node in get_tree().get_nodes_in_group(KEEP_CLEAR_GROUP):
		var control := node as Control
		if control == null or not control.is_visible_in_tree():
			continue
		var rect := control.get_global_rect()
		if rect.size.y <= 0.0 or rect.position.x >= width or rect.end.x <= 0.0:
			continue
		if rect.get_center().y < viewport_height * 0.5:
			top = maxf(top, rect.end.y)
		else:
			bottom = minf(bottom, rect.position.y - KEEP_CLEAR_GAP_PX)
	return Vector2(top, maxf(bottom, top))

# Dragging grows or shrinks the chosen size; the layout clamps what is shown.
func _on_resize_dragged(delta: Vector2) -> void:
	_chosen_size = _view.size + delta
	_layout()

func _save_size() -> void:
	_chosen_size = _view.size
	GameSettings.save_window_layout(LAYOUT_ID, Vector2i(_chosen_size.round()), Vector2i.ZERO, false)

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

func _sync_visual_states() -> void:
	for mode in MetrumDebug.visual_mode_names():
		_view.set_visual_mode_state(mode, MetrumDebug.visual_mode(mode))

func _on_visual_mode_selected(mode: String, value: String) -> void:
	set_visual_mode(mode, value)
	_view.set_status("%s = %s" % [mode, MetrumDebug.visual_mode(mode)])

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
		"visual", "vis":
			_visual_command(words)
		"parking", "park":
			_parking_command()
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

func _visual_command(words: PackedStringArray) -> void:
	var modes := MetrumDebug.visual_mode_names()
	if words.size() == 1:
		var states := PackedStringArray()
		for mode in modes:
			states.append("%s=%s" % [mode, MetrumDebug.visual_mode(mode)])
		_view.set_status(" ".join(states))
		return
	var mode := words[1].to_lower()
	if words.size() == 2 and mode == "off":
		for each in modes:
			set_visual_mode(each, "off")
		_view.set_status("Visual modes off.")
		return
	if not modes.has(mode):
		_view.set_status("Unknown visual mode '%s'. Modes: %s" % [mode, ", ".join(modes)])
		return
	var values := " | ".join(MetrumDebug.visual_mode_values(mode))
	if words.size() < 3:
		_view.set_status("Usage: visual %s %s" % [mode, values])
		return
	if not set_visual_mode(mode, words[2]):
		_view.set_status("Unknown value '%s'. Usage: visual %s %s" % [words[2], mode, values])
		return
	_view.set_status("%s = %s" % [mode, MetrumDebug.visual_mode(mode)])

## Prints parking supply, car custody, live trip phases, why departures drove or walked, and
## every rejected parking spot with its building and reason.
func _parking_command() -> void:
	var sim := _simulation_node()
	if sim == null or sim.is_asset_editor_mode() or sim.is_world_editor_mode():
		_view.set_status("Parking info needs a running game.")
		return
	var info: Dictionary = sim.get_parking_info()
	var lines := PackedStringArray([
		"Parking spots: %d" % int(info.get("places", 0)),
		"Cars: %d  parked %d  driving %d  stored off-map %d  in use %d  with a reserved spot %d" % [
			int(info.get("vehicles", 0)), int(info.get("parked", 0)), int(info.get("moving", 0)),
			int(info.get("stored", 0)), int(info.get("borrowed", 0)), int(info.get("reserved", 0))],
		"Resident cars inside buildings (must be 0): %d" % int(info.get("resident_cars_in_buildings", 0)),
	])
	var phases: Dictionary = info.get("phases", {})
	var phase_text := PackedStringArray()
	for phase in phases.keys():
		if str(phase) != "None":
			phase_text.append("%s %d" % [phase, int(phases[phase])])
	lines.append("Trips under way: " + ("none" if phase_text.is_empty() else ", ".join(phase_text)))
	lines.append("Departure attempts since load:")
	var departures: Dictionary = info.get("departures", {})
	for label in departures.keys():
		if int(departures[label]) > 0:
			lines.append("  %s: %d" % [label, int(departures[label])])
	var rejected: PackedStringArray = info.get("diagnostics", PackedStringArray())
	lines.append("Rejected parking spots: %d" % rejected.size())
	for issue in rejected:
		lines.append("  " + issue)
	var text := "\n".join(lines)
	_view.set_status(text)
	MetrumDebug.log("events", text)

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
	# Save first and log which road network it holds, so the text dump names its save and a
	# replay can tell which `road_attempt` lines were made on that network.
	var save := _save_game(base + ".sqlite")
	if not save.is_empty():
		MetrumDebug.log("events", "dump save=%s road_generation=%d" % [save, _simulation_node().get_road_tool_surface_generation()])
	var path: String = MetrumDebug.write_dump(base + ".txt")
	if path.is_empty():
		_view.set_status("Dump failed; see the events channel.")
		return
	var lines := PackedStringArray(["Dump written to %s" % path])
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
