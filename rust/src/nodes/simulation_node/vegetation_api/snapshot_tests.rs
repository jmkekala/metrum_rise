// SPDX-License-Identifier: GPL-2.0-only

//! The vegetation query from a snapshot (`RENDER-32-D`): bitwise the live query on a fixture
//! with roads, sidewalks, street lights, a footpath, a field, a yard with walls, paving and a
//! planting area, yard hedges, brush stands and the generated scatter; stale requests never
//! delivered; completion order changes no product; edits invalidate the patches whose
//! placements they change and none far away.

use super::jobs::{PatchRequest, VegetationJobs, prepare_patch};
use super::land_cover::Coverage;
use super::tests::{commit_vegetation_test_road, core, square_yard, vegetation_test_path};
use super::world::VegetationPatchInputs;
use super::*;
use crate::nodes::sim::render::vegetation_product::{
    ProductCatalogue, ProductSettings, VegetationProduct,
};
use std::collections::{BTreeMap, BTreeSet};

const SETTINGS: ProductSettings = ProductSettings {
    density: 1.0,
    near_m: 200.0,
    crossfade_begin_m: 150.0,
    far_m: 4500.0,
    bush_range_m: 420.0,
    rock_range_m: 420.0,
};

// The renderer's catalogue as the golden writer packed it.
fn catalogue() -> ProductCatalogue {
    let line = include_str!("product_golden.txt").lines().next().unwrap();
    let hex = line.strip_prefix("C ").unwrap();
    let lanes: Vec<f32> = (0..hex.len() / 8)
        .map(|i| {
            let word = &hex[i * 8..i * 8 + 8];
            f32::from_le_bytes(std::array::from_fn(|b| {
                u8::from_str_radix(&word[b * 2..b * 2 + 2], 16).unwrap()
            }))
        })
        .collect();
    ProductCatalogue::from_lanes(&lanes).unwrap()
}

// Every float of a product by its bits, and its batches and counts as text.
fn fingerprint(product: &VegetationProduct) -> (String, Vec<Vec<u32>>) {
    let text = format!(
        "{:?} {} {:?}",
        product.batches, product.tree_count, product.canopy_box
    );
    let buffers = (0..product.buffer_count())
        .map(|i| product.buffer(i).iter().map(|v| v.to_bits()).collect())
        .collect();
    (text, buffers)
}

fn bits(values: &[f32]) -> Vec<u32> {
    values.iter().map(|v| v.to_bits()).collect()
}

