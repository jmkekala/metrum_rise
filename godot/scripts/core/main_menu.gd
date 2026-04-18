## Main menu shell — default startup surface for normal gameplay launches.
## No map or SimulationNode is present here; the player must pick a world or a
## save before gameplay scene loading begins.
extends Control

const UIStyle = preload("res://scripts/ui/ui_style.gd")

const WORLDS_DIR := "user://worlds"
const SAVES_DIR := "user://saves"

func _ready() -> void:
	_build_ui()

func _build_ui() -> void:
	set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)

	var background := ColorRect.new()
	background.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	background.color = Color(0.05, 0.06, 0.08, 1.0)
	add_child(background)

	var center := CenterContainer.new()
	center.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	add_child(center)

	var shell := PanelContainer.new()
	shell.custom_minimum_size = Vector2(540.0, 420.0)
	shell.add_theme_stylebox_override("panel", UIStyle.window_body_style())
	center.add_child(shell)

	var margin := MarginContainer.new()
	margin.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	margin.add_theme_constant_override("margin_left", 28)
	margin.add_theme_constant_override("margin_right", 28)
	margin.add_theme_constant_override("margin_top", 24)
	margin.add_theme_constant_override("margin_bottom", 24)
	shell.add_child(margin)

	var content := VBoxContainer.new()
	content.add_theme_constant_override("separation", 18)
	margin.add_child(content)

	var title := Label.new()
	title.text = "Metrum Rise"
	title.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	title.add_theme_font_size_override("font_size", 40)
	title.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	content.add_child(title)

	var subtitle := Label.new()
	subtitle.text = "Choose a world or save before entering gameplay."
	subtitle.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	subtitle.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	subtitle.add_theme_font_size_override("font_size", 16)
	subtitle.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	content.add_child(subtitle)

	var buttons := VBoxContainer.new()
	buttons.add_theme_constant_override("separation", 12)
	content.add_child(buttons)

	buttons.add_child(_make_main_button("New Game", _on_new_game_pressed))
	buttons.add_child(_make_main_button("Load Game", _on_load_game_pressed))
	buttons.add_child(_make_main_button("World Editor", _on_world_editor_pressed))
	buttons.add_child(_make_main_button("Quit", _on_quit_pressed))

	var footer := Label.new()
	footer.text = "New Game loads worlds from user://worlds/\nLoad Game loads saves from user://saves/"
	footer.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	footer.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	footer.add_theme_font_size_override("font_size", 13)
	footer.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	content.add_child(footer)

func _make_main_button(label: String, callback: Callable) -> Button:
	var button := Button.new()
	button.text = label
	button.custom_minimum_size = Vector2(320.0, 58.0)
	button.size_flags_horizontal = Control.SIZE_SHRINK_CENTER
	button.pressed.connect(callback)
	return button

func _on_new_game_pressed() -> void:
	_ensure_dir(WORLDS_DIR)
	var dialog := FileDialog.new()
	dialog.access = FileDialog.ACCESS_FILESYSTEM
	dialog.file_mode = FileDialog.FILE_MODE_OPEN_FILE
	dialog.filters = PackedStringArray(["*.sqlite ; WorldDefinition Files"])
	dialog.current_dir = ProjectSettings.globalize_path(WORLDS_DIR)
	dialog.file_selected.connect(func(path: String): _on_new_game_selected(path, dialog))
	dialog.canceled.connect(dialog.queue_free)
	add_child(dialog)
	dialog.popup_centered(Vector2i(880, 620))

func _on_load_game_pressed() -> void:
	_ensure_dir(SAVES_DIR)
	var dialog := FileDialog.new()
	dialog.access = FileDialog.ACCESS_FILESYSTEM
	dialog.file_mode = FileDialog.FILE_MODE_OPEN_FILE
	dialog.filters = PackedStringArray(["*.sqlite ; Save Files"])
	dialog.current_dir = ProjectSettings.globalize_path(SAVES_DIR)
	dialog.file_selected.connect(func(path: String): _on_load_game_selected(path, dialog))
	dialog.canceled.connect(dialog.queue_free)
	add_child(dialog)
	dialog.popup_centered(Vector2i(880, 620))

func _on_world_editor_pressed() -> void:
	var pid := OS.create_instance(["--", "--world-editor"])
	if pid == -1:
		push_error("Failed to launch world editor.")

func _on_quit_pressed() -> void:
	get_tree().quit()

func _on_new_game_selected(path: String, dialog: FileDialog) -> void:
	dialog.hide()
	dialog.call_deferred("queue_free")
	call_deferred("_finish_new_game_selection", path)

func _on_load_game_selected(path: String, dialog: FileDialog) -> void:
	dialog.hide()
	dialog.call_deferred("queue_free")
	call_deferred("_finish_load_game_selection", path)

func _finish_new_game_selection(path: String) -> void:
	LaunchState.queue_new_game(path)
	get_tree().change_scene_to_file("res://scenes/Main.tscn")

func _finish_load_game_selection(path: String) -> void:
	LaunchState.queue_load_game(path)
	get_tree().change_scene_to_file("res://scenes/Main.tscn")

func _ensure_dir(path: String) -> void:
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(path))
