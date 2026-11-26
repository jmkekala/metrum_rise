// SPDX-License-Identifier: GPL-3.0-or-later

//! The backdrop's data (`TERRAIN-41` round D1, `docs/terrain.md` "The backdrop"): the whole map
//! on the 64 m lattice, the base terrain patches' detail step 8, in blocks of
//! [`BLOCK_PATCHES`] x [`BLOCK_PATCHES`] patches, three channels a block:
//!
//! - **ground height**, `f32`, one value a lattice point: the base heightmap's sample there, bit
//!   for bit (the value a step-8 patch draws at that vertex);
//! - **water surface**, `f32`, the same lattice: ground plus the authored water depth where the
//!   depth exceeds [`WET_DEPTH_M`] (the water shader's wet threshold), else the ground itself, so
//!   "above the ground" is exactly "wet";
//! - **land cover**, `u8`, one value a [`COVER_STEP_CELLS`] square of the patches' 8 m coverage
//!   texels (32 m): their mean crown coverage, from each patch's payload as it lands
//!   ([`Product::note_cover`]); open ground (0) until then.
//!
//! A block's height and water textures are `side x side` lattice points with `side =
//! BLOCK_PATCHES x 8 + 1`, shared edges included (129 x 129); its land cover is `cover_side x
//! cover_side` with `cover_side = BLOCK_PATCHES x 16 + 1` (257 x 257): the block's own 256
//! cell-centred values a row and one more, the next block's first, so a filter across the block
//! edge reads the same values on both sides. Lattice points beyond the world's last sample (the
//! last blocks of a map that is not a whole number of blocks) hold the edge sample's value.
//!
//! Round D2 draws it ([`cells`]): every patch of the map the terrain does not draw as a patch
//! draws a cell of its block's textures with the terrain shader's backdrop variant
//! (`terrain_backdrop.gdshader`). The product is built by a background job at the
//! load and refreshed after terrain and water edits (the job,
//! `simulation_node/async_backdrop.rs`); its blocks are uploaded by [`BackdropLayer`] through the
//! stream backend, one texture a unit under the frame's install ledger: a block's texture is
//! created once and then replaced whole (`texture_2d_replace`), never written in place (round
//! B1b: a texture written after its creation costs every draw that binds it).
//!
//! The numbers that are really look choices are one constant each, for round D2 and the user:
//! [`BLOCK_PATCHES`], [`HEIGHT_STEP_CELLS`], [`COVER_STEP_CELLS`], the channels' formats
//! ([`CHANNEL_FORMATS`]) and what the water and land cover channels hold.

use crate::nodes::sim::render::stream::backend::{Handle, RenderBackend, TextureFormat};
use crate::nodes::sim::render::stream::layers::terrain::{PatchGrid, PatchKey};
use crate::nodes::sim::render::stream::schedule::{Clock, FrameBudget, predicted_us};
use crate::simulation::core::sparse_chunk_grid::SparseChunkGrid;
use rayon::prelude::*;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Patches along each side of a block (8,192 m at 512 m patches). A look and memory choice for
/// round D2: one texture set a block, so an edit rewrites one block's textures.
pub(crate) const BLOCK_PATCHES: usize = 16;
/// Terrain cells between two lattice points: the patches' detail step 8 (64 m at 8 m cells), so
/// a backdrop vertex is a step-8 vertex. For round D2.
pub(crate) const HEIGHT_STEP_CELLS: usize = 8;
/// Coverage texels (8 m) along each side of one land cover value (32 m). For round D2.
pub(crate) const COVER_STEP_CELLS: usize = 4;
/// The authored water depth above which a lattice point is wet: the water shader's threshold
/// (`water.gdshader`, `depth > 0.001`).
pub(crate) const WET_DEPTH_M: f32 = 0.001;

/// A block's channels, in upload order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Channel {
    /// Ground height, metres.
    Height,
    /// Water surface height, metres (the ground where dry).
    Water,
    /// Land cover, 0-255 crown coverage.
    Cover,
}

/// The channels in upload order.
pub(crate) const CHANNELS: [Channel; 3] = [Channel::Height, Channel::Water, Channel::Cover];
/// Each channel's texture format, by [`Channel`] order. For round D2 (precision).
pub(crate) const CHANNEL_FORMATS: [TextureFormat; 3] =
    [TextureFormat::R32F, TextureFormat::R32F, TextureFormat::R8];

