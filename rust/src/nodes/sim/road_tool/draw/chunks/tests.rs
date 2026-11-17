// SPDX-License-Identifier: GPL-2.0-only

//! The preview's overlay chunks against the recorder (`ROAD-112` round R2-4a): a preview is
//! staged and swapped in one call with the encoder's bytes and each layer's material at its
//! chunk's centre, an unchanged preview uploads nothing, a commit's chunks hold until released,
//! the pool stays one staging set, and every object made is freed.

use super::super::recorder::{Command, Recorder};
use super::*;
use crate::nodes::sim::render::stream::layers::roads::surfaces::RoadSurface;
use crate::simulation::network::render::NetworkMeshData;
use godot::builtin::{Color, Vector2, Vector3};
use std::sync::Arc;

const MATERIALS: [Handle; LAYER_COUNT] = [11, 12, 13, 14, 15, 16, 17, 18, 19];

fn surface(vertices: u32, seed: u8) -> RoadSurface {
    RoadSurface {
        format: 0x8_0000_001f,
        vertex_count: vertices,
        vertex_data: vec![seed; vertices as usize * 20],
        attribute_data: vec![seed; vertices as usize * 12],
        aabb: [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
    }
}

// An overlay of `keys`, each with a curb and a road layer.
fn overlay(keys: &[SurfaceChunkKey]) -> PreviewOverlay {
    PreviewOverlay {
        chunks: keys
            .iter()
            .enumerate()
            .map(|(at, &key)| {
                let seed = at as u8;
                let surfaces = RoadChunkSurfaces {
                    layers: vec![(1, surface(3, seed)), (4, surface(6, seed + 100))],
                    lining_shadow: None,
                };
                (key, surfaces)
            })
            .collect(),
        runs: Vec::new(),
        hidden: Vec::new(),
        positions: Vec::new(),
        generation: 0,
        span_m: 256.0,
        origin_m: (-1024.0, -512.0),
    }
}

fn pool() -> ChunkOverlays<Recorder> {
    ChunkOverlays::new(Recorder::default(), MATERIALS)
}

fn shown(pool: &ChunkOverlays<Recorder>) -> Vec<(SurfaceChunkKey, bool)> {
    pool.state()
        .filter(|state| pool.backend().shown(state.mesh))
        .map(|state| (state.key, state.held))
        .collect()
}

#[test]
fn a_preview_is_the_encoders_bytes_at_its_chunk_centres() {
    let mut pool = pool();
    let preview = overlay(&[(0, 0), (1, -1)]);
    pool.show(1, &preview);
    assert_eq!(shown(&pool), [((0, 0), false), ((1, -1), false)]);
    for state in pool.state() {
        let (key, surfaces) = preview
            .chunks
            .iter()
            .find(|(key, _)| *key == state.key)
            .unwrap();
        let drawn = &pool.backend().road_meshes[&state.mesh];
        let expected: Vec<_> = surfaces
            .layers
            .iter()
            .map(|(layer, surface)| (surface.clone(), MATERIALS[*layer]))
            .collect();
        assert_eq!(drawn, &expected);
        assert_eq!(state.origin, preview.centre(*key));
    }
    // The centre is the GDScript sum in f64: origin + index * span + span / 2.
    assert_eq!(
        preview.centre((1, -1)),
        [-1024.0 + 256.0 + 128.0, 0.0, -512.0 - 256.0 + 128.0]
    );
    assert_eq!(pool.sequence(), 1);
}

#[test]
fn an_unchanged_preview_uploads_nothing() {
    let mut pool = pool();
    let preview = overlay(&[(0, 0)]);
    pool.show(1, &preview);
    let commands = pool.backend().commands.len();
    pool.show(1, &preview);
    assert_eq!(pool.backend().commands.len(), commands);
    assert_eq!(pool.staged_total(), 1);
}

#[test]
fn a_new_preview_swaps_in_one_call_and_the_pool_stays_one_set() {
    let mut pool = pool();
    pool.show(1, &overlay(&[(0, 0), (1, 0)]));
    let first: Vec<Handle> = pool.state().map(|state| state.mesh).collect();
    pool.show(2, &overlay(&[(2, 0)]));
    // The staged slot is a new one (none was spare); both old ones hide, one goes.
    assert_eq!(shown(&pool), [((2, 0), false)]);
    let live = pool.backend().live.get();
    // One shown, one spare kept for the next staging: two meshes and two instances.
    assert_eq!(live, 4);
    assert!(
        pool.state()
            .all(|state| !first.contains(&state.mesh) || state.surfaces == 2)
    );
    // The next preview stages into the kept spare: it clears the spare's old surfaces first.
    let before = pool.backend().commands.len();
    pool.show(3, &overlay(&[(3, 0)]));
    let commands = &pool.backend().commands[before..];
    assert!(matches!(commands[0], Command::Clear(_)));
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, Command::MeshCreate(_)))
    );
    assert_eq!(shown(&pool), [((3, 0), false)]);
}

#[test]
fn a_commits_chunks_hold_until_released() {
    let mut pool = pool();
    pool.show(1, &overlay(&[(0, 0)]));
    pool.hold();
    assert_eq!(pool.sequence(), 0);
    pool.show(2, &overlay(&[(5, 5)]));
    assert_eq!(shown(&pool), [((5, 5), false), ((0, 0), true)]);
    pool.clear();
    assert_eq!(shown(&pool), [((0, 0), true)]);
    pool.release_held();
    assert!(shown(&pool).is_empty());
}

#[test]
fn the_drop_frees_every_object_made() {
    let mut pool = pool();
    pool.show(1, &overlay(&[(0, 0), (1, 0), (2, 0)]));
    pool.hold();
    pool.show(2, &overlay(&[(0, 1)]));
    pool.show(3, &overlay(&[(0, 2), (0, 3)]));
    let counter = std::rc::Rc::clone(&pool.backend().live);
    drop(pool);
    assert_eq!(counter.get(), 0);
}

#[test]
fn the_encoding_leaves_out_chunks_without_an_overlay() {
    let mut mesh = NetworkMeshData::new();
    mesh.curb_vertices = vec![Vector3::ZERO, Vector3::RIGHT, Vector3::FORWARD];
    mesh.curb_normals = vec![Vector3::UP; 3];
    mesh.curb_uvs = vec![Vector2::ZERO; 3];
    mesh.curb_colors = vec![Color::WHITE; 3];
    let with = PreviewChunk {
        overlay: Arc::new(mesh),
        hidden: Vec::new(),
        hidden_positions: Vec::new(),
    };
    let without = PreviewChunk {
        overlay: Arc::new(NetworkMeshData::new()),
        hidden: vec![[1, 0, 3]],
        hidden_positions: vec![Vector3::ZERO; 3],
    };
    let chunks = BTreeMap::from([((0, 0), with), ((1, 0), without)]);
    let overlay = PreviewOverlay::encode(&chunks, 7, 256.0, (0.0, 0.0));
    assert_eq!(overlay.chunks.len(), 1);
    assert_eq!(overlay.chunks[0].0, (0, 0));
    assert_eq!(
        overlay.chunks[0].1,
        RoadChunkSurfaces::encode_layers(&chunks[&(0, 0)].overlay)
    );
    // The runs keep only the chunk that hides some, with its positions as native bytes.
    assert_eq!(overlay.generation, 7);
    assert_eq!(overlay.runs.len(), 1);
    let (hidden, positions) = overlay.runs_of(&overlay.runs[0]);
    assert_eq!((overlay.runs[0].key, hidden), ((1, 0), &[[1, 0, 3]][..]));
    assert_eq!(positions, &[0u8; 36][..]);
}
