# SPDX-License-Identifier: GPL-2.0-only

## Screen-space hover outlines and constant-size site handles, clipped to the preview pane.
## Receives projected geometry only when the picker invalidates; no per-frame drawing work.
extends Control

const HOVER := Color(0.2, 0.95, 1.0)
const HALO := Color(0.04, 0.07, 0.09, 0.72)
var lines := PackedVector2Array()
var anchors := PackedVector2Array()
var vertices := PackedVector2Array()
var marker := Vector2.INF
var marker_radius := 0.0
var reference := Vector2.INF
var reference_selected := false

func clear() -> void:
	lines.clear()
	anchors.clear()
	vertices.clear()
	marker = Vector2.INF
	reference = Vector2.INF
	queue_redraw()

func _draw() -> void:
	var origin := global_position
	for index in range(0, lines.size(), 2):
		draw_line(lines[index] - origin, lines[index + 1] - origin, HALO, 3.5, true)
		draw_line(lines[index] - origin, lines[index + 1] - origin, HOVER, 1.5, true)
	for point in anchors:
		_ring(point - origin, 12.0, Color(0.7, 0.85, 1.0, 0.65))
	for point in vertices:
		_ring(point - origin, 8.0, Color(1.0, 0.85, 0.12))
	if marker != Vector2.INF:
		_ring(marker - origin, marker_radius, HOVER)
	if reference != Vector2.INF:
		_ring(reference - origin, 10.0, HOVER if reference_selected else Color(1.0, 0.85, 0.1))

func _ring(point: Vector2, radius: float, color: Color) -> void:
	draw_arc(point, radius, 0, TAU, 32, HALO, 3.5, true)
	draw_arc(point, radius, 0, TAU, 32, color, 1.5, true)
