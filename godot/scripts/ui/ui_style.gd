## Shared style constants and helper factories for procedural UI.
##
## New UI code should use these helpers instead of open-coding StyleBoxFlat
## setup in each script.
extends RefCounted

const BG_DARK := Color(0.08, 0.08, 0.12, 0.93)
const BG_PANEL := Color(0.10, 0.10, 0.10, 0.80)
const BG_SUBMENU := Color(0.15, 0.15, 0.15, 0.70)
const BG_HUD_SHELL := Color(0.07, 0.07, 0.07, 0.72)
const BORDER_ACCENT := Color(0.30, 0.30, 0.45, 0.60)
const TEXT_PRIMARY := Color.WHITE
const TEXT_DIM := Color(0.72, 0.72, 0.72)
const TEXT_SECTION := Color(0.65, 0.65, 0.90)
const TEXT_ALERT := Color(1.00, 0.40, 0.30)
const ZONE_RESIDENTIAL := Color(0.20, 0.45, 0.25, 0.75)
const ZONE_COMMERCIAL := Color(0.20, 0.34, 0.62, 0.75)
const ZONE_INDUSTRIAL := Color(0.55, 0.47, 0.14, 0.75)

const CORNER_WINDOW := 8
const CORNER_PANEL := 12
const CORNER_SUB := 10

const PAD_WINDOW := 12
const PAD_PANEL := 15
const PAD_INNER := 8
const CURSOR_WINDOW_GAP := 40.0
const HUD_STRIP_HEIGHT := 60.0
const HUD_BUTTON_HEIGHT := 60.0
const HUD_BOTTOM_MARGIN := 20.0
const HUD_PANEL_GAP := 12.0
const HUD_LEFT_MARGIN := 20.0
const HUD_SHELL_CORNER := 15
const HUD_SHELL_PAD_X := 15
const HUD_SHELL_PAD_Y := 10

static func panel_style(
	bg_color: Color = BG_PANEL,
	corner_radius: int = CORNER_PANEL,
	border_color: Color = BORDER_ACCENT,
	border_width: int = 1
) -> StyleBoxFlat:
	var style := StyleBoxFlat.new()
	style.bg_color = bg_color
	style.set_corner_radius_all(corner_radius)
	style.border_width_left = border_width
	style.border_width_right = border_width
	style.border_width_top = border_width
	style.border_width_bottom = border_width
	style.border_color = border_color
	return style

static func window_body_style() -> StyleBoxFlat:
	return panel_style(BG_DARK, CORNER_WINDOW)

static func submenu_style() -> StyleBoxFlat:
	return panel_style(BG_SUBMENU, CORNER_SUB)

static func hud_shell_style() -> StyleBoxFlat:
	return panel_style(BG_HUD_SHELL, HUD_SHELL_CORNER, BORDER_ACCENT, 0)
