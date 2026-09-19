# SPDX-License-Identifier: GPL-2.0-only

## Editable preview projection of a Rust-owned document part; never owns command history.
## LODs share its transform and LOD0 bounds.
## Preview choices are deliberately absent from the exported manifest.
extends RefCounted

const PreviewGeometry = preload("res://scripts/editors/asset_editor/preview_geometry.gd")

class Lod extends RefCounted:
	var source_path: String
	var file: String
	var distance_min_m := 0.0
	var distance_max_m = null
	var _authored: Dictionary = {}
	var _baseline: Dictionary = {}

	func _init(path: String, entry: Dictionary = {}) -> void:
		source_path = path
		file = str(entry.get("file", path.get_file()))
		distance_min_m = PreviewGeometry.number(entry, "distance_min_m", 0.0)
		distance_max_m = entry.get("distance_max_m", null)
		_authored = entry.duplicate(true)
		_baseline = _current_manifest()

	func to_manifest() -> Dictionary:
		var current := _current_manifest()
		if _authored.is_empty():
			return current
		var result := _authored.duplicate(true)
		for key in current:
			if current[key] != _baseline[key]:
				result[key] = current[key]
		return result

	func _current_manifest() -> Dictionary:
		return {"file": file, "distance_min_m": distance_min_m, "distance_max_m": distance_max_m}

var name: String
var position := Vector3.ZERO
var rotation_y := 0.0
var scale := 1.0
var pivot_offset := Vector3.ZERO
var aabb := AABB()
var lods: Array[Lod] = []
var _authored: Dictionary = {}
var _baseline: Dictionary = {}

func _init(path: String = "", label: String = "") -> void:
	name = label
	if not path.is_empty():
		lods.append(Lod.new(path))

func source_path() -> String:
	return lods[0].source_path if not lods.is_empty() else ""

func append_lod(path: String) -> void:
	var lod := Lod.new(path)
	if not lods.is_empty():
		var previous: Lod = lods.back()
		var defaults := [35.0, 70.0, 120.0]
		lod.distance_min_m = (
			float(previous.distance_max_m) if previous.distance_max_m != null
			else maxf(previous.distance_min_m + 35.0, defaults[mini(lods.size() - 1, 2)])
		)
		previous.distance_max_m = lod.distance_min_m
	lods.append(lod)

func remove_last_lod() -> void:
	if lods.size() > 1:
		lods.pop_back()
		lods.back().distance_max_m = null

func validation_error() -> String:
	var entries: Array[Dictionary] = []
	for index in lods.size():
		var lod := lods[index]
		if not FileAccess.file_exists(lod.source_path):
			return "Missing LOD%d: %s" % [index, lod.source_path]
		entries.append(lod.to_manifest())
	return AssetAuthoringPolicy.lod_chain_error(entries)

## Retain all loaded metadata; only subsequently edited properties replace authored values.
func retain_manifest(entry: Dictionary) -> void:
	_authored = entry.duplicate(true)
	_baseline = _current_manifest()

func to_manifest() -> Dictionary:
	var current := _current_manifest()
	if _authored.is_empty():
		return current
	var result := _authored.duplicate(true)
	for key in current:
		if current[key] != _baseline.get(key):
			result[key] = current[key]
	return result

func _current_manifest() -> Dictionary:
	var tiers: Array = []
	for lod in lods:
		tiers.append(lod.to_manifest())
	var raw_rotation = _authored.get("rotation_degrees")
	var rotation: Array = raw_rotation.duplicate() if raw_rotation is Array and raw_rotation.size() == 3 else [0.0, 0.0, 0.0]
	rotation[1] = rotation_y
	return {
		"name": name,
		"position": [position.x, position.y, position.z],
		"rotation_degrees": rotation,
		"scale": scale,
		"pivot_offset": [pivot_offset.x, pivot_offset.y, pivot_offset.z],
		"lods": tiers,
	}
