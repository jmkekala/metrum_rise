// SPDX-License-Identifier: GPL-2.0-only

//! Residency inputs of the graded terrain (`TERRAIN-07-D`): which tiles of a render patch render
//! graded, from the same inputs as the bake, and a baked tile packed into a page's slot layer and
//! row atlas run for the Godot upload. Godot owns residency; nothing here is kept between calls.

use super::grade::{CAP_M, TIE_IN_SLOPE};
use super::sample::BaseWindow;
use super::seams::{Seam, SeamScratch, gather_road_seams};
use super::sites::{NO_SITE, SiteBuffer, SiteSource};
use super::tile::{BakeError, Level, TILE_M, TileKey, TilePayload, grown, meets};
use crate::simulation::network::roadbed::{PieceId, RoadbedSystem};
use crate::simulation::terrain::TerrainSystem;

// Growth of a tile's rectangle before the road top triangle test, as coverage pads its
// triangle boxes: a larger value only lists more.
const TOUCH_PAD_M: f64 = 0.01;
// Window growth beyond the cap: gathering more seams lists no fewer tiles.
const WINDOW_PAD_M: f64 = 1.0;
// Growth of a cone reach for the rounding of the bilinear base and the seam heights, as the
// bake's pass bounds are grown: it can only list more.
const REACH_RELATIVE: f64 = 1e-9;
const REACH_ABSOLUTE_M: f64 = 1e-6;

/// Reused buffers of [`graded_tiles_in_rect`].
#[derive(Default)]
pub struct PatchTilesScratch {
    seams: SeamScratch,
    window: Vec<Seam>,
    base: BaseWindow,
    pieces: Vec<PieceId>,
    sites: SiteBuffer,
    // Per candidate tile, rows along +z: its rectangle clipped to the query, its base range and
    // whether it renders graded.
    tiles: Vec<Candidate>,
}

#[derive(Clone, Copy)]
struct Candidate {
    key: TileKey,
    rect: [f64; 4],
    base: (f64, f64),
    graded: bool,
}

/// Sets `out` to the tiles whose interior meets `rect` (a render patch, world XZ
/// `[min_x, min_z, max_x, max_z]`) and whose rendering inside `rect` can differ from the base
/// heightmap at some level, sorted by [`TileKey`].
///
/// The test is exact in the safe direction, over the inputs a bake reads (the base, the
/// roadbed's grounded seams and top triangles, the sites within the cap) and with no reach
/// parameter: `g = max(L, min(b, U))` differs from the base only where some seam's cone
/// excludes it, so a tile is listed when a road or pad seam within the cap comes within
/// `max(h_hi - b_lo, b_hi - h_lo) / k` of the tile's part in `rect` (any seam meeting it
/// does), when a grounded top triangle meets it (coverage and the mask's discard), or when a
/// site footprint or surface meets it (pad and material classes). A tile left out has
/// `delta = 0`, no covered sample, no class and no seam inside, so its cells render the base.
/// `O(P + S log S + S x tiles within the cap of each seam + T)`.
pub fn graded_tiles_in_rect(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    sites: &impl SiteSource,
    rect: [f64; 4],
    scratch: &mut PatchTilesScratch,
    out: &mut Vec<TileKey>,
) -> Result<(), BakeError> {
    out.clear();
    if !sites.ready() {
        return Err(BakeError::SitesUnavailable);
    }
    let first = TileKey::at(rect[0], rect[1]);
    let last = [
        (rect[2] / TILE_M).ceil() as i32 - 1,
        (rect[3] / TILE_M).ceil() as i32 - 1,
    ];
    if last[0] < first.x || last[1] < first.z {
        return Ok(());
    }
    let window = grown(rect, CAP_M + WINDOW_PAD_M);
    gather_road_seams(roadbed, window, &mut scratch.seams, &mut scratch.window);
    scratch.sites.gather(sites, window);
    scratch.sites.push_pad_seams(&mut scratch.window);
    scratch.base.fill(terrain, rect);
    let columns = (last[0] - first.x + 1) as usize;
    scratch.tiles.clear();
    for z in first.z..=last[1] {
        for x in first.x..=last[0] {
            let key = TileKey { x, z };
            let [ox, oz] = key.origin();
            let clipped = [
                ox.max(rect[0]),
                oz.max(rect[1]),
                (ox + TILE_M).min(rect[2]),
                (oz + TILE_M).min(rect[3]),
            ];
            scratch.tiles.push(Candidate {
                key,
                rect: clipped,
                base: scratch.base.range(clipped),
                graded: false,
            });
        }
    }
    for seam in &scratch.window {
        mark_cones(seam, first, columns, &mut scratch.tiles);
    }
    let edges = scratch.sites.edges();
    for tile in scratch.tiles.iter_mut().filter(|tile| !tile.graded) {
        tile.graded = edges.iter().any(|&[ax, az, bx, bz]| {
            meets([ax.min(bx), az.min(bz), ax.max(bx), az.max(bz)], tile.rect)
        });
    }
    for tile in scratch.tiles.iter_mut().filter(|tile| !tile.graded) {
        // No footprint side or surface edge meets the tile, so each polygon holds all of it or
        // none: its centre decides.
        let centre = [
            0.5 * (tile.rect[0] + tile.rect[2]),
            0.5 * (tile.rect[1] + tile.rect[3]),
        ];
        tile.graded = scratch.sites.pad_at(centre[0], centre[1]) != NO_SITE
            || scratch.sites.material_at(centre[0], centre[1]).is_some();
    }
    for tile in scratch.tiles.iter_mut().filter(|tile| !tile.graded) {
        let mut covered = false;
        roadbed.for_each_grounded_top_triangle_in_bounds(
            grown(tile.rect, TOUCH_PAD_M),
            &mut scratch.pieces,
            |_| covered = true,
        );
        tile.graded = covered;
    }
    out.extend(
        scratch
            .tiles
            .iter()
            .filter(|tile| tile.graded)
            .map(|tile| tile.key),
    );
    out.sort_unstable();
    Ok(())
}

