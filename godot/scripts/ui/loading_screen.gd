# SPDX-License-Identifier: GPL-3.0-or-later

## The loading screen (`UI-15`): from the click on Load or Start until gameplay, one opaque dark
## screen over the world, the HUD, the menu bar and the frame counter, with "LOADING", the city's
## or world's file name in dim text, a thin progress bar with its percentage, and Cancel. A view
## only: it shows the fraction the input manager's load flow gives it and emits
## `cancel_requested` (the Cancel button, Esc or the mouse's back button); the load flow decides
## what a cancel does. It lives under the scene tree's root, not in a scene, so it stays up across
## the main menu's change to the world scene, and it keeps itself the root's last child: its
## `_input` then runs before every scene node's, and it takes every key, mouse button, wheel and
## gesture while it is up (`O(1)` an event). The world keeps drawing behind it (the first shown
## frame has been drawn before); nothing of it shows.
extends CanvasLayer

## The Cancel button, Esc or the mouse's back button.
signal cancel_requested
## The screen has reached the display once (a frame was drawn with it up).
signal first_drawn

const UIStyle = preload("res://scripts/ui/ui_style.gd")

## The node's name under the root: one screen at a time.
const NODE_NAME := "LoadingScreen"
## Above every canvas layer in use (the FPS overlay is 128).
const LAYER := 129
## The main menu's background (`main_menu.gd`), opaque.
const BACKGROUND := Color(0.055, 0.065, 0.085)
const TRACK := Color(1.0, 1.0, 1.0, 0.10)
## The bar: thickness at 100 % UI scale, its share of the screen's width and its bounds.
const BAR_HEIGHT_PX := 6.0
const BAR_SCREEN_SHARE := 0.4
const BAR_MIN_PX := 320.0
const BAR_MAX_PX := 640.0

var _title := ""
var _drawn := false
var _cover: ColorRect
var _name_label: Label
var _track: ColorRect
var _fill: ColorRect
var _percent: Label
var _cancel: Button
# Whether the last left press landed on Cancel (its release there cancels).
var _cancel_armed := false

## The screen under `tree`'s root, made and shown if there is none; `title` (the file name shown
## under "LOADING") replaces the shown one when given.
static func open(tree: SceneTree, title := ""):
	var screen = current(tree)
	if screen == null:
		screen = load("res://scripts/ui/loading_screen.gd").new()
		screen.name = NODE_NAME
		tree.root.add_child(screen)
	if not title.is_empty():
		screen.set_title(title)
	screen.keep_last()
	return screen

## The screen that is up under `tree`'s root, or null.
static func current(tree: SceneTree):
	var screen := tree.root.get_node_or_null(NODE_NAME)
	return screen if screen != null and not screen.is_queued_for_deletion() else null

func _ready() -> void:
	layer = LAYER
	process_mode = Node.PROCESS_MODE_ALWAYS
	_build()
	get_viewport().size_changed.connect(_fit_bar)
	_fit_bar()
	# A scene that enters the root later (the main menu's change to the world scene) would come
	# after this screen and take input before it.
	get_tree().root.child_entered_tree.connect(func(_node: Node): keep_last.call_deferred())
	# A headless run draws nothing: its first frame stands in for the first draw.
	if DisplayServer.get_name() == "headless":
		await get_tree().process_frame
	else:
		await RenderingServer.frame_post_draw
	_drawn = true
	first_drawn.emit()