/// The map as the backdrop sees it: the render patch grid and the sample grid it lies on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Layout {
    /// Render patches across X and Z.
    pub(crate) patches: [usize; 2],
    /// Terrain cells a patch spans along each axis (64 at 8 m).
    pub(crate) interval: usize,
    /// Height samples across X and Z.
    pub(crate) samples: [usize; 2],
}

impl Layout {
    /// The layout of a map, or `None` where the backdrop does not apply (no map, or a patch
    /// interval that is not a whole number of lattice steps and coverage values).
    pub(crate) fn new(patches: [usize; 2], interval: usize, samples: [usize; 2]) -> Option<Self> {
        let usable = patches[0] > 0
            && patches[1] > 0
            && samples[0] > 1
            && samples[1] > 1
            && interval > 0
            && interval.is_multiple_of(HEIGHT_STEP_CELLS)
            && interval.is_multiple_of(COVER_STEP_CELLS);
        usable.then_some(Self {
            patches,
            interval,
            samples,
        })
    }

    /// Blocks across X and Z.
    pub(crate) fn blocks(&self) -> [usize; 2] {
        [
            self.patches[0].div_ceil(BLOCK_PATCHES),
            self.patches[1].div_ceil(BLOCK_PATCHES),
        ]
    }

    /// Blocks in all.
    pub(crate) fn block_count(&self) -> usize {
        let [x, z] = self.blocks();
        x * z
    }

    /// Lattice intervals a patch spans (8).
    pub(crate) fn patch_lattice(&self) -> usize {
        self.interval / HEIGHT_STEP_CELLS
    }

    /// Land cover values a patch holds along each axis (16).
    pub(crate) fn patch_cover(&self) -> usize {
        self.interval / COVER_STEP_CELLS
    }

    /// Lattice points along a block's side, shared edges included (129).
    pub(crate) fn height_side(&self) -> usize {
        BLOCK_PATCHES * self.patch_lattice() + 1
    }

    /// Land cover values along a block's side, the next block's first included (257).
    pub(crate) fn cover_side(&self) -> usize {
        BLOCK_PATCHES * self.patch_cover() + 1
    }

    /// Texels of `channel` along a block's side.
    pub(crate) fn side(&self, channel: Channel) -> usize {
        match channel {
            Channel::Cover => self.cover_side(),
            _ => self.height_side(),
        }
    }

    /// Bytes of one block's `channel`.
    pub(crate) fn channel_bytes(&self, channel: Channel) -> usize {
        let side = self.side(channel);
        side * side * CHANNEL_FORMATS[channel as usize].texel_bytes()
    }

    /// Bytes of one block, all channels.
    pub(crate) fn block_bytes(&self) -> usize {
        CHANNELS.iter().map(|&c| self.channel_bytes(c)).sum()
    }

    // The last lattice index inside the world along each axis.
    fn lattice_last(&self) -> [usize; 2] {
        [
            (self.samples[0] - 1) / HEIGHT_STEP_CELLS,
            (self.samples[1] - 1) / HEIGHT_STEP_CELLS,
        ]
    }

    // The last lattice index of the last block along each axis.
    fn lattice_end(&self) -> [usize; 2] {
        let [x, z] = self.blocks();
        let per_block = BLOCK_PATCHES * self.patch_lattice();
        [x * per_block, z * per_block]
    }

    // The sample a lattice point reads: its own, or the world's edge beyond it.
    fn sample_of(&self, lattice: [usize; 2]) -> [usize; 2] {
        [
            (lattice[0] * HEIGHT_STEP_CELLS).min(self.samples[0] - 1),
            (lattice[1] * HEIGHT_STEP_CELLS).min(self.samples[1] - 1),
        ]
    }
}

/// One block's channels, row-major, `Z` rows of `X` values.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Block {
    /// Ground height at the lattice points.
    pub(crate) height: Vec<f32>,
    /// Water surface height at the lattice points.
    pub(crate) water: Vec<f32>,
    /// Land cover values.
    pub(crate) cover: Vec<u8>,
}

/// The ground and water surface a lattice point holds. `O(1)`: two sparse grid reads.
pub(crate) fn sample_point(
    terrain: &SparseChunkGrid<f32>,
    water: &SparseChunkGrid<f32>,
    layout: &Layout,
    lattice: [usize; 2],
) -> [f32; 2] {
    let [x, z] = layout.sample_of(lattice);
    let ground = terrain.get(x, z);
    let depth = water.get(x, z);
    let surface = if depth > WET_DEPTH_M {
        ground + depth
    } else {
        ground
    };
    [ground, surface]
}

