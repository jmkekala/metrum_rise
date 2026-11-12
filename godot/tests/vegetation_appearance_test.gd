# SPDX-License-Identifier: GPL-2.0-only

## Deterministic vegetation appearance contracts and a fixed patch upload benchmark.
## Run headless with --script res://tests/vegetation_appearance_test.gd.
## FIXTURE reports the maximum of 24 uploads after four warmups, excluding mesh setup.
extends SceneTree
const Vegetation = preload("res://scripts/renderers/vegetation.gd")
const MeshExport = preload("res://tests/vegetation_lod_measure.gd")
const Species = preload("res://scripts/renderers/tree_species.gd")
const SPAN = 510.0
class Terrain extends Node:
	func get_patch_surface_generation(_key): return 7
	func get_render_patch_span_m(): return SPAN
class Simulation extends Node:
	var data := PackedFloat32Array()
	func _init():
		data.resize(4096 * 6)
		for i in range(4096):
			data[i*6] = float(i % 64) * 7.75 + 0.25
			data[i*6+1] = float(i % 13) * 0.25
			data[i*6+2] = float(i / 64) * 7.75 + 0.5
			data[i*6+3] = float(i % 31) * 0.2
			data[i*6+4] = 0.8 + float(i % 17) * 0.025
			data[i*6+5] = [0, 1, 2, 2, 2, 2, 3, 3][i % 8]
	func get_terrain_world_size(): return Vector2(1020, 1020)
	# Rust packs world positions; the fixture is authored relative to the patch it is fetched for.
	# No crown coverage, so every tree keeps an open stand.
	func get_vegetation_stand_cover(_origin, _span): return {}
	# No yard hedges in the fixture.
	func get_vegetation_hedge_runs(_origin, _span): return PackedFloat32Array()
	func get_decorative_tree_patch(origin, _span, _understory):
		var world := data.duplicate()
		for i in range(0, world.size(), 6):
			world[i] += origin.x
			world[i + 2] += origin.y
		return world
	# The products' catalogue (`rebuild_from_simulation_state` hands it to the background jobs).
	var catalogue := PackedFloat32Array()
	func set_vegetation_product_catalogue(value):
		catalogue = value
		return true
