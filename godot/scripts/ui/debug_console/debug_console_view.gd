# SPDX-License-Identifier: GPL-2.0-only

## Widgets of the debug console: header actions, Log / Perf / Channels tabs and a command line.
##
## Pure view: it builds controls, shows what the controller hands it and reports user actions
## through signals. Only the two text fields take keyboard focus, so the game keeps WASD and
## its hotkeys until one of them is clicked.
extends PanelContainer

const UIStyle := preload("res://scripts/ui/ui_style.gd")

signal channel_toggled(channel: String, on: bool)
signal visual_mode_selected(mode: String, value: String)
signal chip_toggled(channel: String, shown: bool)
signal filter_changed(text: String)
signal command_submitted(text: String)
signal copy_pressed
signal dump_pressed
signal clear_pressed
signal tab_changed(tab: int)

const TAB_LOG := 0
const TAB_PERF := 1
const TAB_CHANNELS := 2
const FONT_SIZE := 13
# Paragraphs kept in the log view; older ones are dropped from the top.
const MAX_VIEW_LINES := 2000

const CHANNEL_COLORS := [
	Color(0.55, 0.80, 1.00), Color(1.00, 0.70, 0.40), Color(0.60, 0.95, 0.60),
	Color(0.95, 0.60, 0.95), Color(1.00, 0.90, 0.45), Color(0.55, 0.95, 0.90),
	Color(1.00, 0.55, 0.55), Color(0.75, 0.75, 1.00),
]
const EVENTS_COLOR := Color(1.00, 0.45, 0.35)
const TIME_COLOR := Color(0.55, 0.55, 0.60)

var _tabs: TabContainer
var _chips: HFlowContainer
var _filter: LineEdit
var _log: RichTextLabel
var _perf: Label
var _channel_toggles: VBoxContainer
var _visual_grid: GridContainer
var _visual_pickers := {}
var _modes: Label
var _command: LineEdit
var _status: Label
var _toggle_buttons := {}
var _colors := {}

func _ready() -> void:
	add_theme_stylebox_override("panel", UIStyle.panel_style(UIStyle.BG_DARK, 0, UIStyle.BORDER_ACCENT, 1))
	mouse_filter = Control.MOUSE_FILTER_STOP
	var root := VBoxContainer.new()
	root.add_theme_constant_override("separation", 6)
	add_child(root)
	root.add_child(_build_header())
	_tabs = TabContainer.new()
	_tabs.size_flags_vertical = Control.SIZE_EXPAND_FILL
	_tabs.get_tab_bar().focus_mode = Control.FOCUS_NONE
	_tabs.tab_changed.connect(func(tab: int) -> void: tab_changed.emit(tab))
	root.add_child(_tabs)
	_tabs.add_child(_build_log_tab())
	_tabs.add_child(_build_perf_tab())
	_tabs.add_child(_build_channels_tab())
	_command = _line_edit("Command: help, channel <name> on|off, visual <mode> <value>, dump, clear")
	_command.text_submitted.connect(_on_command_submitted)
	root.add_child(_command)
	_status = _label("")
	_status.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	_status.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	root.add_child(_status)

## Creates one channel chip and one on/off toggle per channel. Locked channels cannot be
## switched off.
func set_channels(names: PackedStringArray, locked: Dictionary) -> void:
	for i in names.size():
		var channel := names[i]
		_colors[channel] = EVENTS_COLOR if locked.get(channel, false) else CHANNEL_COLORS[i % CHANNEL_COLORS.size()]
		var chip := _button(channel, true)
		chip.button_pressed = true
		chip.add_theme_color_override("font_pressed_color", _colors[channel])
		chip.toggled.connect(func(shown: bool) -> void: chip_toggled.emit(channel, shown))
		_chips.add_child(chip)
		var toggle := CheckButton.new()
		toggle.text = channel
		toggle.focus_mode = Control.FOCUS_NONE
		toggle.disabled = locked.get(channel, false)
		UIStyle.set_font_size(toggle, FONT_SIZE)
		toggle.toggled.connect(func(on: bool) -> void: channel_toggled.emit(channel, on))
		_channel_toggles.add_child(toggle)
		_toggle_buttons[channel] = toggle

## Shows a channel's capture state without emitting `channel_toggled`.
func set_channel_state(channel: String, on: bool) -> void:
	var toggle: CheckButton = _toggle_buttons.get(channel)
	if toggle != null and toggle.button_pressed != on:
		toggle.set_pressed_no_signal(on)

## Creates one value picker per visual mode; `values` maps each mode to its value names.
func set_visual_modes(modes: PackedStringArray, values: Dictionary) -> void:
	for mode in modes:
		_visual_grid.add_child(_label(mode))
		var picker := OptionButton.new()
		picker.focus_mode = Control.FOCUS_NONE
		picker.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		UIStyle.set_font_size(picker, FONT_SIZE)
		for value in values.get(mode, PackedStringArray()):
			picker.add_item(value)
		picker.item_selected.connect(
			func(index: int) -> void: visual_mode_selected.emit(mode, picker.get_item_text(index))
		)
		_visual_grid.add_child(picker)
		_visual_pickers[mode] = picker