func _build() -> void:
	_cover = ColorRect.new()
	_cover.color = BACKGROUND
	_cover.mouse_filter = Control.MOUSE_FILTER_STOP
	_cover.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	add_child(_cover)

	var centre := CenterContainer.new()
	centre.mouse_filter = Control.MOUSE_FILTER_IGNORE
	centre.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	_cover.add_child(centre)
	var column := VBoxContainer.new()
	column.mouse_filter = Control.MOUSE_FILTER_IGNORE
	column.add_theme_constant_override("separation", int(UIStyle.scaled_px(10)))
	centre.add_child(column)

	var heading := Label.new()
	heading.text = "LOADING"
	UIStyle.set_font_size(heading, 28)
	heading.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	column.add_child(heading)
	_name_label = Label.new()
	_name_label.text = _title
	UIStyle.set_font_size(_name_label, 14)
	_name_label.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	column.add_child(_name_label)

	# The bar and its percentage on one line: the track a dim line, the fill the accent colour.
	var row := HBoxContainer.new()
	row.add_theme_constant_override("separation", int(UIStyle.scaled_px(14)))
	column.add_child(row)
	_track = ColorRect.new()
	_track.color = TRACK
	_track.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	row.add_child(_track)
	_fill = ColorRect.new()
	_fill.color = UIStyle.HUD_ACCENT
	_fill.anchor_bottom = 1.0
	_track.add_child(_fill)
	_percent = Label.new()
	UIStyle.set_font_size(_percent, 14)
	_percent.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	# Room for "100 %" so the bar does not move as the number grows.
	_percent.custom_minimum_size.x = UIStyle.scaled_px(48)
	row.add_child(_percent)
	set_fraction(0.0)

	_cancel = UIStyle.menu_text_button("Cancel", 16, false)
	_cancel.size_flags_horizontal = Control.SIZE_SHRINK_BEGIN
	column.add_child(_cancel)

# The bar's width: its share of the screen within its bounds (UI-scaled), and its thickness.
func _fit_bar() -> void:
	var width := get_viewport().get_visible_rect().size.x * BAR_SCREEN_SHARE
	_track.custom_minimum_size = Vector2(
		clampf(width, UIStyle.scaled_px(BAR_MIN_PX), UIStyle.scaled_px(BAR_MAX_PX)),
		maxf(1.0, roundf(UIStyle.scaled_px(BAR_HEIGHT_PX))))

## Shows `fraction` (0 to 1) as the bar's fill and its whole percent, rounded down.
func set_fraction(fraction: float) -> void:
	var shown := clampf(fraction, 0.0, 1.0)
	_fill.anchor_right = shown
	_fill.offset_right = 0.0
	_percent.text = "%d %%" % int(floor(shown * 100.0))

## The fraction shown (0 to 1).
func fraction() -> float:
	return _fill.anchor_right

## The file name shown under "LOADING".
func set_title(title: String) -> void:
	_title = title
	if _name_label != null:
		_name_label.text = title

## The file name shown under "LOADING".
func title() -> String:
	return _title

## Whether the screen has reached the display once.
func is_drawn() -> bool:
	return _drawn

## Returns once a frame was drawn with the screen up (at once if one was).
func wait_drawn() -> void:
	if not _drawn:
		await first_drawn

## Moves the screen to the root's last child, so its `_input` runs first. `O(1)`.
func keep_last() -> void:
	var root := get_parent()
	if root != null and not is_queued_for_deletion() and get_index() != root.get_child_count() - 1:
		root.move_child(self, -1)

## Takes the screen down at once (it is not drawn in this frame) and frees it.
func close() -> void:
	if is_queued_for_deletion():
		return
	visible = false
	name = NODE_NAME + "Closed"
	queue_free()

# Every key, mouse button, wheel and gesture stops here: nothing under the screen takes input
# while it is up. Motion passes on, so the GUI lights Cancel under the mouse (the full-screen
# cover stops it from reaching any control below). A left click on Cancel, Esc and the mouse's
# back button ask for the cancel.
func _input(event: InputEvent) -> void:
	if event is InputEventMouseMotion:
		return
	get_viewport().set_input_as_handled()
	if UIStyle.is_mouse_back(event):
		cancel_requested.emit()
	elif event is InputEventKey:
		if event.pressed and not event.echo and event.keycode == KEY_ESCAPE:
			cancel_requested.emit()
	elif event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT:
		var on_cancel := _cancel.get_global_rect().has_point(event.position)
		if event.pressed:
			_cancel_armed = on_cancel
		elif _cancel_armed and on_cancel:
			_cancel_armed = false
			cancel_requested.emit()
