# SPDX-License-Identifier: GPL-2.0-only

## Decorative species mesh catalogue, ordered by the Rust placement species IDs.
## Owns mesh construction independently of patch residency and instance placement.
extends RefCounted

# Matches the renderers' idiom. A bare SceneLighting class_name reference needs Godot's
# global class cache, which a freshly cloned checkout has not built yet.
const SceneLightingConfig := preload("res://scripts/core/scene_lighting.gd")

# Species ordinals must match vegetation_api.rs.
const CONIFER := 0
const BROADLEAF := 1
const BUSH := 2
const ROCK := 3
const SPECIES_COUNT := 4
const VARIANT_COUNTS := [12, 12, 15, 6]
# Variants the appearance seed may pick when no brush named one. Bush variants past these are
# named yard plants, which the generator must never scatter through a forest.
const SEED_VARIANT_COUNTS := [12, 12, 6, 6]

# Distances over which trees stop receiving cast shadows, as metres inside the sun's shadow range.
# Every tree casts its impostor's silhouette, so trees can receive as far as the shadow maps
# reach; the canopy shade term takes over past them.
const TREE_RECEIVE_FADE_M := 80.0
const TREE_RECEIVE_MARGIN_M := 20.0
# Distances over which each tree hands over from its branched level to its impostor, by its own
# distance to the camera; see vegetation_wind.gdshaderinc. At 1080p and a 75-degree field of view
# a 15 m tree covers 70 px at the start and 53 px at the end, against 64 px baked views.
const TREE_CROSSFADE_M := 50.0
# Distance past which no tree is drawn, and the band each tree dissolves over before it. The
# ground takes the stand over across the same band; see CANOPY_FAR_SIDE_RATIO.
const TREE_FAR_M := 4500.0
const TREE_FAR_FADE_M := 500.0
# An authored crown is open, and past the shadow range it is a few thin pixels over a sunlit
# floor. There a real crown's gaps are below a pixel and a stand reads as solid, and the old
# closed domes covered twice the screen share of these crowns at the same density. So a far
# crown grows from its size at the shadow range to CROWN_GROWTH times it over CROWN_GROWTH_SPAN_M.
# The handover and every cast shadow are nearer, so neither sees the growth.
const CROWN_GROWTH := 1.6
const CROWN_GROWTH_SPAN_M := 1000.0
# Past the trees the terrain shader stands in for them by mixing toward one crown albedo by the
# share of each sight line a random stand of the published crown coverage would stop; see
# terrain.gdshader. The ratio is a crown's side area over its top area, which is near one for
# these crowns. The albedo is fitted to the authored trees: a painted stand 5 km away, seen from
# 450 m and from 1500 m up, rendered as ground at two albedos and as trees drawn to 12 km, solves
# per channel to (0.13-0.14, 0.17-0.18, 0.05-0.06). The procedural trees' (0.12, 0.13, 0.045)
# had a quarter less green and turned an authored stand brown at the handover.
const CANOPY_FAR_SIDE_RATIO := 1.0
const CANOPY_FAR_ALBEDO := Vector3(0.135, 0.18, 0.06)
# Relative albedo spread between neighbouring far crowns. Drawn trees differ crown to crown
# where one albedo draws a flat patch.
const CANOPY_FAR_SPECKLE := 0.8
const TREE_CROSSFADE_END_M := 200.0
# Distance a tree starts to hand over from its full branched level to the reduced one, and the
# band it dissolves over, by its own distance like the impostor handover; see vegetation.gd.
const TREE_NEAR_DETAIL_M := 45.0
const TREE_DETAIL_FADE_M := 10.0

# Understory cards sample the shared foliage_atlas.dds. Its mip 0, alpha >= 102/255 (0.4),
# measured 2026-09-26, has these half-open pixel bounds within each 256x256 cell: (11,24)-(233,230),
# (12,5)-(251,234), (24,5)-(250,234), (8,38)-(249,250).
# UV and position use the SAME affine crop, retaining the opaque content's size/location.
const FOLIAGE_ALPHA_RECTS := [
	Rect2(11.0/256.0, 24.0/256.0, 222.0/256.0, 206.0/256.0),
	Rect2(12.0/256.0, 5.0/256.0, 239.0/256.0, 229.0/256.0),
	Rect2(24.0/256.0, 5.0/256.0, 226.0/256.0, 229.0/256.0),
	Rect2(8.0/256.0, 38.0/256.0, 241.0/256.0, 212.0/256.0),
]

# Share of its baked albedo an impostor keeps, conifer first, so that it renders to the near
# level's luminance. Fitted over the poses of vegetation_level_match_test.gd.
const IMPOSTOR_RADIANCE_MATCH := [1.033, 1.097]
# Volume response of an impostor, conifer first, as (lift gain, wrap floor, wrap gain); see
# vegetation_impostor.gdshader. Fitted with the radiance match above over the same poses, on
# the authored trees; both species fit best at the same response.
const IMPOSTOR_VOLUME := [Vector3(2.0, 0.0, 0.75), Vector3(2.0, 0.0, 0.75)]
# Share of the near crown's sun highlight an impostor draws, fitted with the response above.
# Backlit by a low sun, the near cards catch a sheen the impostor otherwise lacks.
const IMPOSTOR_SHEEN := 0.5
# One baked impostor per near variant, so a tree keeps its own shape across the handover. Four
# shared forms stood in for 24 variants, and each tree changed into another as the camera closed.
const IMPOSTOR_SPECIES_NAMES := ["conifer", "broadleaf"]