func _initialize(): call_deferred("run")
func run():
	var host := Node3D.new()
	root.add_child(host)
	var terrain := Terrain.new()
	terrain.name = "Terrain"
	host.add_child(terrain)
	var simulation := Simulation.new()
	simulation.name = "SimulationNode"
	host.add_child(simulation)
	var stream := StreamNode.new()
	stream.name = "StreamNode"
	host.add_child(stream)
	var camera := Camera3D.new()
	host.add_child(camera)
	camera.current = true
	var vegetation := Vegetation.new()
	vegetation.set_process(false)
	var catalogue_start := Time.get_ticks_usec()
	host.add_child(vegetation)
	var catalogue_ms := float(Time.get_ticks_usec() - catalogue_start) / 1000.0
	# The water reflection's world, where the stream node mirrors the impostors.
	var reflection_world := World3D.new()
	vegetation.set_reflection_scenario(reflection_world.scenario)
	# The residency's settings and the terrain span: the stream node answers the bands a patch
	# takes from the camera (`RENDER-32-F`); its patches here are built and delivered by the test.
	stream.set_terrain_patch_span(SPAN)
	vegetation.rebuild_from_simulation_state()
	var geometry := _geometry_counts(vegetation.meshes)
	_check_geometry_budget(geometry)
	_check_impostors(vegetation)
	_check_canopy_forms(vegetation.meshes)
	_check_meshes(vegetation.meshes)
	_check_atlas_cells()
	for key in [Vector3i(0,0,1), Vector3i(1,0,1), Vector3i(0,1,1), Vector3i(1,1,1)]:
		_upload(vegetation, key, SPAN)
	await process_frame
	for repeat in range(6):
		for key in _records.keys():
			_upload(vegetation, key, SPAN)
		await process_frame
	var nodes := 0
	var resident := 0
	var proxies_seen := 0
	var positions: Array[String] = []
	var appearance: Array[String] = []
	# The headless dummy renderer does not retain uploaded transforms/colours. Check the product
	# Rust builds (`RENDER-32-C`) here and validate actual bucket populations on the nodes.
	var expected_positions: Array[String] = []
	for offset in range(0, simulation.data.size(), 6):
		var data := simulation.data
		expected_positions.append(var_to_bytes([int(data[offset + 5]) & Vegetation.SPECIES_MASK,
			Vector3(data[offset], data[offset + 1], data[offset + 2])]).hex_encode())
	for entry in _near_instances(vegetation, simulation.data):
		var tint: Color = entry[3]
		assert(tint.a == 1.0)
		assert(minf(tint.r, minf(tint.g, tint.b)) >= 0.82 * 0.90)
		assert(maxf(tint.r, maxf(tint.g, tint.b)) <= 1.18 * 1.10)
		assert(entry[1] >= 0 and entry[1] < Species.VARIANT_COUNTS[entry[0]])
		positions.append(var_to_bytes([entry[0], entry[2]]).hex_encode())
		appearance.append(var_to_bytes([entry[2], entry[0], entry[1], tint]).hex_encode())
	# Every placement is drawn once, at exactly the position Rust packed.
	expected_positions.sort()
	var drawn := positions.duplicate()
	drawn.sort()
	assert(drawn == expected_positions)
	for key in _records:
		var record: Dictionary = _records[key]
		# What the stream node drew (`RENDER-32-E`): one dictionary per batch, each instance's state.
		var patch: Dictionary = stream.get_vegetation_patch(key)
		# Seeds are keyed on world position, so each patch of the one fixture splits its
		# variants differently.
		var expected_buckets := {}
		var world_data: PackedFloat32Array = simulation.get_decorative_tree_patch(
			Vector2(key.x, key.y) * SPAN - simulation.get_terrain_world_size() * 0.5, SPAN, true)
		for entry in _near_instances(vegetation, world_data):
			var bucket := Vector2i(entry[0], entry[1])
			expected_buckets[bucket] = int(expected_buckets.get(bucket, 0)) + 1
		assert(record["understory"])
		assert(record["near_band"])
		assert(patch["origin"] == Vector2(key.x, key.y) * SPAN - simulation.get_terrain_world_size() * 0.5)
		# The levels of one species measure their visibility range from one bounds, or the
		# complementary near and distant ranges stop and start at different distances and
		# drop the trees in between. A single-level species keeps its own bounds. The shared
		# bounds hold the impostors' (the near levels' are the engine's own, which the dummy
		# renderer does not compute; the product's parity test holds them bitwise).
		var species_bounds: Dictionary = {}
		for batch: Dictionary in patch["batches"]:
			var species: int = batch["species"]
			var main: Dictionary = batch["main"]
			nodes += 2 if batch.has("caster") else 1
			if species_bounds.has(species):
				assert(main["aabb"] == species_bounds[species])
			species_bounds[species] = main["aabb"]
			if species < Species.BUSH and batch["lod"] == 2:
				assert(main["aabb"].encloses(batch["multimesh_aabb"]))
			# Every instance stands at the patch origin.
			assert(main["origin"] == Vector3(patch["origin"].x, 0.0, patch["origin"].y))
		for species in species_bounds:
			assert((species_bounds[species].size == Vector3.ZERO) == (species >= Species.BUSH))
		# Every canopy tree past TREE_SHADOW_HANDOVER_M casts from its impostor, faced to the light,
		# through one shadows-only instance per species on the impostor's own buffer. A patch near
		# the camera also submits its branched canopy; the shaders split the trees.
		assert(record["shadow_caster"] == Vegetation.ShadowCaster.IMPOSTOR)
		assert(patch["has_caster"])
		for batch: Dictionary in patch["batches"]:
			var species: int = batch["species"]
			var lod: int = batch["lod"]
			var main: Dictionary = batch["main"]
			if batch.has("caster"):
				var caster: Dictionary = batch["caster"]
				assert(lod == 2 and (species == Species.CONIFER or species == Species.BROADLEAF))
				assert(caster["shadow"] == RenderingServer.SHADOW_CASTING_SETTING_SHADOWS_ONLY)
				assert(caster["range"] == Vector2(0.0, vegetation.canopy_far_m()))
				assert(caster["material"] == 2 + species)
				assert(caster["visible"])
				proxies_seen += 1
			# Rocks and yard plants stand in the open and cast; ground cover does not, and the
			# branched canopy only while its patch is near the camera.
			var casts: bool = species == Species.ROCK or (species == Species.BUSH
				and int(batch["variant"]) >= Species.LANDSCAPE_FIRST_VARIANT) or (
				species < Species.BUSH and lod == 0 and bool(patch["branched"]))
			assert(main["shadow"] == (RenderingServer.SHADOW_CASTING_SETTING_ON
				if casts else RenderingServer.SHADOW_CASTING_SETTING_OFF))
			assert(main["visible"])
			# Only the near band carries instance colours; see the renderer's distant level.
			assert(batch["colors"] == (lod == 0))
			# The distant level carries form and tint, and a near canopy tree its stand closure.
			assert(batch["custom"] == (lod == 2 or (lod == 0
				and (species == Species.CONIFER or species == Species.BROADLEAF))))
			resident += int(batch["count"])
			var variant: int = batch["variant"]
			# The understory staggers its cutoff by variant. A canopy range reaches past the
			# handover by how far a tree origin stands from the centre of the shared bounds,
			# which is at most half their diagonal.
			var half: float = main["aabb"].size.length() * 0.5
			var range_m: Vector2 = main["range"]
			if species >= Species.BUSH:
				var reach: float = Vegetation.BUSH_RANGE_M if species == Species.BUSH else Vegetation.ROCK_RANGE_M
				var stagger := lerpf(0.75, 1.0, fmod(float(variant) * 0.6180339887498949, 1.0))
				assert(range_m.x == 0.0)
				# The range is stored in single precision.
				assert(range_m.y == PackedFloat32Array([reach * stagger])[0])
			elif lod == 0:
				assert(range_m.x == 0.0)
				assert(range_m.y >= vegetation.canopy_near_m())
				assert(range_m.y <= vegetation.canopy_near_m() + half)
			else:
				var begin: float = vegetation.canopy_crossfade_begin_m()
				assert(range_m.x <= begin)
				assert(range_m.x >= begin - half)
				assert(range_m.y >= vegetation.canopy_far_m())
				assert(range_m.y <= vegetation.canopy_far_m() + half)
			# Only canopy trees hand over; the understory shares their materials and does not.
			if lod == 0:
				assert(batch["hands_over"] == (species < Species.BUSH))
			if lod >= 2:
				# One shared quad and material per species, with a custom form per tree; the water
				# reflection draws the same impostors.
				assert(lod == 2)
				assert(batch["mesh"] == _impostor_mesh_index())
				assert(main["material"] == species)
				assert(batch["count"] == 512)
				assert(batch["reflection"]["material"] == species and batch["reflection"]["visible"])
				assert(batch["reflection"]["aabb"] == main["aabb"])
			else:
				# The near mesh remains a per-patch distance choice.
				var detail: int = patch["detail"] if species < Species.BUSH else 0
				assert(batch["mesh"] == _near_mesh_index(species, variant, detail))
				assert(main["material"] == -1)
				assert(batch["count"] == expected_buckets[Vector2i(species, variant)])
	positions.sort()
	appearance.sort()
	var appearance_digest := "\n".join(appearance).sha256_text()
	var digest := "\n".join(positions).sha256_text()
	# One proxy per canopy species in each of the four patches.
	assert(proxies_seen == 8)
	assert(nodes == 160 and resident == 20480)
	var planted := 0
	for key in _records:
		planted += int(_records[key]["tree_count"])
	assert(planted == 16384)
	# The detail level swaps the near mesh on the buffer already uploaded, so the reduced level
	# costs no second instance. A patch holds a single level only where the whole box of its
	# trees lies on one side of the handover band, which each tree crosses by its own distance.
	# The stream node follows the camera (`RENDER-32-E`; the rule's boundaries are its Rust tests).
	var detail_key: Vector3i = _records.keys()[0]
	var trees: AABB = stream.get_vegetation_patch(detail_key)["canopy_box"]
	assert(trees.size.x > 0.0 and trees.size.z > 0.0)
	var band_end: float = Vegetation.TREE_NEAR_DETAIL_M + Species.TREE_DETAIL_FADE_M
	for wanted in [Vegetation.DETAIL_REDUCED, Vegetation.DETAIL_BLEND]:
		camera.global_position = trees.get_center() + Vector3.UP * (band_end + trees.size.length()
			if wanted == Vegetation.DETAIL_REDUCED else 0.0)
		stream.flush_vegetation()
		var detail_patch: Dictionary = stream.get_vegetation_patch(detail_key)
		assert(int(detail_patch["detail"]) == wanted)
		for batch: Dictionary in detail_patch["batches"]:
			if int(batch["species"]) >= Species.BUSH or int(batch["lod"]) != 0:
				continue
			assert(batch["mesh"] == _near_mesh_index(batch["species"], batch["variant"], wanted))
	camera.global_position = Vector3.ZERO
	stream.flush_vegetation()
	print("FIXTURE ", JSON.stringify({"catalogue_ms":catalogue_ms, "geometry":geometry, "metrics":vegetation.metrics(), "nodes_per_patch":nodes / 4, "total_nodes":nodes, "resident_instances":resident, "position_sha256":digest, "appearance_sha256":appearance_digest}))
	# The toggle has to reach the caster, and a silent caster has to be hidden as well as
	# silenced, or an OFF caster would draw its unshaded quads.
	vegetation.set_cast_shadows(false)
	for batch: Dictionary in stream.get_vegetation_patch(Vector3i(0, 0, 1))["batches"]:
		assert(batch["main"]["shadow"] == RenderingServer.SHADOW_CASTING_SETTING_OFF)
		assert(batch["main"]["visible"])
		if batch.has("caster"):
			assert(batch["caster"]["shadow"] == RenderingServer.SHADOW_CASTING_SETTING_OFF)
			assert(not batch["caster"]["visible"])
	vegetation.set_cast_shadows(true)
	for batch: Dictionary in stream.get_vegetation_patch(Vector3i(0, 0, 1))["batches"]:
		assert(batch["main"]["visible"])
		if batch.has("caster"):
			assert(batch["caster"]["shadow"] == RenderingServer.SHADOW_CASTING_SETTING_SHADOWS_ONLY)
			assert(batch["caster"]["visible"])
	# Out of the near band a patch drops the variant split, the tints and the understory,
	# keeping one distant instance per canopy species. It is also past the sun's range, so
	# nothing it holds can reach a cascade and it carries no caster either.
	camera.global_position = Vector3(2000.0, 0.0, -255.0)
	_upload(vegetation, Vector3i(0, 0, 1), SPAN)
	var far_record: Dictionary = _records[Vector3i(0, 0, 1)]
	assert(not far_record["near_band"] and not far_record["understory"])
	assert(far_record["shadow_caster"] == Vegetation.ShadowCaster.NONE)
	var far_patch: Dictionary = stream.get_vegetation_patch(Vector3i(0, 0, 1))
	assert(far_patch["batches"].size() == 2 and not far_patch["has_caster"])
	for batch: Dictionary in far_patch["batches"]:
		var species: int = batch["species"]
		assert(batch["lod"] == 2 and not batch.has("caster"))
		assert(species == Species.CONIFER or species == Species.BROADLEAF)
		assert(batch["mesh"] == _impostor_mesh_index())
		assert(batch["custom"] and not batch["colors"])
		assert(batch["main"]["material"] == species)
		assert(batch["main"]["aabb"].size != Vector3.ZERO)
		assert(batch["count"] == 512)
	camera.global_position = Vector3.ZERO
	# Empty buckets emit no nodes, and changing density preserves the original subset rule.
	vegetation.density_fraction = 0.0
	_upload(vegetation, Vector3i(0, 0, 1), SPAN)
	assert(stream.get_vegetation_patch(Vector3i(0, 0, 1))["batches"].is_empty())
	assert(_records[Vector3i(0, 0, 1)]["tree_count"] == 0)
	# Pins name the variant in both the bucketed near level and the far-only impostor rows, whose
	# layer lane is the variant (the dummy backend cannot read a buffer back, so the product is
	# read directly).
	vegetation.density_fraction = 1.0
	var pinned := PackedFloat32Array([
		10, 0, 10, 0, 1, 4, 20, 0, 10, 0, 1, 12,
		30, 0, 10, 0, 1, 5, 40, 0, 10, 0, 1, 13,
	])
	for near_band in [true, false]:
		var product: Dictionary = SimulationNode.build_vegetation_product(pinned, {},
			PackedFloat32Array(), Vector2.ZERO, near_band, true, vegetation._product_settings(),
			vegetation._product_catalogue())
		var batches: PackedInt32Array = product.batches
		var written := 0
		for at in range(0, batches.size(), 8):
			if batches[at + 1] != 2 or batches[at + 3] == 1:
				continue
			var buffer: PackedFloat32Array = product.buffers[batches[at + 7]]
			for row in range(0, buffer.size(), 16):
				# The pins below name variants 0 and 2, and the layer is the variant.
				assert(buffer[row + 12] == (2.0 if int(buffer[row + 3]) % 20 == 0 else 0.0))
				written += 1
		assert(written == 4)
	await process_frame
	host.free()
	print("PASS vegetation appearance, shared material and bounds, LOD buckets, positions and empty density")
	quit()

