# SPDX-License-Identifier: GPL-2.0-only

## The mouse's back (thumb) button acts as Esc in the menus: it closes the main menu's Load
## Game and New Game browsers, the New Game dialog and Options, and in the pause menu it first
## dismisses the leave confirmation, then resumes. Events go through `Input.parse_input_event`,
## the route a real button takes.
extends SceneTree

const PauseMenu = preload("res://scripts/ui/pause_menu.gd")
const SaveBrowser = preload("res://scripts/ui/save_browser.gd")

var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

# Presses and releases the back button at `at`, then lets the frame handle it.
func _back(at: Vector2 = Vector2(400, 300)) -> void:
	for pressed in [true, false]:
		var event := InputEventMouseButton.new()
		event.button_index = MOUSE_BUTTON_XBUTTON1
		event.pressed = pressed
		event.position = at
		event.global_position = at
		Input.parse_input_event(event)
	await process_frame
	await process_frame

func _run() -> void:
	# Loaded at run time: the menu names the LaunchState autoload, which a test script cannot see
	# while it compiles.
	var menu: Control = load("res://scripts/core/main_menu.gd").new()
	root.add_child(menu)
	await process_frame

	menu._on_load_game_pressed()
	var browser = menu.get_node(SaveBrowser.NODE_NAME)
	_expect(browser.is_open(), "Load Game opens the browser")
	await _back()
	_expect(not browser.is_open(), "the back button closes Load Game")

	menu._on_new_game_pressed()
	_expect(browser.is_open(), "New Game opens the world browser")
	await _back()
	_expect(not browser.is_open(), "the back button closes the world browser")

	menu._open_new_game_options("user://worlds/none.json")
	var dialog: Window = menu._new_game_dialog
	await process_frame
	_expect(dialog.visible, "the New Game dialog opens")
	await _back(Vector2(dialog.position) + Vector2(dialog.size) * 0.5)
	_expect(not dialog.visible, "the back button closes the New Game dialog")

	menu._on_options_pressed()
	_expect(menu._options_menu.is_open(), "Options opens")
	await _back()
	_expect(not menu._options_menu.is_open(), "the back button closes Options")
	menu.queue_free()

	var pause := PauseMenu.new()
	root.add_child(pause)
	pause.build([["Resume", PauseMenu.RESUME, ""], ["Quit", 99, "Quit"]])
	var picked: Array[int] = []
	pause.action_requested.connect(func(id: int): picked.append(id))
	pause.show()
	pause._ask(99, "Quit")
	await process_frame
	await _back()
	_expect(not pause._confirm.visible and picked.is_empty(), "the back button dismisses the leave confirmation: %s" % [picked])
	await _back()
	_expect(picked == [PauseMenu.RESUME], "the back button then resumes: %s" % [picked])

	if _failures == 0:
		print("menu_back_button_test: PASS")
	quit(1 if _failures > 0 else 0)
