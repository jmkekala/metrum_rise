// SPDX-License-Identifier: GPL-2.0-only

//! The vegetation patch product (`RENDER-32-C`, round V1a; `docs/terrain.md` "Streaming core"):
//! every instance buffer a vegetation patch draws, built in Rust from the patch's placements, its
//! stand cover and its hedge runs, bit for bit what `vegetation.gd`'s upload loops built before.
//!
//! A batch is one `MultiMesh` of the patch: the near level of one species variant (transform,
//! tint and, for canopy trees, the stand closure), the distant impostors of one canopy species
//! (transform, then the variant layer with the stand closure and the tint), its shadow caster
//! (the same buffer), or the distant boxes of one hedge kind. Each batch carries its bounds and
//! its visibility range: the shared bounds every level of one canopy species measures its range
//! from, which `_share_patch_bounds` used to compute from the nodes.
//!
//! The appearance seed is Godot's `hash()` of the placement's world position, and the jitters
//! hash the seed again; [`godot_math`] reproduces both, and from here on Rust owns them, so a
//! plant's look no longer depends on the engine version. Every operation keeps GDScript's types:
//! `float` arithmetic in double precision, `Vector3`, `Basis`, `Color` and `AABB` lanes in single.
//!
//! Cost: O(P + R + B) for P placements, R hedge runs and B batches (at most 51 near variants,
//! two impostor batches with their casters and three hedge kinds), with one bounded hash and
//! trigonometry per placement and no allocation once the scratch has grown to the largest patch.

pub(crate) mod godot_math;
#[cfg(test)]
mod tests;

use godot_math::{Aabb, Basis, add3, basis_axis_angle, basis_columns, basis_mul, hash_int};
use godot_math::{column_length_squared, hash_vector2, length3, lerp_f64, max_f64, max3, min3};
use godot_math::{smoothstep_f64, sub3};
use std::cell::RefCell;
use std::f64::consts::TAU;

const BUSH: usize = 2;
const ROCK: usize = 3;
const SPECIES_COUNT: usize = 4;
// Canopy species: a near level and a distant impostor.
const CANOPY_SPECIES: usize = 2;
/// Mesh variants per species (`TreeSpecies.VARIANT_COUNTS`).
pub(crate) const VARIANT_COUNTS: [usize; SPECIES_COUNT] = [12, 12, 21, 6];
// Variants the appearance seed chooses among (`TreeSpecies.SEED_VARIANT_COUNTS`); the rest
// only a brush pin plants.
const SEED_VARIANT_COUNTS: [i64; SPECIES_COUNT] = [12, 12, 6, 6];
// Clipped hedge modules (`TreeSpecies.HEDGE_FIRST_VARIANT`, `HEDGE_VARIANTS`).
const HEDGE_FIRST_VARIANT: usize = 12;
const HEDGE_VARIANTS: usize = 3;
// Height and width of each hedge kind's distant box (`TreeSpecies.HEDGE_RUN_SIZES`, x only).
const HEDGE_RUN_HEIGHTS: [f32; HEDGE_VARIANTS] = [0.90, 1.40, 1.90];
const SPECIES_BITS: i64 = 2;
const SPECIES_MASK: i64 = 3;
// A storm leans an impostor's top downwind by up to 1.52 m (vegetation_wind.gdshaderinc).
const WIND_LEAN_MARGIN_M: f64 = 1.6;
// Half the diagonal of a square per metre of its side; also a rotated impostor quad's reach.
const PATCH_HALF_DIAGONAL: f64 = std::f64::consts::FRAC_1_SQRT_2;
// Coverage over 40 m at which a stand is still open and at which it is closed
// (terrain.gdshader's CANOPY_FLOOR_OPEN_COVER and CANOPY_FLOOR_CLOSED_COVER).
const STAND_OPEN_COVER: f64 = 0.35;
const STAND_CLOSED_COVER: f64 = 0.7;
// How far inside its range one understory variant may stop, as a share of the range.
const UNDERSTORY_STAGGER_MIN: f64 = 0.75;
/// Floats per placement record: x, height, z, yaw, scale (a hedge's rise), species and pin.
pub(crate) const PLACEMENT_STRIDE: usize = 6;
/// Floats per hedge run: centre x, height, centre z, yaw, length, rise and hedge kind.
pub(crate) const RUN_STRIDE: usize = 7;
/// Level codes of a batch: near, distant impostor and distant hedge box.
pub(crate) const LOD_NEAR: u8 = 0;
/// Level code of a canopy species' impostors and its shadow caster.
pub(crate) const LOD_IMPOSTOR: u8 = 2;
/// Level code of a hedge kind's distant boxes.
pub(crate) const LOD_HEDGE_RUN: u8 = 3;
// Floats per instance: transform rows; then the colour; then the custom lane.
const TRANSFORM_FLOATS: usize = 12;

