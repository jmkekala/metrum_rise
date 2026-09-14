# SPDX-License-Identifier: GPL-2.0-only

## Deterministic vegetation appearance contracts and a fixed patch upload benchmark.
## Run headless with --script res://tests/vegetation_appearance_test.gd.
## FIXTURE reports the maximum of 24 uploads after four warmups, excluding mesh setup.
extends SceneTree
const Vegetation = preload("res://scripts/renderers/vegetation.gd")
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
	func get_decorative_tree_patch(_origin, _span, _understory): return data
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
	_check_crown_integral()
	var crown_colors := _check_crown_colors(vegetation.meshes)
	_check_birch(vegetation.meshes[Species.BROADLEAF])
	_check_meshes(vegetation.meshes)
	_check_atlas_cells()
	for key in [Vector2i(0,0), Vector2i(1,0), Vector2i(0,1), Vector2i(1,1)]:
		vegetation._upload_patch(key, SPAN)
	await process_frame
	vegetation.generation_ms_max = 0.0
	for repeat in range(6):
		for key in vegetation.patches.keys():
			vegetation._upload_patch(key, SPAN)
		await process_frame
	var nodes := 0
	var resident := 0
	var positions: Array[String] = []
	var appearance: Array[String] = []
	var expected_buckets := {}
	# The headless dummy renderer does not retain uploaded transforms/colours. Check
	# deterministic inputs here and validate actual bucket populations on the nodes.
	for offset in range(0, simulation.data.size(), 6):
		var species := int(simulation.data[offset + 5])
		var seed: int = vegetation._appearance_seed(simulation.data, offset)
		var variant: int = vegetation._variant_index(species, seed)
		var tint: Color = vegetation._instance_tint(seed)
		assert(tint.a == 1.0)
		assert(minf(tint.r, minf(tint.g, tint.b)) >= 0.90)
		assert(maxf(tint.r, maxf(tint.g, tint.b)) <= 1.10)
		assert(variant >= 0 and variant < Species.VARIANT_COUNTS[species])
		var t: Transform3D = vegetation._instance_transform(simulation.data, offset, species, seed)
		var expected := Vector3(simulation.data[offset], simulation.data[offset + 1], simulation.data[offset + 2])
		assert(var_to_bytes(t.origin) == var_to_bytes(expected))
		positions.append(var_to_bytes([species, t.origin]).hex_encode())
		appearance.append(var_to_bytes([t.origin, species, variant, tint]).hex_encode())
		var bucket := Vector2i(species, variant)
		expected_buckets[bucket] = int(expected_buckets.get(bucket, 0)) + 1
	for key in vegetation.patches:
		var patch: Node3D = vegetation.patches[key]
		nodes += patch.get_child_count()
		assert(patch.get_meta("surface_generation") == 7)
		assert(patch.get_meta("understory"))
		for instance in patch.get_children():
			var mm: MultiMesh = instance.multimesh
			# Only the near band carries instance colours; see the renderer's distant level.
			assert(mm.use_colors == (int(instance.get_meta("lod")) == 0))
			resident += mm.instance_count
			var species: int = instance.get_meta("species")
			var lod: int = instance.get_meta("lod")
			assert(instance.visibility_range_begin == vegetation.lod_range(species, lod).x)
			assert(instance.visibility_range_end == vegetation.lod_range(species, lod).y)
			assert(instance.visibility_range_fade_mode == GeometryInstance3D.VISIBILITY_RANGE_FADE_DISABLED)
			assert(instance.cast_shadow == vegetation._shadow_setting(species, lod))
			if lod > 0:
				# One distant instance per species; which crown mesh it carries is the
				# patch's distance choice, not a second instance.
				assert(lod == 1)
				assert(mm.mesh == vegetation.meshes[species][0][patch.get_meta("distant_lod")])
				assert(mm.instance_count == 512)
			else:
				var matched := false
				for variant in range(Species.VARIANT_COUNTS[species]):
					if mm.mesh == vegetation.meshes[species][variant][0]:
						assert(mm.instance_count == expected_buckets[Vector2i(species, variant)])
						matched = true
				assert(matched)
	positions.sort()
	appearance.sort()
	var appearance_digest := "\n".join(appearance).sha256_text()
	var digest := "\n".join(positions).sha256_text()
	assert(nodes == 152 and resident == 20480)
	assert(vegetation.tree_count == 16384)
	print("FIXTURE ", JSON.stringify({"catalogue_ms":catalogue_ms, "geometry":geometry, "crown_colors":crown_colors, "metrics":vegetation.metrics(), "nodes_per_patch":nodes / 4, "total_nodes":nodes, "resident_instances":resident, "position_sha256":digest, "appearance_sha256":appearance_digest}))
	# Out of the near band a patch drops the variant split, the tints and the understory,
	# keeping one distant instance per canopy species over the whole species population.
	camera.global_position = Vector3(2000.0, 0.0, -255.0)
	vegetation._upload_patch(Vector2i(0, 0), SPAN)
	var far_patch: Node3D = vegetation.patches[Vector2i(0, 0)]
	assert(not far_patch.get_meta("near_band") and not far_patch.get_meta("understory"))
	assert(far_patch.get_meta("distant_lod") == 2)
	assert(far_patch.get_child_count() == 2)
	for instance in far_patch.get_children():
		var species: int = instance.get_meta("species")
		assert(instance.get_meta("lod") == 1)
		assert(species == Species.CONIFER or species == Species.BROADLEAF)
		assert(instance.multimesh.mesh == vegetation.meshes[species][0][2])
		assert(instance.multimesh.instance_count == 512)
		assert(not instance.multimesh.use_colors)
	# Crossing back over the mid boundary swaps the mesh on the buffer already uploaded.
	vegetation._refresh_distant_lod(far_patch, 1500.0)
	assert(far_patch.get_meta("distant_lod") == 1)
	for instance in far_patch.get_children():
		assert(instance.multimesh.mesh == vegetation.meshes[instance.get_meta("species")][0][1])
		assert(instance.multimesh.instance_count == 512)
	camera.global_position = Vector3.ZERO
	# Empty buckets emit no nodes, and changing density preserves the original subset rule.
	vegetation.density_fraction = 0.0
	vegetation._upload_patch(Vector2i(0, 0), SPAN)
	assert(vegetation.patches[Vector2i(0, 0)].get_child_count() == 0)
	assert(vegetation.patches[Vector2i(0, 0)].get_meta("tree_count") == 0)
	await process_frame
	host.free()
	print("PASS vegetation appearance, shared material, LOD buckets, positions and empty density")
	quit()

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
	# Far vertex ceilings per species, tighter than the per-variant array they replace, which
	# held 81 for conifer against a measured 77. That array pinned which variants happened to
	# share vertices between their two end caps, and that is not what a distant mesh costs:
	# sizing the distant crowns to the near foliage envelope moved the sharing between variants
	# without adding one triangle anywhere. Triangles are asserted exactly below, so the vertex
	# figure only has to stay a ceiling.
	for species in range(Species.SPECIES_COUNT):
		for variant in range(Species.VARIANT_COUNTS[species]):
			var levels: Array = counts[species][variant]
			var before: int = [1803 if variant in [4, 8] else 1805, 1469, 160, 132][species]
			assert(levels[0][0] <= before * (2 if species < Species.BUSH else 1))
			if species < Species.BUSH:
				assert(levels[1][0] <= [296, 274][species])
				assert(levels[2][0] <= [77, 92][species])
				assert(levels[1][1] == [102, 96][species] and levels[2][1] == [28, 32][species])