## Shows a visual mode's value without emitting `visual_mode_selected`.
func set_visual_mode_state(mode: String, value: String) -> void:
	var picker: OptionButton = _visual_pickers.get(mode)
	if picker == null:
		return
	for i in picker.item_count:
		if picker.get_item_text(i) == value:
			picker.select(i)
			return

## Lists the startup-only modes; they are read-only for this session.
func set_startup_modes(modes: Dictionary) -> void:
	var lines := PackedStringArray()
	for name in modes.keys():
		var value: Variant = modes[name]
		var text := ("on" if value else "off") if value is bool else (String(value) if not String(value).is_empty() else "-")
		lines.append("%s = %s" % [name, text])
	_modes.text = "\n".join(lines)

## Appends one log line: time, channel tag and message. Messages are added as plain text.
func append_line(time_ms: int, channel: String, message: String) -> void:
	if _log.get_paragraph_count() > MAX_VIEW_LINES:
		_log.remove_paragraph(0)
	_log.push_color(TIME_COLOR)
	_log.add_text("%7.3f " % (time_ms / 1000.0))
	_log.pop()
	_log.push_color(_colors.get(channel, UIStyle.TEXT_PRIMARY))
	_log.add_text(channel)
	_log.pop()
	_log.add_text(" " + message)
	_log.newline()

## Empties the log view.
func clear_log() -> void:
	_log.clear()

## Replaces the Perf tab text.
func set_perf_text(text: String) -> void:
	_perf.text = text

## Shows a one-line result of the last action or command.
func set_status(text: String) -> void:
	_status.text = text

## Returns the selected tab index.
func current_tab() -> int:
	return _tabs.current_tab

## Returns `true` if one of the console's text fields has keyboard focus.
func has_text_focus() -> bool:
	return _filter.has_focus() or _command.has_focus()

## Gives the keyboard back to the game.
func release_text_focus() -> void:
	_filter.release_focus()
	_command.release_focus()

func _build_header() -> Control:
	var header := HBoxContainer.new()
	var title := _label("Debug console  (§)")
	title.add_theme_color_override("font_color", UIStyle.TEXT_SECTION)
	title.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	header.add_child(title)
	for action in [["Copy", copy_pressed], ["Dump", dump_pressed], ["Clear", clear_pressed]]:
		var button := _button(action[0], false)
		var pressed: Signal = action[1]
		button.pressed.connect(func() -> void: pressed.emit())
		header.add_child(button)
	return header

func _build_log_tab() -> Control:
	var tab := VBoxContainer.new()
	tab.name = "Log"
	_chips = HFlowContainer.new()
	tab.add_child(_chips)
	_filter = _line_edit("Filter text")
	_filter.text_changed.connect(func(text: String) -> void: filter_changed.emit(text))
	tab.add_child(_filter)
	_log = RichTextLabel.new()
	_log.size_flags_vertical = Control.SIZE_EXPAND_FILL
	_log.scroll_following = true
	_log.focus_mode = Control.FOCUS_NONE
	_log.add_theme_font_override("normal_font", _mono_font())
	_log.add_theme_font_size_override("normal_font_size", UIStyle.scaled_font_size(FONT_SIZE))
	tab.add_child(_log)
	return tab

func _build_perf_tab() -> Control:
	var tab := ScrollContainer.new()
	tab.name = "Perf"
	_perf = _label("")
	_perf.add_theme_font_override("font", _mono_font())
	_perf.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_perf.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	tab.add_child(_perf)
	return tab

func _build_channels_tab() -> Control:
	var tab := ScrollContainer.new()
	tab.name = "Channels"
	var column := VBoxContainer.new()
	column.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	tab.add_child(column)
	_channel_toggles = VBoxContainer.new()
	column.add_child(_channel_toggles)
	var visual_heading := _label("Visual modes")
	visual_heading.add_theme_color_override("font_color", UIStyle.TEXT_SECTION)
	column.add_child(visual_heading)
	_visual_grid = GridContainer.new()
	_visual_grid.columns = 2
	column.add_child(_visual_grid)
	var heading := _label("Startup modes (read-only, set at launch)")
	heading.add_theme_color_override("font_color", UIStyle.TEXT_SECTION)
	column.add_child(heading)
	_modes = _label("")
	_modes.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	column.add_child(_modes)
	return tab

func _on_command_submitted(text: String) -> void:
	_command.clear()
	command_submitted.emit(text)

func _label(text: String) -> Label:
	var label := Label.new()
	label.text = text
	UIStyle.set_font_size(label, FONT_SIZE)
	return label

func _button(text: String, toggle: bool) -> Button:
	var button := Button.new()
	button.text = text
	button.toggle_mode = toggle
	button.focus_mode = Control.FOCUS_NONE
	UIStyle.set_font_size(button, FONT_SIZE)
	return button

func _line_edit(placeholder: String) -> LineEdit:
	var edit := LineEdit.new()
	edit.placeholder_text = placeholder
	edit.focus_mode = Control.FOCUS_CLICK
	UIStyle.set_font_size(edit, FONT_SIZE)
	return edit

func _mono_font() -> SystemFont:
	var font := SystemFont.new()
	font.font_names = PackedStringArray(["monospace"])
	return font