/// The draw distances a product's ranges are measured with, which the renderer's probe
/// overrides can change, in GDScript's double precision.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ProductSettings {
    /// Share of the scatter kept (`density_fraction`).
    pub(crate) density: f64,
    /// Where every canopy tree has handed over to its impostor (`canopy_near_m`).
    pub(crate) near_m: f64,
    /// Where the canopy handover begins (`canopy_crossfade_begin_m`).
    pub(crate) crossfade_begin_m: f64,
    /// The canopy far range (`canopy_far_m`).
    pub(crate) far_m: f64,
    /// Ground cover range (`BUSH_RANGE_M`).
    pub(crate) bush_range_m: f64,
    /// Rock range (`ROCK_RANGE_M`).
    pub(crate) rock_range_m: f64,
}

impl ProductSettings {
    /// The bridge's packing: density, near, crossfade begin, far, bush range, rock range.
    pub(crate) fn from_lanes(lanes: &[f64]) -> Option<Self> {
        let [
            density,
            near_m,
            crossfade_begin_m,
            far_m,
            bush_range_m,
            rock_range_m,
        ] = *lanes.first_chunk::<6>()?;
        Some(Self {
            density,
            near_m,
            crossfade_begin_m,
            far_m,
            bush_range_m,
            rock_range_m,
        })
    }
}

/// What the product needs from the renderer's mesh catalogue: per canopy species, how far its
/// impostor's baked forms reach from their quad's origin (`bounds_centre` and `bounds_size` of
/// the impostor material) and the bounds of each near variant's full mesh, which the engine
/// transforms by every instance to bound a near `MultiMesh`.
#[derive(Clone, Debug)]
pub(crate) struct ProductCatalogue {
    impostor_reach: [f64; CANOPY_SPECIES],
    near_bounds: [Vec<Aabb>; CANOPY_SPECIES],
}

impl ProductCatalogue {
    /// The bridge's packing, per canopy species: the impostor layer count `n`, `n` centres
    /// (three floats each), `n` sizes, then six floats (position, size) for each of the
    /// species' near variants. `None` when the lanes do not hold exactly that.
    pub(crate) fn from_lanes(lanes: &[f32]) -> Option<Self> {
        let mut rest = lanes;
        let mut reach = [0.0; CANOPY_SPECIES];
        let mut near_bounds: [Vec<Aabb>; CANOPY_SPECIES] = Default::default();
        for species in 0..CANOPY_SPECIES {
            let (&layers, tail) = rest.split_first()?;
            let layers = layers as usize;
            let (centres, tail) = tail.split_at_checked(layers * 3)?;
            let (sizes, tail) = tail.split_at_checked(layers)?;
            reach[species] = impostor_reach(centres, sizes);
            let (boxes, tail) = tail.split_at_checked(VARIANT_COUNTS[species] * 6)?;
            near_bounds[species] = boxes
                .as_chunks::<6>()
                .0
                .iter()
                .map(|b| Aabb::new([b[0], b[1], b[2]], [b[3], b[4], b[5]]))
                .collect();
            rest = tail;
        }
        rest.is_empty().then_some(Self {
            impostor_reach: reach,
            near_bounds,
        })
    }
}

// The impostor shader moves vertices outside the quad's own bounds, so every baked form's box
// is merged (from Godot's empty AABB, which holds the origin) and the farthest corner of the
// merge is how far a unit-scale impostor reaches. A rotated square's corner reaches its size
// over the square root of two from its centre.
fn impostor_reach(centres: &[f32], sizes: &[f32]) -> f64 {
    let mut bounds = Aabb::default();
    for (centre, &size) in centres.as_chunks::<3>().0.iter().zip(sizes) {
        let radius = (f64::from(size) * PATCH_HALF_DIAGONAL) as f32;
        let side = radius * 2.0;
        let low = sub3([centre[0], centre[1], centre[2]], [radius; 3]);
        bounds = bounds.merge(&Aabb::new(low, [side; 3]));
    }
    bounds.corners().into_iter().fold(0.0, |reach, corner| {
        max_f64(reach, f64::from(length3(corner)))
    })
}

