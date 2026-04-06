## Zoning paint tool — rectangle drag brush that paints world-space zone types.
##
## Rust methods called: set_zone_rect(), set_zone_rect_raw(), get_zone_subrect(),
##   intersect_terrain()
## Zone types: 0=Erase, 1=Residential, 2=Commercial, 3=Industrial, 4=Office, 5=Mixed.
## Zone type is set externally by InputManager (keys 1–5 / 0).
## Drag left mouse button to paint a rectangle. Undo via InputManager Ctrl+Z.
extends Node3D

@onready var simulation_node = $"../SimulationNode"
@onready var zoning_overlay = $"../ZoningOverlay"

var current_zone_type: int = 1
var active: bool = false

# Drag state
var dragging: bool = false
var drag_start_world: Vector2 = Vector2.ZERO
var drag_end_world: Vector2 = Vector2.ZERO

# Snap to 10 m zone cell grid
const SNAP: float = 10.0

# Undo ring buffer (max 20 ops). Each entry: { x_min, z_min, x_max, z_max, bytes }
var undo_stack: Array = []
const UNDO_MAX: int = 20

# Preview quad shown while dragging
var preview_mesh: MeshInstance3D

func _ready():
	preview_mesh = MeshInstance3D.new()
	var mat = StandardMaterial3D.new()
	mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	mat.albedo_color = Color(0.2, 0.8, 0.2, 0.35)
	mat.cull_mode = BaseMaterial3D.CULL_DISABLED
	preview_mesh.material_override = mat
	add_child(preview_mesh)
	preview_mesh.top_level = true
	preview_mesh.visible = false

func _process(_delta):
	if not active:
		preview_mesh.visible = false
		return

	if dragging:
		var wp = _mouse_world_pos()
		if wp != null:
			drag_end_world = _snap(wp)
		_update_preview()

func _unhandled_input(event):
	if not active: return

	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT:
		if event.pressed:
			var wp = _mouse_world_pos()
			if wp != null:
				drag_start_world = _snap(wp)
				drag_end_world = drag_start_world
				dragging = true
		else:
			if dragging:
				_commit_paint()
			dragging = false
			preview_mesh.visible = false

## Paint the dragged rectangle. Called on mouse release.
func _commit_paint():
	var x_min = min(drag_start_world.x, drag_end_world.x)
	var z_min = min(drag_start_world.y, drag_end_world.y)
	var x_max = max(drag_start_world.x, drag_end_world.x)
	var z_max = max(drag_start_world.y, drag_end_world.y)

	if x_max - x_min < SNAP * 0.5 and z_max - z_min < SNAP * 0.5:
		return

	# Capture before state for undo
	var before = simulation_node.get_zone_subrect(x_min, z_min, x_max, z_max)
	_push_undo(x_min, z_min, x_max, z_max, before)

	simulation_node.set_zone_rect(x_min, z_min, x_max, z_max, current_zone_type)
	if zoning_overlay:
		zoning_overlay.mark_zone_dirty()

## Called by InputManager Ctrl+Z handler.
func undo():
	if undo_stack.is_empty(): return
	var op = undo_stack.pop_back()
	simulation_node.set_zone_rect_raw(op["x_min"], op["z_min"], op["x_max"], op["z_max"], op["bytes"])
	if zoning_overlay:
		zoning_overlay.mark_zone_dirty()

func _push_undo(x_min: float, z_min: float, x_max: float, z_max: float, bytes: PackedByteArray):
	undo_stack.append({ "x_min": x_min, "z_min": z_min, "x_max": x_max, "z_max": z_max, "bytes": bytes })
	if undo_stack.size() > UNDO_MAX:
		undo_stack.pop_front()

func _update_preview():
	var x_min = min(drag_start_world.x, drag_end_world.x)
	var z_min = min(drag_start_world.y, drag_end_world.y)
	var x_max = max(drag_start_world.x, drag_end_world.x)
	var z_max = max(drag_start_world.y, drag_end_world.y)
	var sx = x_max - x_min
	var sz = z_max - z_min

	if sx < 0.1 or sz < 0.1:
		preview_mesh.visible = false
		return

	var quad = QuadMesh.new()
	quad.size = Vector2(sx, sz)
	preview_mesh.mesh = quad
	preview_mesh.position = Vector3((x_min + x_max) * 0.5, 0.3, (z_min + z_max) * 0.5)
	preview_mesh.rotation_degrees = Vector3(-90, 0, 0)
	preview_mesh.visible = true

	var mat = preview_mesh.material_override as StandardMaterial3D
	mat.albedo_color = _zone_color(current_zone_type)

func _zone_color(z: int) -> Color:
	match z:
		1: return Color(0.1, 0.9, 0.1, 0.35)
		2: return Color(0.1, 0.4, 1.0, 0.35)
		3: return Color(1.0, 0.8, 0.1, 0.35)
		4: return Color(0.1, 0.8, 0.8, 0.35)
		5: return Color(0.8, 0.1, 0.8, 0.35)
		_: return Color(1.0, 0.2, 0.2, 0.35)

func _snap(wp: Vector2) -> Vector2:
	return Vector2(floor(wp.x / SNAP) * SNAP, floor(wp.y / SNAP) * SNAP)

func _mouse_world_pos() -> Variant:
	var mouse_pos = get_viewport().get_mouse_position()
	var camera = get_viewport().get_camera_3d()
	if camera == null: return null
	var ray_origin = camera.project_ray_origin(mouse_pos)
	var ray_dir = camera.project_ray_normal(mouse_pos)
	var hit = simulation_node.intersect_terrain(ray_origin, ray_dir)
	if hit == null: return null
	return Vector2(hit.x, hit.z)
