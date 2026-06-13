## Shared dark/light theme helpers for editor shells and editor-owned dialogs.
##
## Gameplay HUD styling remains in `ui_style.gd`; this file is for dense tools
## such as the asset editor where inspectors, lists, and file pickers need a
## coherent app-style palette.
extends RefCounted

const UIStyle = preload("res://scripts/ui/ui_style.gd")

const MODE_DARK := "dark"
const MODE_LIGHT := "light"
const PAD_X := 8
const PAD_Y := 4

const DARK := {
	"bg": Color(0.035, 0.037, 0.043, 1.0),
	"panel": Color(0.060, 0.065, 0.075, 1.0),
	"panel_alt": Color(0.085, 0.092, 0.105, 1.0),
	"panel_hover": Color(0.115, 0.125, 0.140, 1.0),
	"panel_pressed": Color(0.135, 0.145, 0.160, 1.0),
	"panel_disabled": Color(0.050, 0.052, 0.058, 1.0),
	"border": Color(0.24, 0.26, 0.30, 0.95),
	"accent": Color(0.74, 0.80, 0.88, 1.0),
	"text": Color(0.90, 0.92, 0.95, 1.0),
	"text_dim": Color(0.58, 0.62, 0.68, 1.0),
	"text_disabled": Color(0.42, 0.45, 0.50, 1.0),
	"selection": Color(0.32, 0.36, 0.44, 1.0),
	"guide": Color(0.18, 0.19, 0.22, 1.0),
	"preview_bg": Color(0.015, 0.017, 0.020, 1.0),
	"ambient": Color(0.40, 0.43, 0.48, 1.0),
	"status": Color(0.78, 0.86, 0.94, 1.0),
	"error": Color(1.0, 0.45, 0.38, 1.0),
}

const LIGHT := {
	"bg": Color(0.90, 0.91, 0.93, 1.0),
	"panel": Color(0.96, 0.965, 0.975, 1.0),
	"panel_alt": Color(0.88, 0.895, 0.915, 1.0),
	"panel_hover": Color(0.82, 0.84, 0.87, 1.0),
	"panel_pressed": Color(0.75, 0.78, 0.82, 1.0),
	"panel_disabled": Color(0.91, 0.92, 0.935, 1.0),
	"border": Color(0.55, 0.58, 0.63, 0.95),
	"accent": Color(0.19, 0.25, 0.34, 1.0),
	"text": Color(0.10, 0.12, 0.15, 1.0),
	"text_dim": Color(0.38, 0.41, 0.46, 1.0),
	"text_disabled": Color(0.58, 0.61, 0.66, 1.0),
	"selection": Color(0.68, 0.75, 0.86, 1.0),
	"guide": Color(0.70, 0.72, 0.76, 1.0),
	"preview_bg": Color(0.84, 0.86, 0.88, 1.0),
	"ambient": Color(0.88, 0.88, 0.86, 1.0),
	"status": Color(0.22, 0.29, 0.38, 1.0),
	"error": Color(0.78, 0.16, 0.10, 1.0),
}

static func normalize_mode(mode: String) -> String:
	var trimmed := mode.strip_edges().to_lower()
	if trimmed == MODE_LIGHT:
		return MODE_LIGHT
	return MODE_DARK

static func next_mode(mode: String) -> String:
	return MODE_LIGHT if normalize_mode(mode) == MODE_DARK else MODE_DARK

static func color(mode: String, key: String) -> Color:
	var palette := LIGHT if normalize_mode(mode) == MODE_LIGHT else DARK
	if palette.has(key):
		return palette[key]
	return DARK.get(key, Color.WHITE)

static func style_box(mode: String, key: String, radius: int = 4, border_width: int = 1) -> StyleBoxFlat:
	var style := UIStyle.panel_style(color(mode, key), radius, color(mode, "border"), border_width)
	style.content_margin_left = PAD_X
	style.content_margin_right = PAD_X
	style.content_margin_top = PAD_Y
	style.content_margin_bottom = PAD_Y
	return style

static func preview_environment(mode: String) -> Environment:
	var environment := Environment.new()
	environment.background_mode = Environment.BG_COLOR
	environment.background_color = color(mode, "preview_bg")
	environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	environment.ambient_light_color = color(mode, "ambient")
	environment.ambient_light_energy = 0.85
	return environment

