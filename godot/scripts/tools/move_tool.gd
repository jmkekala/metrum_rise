# SPDX-License-Identifier: GPL-2.0-only

## Road node move tool — click to select a node, drag to reposition it on the terrain.
##
## Extends NetworkTool.
## Rust methods called: get_closest_node(), get_node_pos(), move_network_node_step(),
## commit_network_node_move()
## A refused move retains the accepted node position and does not rebuild meshes. A drag step
## whose roads would take zoned land is pending (`ZONE-10`, decision D-move): Rust moves nothing,
## the tool outlines the lots it would take with the bulldoze highlight and names the buildings,
## and the release commits that one move, demolishing them, as one undo step.
## PgUp/PgDown adjusts height; Esc or right-click cancels and reverts to last committed position.
extends "res://scripts/tools/network_tool.gd"

const HighlightMesh := preload("res://scripts/renderers/highlight_mesh.gd")
const RoadReasonText := preload("res://scripts/ui/road_reason_text.gd")

enum State { IDLE, SELECTED, DRAGGING }
var current_state = State.IDLE

var selected_node_id: int = -1
var selected_node_pos: Vector3
# The drag's pending step (Rust's `move_network_node_step` answer with `pending`), committed at
# the release; empty when the last step moved or was refused.
var _pending: Dictionary = {}
var _pending_pos := Vector3.ZERO
var _demolish_mesh: MeshInstance3D = null
var _pending_label: Label

func _ready():
	super._ready()
	# Optional label for instructions
	var label = Label.new()
	label.position = Vector2(20, 160)
	label.add_theme_color_override("font_color", Color(0, 1, 0))
	label.text = "Move Tool Active\n[Click] Select Node\n[Drag] Move Planar\n[PgUp/PgDown] Adjust Height\n[Esc/RClick] Drop/Cancel"
	label.visible = false
	add_child(label)
	_pending_label = Label.new()
	_pending_label.position = Vector2(20, 260)
	_pending_label.add_theme_color_override("font_shadow_color", Color.BLACK)
	_pending_label.add_theme_constant_override("shadow_outline_size", 4)
	_pending_label.visible = false
	add_child(_pending_label)

func _process(delta):
	super._process(delta)
	
	if get_child_count() > 0 and get_child(0) is Label:
		get_child(0).visible = active
		
	if active and current_state == State.SELECTED:
		# Process keyboard height adjustments every frame for smooth nudging
		var y_shift = 0.0
		if Input.is_key_pressed(KEY_PAGEUP): y_shift += 2.0 * delta
		if Input.is_key_pressed(KEY_PAGEDOWN): y_shift -= 2.0 * delta
		
		if y_shift != 0.0:
			selected_node_pos.y += y_shift
			_apply_move(selected_node_pos)
			
	if active and current_state == State.DRAGGING:
		# Lock dragging to terrain X-Z strictly, preserving customized Y offset unless terrain drops drastically
		var ground_pos = get_terrain_interaction()
		if ground_pos != null:
			# Automatically float 0.001m above target terrain unless explicitly raised
			var terrain_h = simulation_node.get_world_surface_height(Vector2(ground_pos.x, ground_pos.z))
			# Keep the relative height difference, but drag planar!
			# Or simpler: Just set it exactly at terra level if dragging planarly!
			var target_pos = Vector3(ground_pos.x, terrain_h + 0.001, ground_pos.z)
			if (target_pos - selected_node_pos).length() > 0.1:
				selected_node_pos = target_pos
				_apply_move(selected_node_pos)

func _input(event):
	if not active: return

	if event is InputEventMouseButton:
		if event.button_index == MOUSE_BUTTON_LEFT:
			if event.pressed:
				_handle_left_click()
			else:
				if current_state == State.DRAGGING:
					current_state = State.SELECTED # Drop but keep selected
					_commit_pending()

					
		if event.button_index == MOUSE_BUTTON_RIGHT and event.pressed:
			cancel_move()

# The click's queries wait for the core, so a click during a frame is not lost (`CODE-25`).
func _handle_left_click():
	var pos_variant = get_terrain_interaction(true)
	if pos_variant == null: return
	var pos: Vector3 = pos_variant
	
	match current_state:
		State.IDLE:
			# Look for a node near the mouse to select
			var closest_pos = simulation_node.get_closest_network_point(pos, 5.0, true)
			if closest_pos != null:
				selected_node_id = simulation_node.get_closest_node(closest_pos, 5.0)
				if selected_node_id != -1:
					selected_node_pos = closest_pos
					current_state = State.DRAGGING
					
		State.SELECTED:
			# Re-click to drag
			var closest_pos = simulation_node.get_closest_network_point(pos, 5.0, true)
			if closest_pos != null:
				var clicked_id = simulation_node.get_closest_node(closest_pos, 5.0)
				if clicked_id == selected_node_id:
					current_state = State.DRAGGING
				else:
					# Clicked another node
					selected_node_id = clicked_id
					selected_node_pos = closest_pos
					current_state = State.DRAGGING

# One drag step: Rust moves the node, refuses the step, or answers it pending when it would take
# zoned land; a pending step keeps the pointer's position for the next step and the release.
func _apply_move(pos: Vector3):
	if selected_node_id == -1:
		return
	var step: Dictionary = simulation_node.move_network_node_step(selected_node_id, pos)
	if bool(step.get("pending", false)):
		_show_pending(step, pos)
		return
	_show_pending({}, pos)
	if not bool(step.get("moved", false)):
		selected_node_pos = simulation_node.get_node_pos(selected_node_id)
		return
	_rebuild_meshes()

# The release commits the pending step's move, taking the land it showed (one undo step).
func _commit_pending() -> void:
	if _pending.is_empty() or selected_node_id == -1:
		return
	var pos := _pending_pos
	_show_pending({}, pos)
	if simulation_node.commit_network_node_move(selected_node_id, pos):
		_rebuild_meshes()
	selected_node_pos = simulation_node.get_node_pos(selected_node_id)

func _rebuild_meshes() -> void:
	# Force mesh rebuilding globally
	var road_tool = get_node_or_null("../RoadTool")
	if road_tool:
		road_tool.update_main_mesh()
	if terrain_node:
		terrain_node.update_terrain_visuals()

# The pending step's lots (four corners each) in the bulldoze highlight and its label; an empty
# `step` clears both.
func _show_pending(step: Dictionary, pos: Vector3) -> void:
	_pending = step
	_pending_pos = pos
	var lots: PackedVector3Array = step.get("demolish_lots", PackedVector3Array())
	var mesh: Mesh = HighlightMesh.polygons(lots, 4)
	if mesh != null and _demolish_mesh == null:
		_demolish_mesh = HighlightMesh.instance()
		add_child(_demolish_mesh)
	if _demolish_mesh != null:
		_demolish_mesh.mesh = mesh
		_demolish_mesh.visible = mesh != null
	_pending_label.visible = not step.is_empty()
	_pending_label.text = RoadReasonText.text(step) if not step.is_empty() else ""

## Whether a drag step is pending its release (for the tests).
func has_pending_move() -> bool:
	return not _pending.is_empty()

func cancel_move():
	active = false
	current_state = State.IDLE
	selected_node_id = -1
	_show_pending({}, Vector3.ZERO)

# Visual overrides
func _update_preview():
	pass

func _update_node_visuals():
	super._update_node_visuals()
	
	# Highlight the selected node
	if selected_node_id != -1 and current_state != State.IDLE:
		if cursor_mesh:
			cursor_mesh.global_position = selected_node_pos
			cursor_mesh.global_position.y += 0.8
			cursor_mesh.visible = true