/// The patch's crown coverage averaged over 40 m (`get_vegetation_stand_cover`): one byte a
/// texel, row-major, and the world rectangle it covers as (x, z, width, depth).
#[derive(Clone, Copy, Debug)]
pub(crate) struct StandCover<'a> {
    /// Coverage bytes, `width * height` of them.
    pub(crate) bytes: &'a [u8],
    /// Texels across.
    pub(crate) width: i64,
    /// Texels down.
    pub(crate) height: i64,
    /// World rectangle of the grid: x, z, width, depth.
    pub(crate) bounds: [f32; 4],
}

/// Everything one patch's product is built from.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ProductInputs<'a> {
    /// Placement records ([`PLACEMENT_STRIDE`] floats each) in world coordinates.
    pub(crate) placements: &'a [f32],
    /// Stand cover around the patch; `None` when empty, which keeps every stand open.
    pub(crate) cover: Option<StandCover<'a>>,
    /// Hedge runs ([`RUN_STRIDE`] floats each), drawn only outside the near band.
    pub(crate) runs: &'a [f32],
    /// The patch origin (its minimum corner); transforms are relative to it.
    pub(crate) origin: [f32; 2],
    /// Whether the patch carries its near levels (and the understory).
    pub(crate) near_band: bool,
    /// Whether the canopy species get a shadow caster batch.
    pub(crate) caster: bool,
    /// Draw distances.
    pub(crate) settings: ProductSettings,
}

/// One `MultiMesh` of a patch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Batch {
    /// Species ordinal.
    pub(crate) species: u8,
    /// [`LOD_NEAR`], [`LOD_IMPOSTOR`] or [`LOD_HEDGE_RUN`].
    pub(crate) lod: u8,
    /// Mesh variant: the near variant, 0 for impostors, the hedge variant for hedge boxes.
    pub(crate) variant: u8,
    /// The shadow caster that shares its impostor batch's buffer.
    pub(crate) proxy: bool,
    /// Instances.
    pub(crate) count: u32,
    /// Whether the buffer carries a colour lane.
    pub(crate) colors: bool,
    /// Whether the buffer carries a custom lane.
    pub(crate) custom: bool,
    /// Index of the batch's buffer in [`VegetationProduct::buffers`].
    pub(crate) buffer: usize,
    /// The `MultiMesh`'s own bounds (impostors only; empty for the engine to compute).
    pub(crate) mesh_bounds: Aabb,
    /// The instance's bounds shared by every level of the species, or empty.
    pub(crate) shared_bounds: Aabb,
    /// Visibility range begin and end, metres.
    pub(crate) range: [f32; 2],
}

/// The product of one patch, in the order the renderer adds its instances. Its buffers are
/// kept between products on one thread, so a patch reuses the storage of the last.
#[derive(Clone, Debug, Default)]
pub(crate) struct VegetationProduct {
    /// Batches, in child order.
    pub(crate) batches: Vec<Batch>,
    // Instance buffers in Godot's `MultiMesh` layout; the first `buffer_count` are this
    // product's, the rest spare storage.
    buffers: Vec<Vec<f32>>,
    buffer_count: usize,
    /// Plants and hedge runs drawn (`tree_count`).
    pub(crate) tree_count: i64,
    /// World box of the canopy tree origins, which the near detail choice measures.
    pub(crate) canopy_box: Aabb,
}

// One kept placement, in placement order.
#[derive(Clone, Copy, Default)]
struct Placed {
    species: u8,
    variant: u8,
    // Basis rows and origin, in the MultiMesh row layout.
    rows: [f32; TRANSFORM_FLOATS],
    tint: [f32; 3],
    // Stand closure, canopy only.
    closure: f64,
}

// Reused between products on one thread, so a patch allocates nothing once the storage has
// grown to the largest patch.
#[derive(Default)]
struct Scratch {
    placed: Vec<Placed>,
    // Placement indices grouped by bucket (species, variant), in placement order.
    order: Vec<u32>,
    starts: Vec<usize>,
    cursor: Vec<usize>,
}

thread_local! {
    static SCRATCH: RefCell<(Scratch, VegetationProduct)> = RefCell::default();
}

const BUCKETS: usize =
    VARIANT_COUNTS[0] + VARIANT_COUNTS[1] + VARIANT_COUNTS[2] + VARIANT_COUNTS[3];

fn bucket(species: usize, variant: usize) -> usize {
    VARIANT_COUNTS[..species].iter().sum::<usize>() + variant
}

