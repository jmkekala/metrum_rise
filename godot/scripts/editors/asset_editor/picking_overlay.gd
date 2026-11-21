# SPDX-License-Identifier: GPL-3.0-or-later

## Constant-size visible site handles, clipped to the preview pane.
## Occluded handles are omitted by the picker; world-space hover lines live in the preview.
## Route waypoints are squares, so they read apart from yard vertices and derived route points.
extends Control

const HOVER := Color(0.2, 0.95, 1.0)
const HALO := Color(0.04, 0.07, 0.09, 0.72)
var anchors := PackedVector2Array()
var vertices := PackedVector2Array()
var waypoints := PackedVector2Array()
var selected_waypoint := Vector2.INF
var marker := Vector2.INF
var marker_radius := 0.0
var reference := Vector2.INF
var reference_selected := false

func clear() -> void:
	anchors.clear()
	vertices.clear()
	waypoints.clear()
	selected_waypoint = Vector2.INF
	marker = Vector2.INF
	reference = Vector2.INF
	queue_redraw()

func _draw() -> void:
	var origin := global_position
	for point in anchors:
		_ring(point - origin, 12.0, Color(0.7, 0.85, 1.0, 0.65))
	for point in vertices:
		_ring(point - origin, 8.0, Color(1.0, 0.85, 0.12))
	for point in waypoints:
		_square(point - origin, 6.0, Color(1.0, 0.85, 0.12))
	if selected_waypoint != Vector2.INF:
		_square(selected_waypoint - origin, 9.0, HOVER)
	if marker != Vector2.INF:
		_ring(marker - origin, marker_radius, HOVER)
	if reference != Vector2.INF:
		_ring(reference - origin, 10.0, HOVER if reference_selected else Color(1.0, 0.85, 0.1))

func _ring(point: Vector2, radius: float, color: Color) -> void:
	draw_arc(point, radius, 0, TAU, 32, HALO, 3.5, true)
	draw_arc(point, radius, 0, TAU, 32, color, 1.5, true)

func _square(point: Vector2, half: float, color: Color) -> void:
	var rect := Rect2(point - Vector2(half, half), Vector2(half, half) * 2.0)
	draw_rect(rect, HALO, false, 3.5)
	draw_rect(rect, color, false, 1.5)