static var _wind_material: ShaderMaterial
static var _card_material: ShaderMaterial
static var _card_texture: Texture2D

# Canopy trees are authored in Blender by tools/model_trees.py and converted by
# tools/prepare_tree_models.py. Each species has a major and a minor form: two of each three
# variants take the major one, which matches the order of Finnish growing stock (pine leads
# spruce, birch leads aspen). Each form has FORM_MODELS models, which its variants cycle through.
const TREE_MODEL_DIR := "res://assets/models/vegetation/trees/"
const CANOPY_FORMS := [["pine", "spruce"], ["birch", "aspen"]]
const FORM_MODELS := 3
# Sway weight of the trunk top, as the procedural trunks carried it. Everything farther from the
# trunk takes the rest in proportion to its reach, so branch tips and their cards sway fully.
const TRUNK_TOP_WEIGHT := 0.16

# Yard plants from tools/model_landscape.py, converted like the trees: the bush variants from
# LANDSCAPE_FIRST_VARIANT on, in the order of the named brush presets in vegetation_api/brush.rs.
# The last three are clipped hedge modules, one metre long, which a line lays end to end.
const LANDSCAPE_MODEL_DIR := "res://assets/models/vegetation/landscape/"
const LANDSCAPE_FORMS := ["lilac", "spirea", "rose", "cotoneaster", "mugo", "juniper",
	"hedge_low", "hedge_mid", "hedge_tall"]
const LANDSCAPE_FIRST_VARIANT := 6
const HEDGE_FIRST_VARIANT := 12
# Sway weight per metre above the ground. A shrub bends from its base like the ground plants; a
# clipped hedge is a dense block and barely moves.
const SHRUB_SWAY_PER_M := 0.18
const HEDGE_SWAY_PER_M := 0.03
# Granite boulders from tools/model_landscape.py, on one tiled stone texture.
const ROCK_MODEL_DIR := "res://assets/models/vegetation/rocks/"

# Per model directory, the converter's measured means of each form.
static var _model_info: Dictionary = {}
static var _rock_material: StandardMaterial3D
static var _form_meshes: Dictionary = {}
# Per form, and per form plus "_lod1" for the reduced level: [wood, cards]. Every one of them is
# also in _canopy_materials for set_crossfade and set_detail.
static var _form_materials: Dictionary = {}
static var _canopy_materials: Array[ShaderMaterial] = []

static var _impostor_metadata: Dictionary = {}
static var _impostor_materials: Array[ShaderMaterial] = [null, null]
static var _impostor_shadow_materials: Array[ShaderMaterial] = [null, null]
static var _impostor_quad: QuadMesh

## Meshes indexed by species, variant, then level: canopy trees carry the full and the reduced
## level, understory and rocks one.
static func build_meshes() -> Array:
	var meshes: Array = []
	meshes.resize(SPECIES_COUNT)
	for species in range(SPECIES_COUNT):
		var variants: Array = []
		variants.resize(VARIANT_COUNTS[species])
		for variant in range(variants.size()):
			match species:
				CONIFER, BROADLEAF:
					variants[variant] = [_canopy_tree(species, variant, 0),
						_canopy_tree(species, variant, 1), _canopy_blend(species, variant)]
				BUSH:
					variants[variant] = [_bush(variant) if variant < LANDSCAPE_FIRST_VARIANT
						else _landscape_plant(variant)]
				ROCK:
					variants[variant] = [_rock(variant)]
		meshes[species] = variants
	return meshes

## Authored form of one canopy variant, as "<form>_<model>".
static func canopy_model(species: int, variant: int) -> String:
	var minor := variant % 3 == 2
	# Position of this variant among the variants that share its form.
	var index := variant / 3 if minor else variant / 3 * 2 + variant % 3
	return "%s_%d" % [CANOPY_FORMS[species][1 if minor else 0], index % FORM_MODELS]

## One authored tree at one level, loaded once per model and shared by the variants that cycle
## onto it. Level 1 is the reduced tree tools/model_trees.py exports beside the full one.
static func _canopy_tree(species: int, variant: int, level: int) -> ArrayMesh:
	return _authored_mesh(TREE_MODEL_DIR, "trees.json",
		canopy_model(species, variant) + ("_lod1" if level == 1 else ""), -1.0)

## Both levels of one canopy model in one mesh: the full level's surfaces, then the reduced
## level's on their own materials. A patch inside the detail band draws this, and each tree
## shares its pixels between the two by its own distance. Startup only; the mesh holds a second
## copy of both levels' vertices, about 0.4 MB per model.
static func _canopy_blend(species: int, variant: int) -> ArrayMesh:
	var model := canopy_model(species, variant) + "_blend"
	if _form_meshes.has(model):
		return _form_meshes[model]
	var mesh := ArrayMesh.new()
	for level in [_canopy_tree(species, variant, 0), _canopy_tree(species, variant, 1)]:
		for surface in range(level.get_surface_count()):
			mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, level.surface_get_arrays(surface))
			mesh.surface_set_material(mesh.get_surface_count() - 1, level.surface_get_material(surface))
	_form_meshes[model] = mesh
	return mesh

