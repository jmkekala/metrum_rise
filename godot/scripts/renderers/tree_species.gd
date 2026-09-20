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
const VARIANT_COUNTS := [12, 12, 6, 6]

# Bark palettes shared by a near trunk and the branches that grow out of it.
const BIRCH_PALE := Color(0.80, 0.78, 0.73)
const ASPEN_PALE := Color(0.335, 0.330, 0.260)
const PINE_LOW_BARK := Color(0.150, 0.115, 0.085)
const PINE_CROWN_BARK := Color(0.360, 0.185, 0.080)

# Hash coverage per species, conifer first, calibrated so the distant crown covers as much
# ground as the measured near crown. The broadleaf figure is the higher of the two because a
# near broadleaf carries cards that stand out of the crown in depth as well as across it, and
# a surface of revolution has no depth to spare: its projected extent is already its width.
# See docs/terrain.md for the instrument.
const DISTANT_COVERAGE := [0.50, 0.82]
# Share of its own albedo a distant crown keeps, per species, so that the two levels render to one
# luminance. Fitted, not derived: the tone map makes rendered luminance a sublinear function of
# albedo, so the value is read off a sweep rather than taken as the luminance ratio itself. The
# calibration follows the near crown's volume normals, including preserving their direction
# on the backs of foliage cards. See `distant_radiance_match` in vegetation_distant.gdshader.
const DISTANT_RADIANCE_MATCH := [0.72, 0.90]

static var _material: StandardMaterial3D
static var _wind_material: ShaderMaterial
static var _card_material: ShaderMaterial
static var _card_texture: Texture2D
static var _distant_materials: Array[ShaderMaterial] = [null, null]

## Meshes indexed by species, variant, then level (near to far).
static func build_meshes() -> Array:
	var meshes: Array = []
	meshes.resize(SPECIES_COUNT)
	for species in range(SPECIES_COUNT):
		var variants: Array = []
		variants.resize(VARIANT_COUNTS[species])
		for variant in range(variants.size()):
			match species:
				CONIFER, BROADLEAF:
					variants[variant] = [_branched_tree(species, variant)]
				BUSH:
					variants[variant] = [_bush(variant)]
				ROCK:
					variants[variant] = [_rock(variant)]
		if species < BUSH:
			# The scatter uses variant zero at distance for the entire species. Integrate
			# all near variants, including the birch mix, once at startup: O(triangles).
			var integral := Vector4.ZERO
			var envelope := Vector3.ZERO
			for levels in variants:
				integral += _crown_integral(levels[0])
				envelope += _crown_envelope(levels[0])
			var crown := Color(integral.x, integral.y, integral.z) / integral.w
			crown.a = 1.0
			envelope /= float(variants.size())
			for variant in range(variants.size()):
				for lod in [1, 2]:
					variants[variant].append(_conifer(lod, variant, crown, envelope)
						if species == CONIFER else _broadleaf(lod, variant, crown, envelope))
		meshes[species] = variants
	return meshes

# Two of each three broadleaf variants are birch: 0, 1, 3, 4, 6, 7, 9, 10.
static func _is_birch(variant: int) -> bool:
	return variant % 3 != 2

# Two of each three conifer variants are pine, which matches the order of Finnish growing
# stock: pine leads, spruce follows. The remaining third are spruce. Only the near level
# pays for variants, so this split reads out to TREE_NEAR_M and no further; see RENDER-02.
static func _is_pine(variant: int) -> bool:
	return variant % 3 != 2

# Foliage hue follows the same measurement that corrected the terrain palette: sunlit
# vegetation in the reference photographs sits at hue 59-79 degrees, and every colour here
# sat at 90-112, which is blue-green. Each is rotated into that band keeping its original
# luminance and saturation, so the trees no longer read bluer than the ground they stand on.
# See the palette section in docs/terrain.md.
static func _leaf_color(conifer: bool, birch: bool) -> Color:
	if birch:
		return Color(0.205, 0.253, 0.062)
	return Color(0.080, 0.106, 0.036) if conifer else Color(0.135, 0.161, 0.051)