// The populated fixture: two crossing streets with their sidewalks and lamps, a footpath, a
// committed field, a yard building with walls, paving and a planting area, two yards' hedges,
// brush stands and a hedge row over the generated scatter.
fn populated() -> SimCore {
    use crate::simulation::buildings::allocator::{BuildingSiteClient, indexed_test_building};
    use crate::simulation::zoning::ZoneType;
    let mut core = core();
    let cell_m = core.config.zone_cell_m;
    core.allocator.prepare_building_site_query_index(cell_m);
    let y = 20.0;
    commit_vegetation_test_road(
        &mut core,
        [Vector3::new(-300.0, y, 10.0), Vector3::new(300.0, y, 10.0)],
    );
    commit_vegetation_test_road(
        &mut core,
        [Vector3::new(40.0, y, -250.0), Vector3::new(40.0, y, 260.0)],
    );
    vegetation_test_path(
        &mut core,
        Vector2::new(-200.0, -120.0),
        Vector2::new(150.0, -60.0),
        0,
    );
    core.transit_network
        .roadbed
        .compile_dirty(&core.region_graph);
    core.precompute_road_mesh_data();
    core.allocator.field_clearance_mut().set(
        0,
        &[
            Vector2::new(-260.0, 60.0),
            Vector2::new(-160.0, 60.0),
            Vector2::new(-160.0, 160.0),
            Vector2::new(-260.0, 160.0),
        ],
    );
    let square = |a: f32, b: f32| {
        vec![
            Vector2::new(a, a + 90.0),
            Vector2::new(a, b + 90.0),
            Vector2::new(b, b + 90.0),
            Vector2::new(b, a + 90.0),
        ]
    };
    let lot = square(80.0, 110.0);
    let mut building = indexed_test_building(String::new(), ZoneType::None, 0);
    (building.center_x, building.center_y) = (95.0, 185.0);
    core.allocator.buildings.push(building);
    let house = square(85.0, 95.0);
    core.allocator.push_site_client(BuildingSiteClient {
        structure_world: vec![[house[0], house[1], house[2], house[3]]],
        // A planting area reaching past the footprint, which the snapshot must still copy.
        planting_world: vec![square(98.0, 118.0)],
        footprint_world: lot.clone(),
        lot_footprint_world: [lot[0], lot[1], lot[2], lot[3]],
        support_height_m: y,
        surfaces: vec![
            crate::simulation::buildings::allocator::BuildingSiteSurfaceClient {
                material: crate::assets::SiteSurfaceMaterial::Asphalt,
                name: String::new(),
                vertices_world: square(100.0, 104.0),
            },
        ],
    });
    core.allocator.recompute_max_site_radius_m();
    core.allocator.prepare_building_site_query_index(cell_m);
    core.terrain_query_index
        .rebuild_all(&core.transit_network.roadbed, &core.allocator);
    core.allocator.pending_yards.push(square_yard(
        (1, 0),
        Vector2::new(-60.0, 200.0),
        Vector2::new(-40.0, 220.0),
    ));
    core.allocator.pending_yards.push(square_yard(
        (2, 0),
        Vector2::new(-39.0, 200.0),
        Vector2::new(-19.0, 220.0),
    ));
    publish_yards(&mut core);
    for (spot, option) in [(0, 0i64), (1, 3), (2, 8), (3, 12), (4, 21), (5, 24)] {
        let at = Vector2::new(-250.0 + spot as f32 * 95.0, -200.0 + spot as f32 * 30.0);
        for _ in 0..3 {
            paint_at(&mut core, at, 35.0, option, 0, stamp_limit(option, 35.0));
        }
    }
    // Planted trees and shrubs on and beside the yard: its lawn, walls, paving and planting.
    for (x, z, option) in [(108.0, 200.0, 0i64), (90.0, 180.0, 21), (101.0, 192.0, 21)] {
        add_at(&mut core, Vector2::new(x, z), option);
    }
    line_at(
        &mut core,
        Vector2::new(-120.0, 30.0),
        Vector2::new(-20.0, 60.0),
        18,
        0,
    );
    core.prepare_site_queries();
    core
}

// Every patch of the vegetation grids over the fixture's built area: the fine grid and the
// coarse one, with the bands alternating so both product shapes are compared.
fn requests(core: &SimCore) -> Vec<PatchRequest> {
    let layout = PatchLayout::new(core);
    let mut out = Vec::new();
    for (divisor, extent) in [(4, 8), (1, 2)] {
        let span = layout.span / divisor as f32;
        let first = ((layout.half_w - 350.0) / span).floor() as i32;
        for x in first..first + extent {
            for z in first..first + extent {
                let near = (x + z) % 2 == 0;
                out.push(PatchRequest {
                    key: [x, z, divisor],
                    origin: Vector2::new(
                        x as f32 * span - layout.half_w,
                        z as f32 * span - layout.half_h,
                    ),
                    span,
                    understory: near,
                    near_band: near,
                    caster: x % 2 == 0,
                    settings: SETTINGS,
                });
            }
        }
    }
    out
}

fn snapshot(core: &mut SimCore, request: &PatchRequest) -> VegetationPatchInputs {
    VegetationPatchInputs::take(core, request.origin, request.span).expect("inputs ready")
}