## One yard plant. A hedge module stays rigid apart from a faint sway at its top.
static func _landscape_plant(variant: int) -> ArrayMesh:
	var hedge := variant >= HEDGE_FIRST_VARIANT
	return _authored_mesh(LANDSCAPE_MODEL_DIR, "landscape.json",
		LANDSCAPE_FORMS[variant - LANDSCAPE_FIRST_VARIANT] + "_0",
		HEDGE_SWAY_PER_M if hedge else SHRUB_SWAY_PER_M)

## Loads one converted model once and gives it the form's wind materials. `sway_per_m` below
## zero weights sway as a tree's: the trunk top takes TRUNK_TOP_WEIGHT and everything farther
## out takes the rest by its reach. Otherwise the weight grows with height above the ground.
## Startup only: O(vertices) to recolour and weight the imported surfaces.
static func _authored_mesh(directory: String, info_file: String, model: String,
		sway_per_m: float) -> ArrayMesh:
	if _form_meshes.has(model):
		return _form_meshes[model]
	var scene: Node = (load(directory + model + ".glb") as PackedScene).instantiate()
	var source: Mesh = (scene.find_children("*", "MeshInstance3D", true, false)[0] as MeshInstance3D).mesh
	scene.free()
	# The form is the model name without its level suffix and its index.
	var base := model.trim_suffix("_lod1")
	var form := base.substr(0, base.rfind("_"))
	var materials := _form_material_pair(directory, info_file, form, base != model)
	var info: Dictionary = _form_info(directory, info_file)[form]
	var bounds := source.get_aabb()
	var reach := maxf(maxf(absf(bounds.position.x), absf(bounds.end.x)),
		maxf(absf(bounds.position.z), absf(bounds.end.z)))
	var mesh := ArrayMesh.new()
	for surface in range(source.get_surface_count()):
		var arrays := source.surface_get_arrays(surface)
		var cards := source.surface_get_material(surface).resource_name.ends_with("_foliage")
		var key: Array = info["leaf_mean" if cards else "bark_mean"]
		var vertices: PackedVector3Array = arrays[Mesh.ARRAY_VERTEX]
		var colors: PackedColorArray = arrays[Mesh.ARRAY_COLOR]
		# The authored colour is crown occlusion on the cards and a tint on the bark. Scaled by the
		# texture mean it becomes the mean albedo the shaders and the distant levels expect, and
		# alpha becomes the sway weight.
		for i in range(vertices.size()):
			var at := vertices[i]
			var weight := clampf(TRUNK_TOP_WEIGHT * at.y / bounds.end.y
				+ (1.0 - TRUNK_TOP_WEIGHT) * Vector2(at.x, at.z).length() / reach, 0.0, 1.0) \
				if sway_per_m < 0.0 else clampf(at.y * sway_per_m, 0.0, 0.8)
			colors[i] = Color(colors[i].r * key[0], colors[i].g * key[1], colors[i].b * key[2], weight)
		var kept := []
		kept.resize(Mesh.ARRAY_MAX)
		for channel in [Mesh.ARRAY_VERTEX, Mesh.ARRAY_NORMAL, Mesh.ARRAY_TEX_UV, Mesh.ARRAY_INDEX]:
			kept[channel] = arrays[channel]
		kept[Mesh.ARRAY_COLOR] = colors
		mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, kept)
		mesh.surface_set_material(mesh.get_surface_count() - 1, materials[1 if cards else 0])
	_form_meshes[model] = mesh
	return mesh

static func _form_info(directory: String, info_file: String) -> Dictionary:
	if not _model_info.has(directory):
		_model_info[directory] = JSON.parse_string(FileAccess.get_file_as_string(directory + info_file))
	return _model_info[directory]

## One wood and one card material per form and level, shared by its models and by every patch.
static func _form_material_pair(directory: String, info_file: String, form: String,
		reduced: bool) -> Array:
	var key := form + ("_lod1" if reduced else "")
	if not _form_materials.has(key):
		var info: Dictionary = _form_info(directory, info_file)[form]
		var wood := ShaderMaterial.new()
		wood.shader = preload("res://scripts/shaders/vegetation_wind.gdshader")
		wood.set_shader_parameter("bark_albedo", load(directory + form + "_bark.png"))
		wood.set_shader_parameter("bark_mean", Vector3(info.bark_mean[0], info.bark_mean[1], info.bark_mean[2]))
		var cards := ShaderMaterial.new()
		cards.shader = preload("res://scripts/shaders/vegetation_wind_cards.gdshader")
		cards.set_shader_parameter("foliage_mask", load(directory + form + "_foliage.dds"))
		cards.set_shader_parameter("leaf_mean", Vector3(info.leaf_mean[0], info.leaf_mean[1], info.leaf_mean[2]))
		cards.set_shader_parameter("leaf_depth_mean", info.leaf_depth_mean)
		for material in [wood, cards]:
			_apply_canopy_shading(material)
			_apply_wind_visibility(material)
			material.set_shader_parameter("reduced_level", reduced)
			_canopy_materials.append(material)
		_form_materials[key] = [wood, cards]
	return _form_materials[key]

## Baked form name of one near variant, as the bake tool and the texture files spell it.
static func impostor_form(species: int, variant: int) -> String:
	return "%s_%02d" % [IMPOSTOR_SPECIES_NAMES[species], variant]

## One quad shared by every form; the shader reconstructs its object-space vertices.
static func impostor_mesh() -> QuadMesh:
	if _impostor_quad == null:
		_impostor_quad = QuadMesh.new()
	return _impostor_quad

