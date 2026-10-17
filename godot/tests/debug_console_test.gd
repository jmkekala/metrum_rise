# SPDX-License-Identifier: GPL-2.0-only

## Debug channel bridge, shared log ordering, events capture and console controls regression.
extends SceneTree

const DebugConsole := preload("res://scripts/ui/debug_console/debug_console.gd")
const DebugEventLogger := preload("res://scripts/ui/debug_console/debug_event_logger.gd")
const GameSettings := preload("res://scripts/core/game_settings.gd")
# Rarely used channel, so the test owns its state.
const PROBE_CHANNEL := "isect"
var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

# Keep-clear HUD controls, handle drags, clamping and the size kept across restarts.
func _check_size(console: Node, viewport: Vector2) -> void:
	var menu_bar := Control.new()
	menu_bar.size = Vector2(viewport.x, 28.0)
	menu_bar.add_to_group(DebugConsole.KEEP_CLEAR_GROUP)
	var hud := Control.new()
	hud.position = Vector2(16.0, 600.0)
	hud.size = Vector2(500.0, 100.0)
	hud.add_to_group(DebugConsole.KEEP_CLEAR_GROUP)
	var beside := Control.new()
	beside.position = Vector2(viewport.x - 100.0, 380.0)
	beside.size = Vector2(80.0, 50.0)
	beside.add_to_group(DebugConsole.KEEP_CLEAR_GROUP)
	var layer := CanvasLayer.new()
	layer.add_child(menu_bar)
	layer.add_child(hud)
	layer.add_child(beside)
	root.add_child(layer)
	console._layout()
	var free_height := 600.0 - DebugConsole.KEEP_CLEAR_GAP_PX - 28.0
	_expect(console._view.position.y == 28.0, "the open console must start below the top menu bar, got %s" % console._view.position)
	_expect(console._view.size.y == free_height, "the console must stop above the HUD strip, got %s" % console._view.size)
	var width: float = console._view.size.x
	# A real drag on the bottom-right grip: press, move, release.
	var grip: Control = console._view.get_node("ResizeHandles/Resize3")
	var press := InputEventMouseButton.new()
	press.button_index = MOUSE_BUTTON_LEFT
	press.pressed = true
	grip.gui_input.emit(press)
	var motion := InputEventMouseMotion.new()
	motion.button_mask = MOUSE_BUTTON_MASK_LEFT
	motion.relative = Vector2(120.0, -200.0)
	grip.gui_input.emit(motion)
	var release := InputEventMouseButton.new()
	release.button_index = MOUSE_BUTTON_LEFT
	grip.gui_input.emit(release)
	var dragged := Vector2(width + 120.0, free_height - 200.0)
	_expect(console._view.size == dragged, "the grip must resize both axes, got %s" % console._view.size)
	var saved := GameSettings.load_window_layout(DebugConsole.LAYOUT_ID)
	_expect(saved["has_size"] and Vector2(saved["size"]) == dragged, "release must save the size, got %s" % saved)
	# The right edge moves only the width; growing past the HUD or the window is clamped.
	var edge: Control = console._view.get_node("ResizeHandles/Resize1")
	motion.relative = Vector2(5000.0, 5000.0)
	edge.gui_input.emit(motion)
	# Now as wide as the window, it also covers the strip at the right and stops above it.
	_expect(console._view.size == Vector2(viewport.x, 380.0 - DebugConsole.KEEP_CLEAR_GAP_PX - 28.0), "the right edge must clamp to the window and clear every HUD strip it covers, got %s" % console._view.size)
	console._view.resize_dragged.emit(Vector2(-10000.0, 10000.0))
	_expect(console._view.size == Vector2(DebugConsole.MIN_SIZE.x, free_height), "sizes must clamp to the minimum and the free height")
	console._view.resize_dragged.emit(dragged - console._view.size)
	var bottom: Control = console._view.get_node("ResizeHandles/Resize2")
	motion.relative = Vector2(500.0, -50.0)
	bottom.gui_input.emit(motion)
	_expect(console._view.size == dragged - Vector2(0.0, 50.0), "the bottom edge must change only the height, got %s" % console._view.size)
	console._view.resize_dragged.emit(Vector2(0.0, 50.0))
	console._view.resize_finished.emit()
	# A new console, as after a restart, restores the saved size.
	var restarted := DebugConsole.new()
	root.add_child(restarted)
	await process_frame
	_expect(restarted._view.size == dragged, "a restarted console must restore its size, got %s" % restarted._view.size)
	restarted.queue_free()
	layer.queue_free()
	await process_frame
	console._layout()

func _messages_since(seq: int, channel: String) -> PackedStringArray:
	var batch: Dictionary = MetrumDebug.read_since(seq, 4096)
	var out := PackedStringArray()
	var channels: PackedStringArray = batch["channel"]
	var messages: PackedStringArray = batch["message"]
	for i in channels.size():
		if channels[i] == channel:
			out.append(messages[i])
	return out