func _check_crown_integral() -> void:
	# Two foliage triangles with areas 1 and 3, on separate surfaces, plus wood of area 5.
	# All emitted geometry contributes; exact 8-bit colours make the sum analytic.
	var mesh := ArrayMesh.new()
	for side in range(2):
		var surface := SurfaceTool.new()
		surface.begin(Mesh.PRIMITIVE_TRIANGLES)
		var color := Color(0.2, 0.4, 0.0) if side == 0 else Color(0.4, 0.8, 0.2)
		Species._tri(surface, color, Vector3.ZERO, Vector3.RIGHT * (2.0 if side == 0 else 6.0), Vector3.UP)
		if side == 0:
			Species._tri(surface, Color(0.8, 0.6, 0.4), Vector3.ZERO, Vector3.RIGHT * 10.0, Vector3.UP)
		surface.index()
		surface.commit(mesh)
	assert(Species._crown_integral(mesh).is_equal_approx(Vector4(5.4, 5.8, 2.6, 9.0)))

func _check_crown_colors(meshes: Array) -> Array:
	var report := []
	for species in [Species.CONIFER, Species.BROADLEAF]:
		var integral := Vector4.ZERO
		for levels in meshes[species]:
			integral += Species._crown_integral(levels[0])
		var mean := Vector3(integral.x, integral.y, integral.z) / integral.w
		for levels in meshes[species]:
			for lod in [1, 2]:
				var colors: PackedColorArray = levels[lod].surface_get_arrays(0)[Mesh.ARRAY_COLOR]
				for color in colors:
					if color.g > color.r:
						var error := (Vector3(color.r, color.g, color.b) - mean).abs()
						assert(maxf(error.x, maxf(error.y, error.z)) <= 1.0 / 255.0)
		report.append({"area":integral.w, "area_rgb":[integral.x, integral.y, integral.z],
			"mean_rgb":[mean.x, mean.y, mean.z]})
	return report