// Marks the candidate tiles within the cap of `seam` whose base its cone can exclude.
fn mark_cones(seam: &Seam, first: TileKey, columns: usize, tiles: &mut [Candidate]) {
    let rows = tiles.len() / columns.max(1);
    let reach_box = grown(seam.box_xz(), CAP_M);
    let range = |lo: f64, hi: f64, origin: i32, count: usize| {
        let a = ((lo / TILE_M).floor() as i64 - i64::from(origin)).max(0);
        let b = ((hi / TILE_M).floor() as i64 - i64::from(origin)).min(count as i64 - 1);
        (a, b)
    };
    let (x0, x1) = range(reach_box[0], reach_box[2], first.x, columns);
    let (z0, z1) = range(reach_box[1], reach_box[3], first.z, rows);
    let (h_lo, h_hi) = seam.height_range();
    for j in z0..=z1 {
        for i in x0..=x1 {
            let tile = &mut tiles[j as usize * columns + i as usize];
            if tile.graded {
                continue;
            }
            let distance = box_distance(seam.box_xz(), tile.rect);
            if distance > CAP_M {
                continue;
            }
            let (b_lo, b_hi) = tile.base;
            let reach = (h_hi - b_lo).max(b_hi - h_lo) / TIE_IN_SLOPE;
            tile.graded = distance <= reach + reach.abs() * REACH_RELATIVE + REACH_ABSOLUTE_M;
        }
    }
}

// Distance between two XZ boxes, 0 where they meet.
fn box_distance(a: [f64; 4], b: [f64; 4]) -> f64 {
    let dx = (a[0] - b[2]).max(b[0] - a[2]).max(0.0);
    let dz = (a[1] - b[3]).max(b[1] - a[3]).max(0.0);
    (dx * dx + dz * dz).sqrt()
}

/// Lattice points per side of a level's slot layer, apron included: 131, 67 and 35 at
/// `GL0`-`GL2`; at `GL3` the most any tile's lattice holds over a base of `cell_m` cells: the
/// two sides, the step lines strictly between them (at most `ceil(64 m / spacing)`) and one
/// apron line beyond each side.
pub fn slot_side(level: Level, cell_m: f64) -> usize {
    match level {
        Level::Gl3 { .. } => (TILE_M / level.spacing_m(cell_m)).ceil() as usize + 4,
        _ => (TILE_M / level.spacing_m(cell_m)) as usize + 3,
    }
}

/// Texels of one row atlas layer (`TERRAIN-07-G` round 8, docs/terrain.md "Pages and the row
/// atlas"): a tile's rows take a run of whole layers of this many RGBAF texels by one, row `i`
/// at texel `i % ATLAS_WIDTH` of the run's layer `i / ATLAS_WIDTH`. A power of two, so the shader
/// addresses a row with a mask and a shift.
pub const ATLAS_WIDTH: usize = 256;

