# SPDX-License-Identifier: GPL-2.0-only

## Exports the live canopy catalogue for tools/vegetation_lod_measure.py.
## Runs with Godot's headless dummy renderer; no gameplay scene or simulation setup.
extends SceneTree

const Species = preload("res://scripts/renderers/tree_species.gd")

func _initialize() -> void:
	var args := OS.get_cmdline_user_args()
	if args.size() != 1:
		quit(2)
		return
	var start := Time.get_ticks_usec()
	var catalogue := Species.build_meshes()
	var build_us := Time.get_ticks_usec() - start
	var meshes := []
	for species in [Species.CONIFER, Species.BROADLEAF]:
		for variant in range(Species.VARIANT_COUNTS[species]):
			for lod in range(catalogue[species][variant].size()):
				# Runtime collapses distant variants to zero.
				if lod >= 2 and variant > 0:
					continue
				var mesh: ArrayMesh = catalogue[species][variant][lod]
				meshes.append(export_mesh(mesh, species, variant, lod))
	var file := FileAccess.open(args[0], FileAccess.WRITE)
	if file == null:
		quit(3)
		return
	file.store_string(JSON.stringify({"build_us": build_us, "meshes": meshes,
		"impostor_source_json": impostor_source_json(catalogue)}))
	quit()


## Canonical mesh payload shared by the CPU bake and the stale-bake regression.
static func export_mesh(mesh: ArrayMesh, species: int, variant: int, lod: int) -> Dictionary:
	var surfaces := []
	for surface in range(mesh.get_surface_count()):
		var arrays := mesh.surface_get_arrays(surface)
		var material := mesh.surface_get_material(surface)
		var data := {"vertices": [], "normals": [], "colors": [], "uv": [],
			"indices": Array(arrays[Mesh.ARRAY_INDEX]), "shader": "standard"}
		for v in arrays[Mesh.ARRAY_VERTEX]:
			data.vertices.append([v.x, v.y, v.z])
		for n in arrays[Mesh.ARRAY_NORMAL]:
			data.normals.append([n.x, n.y, n.z])
		for c in arrays[Mesh.ARRAY_COLOR]:
			data.colors.append([c.r, c.g, c.b, c.a])
		if arrays[Mesh.ARRAY_TEX_UV] != null:
			for uv in arrays[Mesh.ARRAY_TEX_UV]:
				data.uv.append([uv.x, uv.y])
		if material is ShaderMaterial:
			data.shader = material.shader.resource_path.get_file()
			data.parameters = {}
			for uniform in material.shader.get_shader_uniform_list():
				# The impostor applies the leaf share per layer, so the bake reads unscaled leaves.
				if uniform.name == "leaf_albedo_scale":
					continue
				var value = material.get_shader_parameter(uniform.name)
				if value is float or value is int:
					data.parameters[uniform.name] = value
				elif value is Texture2D:
					# The bake reads alpha from the same file the card samples.
					data.parameters[uniform.name] = (value as Texture2D).resource_path
		surfaces.append(data)
	var bounds := mesh.get_aabb()
	var centre := bounds.get_center()
	return {"species": species, "variant": variant, "lod": lod, "surfaces": surfaces,
		"centre": [centre.x, centre.y, centre.z], "size": bounds.size.length()}

## Hash the exact full-precision JSON bytes, avoiding cross-language float formatting.
static func impostor_source_json(catalogue: Array) -> String:
	var sources := []
	# Every variant, from the reduced level: that is the tree the impostor replaces at the switch.
	for species in [Species.CONIFER, Species.BROADLEAF]:
		for variant in range(Species.VARIANT_COUNTS[species]):
			sources.append(export_mesh(catalogue[species][variant][1], species, variant, 1))
	return JSON.stringify(sources, "", true, true)
