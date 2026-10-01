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
	func get_vegetation_patch_generation(_key): return 0
	func get_terrain_world_size(): return Vector2(1020, 1020)
	# Rust packs world positions; the fixture is authored relative to the patch it is fetched for.
	func get_decorative_tree_patch(origin, _span, _understory):
		var world := data.duplicate()
		for i in range(0, world.size(), 6):
			world[i] += origin.x
			world[i + 2] += origin.y
		return world
# Inspect the CPU buffer during actual uploads; the dummy backend cannot read it back.
# Used only for four pinned trees outside the timed fixture.
class BufferProbe extends Vegetation:
	var written := 0
	func _ready() -> void: pass
	func _write_impostor(buffer: PackedFloat32Array, offset: int, placed: Transform3D, layer: float,
		tint: Color) -> void:
		super._write_impostor(buffer, offset, placed, layer, tint)
		# The pins below name variants 0 and 2, and the layer is the variant.
		var expected := 2.0 if int(placed.origin.x) % 20 == 0 else 0.0
		assert(buffer[offset + 12] == expected)
		assert(buffer[offset + 3] == placed.origin.x)
		written += 1

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
	var camera := Camera3D.new()
	host.add_child(camera)
	camera.current = true
	var vegetation := Vegetation.new()
	vegetation.set_process(false)
	var catalogue_start := Time.get_ticks_usec()
	host.add_child(vegetation)
	var catalogue_ms := float(Time.get_ticks_usec() - catalogue_start) / 1000.0
	var geometry := _geometry_counts(vegetation.meshes)
	_check_geometry_budget(geometry)
	_check_impostors(vegetation)
	_check_canopy_forms(vegetation.meshes)
	_check_meshes(vegetation.meshes)
	_check_atlas_cells()
	for key in [Vector3i(0,0,1), Vector3i(1,0,1), Vector3i(0,1,1), Vector3i(1,1,1)]:
		vegetation._upload_patch(key, SPAN)
	await process_frame
	vegetation.generation_ms_max = 0.0
	for repeat in range(6):
		for key in vegetation.patches.keys():
			vegetation._upload_patch(key, SPAN)
		await process_frame
	var nodes := 0
	var resident := 0
	var proxies_seen := 0
	var positions: Array[String] = []
	var appearance: Array[String] = []
	# The headless dummy renderer does not retain uploaded transforms/colours. Check
	# deterministic inputs here and validate actual bucket populations on the nodes.
	for offset in range(0, simulation.data.size(), 6):
		var packed := int(simulation.data[offset + 5])
		var species := packed & Vegetation.SPECIES_MASK
		var seed: int = vegetation._appearance_seed(simulation.data, offset)
		var variant: int = vegetation._variant_index(species, seed, packed >> Vegetation.SPECIES_BITS)
		var tint: Color = vegetation._instance_tint(seed)
		assert(tint.a == 1.0)
		assert(minf(tint.r, minf(tint.g, tint.b)) >= 0.82 * 0.90)
		assert(maxf(tint.r, maxf(tint.g, tint.b)) <= 1.18 * 1.10)
		assert(variant >= 0 and variant < Species.VARIANT_COUNTS[species])
		var t: Transform3D = vegetation._instance_transform(simulation.data, offset, seed, Vector2.ZERO)
		var expected := Vector3(simulation.data[offset], simulation.data[offset + 1], simulation.data[offset + 2])
		assert(var_to_bytes(t.origin) == var_to_bytes(expected))
		positions.append(var_to_bytes([species, t.origin]).hex_encode())
		appearance.append(var_to_bytes([t.origin, species, variant, tint]).hex_encode())
	for key in vegetation.patches:
		var patch: Node3D = vegetation.patches[key]
		# Seeds are keyed on world position, so each patch of the one fixture splits its
		# variants differently.
		var expected_buckets := {}
		var world_data: PackedFloat32Array = simulation.get_decorative_tree_patch(
			Vector2(key.x, key.y) * SPAN - simulation.get_terrain_world_size() * 0.5, SPAN, true)
		for offset in range(0, world_data.size(), 6):
			var packed := int(world_data[offset + 5])
			var species := packed & Vegetation.SPECIES_MASK
			var bucket := Vector2i(species, vegetation._variant_index(species,
				vegetation._appearance_seed(world_data, offset), packed >> Vegetation.SPECIES_BITS))
			expected_buckets[bucket] = int(expected_buckets.get(bucket, 0)) + 1
		nodes += patch.get_child_count()
		assert(patch.get_meta("surface_generation") == 7)
		assert(patch.get_meta("understory"))
		assert(patch.get_meta("near_band"))
		# The levels of one species measure their visibility range from one bounds, or the
		# complementary near and distant ranges stop and start at different distances and
		# drop the trees in between. A single-level species keeps its own bounds. The dummy
		# renderer reports empty automatic bounds; explicit impostor bounds remain available.
		var species_bounds: Dictionary = {}
		var species_union: Dictionary = {}
		for instance in patch.get_children():
			var species: int = instance.get_meta("species")
			var box: AABB = instance.multimesh.custom_aabb
			if box.size == Vector3.ZERO:
				box = instance.get_aabb()
			species_union[species] = (
				box if not species_union.has(species) else (species_union[species] as AABB).merge(box)
			)
			if species_bounds.has(species):
				assert(instance.custom_aabb == species_bounds[species])
			species_bounds[species] = instance.custom_aabb
		for species in species_bounds:
			var union: AABB = species_union[species]
			assert(species_bounds[species] == (union if union.size != Vector3.ZERO else AABB()))
		# Every canopy tree casts from its impostor, faced to the light, through one shadows-only
		# instance per species that shares the impostor's buffer. Nothing else casts.
		assert(patch.get_meta("shadow_caster") == Vegetation.ShadowCaster.IMPOSTOR)
		var casters := {}
		var impostors := {}
		for instance in patch.get_children():
			var mm: MultiMesh = instance.multimesh
			var species: int = instance.get_meta("species")
			var lod: int = instance.get_meta("lod")
			if instance.get_meta("shadow_proxy"):
				assert(species == Species.CONIFER or species == Species.BROADLEAF)
				assert(instance.cast_shadow == GeometryInstance3D.SHADOW_CASTING_SETTING_SHADOWS_ONLY)
				assert(instance.visibility_range_begin == 0.0)
				assert(instance.visibility_range_end == vegetation.canopy_far_m())
				assert(mm.mesh == Species.impostor_mesh())
				assert(instance.material_override == Species.impostor_shadow_material(species))
				assert(mm.use_custom_data and not mm.use_colors)
				casters[species] = mm
				proxies_seen += 1
				continue
			if lod == 2:
				impostors[species] = mm
			# Rocks and yard plants stand in the open and cast; ground cover and trees do not.
			var casts: bool = species == Species.ROCK or (species == Species.BUSH
				and int(instance.get_meta("variant")) >= Species.LANDSCAPE_FIRST_VARIANT)
			assert(instance.cast_shadow == (GeometryInstance3D.SHADOW_CASTING_SETTING_ON
				if casts else GeometryInstance3D.SHADOW_CASTING_SETTING_OFF))
			# Only the near band carries instance colours; see the renderer's distant level.
			assert(mm.use_colors == (lod == 0))
			assert(mm.use_custom_data == (lod == 2))
			resident += mm.instance_count
			var near_band: bool = patch.get_meta("near_band")
			var variant: int = instance.get_meta("variant")
			# The understory staggers its cutoff by variant. A canopy range reaches past the
			# handover by how far a tree origin stands from the centre of the shared bounds,
			# which is at most half their diagonal.
			var half: float = instance.custom_aabb.size.length() * 0.5
			if species >= Species.BUSH:
				assert(Vector2(instance.visibility_range_begin, instance.visibility_range_end)
					== vegetation.lod_range(species, lod, variant, near_band, 0.0))
			elif lod == 0:
				assert(instance.visibility_range_begin == 0.0)
				assert(instance.visibility_range_end >= vegetation.canopy_near_m())
				assert(instance.visibility_range_end <= vegetation.canopy_near_m() + half)
			else:
				var begin: float = vegetation.canopy_crossfade_begin_m()
				assert(instance.visibility_range_begin <= begin)
				assert(instance.visibility_range_begin >= begin - half)
				assert(instance.visibility_range_end >= vegetation.canopy_far_m())
				assert(instance.visibility_range_end <= vegetation.canopy_far_m() + half)
			# Only canopy trees hand over; the understory shares their materials and does not.
			if lod == 0:
				assert(instance.get_instance_shader_parameter("hands_over")
					== (1.0 if species < Species.BUSH else null))
			assert(instance.visibility_range_fade_mode == GeometryInstance3D.VISIBILITY_RANGE_FADE_DISABLED)
			if lod >= 2:
				# One shared quad and material per species, with a custom form per tree.
				assert(lod == 2)
				assert(mm.mesh == Species.impostor_mesh())
				assert(instance.material_override == Species.impostor_material(species))
				assert(mm.instance_count == 512)
			else:
				# The near mesh remains a per-patch distance choice.
				var detail: int = patch.get_meta("near_detail_lod") if species < Species.BUSH else 0
				assert(mm.mesh == vegetation.meshes[species][variant][detail])
				assert(mm.instance_count == expected_buckets[Vector2i(species, variant)])
		for species in casters:
			# Shares the impostor buffer, custom lane included, rather than copying it.
			assert(not is_same(casters[species], impostors[species]))
			assert(casters[species].buffer == impostors[species].buffer)
			assert(casters[species].instance_count == impostors[species].instance_count)
			assert(casters[species].custom_aabb == impostors[species].custom_aabb)
	positions.sort()
	appearance.sort()
	var appearance_digest := "\n".join(appearance).sha256_text()
	var digest := "\n".join(positions).sha256_text()
	# One proxy per canopy species in each of the four patches.
	assert(proxies_seen == 8)
	assert(nodes == 160 and resident == 20480)
	assert(vegetation.tree_count == 16384)
	# The detail level swaps the near mesh on the buffer already uploaded, so the reduced level
	# costs no second instance. A patch holds a single level only where the whole box of its
	# trees lies on one side of the handover band, which each tree crosses by its own distance.
	var detail_patch: Node3D = vegetation.patches[vegetation.patches.keys()[0]]
	var trees: AABB = detail_patch.get_meta("canopy_box")
	assert(trees.size.x > 0.0 and trees.size.z > 0.0)
	var band_end: float = Vegetation.TREE_NEAR_DETAIL_M + Species.TREE_DETAIL_FADE_M
	var tiny := AABB(trees.get_center(), Vector3.ONE)
	assert(vegetation._near_detail_lod(tiny, tiny.get_center() + Vector3.UP * 10.0)
		== Vegetation.DETAIL_FULL)
	assert(vegetation._near_detail_lod(trees, trees.end + Vector3(band_end, 0.0, 0.0))
		== Vegetation.DETAIL_REDUCED)
	assert(vegetation._near_detail_lod(trees, trees.end + Vector3(band_end - 1.0, 0.0, 0.0))
		== Vegetation.DETAIL_BLEND)
	for wanted in [Vegetation.DETAIL_REDUCED, Vegetation.DETAIL_BLEND]:
		vegetation._refresh_near_detail(detail_patch, trees.get_center()
			+ Vector3.UP * (band_end + trees.size.length() if wanted == Vegetation.DETAIL_REDUCED else 0.0))
		assert(int(detail_patch.get_meta("near_detail_lod")) == wanted)
		for instance in detail_patch.get_children():
			var species: int = instance.get_meta("species")
			if species >= Species.BUSH or int(instance.get_meta("lod")) != 0:
				continue
			assert(instance.multimesh.mesh
				== vegetation.meshes[species][int(instance.get_meta("variant"))][wanted])
	print("FIXTURE ", JSON.stringify({"catalogue_ms":catalogue_ms, "geometry":geometry, "metrics":vegetation.metrics(), "nodes_per_patch":nodes / 4, "total_nodes":nodes, "resident_instances":resident, "position_sha256":digest, "appearance_sha256":appearance_digest}))
	# The toggle has to reach the caster, and a silent caster has to be hidden as well as
	# silenced, or an OFF caster would draw its unshaded quads.
	var toggle_patch: Node3D = vegetation.patches[Vector3i(0, 0, 1)]
	vegetation.set_cast_shadows(false)
	for instance in toggle_patch.get_children():
		assert(instance.cast_shadow == GeometryInstance3D.SHADOW_CASTING_SETTING_OFF)
		assert(instance.visible != bool(instance.get_meta("shadow_proxy")))
	vegetation.set_cast_shadows(true)
	for instance in toggle_patch.get_children():
		assert(instance.visible)
		assert((instance.cast_shadow == GeometryInstance3D.SHADOW_CASTING_SETTING_SHADOWS_ONLY)
			== bool(instance.get_meta("shadow_proxy")))
	# Out of the near band a patch drops the variant split, the tints and the understory,
	# keeping one distant instance per canopy species. It is also past the sun's range, so
	# nothing it holds can reach a cascade and it carries no caster either.
	camera.global_position = Vector3(2000.0, 0.0, -255.0)
	vegetation._upload_patch(Vector3i(0, 0, 1), SPAN)
	var far_patch: Node3D = vegetation.patches[Vector3i(0, 0, 1)]
	assert(not far_patch.get_meta("near_band") and not far_patch.get_meta("understory"))
	assert(far_patch.get_meta("shadow_caster") == Vegetation.ShadowCaster.NONE)
	assert(far_patch.get_child_count() == 2)
	for instance in far_patch.get_children():
		var species: int = instance.get_meta("species")
		assert(instance.get_meta("lod") == 2)
		assert(species == Species.CONIFER or species == Species.BROADLEAF)
		assert(instance.multimesh.mesh == Species.impostor_mesh())
		assert(instance.multimesh.use_custom_data)
		assert(instance.material_override == Species.impostor_material(species))
		assert(instance.custom_aabb.size != Vector3.ZERO)
		assert(instance.multimesh.instance_count == 512)
		assert(not instance.multimesh.use_colors)
	# The near band must not follow the span of whichever patch uploaded last. It once did, and
	# with the floor below the coarse diagonal every upload flipped it between 250 m and 721 m,
	# so resident patches disagreed with their own record each frame and rebuilt without end.
	vegetation._upload_patch(Vector3i(0, 0, Vegetation.PATCH_SUBDIVISION), SPAN / Vegetation.PATCH_SUBDIVISION)
	var fine_near: float = vegetation.canopy_near_m()
	vegetation._upload_patch(Vector3i(0, 0, 1), SPAN)
	assert(vegetation.canopy_near_m() == fine_near)
	camera.global_position = Vector3.ZERO
	# Empty buckets emit no nodes, and changing density preserves the original subset rule.
	vegetation.density_fraction = 0.0
	vegetation._upload_patch(Vector3i(0, 0, 1), SPAN)
	assert(vegetation.patches[Vector3i(0, 0, 1)].get_child_count() == 0)
	assert(vegetation.patches[Vector3i(0, 0, 1)].get_meta("tree_count") == 0)
	# Exercise both bucketed near uploads and the far-only placement loop with pins.
	var probe := BufferProbe.new()
	probe.meshes = vegetation.meshes
	probe.set_process(false)
	host.add_child(probe)
	simulation.data = PackedFloat32Array([
		10, 0, 10, 0, 1, 4, 20, 0, 10, 0, 1, 12,
		30, 0, 10, 0, 1, 5, 40, 0, 10, 0, 1, 13,
	])
	for x in [0.0, 2000.0]:
		camera.global_position = Vector3(x, 0, 0)
		probe.written = 0
		probe._upload_patch(Vector3i(0, 0, 1), SPAN)
		assert(probe.written == 4)
		assert(probe.patches[Vector3i(0, 0, 1)].get_meta("near_band") == (x == 0.0))
	await process_frame
	host.free()
	print("PASS vegetation appearance, shared material and bounds, LOD buckets, positions and empty density")
	quit()

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
			assert(vegetation._variant_index(species, 123456, variant + 1) == variant)
	# The dummy renderer discards GPU buffers. Verify the exact CPU layout before upload,
	# with off-diagonal basis entries and translation to detect row/column transposition.
	var buffer := PackedFloat32Array()
	buffer.resize(32)
	var transform := Transform3D(Basis(Vector3(1, 2, 3), Vector3(4, 5, 6), Vector3(7, 8, 9)), Vector3(10, 11, 12))
	vegetation._write_impostor(buffer, 0, transform, 1.0, Color(0.25, 0.5, 0.75))
	vegetation._write_impostor(buffer, 16, transform.translated(Vector3(3, 4, 5)), 0.0, Color.WHITE)
	assert(buffer.slice(0, 12) == PackedFloat32Array([1, 4, 7, 10, 2, 5, 8, 11, 3, 6, 9, 12]))
	# Custom lane: variant layer, then the near tint the impostor multiplies its albedo by.
	assert(buffer.slice(12, 16) == PackedFloat32Array([1.0, 0.25, 0.5, 0.75]) and buffer[28] == 0.0)

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
	# One stone material for every rock: the tile, its normal map and the vertex colour.
	assert(shared is StandardMaterial3D)
	assert(shared.vertex_color_use_as_albedo and shared.albedo_texture != null and shared.normal_enabled)
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
