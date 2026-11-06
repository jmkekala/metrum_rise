# SPDX-License-Identifier: GPL-2.0-only

## The roundabout tool (`ROAD-62`, D11-D14): the Roads menu's "Roundabout" button or O activates
## it; a click builds a ring in place of the junction under the pointer, or a bare ring on empty
## ground. `[` and `]` step the diameter by 2 m, the Roads options panel's Diameter slider by 1 m,
## both held to the range Rust gives for the site. Rust owns the site, the ring's lanes, the
## verdict and the transaction: this script draws the ring outline and arm cuts `preview_roundabout`
## returns, blue when it would build, red with `road_reason_text` when not, and sends the click.
## Rust methods called: get_roundabout_site(), preview_roundabout(), place_roundabout(),
## get_road_tool_surface_generation(), get_zoning_overlay_revision(),
## get_agriculture_field_overlay_revision().
extends "res://scripts/tools/network_tool.gd"

## The tool was switched on or off, for the Roads menu.
signal active_changed(enabled: bool)
## The diameter or the range under the pointer changed, for the Diameter row.
signal size_changed(diameter_m: float, min_m: float, max_m: float, lanes: int)

const RoadReasonText := preload("res://scripts/ui/road_reason_text.gd")
## What `[` and `]` move the diameter by (D12).
const KEY_STEP_M := 2.0
# The overlay's ribbon, above the road surface.
const OUTLINE_WIDTH_M := 0.8
const OUTLINE_LIFT_M := 0.6
# Half the length of an arm's cut mark.
const CUT_MARK_HALF_M := 4.0

# The diameter chosen per ring lanes; a site's range holds it without changing it.
var _diameters := {1: 36.0, 2: 50.0}
var _site: Dictionary = {}
var _preview: Dictionary = {}
var _key: Array = []
var _point := Vector3.ZERO
var _label: Label

func _ready() -> void:
	super._ready()
	var canvas := CanvasLayer.new()
	add_child(canvas)
	_label = Label.new()
	_label.position = Vector2(20, 120)
	_label.visible = false
	_label.add_theme_color_override("font_shadow_color", Color.BLACK)
	_label.add_theme_constant_override("shadow_outline_size", 4)
	canvas.add_child(_label)

## Switches the tool on or off.
func set_tool_active(enabled: bool) -> void:
	active = enabled
	_key = []
	_label.visible = enabled
	if not enabled:
		_clear()
	active_changed.emit(enabled)

## The diameter the next ring takes at the site under the pointer.
func current_diameter() -> float:
	var lanes := int(_site.get("lanes", 1))
	var diameter := float(_diameters.get(lanes, _site.get("default_diameter_m", 36.0)))
	return clampf(diameter, float(_site.get("min_diameter_m", 0.0)), float(_site.get("max_diameter_m", diameter)))

## Sets the ring's diameter at the current lanes (the Diameter row, the keys).
func set_diameter(diameter_m: float) -> void:
	_diameters[int(_site.get("lanes", 1))] = diameter_m
	_key = []

func _process(delta: float) -> void:
	super._process(delta)
	if active:
		_update_preview()

func _unhandled_input(event: InputEvent) -> void:
	if not active:
		return
	if event is InputEventKey and event.pressed and not event.echo:
		match event.keycode:
			KEY_BRACKETLEFT: _step_diameter(-KEY_STEP_M)
			KEY_BRACKETRIGHT: _step_diameter(KEY_STEP_M)
			_: return
		get_viewport().set_input_as_handled()
	elif event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT and event.pressed:
		_place()
		get_viewport().set_input_as_handled()

# The ring goes where the pointer stands on the ground, not where a road would snap it.
func get_world_mouse_pos() -> Vector3:
	var hit: Variant = get_terrain_interaction()
	return hit if hit != null else _last_world_mouse_pos