## Baked bounds and source digest, loaded once alongside the texture arrays.
static func impostor_metadata() -> Dictionary:
	if _impostor_metadata.is_empty():
		_impostor_metadata = JSON.parse_string(FileAccess.get_file_as_string(
			"res://assets/textures/vegetation/tree_impostors.json"))
	return _impostor_metadata

## Each species shares two texture arrays and one material across all resident patches. The
## array layer is the near variant.
static func impostor_material(species: int) -> ShaderMaterial:
	if _impostor_materials[species] == null:
		var metadata := impostor_metadata()
		var material := ShaderMaterial.new()
		material.shader = preload("res://scripts/shaders/vegetation_impostor.gdshader")
		for channel in ["albedo", "normal"]:
			var images: Array[Image] = []
			for variant in range(VARIANT_COUNTS[species]):
				var texture: Texture2D = load("res://assets/textures/vegetation/tree_impostor_%s_%s.dds"
					% [impostor_form(species, variant), channel])
				images.append(texture.get_image())
			var array := Texture2DArray.new()
			var error := array.create_from_images(images)
			assert(error == OK, "Cannot create tree impostor texture array")
			material.set_shader_parameter(channel + "_atlas", array)
		var centres := PackedVector3Array()
		var sizes := PackedFloat32Array()
		for variant in range(VARIANT_COUNTS[species]):
			var bounds: Dictionary = metadata.forms[impostor_form(species, variant)]
			centres.append(Vector3(bounds.centre[0], bounds.centre[1], bounds.centre[2]))
			sizes.append(bounds.size)
		material.set_shader_parameter("bounds_centre", centres)
		material.set_shader_parameter("bounds_size", sizes)
		material.set_shader_parameter("frame_bounds", _impostor_frame_bounds(species, metadata))
		material.set_shader_parameter("impostor_radiance_match", IMPOSTOR_RADIANCE_MATCH[species])
		var volume: Vector3 = IMPOSTOR_VOLUME[species]
		material.set_shader_parameter("volume_lift_gain", volume.x)
		material.set_shader_parameter("volume_wrap_floor", volume.y)
		material.set_shader_parameter("volume_wrap_gain", volume.z)
		material.set_shader_parameter("impostor_sheen", IMPOSTOR_SHEEN)
		var growth_begin := SceneLightingConfig.shadow_max_distance_m()
		material.set_shader_parameter("crown_growth", CROWN_GROWTH)
		material.set_shader_parameter("crown_growth_begin_m", growth_begin)
		material.set_shader_parameter("crown_growth_end_m", growth_begin + CROWN_GROWTH_SPAN_M)
		_apply_canopy_shading(material)
		_apply_far(material, TREE_FAR_M)
		_impostor_materials[species] = material
	return _impostor_materials[species]

## Each baked view's coverage rectangle, one texel per view and one row per form; the impostor
## crops its quad to them. See vegetation_impostor.gdshaderinc.
static func _impostor_frame_bounds(species: int, metadata: Dictionary) -> ImageTexture:
	var image := Image.create(64, VARIANT_COUNTS[species], false, Image.FORMAT_RGBAF)
	for variant in range(VARIANT_COUNTS[species]):
		var bounds: Array = metadata.forms[impostor_form(species, variant)].frame_bounds
		for view in range(64):
			var rect: Array = bounds[view]
			image.set_pixel(view, variant, Color(rect[0], rect[1], rect[2], rect[3]))
	return ImageTexture.create_from_image(image)

## The shadow caster for trees drawn as impostors: the same forms, faced to the light. Shares
## the impostor's texture arrays and bounds, so it adds no texture memory.
static func impostor_shadow_material(species: int) -> ShaderMaterial:
	if _impostor_shadow_materials[species] == null:
		var visible := impostor_material(species)
		var material := ShaderMaterial.new()
		material.shader = preload("res://scripts/shaders/vegetation_impostor_shadow.gdshader")
		for parameter in ["albedo_atlas", "bounds_centre", "bounds_size", "frame_bounds"]:
			material.set_shader_parameter(parameter, visible.get_shader_parameter(parameter))
		_impostor_shadow_materials[species] = material
	return _impostor_shadow_materials[species]

## Moves the far fade of the impostors to end on `end_m`, as set_crossfade does for the handover.
## The ground term keeps TREE_FAR_M, so a probe that draws trees farther also draws both there.
static func set_far(end_m: float) -> void:
	for material in _impostor_materials:
		if material != null:
			_apply_far(material, end_m)

static func _apply_far(material: ShaderMaterial, end_m: float) -> void:
	material.set_shader_parameter("tree_far_begin_m", end_m - TREE_FAR_FADE_M)
	material.set_shader_parameter("tree_far_end_m", end_m)

## The ground half of the far fade, for one terrain material.
static func apply_far_canopy(material: ShaderMaterial) -> void:
	material.set_shader_parameter("canopy_far_begin_m", TREE_FAR_M - TREE_FAR_FADE_M)
	material.set_shader_parameter("canopy_far_end_m", TREE_FAR_M)
	material.set_shader_parameter("canopy_far_side_ratio", CANOPY_FAR_SIDE_RATIO)
	material.set_shader_parameter("canopy_far_albedo", CANOPY_FAR_ALBEDO)
	material.set_shader_parameter("canopy_far_speckle", CANOPY_FAR_SPECKLE)

