# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: engine_terrain_source.gd
#  script_path: scripts/core/engine_terrain_source.gd
#  module_name: engine_terrain_source
#  version: 0.4.0
#  author: [BantedHam]
#  description: Terrain's evaluated height source: patch height textures
#           filled from the 2.5D engine's field instead of Rust payloads,
#           behind the same workflow toggle, byte-compatible with the
#           renderer's RF upload. Endpoint-inclusive sampling makes
#           abutting patches share their edge heights exactly, so the
#           seam every chunked terrain fights simply is not there.
#  kind: module
#  spec: none
#  internal_dependencies: [addons/2.5D_engine/evaluator/fbm_node.gd,
#           scripts/core/engine_mesh_source.gd]
#  external_dependencies: [Godot 4.x]
#  features: [evaluated-heights, seam-exact, toggle]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## Patch heights evaluated from the engine's field.
@tool
extends RefCounted

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const Fbm := preload("res://addons/2.5D_engine/evaluator/fbm_node.gd")
const EngineMeshSource := preload("res://scripts/core/engine_mesh_source.gd")

## One seed for the world's ground; the sim's world seed can replace it
## when the shell wires one through.
const FIELD_SEED := 0x2E5D
const FOOTPRINT := 0.5
const AMPLITUDE_M := 8.0
## Metres of world per unit of field: the fBm's base wavelength is about
## one field unit, so this is the size of the largest landform. Raw
## metres put a full hill in every metre of ground; the windowed run
## drew that as needle terrain and scored every parcel unbuildable,
## which is how this constant earned its place.
const WORLD_SCALE_M := 1000.0


# =========================================================================
# GROUND M
# =========================================================================
## THE ground, in metres, at a world position: every consumer that
## stands on, conforms to, or scores the drawn terrain reads this one
## function, so the scale can never fork between them. The footprint is
## a band limit in field units and was authored in metres, so it rides
## the same division as the coordinates; passing it raw fades out every
## octave finer than half a kilometre, and the terrain spike measured
## that as a 64 m patch with no relief in it.
## The optional footprint is the caller's own sample spacing in metres:
## a sampler asking every four metres cannot see finer octaves, it can
## only alias them, and evaluating them anyway was most of the patch
## upload's cost.
static func ground_m(wx: float, wz: float, footprint_m: float = FOOTPRINT) -> float:
	return Fbm.evaluate(wx / WORLD_SCALE_M, 0.0, wz / WORLD_SCALE_M,
		maxf(FOOTPRINT, footprint_m) / WORLD_SCALE_M, 0.0,
		FIELD_SEED) * AMPLITUDE_M


## The sim terrain grid's cell size, which is the band limit the ground
## fill evaluated at; the deformation baseline must match it exactly or
## untouched ground reads as sculpted. The core's default; a world
## authored at another resolution moves this with it.
const SIM_CELL_M := 10.0
## The game's stored-height convention: stored units are real metres
## divided by this render exaggeration (the level tool divides by it,
## world Y multiplies it back). Every byte this source emits obeys it;
## writing raw metres into stored units drew hills twenty times their
## authored steepness and made placement budgets reject honest ground,
## invisible on flat-zero worlds where every convention agrees.
const RENDER_HEIGHT_SCALE := 20.0
## Deviations under this are float rounding between the fill's f32
## store and this evaluation, not a sculpt.
const DEFORM_EPS_M := 0.001


# =========================================================================
# ENABLED
# =========================================================================
## The one workflow switch, shared with the mesh source.
static func enabled() -> bool:
	return EngineMeshSource.enabled()


# =========================================================================
# HEIGHT BYTES
# =========================================================================
## RF float32 height bytes for one patch, in the exact layout
## `_upload_terrain_patch_height_texture` uploads. Endpoint-inclusive over
## the patch span: a neighbour starting where this patch ends evaluates the
## identical world positions on the shared edge, and identical positions
## give identical heights, which is seam-exactness by construction.
static func height_bytes(patch_data: Dictionary) -> PackedByteArray:
	var tw := int(patch_data.get("texture_width", 0))
	var th := int(patch_data.get("texture_height", 0))
	if tw < 2 or th < 2:
		return PackedByteArray()
	var ox := float(patch_data.get("world_origin_x", 0.0))
	var oz := float(patch_data.get("world_origin_z", 0.0))
	var sx := float(patch_data.get("world_size_x", 1.0))
	var sz := float(patch_data.get("world_size_z", 1.0))
	var floats := PackedFloat32Array()
	floats.resize(tw * th)
	# The patch's own texel spacing is its band limit; bytes are stored
	# units, so the drawn ground is exactly ground_m of real relief.
	var step_m := maxf(sx / float(tw - 1), sz / float(th - 1))
	for row in th:
		var wz := oz + (float(row) / float(th - 1)) * sz
		for col in tw:
			var wx := ox + (float(col) / float(tw - 1)) * sx
			floats[row * tw + col] = ground_m(wx, wz, step_m) / RENDER_HEIGHT_SCALE
	return floats.to_byte_array()


