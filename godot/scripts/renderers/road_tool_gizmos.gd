# SPDX-License-Identifier: GPL-3.0-or-later

## The road tool's drawing setup (`ROAD-112` rounds D, V, HU and R2-3): the palette and the
## materials, made once and handed to Rust with this node's world (`road_tool_attach_draw`).
## Rust's tick draws the gizmos and the batch ghost through the `RenderingServer` itself, and the
## footprint hue at the end of the process step that showed the preview (round R2-4b); this node
## draws nothing. The looks: one line material for every line look (`road_tool_line.gdshader`,
## constant on-screen width, no depth test, so the stroke's path stays visible where the preview
## terrain's grass covers the preview road): the stroke's centreline, solid and thick; the
## alignment guide that took the cursor and the angle ray dashed and strong, a guide near the
## cursor that has not taken it dashed and faint (round UI1); tangent legs, control polygons and
## handle stems in one guide look, dashed and thinner; an end's corner arc solid (round UI1; at a
## right angle its square, round UI2); the zoning-depth guide's outside solid and strong, its
## inside solid and faint (round V item 9); the grid snap's curbs solid in their own colour (item
## 8); the batch ghost's edges (rounds G1 and V2); the preview's join marks, solid and thick, the
## junction outline light cyan and an overlapped stretch a saturated red (`ROAD-118` round JS5e).
## The same looks depth-tested (`road_tool_line_depth.gdshader`) outline the hue's tunnel runs.
## Handles draw as filled round points of a constant size, each point's kind (round V2) picking the
## colour and size: a placed end, a control point, the chain's start, the live end (or the grabbed
## freeform grip), a joint and a handle (a height grip too). The footprint hue (round HU) fills the
## ground a draft's road takes, translucent, depth-tested, one material per state (valid, checking,
## refused and hatched); the batch ghost's fill one per verdict. The palette comes from the UI
## theme's colours (`ui_style.gd`).
extends Node3D

const LineShader := preload("res://scripts/shaders/road_tool_line.gdshader")
const LineDepthShader := preload("res://scripts/shaders/road_tool_line_depth.gdshader")
const HandleShader := preload("res://scripts/shaders/road_tool_handle.gdshader")
const HueShader := preload("res://scripts/shaders/road_tool_hue.gdshader")
# The hue: above the water (0), below the batch ghost's fill (9) and the tool's lines (10).
const HUE_RENDER_PRIORITY := 8

const PALETTE := {
	# Colour, width in pixels, dash and gap in pixels (0 dash: solid) per line look.
	"lines": [
		[Color(1.0, 1.0, 1.0, 0.82), 4.0, 0.0, 0.0],
		[Color(0.72, 0.74, 1.0, 0.72), 2.0, 9.0, 6.0],
		# The zoning-depth guide (round V item 9): its outside solid and strong, its inside solid
		# and fainter. Round UI1: thicker and more opaque, so both read on grass.
		[Color(0.72, 0.74, 1.0, 1.0), 5.0, 0.0, 0.0],
		[Color(0.72, 0.74, 1.0, 0.65), 3.0, 0.0, 0.0],
		# The grid snap's curbs (item 8): solid, in a colour that stands out on the preview road's
		# light edges.
		[Color(0.35, 0.82, 1.0, 0.95), 3.0, 0.0, 0.0],
		# The batch ghost's edges (round V2): the stroke's white when tier 1 accepts the stroke,
		# the HUD's refusal red when not.
		[Color(1.0, 1.0, 1.0, 0.82), 2.0, 0.0, 0.0],
		[Color(1.0, 0.40, 0.30, 0.95), 2.0, 0.0, 0.0],
		# An end's ground marker (round H2): its vertical line to the ground and its ring there,
		# solid, in the stroke's white.
		[Color(1.0, 1.0, 1.0, 0.9), 2.0, 0.0, 0.0],
		# The footprint hue's tunnel outlines (round HU), solid and depth-tested on the ground
		# over the bore, in the hue's colour by state: valid, checking, refused.
		[Color(0.35, 0.65, 1.0, 0.85), 2.0, 0.0, 0.0],
		[Color(1.0, 0.76, 0.18, 0.85), 2.0, 0.0, 0.0],
		[Color(1.0, 0.40, 0.30, 0.9), 2.0, 0.0, 0.0],
		# Round UI1: the alignment guide that took the cursor and the angle lock's ray, thicker and
		# more opaque than the shared guide look; a guide near the cursor that has not taken it,
		# fainter than that look; an end's corner arc or square (UI2), solid in the stroke's white.
		[Color(0.72, 0.74, 1.0, 0.95), 3.5, 12.0, 5.0],
		[Color(0.72, 0.74, 1.0, 0.45), 2.0, 9.0, 6.0],
		[Color(1.0, 1.0, 1.0, 0.9), 2.5, 0.0, 0.0],
		# The preview's join marks (`ROAD-118` round JS5e), solid, drawn over everything: the
		# junction outline a light cyan, thicker than the hue outline so it reads on the hue's blue
		# fill and on bare ground; a `roads_overlap` refusal's stretch a saturated red that reads as a
		# refusal on light concrete (the user's decision D-JS5e-1: a lighter red read salmon).
		[Color(0.55, 0.92, 1.0, 0.95), 4.0, 0.0, 0.0],
		[Color(1.0, 0.22, 0.18, 0.95), 4.0, 0.0, 0.0],
	],
	# The footprint hue's fill (round HU) by state: valid blue, checking the
	# chips' warning amber, refused the HUD's refusal red (also hatched); one alpha, depth-tested.
	"hue": [Color(0.35, 0.65, 1.0), Color(1.0, 0.76, 0.18), Color(1.0, 0.40, 0.30)],
	"hue_alpha": 0.30,
	# The batch ghost's fills (accepted, refused): each stroke's ribbon at its road's width.
	"ghost_fills": [Color(1.0, 1.0, 1.0, 0.18), Color(1.0, 0.40, 0.30, 0.28)],
	"handles": [
		Color(1.0, 1.0, 1.0, 0.95),
		Color(0.65, 0.65, 0.90, 0.95),
		Color(0.35, 0.95, 0.55, 0.95),
		Color(1.0, 1.0, 1.0, 0.95),
		Color(0.65, 0.65, 0.90, 0.95),
		Color(0.82, 0.82, 0.98, 0.95),
	],
	"handle_sizes_px": [14.0, 13.0, 14.0, 16.0, 12.0, 10.0],
}
var _simulation: Object
# Every line look in one material (round V2), shared with the batch ghost's edges, and the same
# looks depth-tested for the hue's tunnel outlines.
var _line_material := ShaderMaterial.new()
var _depth_line_material: ShaderMaterial
var _handle_material := ShaderMaterial.new()
# The hue's fill by state, the batch ghost's fill by verdict (untyped: Rust takes a plain array).
var _hue_materials := []
var _ghost_fills := []