# Sum area * mean(vertex RGB), plus area in w. Read the actual quantized mesh colours.
# Include the whole opaque core surface (wood and foliage) plus the cards, as emitted.
# Cards use geometric area, not alpha coverage; this is albedo, not a lighting estimate.
static func _crown_integral(mesh: ArrayMesh) -> Vector4:
	var result := Vector4.ZERO
	for surface in range(mesh.get_surface_count()):
		var arrays := mesh.surface_get_arrays(surface)
		var vertices: PackedVector3Array = arrays[Mesh.ARRAY_VERTEX]
		var colors: PackedColorArray = arrays[Mesh.ARRAY_COLOR]
		var indices: PackedInt32Array = arrays[Mesh.ARRAY_INDEX]
		for triangle in range(0, indices.size(), 3):
			var a := indices[triangle]
			var b := indices[triangle + 1]
			var c := indices[triangle + 2]
			var color := (colors[a] + colors[b] + colors[c]) / 3.0
			var area := (vertices[b] - vertices[a]).cross(vertices[c] - vertices[a]).length() * 0.5
			result += Vector4(color.r, color.g, color.b, 1.0) * area
	return result

## Mean foliage extent of one near crown as (radius_m, top_m, base_m), from the trunk axis.
##
## The distant lathe is sized from this rather than from the branch base width. `_branched_tree`
## treats its `width` as a base and reaches `width * _crown_reach() * [0.90, 1.08]`, then hangs
## foliage cards past each branch tip, so the near silhouette ends roughly twice as far out as
## that base. Building the distant crown at the base radius shrank the conifer's ground
## footprint to 34% of the near crown's across `TREE_NEAR_M`, which is bright ground opening up
## under a stand as the camera pulls back. Measuring the near mesh keeps the two in step when
## the near crown changes, the same reason the distant colour is integrated rather than authored.
static func _crown_envelope(mesh: ArrayMesh) -> Vector3:
	var envelope := Vector3(0.0, 0.0, INF)
	for surface in range(mesh.get_surface_count()):
		# Foliage only. The trunk runs the full height and would set the extent from bare wood.
		if mesh.surface_get_material(surface) != _foliage_material():
			continue
		var vertices: PackedVector3Array = mesh.surface_get_arrays(surface)[Mesh.ARRAY_VERTEX]
		for vertex in vertices:
			# Axis-aligned, not radial. A lathe ring puts vertices on both axes, so its radius
			# becomes the half width of the bounding box; the radial reach of a near crown is
			# up to sqrt(2) larger than that half width and would oversize the hull to match.
			envelope = Vector3(maxf(envelope.x, maxf(absf(vertex.x), absf(vertex.z))),
				maxf(envelope.y, vertex.y), minf(envelope.z, vertex.y))
	return envelope

# Distant crowns and low vegetation use ragged radius profiles; near trees distribute
# foliage along a bounded, two-level branch skeleton.

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
	weights: PackedFloat32Array = PackedFloat32Array(),
	ring_colors: PackedColorArray = PackedColorArray()
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
		if not ring_colors.is_empty():
			cap_color = ring_colors[end_ring]
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
				if not ring_colors.is_empty():
					face = ring_colors[ring]
					# Break thin horizontal lenticels into short sector-length dashes.
					if ring > 0 and face.r < 0.2 and _noise(ring + seed_offset, segment + 353) < 0.35:
						face = color
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

## Trunk profile with a root flare. A straight cylinder meeting flat ground reads as a
## toothpick pushed into a surface; real trunks widen into the ground over the last metre.
static func _trunk_profile(height_m: float, radius_m: float) -> PackedVector2Array:
	return PackedVector2Array([
		Vector2(-0.4, radius_m * 2.6),
		Vector2(0.35, radius_m * 1.45),
		Vector2(1.2, radius_m * 1.05),
		Vector2(height_m, radius_m * 0.72),
	])

static func _finish(surface: SurfaceTool, material: Material = null,
	crown_centre: Vector3 = Vector3.ZERO) -> ArrayMesh:
	surface.generate_normals()
	if _material == null:
		_material = StandardMaterial3D.new()
		_material.vertex_color_use_as_albedo = true
		_material.roughness = 1.0
	surface.set_material(_material if material == null else material)
	# Primitive expansion duplicates shared vertices. Restore indexing for vertex-cache reuse.
	surface.index()
	if crown_centre != Vector3.ZERO:
		# Generate the wood's geometric normals first, then shade foliage as one crown.
		# The green discriminator is shared with vegetation_backlight; bark keeps its
		# original normals. This O(vertices) pass runs only during catalogue construction.
		var arrays := surface.commit_to_arrays()
		var vertices: PackedVector3Array = arrays[Mesh.ARRAY_VERTEX]
		var colors: PackedColorArray = arrays[Mesh.ARRAY_COLOR]
		var normals: PackedVector3Array = arrays[Mesh.ARRAY_NORMAL]
		for i in range(vertices.size()):
			if colors[i].g > colors[i].r:
				normals[i] = (vertices[i] - crown_centre).normalized()
		arrays[Mesh.ARRAY_NORMAL] = normals
		var mesh := ArrayMesh.new()
		mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
		mesh.surface_set_material(0, material)
		return mesh
	return surface.commit()

