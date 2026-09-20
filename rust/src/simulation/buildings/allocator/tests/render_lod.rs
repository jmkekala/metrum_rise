// SPDX-License-Identifier: GPL-2.0-only

//! Building LOD spatial locality and lifecycle tests using the real allocator index.

use super::support::*;
use super::*;
use crate::assets::lod_policy::LodQuality;
use crate::nodes::sim::render::building_lod::{
    BatchKey, View,
    spatial::{SpatialBatches, query_bounds},
};
use glam::{Mat4, Vec2, Vec3};

fn setup(background: usize) -> (BuildingAllocator, SpatialBatches) {
    let mut allocator = BuildingAllocator::new();
    let id = register_test_asset(&mut allocator, "test", "lod_house", ZoneClass::Residential);
    let mut manifest = allocator.registry.get(&id).unwrap().manifest.clone();
    let part = &mut manifest.mesh_parts[0];
    part.imported_bounds = Some([[-4.0, 0.0, -4.0], [4.0, 8.0, 4.0]]);
    for tier in 1..5 {
        let mut lod = part.lods[0].clone();
        lod.file = format!("lod{tier}.glb");
        part.lods.push(lod);
    }
    allocator.registry.register("test", manifest, String::new());
    for idx in 0..32 + background {
        let mut building = indexed_test_building(id.clone(), ZoneType::Residential, idx as i32);
        if idx < 32 {
            building.center_x = 32.0 + (idx % 8) as f32 * 12.0;
            building.center_y = 32.0 + (idx / 8) as f32 * 12.0;
        } else {
            building.center_x = 20_000.0 + (idx % 256) as f32 * 16.0;
            building.center_y = 20_000.0 + (idx / 256) as f32 * 16.0;
        }
        allocator.buildings.push(building);
    }
    allocator.rebuild_zone_index();
    let mut renderer = SpatialBatches::default();
    renderer.load_catalog(&allocator.registry);
    (allocator, renderer)
}

fn camera(extent: f32) -> View {
    View {
        world_to_clip: Mat4::orthographic_rh_gl(-extent, extent, -extent, extent, 0.1, 5000.0)
            * Mat4::look_at_rh(
                Vec3::new(64.0, 200.0, 64.0),
                Vec3::new(64.0, 0.0, 64.0),
                Vec3::NEG_Z,
            ),
        render_size: Vec2::splat(1024.0),
        quality: LodQuality::Balanced,
    }
}

fn output(renderer: &SpatialBatches) -> Vec<(BatchKey, Vec<[f32; 12]>)> {
    // Copy only for assertions, never in the production update path.
    renderer
        .ordered_chunks
        .iter()
        .flat_map(|key| {
            let chunk = &renderer.chunks[key].batches;
            chunk
                .batches
                .iter()
                .filter(|batch| batch.count != 0)
                .map(|batch| (batch.key, chunk.transforms(batch).to_vec()))
        })
        .collect()
}

#[test]
fn building_lod_spatial_queries_ignore_background_population_and_idle_clock() {
    let mut oracle = None;
    for background in [0, 1024, 65_536] {
        let (allocator, mut renderer) = setup(background);
        assert!(renderer.update(&allocator, [0, 0, 0, 0], camera(256.0), 0.0));
        assert_eq!(renderer.evaluated_parts, 32);
        assert_eq!(renderer.queried_chunks, 1);
        let actual = output(&renderer);
        if let Some(expected) = &oracle {
            assert_eq!(&actual, expected);
        } else {
            oracle = Some(actual);
        }
        renderer.acknowledge();
        assert!(!renderer.update(&allocator, [0, 0, 0, 0], camera(256.0), 0.5));
        assert_eq!(renderer.evaluated_parts, 0);
        assert_eq!(renderer.queried_chunks, 0);
    }
}

#[test]
fn building_lod_lifecycle_updates_construction_desertion_removal_and_world_replacement() {
    let (mut allocator, mut renderer) = setup(0);
    allocator.buildings[0].construction_total_hours = 4;
    allocator.buildings[0].construction_remaining_hours = 4;
    renderer.update(&allocator, [0, 0, 0, 0], camera(64.0), 0.0);
    let initial_y = output(&renderer)[0].1[0][7];
    renderer.acknowledge();
    assert!(renderer.update(&allocator, [0, 0, 0, 0], camera(64.0), 0.5));
    assert!(output(&renderer)[0].1[0][7] > initial_y);
    renderer.acknowledge();
    for _ in 0..4 {
        allocator.advance_construction_hour();
    }
    renderer.update(&allocator, [0, 0, 0, 0], camera(64.0), 0.0);
    assert_eq!(output(&renderer)[0].1[0][7], 0.0);
    renderer.acknowledge();
    allocator.mark_building_deserted(1);
    renderer.update(&allocator, [0, 0, 0, 0], camera(64.0), 0.0);
    assert_eq!(
        output(&renderer)
            .iter()
            .filter(|(key, _)| key.deserted)
            .map(|(_, instances)| instances.len())
            .sum::<usize>(),
        1
    );
    renderer.acknowledge();
    allocator.buildings.swap_remove(0);
    allocator.bump_building_ref_revision();
    allocator.rebuild_zone_index();
    renderer.update(&allocator, [0, 0, 0, 0], camera(64.0), 0.0);
    assert_eq!(
        output(&renderer)
            .iter()
            .map(|(_, instances)| instances.len())
            .sum::<usize>(),
        31
    );
    renderer.acknowledge();
    renderer.invalidate_world();
    assert!(!renderer.removed.is_empty());
    assert!(renderer.update(&allocator, [0, 0, 0, 0], camera(64.0), 0.0));
    assert_eq!(
        output(&renderer)
            .iter()
            .map(|(_, instances)| instances.len())
            .sum::<usize>(),
        31
    );
}