/// The packed seam channel of a texel that is the lower corner of a square with extra
/// candidates (T48 (a)): its named row (`-1` for none) plus 2048 times one more than its
/// square's list's first texel (relative to the lists). Rows stay below 2048 and lists below
/// 8191 texels, so the value is an integer below 2^24, exact in `f32`; the shader takes
/// `list = (v + 1) >> 11` and `row = v - 2048 list`.
pub fn seam_channel(row: f32, first_texel: u32) -> f32 {
    row + 2048.0 * (first_texel + 1) as f32
}

/// Packs a baked tile for upload into a page (docs/terrain.md "Pages and the row atlas"), as
/// little-endian `f32` RGBA.
///
/// `texels` is one slot layer of `side x (side + 1)` texels (`side` from [`slot_side`]), rows
/// along +z: the tile's lattice in the lower corner (the rest zero; a square's lower corner texel
/// with extra candidates carries its list in the seam channel, [`seam_channel`]), then the header
/// line, whose first texel is `(0, edges, heights, lists)`: the first row of the material edges,
/// of the seams' heights and of the extra candidate lists. Godot writes the first atlas layer of
/// the tile's run into the header's first channel before the upload.
///
/// `rows` is the tile's run: the seams from row 0, then the edges, the heights `(a, b, 0, 0)` and
/// the squares' extra candidate lists, each right after the last, zero-padded to whole layers of
/// [`ATLAS_WIDTH`] texels (empty when the tile has no rows). The buffers are resized and
/// overwritten. `O(side^2 + rows)`.
pub fn pack_slot(payload: &TilePayload, side: usize, texels: &mut Vec<u8>, rows: &mut Vec<u8>) {
    const TEXEL_BYTES: usize = 16;
    texels.clear();
    texels.resize(side * (side + 1) * TEXEL_BYTES, 0);
    let [width, depth] = payload.samples;
    for j in 0..depth.min(side) {
        for i in 0..width.min(side) {
            let at = (j * side + i) * TEXEL_BYTES;
            put_texel(
                &mut texels[at..at + TEXEL_BYTES],
                payload.texels[j * width + i],
            );
        }
    }
    for &(point, first) in &payload.square_extras {
        let (i, j) = (point as usize % width, point as usize / width);
        let at = (j * side + i) * TEXEL_BYTES + 8;
        let channel = seam_channel(payload.texels[point as usize][2], first);
        texels[at..at + 4].copy_from_slice(&channel.to_le_bytes());
    }
    let edges_at = payload.seams.len();
    let heights_at = edges_at + payload.edges.len();
    let extras_at = heights_at + payload.seam_heights.len();
    let count = extras_at + payload.extras.len();
    let header = side * side * TEXEL_BYTES;
    put_texel(
        &mut texels[header..header + TEXEL_BYTES],
        [0.0, edges_at as f32, heights_at as f32, extras_at as f32],
    );
    rows.clear();
    rows.resize(count.div_ceil(ATLAS_WIDTH) * ATLAS_WIDTH * TEXEL_BYTES, 0);
    let heights = payload.seam_heights.iter().map(|&[a, b]| [a, b, 0.0, 0.0]);
    let row_texels = payload
        .seams
        .iter()
        .copied()
        .chain(payload.edges.iter().copied())
        .chain(heights)
        .chain(payload.extras.iter().copied());
    for (index, texel) in row_texels.enumerate() {
        let at = index * TEXEL_BYTES;
        put_texel(&mut rows[at..at + TEXEL_BYTES], texel);
    }
}

/// Bytes of one packed wall face: eight little-endian `f32`.
pub const WALL_FACE_BYTES: usize = 32;

/// Packs a baked tile's wall faces for the Godot wall instances, [`WALL_FACE_BYTES`] each as
/// little-endian `f32`: `a.x, a.z, b.x, b.z` (tile-local metres), `bottom` at `a` and `b`, `top`
/// at `a` and `b` (world heights), in the payload's order. The buffer is overwritten.
/// `O(faces)`.
pub fn pack_walls(payload: &TilePayload, out: &mut Vec<u8>) {
    out.clear();
    out.reserve(payload.walls.len() * WALL_FACE_BYTES);
    for face in &payload.walls {
        let values = [
            face.a[0],
            face.a[1],
            face.b[0],
            face.b[1],
            face.bottom[0],
            face.bottom[1],
            face.top[0],
            face.top[1],
        ];
        for value in values {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
}

fn put_texel(out: &mut [u8], texel: [f32; 4]) {
    for (channel, value) in texel.into_iter().enumerate() {
        out[channel * 4..channel * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
}