## Moves the handover of every tree material to end on `end_m`. The renderer calls this with its
## own near range, so a probe override moves the shaders with the patch ranges.
static func set_crossfade(end_m: float) -> void:
	for material in [_wind_material, _card_material, _impostor_materials[0], _impostor_materials[1]] \
			+ _canopy_materials:
		if material != null:
			_apply_crossfade(material, end_m)

static func _apply_crossfade(material: ShaderMaterial, end_m: float) -> void:
	material.set_shader_parameter("tree_crossfade_begin_m", end_m - TREE_CROSSFADE_M)
	material.set_shader_parameter("tree_crossfade_end_m", end_m)

## Moves the handover of every branched tree material from its full to its reduced level to begin
## on `begin_m`. The renderer calls this with its own detail distance, as with set_crossfade.
static func set_detail(begin_m: float) -> void:
	for material in _canopy_materials:
		_apply_detail(material, begin_m)

static func _apply_detail(material: ShaderMaterial, begin_m: float) -> void:
	material.set_shader_parameter("tree_detail_begin_m", begin_m)
	material.set_shader_parameter("tree_detail_end_m", begin_m + TREE_DETAIL_FADE_M)

# Understory and rocks are procedural: ragged radius profiles and crossed cards.

## Deterministic value in [0,1) from two integers. Cosmetic only; never feeds simulation.
static func _noise(a: int, b: int) -> float:
	var h := a * 374761393 + b * 668265263
	h = (h ^ (h >> 13)) * 1274126177
	return float((h ^ (h >> 16)) & 0xFFFFFF) / 16777216.0

## Builds a lathe from `profile` entries of (height_m, radius_m), ordered base upward.
static func _lathe(
	surface: SurfaceTool,
	profile: PackedVector2Array,
	segments: int,
	color: Color,
	ragged: float,
	seed_offset: int,
	shade: float = 0.0,
	weights: PackedFloat32Array = PackedFloat32Array()
) -> void:
	# Flat shading: the existing art is faceted, and smooth normals on a ragged lathe read
	# as a melted blob rather than as foliage.
	surface.set_smooth_group(-1)
	var radii: Array[PackedFloat32Array] = []
	for ring in range(profile.size()):
		var row := PackedFloat32Array()
		for segment in range(segments):
			var jitter := (_noise(ring + seed_offset, segment) - 0.5) * 2.0 * ragged
			row.append(maxf(profile[ring].y * (1.0 + jitter), 0.0))
		radii.append(row)
	# A profile that does not close to a point leaves an open end, which reads as a hollow
	# ring from above. Cap both ends with a fan whenever they still have radius.
	for end_ring in [0, profile.size() - 1]:
		if profile[end_ring].y <= 0.02:
			continue
		var cap_color := color
		if not weights.is_empty():
			cap_color.a = weights[end_ring]
		var centre := Vector3(0.0, profile[end_ring].x, 0.0)
		for segment in range(segments):
			var next_segment := (segment + 1) % segments
			var a0 := TAU * float(segment) / float(segments)
			var a1 := TAU * float(next_segment) / float(segments)
			var rim_a := Vector3(
				cos(a0) * radii[end_ring][segment],
				profile[end_ring].x,
				sin(a0) * radii[end_ring][segment]
			)
			var rim_b := Vector3(
				cos(a1) * radii[end_ring][next_segment],
				profile[end_ring].x,
				sin(a1) * radii[end_ring][next_segment]
			)
			# Wind the base cap the other way so both caps face outward.
			if end_ring == 0:
				_tri(surface, cap_color, centre, rim_b, rim_a)
			else:
				_tri(surface, cap_color, centre, rim_a, rim_b)
	for ring in range(profile.size() - 1):
		var y0 := profile[ring].x
		var y1 := profile[ring + 1].x
		for segment in range(segments):
			var next_segment := (segment + 1) % segments
			var a0 := TAU * float(segment) / float(segments)
			var a1 := TAU * float(next_segment) / float(segments)
			var lower_a := Vector3(cos(a0) * radii[ring][segment], y0, sin(a0) * radii[ring][segment])
			var lower_b := Vector3(cos(a1) * radii[ring][next_segment], y0, sin(a1) * radii[ring][next_segment])
			var upper_a := Vector3(
				cos(a0) * radii[ring + 1][segment], y1, sin(a0) * radii[ring + 1][segment]
			)
			var upper_b := Vector3(
				cos(a1) * radii[ring + 1][next_segment], y1, sin(a1) * radii[ring + 1][next_segment]
			)
			# Degenerate rings appear where a profile closes to a point, so skip zero-area
			# triangles rather than emitting normals that cannot be computed.
			if not (lower_a.is_equal_approx(lower_b) and upper_a.is_equal_approx(upper_b)):
				# Per-face shade. A crown of one flat colour reads as a solid object; real
				# foliage is a mass of differently lit surfaces.
				var face := color.lerp(
					color * 1.35, _noise(ring * 31 + seed_offset, segment * 7) * shade
				)
				if weights.is_empty():
					_tri(surface, face, lower_a, upper_a, upper_b)
					_tri(surface, face, lower_a, upper_b, lower_b)
				else:
					_weighted_tri(surface, face, lower_a, upper_a, upper_b,
						Vector3(weights[ring], weights[ring + 1], weights[ring + 1]))
					_weighted_tri(surface, face, lower_a, upper_b, lower_b,
						Vector3(weights[ring], weights[ring + 1], weights[ring]))