/// Builds block `index` (row-major in [`Layout::blocks`]) from the heightmap and the water,
/// open ground for land cover. `O(side^2)` grid reads.
pub(crate) fn build_block(
    terrain: &SparseChunkGrid<f32>,
    water: &SparseChunkGrid<f32>,
    layout: &Layout,
    index: usize,
) -> Block {
    let side = layout.height_side();
    let per_block = BLOCK_PATCHES * layout.patch_lattice();
    let [blocks_x, _] = layout.blocks();
    let origin = [
        (index % blocks_x) * per_block,
        (index / blocks_x) * per_block,
    ];
    let mut height = Vec::with_capacity(side * side);
    let mut surface = Vec::with_capacity(side * side);
    for row in 0..side {
        for column in 0..side {
            let [ground, top] = sample_point(
                terrain,
                water,
                layout,
                [origin[0] + column, origin[1] + row],
            );
            height.push(ground);
            surface.push(top);
        }
    }
    let cover_side = layout.cover_side();
    Block {
        height,
        water: surface,
        cover: vec![0; cover_side * cover_side],
    }
}

/// Builds every block, in parallel on the background pool (inline on the caller's pool when it
/// is a Rayon worker), in block order whatever the workers. `O(blocks x side^2)` grid reads.
pub(crate) fn build_blocks(
    terrain: &SparseChunkGrid<f32>,
    water: &SparseChunkGrid<f32>,
    layout: &Layout,
) -> Vec<Block> {
    crate::thread_pools::install_background(|| {
        (0..layout.block_count())
            .into_par_iter()
            .map(|index| build_block(terrain, water, layout, index))
            .collect()
    })
}

/// One lattice point's samples, for a refresh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Sample {
    /// The lattice point.
    pub(crate) lattice: [u32; 2],
    /// Ground and water surface.
    pub(crate) values: [f32; 2],
}

/// Appends the lattice rectangles (`[x0, x1, z0, z1]`, inclusive) whose samples may have changed
/// when the storage chunks `chunks` (of `chunk_size` cells) diverged: the lattice points inside
/// each chunk, and where a chunk holds the world's last sample along an axis, every lattice point
/// beyond it to the end of the last block (they repeat the edge). `O(chunks)`.
pub(crate) fn chunk_rects(
    layout: &Layout,
    chunk_size: usize,
    chunks: &[(usize, usize)],
    out: &mut Vec<[usize; 4]>,
) {
    let last = layout.lattice_last();
    let end = layout.lattice_end();
    let range = |chunk: usize, axis: usize| -> Option<(usize, usize)> {
        let first_sample = chunk * chunk_size;
        let last_sample = (first_sample + chunk_size - 1).min(layout.samples[axis] - 1);
        if first_sample > last_sample {
            return None;
        }
        let low = first_sample.div_ceil(HEIGHT_STEP_CELLS);
        let mut high = last_sample / HEIGHT_STEP_CELLS;
        if high >= last[axis] && last_sample == layout.samples[axis] - 1 {
            high = end[axis];
        }
        (low <= high).then_some((low, high))
    };
    for &(chunk_x, chunk_z) in chunks {
        if let (Some((x0, x1)), Some((z0, z1))) = (range(chunk_x, 0), range(chunk_z, 1)) {
            out.push([x0, x1, z0, z1]);
        }
    }
}

/// Samples every lattice point of `rects` (a point in two rectangles twice). `O(points)` grid
/// reads.
pub(crate) fn sample_rects(
    terrain: &SparseChunkGrid<f32>,
    water: &SparseChunkGrid<f32>,
    layout: &Layout,
    rects: &[[usize; 4]],
    out: &mut Vec<Sample>,
) {
    for &[x0, x1, z0, z1] in rects {
        for z in z0..=z1 {
            for x in x0..=x1 {
                out.push(Sample {
                    lattice: [x as u32, z as u32],
                    values: sample_point(terrain, water, layout, [x, z]),
                });
            }
        }
    }
}

/// The backdrop's whole product for one world: the blocks and which of their channels changed
/// since they were last uploaded.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Product {
    /// The map it was built for.
    pub(crate) layout: Layout,
    /// The world serial it was built in ([`BackdropShared::world`]).
    pub(crate) world: u64,
    /// The blocks, row-major.
    pub(crate) blocks: Vec<Block>,
    // Per block, a bit a channel ([`Channel`] order) changed since its last upload.
    pending: Vec<u8>,
}