static func _wind_branch_material() -> ShaderMaterial:
	if _wind_material == null:
		_wind_material = ShaderMaterial.new()
		_wind_material.shader = preload("res://scripts/shaders/vegetation_wind.gdshader")
		_apply_canopy_shading(_wind_material)
		_apply_wind_visibility(_wind_material)
	return _wind_material

## Ties the canopy shading ramp to the shadow cascades. The term replaces the darkening the
## cascades stop supplying, so it has to begin where they begin to fade and reach full strength
## where they end. Reading the range through the accessor keeps the probe override in step.
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
	var far_m := SceneLightingConfig.shadow_max_distance_m()
	material.set_shader_parameter(
		"canopy_shade_begin_m", far_m * SceneLightingConfig.SHADOW_FADE_START
	)
	material.set_shader_parameter("canopy_shade_end_m", far_m)
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

## Fixed branch counts and depth bound startup work to O(emitted vertices) per variant.
static func _branched_tree(species: int, variant: int) -> ArrayMesh:
	var conifer := species == CONIFER
	var pine := conifer and _is_pine(variant)
	var birch := not conifer and _is_birch(variant)
	var aspen := not conifer and not birch
	# Pine and spruce keep one height range. In the field a pine does overtop the spruce
	# around it, but the appearance test holds every variant of a species inside 1.3x of
	# its siblings, and the silhouette carries the species far better than the height does.
	var height := lerpf(14.5, 17.1, _noise(variant, 103)) + lerpf(1.9, 2.5, _noise(variant, 109)) if conifer else (
		lerpf(10.8, 12.8, _noise(variant, 151)) + lerpf(2.9, 3.5, _noise(variant, 163)))
	var width := lerpf(2.65, 3.25, _noise(variant, 107)) if conifer else lerpf(3.15, 3.85, _noise(variant, 157))
	var bark := Color(0.115, 0.085, 0.062) if conifer else Color(0.185, 0.170, 0.150)
	if pine:
		# A pine crown is broad against its own depth, not against the whole tree: it holds
		# its foliage in the top third. Reaching wider than a spruce as well made a canopy
		# of umbrellas, so the two species span about the same width and differ in where
		# that width sits.
		width *= 0.92
		bark = PINE_CROWN_BARK
	if birch:
		width *= 0.78
		bark = BIRCH_PALE
	if aspen:
		width *= 0.86
		bark = ASPEN_PALE
	var radius := lerpf(0.26, 0.34, _noise(variant, 139)) if conifer else lerpf(0.21, 0.27, _noise(variant, 181))
	var surface := SurfaceTool.new()
	surface.begin(Mesh.PRIMITIVE_TRIANGLES)
	var cards := SurfaceTool.new()
	cards.begin(Mesh.PRIMITIVE_TRIANGLES)
	# Cards face out of the crown they sit in, and a pine crown sits far higher than the rest.
	var crown_centre := Vector3.UP * height * (0.80 if pine else (0.56 if conifer else 0.66))
	if birch or aspen:
		_banded_trunk(surface, height, radius, 5, variant, true, birch)
	elif pine:
		_pine_trunk(surface, height, radius, 5, variant)
	else:
		# Retain the grounded flare and carry a tapering leader through the junctions.
		var trunk := _trunk_profile(height * 0.55, radius)
		trunk.append(Vector2(height, 0.0))
		# A linear weight above the flare keeps branch roots on the displaced trunk.
		var trunk_weights := PackedFloat32Array()
		for ring in trunk:
			trunk_weights.append(clampf((ring.x - 0.35) / (height - 0.35), 0.0, 1.0) * 0.16)
		_lathe(surface, trunk, 5, bark, 0.08, 11 + variant * 61, 0.0, trunk_weights)
	# A spruce needs enough boughs to close into a mass rather than to read as a stick with
	# tufts on it; a pine packs its into the top third and needs fewer.
	var count := 14 if pine else (20 if conifer else 12)
	# A spruce keeps live boughs low but not on the ground; below about an eighth of the
	# trunk the drooping tips and their foliage cards cross under the terrain.
	var lowest := 0.60 if pine else (0.13 if conifer else (0.42 if aspen else 0.27))
	var phase := _noise(variant, 233) * TAU
	for branch in range(count):
		var t := float(branch) / float(count - 1)
		var seed := variant * 101 + branch * 7 + species * 1543
		# A spiral avoids identical horizontal whorls and leaves windows onto the wood.
		var angle := phase + float(branch) * 2.39996323 + (_noise(seed, 239) - 0.5) * 0.35
		var start := Vector3(0.0, height * lerpf(lowest, 0.96 if conifer else 0.82, t), 0.0)
		var reach := width * _crown_reach(t, pine, conifer)
		reach *= lerpf(0.90, 1.08, _noise(seed, 241))
		var rise := height * lerpf(0.24, 0.16, t)
		if pine:
			rise = reach * lerpf(0.10, 0.48, t)
		elif conifer:
			# A mature spruce carries its lower boughs well below horizontal and lifts the
			# upper ones; one droop figure for the whole trunk flattens that away.
			rise = reach * lerpf(-0.26, 0.55, t)
		var offset := Vector3(cos(angle) * reach, rise, sin(angle) * reach)
		_branch(surface, cards, crown_centre, start, offset, radius * lerpf(0.58, 0.19, t),
			conifer, 0, seed, bark, (start.y - 0.35) / (height - 0.35) * 0.16, birch, pine)
	var mesh := _finish(surface, _wind_branch_material(), crown_centre)
	# Append to the same mesh: another draw, but no additional instances or uploads.
	cards.set_material(_foliage_material())
	cards.index()
	cards.commit(mesh)
	return mesh