func _step_diameter(step_m: float) -> void:
	var min_m := float(_site.get("min_diameter_m", 0.0))
	var max_m := float(_site.get("max_diameter_m", 0.0))
	set_diameter(clampf(current_diameter() + step_m, min_m, max_m))

func _type_lanes() -> int:
	var road: Node = get_node_or_null("../RoadTool")
	return maxi(road.fwd_lanes, road.bkw_lanes) if road != null else 1

# The commit's verdict at the pointer, asked again only when something it reads changed.
func _update_preview() -> void:
	var pointer := _scripted_pointer_position if _scripted_pointer_enabled else _last_world_mouse_pos
	pointer.x = snappedf(pointer.x, 0.5)
	pointer.z = snappedf(pointer.z, 0.5)
	var lanes := _type_lanes()
	_site = simulation_node.get_roundabout_site(pointer, lanes)
	var diameter := current_diameter()
	var key: Array = [
		pointer, _site.node, diameter, lanes,
		simulation_node.get_road_tool_surface_generation(),
		simulation_node.get_zoning_overlay_revision(),
		simulation_node.get_agriculture_field_overlay_revision(),
	]
	if key == _key:
		return
	_key = key
	_point = pointer
	_preview = simulation_node.preview_roundabout(_point, diameter, lanes)
	is_valid = bool(_preview.get("is_valid", false))
	_show_outline()
	_label.text = _text(diameter)
	size_changed.emit(diameter, float(_site.min_diameter_m), float(_site.max_diameter_m), int(_site.lanes))

func _text(diameter: float) -> String:
	var lanes := int(_site.get("lanes", 1))
	var text := "Roundabout %.0f m, %d %s" % [diameter, lanes, "lane" if lanes == 1 else "lanes"]
	if not is_valid:
		return text + "\n" + RoadReasonText.text(_preview, "The roundabout cannot be placed here")
	var action := "replace the junction" if int(_site.get("node", -1)) >= 0 else "place"
	return text + " · %.0f\nClick to %s · [ / ] size" % [float(_preview.get("cost", 0.0)), action]

# The click re-reads the verdict at the pointer: never commit a pose that was not shown.
func _place() -> void:
	_key = []
	_update_preview()
	if not is_valid:
		return
	if simulation_node.place_roundabout(_point, current_diameter(), _type_lanes()):
		var road: Node = get_node_or_null("../RoadTool")
		if road != null:
			road.mark_network_topology_dirty()
		mark_network_nodes_dirty()
	_key = []

func _clear() -> void:
	_preview = {}
	if blueprint_mesh != null:
		blueprint_mesh.mesh = null

# The ring line and each arm's cut as one ribbon mesh in the blueprint material.
func _show_outline() -> void:
	var vertices := PackedVector3Array()
	var ring: PackedVector3Array = _preview.get("ring", PackedVector3Array())
	for i in range(ring.size() - 1):
		_ribbon(vertices, ring[i], ring[i + 1], OUTLINE_WIDTH_M * 0.5)
	var centre: Vector3 = _site.get("center", Vector3.ZERO)
	for arm: Dictionary in _preview.get("arms", []):
		var cut: Vector3 = arm.cut
		var along := Vector3(cut.z - centre.z, 0.0, centre.x - cut.x).normalized() * CUT_MARK_HALF_M
		_ribbon(vertices, cut - along, cut + along, OUTLINE_WIDTH_M)
	if vertices.is_empty():
		blueprint_mesh.mesh = null
		return
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = vertices
	var mesh := ArrayMesh.new()
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	blueprint_mesh.mesh = mesh

func _ribbon(vertices: PackedVector3Array, a: Vector3, b: Vector3, half_m: float) -> void:
	var side := Vector3(b.z - a.z, 0.0, a.x - b.x).normalized() * half_m
	var lift := Vector3.UP * OUTLINE_LIFT_M
	var quad := [a - side + lift, a + side + lift, b + side + lift, b - side + lift]
	vertices.append_array(PackedVector3Array([quad[0], quad[1], quad[2], quad[0], quad[2], quad[3]]))
