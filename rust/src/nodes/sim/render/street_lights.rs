// SPDX-License-Identifier: GPL-2.0-only

//! Street lights' camera passes (`RENDER-32-H`, `RENDER-33`, `docs/roads.md` `ROAD-59`): the
//! lamp cell grid the lit materials read, the far light map, the pool of real lights, and each
//! road chunk's instance buffers, over the lamps Rust places with the chunk (five floats a
//! light: chunk-local pole base, yaw, model kind). Render-consumer state, never saved; GDScript
//! hands every chunk over as it installs it and uploads what this builds.
//!
//! Every product is bit for bit what `street_lamp_grid.gd` and `street_lights.gd` built (Godot
//! 4.7, `real_t` single precision, no fused multiply-add): single precision where the engine
//! stores `real_t`, double where GDScript computed with `float`, in the engine's operation order.
//!
//! - **Lamp grid**: `SIDE` x `SIDE` cells of `CELL_M` around the camera's cell, `CAPACITY` slots a
//!   cell, every lamp head in the cells its range reaches, in a stable world order (x, z, y,
//!   kind). It re-centres after `GRID_RECENTRE_CELLS` cells of travel: `O(l log l + l k)` for
//!   the `l` lamps of the window's chunks, `k` <= 25 cells a lamp. A chunk edit re-packs only the
//!   cells the chunk's lamps can reach, in place: `O(m log m + m k)` for the `m` lamps of the
//!   chunks around it. A cell's list depends only on the lamps reaching it and the window origin,
//!   so a region re-pack equals a full one.
//! - **Far map**: `FAR_SIDE`^2 texels of `FAR_CELL_M` around the look ray's ground point, each
//!   head's light splatted bilinearly. Rebuilt after `FAR_RECENTRE_M` of focus travel: `O(C + h)`
//!   for `C` chunks and the `h` heads of the chunks over the map, in chunk upload order (the
//!   order of the control's sums); an edit takes the chunk's old heads out and adds the new ones,
//!   `O(chunk heads)`.
//! - **Pool**: the `POOL_SIZE` heads nearest the camera within `POOL_RADIUS_M`, from the chunks
//!   around it, every `POOL_REFRESH_S` or `POOL_REFRESH_MOVE_M`: `O(n)` for the `n` heads of
//!   those chunks (a selection, not a sort).
//!
//! One texture is rebuilt a frame: when both are due, the one that waited goes first. A still
//! camera costs a few comparisons. No allocation once the scratch buffers reached their size.

use super::vegetation_product::godot_math::{Basis, basis_axis_angle};
use godot::prelude::*;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

type Map<K, V> = HashMap<K, V, foldhash::fast::FixedState>;

/// Lamp grid cell side in metres.
pub(crate) const CELL_M: f32 = 16.0;
/// Lamp grid cells per side.
pub(crate) const SIDE: usize = 128;
/// Lamp slots a cell (`street_lamp_light.gdshaderinc` reads the same count): two street sides at
/// 25 m spacing plus junction approaches; excess heads keep their stable priority.
pub(crate) const CAPACITY: usize = 16;
/// Far map texel side in metres.
pub(crate) const FAR_CELL_M: f32 = 16.0;
/// Far map texels per side (8 km).
pub(crate) const FAR_SIDE: usize = 512;
/// Real lights at most.
pub(crate) const POOL_SIZE: usize = 32;
/// Street light models (the Rust `StreetLightKind`).
pub(crate) const KINDS: usize = 5;

const GRID_RECENTRE_CELLS: i32 = 16;
const FAR_RECENTRE_M: f64 = 1024.0;
// The look ray's ground point is capped this far ahead, so a view along the horizon stays centred.
const FAR_FOCUS_MAX_M: f64 = 3000.0;
// Real lights reach this far when fewer than POOL_SIZE lamps are inside it; otherwise the handoff
// moves in to the farthest lamp that got a real light.
const POOL_RADIUS_M: f64 = 80.0;
const POOL_FADE_M: f64 = 30.0;
const POOL_REFRESH_S: f64 = 0.2;
const POOL_REFRESH_MOVE_M: f64 = 3.0;
// Both real and material lamps reach this far past their head height.
const LAMP_RANGE_MARGIN_M: f64 = 20.0;
const LAMP_ENERGY: f64 = 10.0;
const LAMP_ANGLE_DEG: f64 = 80.0;
// The farthest a head stands from its pole (the median's arms), so chunk ownership by pole
// position bounds the heads.
const ARM_M: f32 = 3.4;
// More edited chunks than this before a pack re-packs the whole window instead.
const MAX_GRID_EDITS: usize = 32;
// Floats a cell takes: CAPACITY RGBA slots.
const CELL_FLOATS: usize = CAPACITY * 4;
const CELL_BYTES: usize = SIDE * SIDE * CELL_FLOATS * 4;
const FAR_BYTES: usize = FAR_SIDE * FAR_SIDE * 16;

/// `update` flag: the lamp grid changed; upload [`StreetLightBook::cell_data`].
pub(crate) const CELLS_CHANGED: i64 = 1;
/// `update` flag: the far map changed; upload [`StreetLightBook::far_data`].
pub(crate) const FAR_CHANGED: i64 = 2;
/// `update` flag: the pool was assigned; place [`StreetLightBook::pool`].
pub(crate) const POOL_CHANGED: i64 = 4;

// GDScript's Vector3 literal: a double rounded to real_t.
const fn v3(x: f64, y: f64, z: f64) -> [f32; 3] {
    [x as f32, y as f32, z as f32]
}

