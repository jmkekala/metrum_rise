# SPDX-License-Identifier: GPL-3.0-or-later

## The loading screen (`UI-15`), headless, scripted player's loads of a 20 km blank world's save
## through the real menus and scenes. Checked each frame of every load, as state (what the frame
## drew): from the click until the hand-over the screen is up, visible, opaque, as large as the
## viewport and above every other canvas layer, with no dialog window over it, and from the
## hand-over's frame on it is gone (the world is first shown at the hand-over); the fraction the
## load flow shows starts at 0, never decreases, stays below 1 until the hand-over and is 1 from
## it, and the screen shows that fraction; the stream node's units read `units_done ==
## units_total - 1` exactly when its preload is `done`. While the screen is up no key or mouse
## button reaches a scene node or a HUD control (probes that do receive them after the hand-over).
## Cancel by Esc in the view phase and by a click on Cancel in the fill phase returns to the main
## menu with the screen gone and nothing left of the world scene (the nodes outside the current
## scene as before the load), and a load after them hands over. A refused save in game closes the
## screen with Rust's message and the city as it was; refused from the main menu, the menu shows
## the message and no screen is left.
extends SceneTree

const LoadingScreen := preload("res://scripts/ui/loading_screen.gd")
const TIMEOUT_MS := 120000
const DIR := "user://loading_screen_test"
const MAIN_SCENE := "res://scenes/Main.tscn"
const MENU_SCENE := "res://scenes/MainMenu.tscn"

# Counts the keys and mouse buttons that reach a scene node (`_input`).
class InputProbe extends Node:
	var events := 0
	func _input(event: InputEvent) -> void:
		if event is InputEventKey or event is InputEventMouseButton:
			events += 1

var _failures := 0
var _save_path := ""
var _bad_path := ""
# What a load seen frame by frame found (`_watch_load`).
var _seen := {}

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	await _write_saves()
	var baseline := await _menu_nodes_outside()
	# A load from the main menu, watched; input probed under the screen and after it.
	await _load_from_menu(_save_path)
	await _watch_load("menu load", true)
	_expect(bool(_seen.get("handed_over", false)), "the menu's load hands over")
	# In game: a load cancelled by Esc in its view phase.
	var input: Node = current_scene.get_node("InputManager")
	_start_player_load(input, _save_path)
	await _watch_load("Esc in the view phase", false, 1, "esc")
	await _expect_back_at_menu("Esc in the view phase", baseline)
	# From the menu: cancelled by a click on Cancel in the fill phase.
	await _load_from_menu(_save_path)
	await _watch_load("Cancel in the fill phase", false, 2, "click")
	await _expect_back_at_menu("Cancel in the fill phase", baseline)
	# A load after the cancels hands over.
	await _load_from_menu(_save_path)
	await _watch_load("a load after the cancels", false)
	_expect(bool(_seen.get("handed_over", false)), "a load after the cancels hands over")
	await _test_refused_in_game()
	await _test_refused_from_menu()
	print("Loading screen tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)

# A save of a 20 km blank world, and a file that is not a save.
func _write_saves() -> void:
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(DIR))
	_save_path = ProjectSettings.globalize_path(DIR.path_join("blank_city.sqlite"))
	_bad_path = ProjectSettings.globalize_path(DIR.path_join("not_a_save.sqlite"))
	var main: Node = load(MAIN_SCENE).instantiate()
	main.set_script(null)
	root.add_child(main)
	await process_frame
	var sim: Node = main.get_node("SimulationNode")
	_expect(sim.create_blank_world(20480.0, 20480.0, 8.0, 0.0), "the blank world must load")
	_expect(sim.save_game(_save_path), "the blank world must save")
	main.queue_free()
	var bad := FileAccess.open(_bad_path, FileAccess.WRITE)
	bad.store_string("not a save")
	bad.close()
	await process_frame

# The main menu shown, a few frames in; the nodes outside it (the autoloads and anything left
# over) as the baseline a cancelled load must return to.
func _menu_nodes_outside() -> int:
	change_scene_to_file(MENU_SCENE)
	for frame in 4:
		await process_frame
	return _nodes_outside_scene()

func _nodes_outside_scene() -> int:
	var inside := _subtree(current_scene) if current_scene != null else 0
	return int(Performance.get_monitor(Performance.OBJECT_NODE_COUNT)) - inside

static func _subtree(node: Node) -> int:
	var count := 1
	for child in node.get_children(true):
		count += _subtree(child)
	return count

# Load Game's choice in the main menu (the browser's callback), as a player's click.
func _load_from_menu(path: String) -> void:
	if current_scene == null or current_scene.scene_file_path != MENU_SCENE:
		change_scene_to_file(MENU_SCENE)
		for frame in 4:
			await process_frame
	current_scene._finish_load_game_selection(path)

# An in-game player's load (Load in the menu bar), not awaited.
func _start_player_load(input: Node, path: String) -> void:
	await input.player_load(input.LoadKind.SAVE, path)

