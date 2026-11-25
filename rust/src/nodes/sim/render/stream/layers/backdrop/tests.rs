// SPDX-License-Identifier: GPL-3.0-or-later

//! The backdrop's product (`TERRAIN-41` D1): bitwise samples, worker and order independence,
//! refreshes equal to rebuilds and local to their blocks, uploads through the recorder.

use super::*;
use crate::nodes::sim::render::stream::backend::recorder::{Command, Recorder};
use crate::nodes::sim::render::stream::schedule::MonotonicClock;

// A small map on the real lattice rules with patches of 16 cells (2 lattice intervals, 4 cover
// values a patch), so a block is 256 x 256 samples. 20 x 18 patches (321 x 289 samples): 2 x 2
// blocks, the last ones partly beyond the world.
const INTERVAL: usize = 16;
const PATCHES: [usize; 2] = [20, 18];

fn layout() -> Layout {
    Layout::new(
        PATCHES,
        INTERVAL,
        [PATCHES[0] * INTERVAL + 1, PATCHES[1] * INTERVAL + 1],
    )
    .unwrap()
}

// A deterministic value per sample: no two lattice points alike.
fn height_at(x: usize, z: usize) -> f32 {
    let hash = (x as u32).wrapping_mul(0x9e37_79b1) ^ (z as u32).wrapping_mul(0x85eb_ca77);
    50.0 + (hash % 100_000) as f32 * 0.001_3
}

fn terrain() -> SparseChunkGrid<f32> {
    let [w, h] = layout().samples;
    let mut grid = SparseChunkGrid::new(w, h, INTERVAL, 0.0);
    for z in 0..h {
        for x in 0..w {
            grid.set(x, z, height_at(x, z));
        }
    }
    grid
}

// Two lakes, one in the first block and one in the last, as rectangles of depth.
fn lakes(depths: [f32; 2]) -> Arc<SparseChunkGrid<f32>> {
    let [w, h] = layout().samples;
    let mut grid = SparseChunkGrid::new(w, h, INTERVAL, 0.0);
    for z in 40..90 {
        for x in 30..120 {
            grid.set(x, z, depths[0]);
        }
    }
    for z in 262..280 {
        for x in 270..310 {
            grid.set(x, z, depths[1]);
        }
    }
    Arc::new(grid)
}

fn product(terrain: &SparseChunkGrid<f32>, water: &SparseChunkGrid<f32>) -> Product {
    let layout = layout();
    Product::new(layout, 1, build_blocks(terrain, water, &layout))
}

// The refresh the sampling job runs: the diverged chunks of both grids, their lattice points
// sampled and written.
fn refresh(
    product: &mut Product,
    old: (&SparseChunkGrid<f32>, &SparseChunkGrid<f32>),
    new: (&SparseChunkGrid<f32>, &SparseChunkGrid<f32>),
) -> usize {
    let mut chunks = Vec::new();
    new.0.diverged_chunks_since(old.0, &mut chunks);
    new.1.diverged_chunks_since(old.1, &mut chunks);
    let mut rects = Vec::new();
    chunk_rects(&product.layout, INTERVAL, &chunks, &mut rects);
    let mut samples = Vec::new();
    sample_rects(new.0, new.1, &product.layout, &rects, &mut samples);
    product.apply_samples(&samples)
}

fn clear_pending(product: &mut Product) {
    product.pending.fill(0);
}

fn pending_blocks(product: &Product, channel: Channel) -> Vec<usize> {
    (0..product.blocks.len())
        .filter(|&block| product.is_pending(block, channel))
        .collect()
}