// Light points relative to the pole base in model space (arm reach along +X, lens height), from
// the models' metadata; the median pole has a lamp at each end.
const LIGHT_POINTS: [&[[f32; 3]]; KINDS] = [
    &[v3(1.45, 7.95, 0.0)],
    &[v3(1.4, 8.05, 0.0)],
    &[v3(2.55, 9.95, 0.0)],
    &[v3(3.4, 11.15, 0.0), v3(-3.4, 11.15, 0.0)],
    &[v3(0.0, 4.4, 0.0)],
];

// A kind's light range: head height plus the margin, as GDScript stored it (double sum, then a
// single-precision array).
fn kind_reach(kind: usize) -> f32 {
    (f64::from(LIGHT_POINTS[kind][0][1]) + LAMP_RANGE_MARGIN_M) as f32
}

// Godot's CLAMP(a, min, max).
fn clamp_godot(value: f32, low: f32, high: f32) -> f32 {
    if value < low {
        low
    } else if value > high {
        high
    } else {
        value
    }
}

fn write_f32(bytes: &mut [u8], index: usize, value: f32) {
    bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
}

fn read_f32(bytes: &[u8], index: usize) -> f32 {
    f32::from_le_bytes(bytes[index * 4..index * 4 + 4].try_into().expect("four bytes"))
}

/// A rectangle of cells or chunk keys, both ends inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CellRect {
    /// Lowest x.
    pub x0: i32,
    /// Lowest z.
    pub z0: i32,
    /// Highest x.
    pub x1: i32,
    /// Highest z.
    pub z1: i32,
}

impl CellRect {
    const WINDOW: CellRect = CellRect {
        x0: 0,
        z0: 0,
        x1: SIDE as i32 - 1,
        z1: SIDE as i32 - 1,
    };

    fn intersect(self, other: CellRect) -> Option<CellRect> {
        let rect = CellRect {
            x0: self.x0.max(other.x0),
            z0: self.z0.max(other.z0),
            x1: self.x1.min(other.x1),
            z1: self.z1.min(other.z1),
        };
        (rect.x0 <= rect.x1 && rect.z0 <= rect.z1).then_some(rect)
    }

    fn contains(self, x: i32, z: i32) -> bool {
        x >= self.x0 && x <= self.x1 && z >= self.z0 && z <= self.z1
    }
}

// A lamp head in the grid's priority order: its world position, kind and range.
#[derive(Clone, Copy, Debug)]
struct Row {
    head: [f32; 3],
    kind: u8,
    reach: f32,
}

// The stable world order of `_rebuild_grid`: x, z, y, kind (equal rows are identical).
fn row_order(a: &Row, b: &Row) -> Ordering {
    let by = |a: f32, b: f32| a.partial_cmp(&b).unwrap_or(Ordering::Equal);
    by(a.head[0], b.head[0])
        .then_with(|| by(a.head[2], b.head[2]))
        .then_with(|| by(a.head[1], b.head[1]))
        .then_with(|| a.kind.cmp(&b.kind))
}

/// Packs one head into the cells of `region` (window cells) its range reaches, after the heads
/// packed before it: `pack_cells` of `street_lamp_grid.gd` for one head. `slot_kind` is the value
/// the slot's fourth channel stores (the kind plus one).
fn pack_head(
    cells: &mut [u8],
    counts: &mut [u8],
    head: [f32; 3],
    slot_kind: f32,
    reach: f32,
    origin: [f32; 2],
    region: CellRect,
) {
    let point = [head[0] - origin[0], head[2] - origin[1]];
    let reach_sq = f64::from(reach) * f64::from(reach);
    let first = |p: f32| ((((p - reach) / CELL_M).ceil()) as i32 - 1).max(0);
    let last = |p: f32| ((((p + reach) / CELL_M).floor()) as i32).min(SIDE as i32 - 1);
    let x0 = first(point[0]).max(region.x0);
    let z0 = first(point[1]).max(region.z0);
    let x1 = last(point[0]).min(region.x1);
    let z1 = last(point[1]).min(region.z1);
    for z in z0..=z1 {
        for x in x0..=x1 {
            let low = [x as f32 * CELL_M, z as f32 * CELL_M];
            let closest = [
                clamp_godot(point[0], low[0], low[0] + CELL_M),
                clamp_godot(point[1], low[1], low[1] + CELL_M),
            ];
            let dx = point[0] - closest[0];
            let dz = point[1] - closest[1];
            if f64::from(dx * dx + dz * dz) > reach_sq {
                continue;
            }
            let cell = z as usize * SIDE + x as usize;
            let count = usize::from(counts[cell]);
            if count < CAPACITY {
                let offset = (cell * CAPACITY + count) * 4;
                write_f32(cells, offset, head[0]);
                write_f32(cells, offset + 1, head[1]);
                write_f32(cells, offset + 2, head[2]);
                write_f32(cells, offset + 3, slot_kind);
                counts[cell] += 1;
            }
        }
    }
}

// Zeroes the slots the cells of `region` hold.
fn clear_cells(cells: &mut [u8], counts: &mut [u8], region: CellRect) {
    for z in region.z0..=region.z1 {
        for x in region.x0..=region.x1 {
            let cell = z as usize * SIDE + x as usize;
            let used = usize::from(counts[cell]);
            if used > 0 {
                let start = cell * CELL_FLOATS * 4;
                cells[start..start + used * 16].fill(0);
                counts[cell] = 0;
            }
        }
    }
}