# What each patch was built with, by key: its bands and plants (the residency's records hold
# what the game requests; this test builds and delivers its products itself).
var _records := {}
var _serial := 0

## Builds a patch's product from the mock's placements with the bands the stream node gives it
## from the camera now (its residency's rule), delivers it and installs it.
func _upload(vegetation, key: Vector3i, span: float) -> void:
	var simulation = vegetation.simulation
	var bands: Vector3i = vegetation.stream.get_vegetation_bands(key)
	assert(bands.x >= 0)
	var world: Vector2 = simulation.get_terrain_world_size()
	var origin := Vector2(key.x, key.y) * span - world * 0.5
	var product: Dictionary = SimulationNode.build_vegetation_product(
		simulation.get_decorative_tree_patch(origin, span, bands.x == 1), {}, PackedFloat32Array(),
		origin, bands.y == 1, bands.z == 1, vegetation._product_settings(), simulation.catalogue)
	_serial += 1
	vegetation.stream.deliver_vegetation_product(key, _serial, product, origin, span)
	vegetation.stream.flush_vegetation()
	_records[key] = {"understory": bands.x == 1, "near_band": bands.y == 1,
		"shadow_caster": bands.z, "tree_count": int(product["tree_count"])}

## The stream node's mesh index of a near level (`StreamNode.set_vegetation_catalogue`'s order:
## three levels per variant, species by species), and of the impostor quad after them.
func _near_mesh_index(species: int, variant: int, level: int) -> int:
	var bucket := variant
	for earlier in range(species):
		bucket += Species.VARIANT_COUNTS[earlier]
	return bucket * 3 + level