static func apply_to_tree(root: Node, mode: String) -> void:
	if root is Control:
		style_control(root as Control, mode)
	for child in root.find_children("*", "", true, false):
		if child is Control:
			style_control(child as Control, mode)
		elif child is PopupMenu:
			style_popup_menu(child as PopupMenu, mode)

static func style_control(control: Control, mode: String) -> void:
	if control is PanelContainer:
		_style_panel_container(control as PanelContainer, mode)
	elif control is TabContainer:
		_style_tab_container(control as TabContainer, mode)
	elif control is MenuBar:
		style_menu_bar(control as MenuBar, mode)
	elif control is CheckButton:
		_style_check_button(control as CheckButton, mode)
	elif control is OptionButton:
		_style_option_button(control as OptionButton, mode)
	elif control is SpinBox:
		_style_spin_box(control as SpinBox, mode)
	elif control is Button:
		_style_button(control as Button, mode)
	elif control is LineEdit:
		_style_line_edit(control as LineEdit, mode)
	elif control is Tree:
		_style_tree(control as Tree, mode)
	elif control is ItemList:
		_style_item_list(control as ItemList, mode)
	elif control is RichTextLabel:
		_style_rich_text(control as RichTextLabel, mode)
	elif control is Label:
		_style_label(control as Label, mode)

static func style_menu_bar(menu_bar: MenuBar, mode: String) -> void:
	menu_bar.add_theme_color_override("font_color", color(mode, "text"))
	menu_bar.add_theme_color_override("font_hover_color", color(mode, "text"))
	menu_bar.add_theme_color_override("font_focus_color", color(mode, "text"))
	menu_bar.add_theme_color_override("font_pressed_color", color(mode, "text"))
	menu_bar.add_theme_color_override("font_disabled_color", color(mode, "text_dim"))

static func style_popup_menu(popup: PopupMenu, mode: String) -> void:
	popup.add_theme_stylebox_override("panel", style_box(mode, "panel", 4, 1))
	popup.add_theme_stylebox_override("hover", style_box(mode, "panel_hover", 3, 0))
	popup.add_theme_color_override("font_color", color(mode, "text"))
	popup.add_theme_color_override("font_hover_color", color(mode, "text"))
	popup.add_theme_color_override("font_disabled_color", color(mode, "text_disabled"))

static func _style_panel_container(panel: PanelContainer, mode: String) -> void:
	panel.add_theme_stylebox_override("panel", style_box(mode, "panel", 0, 1))

static func _style_tab_container(tabs: TabContainer, mode: String) -> void:
	tabs.add_theme_stylebox_override("panel", style_box(mode, "panel", 0, 1))
	tabs.add_theme_stylebox_override("tab_selected", style_box(mode, "panel", 4, 1))
	tabs.add_theme_stylebox_override("tab_hovered", style_box(mode, "panel_hover", 4, 1))
	tabs.add_theme_stylebox_override("tab_unselected", style_box(mode, "panel_alt", 4, 1))
	tabs.add_theme_color_override("font_selected_color", color(mode, "text"))
	tabs.add_theme_color_override("font_hovered_color", color(mode, "text"))
	tabs.add_theme_color_override("font_unselected_color", color(mode, "text_dim"))
	tabs.add_theme_color_override("font_disabled_color", color(mode, "text_disabled"))

static func _style_button(button: Button, mode: String) -> void:
	button.add_theme_stylebox_override("normal", style_box(mode, "panel_alt", 4, 1))
	button.add_theme_stylebox_override("hover", style_box(mode, "panel_hover", 4, 1))
	button.add_theme_stylebox_override("pressed", style_box(mode, "panel_pressed", 4, 1))
	button.add_theme_stylebox_override("focus", style_box(mode, "panel_pressed", 4, 1))
	button.add_theme_stylebox_override("disabled", style_box(mode, "panel_disabled", 4, 1))
	button.add_theme_color_override("font_color", color(mode, "text"))
	button.add_theme_color_override("font_hover_color", color(mode, "text"))
	button.add_theme_color_override("font_pressed_color", color(mode, "text"))
	button.add_theme_color_override("font_focus_color", color(mode, "text"))
	button.add_theme_color_override("font_disabled_color", color(mode, "text_disabled"))