/// Adds one head's light times `scale` to the four far texels around it (`splat_far` of
/// `street_lamp_grid.gd`): `texel += light * w` in double precision, stored single.
#[allow(clippy::too_many_arguments)]
fn splat_head(
    texels: &mut [u8],
    head: [f32; 3],
    light: [f32; 3],
    scale: f32,
    origin: [f32; 2],
    cell_m: f32,
    side: i32,
) {
    let f = [
        (head[0] - origin[0]) / cell_m - 0.5,
        (head[2] - origin[1]) / cell_m - 0.5,
    ];
    let base = [f[0].floor() as i32, f[1].floor() as i32];
    let t = [f[0] - base[0] as f32, f[1] - base[1] as f32];
    let light = light.map(|channel| f64::from(channel * scale));
    for corner in 0..4 {
        let x = base[0] + (corner & 1);
        let z = base[1] + (corner >> 1);
        if x < 0 || z < 0 || x >= side || z >= side {
            continue;
        }
        let wx = if corner & 1 != 0 {
            f64::from(t[0])
        } else {
            1.0 - f64::from(t[0])
        };
        let wz = if corner >> 1 != 0 {
            f64::from(t[1])
        } else {
            1.0 - f64::from(t[1])
        };
        let w = wx * wz;
        let offset = (z as usize * side as usize + x as usize) * 4;
        for (channel, value) in light.iter().enumerate() {
            let sum = f64::from(read_f32(texels, offset + channel)) + value * w;
            write_f32(texels, offset + channel, sum as f32);
        }
    }
}

/// Light a lamp puts on level ground below it, integrated over the ground (m² times the shader's
/// irradiance with energy 1): the same range window, cone and Lambert as the exact lamp, summed on
/// a 1 m grid (`ground_flux` of `street_lamp_grid.gd`, double precision).
pub(crate) fn ground_flux(height: f64, reach: f64, cone_cos: f64) -> f64 {
    let mut flux = 0.0;
    let step = 1.0;
    let steps = (reach / step).ceil() as i64;
    let max = |a: f64, b: f64| if a > b { a } else { b };
    for zi in -steps..steps {
        for xi in -steps..steps {
            let x = (xi as f64 + 0.5) * step;
            let z = (zi as f64 + 0.5) * step;
            let d = (x * x + z * z + height * height).sqrt();
            if d >= reach {
                continue;
            }
            let nd = (d / reach).powf(4.0);
            let window = max(1.0 - nd, 0.0);
            let down = height / d;
            let rim = (1.0 - max(down, cone_cos)) / (1.0 - cone_cos);
            flux += window * window * (1.0 - rim * rim) * down / d * step * step;
        }
    }
    flux
}

// `cos(deg_to_rad(LAMP_ANGLE_DEG))` as GDScript evaluates it.
fn cone_cos() -> f64 {
    (LAMP_ANGLE_DEG * (std::f64::consts::PI / 180.0)).cos()
}

// A chunk's transforms of one (cell x, cell z, kind), in light order.
type Bucket = ((i64, i64, u8), Vec<(Basis, [f32; 3])>);

// One chunk's lamp heads (world space) and kinds, in the control's bucket order.
#[derive(Default)]
struct Chunk {
    order: u64,
    heads: Vec<[f32; 3]>,
    kinds: Vec<u8>,
}

/// A chunk's instance buffers for `MultiMesh.buffer`: per batch (one 64 m cell and kind, in the
/// order the control created them) its kind and transforms (12 floats an instance), and one glow
/// batch of every head (12 transform floats and the lamp colour an instance).
#[derive(Default, Debug, PartialEq)]
pub(crate) struct ChunkBatches {
    /// Kind of each batch.
    pub kinds: Vec<u8>,
    /// Transforms of each batch.
    pub transforms: Vec<Vec<f32>>,
    /// The glow batch.
    pub glow: Vec<f32>,
}

// A pool candidate: squared distance (single precision, as Godot measured it) and head.
#[derive(Clone, Copy, Debug)]
struct Candidate {
    distance_sq: f32,
    head: [f32; 3],
    kind: u8,
}

fn candidate_order(a: &Candidate, b: &Candidate) -> Ordering {
    a.distance_sq
        .total_cmp(&b.distance_sq)
        .then_with(|| a.head[0].total_cmp(&b.head[0]))
        .then_with(|| a.head[2].total_cmp(&b.head[2]))
        .then_with(|| a.head[1].total_cmp(&b.head[1]))
        .then_with(|| a.kind.cmp(&b.kind))
}

// The lamp grid's window: its centre cell and origin.
#[derive(Clone, Copy, Debug)]
struct GridWindow {
    cell: [i32; 2],
    origin: [f32; 2],
}

/// The street lights' books: chunk heads, the lamp grid, the far map and the pool.
pub(crate) struct LampBook {
    span_m: f32,
    chunk_origin: [f32; 2],
    loaded: [bool; KINDS],
    colors: [[f32; 4]; KINDS],
    // Per kind: linear colour * energy * ground flux / far cell area.
    kind_light: [[f32; 3]; KINDS],
    chunks: Map<(i32, i32), Chunk>,
    // Chunks in upload order (a re-upload moves a chunk to the end, as a Dictionary does).
    order: BTreeMap<u64, (i32, i32)>,
    next_order: u64,
    grid: Option<GridWindow>,
    grid_edits: Vec<(i32, i32)>,
    counts: Vec<u8>,
    rows: Vec<Row>,
    far_built: bool,
    far_origin: [f32; 2],
    far_centre: [f32; 2],
    far_upload: bool,
    // Both textures were due and the far map waited.
    far_waited: bool,
    pool_age_s: f64,
    pool_camera: Option<[f32; 3]>,
    candidates: Vec<Candidate>,
    pool: Vec<f32>,
    // Last update's work: lamps packed into the grid, heads splatted.
    packed_rows: usize,
}