impl Product {
    /// A product of `blocks` with every channel waiting for its upload.
    pub(crate) fn new(layout: Layout, world: u64, blocks: Vec<Block>) -> Self {
        let pending = vec![0b111; blocks.len()];
        Self {
            layout,
            world,
            blocks,
            pending,
        }
    }

    /// The bytes the blocks hold (the system-memory copy; the textures hold the same again).
    pub(crate) fn bytes(&self) -> usize {
        self.blocks
            .iter()
            .map(|b| b.height.len() * 4 + b.water.len() * 4 + b.cover.len())
            .sum()
    }

    /// Whether any channel waits for its upload. `O(blocks)`.
    pub(crate) fn has_pending(&self) -> bool {
        self.pending.iter().any(|&bits| bits != 0)
    }

    /// Channels waiting for their upload. `O(blocks)`.
    pub(crate) fn pending_count(&self) -> usize {
        self.pending
            .iter()
            .map(|bits| bits.count_ones() as usize)
            .sum()
    }

    /// Whether block `block`'s `channel` waits for its upload.
    pub(crate) fn is_pending(&self, block: usize, channel: Channel) -> bool {
        self.pending[block] & (1 << channel as u8) != 0
    }

    fn mark(&mut self, block: usize, channel: Channel) {
        self.pending[block] |= 1 << channel as u8;
    }

    fn unmark(&mut self, block: usize, channel: Channel) {
        self.pending[block] &= !(1 << channel as u8);
    }

    /// Writes refreshed samples into every block that holds their lattice point (a point on a
    /// block edge lies in two or four) and marks a block's channel for upload only where a value
    /// changed (bitwise). Returns the points that changed. `O(samples)`.
    pub(crate) fn apply_samples(&mut self, samples: &[Sample]) -> usize {
        let per_block = BLOCK_PATCHES * self.layout.patch_lattice();
        let side = self.layout.height_side();
        let [blocks_x, blocks_z] = self.layout.blocks();
        let mut changed = 0;
        for sample in samples {
            let [x, z] = sample.lattice.map(|v| v as usize);
            let mut point_changed = false;
            for block_z in blocks_holding(z, per_block, blocks_z) {
                for block_x in blocks_holding(x, per_block, blocks_x) {
                    let index = block_z * blocks_x + block_x;
                    let at = (z - block_z * per_block) * side + (x - block_x * per_block);
                    let block = &mut self.blocks[index];
                    let [ground, surface] = sample.values;
                    let height_changed = block.height[at].to_bits() != ground.to_bits();
                    let water_changed = block.water[at].to_bits() != surface.to_bits();
                    block.height[at] = ground;
                    block.water[at] = surface;
                    if height_changed {
                        self.mark(index, Channel::Height);
                    }
                    if water_changed {
                        self.mark(index, Channel::Water);
                    }
                    point_changed |= height_changed || water_changed;
                }
            }
            changed += usize::from(point_changed);
        }
        changed
    }

    /// Writes patch `patch`'s land cover (`values`, [`Layout::patch_cover`] squared, row-major)
    /// into its block and into the overlap row or column of the blocks before it, and marks the
    /// blocks whose values changed. Returns whether any did. `O(values)`.
    pub(crate) fn note_cover(&mut self, patch: [usize; 2], values: &[u8]) -> bool {
        let per_patch = self.layout.patch_cover();
        if patch[0] >= self.layout.patches[0]
            || patch[1] >= self.layout.patches[1]
            || values.len() != per_patch * per_patch
        {
            return false;
        }
        let side = self.layout.cover_side();
        let per_block = BLOCK_PATCHES * per_patch;
        let [blocks_x, _] = self.layout.blocks();
        let first = [patch[0] * per_patch, patch[1] * per_patch];
        let mut any = false;
        for row in 0..per_patch {
            let z = first[1] + row;
            for column in 0..per_patch {
                let x = first[0] + column;
                let value = values[row * per_patch + column];
                // The cover point's own block, and the blocks whose overlap row or column it is.
                for block_z in overlap_blocks(z, per_block) {
                    for block_x in overlap_blocks(x, per_block) {
                        let index = block_z * blocks_x + block_x;
                        let at = (z - block_z * per_block) * side + (x - block_x * per_block);
                        let slot = &mut self.blocks[index].cover[at];
                        if *slot != value {
                            *slot = value;
                            self.mark(index, Channel::Cover);
                            any = true;
                        }
                    }
                }
            }
        }
        any
    }