static func _style_check_button(button: CheckButton, mode: String) -> void:
	button.add_theme_color_override("font_color", color(mode, "text"))
	button.add_theme_color_override("font_hover_color", color(mode, "text"))
	button.add_theme_color_override("font_pressed_color", color(mode, "text"))
	button.add_theme_color_override("font_focus_color", color(mode, "text"))
	button.add_theme_color_override("font_disabled_color", color(mode, "text_disabled"))

static func _style_option_button(button: OptionButton, mode: String) -> void:
	_style_button(button, mode)
	style_popup_menu(button.get_popup(), mode)

static func _style_spin_box(spin: SpinBox, mode: String) -> void:
	spin.add_theme_stylebox_override("normal", style_box(mode, "panel", 4, 1))
	spin.add_theme_stylebox_override("focus", style_box(mode, "panel_alt", 4, 1))
	spin.add_theme_stylebox_override("read_only", style_box(mode, "panel_disabled", 4, 1))
	spin.add_theme_color_override("font_color", color(mode, "text"))
	spin.add_theme_color_override("font_disabled_color", color(mode, "text_disabled"))
	spin.add_theme_color_override("font_readonly_color", color(mode, "text_dim"))
	var line_edit := spin.get_line_edit()
	if line_edit:
		_style_line_edit(line_edit, mode)

static func _style_line_edit(edit: LineEdit, mode: String) -> void:
	edit.add_theme_stylebox_override("normal", style_box(mode, "panel", 4, 1))
	edit.add_theme_stylebox_override("focus", style_box(mode, "panel_alt", 4, 1))
	edit.add_theme_stylebox_override("read_only", style_box(mode, "panel_disabled", 4, 1))
	edit.add_theme_color_override("font_color", color(mode, "text"))
	edit.add_theme_color_override("font_placeholder_color", color(mode, "text_dim"))
	edit.add_theme_color_override("caret_color", color(mode, "accent"))
	edit.add_theme_color_override("selection_color", color(mode, "selection"))

static func _style_tree(tree: Tree, mode: String) -> void:
	tree.add_theme_stylebox_override("panel", style_box(mode, "panel", 0, 1))
	tree.add_theme_stylebox_override("focus", style_box(mode, "panel_alt", 0, 1))
	tree.add_theme_stylebox_override("selected", style_box(mode, "panel_pressed", 3, 0))
	tree.add_theme_stylebox_override("selected_focus", style_box(mode, "panel_pressed", 3, 0))
	tree.add_theme_color_override("font_color", color(mode, "text"))
	tree.add_theme_color_override("font_selected_color", color(mode, "text"))
	tree.add_theme_color_override("font_disabled_color", color(mode, "text_dim"))
	tree.add_theme_color_override("guide_color", color(mode, "guide"))
	tree.add_theme_color_override("relationship_line_color", color(mode, "border"))

static func _style_item_list(list: ItemList, mode: String) -> void:
	list.add_theme_stylebox_override("panel", style_box(mode, "panel", 4, 1))
	list.add_theme_stylebox_override("focus", style_box(mode, "panel_alt", 4, 1))
	list.add_theme_stylebox_override("selected", style_box(mode, "panel_pressed", 3, 0))
	list.add_theme_stylebox_override("selected_focus", style_box(mode, "panel_pressed", 3, 0))
	list.add_theme_color_override("font_color", color(mode, "text"))
	list.add_theme_color_override("font_selected_color", color(mode, "text"))
	list.add_theme_color_override("font_disabled_color", color(mode, "text_dim"))
	list.add_theme_color_override("guide_color", color(mode, "guide"))

static func _style_rich_text(label: RichTextLabel, mode: String) -> void:
	label.add_theme_stylebox_override("normal", style_box(mode, "panel", 0, 1))
	label.add_theme_color_override("default_color", color(mode, "text"))
	label.add_theme_color_override("font_selected_color", color(mode, "text"))
	label.add_theme_color_override("selection_color", color(mode, "selection"))

static func _style_label(label: Label, mode: String) -> void:
	label.add_theme_color_override("font_color", color(mode, "text"))