/// Builds one patch's product and hands it to `read`, in this thread's reused storage.
/// Deterministic and pure: the product is a function of the inputs alone.
pub(crate) fn with_product<R>(
    inputs: &ProductInputs,
    catalogue: &ProductCatalogue,
    read: impl FnOnce(&VegetationProduct) -> R,
) -> R {
    SCRATCH.with_borrow_mut(|(scratch, product)| {
        build_into(inputs, catalogue, scratch, product);
        read(product)
    })
}

fn build_into(
    inputs: &ProductInputs,
    catalogue: &ProductCatalogue,
    scratch: &mut Scratch,
    product: &mut VegetationProduct,
) {
    place(inputs, &mut scratch.placed);
    group(scratch);
    product.reset();
    let mut origins: [Option<Aabb>; CANOPY_SPECIES] = [None; CANOPY_SPECIES];
    for species in 0..SPECIES_COUNT {
        let canopy = species < CANOPY_SPECIES;
        // Bush and rock have only a near level, so out of the near band they draw nothing.
        if !inputs.near_band && !canopy {
            continue;
        }
        let first = product.batches.len();
        let mut species_count = 0;
        if inputs.near_band {
            for variant in 0..VARIANT_COUNTS[species] {
                let b = bucket(species, variant);
                let members = &scratch.order[scratch.starts[b]..scratch.starts[b + 1]];
                if members.is_empty() {
                    continue;
                }
                species_count += members.len();
                product.push_near(species, variant, members, &scratch.placed);
            }
        } else {
            species_count = scratch
                .placed
                .iter()
                .filter(|p| usize::from(p.species) == species)
                .count();
        }
        if species_count == 0 {
            continue;
        }
        product.tree_count += species_count as i64;
        if canopy {
            let spread = product.push_impostors(species, species_count, inputs, scratch, catalogue);
            origins[species] = Some(spread);
            product.share_bounds(first, species, spread, catalogue, inputs);
        }
    }
    if !inputs.near_band {
        product.push_hedge_runs(inputs);
    }
    product.set_ranges(inputs);
    product.canopy_box = canopy_box(&origins, inputs.origin);
}

// The placements the patch draws, in order, with their appearance. GDScript ran this once per
// species pass; one pass here keeps each species' order.
fn place(inputs: &ProductInputs, placed: &mut Vec<Placed>) {
    placed.clear();
    let cover = inputs
        .cover
        .map(|cover| CoverSampler::new(cover, inputs.origin));
    for record in inputs.placements.as_chunks::<PLACEMENT_STRIDE>().0 {
        let packed = record[5] as i64;
        let species = (packed & SPECIES_MASK) as usize;
        if !inputs.near_band && species >= BUSH {
            continue;
        }
        // Stable subset for paired density experiments, independent of residency order.
        let (x, z) = (f64::from(record[0]), f64::from(record[2]));
        if (x * 0.754877 + z * 0.56984).abs() % 1.0 >= inputs.settings.density {
            continue;
        }
        let seed = i64::from(hash_vector2(record[0], record[2]));
        let pin = packed >> SPECIES_BITS;
        let variant = if pin > 0 {
            (pin - 1) as usize
        } else {
            (((seed >> 11) & 0xFFFF) % SEED_VARIANT_COUNTS[species]) as usize
        };
        // A pin past the species' meshes names nothing to draw (Rust validates pins at the
        // edit, so only a hand-made record reaches this).
        if variant >= VARIANT_COUNTS[species] {
            continue;
        }
        let hedge = inputs.near_band && is_hedge(species, variant);
        let rows = if hedge {
            hedge_module_rows(record, inputs.origin)
        } else {
            instance_rows(record, seed, inputs.origin)
        };
        let tint = if hedge { [1.0; 3] } else { instance_tint(seed) };
        let closure = match (&cover, species < CANOPY_SPECIES) {
            (Some(cover), true) => cover.closure(rows[3], rows[11]),
            _ => 0.0,
        };
        placed.push(Placed {
            species: species as u8,
            variant: variant as u8,
            rows,
            tint,
            closure,
        });
    }
}

// Counting sort by (species, variant), stable, so each bucket keeps placement order.
fn group(scratch: &mut Scratch) {
    let Scratch {
        placed,
        order,
        starts,
        cursor,
    } = scratch;
    starts.clear();
    starts.resize(BUCKETS + 1, 0);
    for p in placed.iter() {
        starts[bucket(p.species.into(), p.variant.into()) + 1] += 1;
    }
    for b in 0..BUCKETS {
        starts[b + 1] += starts[b];
    }
    order.clear();
    order.resize(placed.len(), 0);
    cursor.clear();
    cursor.extend_from_slice(&starts[..BUCKETS]);
    for (index, p) in placed.iter().enumerate() {
        let b = bucket(p.species.into(), p.variant.into());
        order[cursor[b]] = index as u32;
        cursor[b] += 1;
    }
}

