# SPDX-License-Identifier: GPL-2.0-only

## The stream benchmark's draw census (`RENDER-44`, `RENDER-46`; benchmark-only, untimed: it reads
## buffers and meshes back from the GPU). At a pose it writes `<dir>/<label>.census.json`:
##
## - `camera`: the main camera's transform, field of view, planes and the viewport size, and the
##   mesh level-of-detail threshold Godot picks levels with;
## - `godot`: the main viewport's own counters (draw calls, objects, primitives) of the main
##   (visible) and the shadow passes, the mean of a few frames;
## - `items`: every drawn geometry instance, the scene's nodes (`GeometryInstance3D` under Main,
##   named by the child of Main they stand under and their shadow policy category) and the stream
##   core's `RenderingServer` slots (vegetation, the building batches, the parked cars, the
##   graded cells, from their census functions): kind (`mesh`, `multimesh`), mesh, instance
##   transform, local bounds, visibility range, shadow casting, material override, and for a
##   MultiMesh its drawn instances' transforms (base64 float32, 12 a row);
## - `meshes`: per mesh its surfaces' bounds, index and vertex counts, material and level-of-detail
##   index counts with their edge lengths;
## - `layers`: per layer the main pass's counters with that layer hidden from the camera (each
##   layer's instances moved to a render layer of its own for the census, then restored; a layer
##   hidden from the camera also stops casting into its directional shadows), and the GPU time of
##   those frames (with METRUM_STREAM_CENSUS_HOLD, each layer state is held that
##   many frames more after a `[STREAM] BEGIN census_<state>` line, for an outside tracer);
## - with `depth`, `<label>.depth.png`: the frame's linear view depth, `log2(d) + 4` over 20
##   octaves in 16 bits (red the high byte, green the low), taken by a full-screen quad that reads
##   the depth texture, with tone mapping, anti-aliasing and the interface off for that frame.
##
## `tools/stream_benchmark.py census` reads them.
extends RefCounted

const ROW_FLOATS := 12
const COUNTER_FRAMES := 8
const SETTLE_FRAMES := 5
# Render layers 1-15 are free (the water reflection uses 16-19); bit 0 stays the default.
const FIRST_LAYER_BIT := 1
const LAST_LAYER_BIT := 15
const DEPTH_SHADER := """
shader_type spatial;
render_mode unshaded, depth_draw_never, depth_test_disabled, cull_disabled, fog_disabled, shadows_disabled;
uniform sampler2D depth_texture : hint_depth_texture, filter_nearest, repeat_disable;
vec3 to_linear(vec3 c) {
	return mix(pow((c + vec3(0.055)) / vec3(1.055), vec3(2.4)), c / vec3(12.92), lessThan(c, vec3(0.04045)));
}
void vertex() {
	POSITION = vec4(VERTEX.xy, 0.5, 1.0);
}
void fragment() {
	float raw = texture(depth_texture, SCREEN_UV).r;
	vec4 view = INV_PROJECTION_MATRIX * vec4(SCREEN_UV * 2.0 - 1.0, raw, 1.0);
	float d = max(-view.z / view.w, 0.0625);
	float q = floor(clamp((log2(d) + 4.0) / 20.0, 0.0, 1.0) * 65535.0 + 0.5);
	float hi = floor(q / 256.0);
	ALBEDO = to_linear(vec3(hi, q - hi * 256.0, 0.0) / 255.0);
	ALPHA = 1.0;
}
"""

var _bench: Node
var _main: Node
var _dir: String
var _meshes := {}

func _init(bench: Node, dir: String) -> void:
	_bench = bench
	_main = bench.get_parent()
	_dir = dir
	DirAccess.make_dir_recursive_absolute(dir)

## Writes the census of the current frame as `label` (see the header).
func capture(label: String, depth: bool) -> void:
	var viewport := _bench.get_viewport()
	var camera := viewport.get_camera_3d()
	await _frames(SETTLE_FRAMES)
	var census := {"label": label, "camera": _camera(viewport, camera), "godot": await _counters(viewport)}
	var items: Array = []
	_node_items(items, viewport)
	_slot_items(items)
	_graded_items(items)
	census["items"] = items
	census["meshes"] = _meshes
	census["layers"] = await _layer_counters(viewport, camera, items)
	if depth:
		await _depth(viewport, camera, _dir.path_join(label + ".depth.png"))
		census["depth"] = label + ".depth.png"
	var file := FileAccess.open(_dir.path_join(label + ".census.json"), FileAccess.WRITE)
	file.store_string(JSON.stringify(census))
	file.close()
	print("[STREAM] CENSUS %s items=%d meshes=%d" % [label, items.size(), _meshes.size()])

func _frames(count: int) -> void:
	for _frame in count:
		await _bench.get_tree().process_frame