    /// The bytes of block `block`'s `channel` as its texture holds them (little-endian floats).
    pub(crate) fn write_channel(&self, block: usize, channel: Channel, out: &mut [u8]) {
        let block = &self.blocks[block];
        match channel {
            Channel::Cover => out.copy_from_slice(&block.cover),
            Channel::Height | Channel::Water => {
                let values = if channel == Channel::Height {
                    &block.height
                } else {
                    &block.water
                };
                for (bytes, value) in out.as_chunks_mut::<4>().0.iter_mut().zip(values) {
                    *bytes = value.to_le_bytes();
                }
            }
        }
    }
}

// The blocks whose height lattice holds lattice index `index` along one axis: its own, and the
// one before where it lies on their shared edge (never past the last block).
fn blocks_holding(
    index: usize,
    per_block: usize,
    blocks: usize,
) -> std::ops::RangeInclusive<usize> {
    let own = (index / per_block).min(blocks - 1);
    let first = if index.is_multiple_of(per_block) && index > 0 {
        index / per_block - 1
    } else {
        own
    };
    first..=own
}

// The blocks whose land cover holds cover index `index` along one axis: its own block, and the
// one before when it is that block's overlap (its block's first).
fn overlap_blocks(index: usize, per_block: usize) -> std::ops::RangeInclusive<usize> {
    let own = index / per_block;
    let first = if index.is_multiple_of(per_block) && own > 0 {
        own - 1
    } else {
        own
    };
    first..=own
}

/// The key a refresh is due on: the world, the terrain's source generation and the water's
/// depth grid (the backdrop's held copy of it is shared until the water's next write, so its
/// address changes with every water change).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourceKey {
    /// `SimCore::world_epoch`.
    pub(crate) epoch: u64,
    /// `TerrainSystem::source_generation`.
    pub(crate) terrain: u64,
    /// The water depth grid's address.
    pub(crate) water: usize,
    /// The map's layout.
    pub(crate) layout: Option<Layout>,
}

/// What the sampling job keeps between runs: the key and the sources it last sampled.
#[derive(Default)]
pub(crate) struct Sampled {
    /// The key of the sources below.
    pub(crate) key: Option<SourceKey>,
    /// The terrain's source grid as last sampled (its chunks shared with the live grid).
    pub(crate) terrain: Option<SparseChunkGrid<f32>>,
    /// The water's depth grid as last sampled.
    pub(crate) water: Option<Arc<SparseChunkGrid<f32>>>,
}

/// Counters for the debug view and the measurements.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Stats {
    /// Full builds since the node started.
    pub(crate) builds: u64,
    /// The last full build's time on its worker, microseconds.
    pub(crate) build_us: f64,
    /// Refreshes (jobs that compared samples) since the node started.
    pub(crate) refreshes: u64,
    /// The last refresh's time on its worker, microseconds (from the snapshot to the write).
    pub(crate) refresh_us: f64,
    /// The last refresh's chunks diverged and lattice points compared and changed.
    pub(crate) refresh_chunks: usize,
    /// Lattice points compared by the last refresh.
    pub(crate) refresh_points: usize,
    /// Lattice points changed by the last refresh.
    pub(crate) refresh_changed: usize,
    /// Jobs that found the core busy.
    pub(crate) busy: u64,
    /// Land cover notes taken (written or equal).
    pub(crate) covers: u64,
}

/// The backdrop's state shared by the sampling job, the payload workers (land cover) and the
/// streaming node (uploads).
#[derive(Default)]
pub(crate) struct BackdropState {
    /// The product of the current world, once built.
    pub(crate) product: Option<Product>,
    /// Land cover noted before the product of its world was built: `(world, patch, values)`.
    pub(crate) early_covers: Vec<(u64, [usize; 2], Vec<u8>)>,
    /// The sampling job's memory.
    pub(crate) sampled: Sampled,
    /// Whether a sampling job is queued or running.
    pub(crate) running: bool,
    /// The sources' key as the terrain payload poll last read it.
    pub(crate) published: Option<SourceKey>,
    /// Counters.
    pub(crate) stats: Stats,
}

/// The shared handle: the state and the world serial (moved by every world change).
#[derive(Default)]
pub(crate) struct BackdropShared {
    /// The state.
    pub(crate) state: Mutex<BackdropState>,
    /// The world serial: a product, a job's result or a land cover note of another serial is
    /// stale.
    pub(crate) world: AtomicU64,
}

impl BackdropShared {
    /// The current world serial.
    pub(crate) fn world(&self) -> u64 {
        self.world.load(Ordering::Acquire)
    }

