// SPDX-License-Identifier: GPL-3.0-or-later

//! The backdrop's sampling job (`TERRAIN-41` D1), beside the terrain payload jobs: it keeps the
//! backdrop's product ([`backdrop`](crate::nodes::sim::render::stream::layers::backdrop)) equal
//! to the heightmap and the authored water.
//!
//! The terrain payload poll, which holds the core under `try_lock` every frame, publishes the
//! sources' key: the world epoch, the terrain's source generation and the water grid's address
//! ([`source_key`], `O(1)`). The streaming node compares it once a frame ([`check`]) with the key
//! the job last sampled; only when it moved does a job run, on the background pool, one at a time.
//! Nothing here adds a lock of the core to a frame. The job takes the core under `try_lock` (a busy core ends it; the next frame
//! asks again) for `O(storage chunks)` pointer copies of the terrain's source grid and one `Arc`
//! of the water's depth grid, and samples off the core:
//!
//! - **build** (the first job of a world, or a new layout): every block in parallel on the
//!   background pool, `O(blocks x 129^2)` reads;
//! - **refresh**: the storage chunks whose pointers diverged from the grids it last sampled
//!   (terrain and water apart), their lattice points sampled (`O(dirty chunks x 64)` reads) and
//!   compared with the product bitwise under the backdrop's lock, so a block's channel is marked
//!   for upload only where a value changed. A water re-solve that rewrites the whole grid costs
//!   one comparison pass on the worker, not an upload a block.
//!
//! The land cover reaches the product from the terrain payload workers and the land cover jobs
//! ([`note_cover`]): each baked coverage's 4 x 4 means.

use super::*;
use crate::nodes::sim::render::stream::layers::backdrop::{
    BackdropShared, COVER_STEP_CELLS, Layout, Sample, SourceKey, build_blocks, chunk_rects,
    sample_rects,
};
use crate::simulation::core::sparse_chunk_grid::SparseChunkGrid;
use std::time::Instant;

/// What the streaming node needs to keep the backdrop: the core, its turnstile, the shared
/// state and the background pool's tracker.
#[derive(Clone)]
pub(crate) struct BackdropSpawner {
    /// The simulation core the sources are read from.
    pub(crate) core: Arc<Mutex<SimCore>>,
    /// The core's turnstile: a thread waiting for the core is never kept waiting by the backdrop.
    pub(crate) turnstile: Arc<crate::nodes::sim::core::CoreTurnstile>,
    /// The backdrop's state.
    pub(crate) shared: Arc<BackdropShared>,
    /// The background pool's job tracker.
    pub(crate) async_jobs: Arc<super::async_jobs::AsyncJobs>,
}

impl SimulationNode {
    /// The backdrop's spawner for the streaming node (`TERRAIN-41` D1).
    pub(crate) fn backdrop_spawner(&self) -> BackdropSpawner {
        BackdropSpawner {
            core: Arc::clone(&self.core),
            turnstile: Arc::clone(&self.core_turnstile),
            shared: Arc::clone(&self.backdrop),
            async_jobs: Arc::clone(&self.async_jobs),
        }
    }
}

/// The key of the core's sources now. `O(1)`.
pub(crate) fn source_key(core: &SimCore) -> SourceKey {
    let terrain = &core.heightmap;
    SourceKey {
        epoch: core.world_epoch,
        terrain: terrain.source_generation(),
        water: core.watermap.baseline_depth_address(),
        layout: Layout::new(
            [terrain.render_patch_cols(), terrain.render_patch_rows()],
            terrain.render_patch_interval_cells(),
            [terrain.width, terrain.height],
        ),
    }
}

/// Once a frame on the main thread: starts the sampling job when the sources' key the terrain
/// payload poll published this frame differs from the key the job last sampled. `O(1)`: the
/// backdrop's lock under `try_lock`; no core lock (the poll read the key in its own hold).
pub(crate) fn check(spawner: &BackdropSpawner) {
    let Ok(mut state) = spawner.shared.state.try_lock() else {
        return;
    };
    let due = !state.running
        && state.published.is_some_and(|key| {
            key.layout.is_some() && state.sampled.key != Some(key)
        });
    if !due {
        return;
    }
    state.running = true;
    drop(state);
    let next = spawner.clone();
    spawner.async_jobs.spawn(move || run(&next));
}

// The sources a job read, with their key.
struct Taken {
    key: SourceKey,
    layout: Layout,
    terrain: SparseChunkGrid<f32>,
    water: Arc<SparseChunkGrid<f32>>,
}