## Horizontal branch reach as a fraction of the crown width, from the base of the crown
## (t = 0) to its top (t = 1). These three curves are the species silhouettes: a pine holds
## a broad, blunt crown over a bare stem, a spruce tapers from a wide skirt to a point, and
## a broadleaf is widest across its middle.
static func _crown_reach(t: float, pine: bool, conifer: bool) -> float:
	if pine:
		return sin(lerpf(0.30, 0.92, t) * PI)
	if conifer:
		return 0.13 + 0.87 * pow(1.0 - t, 0.85)
	return sin(lerpf(0.23, 0.82, t) * PI) * 0.78

## Pine bark is the clearest signal the species has: grey-brown and fissured at the foot,
## turning orange-red over the upper stem and through the crown. Ring colours put the
## gradient on the existing lathe, so it costs no second surface, material or draw call.
static func _pine_trunk(surface: SurfaceTool, height: float, radius: float,
	segments: int, variant: int) -> void:
	var mid := Color(0.265, 0.150, 0.080)
	var high := Color(0.450, 0.185, 0.065)
	# The colour turns at the crown base, which is where the plates thin in the field.
	var turn := lerpf(0.46, 0.58, _noise(variant, 389))
	var profile := PackedVector2Array([
		Vector2(-0.4, radius * 2.6),
		Vector2(0.35, radius * 1.45),
		Vector2(1.2, radius * 1.05),
		Vector2(height * turn, radius * 0.80),
		Vector2(height * lerpf(0.72, 0.80, _noise(variant, 397)), radius * 0.58),
		Vector2(height, 0.0),
	])
	var colors := PackedColorArray([PINE_LOW_BARK, PINE_LOW_BARK, mid, high, high, high])
	var weights := PackedFloat32Array()
	for ring in profile:
		weights.append(clampf((ring.x - 0.35) / (height - 0.35), 0.0, 1.0) * 0.16)
	# The lathe breaks any ring darker than 0.2 red into dashes, which is the birch lenticel
	# rule. Ring zero is exempt, the fallback colour here equals the only other ring below
	# that threshold, and every ring above the turn is far too red for the rule to fire.
	_lathe(surface, profile, segments, PINE_LOW_BARK, 0.08, 11 + variant * 61, 0.0, weights, colors)