impl Default for LampBook {
    fn default() -> Self {
        Self {
            span_m: 512.0,
            chunk_origin: [0.0, 0.0],
            loaded: [true; KINDS],
            colors: [[1.0; 4]; KINDS],
            kind_light: [[0.0; 3]; KINDS],
            chunks: Map::default(),
            order: BTreeMap::new(),
            next_order: 0,
            grid: None,
            grid_edits: Vec::new(),
            counts: vec![0; SIDE * SIDE],
            rows: Vec::new(),
            far_built: false,
            far_origin: [0.0, 0.0],
            far_centre: [f32::INFINITY, f32::INFINITY],
            far_upload: false,
            far_waited: false,
            pool_age_s: f64::INFINITY,
            pool_camera: None,
            candidates: Vec::new(),
            pool: Vec::with_capacity(4 + POOL_SIZE * 4),
            packed_rows: 0,
        }
    }
}

impl LampBook {
    /// Sets the road chunk grid; a change re-packs both textures.
    pub(crate) fn configure_chunks(&mut self, span_m: f32, origin: [f32; 2]) {
        if span_m != self.span_m || origin != self.chunk_origin {
            self.span_m = span_m;
            self.chunk_origin = origin;
            self.grid = None;
            self.grid_edits.clear();
            self.far_built = false;
        }
    }

    /// Sets the lamp colours (sRGB, for the glow), their linear values (for the far map's light)
    /// and which kinds have meshes (a light of another kind is skipped, as the control did).
    pub(crate) fn set_kinds(&mut self, colors: &[[f32; 4]], linear: &[[f32; 4]], loaded: &[bool]) {
        let cone = cone_cos();
        for (kind, points) in LIGHT_POINTS.iter().enumerate() {
            self.colors[kind] = colors.get(kind).copied().unwrap_or([1.0; 4]);
            self.loaded[kind] = loaded.get(kind).copied().unwrap_or(false);
            let height = f64::from(points[0][1]);
            let flux = ground_flux(height, height + LAMP_RANGE_MARGIN_M, cone);
            let scale = (LAMP_ENERGY * flux / f64::from(FAR_CELL_M * FAR_CELL_M)) as f32;
            let color = linear.get(kind).copied().unwrap_or([0.0; 4]);
            self.kind_light[kind] = [color[0] * scale, color[1] * scale, color[2] * scale];
        }
    }

    // Godot's `_chunk_bounds`: the chunk keys over the world rectangle `low`..`high`.
    fn chunk_bounds(&self, low: [f32; 2], high: [f32; 2]) -> CellRect {
        let key = |p: f32, o: f32| ((p - o) / self.span_m).floor() as i32;
        CellRect {
            x0: key(low[0], self.chunk_origin[0]),
            z0: key(low[1], self.chunk_origin[1]),
            x1: key(high[0], self.chunk_origin[0]),
            z1: key(high[1], self.chunk_origin[1]),
        }
    }

    /// Installs a chunk's lights (five floats each: chunk-local pole base, yaw, kind) with its
    /// frame `origin`, replacing what it held, and returns its batches. `O(lights)` plus the far
    /// map's patch; the grid re-packs the chunk's cells at the next update.
    pub(crate) fn set_chunk(
        &mut self,
        key: (i32, i32),
        origin: [f32; 3],
        lights: &[f32],
        far: &mut [u8],
    ) -> ChunkBatches {
        self.remove_chunk(key, far);
        let mut batches = ChunkBatches::default();
        if lights.len() < 5 {
            return batches;
        }
        // Transforms bucketed by (cell x, cell z, kind), buckets in first-seen order.
        let mut buckets: Vec<Bucket> = Vec::new();
        let mut index: Map<(i64, i64, u8), usize> = Map::default();
        for light in lights.as_chunks::<5>().0 {
            let kind = light[4] as i64;
            if kind < 0 || kind >= KINDS as i64 || !self.loaded[kind as usize] {
                continue;
            }
            let base = [light[0], light[1], light[2]];
            let cell_m = 64.0;
            let bucket = (
                (f64::from(base[0]) / cell_m).floor() as i64,
                (f64::from(base[2]) / cell_m).floor() as i64,
                kind as u8,
            );
            let slot = *index.entry(bucket).or_insert_with(|| {
                buckets.push((bucket, Vec::new()));
                buckets.len() - 1
            });
            buckets[slot]
                .1
                .push((basis_axis_angle([0.0, 1.0, 0.0], light[3]), base));
        }
        let mut chunk = Chunk::default();
        for ((_, _, kind), transforms) in &buckets {
            let mut buffer = Vec::with_capacity(transforms.len() * 12);
            for (basis, base) in transforms {
                for (row, origin_lane) in basis.iter().zip(base) {
                    buffer.extend_from_slice(&[row[0], row[1], row[2], *origin_lane]);
                }
                for local in LIGHT_POINTS[usize::from(*kind)] {
                    // origin + Transform3D(basis, base) * local, in single precision.
                    let head: [f32; 3] = std::array::from_fn(|i| {
                        let row = basis[i];
                        origin[i] + (row[0] * local[0] + row[1] * local[1] + row[2] * local[2] + base[i])
                    });
                    chunk.heads.push(head);
                    chunk.kinds.push(*kind);
                }
            }
            batches.kinds.push(*kind);
            batches.transforms.push(buffer);
        }
        batches.glow.reserve(chunk.heads.len() * 16);
        for (head, &kind) in chunk.heads.iter().zip(&chunk.kinds) {
            let local: [f32; 3] = std::array::from_fn(|i| head[i] - origin[i]);
            batches.glow.extend_from_slice(&[1.0, 0.0, 0.0, local[0], 0.0, 1.0, 0.0, local[1]]);
            batches.glow.extend_from_slice(&[0.0, 0.0, 1.0, local[2]]);
            batches.glow.extend_from_slice(&self.colors[usize::from(kind)]);
        }
        self.insert_heads(key, chunk.heads, chunk.kinds, far);
        batches
    }