func _impostor_mesh_index() -> int:
	return _near_mesh_index(Species.SPECIES_COUNT, 0, 0)

func _check_impostors(vegetation: Node3D) -> void:
	var metadata := Species.impostor_metadata()
	assert(metadata.frames == 8 and metadata.frame_px == 64)
	assert(MeshExport.impostor_source_json(vegetation.meshes).sha256_text() == metadata.source_sha256,
		"Tree impostor bake is stale: re-export and run tools/bake_tree_impostors.py")
	assert(Species.impostor_mesh() is QuadMesh)
	assert(vegetation.canopy_near_m() == Species.TREE_CROSSFADE_END_M)
	for species in [Species.CONIFER, Species.BROADLEAF]:
		var material := Species.impostor_material(species)
		assert(material == Species.impostor_material(species))
		assert(material.shader == preload("res://scripts/shaders/vegetation_impostor.gdshader"))
		assert(material.get_shader_parameter("impostor_radiance_match") == Species.IMPOSTOR_RADIANCE_MATCH[species])
		for channel in ["albedo", "normal"]:
			var texture: Texture2DArray = material.get_shader_parameter(channel + "_atlas")
			# One layer per near variant, so a tree keeps its own shape across the handover.
			assert(texture.get_layers() == Species.VARIANT_COUNTS[species])
			assert(texture.get_width() == 512 and texture.get_height() == 512 and texture.has_mipmaps())
		for variant in range(Species.VARIANT_COUNTS[species]):
			assert(metadata.forms.has(Species.impostor_form(species, variant)))
			# Every brush pin overrides the seed, even in the distant placement path.
			for near_band in [true, false]:
				var product: Dictionary = SimulationNode.build_vegetation_product(
					PackedFloat32Array([5, 0, 5, 0, 1, ((variant + 1) << Vegetation.SPECIES_BITS) | species]),
					{}, PackedFloat32Array(), Vector2.ZERO, near_band, false,
					vegetation._product_settings(), vegetation._product_catalogue())
				var batches: PackedInt32Array = product.batches
				var impostor: PackedFloat32Array = product.buffers[batches[batches.size() - 1]]
				# The near level's variant, and the impostor row's layer lane.
				assert(not near_band or batches[2] == variant)
				assert(impostor[12] == float(variant))

