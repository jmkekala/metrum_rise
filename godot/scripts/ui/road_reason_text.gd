# SPDX-License-Identifier: GPL-2.0-only

## The player's text for Rust's road refusal keys, shared by every host that shows a road
## verdict: the road tool's stroke preview and the Road Properties window's in-place edits
## (`ROAD-80`). Rust decides and names the refusal (`invalid_reason`, with the numbers the
## stroke preview carries); this only words it.
extends RefCounted

## The text for `preview`'s `invalid_reason`, or `fallback` for a key without one.
static func text(preview: Dictionary, fallback: String = "Road cannot be placed") -> String:
	var reason := String(preview.get("invalid_reason", ""))
	match reason:
		"bridge_clearance":
			return "Bridge clearance %.1f m / %.1f m" % [
				float(preview.get("clearance_m", 0.0)),
				float(preview.get("required_clearance_m", 0.0)),
			]
		"tunnel_cover":
			return "Tunnel cover %.1f m / %.1f m: draw the ends outside the hill" % [
				float(preview.get("clearance_m", 0.0)),
				float(preview.get("required_clearance_m", 0.0)),
			]
		"tunnel_approach_too_short":
			return "Tunnel approach %.1f m, needs %.1f m: start further from the hill" % [
				float(preview.get("geometry_measured", 0.0)),
				float(preview.get("geometry_limit", 0.0)),
			]
		"tunnel_too_close":
			return "Too close above a tunnel: %.1f m / %.1f m over its deck" % [
				float(preview.get("clearance_m", 0.0)),
				float(preview.get("required_clearance_m", 0.0)),
			]
		"bridge_too_low":
			return "Too low under a bridge: %.2f m / %.2f m deck to deck" % [
				float(preview.get("clearance_m", 0.0)),
				float(preview.get("required_clearance_m", 0.0)),
			]
		"tunnel_join":
			return "Roads cannot join a tunnel inside its bore: join it at a portal"
		"tunnel_crossing":
			return "Tunnels cannot cross other tunnels or meet roads underground"
		"junction_in_trench":
			return "Junction %.1f m below the ground in a tunnel approach: join where the road meets the ground" % [
				float(preview.get("geometry_measured", 0.0)),
			]
		"tunnel_under_water":
			return "Tunnels cannot pass under water"
		"water_requires_bridge":
			return "Water crossing requires a bridge"
		"field_overlap":
			return "Road overlaps a farm field"
		"zoning_cell_overlap":
			return "Road overlaps painted zoning cells"
		"surface_geometry_invalid":
			return "Road surface compilation failed"
		"terrain_clip_missing_outer_boundary_owner":
			return "Road terrain boundary could not be built"
		"parcel_overlap":
			return "Road overlaps existing zoning parcels"
		"road_grade_change":
			return "Road bends too sharply up or down"
		"road_too_steep":
			if not preview.has("geometry_measured"):
				return "Road too steep"
			return "Road too steep: %.0f%%, at most %.0f%%" % [
				float(preview.get("geometry_measured", 0.0)) * 100.0,
				float(preview.get("geometry_limit", 0.0)) * 100.0,
			]
		"road_tight_curve":
			return "Curve too tight: radius %.1f m, needs %.1f m" % [
				float(preview.get("geometry_measured", 0.0)),
				float(preview.get("geometry_limit", 0.0)),
			]
		"junction_arm_angle":
			return "Roads meet at %.0f°; junction arms need %.0f°" % [
				float(preview.get("geometry_measured", 0.0)),
				float(preview.get("geometry_limit", 0.0)),
			]
		"junction_corner_does_not_fit":
			return "Junction corner does not fit; lengthen the roads between junctions"
		"junction_core_not_simple":
			return "Junction roads overlap; widen the angle between them"
		"junction_band_mismatch":
			return "These road types cannot meet at a junction yet"
		"junction_arm_too_short":
			return "Road too short for its junction"
		"junction_profile_mismatch":
			return "Junction heights do not fit; adjust the road grade"
		"junction_layout_invalid":
			return "Junction could not be laid out"
		"junction_degree":
			return "Too many roads at one junction (%d, at most %d)" % [
				int(preview.get("geometry_measured", 0.0)),
				int(preview.get("geometry_limit", 0.0)),
			]
		"same_node_connection":
			return "Road endpoints connect to the same junction"
		"too_short":
			return "Road too short"
		"validation_unavailable":
			return "Road check unavailable"
		"invalid_road":
			return "The road no longer exists"
		"cross_section_not_road":
			return "Only roads with lanes can change their lanes"
		"too_many_lanes":
			return "At most 4 lanes in each direction"
		"cross_section_cannot_change_transit_mode":
			return "A road keeps at least one lane"
		"cross_section_unchanged":
			return "No change"
		"edit_tunnel_width":
			return "Widen a tunnel by drawing it again"
		"cross_section_too_wide_for_curve":
			return "Too wide for this curve"
		"node_piece_failed":
			return "A junction at the road's end cannot fit this width"
		"road_commit_rejected":
			return "Road could not be built; adjust the road and try again"
		_:
			return fallback