    /// Installs a chunk's world-space heads and kinds directly (what `set_chunk` derives from its
    /// lights). For tests and the parity fixtures.
    pub(crate) fn insert_heads(
        &mut self,
        key: (i32, i32),
        heads: Vec<[f32; 3]>,
        kinds: Vec<u8>,
        far: &mut [u8],
    ) {
        self.remove_chunk(key, far);
        let order = self.next_order;
        self.next_order += 1;
        self.order.insert(order, key);
        self.chunks.insert(key, Chunk { order, heads, kinds });
        self.chunk_edited(key);
        self.splat_chunk(key, 1.0, far);
    }

    /// Takes a chunk's lamps out; false when it held none. `O(chunk heads)`.
    pub(crate) fn remove_chunk(&mut self, key: (i32, i32), far: &mut [u8]) -> bool {
        if !self.chunks.contains_key(&key) {
            return false;
        }
        self.splat_chunk(key, -1.0, far);
        let chunk = self.chunks.remove(&key).expect("present");
        self.order.remove(&chunk.order);
        self.chunk_edited(key);
        true
    }

    /// Takes every chunk out, in upload order (the far map's sums follow it).
    pub(crate) fn clear(&mut self, far: &mut [u8]) {
        while let Some((_, key)) = self.order.first_key_value() {
            let key = *key;
            self.remove_chunk(key, far);
        }
    }

    fn chunk_edited(&mut self, key: (i32, i32)) {
        self.pool_age_s = f64::INFINITY;
        if self.grid.is_none() {
            return;
        }
        if self.grid_edits.len() >= MAX_GRID_EDITS {
            self.grid = None;
            self.grid_edits.clear();
        } else {
            self.grid_edits.push(key);
        }
    }

    // Adds (1) or takes back out (-1) one chunk's heads, once the far map is built.
    fn splat_chunk(&mut self, key: (i32, i32), scale: f32, far: &mut [u8]) {
        if !self.far_built {
            return;
        }
        let Some(chunk) = self.chunks.get(&key) else {
            return;
        };
        for (head, &kind) in chunk.heads.iter().zip(&chunk.kinds) {
            let light = self.kind_light[usize::from(kind)];
            splat_head(far, *head, light, scale, self.far_origin, FAR_CELL_M, FAR_SIDE as i32);
        }
        self.far_upload = true;
    }

    /// A day frame: the pool ages and goes dark. True when the lights were on and must hide.
    pub(crate) fn rest(&mut self, delta_s: f64) -> bool {
        self.pool_age_s += delta_s;
        self.pool_camera.take().is_some()
    }