func _init(simulation: Object) -> void:
	_simulation = simulation
	var colors := PackedColorArray()
	var widths := PackedFloat32Array()
	var dashes := PackedFloat32Array()
	var gaps := PackedFloat32Array()
	for look in PALETTE["lines"]:
		colors.append(look[0])
		widths.append(look[1])
		dashes.append(look[2])
		gaps.append(look[3])
	_line_material.shader = LineShader
	_line_material.set_shader_parameter("colors", colors)
	_line_material.set_shader_parameter("widths_px", widths)
	_line_material.set_shader_parameter("dashes_px", dashes)
	_line_material.set_shader_parameter("gaps_px", gaps)
	_line_material.render_priority = 10
	_depth_line_material = _line_material.duplicate()
	_depth_line_material.shader = LineDepthShader
	_depth_line_material.render_priority = 8
	# One handle surface: each point's kind (its UV) picks the colour and size (round V2).
	_handle_material.shader = HandleShader
	_handle_material.set_shader_parameter("colors", PackedColorArray(PALETTE["handles"]))
	_handle_material.set_shader_parameter("sizes_px", PackedFloat32Array(PALETTE["handle_sizes_px"]))
	_handle_material.render_priority = 11
	for state in PALETTE["hue"].size():
		var fill := ShaderMaterial.new()
		fill.shader = HueShader
		fill.set_shader_parameter("hue_alpha", PALETTE["hue_alpha"])
		fill.set_shader_parameter("hue_color", PALETTE["hue"][state])
		# A refusal is hatched as well as red.
		fill.set_shader_parameter("hatched", state == 2)
		fill.render_priority = HUE_RENDER_PRIORITY
		_hue_materials.append(fill)
	for color in PALETTE["ghost_fills"]:
		_ghost_fills.append(_fill_material(color))

func _enter_tree() -> void:
	if _attachable():
		_simulation.road_tool_attach_draw(get_world_3d().scenario, _line_material, _depth_line_material, _handle_material, _hue_materials, _ghost_fills)

func _exit_tree() -> void:
	if _attachable():
		_simulation.road_tool_detach_draw()

# Whether the simulation draws the tool (test mocks do not).
func _attachable() -> bool:
	return is_instance_valid(_simulation) and _simulation.has_method("road_tool_attach_draw")

# The ghost's fill: unshaded, translucent, over the terrain like the lines (no depth test), just
# under them.
static func _fill_material(color: Color) -> StandardMaterial3D:
	var material := StandardMaterial3D.new()
	material.albedo_color = color
	material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	material.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	material.cull_mode = BaseMaterial3D.CULL_DISABLED
	material.no_depth_test = true
	material.render_priority = 9
	return material