## Banded pale trunk for birch and aspen. Near-only extra rings define the marks without
## another material or texture. The distant trunk retains the original four-ring budget:
## scar, short band, pale shaft.
static func _banded_trunk(surface: SurfaceTool, height: float, radius: float,
	segments: int, variant: int, near: bool, birch: bool) -> void:
	var pale := BIRCH_PALE if birch else ASPEN_PALE
	var dark := Color(0.075, 0.068, 0.060) if birch else Color(0.062, 0.058, 0.048)
	# Rough bark at the foot. Ring zero is the one ring the lathe never dashes, so the
	# darkest scar survives there, and the rings above it stay above the 0.2 red that the
	# dashing rule tests, so the foot reads as solid bark rather than as broken blotches.
	var scar := Color(0.16, 0.14, 0.12)
	var rough := Color(0.215, 0.185, 0.150) if birch else Color(0.205, 0.195, 0.150)
	var profile := PackedVector2Array([Vector2(-0.4, radius * 2.6), Vector2(0.35, radius * 1.45)])
	var colors := PackedColorArray([scar, rough])
	if near:
		# A mature birch is dark and fissured for the first metre or two. The trunks had
		# none of that: the only dark ring ended at 0.35 m, and the ground covered most of
		# it. Aspen keeps a shorter foot under a grey-green shaft.
		var foot := lerpf(1.15, 1.75, _noise(variant, 359)) if birch else lerpf(0.70, 1.05, _noise(variant, 359))
		profile.append(Vector2(foot * 0.6, radius * 1.20))
		colors.append(rough)
		profile.append(Vector2(foot, radius * 1.08))
		colors.append(pale)
		var count := 10 if birch else 5
		for band in range(count):
			# The marks crowd toward the foot and thin as they rise, rather than repeating
			# on one spacing up the whole trunk. Even spacing is what made them read as
			# machined rings instead of as bark.
			var f := pow(float(band) / float(count - 1), 1.55)
			var y := lerpf(foot + 0.30, height * 0.86, f) + (_noise(variant, 367 + band) - 0.5) * 0.26
			var r := radius * lerpf(1.05, 0.34, y / height)
			var thickness := lerpf(0.22, 0.055, f) * lerpf(0.80, 1.25, _noise(variant, 379 + band))
			profile.append(Vector2(y, r))
			colors.append(dark)
			profile.append(Vector2(y + thickness, r))
			colors.append(pale)
		profile.append(Vector2(height, 0.0))
		colors.append(pale)
	else:
		colors[1] = dark
		profile.append(Vector2(0.43, radius * 1.05))
		profile.append(Vector2(height, radius * 0.72))
		colors.append(pale)
		colors.append(pale)
	var weights := PackedFloat32Array()
	if near:
		for ring in profile:
			weights.append(clampf((ring.x - 0.35) / (height - 0.35), 0.0, 1.0) * 0.16)
	_lathe(surface, profile, segments, pale, 0.08, 23 + variant * 61, 0.0, weights, colors)

## A quadratic centreline gives drooping spruce boughs and upward-curving broadleaf limbs.
static func _branch_point(start: Vector3, offset: Vector3, bend: float, t: float) -> Vector3:
	return start + offset * t + Vector3.UP * (4.0 * bend * t * (1.0 - t))

## Emit a primary and two smaller children, then stop; no catalogue-wide mutable state.
static func _branch(surface: SurfaceTool, cards: SurfaceTool, crown_centre: Vector3,
	start: Vector3, offset: Vector3, radius: float,
	conifer: bool, depth: int, seed: int, bark: Color, root_weight: float, birch: bool,
	pine: bool = false) -> void:
	# A spruce bough hangs and a pine limb lifts. One sign carries most of that difference.
	var bend := offset.length() * (0.16 if birch and depth == 1 else (
		0.12 if pine else (-0.14 if conifer else -0.10)))
	var middle := _branch_point(start, offset, bend, 0.5)
	var tip_weight := 0.70 if depth == 0 else 1.0
	_branch_tube(surface, start, middle, start + offset, radius, bark, root_weight, tip_weight)
	if depth == 0:
		for child in range(2):
			var t := 0.60 + float(child) * 0.23 + _noise(seed + child, 347) * 0.06
			# Attach to the emitted two-span centreline, not the unsampled curve.
			var origin := start.lerp(middle, t * 2.0) if t <= 0.5 else middle.lerp(start + offset, (t - 0.5) * 2.0)
			var angle := atan2(offset.z, offset.x) + (-0.72 if child == 0 else 0.72)
			var reach := Vector2(offset.x, offset.z).length() * lerpf(0.38, 0.52, _noise(seed + child, 251))
			var rise := reach * (-0.55 if birch else (0.12 if conifer else 0.95))
			_branch(surface, cards, crown_centre, origin, Vector3(cos(angle) * reach, rise, sin(angle) * reach),
				radius * (0.24 if birch else 0.32), conifer, 1, seed + child + 1,
				Color(0.22, 0.17, 0.12) if birch else bark, lerpf(root_weight, tip_weight, t), birch, pine)
	else:
		# Small solid cores anchor the cut-out clusters at the child tips.
		var size := offset.length()
		var length := maxf(size * 1.60, 0.45) if conifer else lerpf(1.15, 1.50, _noise(seed, 257))
		var width := length * (0.85 if conifer else 0.86)
		var height := maxf(size * 1.65, 0.85) if conifer else lerpf(1.65, 2.15, _noise(seed, 263))
		if pine:
			# Pine needles sit in shallow brushes at the ends of the limbs, not in the deep
			# sprays a spruce carries along the whole bough. The bound matters: a pine limb
			# is long, so a brush that scales freely with it doubles the crown width.
			length = clampf(size * 1.05, 0.60, 1.45)
			width = length * 1.05
			height = clampf(size * 0.62, 0.45, 0.95)
		var centre := start + offset + Vector3.UP * height * 0.18
		var dimensions := Vector3(length, height, width)
		if birch:
			dimensions *= Vector3(0.86, 1.15, 0.86)
		_foliage_tuft(surface, centre, offset, dimensions * 0.45, conifer, seed, birch)
		_foliage_cards(cards, centre, crown_centre, offset,
			dimensions * (1.10 if conifer else 1.25), conifer, seed, 1.0, birch)