    /// A world change: the product, the land cover noted early and the job's memory are dropped;
    /// a job still running finds its serial stale and drops its result. `O(1)` beyond the drops.
    pub(crate) fn reset_world(&self) {
        self.world.fetch_add(1, Ordering::AcqRel);
        let mut state = self.state.lock().unwrap();
        state.product = None;
        state.early_covers.clear();
        state.sampled = Sampled::default();
    }

    /// Publishes the sources' key read under the core (the terrain payload poll's hold, once a
    /// frame); a busy state skips this frame's. `O(1)`.
    pub(crate) fn publish_key(&self, key: SourceKey) {
        if let Ok(mut state) = self.state.try_lock() {
            state.published = Some(key);
        }
    }

    /// Notes patch `patch`'s land cover from a payload of world `world`: written into the
    /// product, or kept until the product of that world is built. A note of an older world is
    /// dropped. `O(values)` under the state's lock.
    pub(crate) fn note_cover(&self, world: u64, patch: [usize; 2], values: Vec<u8>) {
        if world != self.world() {
            return;
        }
        let mut state = self.state.lock().unwrap();
        state.stats.covers += 1;
        match state.product.as_mut() {
            Some(product) if product.world == world => {
                product.note_cover(patch, &values);
            }
            _ => state.early_covers.push((world, patch, values)),
        }
    }

    /// Installs a full build of world `world` (when it is still the current world), with the
    /// land cover noted early. Returns whether it was installed.
    pub(crate) fn install_build(&self, world: u64, layout: Layout, blocks: Vec<Block>) -> bool {
        let mut state = self.state.lock().unwrap();
        if world != self.world() {
            return false;
        }
        let mut product = Product::new(layout, world, blocks);
        for (noted, patch, values) in std::mem::take(&mut state.early_covers) {
            if noted == world {
                product.note_cover(patch, &values);
            }
        }
        state.product = Some(product);
        true
    }
}

/// What draws the cells (`terrain.gd` hands it over at a world's setup): the cell's mesh and the
/// textures' parameter names (catalogue indices), and one material a block of the terrain
/// shader's backdrop variant, owned by the script.
struct Drawing {
    mesh: u32,
    // The catalogue index of the first channel's parameter; the others follow in [`Channel`]
    // order.
    parameter: u32,
    materials: Vec<Handle>,
}

/// The backdrop's textures on the main thread: one a block and channel, created on its first
/// upload and replaced whole after (`O(1)` a unit). Uploads run as units of the streaming node's
/// pass, each predicted from its bytes, only in the room the frame's ledger has left (never as
/// a frame's exempt first unit: nothing waits for them yet), or all at once while the load holds
/// the frame.
#[derive(Default)]
pub(crate) struct BackdropLayer {
    // The textures by block, [`Channel`] order, and the world and layout they belong to.
    textures: Vec<[Option<Handle>; 3]>,
    owner: Option<(u64, Layout)>,
    // The cells drawn with them (`TERRAIN-41` D2) and what draws them.
    cells: cells::Cells,
    drawing: Option<Drawing>,
    /// Texture uploads since the node started (creates and replaces).
    pub(crate) uploads: u64,
    /// Bytes uploaded since the node started.
    pub(crate) uploaded_bytes: u64,
    /// The textures' bytes as created (the video memory the product asks for).
    pub(crate) texture_bytes: u64,
}

impl BackdropLayer {
    /// The texture of block `block`'s `channel`, if uploaded (tests; round D2 binds them).
    #[cfg(test)]
    pub(crate) fn texture(&self, block: usize, channel: Channel) -> Option<Handle> {
        self.textures
            .get(block)
            .and_then(|set| set[channel as usize])
    }

    /// Frees every texture (the cells stop drawing until their blocks' textures exist again).
    pub(crate) fn clear(&mut self, backend: &mut impl RenderBackend) {
        for (block, set) in self.textures.drain(..).enumerate() {
            let bound = self.drawing.as_ref().and_then(|drawing| {
                drawing.materials.get(block).map(|&material| (material, drawing.parameter))
            });
            for (channel, texture) in set.into_iter().enumerate() {
                let Some(texture) = texture else {
                    continue;
                };
                // Unbound first: the material never names a freed texture.
                if let Some((material, parameter)) = bound {
                    backend.material_set_texture(material, parameter + channel as u32, None);
                }
                backend.free(texture);
            }
        }
        self.owner = None;
        self.texture_bytes = 0;
        self.cells.touch_all();
    }