static func _tri(surface: SurfaceTool, color: Color, a: Vector3, b: Vector3, c: Vector3) -> void:
	# Godot treats clockwise-from-the-front as the front face. The lathe walks rings in
	# increasing angle, which is counter-clockwise seen from outside, so the order is
	# reversed here. Emitted the other way every surface is inside out: back-face culling
	# then leaves only the far inner wall, and a small dome reads as a hollow ring.
	for vertex in [a, c, b]:
		surface.set_color(color)
		surface.add_vertex(vertex)

# Same winding as _tri; only alpha varies between vertices, preserving face RGB.
static func _weighted_tri(surface: SurfaceTool, color: Color,
	a: Vector3, b: Vector3, c: Vector3, weights: Vector3) -> void:
	color.a = weights.x
	surface.set_color(color)
	surface.add_vertex(a)
	color.a = weights.z
	surface.set_color(color)
	surface.add_vertex(c)
	color.a = weights.y
	surface.set_color(color)
	surface.add_vertex(b)

static func _finish(surface: SurfaceTool, material: Material) -> ArrayMesh:
	surface.generate_normals()
	surface.set_material(material)
	# Primitive expansion duplicates shared vertices. Restore indexing for vertex-cache reuse.
	surface.index()
	return surface.commit()

static func _wind_branch_material() -> ShaderMaterial:
	if _wind_material == null:
		_wind_material = ShaderMaterial.new()
		_wind_material.shader = preload("res://scripts/shaders/vegetation_wind.gdshader")
		_apply_canopy_shading(_wind_material)
		_apply_wind_visibility(_wind_material)
	return _wind_material

## Ties the canopy shading ramp to the distance trees stop receiving cast shadows. The term
## replaces the darkening those shadows stop supplying, so both use one pair of distances.
## Every material is built once at startup, so this is not on a per-frame path.
## Vertical field of view of the player camera, in degrees. Nothing in the project assigns
## `Camera3D.fov`, so the camera runs on the engine default and this is that value. It feeds the
## one projection term the wind gate cannot read from a vertex built-in.
const CAMERA_FOV_DEG := 75.0

## Supplies the wind gate its projection term. The gate decides how much sway a tree keeps from
## how many pixels that sway covers, and reads the window size from `VIEWPORT_SIZE` itself, so
## this is the only part of the projection it needs and a resize needs no update. Materials are
## built once at startup, so this is not on a per-frame path.
static func _apply_wind_visibility(material: ShaderMaterial) -> void:
	material.set_shader_parameter(
		"camera_projection_scale", 1.0 / tan(deg_to_rad(CAMERA_FOV_DEG * 0.5))
	)

static func _apply_canopy_shading(material: ShaderMaterial) -> void:
	_apply_crossfade(material, TREE_CROSSFADE_END_M)
	_apply_detail(material, TREE_NEAR_DETAIL_M)
	material.set_shader_parameter("canopy_base_begin_m", TREE_CROSSFADE_END_M)
	var receive_end := SceneLightingConfig.shadow_max_distance_m() - TREE_RECEIVE_MARGIN_M
	material.set_shader_parameter("tree_receive_begin_m", receive_end - TREE_RECEIVE_FADE_M)
	material.set_shader_parameter("tree_receive_end_m", receive_end)
	material.set_shader_parameter("canopy_shade", SceneLightingConfig.canopy_shade_strength())

## One cached atlas serves every card; DDS retains the baked coverage-corrected mips.
static func _foliage_material() -> ShaderMaterial:
	if _card_material == null:
		if _card_texture == null:
			_card_texture = load("res://assets/textures/vegetation/foliage_atlas.dds")
		_card_material = ShaderMaterial.new()
		_card_material.shader = preload("res://scripts/shaders/vegetation_wind_cards.gdshader")
		_card_material.set_shader_parameter("foliage_mask", _card_texture)
		_apply_canopy_shading(_card_material)
		_apply_wind_visibility(_card_material)
	return _card_material

## Three crossed quads share a tilted growth axis, lit as a cluster facing out of `crown_centre`.
## Colour is per plant; sway grows with height above the plant's base at `sway_per_m`.
static func _foliage_cards(surface: SurfaceTool, centre: Vector3, crown_centre: Vector3,
	direction: Vector3, size: Vector3, conifer: bool, seed: int, color: Color,
	growth_axis: Vector3, sway_per_m: float) -> void:
	var along := Vector3(direction.x, 0.0, direction.z).normalized()
	var axis := growth_axis.normalized()
	var right := along.cross(axis).normalized()
	var normal := (centre - crown_centre).normalized()
	var phase := _noise(seed, 307) * PI
	# Top left broadleaf cell, bottom left spruce frond.
	var uv_origin := Vector2(0.0, 0.5 if conifer else 0.0)
	var trim: Rect2 = FOLIAGE_ALPHA_RECTS[2 if conifer else 0]
	var local_uvs := [Vector2(trim.position.x, trim.end.y), trim.end,
		Vector2(trim.end.x, trim.position.y), trim.position]
	for plane in range(3):
		var radial := right.rotated(axis, phase + float(plane) * PI / 3.0)
		var half_width := lerpf(size.x, size.z, float(plane) / 2.0)
		var bottom := centre - axis * size.y * (0.50 if conifer else 0.72)
		var top := centre + axis * size.y
		var face := color.lerp(color * 1.35, _noise(seed + plane, 271) * 0.60)
		surface.set_normal(normal)
		for vertex in [0, 2, 1, 0, 3, 2]:
			var local_uv: Vector2 = local_uvs[vertex]
			var position := top.lerp(bottom, local_uv.y) + radial * half_width * (2.0 * local_uv.x - 1.0)
			# Ground shoots bend from their base.
			position.y = maxf(position.y, 0.0)
			face.a = clampf(position.y * sway_per_m, 0.0, 0.8)
			surface.set_color(face)
			surface.set_uv(uv_origin + local_uv * 0.5)
			surface.add_vertex(position)