#[test]
fn every_sample_is_the_heightmaps_value_bit_for_bit() {
    let (terrain, water) = (terrain(), lakes([3.25, 0.0005]));
    let layout = layout();
    let product = product(&terrain, &water);
    let per_block = BLOCK_PATCHES * layout.patch_lattice();
    let side = layout.height_side();
    let [blocks_x, _] = layout.blocks();
    let [w, h] = layout.samples;
    let mut wet = 0;
    for (index, block) in product.blocks.iter().enumerate() {
        for row in 0..side {
            for column in 0..side {
                let lattice = [
                    (index % blocks_x) * per_block + column,
                    (index / blocks_x) * per_block + row,
                ];
                // Beyond the world a point repeats the edge sample.
                let x = (lattice[0] * HEIGHT_STEP_CELLS).min(w - 1);
                let z = (lattice[1] * HEIGHT_STEP_CELLS).min(h - 1);
                let ground = block.height[row * side + column];
                assert_eq!(ground.to_bits(), terrain.get(x, z).to_bits(), "{lattice:?}");
                let depth = water.get(x, z);
                let surface = block.water[row * side + column];
                if depth > WET_DEPTH_M {
                    wet += 1;
                    assert_eq!(surface.to_bits(), (ground + depth).to_bits());
                } else {
                    assert_eq!(surface.to_bits(), ground.to_bits(), "dry is the ground");
                }
            }
        }
    }
    assert!(wet > 0, "the deep lake is sampled");
}

#[test]
fn the_product_is_the_same_for_one_and_eight_workers() {
    let (terrain, water) = (terrain(), lakes([2.0, 4.0]));
    let run = |threads| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .expect("test pool")
            .install(|| product(&terrain, &water))
    };
    assert_eq!(run(1), run(8));
}

// A patch's cover values: distinct per patch and point.
fn cover_of(patch: [usize; 2], per_patch: usize) -> Vec<u8> {
    (0..per_patch * per_patch)
        .map(|i| ((patch[0] * 31 + patch[1] * 17 + i * 7) % 251) as u8)
        .collect()
}

#[test]
fn land_cover_in_any_order_gives_one_product_and_the_overlap_repeats_the_next_block() {
    let (terrain, water) = (terrain(), lakes([2.0, 4.0]));
    let layout = layout();
    let per_patch = layout.patch_cover();
    let patches: Vec<[usize; 2]> = (0..PATCHES[1])
        .flat_map(|z| (0..PATCHES[0]).map(move |x| [x, z]))
        .collect();
    let mut forward = product(&terrain, &water);
    for &patch in &patches {
        forward.note_cover(patch, &cover_of(patch, per_patch));
    }
    let mut shuffled = product(&terrain, &water);
    // A fixed permutation: a stride coprime with the count.
    for i in 0..patches.len() {
        let patch = patches[(i * 97) % patches.len()];
        shuffled.note_cover(patch, &cover_of(patch, per_patch));
    }
    assert_eq!(forward, shuffled);
    // The overlap column of block 0 is block 1's first column, row for row.
    let side = layout.cover_side();
    let last = side - 1;
    for row in 0..side {
        assert_eq!(
            forward.blocks[0].cover[row * side + last],
            forward.blocks[1].cover[row * side],
            "row {row}"
        );
    }
    // A patch's own values sit at its place.
    let patch = [3, 2];
    let values = cover_of(patch, per_patch);
    for row in 0..per_patch {
        for column in 0..per_patch {
            let at = (patch[1] * per_patch + row) * side + patch[0] * per_patch + column;
            assert_eq!(
                forward.blocks[0].cover[at],
                values[row * per_patch + column]
            );
        }
    }
    // A note equal to what is there changes nothing.
    clear_pending(&mut forward);
    assert!(!forward.note_cover(patch, &values));
    assert!(!forward.has_pending());
}