func _camera(viewport: Viewport, camera: Camera3D) -> Dictionary:
	var size := viewport.get_visible_rect().size
	return {"transform": _transform(camera.global_transform), "fov": camera.fov, "keep_aspect": camera.keep_aspect,
		"near": camera.near, "far": camera.far, "size": [size.x, size.y],
		"lod_threshold": viewport.mesh_lod_threshold, "cull_mask": camera.cull_mask}

# The main viewport's main-pass and shadow-pass counters and GPU time, means of a few frames.
func _counters(viewport: Viewport) -> Dictionary:
	var sums := [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
	for _frame in COUNTER_FRAMES:
		await _bench.get_tree().process_frame
		var values := [
			viewport.get_render_info(Viewport.RENDER_INFO_TYPE_VISIBLE, Viewport.RENDER_INFO_DRAW_CALLS_IN_FRAME),
			viewport.get_render_info(Viewport.RENDER_INFO_TYPE_VISIBLE, Viewport.RENDER_INFO_OBJECTS_IN_FRAME),
			viewport.get_render_info(Viewport.RENDER_INFO_TYPE_VISIBLE, Viewport.RENDER_INFO_PRIMITIVES_IN_FRAME),
			viewport.get_render_info(Viewport.RENDER_INFO_TYPE_SHADOW, Viewport.RENDER_INFO_DRAW_CALLS_IN_FRAME),
			viewport.get_render_info(Viewport.RENDER_INFO_TYPE_SHADOW, Viewport.RENDER_INFO_OBJECTS_IN_FRAME),
			viewport.get_render_info(Viewport.RENDER_INFO_TYPE_SHADOW, Viewport.RENDER_INFO_PRIMITIVES_IN_FRAME),
			RenderingServer.viewport_get_measured_render_time_gpu(viewport.get_viewport_rid())]
		for index in values.size():
			sums[index] += float(values[index]) / COUNTER_FRAMES
	return {"visible": sums.slice(0, 3), "shadow": sums.slice(3, 6), "gpu_ms": sums[6]}

static func _transform(t: Transform3D) -> Array:
	return [t.basis.x.x, t.basis.y.x, t.basis.z.x, t.origin.x, t.basis.x.y, t.basis.y.y, t.basis.z.y, t.origin.y,
		t.basis.x.z, t.basis.y.z, t.basis.z.z, t.origin.z]

static func _aabb(box: AABB) -> Array:
	return [box.position.x, box.position.y, box.position.z, box.size.x, box.size.y, box.size.z]

# The mesh's surfaces once per mesh: bounds, counts, material, levels of detail.
func _mesh(mesh: RID) -> int:
	var id := mesh.get_id()
	if mesh.is_valid() and not _meshes.has(str(id)):
		var surfaces: Array = []
		for index in RenderingServer.mesh_get_surface_count(mesh):
			var surface: Dictionary = RenderingServer.mesh_get_surface(mesh, index)
			var vertices := int(surface.get("vertex_count", 0))
			var width := 2 if vertices <= 65535 else 4
			var lods: Array = []
			for lod: Dictionary in surface.get("lods", []):
				lods.append([float(lod.get("edge_length", 0.0)), (lod.get("index_data", PackedByteArray()) as PackedByteArray).size() / width])
			var material: RID = surface.get("material", RID())
			surfaces.append({"aabb": _aabb(surface.get("aabb", AABB())), "primitive": int(surface.get("primitive", 3)),
				"vertices": vertices, "indices": int(surface.get("index_count", 0)), "lods": lods,
				"material": material.get_id()})
		_meshes[str(id)] = surfaces
	return id

func _multimesh_rows(multimesh: RID, drawn: int) -> String:
	var buffer := RenderingServer.multimesh_get_buffer(multimesh)
	var count := RenderingServer.multimesh_get_instance_count(multimesh)
	if count == 0 or drawn == 0:
		return ""
	var stride := buffer.size() / count
	var rows := PackedFloat32Array()
	rows.resize(drawn * ROW_FLOATS)
	for i in drawn:
		for k in ROW_FLOATS:
			rows[i * ROW_FLOATS + k] = buffer[i * stride + k]
	return Marshalls.raw_to_base64(rows.to_byte_array())

func _multimesh_item(layer: String, multimesh: RID, transform: Transform3D, extra: Dictionary) -> Dictionary:
	var drawn := RenderingServer.multimesh_get_visible_instances(multimesh)
	if drawn < 0:
		drawn = RenderingServer.multimesh_get_instance_count(multimesh)
	var item := {"layer": layer, "kind": "multimesh", "mesh": _mesh(RenderingServer.multimesh_get_mesh(multimesh)),
		"transform": _transform(transform), "aabb": _aabb(RenderingServer.multimesh_get_aabb(multimesh)),
		"count": drawn, "rows": _multimesh_rows(multimesh, drawn), "multimesh": multimesh.get_id()}
	item.merge(extra)
	return item

# The scene's drawn geometry nodes under Main, in the main viewport.
func _node_items(items: Array, viewport: Viewport) -> void:
	for node: Node in _main.find_children("*", "GeometryInstance3D", true, false):
		var instance := node as GeometryInstance3D
		if not instance.is_visible_in_tree() or instance.get_viewport() != viewport:
			continue
		var top := instance as Node
		while top.get_parent() != _main:
			top = top.get_parent()
		var category := str(instance.get_meta("shadow_policy_category", ""))
		var layer := "node:" + top.name + ("/" + category if not category.is_empty() else "")
		var extra := {"node": str(_main.get_path_to(instance)), "cast": int(instance.cast_shadow),
			"range": [instance.visibility_range_begin, instance.visibility_range_end],
			"layers": instance.layers, "margin": instance.extra_cull_margin, "lod_bias": instance.lod_bias,
			"override": instance.material_override.get_rid().get_id() if instance.material_override != null else 0,
			"instance_id": instance.get_instance_id()}
		if instance is MultiMeshInstance3D:
			var multimesh := (instance as MultiMeshInstance3D).multimesh
			if multimesh == null:
				continue
			items.append(_multimesh_item(layer, multimesh.get_rid(), instance.global_transform, extra))
		elif instance is MeshInstance3D:
			var mesh := (instance as MeshInstance3D).mesh
			if mesh == null:
				continue
			var item := {"layer": layer, "kind": "mesh", "mesh": _mesh(mesh.get_rid()),
				"transform": _transform(instance.global_transform), "aabb": _aabb(instance.get_aabb()), "count": 1}
			item.merge(extra)
			items.append(item)
		else:
			items.append({"layer": layer, "kind": instance.get_class(), "count": 0, "node": extra.node})

# The stream core's pooled slots: each drawn main instance and each casting twin.
func _slot_items(items: Array) -> void:
	var sources := {
		"rs:vegetation": _main.get_node("StreamNode").get_draw_census("vegetation"),
		"rs:parked": _main.get_node("StreamNode").get_draw_census("parked"),
		"rs:buildings": _main.get_node("SimulationNode").get_building_draw_census(),
	}
	for layer: String in sources:
		var census: Dictionary = sources[layer]
		if census.is_empty():
			continue
		var multimeshes: PackedInt64Array = census.multimesh
		for i in multimeshes.size():
			var origin: Vector3 = census.origin[i]
			var transform := Transform3D(Basis.IDENTITY, origin)
			var multimesh := rid_from_int64(multimeshes[i])
			var range: Vector2 = census.range[i]
			if census.visible[i] == 1:
				items.append(_multimesh_item(layer, multimesh, transform, {"cast": int(census.shadow[i]),
					"range": [range.x, range.y], "instance": census.instance[i], "role": "main"}))
			if census.caster[i] != 0 and census.caster_visible[i] == 1:
				# A caster twin draws in the shadow pass only (shadow 2); recorded for the census.
				items.append(_multimesh_item(layer, multimesh, transform, {"cast": int(census.caster_shadow[i]),
					"range": [0.0, 0.0], "instance": census.caster[i], "role": "caster"}))

# The graded cells and walls of every drawing set, at their patch node.
func _graded_items(items: Array) -> void:
	var tiles: Variant = _main.get_node("Terrain").get("graded_tiles")
	if tiles == null or tiles.get("_book") == null:
		return
	var census: Dictionary = tiles.get("_book").draw_census()
	if census.is_empty():
		return
	var multimeshes: PackedInt64Array = census.multimesh
	for i in multimeshes.size():
		if census.drawn[i] != 1 or census.shown[i] != 1 or census.count[i] == 0:
			continue
		var parent := instance_from_id(census.parent[i]) as Node3D
		var origin := parent.global_position if parent != null else Vector3.ZERO
		items.append(_multimesh_item("rs:graded" + (":walls" if census.wall[i] == 1 else ""),
			rid_from_int64(multimeshes[i]), Transform3D(Basis.IDENTITY, origin),
			{"cast": 0 if census.wall[i] == 0 else 1, "range": [0.0, 0.0], "instance": census.instance[i], "role": "main"}))

# Moves each layer's instances on the default render layer to a layer of its own, hides one
# layer at a time from the camera and reads the main pass's counters, then restores everything.
func _layer_counters(viewport: Viewport, camera: Camera3D, items: Array) -> Dictionary:
	# The layers with the most instances get a render layer each, the rest share the last.
	var instances := {}
	for item: Dictionary in items:
		instances[item.layer] = int(instances.get(item.layer, 0)) + maxi(1, int(item.get("count", 0)))
	var names: Array = instances.keys()
	names.sort_custom(func(a: String, b: String) -> bool: return instances[a] > instances[b] or (instances[a] == instances[b] and a < b))
	var bits := {}
	for name: String in names:
		bits[name] = mini(FIRST_LAYER_BIT + bits.size(), LAST_LAYER_BIT)
	var restore: Array = []
	for item: Dictionary in items:
		var bit := 1 << int(bits[item.layer])
		if item.has("instance_id"):
			var node := instance_from_id(item.instance_id) as GeometryInstance3D
			if node != null and node.layers == 1:
				restore.append([node, 1])
				node.layers = bit
		elif item.has("instance"):
			var rid := rid_from_int64(item.instance)
			RenderingServer.instance_set_layer_mask(rid, bit)
			restore.append([rid, 1])
	var mask := camera.cull_mask
	await _frames(SETTLE_FRAMES)
	var out := {"bits": bits, "full": await _counters(viewport)}
	var shared := {}
	for layer: String in bits:
		shared[bits[layer]] = shared.get(bits[layer], []) + [layer]
	var hold := int(OS.get_environment("METRUM_STREAM_CENSUS_HOLD"))
	await _hold("full", hold)
	for bit: int in shared:
		camera.cull_mask = mask & ~(1 << bit)
		await _frames(SETTLE_FRAMES)
		await _hold("hidden_%d" % bit, hold)
		out["hidden_%d" % bit] = {"layers": shared[bit], "counters": await _counters(viewport)}
	camera.cull_mask = mask
	for entry: Array in restore:
		if entry[0] is RID:
			RenderingServer.instance_set_layer_mask(entry[0], entry[1])
		else:
			entry[0].layers = entry[1]
	await _frames(SETTLE_FRAMES)
	out["restored"] = await _counters(viewport)
	return out

# METRUM_STREAM_CENSUS_HOLD frames more of a layer state, after `[STREAM] BEGIN census_<state>`,
# for an outside tracer (`RENDER-46`'s Intel PT captures of draws and binds by layer).
func _hold(state: String, frames: int) -> void:
	if frames > 0:
		print("[STREAM] BEGIN census_%s" % state)
		await _frames(frames)

# One frame's linear depth through a full-screen quad (see the header), saved as a PNG.
func _depth(viewport: Viewport, camera: Camera3D, path: String) -> void:
	var shader := Shader.new()
	shader.code = DEPTH_SHADER
	var material := ShaderMaterial.new()
	material.shader = shader
	material.render_priority = Material.RENDER_PRIORITY_MAX
	var quad := MeshInstance3D.new()
	var mesh := QuadMesh.new()
	mesh.size = Vector2(2.0, 2.0)
	quad.mesh = mesh
	quad.material_override = material
	quad.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	quad.custom_aabb = AABB(Vector3(-1.0e6, -1.0e6, -1.0e6), Vector3(2.0e6, 2.0e6, 2.0e6))
	camera.add_child(quad)
	var environment := Environment.new()
	environment.tonemap_mode = Environment.TONE_MAPPER_LINEAR
	var saved := [camera.environment, camera.attributes, viewport.use_taa, viewport.screen_space_aa,
		viewport.use_debanding, viewport.scaling_3d_mode, viewport.scaling_3d_scale, viewport.msaa_3d]
	camera.environment = environment
	camera.attributes = null
	viewport.use_taa = false
	viewport.screen_space_aa = Viewport.SCREEN_SPACE_AA_DISABLED
	viewport.use_debanding = false
	viewport.scaling_3d_mode = Viewport.SCALING_3D_MODE_BILINEAR
	viewport.scaling_3d_scale = 1.0
	viewport.msaa_3d = Viewport.MSAA_DISABLED
	var hidden: Array = []
	for layer: Node in _main.find_children("*", "CanvasLayer", true, false):
		if (layer as CanvasLayer).visible:
			hidden.append(layer)
			(layer as CanvasLayer).visible = false
	await _frames(SETTLE_FRAMES + 2)
	var image := viewport.get_texture().get_image()
	image.save_png(path)
	print("[STREAM] DEPTH %s format=%d %dx%d" % [path.get_file(), image.get_format(), image.get_width(), image.get_height()])
	for layer: CanvasLayer in hidden:
		layer.visible = true
	camera.environment = saved[0]
	camera.attributes = saved[1]
	viewport.use_taa = saved[2]
	viewport.screen_space_aa = saved[3]
	viewport.use_debanding = saved[4]
	viewport.scaling_3d_mode = saved[5]
	viewport.scaling_3d_scale = saved[6]
	viewport.msaa_3d = saved[7]
	quad.queue_free()
	await _frames(SETTLE_FRAMES)