func _check_birch(variants: Array) -> void:
	var birches := 0
	for variant in range(variants.size()):
		var birch: bool = Species._is_birch(variant)
		assert(birch == (variant % 3 != 2))
		var mesh: ArrayMesh = variants[variant][0]
		var core := mesh.surface_get_arrays(0)
		var colors: PackedColorArray = core[Mesh.ARRAY_COLOR]
		var vertices: PackedVector3Array = core[Mesh.ARRAY_VERTEX]
		var pale := false
		var scar := false
		var bands := {}
		for vertex in range(vertices.size()):
			var color := colors[vertex]
			pale = pale or (color.r > 0.7 and color.b > 0.6)
			if birch and color.r < 0.2 and color.r > color.g:
				scar = scar or vertices[vertex].y < 0.35
				if vertices[vertex].y > 0.8 and color.r < 0.1:
					bands[snappedf(vertices[vertex].y, 0.001)] = true
		assert(pale == birch)
		if birch:
			birches += 1
			assert(scar and bands.size() >= 8)
		var cards := mesh.surface_get_arrays(1)
		for uv in cards[Mesh.ARRAY_TEX_UV]:
			assert(uv.y <= 0.5)
			assert(uv.x >= 0.5 if birch else uv.x <= 0.5)
		for color in cards[Mesh.ARRAY_COLOR]:
			assert(color.g > 0.25 if birch else color.g < 0.22)
	assert(birches == 8)

