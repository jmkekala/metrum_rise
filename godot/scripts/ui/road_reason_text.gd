# SPDX-License-Identifier: GPL-2.0-only

## The player's text for Rust's road refusal keys, shared by every host that shows a road
## verdict: the road tool's stroke preview, the Road Properties window's in-place edits
## (`ROAD-80`) and the road types' layout, name and library refusals (`ROAD-101`). Rust decides
## and names the refusal (`invalid_reason`, with the numbers the stroke preview carries); this
## only words it.
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
		"extraction_area_overlap":
			return "Road overlaps a mine's extraction area"
		"plotted_building_overlap":
			return "Road overlaps a placed building's lot"
		"pending_demolition":
			var count := int(preview.get("demolish_count", 0))
			return "Release to demolish %d building%s" % [count, "" if count == 1 else "s"]
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
			return "A walkway cannot become a road: draw a road in its place"
		"too_many_lanes":
			return "At most 4 lanes in each direction"
		"cross_section_cannot_change_transit_mode":
			return "A road cannot become a walkway"
		"cross_section_unchanged":
			return "No change"
		"sidewalk_under_door":
			return "A building's door is on this sidewalk: remove the building first"
		"edit_tunnel_width":
			return "Widen a tunnel by drawing it again"
		"cross_section_too_wide_for_curve":
			return "Too wide for this curve"
		# Turn pockets (`ROAD-95`): the pocket edit and the strokes and edits it refuses.
		"pocket_road_too_short":
			return "Road too short for a pocket here"
		"pocket_movement_missing":
			return "The junction has no such turn from this road"
		"pocket_junction_whitelisted":
			return "Clear the junction's turn rules first"
		"pocket_median_gap":
			return "The median cannot continue past a pocket here"
		"pocket_requires_surface_road":
			return "Turn pockets are for roads on the ground"
		"pocket_lane_limit":
			return "At most 4 lanes in each direction, the pocket included"
		"pocket_requires_wide_median":
			return "A left turn pocket needs a raised median at least 3.8 m wide"
		"invalid_pocket_dimensions":
			return "Pocket length 20 to 300 m, in steps of 10"
		"pocket_unchanged":
			return "No change"
		"pocket_transition_changed":
			return "The pocket's road has changed: remove the pocket and add it again"
		"pocket_edit_first":
			return "Remove the turn pocket first"
		"stroke_splits_pocket":
			return "Roads cannot cross or join a turn pocket: remove it first"
		"roundabout_fixed":
			return "A roundabout's ring cannot be changed"
		"roundabout_crossed":
			return "Roads cannot cross a roundabout: end the road on its ring"
		"roundabout_island":
			return "Roads cannot go on a roundabout's island"
		"roundabout_arm_node_taken":
			return "This point of the roundabout already has a road"
		"roundabout_neighbour_nodes":
			return "A road cannot link two neighbouring points of a roundabout"
		"roundabout_arms_too_close":
			return "Arms need more room. Increase the diameter"
		"roundabout_overlaps_road":
			return "Another road occupies this site"
		"roundabout_requires_three_to_six_arms":
			return "Choose a junction with three to six roads"
		"roundabout_requires_ordinary_surface_arms":
			return "Every road here must be an ordinary surface road"
		"roundabout_has_lane_restrictions":
			return "Remove the junction's lane rules first"
		"roundabout_arm_has_pocket":
			return "Remove the turn pockets at this junction first"
		"roundabout_arm_too_short":
			return "A road here is too short for this diameter"
		"roundabout_replacement_too_large":
			return "A junction takes a roundabout of at most 60 m"
		"invalid_roundabout_junction":
			return "This junction no longer exists"
		"invalid_roundabout_dimensions":
			return "This diameter does not suit this roundabout"
		"median_requires_both_directions":
			return "A median needs lanes in both directions"
		"median_requires_vehicle_road":
			return "Only roads with lanes can have a median"
		"median_width_out_of_range":
			return "Median width out of range for its style"
		"unknown_median_style":
			return "Unknown median style"
		# Road types and their library (`ROAD-101`): the layout check, names, library edits.
		"layout_footpath_alone":
			return "A footpath is a walkway on its own"
		"layout_sidewalks":
			return "A sidewalk goes at an end of the road"
		"sidewalks_out_of_range":
			return "Unknown sidewalk sides"
		"layout_two_medians":
			return "One median at most"
		"layout_lane_order":
			return "Lanes against traffic go on the left"
		"layout_no_car_lane":
			return "A road needs at least one car lane"
		"layout_lanes_split":
			return "A median sits between the two directions, not inside one"
		"layout_median_at_edge":
			return "A median sits between the two directions"
		# Kerbside parking (`PARK-05`).
		"layout_parking_at_kerb":
			return "A parking lane goes at the kerb, the outermost part of the road"
		"layout_parking_needs_sidewalk":
			return "A parking lane needs a sidewalk on its side"
		"parking_requires_vehicle_road":
			return "Only roads with lanes can have parking"
		"parking_code_out_of_range":
			return "Unknown parking setting"
		"layout_unknown_part":
			return "Unknown road part"
		"speed_out_of_range":
			return "Speed between 20 and 120 km/h, in steps of 10"
		"name_empty":
			return "Enter a name"
		"name_too_long":
			return "At most 40 characters"
		"name_invalid_characters":
			return "No line breaks or control characters"
		"name_taken":
			return "Another road type has this name"
		"road_type_builtin":
			return "Built-in road types cannot be changed"
		"road_type_unknown":
			return "This road type no longer exists"
		"road_type_duplicate_id":
			return "Another road type in the library has the same id"
		"entry_unreadable":
			return "This road type could not be read from the library"
		"library_unreadable":
			return "The road library file could not be read: fix or remove it to save changes"
		"library_newer_version":
			return "The road library is from a newer version of the game: read-only"
		"library_write_failed":
			return "The road library could not be saved"
		"node_piece_failed":
			return "A junction at the road's end cannot fit this width"
		"road_commit_rejected":
			return "Road could not be built; adjust the road and try again"
		_:
			return fallback