## Four low ground-layer forms, spreading juniper and an airy spruce sapling.
## Fixed loops emit at most 64 triangles per variant, once at catalogue construction.
static func _bush(variant: int) -> ArrayMesh:
	if variant == 2:
		return _bush_grass(variant)
	if variant == 3:
		return _bush_fern(variant)
	var cards := SurfaceTool.new()
	cards.begin(Mesh.PRIMITIVE_TRIANGLES)
	var stems := SurfaceTool.new()
	stems.begin(Mesh.PRIMITIVE_TRIANGLES)
	var phase := _noise(variant, 211) * TAU
	var mesh: ArrayMesh
	match variant:
		0, 1:
			# Blueberry and lingonberry: scattered overlapping lobes, no trunk or apex.
			# Oblique shoots expose leaf surfaces from above as well as at eye level.
			var count := 6 + variant
			var color := Color(0.246, 0.294, 0.090) if variant == 0 else Color(0.26, 0.30, 0.105)
			for shoot in range(count):
				var seed := variant * 149 + shoot * 11
				var angle := phase + float(shoot) * 2.39996323
				var radial := Vector3(cos(angle), 0, sin(angle))
				var reach := sqrt(float(shoot) / float(count)) * (0.42 if variant == 0 else 0.36)
				var at := radial * reach + Vector3.UP * lerpf(0.06, 0.09, _noise(seed, 227))
				var size := Vector3(0.17, lerpf(0.08, 0.12, _noise(seed, 229)), 0.14)
				_foliage_cards(cards, at, Vector3.DOWN * 0.15, radial, size, false, seed,
					color, radial * 0.8 + Vector3.UP, 0.16)
		4:
			# Prostrate juniper: uneven, outward-growing sprays through the whole volume.
			for shoot in range(7):
				var seed := variant * 149 + shoot * 11
				var angle := phase + float(shoot) * 2.39996323
				var radial := Vector3(cos(angle), 0, sin(angle))
				var at := radial * lerpf(0.08, 0.42, _noise(seed, 227))
				at.y = lerpf(0.15, 0.36, _noise(seed, 229))
				var axis := radial * 0.9 + Vector3.UP * lerpf(0.35, 1.2, _noise(seed, 233))
				_bush_stem(stems, radial * 0.04, at + axis.normalized() * 0.12,
					Color(0.32, 0.23, 0.15), 0.24)
				_foliage_cards(cards, at, Vector3.ZERO, radial, Vector3(0.20, 0.38, 0.15),
					true, seed, Color(0.199, 0.261, 0.155), axis, 0.24)
		5:
			# A 24 mm stem and separated branch whorls, never a crown shell.
			# Exact 8-bit endpoints (0 and 102/255); cards use the same 0.25/m gradient.
			_lathe(stems, PackedVector2Array([Vector2(0, 0.012), Vector2(1.6, 0.003)]),
				5, Color(0.30, 0.20, 0.12), 0.0, variant, 0.0, PackedFloat32Array([0.0, 0.4]))
			for whorl in range(3):
				for branch in range(2):
					var seed := variant * 149 + whorl * 23 + branch * 11
					var angle := phase + float(whorl) * 2.39996323 + float(branch) * PI
					var radial := Vector3(cos(angle), 0, sin(angle))
					var length := (0.36 - float(whorl) * 0.08) * lerpf(0.9, 1.1, _noise(seed, 227))
					var at := radial * length * 0.48 + Vector3.UP * (0.30 + float(whorl) * 0.40)
					_bush_stem(stems, Vector3.UP * at.y, at + radial * length * 0.3,
						Color(0.30, 0.20, 0.12), 0.25)
					_foliage_cards(cards, at, Vector3.UP * at.y - Vector3.UP * 0.1, radial,
						Vector3(0.12, length, 0.09), true, seed,
						Color(0.186, 0.237, 0.085), radial + Vector3.UP * 0.25, 0.25)
			_foliage_cards(cards, Vector3.UP * 1.40, Vector3.UP, Vector3.RIGHT,
				Vector3(0.075, 0.20, 0.06), true, variant,
				Color(0.18, 0.29, 0.11), Vector3.UP, 0.25)
	# Card-only plants need one surface. Append to the same mesh when a stem is present.
	if variant >= 4:
		mesh = _finish(stems, _wind_branch_material())
	cards.set_material(_foliage_material())
	cards.index()
	return cards.commit(mesh)

## Two-sided tapered stem ribbon, narrow enough to remain support rather than silhouette.
static func _bush_stem(surface: SurfaceTool, start: Vector3, end: Vector3,
	color: Color, sway_per_m: float) -> void:
	surface.set_smooth_group(-1)
	var across := (end - start).cross(Vector3.UP).normalized() * 0.006
	var weights := Vector3(start.y, start.y, end.y) * sway_per_m
	_weighted_tri(surface, color, start - across, start + across, end, weights)
	_weighted_tri(surface, color, start + across, start - across, end, weights)