#[test]
fn a_brush_edit_changes_only_its_chunks_blocks_and_equals_a_full_rebuild() {
    let (old_terrain, water) = (terrain(), lakes([2.0, 4.0]));
    let mut product = product(&old_terrain, &water);
    clear_pending(&mut product);
    let mut terrain = old_terrain.clone();
    // A brush in block 3 over the second lake: lattice and off-lattice samples.
    for z in 262..285 {
        for x in 270..310 {
            terrain.set(x, z, terrain.get(x, z) + 1.5);
        }
    }
    let changed = refresh(&mut product, (&old_terrain, &water), (&terrain, &water));
    assert!(changed > 0);
    assert_eq!(pending_blocks(&product, Channel::Height), vec![3]);
    assert_eq!(
        pending_blocks(&product, Channel::Water),
        vec![3],
        "the lake rides the ground"
    );
    assert!(pending_blocks(&product, Channel::Cover).is_empty());
    let rebuilt = self::product(&terrain, &water);
    assert_eq!(product.blocks, rebuilt.blocks);

    // An edit of samples between lattice points only: compared, nothing changes.
    clear_pending(&mut product);
    let before = terrain.clone();
    terrain.set(17, 17, 999.0);
    terrain.set(23, 9, 999.0);
    assert_eq!(
        refresh(&mut product, (&before, &water), (&terrain, &water)),
        0
    );
    assert!(!product.has_pending());

    // On the block edge (x = 32 lattice intervals = sample 256): both blocks change.
    clear_pending(&mut product);
    let before = terrain.clone();
    terrain.set(256, 40, -7.0);
    refresh(&mut product, (&before, &water), (&terrain, &water));
    assert_eq!(pending_blocks(&product, Channel::Height), vec![0, 1]);
    assert_eq!(product.blocks, self::product(&terrain, &water).blocks);

    // The world's last sample: the points beyond the world repeat it.
    clear_pending(&mut product);
    let before = terrain.clone();
    let [w, h] = layout().samples;
    terrain.set(w - 1, h - 1, 1234.5);
    refresh(&mut product, (&before, &water), (&terrain, &water));
    assert_eq!(product.blocks, self::product(&terrain, &water).blocks);
    assert_eq!(pending_blocks(&product, Channel::Height), vec![3]);
}

#[test]
fn a_water_change_in_one_lake_dirties_only_its_blocks() {
    let terrain = terrain();
    let water = lakes([2.0, 4.0]);
    let mut product = product(&terrain, &water);
    clear_pending(&mut product);
    // A whole re-solve: every chunk of the grid is new storage, one lake's level moved.
    let resolved = lakes([2.0, 4.5]);
    let mut chunks = Vec::new();
    resolved.diverged_chunks_since(&water, &mut chunks);
    assert!(
        chunks.len() > 2,
        "every stored chunk diverged: {}",
        chunks.len()
    );
    refresh(&mut product, (&terrain, &water), (&terrain, &resolved));
    assert_eq!(pending_blocks(&product, Channel::Water), vec![3]);
    assert!(pending_blocks(&product, Channel::Height).is_empty());
    assert_eq!(product.blocks, self::product(&terrain, &resolved).blocks);
    // The same water again: nothing.
    clear_pending(&mut product);
    refresh(
        &mut product,
        (&terrain, &resolved),
        (&terrain, &lakes([2.0, 4.5])),
    );
    assert!(!product.has_pending());
}

#[test]
fn bytes_follow_the_plans_table() {
    // The shipped 18,432 m map and the 59,904 m test map: 8 m cells, 512 m patches.
    let shipped = Layout::new([36, 36], 64, [2305, 2305]).unwrap();
    let large = Layout::new([117, 117], 64, [14_977, 14_977]).unwrap();
    assert_eq!(shipped.height_side(), 129);
    assert_eq!(shipped.cover_side(), 257);
    assert_eq!(shipped.channel_bytes(Channel::Height), 66_564);
    assert_eq!(shipped.channel_bytes(Channel::Cover), 66_049);
    assert_eq!(shipped.block_bytes(), 199_177);
    assert_eq!(shipped.block_count(), 9);
    assert_eq!(shipped.block_count() * shipped.block_bytes(), 1_792_593);
    assert_eq!(large.block_count(), 64);
    assert_eq!(large.block_count() * large.block_bytes(), 12_747_328);
    // A built product holds exactly its blocks' bytes.
    let product = product(&terrain(), &lakes([1.0, 1.0]));
    assert_eq!(
        product.bytes(),
        product.blocks.len() * layout().block_bytes()
    );
}