#[test]
fn building_lod_valid_meshless_sites_do_not_become_broken_placeholders() {
    let (mut allocator, mut renderer) = setup(0);
    let id = allocator.buildings[0].asset_id.clone();
    let mut manifest = allocator.registry.get(&id).unwrap().manifest.clone();
    manifest.mesh_parts.clear();
    allocator.registry.register("test", manifest, String::new());
    renderer.load_catalog(&allocator.registry);
    renderer.update(&allocator, [0, 0, 0, 0], camera(64.0), 0.0);
    assert!(output(&renderer).is_empty());
    allocator.buildings[0].broken = true;
    allocator.bump_building_ref_revision();
    renderer.update(&allocator, [0, 0, 0, 0], camera(64.0), 0.0);
    assert_eq!(output(&renderer)[0].0.part, 0);
    assert_eq!(output(&renderer)[0].1.len(), 1);
}

#[test]
fn building_lod_disconnected_buildings_remain_visible_without_becoming_economy_candidates() {
    let (mut allocator, mut renderer) = setup(0);
    allocator.buildings[0].edge_idx = usize::MAX;
    allocator.rebuild_zone_index();
    assert!(allocator.building_chunks[&(0, 0)].contains(&0));
    renderer.update(&allocator, [0, 0, 0, 0], camera(64.0), 0.0);
    assert_eq!(renderer.evaluated_parts, 32);
    let mut candidates = Vec::new();
    allocator.fill_nearby_buildings(32.0, 32.0, 0, 100, &mut candidates, |_, _| true);
    assert!(!candidates.contains(&0));
}

#[test]
fn building_lod_bounds_cover_cross_chunk_geometry_and_offscreen_shadow_casters() {
    let view = View {
        world_to_clip: Mat4::orthographic_rh_gl(-10.0, 10.0, -10.0, 10.0, 0.1, 100.0),
        render_size: Vec2::splat(1024.0),
        quality: LodQuality::Balanced,
    };
    let occupied = [-20, -20, 20, 20];
    let base = query_bounds(
        view,
        Vec3::ZERO,
        Vec3::NEG_Z,
        Vec3::Y,
        0.0,
        0.0,
        20.0,
        occupied,
    )
    .unwrap();
    assert_eq!(base, [-1, -1, 0, -1]);
    let oversized = query_bounds(
        view,
        Vec3::ZERO,
        Vec3::NEG_Z,
        Vec3::Y,
        0.0,
        600.0,
        20.0,
        occupied,
    )
    .unwrap();
    assert_eq!(oversized, [-2, -2, 1, 1]);
    let shadow = query_bounds(
        view,
        Vec3::ZERO,
        Vec3::NEG_Z,
        Vec3::new(1.0, 0.1, 0.0).normalize(),
        100.0,
        0.0,
        100.0,
        occupied,
    )
    .unwrap();
    assert!(shadow[2] > base[2]);
    assert_eq!(
        query_bounds(
            view,
            Vec3::ZERO,
            Vec3::NEG_Z,
            Vec3::X,
            100.0,
            0.0,
            100.0,
            occupied
        ),
        Some(occupied)
    );
}

#[test]
#[ignore = "release building LOD locality benchmark"]
fn benchmark_building_lod_locality() {
    use std::{hint::black_box, time::Instant};
    for background in [0, 1_024, 65_536] {
        let (allocator, mut renderer) = setup(background);
        let mut samples = Vec::new();
        for sample in 0..14 {
            let start = Instant::now();
            for iteration in 0..2000 {
                renderer.update(
                    black_box(&allocator),
                    [0, 0, 0, 0],
                    camera(if iteration % 2 == 0 { 10.0 } else { 256.0 }),
                    0.0,
                );
                assert_eq!(renderer.evaluated_parts, 32);
                renderer.acknowledge();
            }
            if sample >= 3 {
                samples.push(start.elapsed().as_secs_f64() * 1e6 / 2000.0);
            }
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "building_lod: local=32 background={background} median_us={:.3}",
            samples[samples.len() / 2]
        );
    }
}