    /// A night frame from the camera at `eye` looking along `forward`: re-centres the lamp grid
    /// or re-packs its edited cells, rebuilds or re-uploads the far map (one texture a frame),
    /// and assigns the pool when due. Returns the `*_CHANGED` flags.
    pub(crate) fn update(
        &mut self,
        eye: [f32; 3],
        forward: [f32; 3],
        delta_s: f64,
        cells: &mut [u8],
        far: &mut [u8],
    ) -> i64 {
        self.pool_age_s += delta_s;
        self.packed_rows = 0;
        let mut flags = 0;
        let cell = [
            (f64::from(eye[0]) / f64::from(CELL_M)).floor() as i32,
            (f64::from(eye[2]) / f64::from(CELL_M)).floor() as i32,
        ];
        let recentre = self.grid.is_none_or(|grid| {
            let moved = [(cell[0] - grid.cell[0]).abs(), (cell[1] - grid.cell[1]).abs()];
            moved[0].max(moved[1]) > GRID_RECENTRE_CELLS
        });
        let grid_due = recentre || !self.grid_edits.is_empty();
        let focus = ground_focus(eye, forward);
        let far_rebuild = !self.far_built || {
            let dx = focus[0] - self.far_centre[0];
            let dz = focus[1] - self.far_centre[1];
            f64::from((dx * dx + dz * dz).sqrt()) > FAR_RECENTRE_M
        };
        let far_due = far_rebuild || self.far_upload;
        let far_first = far_due && (!grid_due || self.far_waited);
        if grid_due && !far_first {
            if recentre {
                self.recentre_grid(cell, cells);
                self.pool_age_s = f64::INFINITY;
            } else {
                self.pack_edits(cells);
            }
            flags |= CELLS_CHANGED;
            self.far_waited = far_due;
        } else if far_due {
            if far_rebuild {
                self.rebuild_far(focus, far);
            }
            self.far_upload = false;
            self.far_waited = false;
            flags |= FAR_CHANGED;
        }
        let pool_due = self.pool_age_s >= POOL_REFRESH_S
            || self.pool_camera.is_none_or(|camera| {
                let d: [f32; 3] = std::array::from_fn(|i| eye[i] - camera[i]);
                f64::from((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()) >= POOL_REFRESH_MOVE_M
            });
        if pool_due {
            self.pool_age_s = 0.0;
            self.pool_camera = Some(eye);
            self.assign_pool(eye);
            flags |= POOL_CHANGED;
        }
        flags
    }

    /// Re-centres the lamp grid on `cell` and packs every cell. `O(l log l + l k)`.
    pub(crate) fn recentre_grid(&mut self, cell: [i32; 2], cells: &mut [u8]) {
        let origin = [
            (cell[0] - SIDE as i32 / 2) as f32 * CELL_M,
            (cell[1] - SIDE as i32 / 2) as f32 * CELL_M,
        ];
        self.grid = Some(GridWindow { cell, origin });
        self.grid_edits.clear();
        self.pack_region(CellRect::WINDOW, cells);
    }

    // Re-packs the cells each edited chunk's lamps can reach.
    fn pack_edits(&mut self, cells: &mut [u8]) {
        let Some(grid) = self.grid else {
            return;
        };
        let mut edits = std::mem::take(&mut self.grid_edits);
        edits.sort_unstable();
        edits.dedup();
        let reach = f64::from(ARM_M) + f64::from(kind_reach(3)) + 1.0;
        for &(x, z) in &edits {
            let low = [
                f64::from(self.chunk_origin[0]) + f64::from(x) * f64::from(self.span_m),
                f64::from(self.chunk_origin[1]) + f64::from(z) * f64::from(self.span_m),
            ];
            let cell = |p: f64, o: f32| ((p - f64::from(o)) / f64::from(CELL_M)).floor() as i32;
            let span = f64::from(self.span_m);
            let region = CellRect {
                x0: cell(low[0] - reach, grid.origin[0]),
                z0: cell(low[1] - reach, grid.origin[1]),
                x1: cell(low[0] + span + reach, grid.origin[0]),
                z1: cell(low[1] + span + reach, grid.origin[1]),
            };
            if let Some(region) = region.intersect(CellRect::WINDOW) {
                self.pack_region(region, cells);
            }
        }
        edits.clear();
        self.grid_edits = edits;
    }

    // Packs the window cells of `region` from every lamp that can reach them, as
    // `_rebuild_grid` packed the whole window: the window's chunks and heads within 32 m of it,
    // in the stable world order.
    fn pack_region(&mut self, region: CellRect, cells: &mut [u8]) {
        let Some(grid) = self.grid else {
            return;
        };
        let origin = grid.origin;
        let side_m = SIDE as f32 * CELL_M;
        let end = [origin[0] + side_m, origin[1] + side_m];
        let window_keys = self.chunk_bounds(
            [origin[0] - 35.0, origin[1] - 35.0],
            [end[0] + 35.0, end[1] + 35.0],
        );
        let low = [
            origin[0] + region.x0 as f32 * CELL_M,
            origin[1] + region.z0 as f32 * CELL_M,
        ];
        let high = [
            origin[0] + (region.x1 + 1) as f32 * CELL_M,
            origin[1] + (region.z1 + 1) as f32 * CELL_M,
        ];
        let margin = 32.0 + ARM_M + 1.0;
        let region_keys = self.chunk_bounds(
            [low[0] - margin, low[1] - margin],
            [high[0] + margin, high[1] + margin],
        );
        self.rows.clear();
        if let Some(keys) = window_keys.intersect(region_keys) {
            let near = |p: f32, low: f32, high: f32| {
                f64::from(p) >= f64::from(low) - 32.0 && f64::from(p) <= f64::from(high) + 32.0
            };
            for z in keys.z0..=keys.z1 {
                for x in keys.x0..=keys.x1 {
                    let Some(chunk) = self.chunks.get(&(x, z)) else {
                        continue;
                    };
                    for (head, &kind) in chunk.heads.iter().zip(&chunk.kinds) {
                        if near(head[0], origin[0], end[0])
                            && near(head[2], origin[1], end[1])
                            && near(head[0], low[0], high[0])
                            && near(head[2], low[1], high[1])
                        {
                            self.rows.push(Row {
                                head: *head,
                                kind,
                                reach: kind_reach(usize::from(kind)),
                            });
                        }
                    }
                }
            }
        }
        self.rows.sort_unstable_by(row_order);
        clear_cells(cells, &mut self.counts, region);
        for row in &self.rows {
            pack_head(
                cells,
                &mut self.counts,
                row.head,
                f32::from(row.kind + 1),
                row.reach,
                origin,
                region,
            );
        }
        self.packed_rows += self.rows.len();
    }

    /// Rebuilds the far map around `focus`. `O(C + h)`.
    pub(crate) fn rebuild_far(&mut self, focus: [f32; 2], far: &mut [u8]) {
        let half = (FAR_SIDE / 2) as f32 * FAR_CELL_M;
        self.far_built = true;
        self.far_origin = [
            (focus[0] / FAR_CELL_M).floor() * FAR_CELL_M - half,
            (focus[1] / FAR_CELL_M).floor() * FAR_CELL_M - half,
        ];
        self.far_centre = [self.far_origin[0] + half, self.far_origin[1] + half];
        far.fill(0);
        // A head touches texels within one texel of it; poles own chunks, heads stand ARM_M off.
        let margin = FAR_CELL_M + ARM_M + 1.0;
        let side_m = FAR_SIDE as f32 * FAR_CELL_M;
        let keys = self.chunk_bounds(
            [self.far_origin[0] - margin, self.far_origin[1] - margin],
            [
                self.far_origin[0] + side_m + margin,
                self.far_origin[1] + side_m + margin,
            ],
        );
        for key in self.order.values() {
            if !keys.contains(key.0, key.1) {
                continue;
            }
            let chunk = &self.chunks[key];
            for (head, &kind) in chunk.heads.iter().zip(&chunk.kinds) {
                let light = self.kind_light[usize::from(kind)];
                splat_head(far, *head, light, 1.0, self.far_origin, FAR_CELL_M, FAR_SIDE as i32);
            }
            self.packed_rows += chunk.heads.len();
        }
        self.far_upload = true;
    }

    // The nearest POOL_SIZE heads within POOL_RADIUS_M of `eye`, and the handoff.
    fn assign_pool(&mut self, eye: [f32; 3]) {
        let reach = (POOL_RADIUS_M + f64::from(ARM_M)) as f32;
        let keys = self.chunk_bounds(
            [eye[0] - reach, eye[2] - reach],
            [eye[0] + reach, eye[2] + reach],
        );
        let radius_sq = POOL_RADIUS_M * POOL_RADIUS_M;
        self.candidates.clear();
        for z in keys.z0..=keys.z1 {
            for x in keys.x0..=keys.x1 {
                let Some(chunk) = self.chunks.get(&(x, z)) else {
                    continue;
                };
                for (head, &kind) in chunk.heads.iter().zip(&chunk.kinds) {
                    let d: [f32; 3] = std::array::from_fn(|i| eye[i] - head[i]);
                    let distance_sq = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                    if f64::from(distance_sq) < radius_sq {
                        self.candidates.push(Candidate {
                            distance_sq,
                            head: *head,
                            kind,
                        });
                    }
                }
            }
        }
        let mut handoff = POOL_RADIUS_M;
        if self.candidates.len() > POOL_SIZE {
            self.candidates
                .select_nth_unstable_by(POOL_SIZE - 1, candidate_order);
            handoff = f64::from(self.candidates[POOL_SIZE - 1].distance_sq).sqrt();
            self.candidates.truncate(POOL_SIZE);
        }
        self.candidates.sort_unstable_by(candidate_order);
        let fade = POOL_FADE_M.min(handoff * 0.5);
        self.pool.clear();
        self.pool.extend_from_slice(&[
            (handoff - fade) as f32,
            fade as f32,
            handoff as f32,
            self.candidates.len() as f32,
        ]);
        for candidate in &self.candidates {
            self.pool.extend_from_slice(&candidate.head);
            self.pool.push(f32::from(candidate.kind));
        }
    }
}

// Where the look ray meets the ground, at most FAR_FOCUS_MAX_M ahead (`_ground_focus`).
fn ground_focus(eye: [f32; 3], forward: [f32; 3]) -> [f32; 2] {
    let mut ahead = FAR_FOCUS_MAX_M;
    if f64::from(forward[1]) < -0.001 {
        ahead = (f64::from(eye[1]) / -f64::from(forward[1])).min(FAR_FOCUS_MAX_M);
    }
    [
        (f64::from(eye[0]) + f64::from(forward[0]) * ahead) as f32,
        (f64::from(eye[2]) + f64::from(forward[2]) * ahead) as f32,
    ]
}

fn color_array(colors: &PackedColorArray) -> Vec<[f32; 4]> {
    colors.as_slice().iter().map(|c| [c.r, c.g, c.b, c.a]).collect()
}

fn heads_of(heads: &PackedVector3Array) -> impl Iterator<Item = [f32; 3]> + '_ {
    heads.as_slice().iter().map(|h| [h.x, h.y, h.z])
}

