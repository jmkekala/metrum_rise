# SPDX-License-Identifier: GPL-2.0-only

## Stranded-walker notice and junction highlight (`ROAD-74`): the gridlock display's poll, rings
## and HUD line, read from Rust's stranded-walker report (`get_stranded_walkers_report`): the
## junctions where walkers wait with no route home, and how many, and the residents waiting on the
## road for a home after their household's eviction (`TRAFFIC-19`). Rust decides who is stranded
## and where; this node only shows the list, one HUD row under the gridlock notice.
extends "res://scripts/renderers/gridlock_display.gd"

const STRANDED_RING_COLOR := Color(0.25, 0.65, 1.0, 0.95)

func _init() -> void:
	ring_color = STRANDED_RING_COLOR
	notice_row = 1

func read_report() -> Dictionary:
	return simulation_node.get_stranded_walkers_report()

func notice_message(report: Dictionary) -> String:
	var junctions: int = (report["junctions"] as PackedInt32Array).size()
	var walkers := int(report["stranded"])
	var homeless := int(report.get("waiting_for_home", 0))
	var parts := PackedStringArray()
	if junctions > 0:
		parts.append("%d %s stranded at %d %s: no way home" % [
			walkers, "pedestrian" if walkers == 1 else "pedestrians",
			junctions, "junction" if junctions == 1 else "junctions"])
	if homeless > 0:
		parts.append("%d %s waiting for a home" % [homeless, "resident" if homeless == 1 else "residents"])
	return "; ".join(parts)