fn is_hedge(species: usize, variant: usize) -> bool {
    species == BUSH
        && (HEDGE_FIRST_VARIANT..HEDGE_FIRST_VARIANT + HEDGE_VARIANTS).contains(&variant)
}

// A hedge row closes only if every module keeps its authored size, stands upright and shares
// one colour. Lane four is the ground's rise along the module, which shears its length to
// follow the slope while its sides stay vertical.
fn hedge_module_rows(record: &[f32], origin: [f32; 2]) -> [f32; TRANSFORM_FLOATS] {
    let yaw = f64::from(record[3]);
    let basis = basis_columns(
        [yaw.cos() as f32, record[4], (-yaw.sin()) as f32],
        [0.0, 1.0, 0.0],
        [yaw.sin() as f32, 0.0, yaw.cos() as f32],
    );
    let position = [
        (f64::from(record[0]) - f64::from(origin[0])) as f32,
        record[1],
        (f64::from(record[2]) - f64::from(origin[1])) as f32,
    ];
    rows_of(&basis, position)
}

// The jitters take disjoint uses of one hash each: height and width ratios, and a small lean
// off vertical in a seeded direction. Yaw alone is nearly invisible on a near-symmetric lathe.
fn instance_rows(record: &[f32], seed: i64, origin: [f32; 2]) -> [f32; TRANSFORM_FLOATS] {
    let unit = |salt: i64| f64::from(hash_int(seed ^ salt) & 0xFF_FFFF) / 16_777_216.0;
    let scale = f64::from(record[4]);
    let height = scale * (0.82 + unit(17) * 0.42);
    let width = scale * (0.80 + unit(53) * 0.48);
    // A boulder's lean is its tilt in the ground, so rocks take it as well as plants.
    let lean = 0.02 + unit(91) * 0.07;
    let lean_direction = unit(137) * TAU;
    let yawed = basis_axis_angle([0.0, 1.0, 0.0], record[3]);
    let leaned = basis_axis_angle(
        [
            lean_direction.cos() as f32,
            0.0,
            lean_direction.sin() as f32,
        ],
        lean as f32,
    );
    let (width, height) = (width as f32, height as f32);
    let scaled = [[width, 0.0, 0.0], [0.0, height, 0.0], [0.0, 0.0, width]];
    let basis = basis_mul(&basis_mul(&yawed, &leaned), &scaled);
    let position = [
        (f64::from(record[0]) - f64::from(origin[0])) as f32,
        record[1],
        (f64::from(record[2]) - f64::from(origin[1])) as f32,
    ];
    rows_of(&basis, position)
}

// A small value spread and opposing red and blue shifts keep the foliage on its green axis,
// from disjoint bit ranges of the already mixed seed.
fn instance_tint(seed: i64) -> [f32; 3] {
    let value = lerp_f64(0.82, 1.18, ((seed >> 3) & 0xFFF) as f64 / 4096.0);
    let warmth = lerp_f64(-0.10, 0.10, ((seed >> 27) & 0xFFF) as f64 / 4096.0);
    [
        (value * (1.0 + warmth)) as f32,
        value as f32,
        (value * (1.0 - warmth)) as f32,
    ]
}

fn rows_of(basis: &Basis, origin: [f32; 3]) -> [f32; TRANSFORM_FLOATS] {
    let mut rows = [0.0; TRANSFORM_FLOATS];
    for i in 0..3 {
        rows[i * 4..i * 4 + 3].copy_from_slice(&basis[i]);
        rows[i * 4 + 3] = origin[i];
    }
    rows
}

fn basis_of(rows: &[f32; TRANSFORM_FLOATS]) -> Basis {
    std::array::from_fn(|i| [rows[i * 4], rows[i * 4 + 1], rows[i * 4 + 2]])
}

// Samples the stand cover at a tree's patch-local position: bilinear, gated as the forest floor
// is, so a lone crown or a small clump stays open and the inside of a stand reaches one.
struct CoverSampler<'a> {
    cover: StandCover<'a>,
    // Texel coordinates of the patch origin, and texels per metre.
    origin: [f32; 2],
    scale: [f32; 2],
}