/// The street lights' camera passes (`street_lights.gd`, `RENDER-32-H`): chunks in, the lamp
/// grid and far map out as texture bytes (RGBAF rows, ready for `Image.create_from_data`), the
/// pool as light positions. One per street light renderer; the textures' bytes stay shared with
/// Godot only while it uploads them.
#[derive(GodotClass)]
#[class(init, base = RefCounted)]
pub struct StreetLightBook {
    book: LampBook,
    cells: PackedByteArray,
    far: PackedByteArray,
}

impl StreetLightBook {
    fn buffers(&mut self) {
        if self.cells.len() != CELL_BYTES {
            self.cells.resize(CELL_BYTES);
            self.cells.as_mut_slice().fill(0);
        }
        if self.far.len() != FAR_BYTES {
            self.far.resize(FAR_BYTES);
            self.far.as_mut_slice().fill(0);
        }
    }
}

#[godot_api]
impl StreetLightBook {
    /// Lamp grid cells per side.
    #[constant]
    const SIDE: i64 = SIDE as i64;
    /// Lamp slots a cell.
    #[constant]
    const CAPACITY: i64 = CAPACITY as i64;
    /// Far map texels per side.
    #[constant]
    const FAR_SIDE: i64 = FAR_SIDE as i64;
    /// Real lights at most.
    #[constant]
    const POOL_SIZE: i64 = POOL_SIZE as i64;

    /// Sets the road chunk grid (`span_m`, `origin`); a change re-packs both textures.
    #[func]
    pub fn configure_chunks(&mut self, span_m: f32, origin: Vector2) {
        self.book.configure_chunks(span_m, [origin.x, origin.y]);
    }

    /// Sets the lamp colours per kind (sRGB, the glow's), their linear values (the far map's
    /// light) and which kinds have meshes (1 a byte), before any chunk.
    #[func]
    pub fn set_kinds(
        &mut self,
        colors: PackedColorArray,
        linear: PackedColorArray,
        loaded: PackedByteArray,
    ) {
        let loaded: Vec<bool> = loaded.as_slice().iter().map(|&b| b != 0).collect();
        self.book
            .set_kinds(&color_array(&colors), &color_array(&linear), &loaded);
    }

    /// Installs a road chunk's lights (`street_lights` of its payload, in its frame at `origin`),
    /// replacing what the chunk held. Returns `{kinds: PackedByteArray, transforms: Array of
    /// PackedFloat32Array, glow: PackedFloat32Array}`: one batch per 64 m cell and kind, 12
    /// floats an instance, and the glow batch, 16 floats an instance (transform and colour).
    /// `O(lights)`.
    #[func]
    pub fn set_chunk(
        &mut self,
        key: Vector2i,
        origin: Vector3,
        lights: PackedFloat32Array,
    ) -> VarDictionary {
        self.buffers();
        let batches = self.book.set_chunk(
            (key.x, key.y),
            [origin.x, origin.y, origin.z],
            lights.as_slice(),
            self.far.as_mut_slice(),
        );
        let mut transforms = VarArray::new();
        for buffer in &batches.transforms {
            transforms.push(&PackedFloat32Array::from(buffer.as_slice()).to_variant());
        }
        let mut result = VarDictionary::new();
        result.set("kinds", PackedByteArray::from(batches.kinds.as_slice()));
        result.set("transforms", transforms);
        result.set("glow", PackedFloat32Array::from(batches.glow.as_slice()));
        result
    }

