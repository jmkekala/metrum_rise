# SPDX-License-Identifier: GPL-2.0-only

## The player's text for Rust's road refusal keys, shared by every host that shows a road
## verdict: the road tool's stroke preview, the Road Properties window's in-place edits
## (`ROAD-80`) and the road types' layout, name and library refusals (`ROAD-101`). Rust decides
## and names the refusal (`invalid_reason`, with the numbers the stroke preview carries) and
## words it (`RoadWording`, the one place the strings live, `ROAD-112` round V2).
extends RefCounted

## The text for `preview`'s `invalid_reason`, or `fallback` for a key without one (null: the
## default refusal text).
static func text(preview: Dictionary, fallback: Variant = null) -> String:
	return RoadWording.reason_text(preview, RoadWording.default_refusal() if fallback == null else String(fallback))