    /// Starts the cells over on a map's patch grid (`StreamNode.reset_terrain_patches`: nothing
    /// resident, so every patch of the map wants its cell), freeing the old cells' objects.
    /// `O(patches)`, once a world.
    pub(crate) fn reset_cells(&mut self, grid: Option<PatchGrid>, backend: Option<&mut impl RenderBackend>) {
        if let Some(backend) = backend {
            self.cells.free_objects(backend);
        }
        self.cells.reset(grid);
    }

    /// Frees the cells' objects (the node leaves the tree).
    pub(crate) fn free_cells(&mut self, backend: &mut impl RenderBackend) {
        self.cells.free_objects(backend);
    }

    /// Hands over what draws the cells: the cell's `mesh` and the first texture parameter
    /// `parameter` (catalogue indices; the channels' parameters follow in [`Channel`] order) and
    /// one material a block, row-major as the product's blocks. The textures that exist are
    /// bound to them; the cells are drawn again with them at the next flush. `O(blocks)`.
    pub(crate) fn set_drawing(
        &mut self,
        mesh: u32,
        parameter: u32,
        materials: Vec<Handle>,
        backend: &mut impl RenderBackend,
    ) {
        // The objects keep their mesh: made again with the new one.
        self.cells.free_objects(backend);
        for (block, set) in self.textures.iter().enumerate() {
            let Some(&material) = materials.get(block) else {
                continue;
            };
            for (channel, texture) in set.iter().enumerate() {
                if texture.is_some() {
                    backend.material_set_texture(material, parameter + channel as u32, *texture);
                }
            }
        }
        self.drawing = Some(Drawing {
            mesh,
            parameter,
            materials,
        });
    }

    /// Patch `key` became resident or left (`O(1)`): its cell goes (unless the patch draws
    /// nothing) or comes at the next flush.
    pub(crate) fn set_resident(&mut self, key: PatchKey, resident: bool) {
        self.cells.set_resident(key, resident);
    }

    /// Resident patch `key` draws nothing (`blank`, the graded book's: its cell comes) or draws
    /// again (its cell goes) at the next flush (`TERRAIN-41` D3). `O(1)`.
    pub(crate) fn set_blank(&mut self, key: PatchKey, blank: bool) {
        self.cells.set_blank(key, blank);
    }

    /// Whether patches can be held while their water does not draw (`TERRAIN-58`: a water layer
    /// and a graded book). `O(patches)` on a change.
    pub(crate) fn set_holds_on(&mut self, on: bool) {
        self.cells.set_holds_on(on);
    }

    /// The water layer showed (with its answer) or hid patch `key`'s water (`TERRAIN-58`). `O(1)`.
    pub(crate) fn set_water_shown(&mut self, key: PatchKey, shown: bool, state: Option<WaterState>) {
        self.cells.set_water_shown(key, shown, state);
    }

    /// The water's report of patch `key`'s water (`TERRAIN-58`). `O(1)`.
    pub(crate) fn set_water_state(&mut self, key: PatchKey, state: WaterState) {
        self.cells.set_water_state(key, state);
    }

    /// The water forgot patch `key`'s node (`TERRAIN-58`). `O(1)`.
    pub(crate) fn forget_water(&mut self, key: PatchKey) {
        self.cells.forget_water(key);
    }

    /// The water started over (`TERRAIN-58`). `O(patches)`.
    pub(crate) fn forget_all_water(&mut self) {
        self.cells.forget_all_water();
    }