    /// Takes a chunk's lights out; false when it held none. `O(chunk lights)`.
    #[func]
    pub fn remove_chunk(&mut self, key: Vector2i) -> bool {
        self.buffers();
        self.book.remove_chunk((key.x, key.y), self.far.as_mut_slice())
    }

    /// Takes every chunk out.
    #[func]
    pub fn clear(&mut self) {
        self.buffers();
        self.book.clear(self.far.as_mut_slice());
    }

    /// A day frame of `delta_s`: true when the pool's lights were on and must hide.
    #[func]
    pub fn rest(&mut self, delta_s: f64) -> bool {
        self.book.rest(delta_s)
    }

    /// A night frame of `delta_s` from the camera at `eye` looking along `forward`. Returns the
    /// flags: 1 the lamp grid changed, 2 the far map changed, 4 the pool was assigned.
    #[func]
    pub fn update(&mut self, eye: Vector3, forward: Vector3, delta_s: f64) -> i64 {
        self.buffers();
        self.book.update(
            [eye.x, eye.y, eye.z],
            [forward.x, forward.y, forward.z],
            delta_s,
            self.cells.as_mut_slice(),
            self.far.as_mut_slice(),
        )
    }

    /// The lamp grid: `SIDE * CAPACITY` x `SIDE` RGBAF texels, each slot a head's world position
    /// and its kind plus one (0 ends a cell's list).
    #[func]
    pub fn cell_data(&self) -> PackedByteArray {
        self.cells.clone()
    }

    /// The lamp grid's window for `street_lamp_window`: origin x, z, cell side, cells per side.
    #[func]
    pub fn cell_window(&self) -> Vector4 {
        let origin = self.book.grid.map_or([0.0, 0.0], |grid| grid.origin);
        Vector4::new(origin[0], origin[1], CELL_M, SIDE as f32)
    }

    /// The far map: `FAR_SIDE` x `FAR_SIDE` RGBAF texels of mean ground light.
    #[func]
    pub fn far_data(&self) -> PackedByteArray {
        self.far.clone()
    }

    /// The far map's window for `street_lamp_far_window`.
    #[func]
    pub fn far_window(&self) -> Vector4 {
        let origin = self.book.far_origin;
        Vector4::new(origin[0], origin[1], FAR_CELL_M, FAR_SIDE as f32)
    }

    /// The pool: fade begin, fade length, handoff distance, light count `n`, then `n` times head
    /// x, y, z and kind, nearest first.
    #[func]
    pub fn pool(&self) -> PackedFloat32Array {
        PackedFloat32Array::from(self.book.pool.as_slice())
    }

    /// Lamp heads packed into the grid or splatted into the far map by the last update.
    #[func]
    pub fn last_work(&self) -> i64 {
        self.book.packed_rows as i64
    }

    /// Rebuilds the far map around `focus` now. For tests.
    #[func]
    pub fn rebuild_far_at(&mut self, focus: Vector2) {
        self.buffers();
        self.book
            .rebuild_far([focus.x, focus.y], self.far.as_mut_slice());
    }

    /// Each kind's light range (head height plus the margin).
    #[func]
    pub fn kind_ranges() -> PackedFloat32Array {
        (0..KINDS).map(kind_reach).collect()
    }

    /// Packs `heads` (with their `kinds` and `ranges`, in priority order) into a lamp grid at
    /// `origin`, as the book packs a window. For tests.
    #[func]
    pub fn pack_cells(
        heads: PackedVector3Array,
        kinds: PackedByteArray,
        ranges: PackedFloat32Array,
        origin: Vector2,
    ) -> PackedFloat32Array {
        let mut bytes = vec![0u8; CELL_BYTES];
        let mut counts = vec![0u8; SIDE * SIDE];
        for ((head, &kind), &reach) in heads_of(&heads).zip(kinds.as_slice()).zip(ranges.as_slice()) {
            pack_head(
                &mut bytes,
                &mut counts,
                head,
                f32::from(kind) + 1.0,
                reach,
                [origin.x, origin.y],
                CellRect::WINDOW,
            );
        }
        bytes.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect()
    }

    /// Adds `heads`' light (`kind_light` per kind) times `scale` into `texels` (RGBA rows of
    /// `side`, cells of `cell_m` from `origin`) and returns them. For tests.
    #[func]
    #[allow(clippy::too_many_arguments)]
    pub fn splat_far(
        texels: PackedFloat32Array,
        heads: PackedVector3Array,
        kinds: PackedByteArray,
        kind_light: PackedColorArray,
        origin: Vector2,
        cell_m: f32,
        side: i64,
        scale: f32,
    ) -> PackedFloat32Array {
        let mut bytes: Vec<u8> = texels.as_slice().iter().flat_map(|v| v.to_le_bytes()).collect();
        let light = color_array(&kind_light);
        for (head, &kind) in heads_of(&heads).zip(kinds.as_slice()) {
            let [r, g, b, _] = light[usize::from(kind)];
            splat_head(&mut bytes, head, [r, g, b], scale, [origin.x, origin.y], cell_m, side as i32);
        }
        bytes.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect()
    }

    /// The ground flux of a lamp at `height` with range `reach` and cone `cone_cos`.
    #[func]
    pub fn ground_flux(height: f64, reach: f64, cone_cos: f64) -> f64 {
        ground_flux(height, reach, cone_cos)
    }
}

#[cfg(test)]
mod tests;