// One sampling job: a build or a refresh, then the job's end.
fn run(spawner: &BackdropSpawner) {
    let world = spawner.shared.world();
    let taken = take(spawner);
    let mut state = spawner.shared.state.lock().unwrap();
    let Some(taken) = taken else {
        state.running = false;
        state.stats.busy += 1;
        return;
    };
    // A refresh needs this world's product on this layout and the sources it was sampled from.
    let current = state
        .product
        .as_ref()
        .is_some_and(|product| product.world == world && product.layout == taken.layout)
        && state
            .sampled
            .key
            .is_some_and(|key| key.epoch == taken.key.epoch);
    let previous = if current {
        let sampled = &mut state.sampled;
        sampled.terrain.take().zip(sampled.water.take())
    } else {
        None
    };
    // Sampled off the backdrop's lock, so the uploads and land cover notes never wait for it.
    drop(state);
    let start = Instant::now();
    match previous {
        None => {
            let blocks = crate::thread_pools::install_background(|| {
                build_blocks(&taken.terrain, &taken.water, &taken.layout)
            });
            let installed = spawner.shared.install_build(world, taken.layout, blocks);
            let mut state = spawner.shared.state.lock().unwrap();
            state.running = false;
            if installed {
                state.stats.builds += 1;
                state.stats.build_us = start.elapsed().as_secs_f64() * 1e6;
                remember(&mut state.sampled, taken);
            }
        }
        Some((old_terrain, old_water)) => {
            let (samples, chunks) = refresh_samples(&taken, &old_terrain, &old_water);
            let mut state = spawner.shared.state.lock().unwrap();
            state.running = false;
            let Some(product) = state
                .product
                .as_mut()
                .filter(|product| product.world == world && product.layout == taken.layout)
            else {
                return;
            };
            let changed = product.apply_samples(&samples);
            let stats = &mut state.stats;
            stats.refreshes += 1;
            stats.refresh_us = start.elapsed().as_secs_f64() * 1e6;
            stats.refresh_chunks = chunks;
            stats.refresh_points = samples.len();
            stats.refresh_changed = changed;
            remember(&mut state.sampled, taken);
        }
    }
}

// The core's sources under `try_lock`: `O(storage chunks)` pointer copies and one `Arc`.
fn take(spawner: &BackdropSpawner) -> Option<Taken> {
    if spawner.turnstile.has_waiter() {
        return None;
    }
    let core = SimulationNode::try_lock_shared_core(&spawner.core)?;
    let key = source_key(&core);
    let layout = key.layout?;
    Some(Taken {
        key,
        layout,
        terrain: core.heightmap.source_snapshot(),
        water: core.watermap.baseline_depth_snapshot(),
    })
}

fn remember(
    sampled: &mut crate::nodes::sim::render::stream::layers::backdrop::Sampled,
    taken: Taken,
) {
    sampled.key = Some(taken.key);
    sampled.terrain = Some(taken.terrain);
    sampled.water = Some(taken.water);
}

/// The lattice samples a refresh compares: the points of every storage chunk whose pointer
/// diverged from the sources last sampled, terrain and water apart, and the number of such
/// chunks. `O(stored chunks)` pointer comparisons plus `O(dirty chunks x 64)` reads.
fn refresh_samples(
    taken: &Taken,
    old_terrain: &SparseChunkGrid<f32>,
    old_water: &Arc<SparseChunkGrid<f32>>,
) -> (Vec<Sample>, usize) {
    let mut rects = Vec::new();
    let mut chunks = Vec::new();
    let mut count = 0;
    if taken.terrain.layout_matches(old_terrain) {
        taken
            .terrain
            .diverged_chunks_since(old_terrain, &mut chunks);
    }
    count += chunks.len();
    chunk_rects(
        &taken.layout,
        taken.terrain.chunk_size(),
        &chunks,
        &mut rects,
    );
    if !Arc::ptr_eq(&taken.water, old_water) && taken.water.layout_matches(old_water) {
        chunks.clear();
        taken.water.diverged_chunks_since(old_water, &mut chunks);
        count += chunks.len();
        chunk_rects(&taken.layout, taken.water.chunk_size(), &chunks, &mut rects);
    }
    let mut samples = Vec::new();
    sample_rects(
        &taken.terrain,
        &taken.water,
        &taken.layout,
        &rects,
        &mut samples,
    );
    (samples, count)
}