    /// The holds changed since the last call (`TERRAIN-58`), for the graded book. `O(news)`.
    pub(crate) fn take_hold_news(&mut self) -> std::vec::Drain<'_, (PatchKey, bool)> {
        self.cells.take_hold_news()
    }

    /// Patches held now, their own draw waiting for their water (`TERRAIN-58`). `O(1)`.
    pub(crate) fn held_count(&self) -> usize {
        self.cells.held_count()
    }

    /// Whether patch `key`'s own draw is held for its water (`TERRAIN-58`). `O(1)`.
    pub(crate) fn is_held(&self, key: PatchKey) -> bool {
        self.cells.is_held(key)
    }

    /// Whether patch `key`'s water would hold its ground back while it is resident (`UI-15`:
    /// the loading screen's water unit). `O(1)`.
    pub(crate) fn water_waits(&self, key: PatchKey) -> bool {
        self.cells.water_waits(key)
    }

    /// Patch `key`'s water as last reported: `(dry, holding a mesh)` (the kept meshes' debug
    /// cross-check, `TERRAIN-63`). `O(1)`.
    #[cfg(debug_assertions)]
    pub(crate) fn water_bits(&self, key: PatchKey) -> (bool, bool) {
        self.cells.water_bits(key)
    }

    /// Whether patch `key` is drawn as a patch: resident, drawing something and not held for its water. `O(1)`.
    pub(crate) fn patch_draws(&self, key: PatchKey) -> bool {
        self.cells.patch_draws(key)
    }

    /// Whether patch `key` draws a cell (as last flushed). `O(1)`.
    pub(crate) fn cell_shown(&self, key: PatchKey) -> bool {
        self.cells.is_shown(key)
    }

    /// Cells listed, cells drawn and blocks drawing. `O(blocks)`.
    pub(crate) fn cell_counts(&self) -> [usize; 3] {
        [
            self.cells.listed(),
            self.cells.shown_count(),
            self.cells.shown_blocks(),
        ]
    }

    /// Sends the cells' changed blocks (both of the stream node's passes end with it, so a change
    /// of residency and its cell reach the same frame). A block draws once its three textures
    /// exist for the cells' map and it has a material; `heights` bound the cells' ground and
    /// skirts. `O(changed blocks x 256)`; nothing on a frame without a change.
    pub(crate) fn flush(&mut self, backend: &mut impl RenderBackend, heights: [f32; 2]) {
        let textures = &self.textures;
        let drawing = self.drawing.as_ref();
        let same_map = match (self.owner, self.cells.grid()) {
            (Some((_, layout)), Some(grid)) => {
                layout.patches == [grid.cols as usize, grid.rows as usize]
            }
            _ => false,
        };
        let material = |block: usize| {
            let ready = same_map
                && textures
                    .get(block)
                    .is_some_and(|set| set.iter().all(Option::is_some));
            ready
                .then(|| drawing.and_then(|drawing| drawing.materials.get(block).copied()))
                .flatten()
        };
        self.cells
            .flush(backend, drawing.map(|drawing| drawing.mesh), material, heights);
    }

    /// Follows `product`: frees the textures of another world or layout, then uploads the
    /// channels that wait, in block and channel order, while `budget` admits them (all of them
    /// when `unbudgeted`). Returns the units run. `O(blocks)` to find the waiting channels plus
    /// `O(bytes)` a unit; a frame with nothing waiting reads one bit a block.
    pub(crate) fn upload(
        &mut self,
        product: Option<&mut Product>,
        backend: &mut impl RenderBackend,
        budget: &mut FrameBudget,
        clock: &impl Clock,
        unbudgeted: bool,
    ) -> usize {
        let Some(product) = product else {
            if self.owner.is_some() {
                self.clear(backend);
            }
            return 0;
        };
        let owner = (product.world, product.layout);
        if self.owner != Some(owner) {
            self.clear(backend);
            self.textures = vec![[None; 3]; product.blocks.len()];
            self.owner = Some(owner);
        }
        let mut units = 0;
        for block in 0..product.blocks.len() {
            if product.pending[block] == 0 {
                continue;
            }
            for channel in CHANNELS {
                if !product.is_pending(block, channel) {
                    continue;
                }
                let bytes = product.layout.channel_bytes(channel);
                let predicted = predicted_us(2, bytes);
                if !unbudgeted && !budget.fits(predicted) {
                    return units;
                }
                let start = clock.now_us();
                let side = product.layout.side(channel) as u32;
                let format = CHANNEL_FORMATS[channel as usize];
                let mut fill = |out: &mut [u8]| product.write_channel(block, channel, out);
                match self.textures[block][channel as usize] {
                    Some(texture) => {
                        backend.texture_2d_replace(texture, [side, side], format, &mut fill)
                    }
                    None => {
                        let texture = backend.texture_2d_create([side, side], format, &mut fill);
                        self.textures[block][channel as usize] = Some(texture);
                        self.texture_bytes += bytes as u64;
                        if let Some(drawing) = self.drawing.as_ref()
                            && let Some(&material) = drawing.materials.get(block)
                        {
                            let parameter = drawing.parameter + channel as u32;
                            backend.material_set_texture(material, parameter, Some(texture));
                        }
                        // The block may draw its cells now.
                        self.cells.touch(block);
                    }
                }
                product.unmark(block, channel);
                budget.record(predicted, clock.now_us() - start);
                self.uploads += 1;
                self.uploaded_bytes += bytes as u64;
                units += 1;
            }
        }
        units
    }
}

mod cells;

pub(crate) use cells::WaterState;

#[cfg(test)]
mod tests;