#[test]
fn snapshot_queries_equal_live_queries_bitwise_on_a_populated_fixture() {
    let mut core = populated();
    let catalogue = catalogue();
    let (mut plants, mut runs_seen) = (0, 0);
    for request in requests(&core) {
        let inputs = snapshot(&mut core, &request);
        let (origin, span) = (request.origin, request.span);
        let live = decorative_tree_patch(&core, origin, span, request.understory);
        let copied = decorative_tree_patch(&inputs, origin, span, request.understory);
        assert_eq!(
            bits(&live),
            bits(&copied),
            "placements of {:?}",
            request.key
        );
        plants += live.len() / 6;
        let margin = STAND_RADIUS_TEXELS as f32 * land_cover::TEXEL_M;
        let cover = |world: &dyn Fn(Vector2, f32) -> Coverage| {
            world(origin - Vector2::splat(margin), span + 2.0 * margin)
                .stand_parts(STAND_RADIUS_TEXELS)
        };
        assert_eq!(
            cover(&|o, s| Coverage::build(&core, o, s)),
            cover(&|o, s| Coverage::build(&inputs, o, s)),
            "stand cover of {:?}",
            request.key
        );
        let live_runs = hedge_runs::hedge_runs(&core, origin, span);
        assert_eq!(
            bits(&live_runs),
            bits(&hedge_runs::hedge_runs(&inputs, origin, span)),
            "hedge runs of {:?}",
            request.key
        );
        runs_seen += live_runs.len() / 7;
        let (mut a, mut b) = (VegetationProduct::default(), VegetationProduct::default());
        prepare_patch(&core, &request, &catalogue, &mut a);
        prepare_patch(&inputs, &request, &catalogue, &mut b);
        assert_eq!(
            fingerprint(&a),
            fingerprint(&b),
            "product of {:?}",
            request.key
        );
    }
    // The fixture exercises what it claims to.
    assert!(
        plants > 2000 && runs_seen > 4,
        "{plants} plants, {runs_seen} hedge runs"
    );
}

// Lands the products of `requests` in `order` and returns each key's delivered product.
fn delivered(
    products: &[VegetationProduct],
    requests: &[PatchRequest],
    order: &[usize],
) -> BTreeMap<[i32; 3], (String, Vec<Vec<u32>>)> {
    let mut jobs = VegetationJobs::default();
    jobs.set_catalogue(catalogue());
    let serials: Vec<u64> = requests
        .iter()
        .map(|request| jobs.start(request.key).unwrap().0)
        .collect();
    for &i in order {
        jobs.land(requests[i].key, serials[i], Some(products[i].clone()));
    }
    let mut out = BTreeMap::new();
    while let Some(result) = jobs.poll() {
        let product = result.product.unwrap();
        assert!(out.insert(result.key, fingerprint(&product)).is_none());
        jobs.recycle(product);
    }
    assert_eq!(jobs.in_flight(), 0);
    out
}

#[test]
fn shuffled_completion_delivers_the_same_products() {
    let mut core = populated();
    let catalogue = catalogue();
    let requests: Vec<_> = requests(&core).into_iter().take(24).collect();
    let products: Vec<VegetationProduct> = requests
        .iter()
        .map(|request| {
            let inputs = snapshot(&mut core, request);
            let mut product = VegetationProduct::default();
            prepare_patch(&inputs, request, &catalogue, &mut product);
            product
        })
        .collect();
    let forward: Vec<usize> = (0..requests.len()).collect();
    let expected = delivered(&products, &requests, &forward);
    assert_eq!(expected.len(), requests.len());
    let mut order = forward.clone();
    // A fixed LCG permutation, three times: deterministic, and far from either end's order.
    let mut state = 0x2545_f491_u64;
    for _ in 0..3 {
        for i in (1..order.len()).rev() {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            order.swap(i, (state >> 33) as usize % (i + 1));
        }
        assert_eq!(delivered(&products, &requests, &order), expected);
    }
    order.reverse();
    assert_eq!(delivered(&products, &requests, &order), expected);
}

#[test]
fn a_stale_request_is_never_delivered() {
    let mut jobs = VegetationJobs::default();
    jobs.set_catalogue(catalogue());
    let marked = |count: i64| {
        let mut product = VegetationProduct::default();
        product.tree_count = count;
        Some(product)
    };
    let key = [3, 4, 4];
    let old = jobs.start(key).unwrap().0;
    let new = jobs.start(key).unwrap().0;
    // The newer request lands first and the older after it: only the newer is delivered.
    jobs.land(key, new, marked(2));
    jobs.land(key, old, marked(1));
    let result = jobs.poll().unwrap();
    assert_eq!(
        (result.serial, result.product.unwrap().tree_count),
        (new, 2)
    );
    assert!(
        jobs.poll().is_none(),
        "the older request's product is dropped"
    );
    // A cancelled request and a cleared one are dropped too, and a busy one is delivered so the
    // renderer asks again.
    let cancelled = jobs.start([1, 1, 1]).unwrap().0;
    jobs.cancel([1, 1, 1]);
    let cleared = jobs.start([2, 2, 1]).unwrap().0;
    jobs.clear();
    let busy = jobs.start([5, 5, 1]).unwrap().0;
    jobs.land([1, 1, 1], cancelled, marked(3));
    jobs.land([2, 2, 1], cleared, marked(4));
    jobs.land([5, 5, 1], busy, None);
    let result = jobs.poll().unwrap();
    assert_eq!((result.key, result.serial), ([5, 5, 1], busy));
    assert!(result.product.is_none());
    assert!(jobs.poll().is_none());
    assert_eq!(jobs.in_flight(), 0);
}