# =========================================================================
# COMPOSITE BYTES
# =========================================================================
## The drawn ground with the sim's measured deformation on it: the fine
## field plus the sim's deviation from its undeformed baseline, so a
## sculpt or an earthwork draws exactly where the sim holds it, and
## untouched ground keeps the fine octaves the sim grid cannot carry.
## The baseline lives on the sim's own cell lattice and interpolates
## to texels exactly as the sim's payload does, so the difference is
## deformation and nothing else; detection is a free arithmetic scan,
## and an undeformed patch returns the pure field.
static func composite_bytes(patch_data: Dictionary, sim_bytes: PackedByteArray) -> PackedByteArray:
	var tw := int(patch_data.get("texture_width", 0))
	var th := int(patch_data.get("texture_height", 0))
	if tw < 2 or th < 2:
		return PackedByteArray()
	var sim := sim_bytes.to_float32_array()
	if sim.size() != tw * th:
		return height_bytes(patch_data)
	var ox := float(patch_data.get("world_origin_x", 0.0))
	var oz := float(patch_data.get("world_origin_z", 0.0))
	var sx := float(patch_data.get("world_size_x", 1.0))
	var sz := float(patch_data.get("world_size_z", 1.0))
	var baseline := baseline_texels(ox, oz, sx, sz, tw, th)
	var deformed := false
	for i in tw * th:
		if absf(sim[i] - baseline[i]) > DEFORM_EPS_M:
			deformed = true
			break
	if not deformed:
		return height_bytes(patch_data)
	var floats := PackedFloat32Array()
	floats.resize(tw * th)
	var step_m := maxf(sx / float(tw - 1), sz / float(th - 1))
	for row in th:
		var wz := oz + (float(row) / float(th - 1)) * sz
		for col in tw:
			var wx := ox + (float(col) / float(tw - 1)) * sx
			var i := row * tw + col
			floats[i] = ground_m(wx, wz, step_m) / RENDER_HEIGHT_SCALE \
				+ (sim[i] - baseline[i])
	return floats.to_byte_array()


# =========================================================================
# BASELINE TEXELS
# =========================================================================
## The undeformed fill, sampled on the global sim cell lattice and
## bilinearly interpolated to the patch's texels, which is exactly how
## the sim's own payload reaches texels, so sim minus this is the
## deformation alone.
static func baseline_texels(ox: float, oz: float, sx: float, sz: float,
		tw: int, th: int) -> PackedFloat32Array:
	var cx0 := floorf(ox / SIM_CELL_M) * SIM_CELL_M
	var cz0 := floorf(oz / SIM_CELL_M) * SIM_CELL_M
	var nx := int(ceilf((ox + sx - cx0) / SIM_CELL_M)) + 2
	var nz := int(ceilf((oz + sz - cz0) / SIM_CELL_M)) + 2
	var lattice := PackedFloat32Array()
	lattice.resize(nx * nz)
	for j in nz:
		for i in nx:
			lattice[j * nx + i] = ground_m(
				cx0 + float(i) * SIM_CELL_M, cz0 + float(j) * SIM_CELL_M,
				SIM_CELL_M) / RENDER_HEIGHT_SCALE
	var out := PackedFloat32Array()
	out.resize(tw * th)
	for row in th:
		var wz := oz + (float(row) / float(th - 1)) * sz
		var gz := (wz - cz0) / SIM_CELL_M
		var j0 := clampi(int(floorf(gz)), 0, nz - 2)
		var tz := gz - float(j0)
		for col in tw:
			var wx := ox + (float(col) / float(tw - 1)) * sx
			var gx := (wx - cx0) / SIM_CELL_M
			var i0 := clampi(int(floorf(gx)), 0, nx - 2)
			var tx := gx - float(i0)
			var v00 := lattice[j0 * nx + i0]
			var v10 := lattice[j0 * nx + i0 + 1]
			var v01 := lattice[(j0 + 1) * nx + i0]
			var v11 := lattice[(j0 + 1) * nx + i0 + 1]
			out[row * tw + col] = lerpf(lerpf(v00, v10, tx),
				lerpf(v01, v11, tx), tz)
	return out
