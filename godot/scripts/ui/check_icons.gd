# SPDX-License-Identifier: GPL-2.0-only

## The game's check box and switch icons, set on Godot's default theme so every CheckBox and
## CheckButton (in-game windows, cards and editors) uses them: every theme lookup ends there,
## while a theme on the root window stops at the game UI's CanvasLayer. Godot's defaults draw an unchecked
## box and an off switch so dark that on the game's dark panels they read as stray text; here the
## off states keep a visible outline and track, and the on states fill with the HUD accent.
## Drawn from SVG at the UI scale, at Godot's default icon sizes (16 x 16 box, 32 x 16 switch), so
## layouts measure the same; `install` again after the UI scale changes.
extends RefCounted

const UIStyle = preload("res://scripts/ui/ui_style.gd")

const DISABLED_OPACITY := 0.4

## Sets the check icons on the default theme.
static func install() -> void:
	var theme := ThemeDB.get_default_theme()
	var accent := "#" + UIStyle.HUD_ACCENT.to_html(false)
	var scale := UIStyle.ui_scale()
	for state in [["", 1.0], ["_disabled", DISABLED_OPACITY]]:
		var suffix: String = state[0]
		var opacity: float = state[1]
		theme.set_icon("unchecked" + suffix, "CheckBox", _icon(_box(false, accent), 16, opacity, scale))
		theme.set_icon("checked" + suffix, "CheckBox", _icon(_box(true, accent), 16, opacity, scale))
		theme.set_icon("radio_unchecked" + suffix, "CheckBox", _icon(_radio(false, accent), 16, opacity, scale))
		theme.set_icon("radio_checked" + suffix, "CheckBox", _icon(_radio(true, accent), 16, opacity, scale))
		for mirrored in [false, true]:
			var tail: String = ("_mirrored" if mirrored else "") + suffix
			theme.set_icon("unchecked" + tail, "CheckButton", _icon(_switch(false, mirrored, accent), 32, opacity, scale))
			theme.set_icon("checked" + tail, "CheckButton", _icon(_switch(true, mirrored, accent), 32, opacity, scale))

static func _box(on: bool, accent: String) -> String:
	if not on:
		return '<rect x="1.75" y="1.75" width="12.5" height="12.5" rx="3.25" fill="#ffffff" fill-opacity="0.07" stroke="#ffffff" stroke-opacity="0.62" stroke-width="1.5"/>'
	return ('<rect x="1" y="1" width="14" height="14" rx="3.5" fill="%s"/>' % accent
		+ '<path d="M4.4 8.2 L6.9 10.7 L11.7 5.5" fill="none" stroke="#0c1417" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"/>')

static func _radio(on: bool, accent: String) -> String:
	if not on:
		return '<circle cx="8" cy="8" r="6.25" fill="#ffffff" fill-opacity="0.07" stroke="#ffffff" stroke-opacity="0.62" stroke-width="1.5"/>'
	return ('<circle cx="8" cy="8" r="6.25" fill="none" stroke="%s" stroke-width="1.5"/>' % accent
		+ '<circle cx="8" cy="8" r="3.4" fill="%s"/>' % accent)

# A pill track with a round knob: off is a light outline with the knob at the start, on is the
# accent with a white knob at the end (the ends swap for right-to-left layouts).
static func _switch(on: bool, mirrored: bool, accent: String) -> String:
	var knob_x := 23.5 if on != mirrored else 8.5
	if not on:
		return ('<rect x="1.6" y="1.6" width="28.8" height="12.8" rx="6.4" fill="#ffffff" fill-opacity="0.10" stroke="#ffffff" stroke-opacity="0.50" stroke-width="1.2"/>'
			+ '<circle cx="%.1f" cy="8" r="4.3" fill="#d4d8de"/>' % knob_x)
	return ('<rect x="1" y="1" width="30" height="14" rx="7" fill="%s"/>' % accent
		+ '<circle cx="%.1f" cy="8" r="5" fill="#ffffff"/>' % knob_x)

static func _icon(body: String, width: int, opacity: float, scale: float) -> ImageTexture:
	var svg := '<svg xmlns="http://www.w3.org/2000/svg" width="%d" height="16" viewBox="0 0 %d 16"><g opacity="%.2f">%s</g></svg>' % [
		width, width, opacity, body]
	var image := Image.new()
	var err := image.load_svg_from_string(svg, scale)
	if err != OK:
		push_error("Check icon SVG failed to load (%s): %s" % [error_string(err), svg])
	return ImageTexture.create_from_image(image)