func _geometry_counts(meshes: Array) -> Array:
	var result := []
	for variants in meshes:
		var species_counts := []
		for levels in variants:
			var level_counts := []
			for mesh in levels:
				var vertices := 0
				var triangles := 0
				for surface in range(mesh.get_surface_count()):
					var arrays: Array = mesh.surface_get_arrays(surface)
					vertices += arrays[Mesh.ARRAY_VERTEX].size()
					triangles += arrays[Mesh.ARRAY_INDEX].size() / 3
				level_counts.append([vertices, triangles])
			species_counts.append(level_counts)
		result.append(species_counts)
	return result

func _check_geometry_budget(counts: Array) -> void:
	# Authored budgets: tools/model_trees.py keeps a full tree under 4000 triangles and its
	# reduced level near a fifth of that, and tools/model_landscape.py keeps a yard plant under
	# 2500 and a rock under 600. Generated ground cover is procedural and keeps its ceiling.
	for species in range(Species.SPECIES_COUNT):
		for variant in range(Species.VARIANT_COUNTS[species]):
			var levels: Array = counts[species][variant]
			if species < Species.BUSH:
				assert(levels[0][1] <= 4000 and levels[1][1] <= 1000)
				assert(levels[1][1] * 3 < levels[0][1])
			elif species == Species.ROCK:
				assert(levels[0][1] <= 600)
			elif variant >= Species.LANDSCAPE_FIRST_VARIANT:
				assert(levels[0][1] <= 2500)
			else:
				assert(levels[0][0] <= 160)