func _run() -> void:
	var names := MetrumDebug.channel_names()
	_expect(names.has("road") and names.has("events") and names.has("world-editor"), "registry must list the existing tags")
	_expect(MetrumDebug.is_locked("events") and MetrumDebug.is_enabled("events"), "events must be locked on")
	_expect(not MetrumDebug.set_enabled("events", false) and MetrumDebug.is_enabled("events"), "events must refuse switching off")
	_expect(not MetrumDebug.set_enabled("no-such-channel", true), "unknown channels must be rejected")

	var start: int = MetrumDebug.read_since(0, 0)["last_seq"]
	MetrumDebug.set_enabled(PROBE_CHANNEL, false)
	_expect(not MetrumDebug.log(PROBE_CHANNEL, "dropped"), "a disabled channel must drop GDScript lines")
	_expect(MetrumDebug.set_enabled(PROBE_CHANNEL, true), "a runtime toggle must succeed")
	_expect(MetrumDebug.log(PROBE_CHANNEL, "kept"), "an enabled channel must keep GDScript lines")
	var batch: Dictionary = MetrumDebug.read_since(start, 4096)
	var kept := _messages_since(start, PROBE_CHANNEL)
	_expect(kept == PackedStringArray(["kept"]), "only the enabled line may be buffered")
	_expect(Array(batch["source"]).has("gd"), "GDScript lines must be tagged gd")

	# `--script` runs load autoloads, so the console's logger may already be installed.
	var logger: Logger = null
	if root.get_node_or_null("DebugConsole") == null:
		logger = DebugEventLogger.new()
		OS.add_logger(logger)
	var before_warning: int = MetrumDebug.read_since(0, 0)["last_seq"]
	push_warning("debug-console-test-warning")
	var events := _messages_since(before_warning, "events")
	_expect(events.size() == 1 and events[0].contains("debug-console-test-warning"), "a warning must reach the events channel once")
	_expect(events.size() == 1 and events[0].contains("debug_console_test.gd"), "events must name the script call site")
	if logger != null:
		OS.remove_logger(logger)

	# Headless starts with a 64 px window; use a realistic one for the layout check.
	root.size = Vector2i(1280, 720)
	var console := DebugConsole.new()
	root.add_child(console)
	await process_frame
	_expect(not console.is_open(), "the console starts closed")
	var key := InputEventKey.new()
	key.pressed = true
	key.physical_keycode = KEY_QUOTELEFT
	console._input(key)
	_expect(console.is_open(), "the key left of 1 must open the console")
	var iso_mac_keys := [[KEY_SECTION, KEY_SECTION, true], [KEY_QUOTELEFT, KEY_LESS, false]]
	for entry in iso_mac_keys:
		var iso_key := InputEventKey.new()
		iso_key.physical_keycode = entry[0]
		iso_key.key_label = entry[1]
		_expect(DebugConsole._is_toggle_key(iso_key) == entry[2], "on a macOS ISO keyboard § toggles the console and < types (%s)" % OS.get_keycode_string(entry[0]))
	await process_frame
	var visible_size := root.get_visible_rect().size
	_expect(console._view.size.x <= visible_size.x / 3.0 + 0.5, "the console must use at most a third of the width")
	_expect(console._view.size.y == visible_size.y and console._view.position.x == 0.0, "without HUD strips the console must be full height on the left")
	await _check_size(console, visible_size)
	console._run_command("channel %s off" % PROBE_CHANNEL)
	_expect(not MetrumDebug.is_enabled(PROBE_CHANNEL), "the channel command must switch channels")
	# A stand-in gameplay scene, so Dump also saves the running game.
	var scene := Node3D.new()
	var simulation := SimulationNode.new()
	simulation.name = "SimulationNode"
	scene.add_child(simulation)
	root.add_child(scene)
	current_scene = scene
	await process_frame
	console._run_command("dump")
	var dumps := Array(DirAccess.get_files_at("user://debug")).filter(func(name: String) -> bool: return name.ends_with(".txt"))
	_expect(dumps.size() == 1 and dumps[0].begins_with("dump-"), "dump must write one text file under user://debug")
	var saves := Array(DirAccess.get_files_at("user://debug")).filter(func(name: String) -> bool: return name.ends_with(".sqlite"))
	_expect(saves.size() == 1 and dumps.size() == 1 and saves[0] == dumps[0].replace(".txt", ".sqlite"), "a gameplay dump must save the game beside the text dump")
	if DisplayServer.get_name() != "headless":
		await RenderingServer.frame_post_draw
		await process_frame
		var shots := Array(DirAccess.get_files_at("user://debug")).filter(func(name: String) -> bool: return name.ends_with(".png"))
		_expect(shots.size() == 1 and shots[0] == dumps[0].replace(".txt", ".png"), "a rendered dump must save a matching screenshot")
		_expect(console._view.visible, "the console must reappear after the screenshot")
	if dumps.size() == 1:
		var text := FileAccess.get_file_as_string("user://debug/" + dumps[0])
		_expect(text.contains("%-18s off" % PROBE_CHANNEL) and text.contains("[DEBUG:%s] kept" % PROBE_CHANNEL), "dump must hold channel state and buffered lines")
	console._input(key)
	_expect(not console.is_open(), "the key must close the console again")
	console.queue_free()
	scene.queue_free()
	await process_frame
	quit(1 if _failures > 0 else 0)