// Each fine patch's placements from a snapshot, and each terrain patch's two generations.
// Placements by fine patch, and the two generations by terrain patch.
type EditState = (
    BTreeMap<[i32; 3], Vec<u32>>,
    BTreeMap<(i32, i32), (u64, u64)>,
);

fn state(core: &mut SimCore) -> EditState {
    let mut placements = BTreeMap::new();
    let mut generations = BTreeMap::new();
    for request in requests(core).into_iter().filter(|r| r.key[2] == 4) {
        let inputs = snapshot(core, &request);
        let scatter = decorative_tree_patch(&inputs, request.origin, request.span, true);
        placements.insert(request.key, bits(&scatter));
        let owner = (request.key[0] / 4, request.key[1] / 4);
        generations.entry(owner).or_insert_with(|| {
            (
                core.terrain_payload_generation_for_patch(owner.0 as usize, owner.1 as usize),
                core.vegetation_edits
                    .patch_generation(pack_patch_key(owner.0, owner.1)),
            )
        });
    }
    (placements, generations)
}

// After `edit`: every fine patch whose placements changed has an owner whose generations moved
// (no stale patch). With `reach`, every owner whose generations moved also lies within it of
// `at` (no far patch is rebuilt). Returns how many patches changed.
fn check_edit(
    core: &mut SimCore,
    at: Vector2,
    reach: Option<f32>,
    edit: impl FnOnce(&mut SimCore),
) -> usize {
    let (before, generations) = state(core);
    edit(core);
    core.prepare_site_queries();
    let (after, moved_generations) = state(core);
    let moved: BTreeSet<(i32, i32)> = generations
        .iter()
        .filter(|(owner, pair)| moved_generations[*owner] != **pair)
        .map(|(owner, _)| *owner)
        .collect();
    let layout = PatchLayout::new(core);
    for owner in moved.iter().filter(|_| reach.is_some()) {
        let low = Vector2::new(
            owner.0 as f32 * layout.span - layout.half_w,
            owner.1 as f32 * layout.span - layout.half_h,
        );
        let nearest = at.clamp(low, low + Vector2::splat(layout.span));
        assert!(
            nearest.distance_to(at) <= reach.unwrap(),
            "owner {owner:?} invalidated {} m from the edit",
            nearest.distance_to(at)
        );
    }
    let mut changed = 0;
    for (key, placements) in &before {
        if after[key] != *placements {
            changed += 1;
            let owner = (key[0] / 4, key[1] / 4);
            assert!(
                moved.contains(&owner),
                "patch {key:?} changed without an invalidation"
            );
        }
    }
    changed
}

#[test]
fn brush_road_commit_and_undo_invalidate_exactly_the_touched_patches() {
    let mut core = populated();
    // A brush stroke moves the vegetation revision of exactly the patches it edits.
    let brush_at = Vector2::new(-150.0, -150.0);
    let painted = check_edit(&mut core, brush_at, Some(40.0), |core| {
        paint_at(core, brush_at, 30.0, 0, 7, stamp_limit(0, 30.0));
    });
    assert!(painted > 0, "the brush changed some patch");
    // A road edit moves terrain payload generations, which the terrain's own publication
    // decides and may move world-wide (`terrain_payload_global_generation`): only the
    // no-stale-patch half is the vegetation's to hold.
    let road_at = Vector2::new(-100.0, 230.0);
    let committed = check_edit(&mut core, road_at, None, |core| {
        commit_vegetation_test_road(
            core,
            [
                Vector3::new(-300.0, 20.0, 230.0),
                Vector3::new(100.0, 20.0, 230.0),
            ],
        );
    });
    assert!(committed > 0, "the road cleared some patch");
    let undone = check_edit(&mut core, road_at, None, |core| {
        assert!(core.undo_action_internal());
        core.precompute_road_mesh_data();
    });
    assert!(undone > 0, "the undo restored some patch");
}