impl<'a> CoverSampler<'a> {
    fn new(cover: StandCover<'a>, patch_origin: [f32; 2]) -> Self {
        let [bx, bz, bw, bh] = cover.bounds;
        let scale = [
            cover.width as f32 / max_f64(f64::from(bw), 1.0) as f32,
            cover.height as f32 / max_f64(f64::from(bh), 1.0) as f32,
        ];
        let origin = [
            (patch_origin[0] - bx) * scale[0] - 0.5,
            (patch_origin[1] - bz) * scale[1] - 0.5,
        ];
        Self {
            cover,
            origin,
            scale,
        }
    }

    // O(1).
    fn closure(&self, x: f32, z: f32) -> f64 {
        let StandCover {
            bytes,
            width,
            height,
            ..
        } = self.cover;
        if bytes.is_empty() {
            return 0.0;
        }
        let at = [
            self.origin[0] + x * self.scale[0],
            self.origin[1] + z * self.scale[1],
        ];
        let (ax, az) = (f64::from(at[0]), f64::from(at[1]));
        let x0 = clamp_i64(ax.floor() as i64, 0, width - 1);
        let z0 = clamp_i64(az.floor() as i64, 0, height - 1);
        let x1 = (x0 + 1).min(width - 1);
        let z1 = (z0 + 1).min(height - 1);
        let fx = (ax - ax.floor()).clamp(0.0, 1.0);
        let fz = (az - az.floor()).clamp(0.0, 1.0);
        let byte = |row: i64, column: i64| {
            bytes
                .get((row * width + column) as usize)
                .map_or(0.0, |&b| f64::from(b))
        };
        let c = lerp_f64(
            lerp_f64(byte(z0, x0), byte(z0, x1), fx),
            lerp_f64(byte(z1, x0), byte(z1, x1), fx),
            fz,
        ) / 255.0;
        c * smoothstep_f64(STAND_OPEN_COVER, STAND_CLOSED_COVER, c)
    }
}

// Godot's CLAMP.
fn clamp_i64(value: i64, low: i64, high: i64) -> i64 {
    if value < low {
        low
    } else if value > high {
        high
    } else {
        value
    }
}

impl VegetationProduct {
    /// The instance buffer of a batch ([`Batch::buffer`]).
    pub(crate) fn buffer(&self, index: usize) -> &[f32] {
        &self.buffers[..self.buffer_count][index]
    }

    /// Instance buffers in use.
    pub(crate) fn buffer_count(&self) -> usize {
        self.buffer_count
    }

    fn reset(&mut self) {
        self.batches.clear();
        self.buffer_count = 0;
        self.tree_count = 0;
        self.canopy_box = Aabb::default();
    }

    // The next buffer's storage, emptied: a spare one when there is one.
    fn next_buffer(&mut self) -> Vec<f32> {
        if self.buffer_count == self.buffers.len() {
            self.buffers.push(Vec::new());
        }
        let mut buffer = std::mem::take(&mut self.buffers[self.buffer_count]);
        buffer.clear();
        buffer
    }

    fn push_batch(&mut self, batch: Batch, buffer: Vec<f32>) {
        self.buffers[self.buffer_count] = buffer;
        self.buffer_count += 1;
        self.batches.push(batch);
    }

    // One near variant: transform, tint and, for a canopy tree, its stand closure.
    fn push_near(&mut self, species: usize, variant: usize, members: &[u32], placed: &[Placed]) {
        let canopy = species < CANOPY_SPECIES;
        let stride = TRANSFORM_FLOATS + 4 + if canopy { 4 } else { 0 };
        let mut buffer = self.next_buffer();
        buffer.reserve(members.len() * stride);
        for &index in members {
            let p = &placed[index as usize];
            buffer.extend_from_slice(&p.rows);
            buffer.extend_from_slice(&[p.tint[0], p.tint[1], p.tint[2], 1.0]);
            if canopy {
                buffer.extend_from_slice(&[p.closure as f32, 0.0, 0.0, 0.0]);
            }
        }
        let batch = Batch {
            species: species as u8,
            lod: LOD_NEAR,
            variant: variant as u8,
            proxy: false,
            count: members.len() as u32,
            colors: true,
            custom: canopy,
            buffer: self.buffer_count,
            mesh_bounds: Aabb::default(),
            shared_bounds: Aabb::default(),
            range: [0.0; 2],
        };
        self.push_batch(batch, buffer);
    }