#[test]
fn the_layer_uploads_each_channel_once_then_replaces_only_what_changed() {
    let (old_terrain, water) = (terrain(), lakes([2.0, 4.0]));
    let mut product = product(&old_terrain, &water);
    let mut backend = Recorder::new();
    let mut layer = BackdropLayer::default();
    let clock = MonotonicClock::default();
    // A frame with no room runs nothing (never an exempt first unit).
    let mut full = FrameBudget::new(0.0);
    assert_eq!(
        layer.upload(Some(&mut product), &mut backend, &mut full, &clock, false),
        0
    );
    let mut budget = FrameBudget::new(f64::INFINITY);
    let units = layer.upload(Some(&mut product), &mut backend, &mut budget, &clock, false);
    assert_eq!(units, 4 * 3);
    assert!(!product.has_pending());
    assert_eq!(backend.textures_2d.len(), 12);
    assert_eq!(layer.texture_bytes as usize, product.bytes());
    for block in 0..4 {
        for channel in CHANNELS {
            let texture = layer.texture(block, channel).unwrap();
            let (size, format, bytes) = &backend.textures_2d[&texture];
            let side = layout().side(channel) as u32;
            assert_eq!(
                (*size, *format),
                ([side, side], CHANNEL_FORMATS[channel as usize])
            );
            let mut expected = vec![0; bytes.len()];
            product.write_channel(block, channel, &mut expected);
            assert_eq!(bytes, &expected);
        }
    }
    // Nothing waits: nothing is sent.
    let sent = backend.commands.len();
    assert_eq!(
        layer.upload(Some(&mut product), &mut backend, &mut budget, &clock, false),
        0
    );
    assert_eq!(backend.commands.len(), sent);
    // An edit in block 3: its height and water are replaced under their handles.
    let mut terrain = old_terrain.clone();
    terrain.set(296, 272, 0.0);
    refresh(&mut product, (&old_terrain, &water), (&terrain, &water));
    layer.upload(Some(&mut product), &mut backend, &mut budget, &clock, false);
    let replaced: Vec<_> = backend.commands[sent..]
        .iter()
        .map(|command| match command {
            Command::Texture2dReplace(texture, ..) => *texture,
            other => panic!("only replaces: {other:?}"),
        })
        .collect();
    assert_eq!(
        replaced,
        vec![
            layer.texture(3, Channel::Height).unwrap(),
            layer.texture(3, Channel::Water).unwrap()
        ]
    );
    // A product of another world: every texture freed, the new one created.
    let mut next = Product::new(product.layout, 2, product.blocks.clone());
    layer.upload(Some(&mut next), &mut backend, &mut budget, &clock, true);
    assert_eq!(backend.textures_2d.len(), 12);
    layer.upload(None, &mut backend, &mut budget, &clock, true);
    assert!(backend.textures_2d.is_empty(), "no product: no texture");
}

#[test]
fn a_cover_noted_before_the_build_reaches_the_product_of_its_world_only() {
    let shared = BackdropShared::default();
    let world = shared.world();
    let values = cover_of([1, 1], layout().patch_cover());
    shared.note_cover(world, [1, 1], values.clone());
    shared.note_cover(world + 5, [2, 1], values.clone());
    let blocks = build_blocks(&terrain(), &lakes([1.0, 1.0]), &layout());
    assert!(shared.install_build(world, layout(), blocks.clone()));
    let state = shared.state.lock().unwrap();
    let product = state.product.as_ref().unwrap();
    let mut expected = Product::new(layout(), world, blocks);
    expected.note_cover([1, 1], &values);
    assert_eq!(
        product.blocks, expected.blocks,
        "the stale world's note is dropped"
    );
    drop(state);
    shared.reset_world();
    assert!(shared.state.lock().unwrap().product.is_none());
    assert!(
        !shared.install_build(world, layout(), Vec::new()),
        "a stale build is refused"
    );
}
