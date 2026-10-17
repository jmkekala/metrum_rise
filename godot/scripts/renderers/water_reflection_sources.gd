# SPDX-License-Identifier: GPL-2.0-only

## One shared MultiMesh reflection instance per resident building/scaffold batch.
## Source events own residency; a dense round robin bounds transform/material polling.
extends RefCounted

# Building LODs are pixel-based, not distance bands. This explicit mirror-only cap is
# provisional; it does not alter source LOD selection or main-view visibility.
const VISIBILITY_END_M := 1000.0
const VISIBILITY_MARGIN_M := 32.0

var _scenario: RID
var _sources: Array[int] = []
var _slots := {}
var _states := {}
var _dirty := {}
var _cursor := 0
var _frames := 10
var _connections := {}

## Watches renderer children once, then only sources added or removed by their owners.
func setup(scenario: RID, frames: int, buildings: Node) -> void:
	_scenario = scenario
	_frames = frames
	if buildings != null:
		_watch(buildings)

func _watch(node: Node) -> void:
	_connect(node, &"child_entered_tree", _watch)
	_connect(node, &"tree_exiting", _unwatch.bind(node.get_instance_id()))
	if node.has_signal("reflection_batch_changed"):
		_connect(node, &"reflection_batch_changed", _mark_dirty)
	if node is MultiMeshInstance3D and bool(node.get_meta("reflection_source", false)):
		_register(node)
	for child in node.get_children():
		_watch(child)

func _connect(node: Node, signal_name: StringName, callback: Callable) -> void:
	if node.is_connected(signal_name, callback):
		return
	node.connect(signal_name, callback)
	var id := node.get_instance_id()
	if not _connections.has(id):
		_connections[id] = []
	_connections[id].append([signal_name, callback])

func _unwatch(id: int) -> void:
	_remove(id)
	var node := instance_from_id(id)
	if node != null and _connections.has(id):
		for connection in _connections[id]:
			node.disconnect(connection[0], connection[1])
	_connections.erase(id)

func _register(source: GeometryInstance3D) -> void:
	var id := source.get_instance_id()
	if _slots.has(id):
		return
	_slots[id] = _sources.size()
	_sources.append(id)
	_connect(source, &"visibility_changed", _mark_dirty.bind(source))
	_mark_dirty(source)

func _mark_dirty(source: GeometryInstance3D) -> void:
	var id := source.get_instance_id()
	if _slots.has(id):
		_dirty[id] = true

func _remove(id: int) -> void:
	_free_instances(id)
	_dirty.erase(id)
	if not _slots.has(id):
		return
	var slot: int = _slots[id]
	var last: int = _sources.back()
	_sources[slot] = last
	_sources.pop_back()
	_slots[last] = slot
	_slots.erase(id)
	if last != id:
		# A swap can move a source behind the cursor. Sync it through the event queue so
		# removal churn never extends the ten-frame visibility bound.
		_dirty[last] = true

## O(changed sources + resident source nodes / frames), independent of batch population.
## Shared buffers update directly; no sync walks surfaces or individual building transforms.
func sync() -> void:
	for id in _dirty:
		_sync(id)
	_dirty.clear()
	var budget := ceili(float(_sources.size()) / float(_frames))
	for _index in budget:
		if _sources.is_empty():
			break
		_cursor %= _sources.size()
		var id: int = _sources[_cursor]
		if instance_from_id(id) == null:
			_unwatch(id)
		else:
			_sync(id)
			_cursor += 1

func _sync(id: int) -> void:
	var source := instance_from_id(id) as MultiMeshInstance3D
	if source == null or not source.is_inside_tree() or not source.is_visible_in_tree() \
			or source.multimesh == null or source.multimesh.mesh == null:
		_free_instances(id)
		return
	var batch := source.multimesh
	var count := batch.visible_instance_count
	if count < 0:
		count = batch.instance_count
	if count == 0:
		_free_instances(id)
		return
	var state: Dictionary = _states[id] if _states.has(id) else {}
	if not state.is_empty() and state["base"] != batch:
		_free_instances(id)
		state = {}
	if state.is_empty():
		# Godot cannot override MultiMesh surfaces. The existing window shader clips only
		# when this instance parameter is enabled, so all source surfaces/schemes and
		# INSTANCE_CUSTOM window schedules stay shared, without geometry/material copies.
		var proxy := RenderingServer.instance_create2(batch.get_rid(), _scenario)
		RenderingServer.instance_geometry_set_shader_parameter(proxy, "water_reflection_proxy", true)
		RenderingServer.instance_geometry_set_cast_shadows_setting(proxy, RenderingServer.SHADOW_CASTING_SETTING_OFF)
		RenderingServer.instance_geometry_set_visibility_range(proxy, 0.0, VISIBILITY_END_M,
			0.0, VISIBILITY_MARGIN_M, RenderingServer.VISIBILITY_RANGE_FADE_DISABLED)
		state = {"instance": proxy, "base": batch}
		_states[id] = state
	var proxy: RID = state["instance"]
	var transform := source.global_transform
	var material := source.material_override
	if not state.has("transform") or state["transform"] != transform:
		RenderingServer.instance_set_transform(proxy, transform)
		state["transform"] = transform
	if not state.has("material") or state["material"] != material:
		RenderingServer.instance_geometry_set_material_override(proxy,
			material.get_rid() if material != null else RID())
		state["material"] = material
	if not state.has("aabb") or state["aabb"] != source.custom_aabb:
		RenderingServer.instance_set_custom_aabb(proxy, source.custom_aabb)
		state["aabb"] = source.custom_aabb
	if not state.has("margin") or state["margin"] != source.extra_cull_margin:
		RenderingServer.instance_set_extra_visibility_margin(proxy, source.extra_cull_margin)
		state["margin"] = source.extra_cull_margin

func _free_instances(id: int) -> void:
	if not _states.has(id):
		return
	RenderingServer.free_rid(_states[id]["instance"])
	_states.erase(id)

## Releases server instances before the mirror's World3D goes away.
func clear() -> void:
	for id in _connections:
		var node := instance_from_id(id)
		if node != null:
			for connection in _connections[id]:
				node.disconnect(connection[0], connection[1])
	_connections.clear()
	for state in _states.values():
		RenderingServer.free_rid(state["instance"])
	_states.clear()
	_dirty.clear()
	_slots.clear()
	_sources.clear()