/// Notes a baked land cover of patch `key` in world `world` (read when its job was spawned):
/// its means over [`COVER_STEP_CELLS`] squares. `O(texels)` on the calling worker.
pub(crate) fn note_cover(
    shared: &BackdropShared,
    world: u64,
    key: Vector2i,
    cover: &super::vegetation_api::PatchLandCover,
) {
    let (Ok(x), Ok(z)) = (usize::try_from(key.x), usize::try_from(key.y)) else {
        return;
    };
    if let Some(values) = cover.coarse_means(COVER_STEP_CELLS) {
        shared.note_cover(world, [x, z], values);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::sim::core::CityTreasury;
    use crate::nodes::sim::render::stream::layers::backdrop::Product;
    use crate::simulation::core::config::WorldConfig;

    // A 10,240 m world at 8 m (20 x 20 patches, 2 x 2 blocks) with rolling ground and a pond.
    fn world() -> SimCore {
        let config =
            WorldConfig::new(10_240.0, 10_240.0, 10_240.0, 10.0).with_terrain_resolution(8.0);
        let mut core = SimCore {
            treasury: CityTreasury::new(0.0),
            benchmark_mode: true,
            ..SimCore::new(config)
        };
        let (width, height) = (core.heightmap.width, core.heightmap.height);
        let dense: Vec<f32> = (0..width * height)
            .map(|i| {
                100.0
                    + ((i % width) as f32 * 0.013).sin() * 9.0
                    + ((i / width) as f32 * 0.021).cos() * 7.0
            })
            .collect();
        core.heightmap.replace_source_from_dense(&dense).unwrap();
        let pond: Vec<(usize, f32)> = (200..260)
            .flat_map(|z| (100..180).map(move |x| (z * width + x, 2.5)))
            .collect();
        core.watermap.write_baseline_depth_samples(&pond);
        core
    }

    fn take(core: &SimCore) -> Taken {
        let key = source_key(core);
        Taken {
            key,
            layout: key.layout.unwrap(),
            terrain: core.heightmap.source_snapshot(),
            water: core.watermap.baseline_depth_snapshot(),
        }
    }

    fn build(taken: &Taken) -> Product {
        Product::new(
            taken.layout,
            0,
            build_blocks(&taken.terrain, &taken.water, &taken.layout),
        )
    }

    // The live core's copy-on-write: a sculpt and an in-place water write leave the held sources'
    // chunks behind, so a refresh reads exactly their chunks and lands on the full rebuild.
    #[test]
    fn a_sculpt_and_a_water_write_refresh_only_their_blocks_to_the_full_rebuild() {
        let mut core = world();
        let first = take(&core);
        let mut product = build(&first);
        assert_eq!(source_key(&core), first.key, "nothing moved: no job");

        // A sculpt in block 3 (samples beyond 1,024 on both axes); the water's address stays.
        core.sculpt_terrain_internal(Vector2::new(4_000.0, 3_500.0), 120.0, 6.0);
        let second = take(&core);
        assert_ne!(second.key.terrain, first.key.terrain);
        assert_eq!(second.key.water, first.key.water);
        let (samples, chunks) = refresh_samples(&second, &first.terrain, &first.water);
        assert!(
            chunks > 0 && chunks < 16,
            "the brush's chunks only: {chunks}"
        );
        let before = product.blocks.clone();
        assert!(product.apply_samples(&samples) > 0);
        assert_eq!(product.blocks, build(&second).blocks);
        assert_eq!(product.blocks[..3], before[..3], "blocks 0-2 untouched");
        assert_ne!(product.blocks[3].height, before[3].height);

        // An in-place water write in block 0 (the pond; `TERRAIN-38`'s path).
        let width = core.heightmap.width;
        core.watermap
            .write_baseline_depth_samples(&[(208 * width + 104, 4.0), (216 * width + 120, 0.0)]);
        let third = take(&core);
        assert_ne!(
            third.key.water, second.key.water,
            "a water write moves its address"
        );
        let (samples, _) = refresh_samples(&third, &second.terrain, &second.water);
        let before = product.blocks.clone();
        assert_eq!(product.apply_samples(&samples), 2);
        assert_eq!(product.blocks, build(&third).blocks);
        assert_ne!(product.blocks[0].water, before[0].water);
        assert_eq!(product.blocks[0].height, before[0].height);
        assert_eq!(product.blocks[1..], before[1..], "no other block changed");
    }
}