## Two of each three variants take the species' major form, and every variant of one form
## cycles through its authored models.
func _check_canopy_forms(meshes: Array) -> void:
	for species in [Species.CONIFER, Species.BROADLEAF]:
		var models := {}
		for variant in range(Species.VARIANT_COUNTS[species]):
			var model := Species.canopy_model(species, variant)
			assert(model.begins_with(Species.CANOPY_FORMS[species][1 if variant % 3 == 2 else 0] + "_"))
			if models.has(model):
				assert(meshes[species][variant][0] == models[model])
			models[model] = meshes[species][variant][0]
		assert(models.size() == Species.FORM_MODELS * 2)

func _check_meshes(meshes: Array) -> void:
	var shared: Material = meshes[Species.ROCK][0][0].surface_get_material(0)
	var shared_wind: Material = Species._wind_branch_material()
	var shared_cards: Material = Species._foliage_material()
	# One cloud-lit stone material for every rock: the tile, its normal map and the vertex colour.
	assert(shared is ShaderMaterial)
	var rock_code: String = shared.shader.code
	assert(rock_code.contains("#define WINDOW_VERTEX_COLOR") and rock_code.contains("#define WINDOW_NORMAL"))
	assert(shared.get_shader_parameter("texture_albedo") != null)
	assert(shared_wind != shared_cards)
	assert(shared_wind.shader == preload("res://scripts/shaders/vegetation_wind.gdshader"))
	assert(shared_cards.shader == preload("res://scripts/shaders/vegetation_wind_cards.gdshader"))
	# Source contracts only: the dummy renderer cannot compile shaders or prove pass routing.
	var card_code: String = shared_cards.shader.code
	assert(card_code.contains("render_mode world_vertex_coords, cull_disabled;"))
	assert(card_code.contains("? leaf.a : 0.0;"))
	assert(card_code.contains("ALPHA_SCISSOR_THRESHOLD = 0.4;"))
	for code in [shared_wind.shader.code, card_code]:
		for forbidden in ["blend_", "depth_draw_", "depth_prepass_alpha", "depth_test_",
			"unshaded", "ambient_light_disabled", "ALPHA_HASH", "alpha_to_coverage"]:
			assert(not code.contains(forbidden))
	assert(not shared_wind.shader.code.contains("ALPHA"))
	for species in range(Species.SPECIES_COUNT):
		assert(meshes[species].size() == Species.VARIANT_COUNTS[species])
		var silhouettes := {}
		for levels in meshes[species]:
			assert(levels.size() == (3 if species < Species.BUSH else 1))
			var bounds: AABB = levels[0].get_aabb()
			silhouettes[bounds] = true
			for level in range(levels.size()):
				var mesh: ArrayMesh = levels[level]
				var surfaces: int = mesh.get_surface_count()
				if species < Species.BUSH:
					# One wood and one card surface per level, on the form's own pair of materials
					# for that level. The blend mesh carries the full level's, then the reduced one's.
					assert(surfaces == (4 if level == 2 else 2))
					for surface in range(surfaces):
						var material: ShaderMaterial = mesh.surface_get_material(surface)
						assert(bool(material.get_shader_parameter("reduced_level"))
							== (level == 1 or surface >= 2))
						assert(material.shader in [shared_wind.shader, shared_cards.shader])
						# Vertex colour alpha is the sway weight, never opacity.
						_check_sway_range(mesh, surface, 1.0)
				elif species == Species.BUSH:
					# The understory is a catalogue of different ground plants, so its
					# surfaces are per-variant: a dwarf shrub mat is cards alone, a grass tuft
					# is blades alone, and only the woody forms carry a stem surface and cards.
					assert(surfaces == 1 or surfaces == 2)
					for surface in range(surfaces):
						var material: ShaderMaterial = mesh.surface_get_material(surface)
						assert(material.shader in [shared_wind.shader, shared_cards.shader])
						# Ground plants bend from their own base: anchored at zero and never
						# past the cap a bush is allowed, on every surface.
						_check_sway_range(mesh, surface, 0.8)
				else:
					assert(surfaces == 1 and mesh.surface_get_material(0) == shared)
					_check_sway_weights(mesh, 0, 1.0, 1.0)
		# Canopy variants cycle over fewer models, so only the others are all distinct.
		if species >= Species.BUSH:
			assert(silhouettes.size() == Species.VARIANT_COUNTS[species])