func _check_meshes(meshes: Array) -> void:
	var shared: Material = meshes[Species.ROCK][0][0].surface_get_material(0)
	var shared_distant: Array[Material] = [meshes[Species.CONIFER][0][1].surface_get_material(0),
		meshes[Species.BROADLEAF][0][1].surface_get_material(0)]
	var shared_wind: Material = meshes[0][0][0].surface_get_material(0)
	var shared_cards: Material = meshes[0][0][0].surface_get_material(1)
	assert(shared is StandardMaterial3D)
	assert(shared.vertex_color_use_as_albedo and shared.roughness == 1.0)
	assert(shared_wind is ShaderMaterial and shared_cards is ShaderMaterial)
	assert(shared_wind != shared_cards)
	assert(shared_wind.shader == preload("res://scripts/shaders/vegetation_wind.gdshader"))
	assert(shared_cards.shader == preload("res://scripts/shaders/vegetation_wind_cards.gdshader"))
	# One distant material per species, sharing one shader. They differ only in hash
	# coverage, which each species needs to reach its own near crown's silhouette fill.
	assert(shared_distant[0] != shared_distant[1])
	for material in shared_distant:
		assert(material is ShaderMaterial)
		assert(material.shader == preload("res://scripts/shaders/vegetation_distant.gdshader"))
	assert(shared_distant[0].get_shader_parameter("crown_coverage")
		!= shared_distant[1].get_shader_parameter("crown_coverage"))
	var distant_code: String = shared_distant[0].shader.code
	assert(distant_code.contains("BACKLIGHT = vegetation_backlight(COLOR.rgb);"))
	assert(distant_code.contains("ALPHA_SCISSOR_THRESHOLD = 0.4;"))
	assert(distant_code.contains("COLOR.g > COLOR.r"))
	# Source contracts only: the dummy renderer cannot compile shaders or prove pass routing.
	var card_code: String = shared_cards.shader.code
	assert(card_code.contains("render_mode world_vertex_coords, cull_disabled;"))
	assert(card_code.contains("ALPHA = texture(foliage_mask, UV).a;"))
	assert(card_code.contains("ALPHA_SCISSOR_THRESHOLD = 0.4;"))
	for code in [shared_wind.shader.code, card_code, distant_code]:
		for forbidden in ["blend_", "depth_draw_", "depth_prepass_alpha", "depth_test_",
			"unshaded", "ambient_light_disabled", "ALPHA_HASH", "alpha_to_coverage"]:
			assert(not code.contains(forbidden))
	assert(not shared_wind.shader.code.contains("ALPHA"))
	for species in range(Species.SPECIES_COUNT):
		assert(meshes[species].size() == Species.VARIANT_COUNTS[species])
		var silhouettes := {}
		var min_height := INF
		var max_height := 0.0
		for levels in meshes[species]:
			assert(levels.size() == (3 if species < Species.BUSH else 1))
			var bounds: AABB = levels[0].get_aabb()
			silhouettes[bounds] = true
			min_height = minf(min_height, bounds.size.y)
			max_height = maxf(max_height, bounds.size.y)
			for mesh in levels:
				# The understory is a catalogue of different ground plants, so its surfaces
				# are per-variant: a dwarf shrub mat is cards alone, a grass tuft is blades
				# alone, and only the woody forms carry both a stem surface and cards.
				var bush: bool = species == Species.BUSH
				var surfaces: int = mesh.get_surface_count()
				var has_cards: bool = mesh == levels[0] and (surfaces == 2 or (bush and _card_surface(mesh, shared_cards) == 0))
				if bush:
					assert(surfaces == 1 or surfaces == 2)
				else:
					assert(surfaces == (2 if species < Species.BUSH and mesh == levels[0] else 1))
				# Materials are cached across variants. The surface count above, rather than
				# the number of distinct materials, bounds the per-patch draw count.
				for surface in range(surfaces):
					var material: Material = mesh.surface_get_material(surface)
					assert(material in [shared_cards, shared_wind, shared] or material in shared_distant)
				if not bush:
					if has_cards:
						assert(mesh.surface_get_material(1) == shared_cards)
					var expected := shared_wind if has_cards else (
						shared_distant[species] if species < Species.BUSH else shared)
					assert(mesh.surface_get_material(0) == expected)
				# Vertex colour alpha is the sway weight, never opacity. A wind mesh must stay
				# anchored somewhere and reach full sway somewhere, and every card must carry
				# the weight of the core beneath it, because a card and that core tear apart
				# the moment their weights differ.
				if bush:
					# Ground plants bend from their own base, so weight is proportional to
					# height above the ground rather than to a shared ring schedule. The
					# contract that survives is the range: anchored at zero and never past
					# the cap a bush is allowed, on every surface.
					for surface in range(surfaces):
						_check_sway_range(mesh, surface, 0.8)
				else:
					var top_weight := 1.0
					_check_sway_weights(mesh, 0, 0.0 if has_cards else 1.0, top_weight)
					if has_cards:
						# Tree cards all sit at the crown.
						_check_sway_weights(mesh, 1, 1.0, top_weight)
		assert(silhouettes.size() == Species.VARIANT_COUNTS[species])
		# Trees and rocks are variants of one plant and must keep its proportions. The
		# understory deliberately is not: a 0.2 m blueberry mat and a 1.6 m spruce sapling
		# are both ground layer, and forcing them to one height is what produced a field of
		# identical 3 m cones.
		if species != Species.BUSH:
			assert(max_height / min_height < 1.3, "Variants must retain species proportions")

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
	assert(is_equal_approx(lowest, expect_min) and is_equal_approx(highest, expect_max))

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

## Both broadleaf appearances and both conifer seed parities retain card topology.
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
	for kind in range(3):
		var conifer := kind == 2
		var birch := kind == 1
		for seed in [0, 1, 2, 3, -1]:
			var surface := SurfaceTool.new()
			surface.begin(Mesh.PRIMITIVE_TRIANGLES)
			Species._foliage_cards(surface, Vector3.UP, Vector3.ZERO,
				Vector3.RIGHT, Vector3.ONE, conifer, seed, 0.8, birch)
			var arrays := surface.commit().surface_get_arrays(0)
			var uvs: PackedVector2Array = arrays[Mesh.ARRAY_TEX_UV]
			assert(uvs.size() == 18)
			var origin := Vector2(float(seed & 1) if conifer else float(kind), 1.0 if conifer else 0.0) * 0.5
			var corners := [origin + Vector2(0, 0.5), origin + Vector2(0.5, 0.5),
				origin + Vector2(0.5, 0), origin]
			for vertex in range(18):
				assert(uvs[vertex] == corners[[0, 2, 1, 0, 3, 2][vertex % 6]])