## Four-sided, two-span taper; roots intersect their parent and terminal rings close to a tip.
static func _branch_tube(surface: SurfaceTool, start: Vector3, middle: Vector3,
	end: Vector3, radius: float, color: Color, root_weight: float, tip_weight: float) -> void:
	var middle_weight := lerpf(root_weight, tip_weight, 0.5)
	var axis := (end - start).normalized()
	var right := axis.cross(Vector3.UP).normalized()
	var forward := right.cross(axis)
	surface.set_smooth_group(-1)
	for side in range(4):
		var a := TAU * float(side) / 4.0
		var b := TAU * float(side + 1) / 4.0
		var radial_a := right * cos(a) + forward * sin(a)
		var radial_b := right * cos(b) + forward * sin(b)
		var lower_a := start + radial_a * radius
		var lower_b := start + radial_b * radius
		var upper_a := middle + radial_a * radius * 0.56
		var upper_b := middle + radial_b * radius * 0.56
		# Same outward pre-reversal order as the lathe, in the branch's local frame.
		_weighted_tri(surface, color, lower_a, upper_a, upper_b,
			Vector3(root_weight, middle_weight, middle_weight))
		_weighted_tri(surface, color, lower_a, upper_b, lower_b,
			Vector3(root_weight, middle_weight, root_weight))
		_weighted_tri(surface, color, upper_a, end, upper_b,
			Vector3(middle_weight, tip_weight, middle_weight))

## Six-face solid cores retain volume between the crossed cut-outs.
static func _foliage_tuft(surface: SurfaceTool, centre: Vector3, direction: Vector3,
	size: Vector3, conifer: bool, seed: int, birch: bool = false) -> void:
	var along := Vector3(direction.x, 0.0, direction.z).normalized()
	var across := along.cross(Vector3.UP)
	var top := centre + Vector3.UP * size.y
	var bottom := centre - Vector3.UP * size.y * (0.38 if conifer else 0.72)
	var color := _leaf_color(conifer, birch)
	var sides := 3
	for side in range(sides):
		var a := TAU * float(side) / float(sides)
		var b := TAU * float((side + 1) % sides) / float(sides)
		var radial_a := (along * cos(a) * size.x + across * sin(a) * size.z) * lerpf(0.86, 1.12, _noise(seed + side, 269))
		var radial_b := (along * cos(b) * size.x + across * sin(b) * size.z) * lerpf(0.86, 1.12, _noise(seed + (side + 1) % sides, 269))
		# Keep the old top/bottom mean albedo without independently bright triangles.
		# Crown normals now supply the light gradient across both cores and cards.
		var face := color * 1.04975
		face.a = 1.0
		_tri(surface, face, centre + radial_a, top, centre + radial_b)
		_tri(surface, face, centre + radial_a, centre + radial_b, bottom)