## Asserts the weight range a surface's vertex colour alpha spans. Rigid meshes keep the
## original alpha of 1, so they are checked against the same contract with a floor of 1.
## Face shading multiplies alpha along with RGB, and the 8-bit vertex colour format clamps
## the result, which is why a rigid surface reads exactly 1 rather than the 1.35 it computes.
func _check_sway_weights(mesh: ArrayMesh, surface: int, expect_min: float, expect_max: float) -> void:
	var colors: PackedColorArray = mesh.surface_get_arrays(surface)[Mesh.ARRAY_COLOR]
	var lowest := INF
	var highest := -INF
	for color in colors:
		lowest = minf(lowest, color.a)
		highest = maxf(highest, color.a)
	# The retained primary tips' 0.70 weight is quantized to an 8-bit colour channel.
	assert(is_equal_approx(lowest, expect_min) and absf(highest - expect_max) <= 1.0 / 255.0)

## Index of the surface carrying the shared card material, or -1 when there is none.
func _card_surface(mesh: ArrayMesh, shared_cards: Material) -> int:
	for surface in range(mesh.get_surface_count()):
		if mesh.surface_get_material(surface) == shared_cards:
			return surface
	return -1

## Asserts a swaying surface is anchored at zero and never exceeds its species cap.
func _check_sway_range(mesh: ArrayMesh, surface: int, cap: float) -> void:
	var colors: PackedColorArray = mesh.surface_get_arrays(surface)[Mesh.ARRAY_COLOR]
	var lowest := INF
	var highest := -INF
	for color in colors:
		lowest = minf(lowest, color.a)
		highest = maxf(highest, color.a)
	assert(lowest >= 0.0 and highest <= cap + 0.001 and highest > 0.0)