    // Every tree's impostor, in the near buckets' order inside the band and placement order
    // outside it, and the shadow caster sharing the buffer. Returns the box of tree origins.
    fn push_impostors(
        &mut self,
        species: usize,
        count: usize,
        inputs: &ProductInputs,
        scratch: &Scratch,
        catalogue: &ProductCatalogue,
    ) -> Aabb {
        let mut buffer = self.next_buffer();
        buffer.reserve(count * 16);
        let mut low = [f32::INFINITY; 3];
        let mut high = [f32::NEG_INFINITY; 3];
        let mut scale_squared = 0.0f64;
        let mut write = |p: &Placed| {
            buffer.extend_from_slice(&p.rows);
            // The layer lane carries the stand closure below one half; see the impostor shaders.
            let layer = f64::from(p.variant) + 0.5 * p.closure;
            buffer.extend_from_slice(&[layer as f32, p.tint[0], p.tint[1], p.tint[2]]);
            let origin = [p.rows[3], p.rows[7], p.rows[11]];
            low = min3(low, origin);
            high = max3(high, origin);
            let basis = basis_of(&p.rows);
            scale_squared = max_f64(
                scale_squared,
                max_f64(
                    column_length_squared(&basis, 0).into(),
                    column_length_squared(&basis, 1).into(),
                ),
            );
        };
        if inputs.near_band {
            for variant in 0..VARIANT_COUNTS[species] {
                let b = bucket(species, variant);
                for &index in &scratch.order[scratch.starts[b]..scratch.starts[b + 1]] {
                    write(&scratch.placed[index as usize]);
                }
            }
        } else {
            for p in scratch
                .placed
                .iter()
                .filter(|p| usize::from(p.species) == species)
            {
                write(p);
            }
        }
        let spread = Aabb::new(low, sub3(high, low));
        // Per tree only the origin span and the largest axis scale; the box then grows by the
        // impostor's reach at that scale, which contains every tree's box.
        let grow = catalogue.impostor_reach[species] * scale_squared.sqrt() + WIND_LEAN_MARGIN_M;
        let bounds = spread.grow(grow as f32);
        let impostor = Batch {
            species: species as u8,
            lod: LOD_IMPOSTOR,
            variant: 0,
            proxy: false,
            count: count as u32,
            colors: false,
            custom: true,
            buffer: self.buffer_count,
            mesh_bounds: bounds,
            shared_bounds: Aabb::default(),
            range: [0.0; 2],
        };
        self.push_batch(impostor, buffer);
        if inputs.caster {
            self.batches.push(Batch {
                proxy: true,
                ..impostor
            });
        }
        spread
    }

    // Every level of one canopy species measures its range from one box: the merge of each
    // level's bounds in child order, the near levels' being the engine's own (each instance's
    // transform of the full mesh's bounds, merged in instance order). Applied to every level of
    // the species, since a canopy species always has its impostor level beside the near one.
    fn share_bounds(
        &mut self,
        first: usize,
        species: usize,
        spread: Aabb,
        catalogue: &ProductCatalogue,
        inputs: &ProductInputs,
    ) {
        let mut shared: Option<Aabb> = None;
        for batch in &self.batches[first..] {
            let bounds = if batch.mesh_bounds.is_zero_size() {
                near_bounds(
                    self.buffer(batch.buffer),
                    batch.count as usize,
                    &catalogue.near_bounds[species][batch.variant as usize],
                )
            } else {
                batch.mesh_bounds
            };
            shared = Some(shared.map_or(bounds, |box_| box_.merge(&bounds)));
        }
        let shared = shared.unwrap_or_default();
        // How far a tree origin stands from the centre the range is measured from: its farthest
        // corner. The box is padded for crowns and quads, which the handover ignores.
        let centre = shared.center();
        let reach = spread.corners().into_iter().fold(0.0, |reach, corner| {
            max_f64(reach, f64::from(length3(sub3(centre, corner))))
        });
        for batch in &mut self.batches[first..] {
            if !shared.is_zero_size() {
                batch.shared_bounds = shared;
            }
            batch.range = canopy_range(batch, reach, inputs);
        }
    }