## Three crossed quads share a tilted axis, with crown-outward lighting instead of plate normals.
## Ground plants can override the palette, growth axis and height-based sway gradient.
static func _foliage_cards(surface: SurfaceTool, centre: Vector3, crown_centre: Vector3,
	direction: Vector3, size: Vector3, conifer: bool, seed: int, weight: float = 1.0, birch: bool = false,
	leaf_color: Color = Color(-1, -1, -1), growth_axis: Vector3 = Vector3.ZERO,
	sway_per_m: float = -1.0) -> void:
	var along := Vector3(direction.x, 0.0, direction.z).normalized()
	var axis := (Vector3.UP + along * 0.35).normalized() if growth_axis == Vector3.ZERO else growth_axis.normalized()
	var right := along.cross(axis).normalized()
	var normal := (centre - crown_centre).normalized()
	var color := _leaf_color(conifer, birch) if leaf_color.r < 0.0 else leaf_color
	var tree_foliage := leaf_color.r < 0.0
	var phase := _noise(seed, 307) * PI
	# Top left generic broadleaf, top right birch; bottom row seeded conifer sprays.
	var uv_origin := Vector2(float(seed & 1) if conifer else (1.0 if birch else 0.0),
		1.0 if conifer else 0.0) * 0.5
	var uvs := [uv_origin + Vector2(0.0, 0.5), uv_origin + Vector2(0.5, 0.5),
		uv_origin + Vector2(0.5, 0.0), uv_origin]
	for plane in range(3):
		var radial := right.rotated(axis, phase + float(plane) * PI / 3.0)
		var half_width := lerpf(size.x, size.z, float(plane) / 2.0)
		var bottom := centre - axis * size.y * (0.50 if conifer else 0.72)
		var top := centre + axis * size.y
		var corners := [bottom - radial * half_width, bottom + radial * half_width,
			top + radial * half_width, top - radial * half_width]
		# Preserve the former mean albedo, but stop crossed tree cards from looking like
		# separate bright scraps. Ground plants retain their own colour and normal treatment.
		var face := color * 1.105 if tree_foliage else color.lerp(color * 1.35, _noise(seed + plane, 271) * 0.60)
		# Reset alpha after RGB shading; a card must sway with the core it sits on.
		face.a = weight
		surface.set_color(face)
		surface.set_normal(normal)
		for vertex in [0, 2, 1, 0, 3, 2]:
			var position: Vector3 = corners[vertex]
			if tree_foliage:
				surface.set_normal((position - crown_centre).normalized())
			if sway_per_m >= 0.0:
				# Ground shoots bend from their base; tree tufts retain their rigid weight.
				position.y = maxf(position.y, 0.0)
				face.a = clampf(position.y * sway_per_m, 0.0, 0.8)
				surface.set_color(face)
			surface.set_uv(uvs[vertex])
			surface.add_vertex(position)

## Spruce with swept near branches and the continuous ragged crown at distance.
static func _conifer(lod: int, variant: int, crown: Color, envelope: Vector3) -> ArrayMesh:
	var segments: int = [9, 6, 4][lod]
	var rings: int = [9, 6, 3][lod]
	var ragged: float = [0.15, 0.11, 0.0][lod] * lerpf(0.8, 1.2, _noise(variant, 101))
	# Sized to the near crown's own foliage extent, with no per-variant spread: the scatter draws
	# variant zero's distant mesh for the whole species, so a spread produces no variety and only
	# moves this one crown off the envelope it is meant to match. The base matters as much as the
	# top: an authored base hung the distant crown 3.5 m below where a pine actually carries its
	# foliage, which measured as 130% of the near crown's footprint once the width was right.
	var crown_base := envelope.z
	var crown_radius := envelope.x
	var crown_height := envelope.y - envelope.z
	var widest := lerpf(0.0, 0.16, _noise(variant, 113))
	var taper := lerpf(0.75, 1.05, _noise(variant, 127))
	var surface := SurfaceTool.new()
	surface.begin(Mesh.PRIMITIVE_TRIANGLES)
	if lod < 2:
		_lathe(surface, _trunk_profile(lerpf(3.1, 3.7, _noise(variant, 131)),
			lerpf(0.26, 0.34, _noise(variant, 139))), maxi(segments - 4, 3),
			Color(0.115, 0.085, 0.062), 0.10, 11 + variant * 61)
	var profile := PackedVector2Array()
	for ring in range(rings + 1):
		var t := float(ring) / float(rings)
		# A short lower skirt rises to the widest ring; the upper crown tapers to a point.
		var radius := crown_radius * lerpf(0.88, 1.0, t / widest) if t < widest else (
			crown_radius * pow((1.0 - t) / (1.0 - widest), taper)
		)
		profile.append(Vector2(crown_base + t * crown_height, radius))
	_lathe(surface, profile, segments, crown, ragged, 3 + variant * 61)
	return _finish(surface, _distant_crown_material(true))