## The broadleaf and conifer card cells retain their crop and topology.
func _check_atlas_cells() -> void:
	var texture: Texture2D = Species._foliage_material().get_shader_parameter("foliage_mask")
	assert(texture.resource_path == "res://assets/textures/vegetation/foliage_atlas.dds")
	# Atlas resolution is a tuning decision, so assert the shape rather than the number:
	# square, a power of two, and four cells that leave at least 128 px per cluster.
	var size := texture.get_width()
	assert(texture.get_height() == size and size >= 256 and size & (size - 1) == 0)
	var image := texture.get_image()
	# A chain that stops short of 1x1 leaves the smallest mip to the driver.
	var levels := 0
	var probe := size
	while probe > 1:
		probe /= 2
		levels += 1
	assert(image.has_mipmaps() and image.get_mipmap_count() == levels)
	# The native DDS loader must retain every authored byte, including corrected alpha.
	assert(image.get_data() == FileAccess.get_file_as_bytes(texture.resource_path).slice(128))
	var axis := (Vector3.UP + Vector3.RIGHT * 0.35).normalized()
	for conifer in [false, true]:
		for seed in [0, 1, 2, 3, -1]:
			var surface := SurfaceTool.new()
			surface.begin(Mesh.PRIMITIVE_TRIANGLES)
			Species._foliage_cards(surface, Vector3.UP, Vector3.ZERO,
				Vector3.RIGHT, Vector3.ONE, conifer, seed, Color(0.2, 0.3, 0.1), axis, 0.2)
			var arrays := surface.commit().surface_get_arrays(0)
			var uvs: PackedVector2Array = arrays[Mesh.ARRAY_TEX_UV]
			assert(uvs.size() == 18)
			var origin := Vector2(0.0, 1.0 if conifer else 0.0) * 0.5
			var quadrant := int(origin.x * 2.0) + int(origin.y * 2.0) * 2
			var trim: Rect2 = Species.FOLIAGE_ALPHA_RECTS[quadrant]
			var corners := [origin + Vector2(trim.position.x, trim.end.y) * 0.5,
				origin + trim.end * 0.5, origin + Vector2(trim.end.x, trim.position.y) * 0.5,
				origin + trim.position * 0.5]
			# The baked rect is only worth trusting if it still matches the atlas it was
			# measured from, so it is checked against the alpha threshold itself.
			var cell := Vector2i(origin * size)
			var found := Rect2i()
			for y in range(size / 2):
				for x in range(size / 2):
					if image.get_pixel(cell.x + x, cell.y + y).a >= 0.4:
						var pixel := Rect2i(x, y, 1, 1)
						found = pixel if not found.has_area() else found.merge(pixel)
			assert(Rect2(found) == Rect2(trim.position * (size / 2), trim.size * (size / 2)))
			var positions: PackedVector3Array = arrays[Mesh.ARRAY_VERTEX]
			var top := Vector3.UP + axis
			var bottom := Vector3.UP - axis * (0.5 if conifer else 0.72)
			for vertex in range(18):
				assert(uvs[vertex] == corners[[0, 2, 1, 0, 3, 2][vertex % 6]])
				# The crop is one affine map, so a corner must sit where its own UV says.
				var uv := (uvs[vertex] - origin) * 2.0
				var radial := Vector3.RIGHT.cross(axis).normalized().rotated(axis,
					Species._noise(seed, 307) * PI + float(vertex / 6) * PI / 3.0)
				assert(positions[vertex].is_equal_approx(top.lerp(bottom, uv.y) + radial * (2.0 * uv.x - 1.0)))

## [species, variant, origin, tint] of every near instance the product draws for `data` at the
## origin, decoded from the near buffers: transform rows, then the tint.
func _near_instances(vegetation, data: PackedFloat32Array) -> Array:
	var product: Dictionary = SimulationNode.build_vegetation_product(data, {},
		PackedFloat32Array(), Vector2.ZERO, true, false, vegetation._product_settings(),
		vegetation._product_catalogue())
	var batches: PackedInt32Array = product.batches
	var result := []
	for at in range(0, batches.size(), 8):
		if batches[at + 1] != 0:
			continue
		var buffer: PackedFloat32Array = product.buffers[batches[at + 7]]
		var stride := 20 if batches[at + 6] == 1 else 16
		for row in range(0, buffer.size(), stride):
			result.append([batches[at], batches[at + 2],
				Vector3(buffer[row + 3], buffer[row + 7], buffer[row + 11]),
				Color(buffer[row + 12], buffer[row + 13], buffer[row + 14], buffer[row + 15])])
	return result