# Frames of a load from the click until the hand-over (or back at the menu), each checked as
# described above. `probe`: input sent under the screen and after it. `cancel_phase` (1 view, 2
# fill): cancel by `how` ("esc" or "click") in the first frame of that phase.
func _watch_load(what: String, probe: bool, cancel_phase := 0, how := "") -> void:
	_seen = {"handed_over": false, "covered_frames": 0, "cancelled": false}
	var start := Time.get_ticks_msec()
	var last_fraction := 0.0
	var probes: Array = []
	var screen_seen := false
	# The load call ran (the world not ready since): before it, in game, the screen covers the
	# running city for its first frame.
	var called := false
	while Time.get_ticks_msec() - start < TIMEOUT_MS:
		await process_frame
		var screen = root.get_node_or_null(LoadingScreen.NODE_NAME)
		var covered := _covers(screen, what)
		screen_seen = screen_seen or covered
		var scene := current_scene
		var input: Node = scene.get_node_or_null("InputManager") if scene != null else null
		var in_world := input != null
		if in_world and probes.is_empty() and probe:
			probes = _add_probes(scene)
		if in_world:
			var ready: bool = input.is_world_ready()
			called = called or not ready
		if in_world and not called:
			_expect(covered, "%s: the scene showed before its load call" % what)
		elif in_world:
			var ready: bool = input.is_world_ready()
			# What this frame drew: the world exactly when it was handed over.
			_expect(ready != covered, "%s: world ready %s but screen covering %s" % [what, ready, covered])
			var fraction: float = input.load_fraction()
			_expect(fraction >= last_fraction, "%s: the fraction went back from %f to %f" % [what, last_fraction, fraction])
			_expect(ready == (fraction == 1.0), "%s: fraction %f with the world ready %s" % [what, fraction, ready])
			if covered:
				_expect(is_equal_approx(screen.fraction(), fraction), "%s: the screen shows %f, the load %f" % [what, screen.fraction(), fraction])
			last_fraction = fraction
			var stream: Node = scene.get_node("StreamNode")
			var state: Dictionary = stream.get_terrain_preload()
			var total := int(state.get("units_total", 0))
			if stream.is_terrain_preloading() and total > 0:
				var units_done := int(state.get("units_done", -1))
				_expect(units_done <= total - 1 and (units_done == total - 1) == bool(state.get("done", false)),
					"%s: units %d of %d with done %s" % [what, units_done, total, state.get("done")])
			if covered and probe:
				_send_probe_input(screen)
				_expect(probes[0].events == 0 and probes[1].get_meta("presses", 0) == 0,
					"%s: input reached the world under the screen (%d keys, %d clicks)"
					% [what, probes[0].events, probes[1].get_meta("presses", 0)])
			if covered and cancel_phase > 0 and not _seen.cancelled \
					and int(state.get("phase", 0)) == cancel_phase:
				_seen.cancelled = true
				_cancel(screen, how)
			if ready:
				_seen.handed_over = true
				break
		elif _seen.cancelled and scene != null and scene.scene_file_path == MENU_SCENE:
			break
		if covered:
			_seen.covered_frames += 1
	_expect(screen_seen, "%s: the screen was up" % what)
	print("[loading_screen] %s: %d covered frames, handed over %s, cancelled %s" % [
		what, _seen.covered_frames, _seen.handed_over, _seen.cancelled])
	if cancel_phase > 0:
		_expect(_seen.cancelled, "%s: the load reached phase %d under the screen" % [what, cancel_phase])
	if probe and _seen.handed_over:
		# After the hand-over the same input reaches both probes.
		for frame in 2:
			await process_frame
		_expect(root.get_node_or_null(LoadingScreen.NODE_NAME) == null, "%s: no screen after the hand-over" % what)
		_send_probe_input(null)
		_expect(probes[0].events > 0 and probes[1].get_meta("presses", 0) > 0,
			"%s: the probes take input after the hand-over (%d keys, %d clicks)"
			% [what, probes[0].events, probes[1].get_meta("presses", 0)])
		probes[0].queue_free()
		probes[1].get_parent().queue_free()

# Whether `screen` covers the whole viewport this frame: in the tree, visible, opaque, the size of
# the viewport, above every other visible canvas layer, and no visible dialog window over it.
func _covers(screen, what: String) -> bool:
	if screen == null or not screen.visible:
		return false
	var cover: ColorRect = screen.get_child(0)
	var view := root.get_visible_rect()
	_expect(cover.visible and cover.color.a == 1.0 and cover.get_global_rect().encloses(view),
		"%s: the cover is opaque over the viewport (%s over %s)" % [what, cover.get_global_rect(), view])
	for layer: CanvasLayer in root.find_children("*", "CanvasLayer", true, false):
		if layer != screen and layer.visible:
			_expect(layer.layer < screen.layer, "%s: canvas layer %s (%d) over the screen" % [what, layer.name, layer.layer])
	for window: Window in root.find_children("*", "Window", true, false):
		_expect(not window.visible, "%s: window %s shown over the screen" % [what, window.name])
	return true

