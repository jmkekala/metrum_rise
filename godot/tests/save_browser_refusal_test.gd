# SPDX-License-Identifier: GPL-3.0-or-later

## The Load and New Game lists grey out a file the game would refuse (`UI-16`): in a scratch
## folder of the test profile, the bundled blank world and a damaged file. New Game shows the
## damaged row greyed with its reason and the load's message, never picks it (first pick, Up and
## Down, a double click) and confirms the world; Load refuses both (a world is no save), picks
## nothing and confirms nothing. Which kinds Rust refuses is tested in Rust.
extends SceneTree

const SaveBrowser = preload("res://scripts/ui/save_browser.gd")
const DIR := "user://save-browser-test"

var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	if not OS.get_user_data_dir().to_lower().contains("godot-tests"):
		push_error("save_browser_refusal_test must run in the isolated profile from ./run.sh --test")
		quit(1)
		return
	var native := ProjectSettings.globalize_path(DIR)
	DirAccess.make_dir_recursive_absolute(native)
	_expect(DirAccess.copy_absolute("res://bootstrap/worlds/test.sqlite", native.path_join("blank.sqlite")) == OK, "fixture world copied")
	var broken := FileAccess.open(DIR.path_join("broken.sqlite"), FileAccess.WRITE)
	broken.store_string("not a world, only bytes that are long enough to be no SQLite header")
	broken.close()

	var chosen: Array[String] = []
	var browser = SaveBrowser.open(root, SaveBrowser.MODE_WORLD, DIR, func(path: String): chosen.append(path))
	await process_frame
	var rows := {}
	for entry: Dictionary in browser._entries:
		rows[entry.file] = entry
	_expect(rows.size() == 2, "both files listed: %s" % [rows.keys()])
	var bad: Dictionary = rows.get("broken.sqlite", {})
	var good: Dictionary = rows.get("blank.sqlite", {})
	_expect(not bad.is_empty() and bad.row.disabled, "the damaged file is greyed out")
	_expect(not good.is_empty() and not good.row.disabled and good.refusal.is_empty(), "the world loads")
	if bad.is_empty() or good.is_empty():
		_finish()
		return
	_expect(str(bad.refusal.reason) == "Damaged file, cannot load", "short reason: %s" % bad.refusal)
	_expect(_row_texts(bad.row).has("Damaged file, cannot load"), "the row shows the reason: %s" % [_row_texts(bad.row)])
	_expect(bad.row.tooltip_text.begins_with("Cannot load: ") and bad.row.tooltip_text.contains(str(bad.refusal.message)), "tooltip holds the load's message: " + bad.row.tooltip_text)
	_expect(good.row.button_pressed and not bad.row.button_pressed, "the first pick skips the damaged file")
	for step in [-1, 1, -1]:
		browser._move_selection(step)
		_expect(good.row.button_pressed and not bad.row.button_pressed, "Up/Down never pick the damaged file")
	var click := InputEventMouseButton.new()
	click.button_index = MOUSE_BUTTON_LEFT
	click.pressed = true
	click.double_click = true
	bad.row.gui_input.emit(click)
	_expect(chosen.is_empty() and browser.is_open(), "a double click on the damaged file does nothing")
	browser._on_confirm()
	_expect(chosen == [native.path_join("blank.sqlite")], "Continue takes the world: %s" % [chosen])

	# A world is no save: Load greys out both, picks nothing and confirms nothing.
	chosen.clear()
	browser = SaveBrowser.open(root, SaveBrowser.MODE_LOAD, DIR, func(path: String): chosen.append(path))
	await process_frame
	_expect(browser._entries.all(func(entry: Dictionary) -> bool: return entry.row.disabled and not entry.row.button_pressed), "Load greys out a world and a damaged file")
	_expect(browser._confirm.disabled and browser._status.text.is_empty(), "nothing to confirm, no pick hint: '%s'" % browser._status.text)
	browser._on_confirm()
	_expect(chosen.is_empty(), "Load confirms nothing")
	browser._close()
	_finish()

func _row_texts(row: Control) -> Array:
	return row.find_children("*", "Label", true, false).map(func(label: Label) -> String: return label.text)

func _finish() -> void:
	for file in DirAccess.get_files_at(ProjectSettings.globalize_path(DIR)):
		DirAccess.remove_absolute(ProjectSettings.globalize_path(DIR).path_join(file))
	DirAccess.remove_absolute(ProjectSettings.globalize_path(DIR))
	if _failures == 0:
		print("PASS save_browser_refusal_test")
	quit(0 if _failures == 0 else 1)