// Every patch of both grids over the whole world, both band shapes alternating.
fn world_requests(core: &SimCore) -> Vec<PatchRequest> {
    let layout = PatchLayout::new(core);
    let mut out = Vec::new();
    for divisor in [4, 1] {
        let span = layout.span / divisor as f32;
        let count = ((layout.half_w * 2.0) / span).ceil() as i32;
        for x in 0..count {
            for z in 0..count {
                let near = (x + z) % 2 == 0;
                out.push(PatchRequest {
                    key: [x, z, divisor],
                    origin: Vector2::new(
                        x as f32 * span - layout.half_w,
                        z as f32 * span - layout.half_h,
                    ),
                    span,
                    understory: near,
                    near_band: near,
                    caster: true,
                    settings: SETTINGS,
                });
            }
        }
    }
    out
}

fn percentile(values: &mut [f64], q: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    values[((values.len() - 1) as f64 * q).round() as usize]
}

// `RENDER-32-D` on a saved city (`METRUM_VEG_SAVE`, e.g. the timing cache's 30k city; without
// it the 2040 m generated forest of the product goldens): every patch of both grids from a
// snapshot equals the live query bitwise, and what a snapshot and a worker's query cost a patch.
// Release: `cargo test --release --lib vegetation_snapshot_on_a_saved_city -- --ignored --nocapture`.
#[test]
#[ignore = "manual: a saved city's every patch, live against snapshot, with per-patch costs"]
fn vegetation_snapshot_on_a_saved_city() {
    use std::time::Instant;
    let mut core = match std::env::var("METRUM_VEG_SAVE") {
        Ok(path) => {
            let mut core = SimCore::new(crate::simulation::core::config::WorldConfig::new(
                2048.0, 2048.0, 40.0, 10.0,
            ));
            let mods =
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../godot/bootstrap/mods");
            let scan = crate::assets::scanner::scan_pack_dir(&mods);
            core.replace_asset_registry(scan.packs.into_iter().flat_map(|pack| {
                let pack_id = pack.pack.pack_id;
                pack.assets
                    .into_iter()
                    .map(move |(manifest, dir)| (pack_id.clone(), manifest, dir))
            }));
            core.load_game_internal(&path).unwrap();
            core.precompute_road_mesh_data();
            core
        }
        Err(_) => {
            let mut core = super::super::tests::vegetation_test_core(Default::default());
            core.create_blank_world_internal(2040.0, 2040.0, 10.0, 510.0, 20.0)
                .unwrap();
            core
        }
    };
    core.prepare_site_queries();
    let catalogue = catalogue();
    let (mut take_ms, mut query_ms, mut live_ms) = (Vec::new(), Vec::new(), Vec::new());
    let mut plants = 0;
    for request in world_requests(&core) {
        let start = Instant::now();
        let inputs = snapshot(&mut core, &request);
        take_ms.push(start.elapsed().as_secs_f64() * 1e3);
        let mut copied = VegetationProduct::default();
        let start = Instant::now();
        prepare_patch(&inputs, &request, &catalogue, &mut copied);
        query_ms.push(start.elapsed().as_secs_f64() * 1e3);
        let mut live = VegetationProduct::default();
        let start = Instant::now();
        prepare_patch(&core, &request, &catalogue, &mut live);
        live_ms.push(start.elapsed().as_secs_f64() * 1e3);
        assert_eq!(
            fingerprint(&live),
            fingerprint(&copied),
            "patch {:?}",
            request.key
        );
        plants += live.tree_count;
    }
    let patches = take_ms.len();
    let total = |v: &[f64]| v.iter().sum::<f64>();
    let (take_total, query_total, live_total) =
        (total(&take_ms), total(&query_ms), total(&live_ms));
    println!(
        "patches {patches} plants {plants}; snapshot take p50 {:.3} max {:.3} total {:.1} ms; \
         snapshot query+product p50 {:.3} max {:.3} total {:.1} ms; live p50 {:.3} max {:.3} \
         total {:.1} ms",
        percentile(&mut take_ms, 0.5),
        percentile(&mut take_ms, 1.0),
        take_total,
        percentile(&mut query_ms, 0.5),
        percentile(&mut query_ms, 1.0),
        query_total,
        percentile(&mut live_ms, 0.5),
        percentile(&mut live_ms, 1.0),
        live_total,
    );
}
