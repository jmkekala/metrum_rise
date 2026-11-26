# SPDX-License-Identifier: GPL-3.0-or-later

## Main menu shell — default startup surface for normal gameplay launches.
## No map or SimulationNode is present here; the player must pick a world or a
## save before gameplay scene loading begins.
extends Control

const NewGameDialog = preload("res://scripts/ui/new_game_dialog.gd")
const OptionsMenu = preload("res://scripts/ui/options_menu.gd")
const OptionsRows = preload("res://scripts/ui/options_rows.gd")
const SaveBrowser = preload("res://scripts/ui/save_browser.gd")
const UIStyle = preload("res://scripts/ui/ui_style.gd")
const GameSettings = preload("res://scripts/core/game_settings.gd")
const MenuPipelinePrewarm = preload("res://scripts/core/menu_pipeline_prewarm.gd")
const LoadingScreen = preload("res://scripts/ui/loading_screen.gd")

const WORLDS_DIR := "user://worlds"
const SAVES_DIR := "user://saves"
const BACKGROUND := Color(0.055, 0.065, 0.085)

var _options_menu = null
var _new_game_dialog: Window = null
var _prewarm: Node = null
# The benchmark's menu (`RENDER-43`): the last frame's start, for its longest frame.
var _benchmark_frame_us := 0

func _enter_tree() -> void:
	GameSettings.set_menu_frame_cap(true)

func _exit_tree() -> void:
	GameSettings.set_menu_frame_cap(false)

func _ready() -> void:
	# Back from a cancelled or refused player's load: its loading screen stayed up across the
	# change (`UI-15`) and goes before the menu's first frame.
	var screen = LoadingScreen.current(get_tree())
	if screen != null:
		screen.close()
	_build_ui()
	var refusal: Dictionary = LaunchState.take_refusal()
	if not refusal.is_empty():
		_show_refusal(refusal)
	if LaunchState.benchmark_menu_dwell_s >= 0.0:
		# The menu's frames, its prewarm's build frames included, for the benchmark's report.
		LaunchState.benchmark_menu = {"longest_frame_ms": 0.0, "frames": 0}
		_benchmark_frame_us = Time.get_ticks_usec()
		get_tree().process_frame.connect(_note_benchmark_frame)
	# The first load's pipelines compile while the menu shows (`RENDER-43`), from its second
	# frame: the first shows the menu at once.
	await get_tree().process_frame
	_prewarm = MenuPipelinePrewarm.new()
	add_child(_prewarm)
	if LaunchState.benchmark_menu_dwell_s >= 0.0:
		_leave_after_benchmark_dwell()

# A benchmark's menu (`tools/stream_benchmark.py --menu-dwell`): the menu stays its dwell from the
# launch, and at least until the prewarm view has rendered once (its surfaces are created at that
# draw: a player's click comes later), then the benchmark's world scene runs as from the router.
# The dwell is wall-clock time, as a player's: a scene tree timer counts process time, which fell
# 3 s behind the wall clock across the prewarm's long frames (round R43c).
func _leave_after_benchmark_dwell() -> void:
	LaunchState.benchmark_menu_ms[0] = Time.get_ticks_msec()
	await RenderingServer.frame_post_draw
	while Time.get_ticks_msec() < LaunchState.benchmark_menu_dwell_s * 1000.0:
		await get_tree().process_frame
	LaunchState.benchmark_menu_ms[1] = Time.get_ticks_msec()
	get_tree().process_frame.disconnect(_note_benchmark_frame)
	LaunchState.benchmark_menu["prewarm"] = _prewarm.report()
	get_tree().change_scene_to_file("res://scenes/Main.tscn")

func _note_benchmark_frame() -> void:
	var now := Time.get_ticks_usec()
	var report: Dictionary = LaunchState.benchmark_menu
	report.frames = int(report.frames) + 1
	report.longest_frame_ms = maxf(float(report.longest_frame_ms), float(now - _benchmark_frame_us) / 1000.0)
	_benchmark_frame_us = now

