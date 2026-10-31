# SPDX-License-Identifier: GPL-2.0-only

## Gridlock notice and junction highlight (`TRAFFIC-14`). Reads Rust's last gridlock report
## (`get_gridlock_report`) twice a second and, only when its junctions differ from the ones
## shown, rebuilds a ring round every reported junction and updates the HUD notice. Every
## decision (which cycles are saturation, which junctions they hold) is made in Rust; this node
## only shows the list.
extends Node3D

const GridlockNotice = preload("res://scripts/ui/gridlock_notice.gd")
# The check runs once a game minute; half a second keeps up with it at normal speed.
const POLL_S := 0.5
const RING_RADIUS_M := 14.0
const RING_SEGMENTS := 32
# Above the road surface so the ring is not hidden in it.
const RING_LIFT_M := 1.0
const RING_COLOR := Color(1.0, 0.25, 0.15, 0.95)

var simulation_node: Node
## The junction node ids the highlight and the notice show, ascending.
var shown_junctions := PackedInt32Array()
var _shown_positions := PackedVector3Array()

var _notice: Control
var _mesh := ArrayMesh.new()
var _elapsed_s := POLL_S

func _ready() -> void:
	if simulation_node == null:
		simulation_node = get_node_or_null("../SimulationNode")
	var material := StandardMaterial3D.new()
	material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	material.vertex_color_use_as_albedo = true
	material.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	material.no_depth_test = true
	var mesh_instance := MeshInstance3D.new()
	mesh_instance.mesh = _mesh
	mesh_instance.material_override = material
	mesh_instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	add_child(mesh_instance)
	var layer := CanvasLayer.new()
	add_child(layer)
	_notice = GridlockNotice.new()
	layer.add_child(_notice)

func _process(delta: float) -> void:
	_elapsed_s += delta
	if _elapsed_s < POLL_S or simulation_node == null:
		return
	_elapsed_s = 0.0
	refresh()

## Reads the report now and updates the notice and highlight if its junctions changed.
func refresh() -> void:
	# From the render snapshot: never waits for the simulation core.
	var report: Dictionary = simulation_node.get_gridlock_report()
	var junctions: PackedInt32Array = report["junctions"]
	var positions: PackedVector3Array = report["positions"]
	if junctions == shown_junctions and positions == _shown_positions:
		return
	shown_junctions = junctions
	_shown_positions = positions
	_rebuild_rings(positions)
	_notice.show_junctions(junctions.size())

## The notice's text while it shows, else empty.
func notice_text() -> String:
	return _notice.text()

func _rebuild_rings(positions: PackedVector3Array) -> void:
	_mesh.clear_surfaces()
	if positions.is_empty():
		return
	var points := PackedVector3Array()
	for center in positions:
		# One closed line loop per junction, as line pairs.
		for k in RING_SEGMENTS:
			for end in [k, k + 1]:
				var angle := TAU * float(end) / float(RING_SEGMENTS)
				points.push_back(center + Vector3(cos(angle) * RING_RADIUS_M, RING_LIFT_M, sin(angle) * RING_RADIUS_M))
	var colors := PackedColorArray()
	colors.resize(points.size())
	colors.fill(RING_COLOR)
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = points
	arrays[Mesh.ARRAY_COLOR] = colors
	_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_LINES, arrays)