## Birch and generic broadleaf share the catalogue slot and distant species mean.
static func _broadleaf(lod: int, variant: int, crown: Color, envelope: Vector3) -> ArrayMesh:
	var segments: int = [9, 6, 4][lod]
	var rings: int = [7, 5, 3][lod]
	var ragged: float = [0.14, 0.10, 0.0][lod] * lerpf(0.8, 1.2, _noise(variant, 149))
	var crown_base := envelope.z
	# See _conifer: the near crown's own foliage extent. The birch narrowing that used to apply
	# here is already in that measurement, because the envelope is a mean over all twelve near
	# variants and eight of them are birch. Narrowing again also split the two distant levels,
	# which both stand for the same mixed stand: variant zero is birch and is the variant the
	# scatter draws at distance, so mid came out 91% of the near crown and far 111%.
	var crown_radius := envelope.x
	var crown_height := envelope.y - envelope.z
	# Snapped to a ring the profile actually samples. A broadleaf dome is widest across its
	# middle, and the far level has three rings at t = 0, 1/3, 2/3 and 1: an authored 0.38-0.49
	# falls between two of them, so the widest point of the crown was never built and the far
	# footprint measured 75% of the near crown against the mid level's 90%. Snapping costs no
	# geometry, and the ring count is what the appearance test pins.
	var widest := roundf(lerpf(0.38, 0.49, _noise(variant, 167)) * rings) / float(rings)
	var taper := lerpf(0.85, 1.15, _noise(variant, 173))
	var surface := SurfaceTool.new()
	surface.begin(Mesh.PRIMITIVE_TRIANGLES)
	if lod < 2:
		if _is_birch(variant):
			_banded_trunk(surface, lerpf(4.2, 5.0, _noise(variant, 179)),
				lerpf(0.21, 0.27, _noise(variant, 181)), maxi(segments - 4, 3), variant, false, true)
		else:
			_lathe(surface, _trunk_profile(lerpf(4.2, 5.0, _noise(variant, 179)),
				lerpf(0.21, 0.27, _noise(variant, 181))), maxi(segments - 4, 3),
				Color(0.185, 0.170, 0.150), 0.08, 23 + variant * 61)
	var profile := PackedVector2Array()
	for ring in range(rings + 1):
		var t := float(ring) / float(rings)
		# Move the widest point independently of the roundness of the upper crown.
		var phase := t / widest * 0.5 if t < widest else 0.5 + (t - widest) / (1.0 - widest) * 0.5
		var radius := crown_radius * pow(maxf(sin(phase * PI), 0.0), taper) + 0.15
		profile.append(Vector2(crown_base + t * crown_height, radius))
	_lathe(surface, profile, segments, crown, ragged, 7 + variant * 61)
	return _finish(surface, _distant_crown_material(false))

# One material per species, shared by both distant levels; still one surface per mesh.
# The two need different hash coverage to reach the same silhouette fill as their own
# near crowns, because a solid spruce cone fills far more of its bounding box than a
# broadleaf dome fills its own. Placement already draws the species separately, so the
# second material costs no extra draw call.
static func _distant_crown_material(conifer: bool) -> ShaderMaterial:
	var index := 0 if conifer else 1
	if _distant_materials[index] == null:
		var material := ShaderMaterial.new()
		material.shader = preload("res://scripts/shaders/vegetation_distant.gdshader")
		material.set_shader_parameter("crown_coverage", DISTANT_COVERAGE[index])
		material.set_shader_parameter("distant_radiance_match", DISTANT_RADIANCE_MATCH[index])
		_apply_canopy_shading(material)
		_distant_materials[index] = material
	return _distant_materials[index]

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
					0.0, false, color, radial * 0.8 + Vector3.UP, 0.16)
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
					true, seed, 0.0, false, Color(0.199, 0.261, 0.155), axis, 0.24)
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
						Vector3(0.12, length, 0.09), true, seed, 0.0, false,
						Color(0.186, 0.237, 0.085), radial + Vector3.UP * 0.25, 0.25)
			_foliage_cards(cards, Vector3.UP * 1.40, Vector3.UP, Vector3.RIGHT,
				Vector3(0.075, 0.20, 0.06), true, variant, 0.0, false,
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

## Boulder. Strong raggedness and a squat profile; per-instance scale flattens it further.
static func _rock(variant: int) -> ArrayMesh:
	var height := lerpf(1.20, 1.50, _noise(variant, 211))
	var radius := lerpf(0.76, 0.94, _noise(variant, 223))
	var taper := lerpf(0.42, 0.60, _noise(variant, 227))
	var surface := SurfaceTool.new()
	surface.begin(Mesh.PRIMITIVE_TRIANGLES)
	var profile := PackedVector2Array()
	for ring in range(4):
		var t := float(ring) / 3.0
		profile.append(Vector2(-0.5 + t * height, radius * pow(maxf(1.0 - t * t, 0.0), taper) + 0.05))
	_lathe(surface, profile, 6, Color(0.190, 0.196, 0.175), lerpf(0.18, 0.26, _noise(variant, 229)), 57 + variant * 61, 0.35)
	return _finish(surface)