    // The distant hedge: one box per straight run Rust merged from the modules, stretched along
    // the run, sheared up its slope with the sides kept vertical, and sunk into the lawn.
    fn push_hedge_runs(&mut self, inputs: &ProductInputs) {
        let runs = inputs.runs.as_chunks::<RUN_STRIDE>().0;
        for (hedge, &height) in HEDGE_RUN_HEIGHTS.iter().enumerate() {
            let mut buffer = self.next_buffer();
            for run in runs.iter().filter(|run| run[6] as i64 == hedge as i64) {
                let (yaw, length) = (f64::from(run[3]), f64::from(run[4]));
                let basis = basis_columns(
                    [
                        (yaw.cos() * length) as f32,
                        run[5],
                        (-yaw.sin() * length) as f32,
                    ],
                    [0.0, 1.0, 0.0],
                    [yaw.sin() as f32, 0.0, yaw.cos() as f32],
                );
                let lift = f64::from(height) * 0.5;
                let position = [
                    (f64::from(run[0]) - f64::from(inputs.origin[0])) as f32,
                    (f64::from(run[1]) - 0.05 + lift) as f32,
                    (f64::from(run[2]) - f64::from(inputs.origin[1])) as f32,
                ];
                buffer.extend_from_slice(&rows_of(&basis, position));
            }
            if buffer.is_empty() {
                // Back to the spares.
                self.buffers[self.buffer_count] = buffer;
                continue;
            }
            let batch = Batch {
                species: BUSH as u8,
                lod: LOD_HEDGE_RUN,
                variant: (HEDGE_FIRST_VARIANT + hedge) as u8,
                proxy: false,
                count: (buffer.len() / TRANSFORM_FLOATS) as u32,
                colors: false,
                custom: false,
                buffer: self.buffer_count,
                mesh_bounds: Aabb::default(),
                shared_bounds: Aabb::default(),
                range: [0.0; 2],
            };
            self.push_batch(batch, buffer);
        }
        self.tree_count += (inputs.runs.len() / RUN_STRIDE) as i64;
    }

    // Ranges of the batches no canopy pass set: ground cover, rocks and hedges.
    fn set_ranges(&mut self, inputs: &ProductInputs) {
        let s = &inputs.settings;
        for batch in &mut self.batches {
            let (species, variant) = (usize::from(batch.species), usize::from(batch.variant));
            if species < CANOPY_SPECIES {
                continue;
            }
            let end = if is_hedge(species, variant) {
                s.far_m
            } else if species == ROCK {
                s.rock_range_m * understory_stagger(variant)
            } else {
                s.bush_range_m * understory_stagger(variant)
            };
            batch.range = [0.0, end as f32];
        }
    }
}

// The engine's bounds of a near MultiMesh: the full mesh's bounds through each instance's
// transform, merged in instance order.
fn near_bounds(buffer: &[f32], count: usize, mesh: &Aabb) -> Aabb {
    let stride = buffer.len() / count.max(1);
    let mut bounds = Aabb::default();
    for (i, rows) in buffer.chunks_exact(stride).enumerate() {
        let one = mesh.transformed(rows);
        bounds = if i == 0 { one } else { bounds.merge(&one) };
    }
    bounds
}

// A canopy batch's range (`lod_range`): the caster lives as long as the patch; the near level
// reaches past the handover by the tree origins' reach from the shared centre, and the
// impostor begins that much before the handover begins when the patch also has a near level.
fn canopy_range(batch: &Batch, reach: f64, inputs: &ProductInputs) -> [f32; 2] {
    let s = &inputs.settings;
    if batch.proxy {
        return [0.0, s.far_m as f32];
    }
    if batch.lod == LOD_NEAR {
        return [0.0, (s.near_m + reach) as f32];
    }
    let begin = if inputs.near_band {
        max_f64(s.crossfade_begin_m - reach, 0.0)
    } else {
        0.0
    };
    [begin as f32, (s.far_m + reach) as f32]
}

// Share of its range one understory variant keeps: a low-discrepancy sequence, so the variants
// end evenly over the last quarter of the band instead of on a few lines.
fn understory_stagger(variant: usize) -> f64 {
    lerp_f64(
        UNDERSTORY_STAGGER_MIN,
        1.0,
        (variant as f64 * 0.6180339887498949) % 1.0,
    )
}

// Every canopy tree origin of the patch in world space: the species' origin boxes merged (the
// first with itself too, as the GDScript did), moved by the patch origin.
fn canopy_box(origins: &[Option<Aabb>; CANOPY_SPECIES], patch_origin: [f32; 2]) -> Aabb {
    let spreads = origins.iter().flatten();
    let mut box_ = spreads.clone().next().copied().unwrap_or_default();
    for spread in spreads {
        box_ = box_.merge(spread);
    }
    box_.position = add3(box_.position, [patch_origin[0], 0.0, patch_origin[1]]);
    box_
}