## Eight bent, tapered blades with open gaps; reversed faces share the opaque material.
static func _bush_grass(variant: int) -> ArrayMesh:
	var surface := SurfaceTool.new()
	surface.begin(Mesh.PRIMITIVE_TRIANGLES)
	surface.set_smooth_group(-1)
	for blade in range(8):
		var seed := variant * 149 + blade * 11
		var angle := _noise(variant, 211) * TAU + float(blade) * 2.39996323
		var radial := Vector3(cos(angle), 0, sin(angle))
		var across := radial.cross(Vector3.UP) * lerpf(0.012, 0.022, _noise(seed, 233))
		var height := lerpf(0.34, 0.65, _noise(seed, 227))
		var root := radial * 0.035
		var bend := radial * 0.16 + Vector3.UP * height * 0.78
		var tip := radial * lerpf(0.24, 0.40, _noise(seed, 229)) + Vector3.UP * height
		var color := Color(0.292, 0.328, 0.116).lerp(Color(0.46, 0.40, 0.22), _noise(seed, 239))
		var vertices := [root - across, root + across, bend - across * 0.55, bend + across * 0.55, tip]
		for triangle in [Vector3i(0, 2, 3), Vector3i(0, 3, 1), Vector3i(2, 4, 3)]:
			var a: Vector3 = vertices[triangle.x]
			var b: Vector3 = vertices[triangle.y]
			var c: Vector3 = vertices[triangle.z]
			_weighted_tri(surface, color, a, b, c, Vector3(a.y, b.y, c.y) * 0.2)
			_weighted_tri(surface, color, a, c, b, Vector3(a.y, c.y, b.y) * 0.2)
	return _finish(surface, _wind_branch_material())

## Three arching fronds with six narrow leaflet pairs each, within 48 triangles.
static func _bush_fern(variant: int) -> ArrayMesh:
	var surface := SurfaceTool.new()
	surface.begin(Mesh.PRIMITIVE_TRIANGLES)
	surface.set_smooth_group(-1)
	var leaves := SurfaceTool.new()
	leaves.begin(Mesh.PRIMITIVE_TRIANGLES)
	# A solid texel in the atlas's central conifer twig lets the existing two-sided
	# material render geometric leaflets without doubling triangles for their backs.
	leaves.set_uv(Vector2(0.25, 0.78125))
	for frond in range(3):
		var seed := variant * 149 + frond * 11
		var angle := _noise(variant, 211) * TAU + float(frond) * TAU / 3.0 + (_noise(seed, 227) - 0.5) * 0.4
		var radial := Vector3(cos(angle), 0, sin(angle))
		var across := radial.cross(Vector3.UP)
		var height := lerpf(0.45, 0.60, _noise(seed, 229))
		var bend := radial * 0.40 + Vector3.UP * height
		var tip := radial * 0.68 + Vector3.UP * height * 0.72
		_bush_stem(surface, Vector3.ZERO, bend, Color(0.30, 0.27, 0.12), 0.20)
		_bush_stem(surface, bend, tip, Color(0.30, 0.27, 0.12), 0.20)
		leaves.set_normal((radial * 0.3 + Vector3.UP).normalized())
		for pair in range(6):
			var t := float(pair + 1) / 6.5
			var at := bend * (t * 1.5) if t <= 2.0 / 3.0 else bend.lerp(tip, (t - 2.0 / 3.0) * 3.0)
			var along := bend.normalized() if t <= 2.0 / 3.0 else (tip - bend).normalized()
			var width := lerpf(0.19, 0.07, t)
			var color := Color(0.291, 0.339, 0.099) * lerpf(0.94, 1.12, _noise(seed, 239 + pair))
			for side in [-1.0, 1.0]:
				# Leaflets share the bent rachis and spread in its plane, including overhead.
				var a := at - along * 0.012
				var b: Vector3 = at + across * width * side + along * 0.08 - Vector3.UP * 0.025
				var c := at + along * 0.035
				_weighted_tri(leaves, color, a, b, c, Vector3(a.y, b.y, c.y) * 0.20)
	var mesh := _finish(surface, _wind_branch_material())
	leaves.set_material(_foliage_material())
	leaves.index()
	return leaves.commit(mesh)

## One granite boulder. The shared stone tile carries the surface; vertex colour carries the
## lichen, the moss at the foot and the occlusion under it. Startup only.
static func _rock(variant: int) -> ArrayMesh:
	if _rock_material == null:
		_rock_material = StandardMaterial3D.new()
		_rock_material.vertex_color_use_as_albedo = true
		_rock_material.albedo_texture = load(ROCK_MODEL_DIR + "rock_albedo.png")
		_rock_material.normal_enabled = true
		_rock_material.normal_texture = load(ROCK_MODEL_DIR + "rock_normal.png")
		_rock_material.roughness = 0.9
	var scene: Node = (load(ROCK_MODEL_DIR + "rock_%d.glb" % variant) as PackedScene).instantiate()
	var source: Mesh = (scene.find_children("*", "MeshInstance3D", true, false)[0] as MeshInstance3D).mesh
	scene.free()
	# The normal map needs tangents, which the exported model does not carry.
	var surface := SurfaceTool.new()
	surface.create_from(source, 0)
	surface.generate_tangents()
	surface.set_material(_rock_material)
	return surface.commit()