# Plain text entries down the left side over a flat dark fill: no card, no button boxes. (A
# gradient this dark bands visibly in 8-bit colour.)
func _build_ui() -> void:
	set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)

	var background := ColorRect.new()
	background.color = BACKGROUND
	background.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	add_child(background)

	var frame := MarginContainer.new()
	frame.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	frame.add_theme_constant_override("margin_left", int(UIStyle.scaled_px(96)))
	add_child(frame)
	var content := VBoxContainer.new()
	content.size_flags_horizontal = Control.SIZE_SHRINK_BEGIN
	content.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	content.add_theme_constant_override("separation", int(UIStyle.scaled_px(4)))
	content.add_to_group(OptionsRows.COVERED_GROUP)
	frame.add_child(content)

	# Inset by the entries' text margin, so the title lines up with their text.
	var heading := VBoxContainer.new()
	heading.add_theme_constant_override("separation", 6)
	var inset := MarginContainer.new()
	inset.add_theme_constant_override("margin_left", 14)
	inset.add_child(heading)
	content.add_child(inset)
	var title := Label.new()
	title.text = "METRUM RISE"
	UIStyle.set_font_size(title, 44)
	title.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	heading.add_child(title)
	var rule := ColorRect.new()
	rule.color = UIStyle.HUD_ACCENT
	rule.custom_minimum_size = Vector2(UIStyle.scaled_px(56), 3)
	rule.size_flags_horizontal = Control.SIZE_SHRINK_BEGIN
	heading.add_child(rule)
	var gap := Control.new()
	gap.custom_minimum_size.y = UIStyle.scaled_px(36)
	content.add_child(gap)

	content.add_child(_make_main_button("New Game", _on_new_game_pressed))
	content.add_child(_make_main_button("Load Game", _on_load_game_pressed))
	content.add_child(_make_main_button("World Editor", _on_world_editor_pressed))
	content.add_child(_make_main_button("Options", _on_options_pressed))
	content.add_child(_make_main_button("Quit", _on_quit_pressed))

	# Release builds carry the version CI stamps into the project (docs/release.md); a checkout has none.
	var version := str(ProjectSettings.get_setting("application/config/version", ""))
	var version_label := Label.new()
	version_label.text = "Version " + (version if not version.is_empty() else "dev")
	version_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	UIStyle.set_font_size(version_label, 13)
	version_label.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	content.add_child(version_label)

func _make_main_button(label: String, callback: Callable) -> Button:
	var button := UIStyle.menu_text_button(label, 20)
	button.custom_minimum_size.y = UIStyle.scaled_px(44)
	button.pressed.connect(callback)
	return button

func _on_new_game_pressed() -> void:
	SaveBrowser.open(self, SaveBrowser.MODE_WORLD, WORLDS_DIR, _open_new_game_options)

func _on_load_game_pressed() -> void:
	SaveBrowser.open(self, SaveBrowser.MODE_LOAD, SAVES_DIR, _finish_load_game_selection)

# A load or new game the world scene refused: its browser again, saying why.
func _show_refusal(refusal: Dictionary) -> void:
	if refusal["kind"] == "save":
		_on_load_game_pressed()
	else:
		_on_new_game_pressed()
	get_node(SaveBrowser.NODE_NAME).show_refusal(str(refusal["message"]))

func _on_world_editor_pressed() -> void:
	var pid := OS.create_instance(["--", "--world-editor"])
	if pid == -1:
		push_error("Failed to launch world editor.")

func _on_options_pressed() -> void:
	if not _options_menu:
		_options_menu = OptionsMenu.new()
		add_child(_options_menu)
	_options_menu.popup_options(OptionsMenu.CONTEXT_MAIN_MENU)

func _on_quit_pressed() -> void:
	get_tree().quit()

func _open_new_game_options(path: String) -> void:
	if _new_game_dialog == null:
		_new_game_dialog = NewGameDialog.new()
		_new_game_dialog.start_requested.connect(_finish_new_game_selection)
		add_child(_new_game_dialog)
	_new_game_dialog.popup_for_world(path)

func _finish_new_game_selection(path: String, vegetation: Dictionary) -> void:
	LaunchState.queue_new_game(path, vegetation)
	await _enter_world_scene(path)

func _finish_load_game_selection(path: String) -> void:
	LaunchState.queue_load_game(path)
	await _enter_world_scene(path)

# The loading screen first (`UI-15`), drawn once, so it is what shows while the world scene and
# the save load; the world scene's load flow takes it over.
func _enter_world_scene(path: String) -> void:
	var screen = LoadingScreen.open(get_tree(), path.get_file().get_basename())
	await screen.wait_drawn()
	get_tree().change_scene_to_file("res://scenes/Main.tscn")