# A node at the scene's end taking `_input`, and a HUD button over the whole screen.
func _add_probes(scene: Node) -> Array:
	var node := InputProbe.new()
	scene.add_child(node)
	var hud := CanvasLayer.new()
	hud.layer = 127
	var button := Button.new()
	button.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	button.pressed.connect(func(): button.set_meta("presses", int(button.get_meta("presses", 0)) + 1))
	hud.add_child(button)
	scene.add_child(hud)
	return [node, button]

# A key (A) and a left click away from the screen's Cancel.
func _send_probe_input(screen) -> void:
	var key := InputEventKey.new()
	key.keycode = KEY_A
	key.physical_keycode = KEY_A
	for pressed in [true, false]:
		key.pressed = pressed
		root.push_input(key.duplicate())
	var click := InputEventMouseButton.new()
	click.button_index = MOUSE_BUTTON_LEFT
	click.position = Vector2(8, root.get_visible_rect().size.y - 8)
	click.global_position = click.position
	for pressed in [true, false]:
		click.pressed = pressed
		click.button_mask = MOUSE_BUTTON_MASK_LEFT if pressed else 0
		root.push_input(click.duplicate())

func _cancel(screen, how: String) -> void:
	if how == "esc":
		var key := InputEventKey.new()
		key.keycode = KEY_ESCAPE
		key.physical_keycode = KEY_ESCAPE
		key.pressed = true
		root.push_input(key)
		return
	var at: Vector2 = screen._cancel.get_global_rect().get_center()
	var click := InputEventMouseButton.new()
	click.button_index = MOUSE_BUTTON_LEFT
	click.position = at
	click.global_position = at
	for pressed in [true, false]:
		click.pressed = pressed
		root.push_input(click.duplicate())

# Back at the main menu after a cancel: the screen gone, the world scene freed, the nodes outside
# the menu as before the load.
func _expect_back_at_menu(what: String, baseline: int) -> void:
	for frame in 4:
		await process_frame
	_expect(current_scene != null and current_scene.scene_file_path == MENU_SCENE, "%s: back at the main menu" % what)
	_expect(root.get_node_or_null(LoadingScreen.NODE_NAME) == null and root.find_children("LoadingScreen*", "", false, false).is_empty(),
		"%s: no screen left at the menu" % what)
	var outside := _nodes_outside_scene()
	_expect(outside == baseline, "%s: %d nodes outside the menu, %d before the load" % [what, outside, baseline])

# A refused save in game: the screen goes, the browser says why, the city stays as it was.
func _test_refused_in_game() -> void:
	var scene := current_scene
	var input: Node = scene.get_node("InputManager")
	var stream: Node = scene.get_node("StreamNode")
	var built := int(stream.get_terrain_budget().get("terrain_built", -1))
	input._finish_load_game_selection(_bad_path)
	var frames := 0
	while frames < 30 and root.get_node_or_null(LoadingScreen.NODE_NAME) != null:
		await process_frame
		frames += 1
	_expect(root.get_node_or_null(LoadingScreen.NODE_NAME) == null, "refused in game: the screen goes")
	_expect(not str(input.world_load_error()).is_empty(), "refused in game: Rust says why")
	var browser: Node = scene.get_node("TopMenu").get_node_or_null("SaveBrowser")
	_expect(browser != null and browser.is_open() and not str(browser._status.text).is_empty(),
		"refused in game: the browser shows the refusal")
	_expect(current_scene == scene and input.is_world_ready()
		and int(stream.get_terrain_budget().get("terrain_built", -2)) == built,
		"refused in game: the city stays as it was")
	browser._close()

# A refused save from the main menu: back at the menu, its browser saying why, no screen left; no
# frame of the world scene was drawn without the screen.
func _test_refused_from_menu() -> void:
	await _load_from_menu(_bad_path)
	var menu := current_scene
	var start := Time.get_ticks_msec()
	var left_menu := false
	# Until a new menu stands (the world scene may come and go between two of these frames).
	while Time.get_ticks_msec() - start < TIMEOUT_MS:
		await process_frame
		var scene := current_scene
		var screen = root.get_node_or_null(LoadingScreen.NODE_NAME)
		if scene != null and scene.scene_file_path == MAIN_SCENE:
			_expect(_covers(screen, "refused from the menu"), "refused from the menu: the world scene showed")
		elif scene != menu and scene != null and scene.scene_file_path == MENU_SCENE:
			left_menu = true
			break
	await process_frame
	_expect(left_menu and current_scene.scene_file_path == MENU_SCENE, "refused from the menu: back at the menu")
	_expect(root.get_node_or_null(LoadingScreen.NODE_NAME) == null, "refused from the menu: no screen left")
	var browser: Node = current_scene.get_node_or_null("SaveBrowser")
	_expect(browser != null and browser.is_open() and not str(browser._status.text).is_empty(),
		"refused from the menu: the browser shows the refusal")
