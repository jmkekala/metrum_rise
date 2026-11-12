// SPDX-License-Identifier: GPL-2.0-only

//! The stream benchmark's water input of record (`RENDER-32-M` step 0, `docs/terrain.md`
//! "Stream benchmark"): a pond made on the prepared 30k city by a fixed recipe through the
//! world's own mutators, saved beside the prepared cities under a name keyed by the save version.
//!
//! Recipe: the terrain is levelled towards `POND_DEPTH_M` below the flat ground over a disc of
//! `POND_RADIUS_M` about `POND_CENTRE` (`level_terrain_internal`, strength 1: a flat bottom with
//! the brush's cosine bank), then a lake fill is begun, updated and committed at the centre with
//! its surface `POND_SURFACE_M` (`begin` / `update` / `commit_world_lake_fill_preview_internal`):
//! the flood fill takes every cell below the surface connected to the centre, the basin only. The
//! disc lies north of the city's last street, clear of every road node and building (asserted).
//! The city's clock and everything else stay as loaded.

use super::*;
use godot::prelude::Vector2;
use crate::nodes::sim::core::scenario::install_bootstrap_assets;
use crate::simulation::save::schema::SAVE_VERSION;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

const POND_CENTRE: [f32; 2] = [0.0, 2048.0];
const POND_RADIUS_M: f32 = 200.0;
const POND_DEPTH_M: f32 = 3.0;
// Below the flat ground (0 m) by a few centimetres: a fill at the ground's own height would
// spread over the whole flat world and escape at its edge.
const POND_SURFACE_M: f32 = -0.05;
// Nothing of the city may stand within this distance of the disc.
const CLEARANCE_M: f32 = 40.0;

fn sha256_hex(path: &std::path::Path) -> String {
    let bytes = std::fs::read(path).unwrap();
    Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The pond save's name: keyed by the save version, so a version bump names a new file that the
/// recipe makes again from the re-prepared city.
fn pond_name(residents: u32) -> String {
    format!("stream_pond_{residents}_v{SAVE_VERSION}.sqlite")
}

#[test]
#[ignore = "writes the stream benchmark's pond: METRUM_PERF_CACHE=<prepared cities> $TEST_BINARY nodes::sim::core::tests::stream_pond_fixture::generate_stream_pond_fixture --exact --ignored --nocapture"]
fn generate_stream_pond_fixture() {
    let directory = std::env::var_os("METRUM_PERF_CACHE").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/perf-contract"),
        PathBuf::from,
    );
    let residents = 30_000;
    let source = directory.join(format!("populated_city_{residents}.sqlite"));
    let mut core = SimCore {
        benchmark_mode: true,
        ..SimCore::new(WorldConfig::new(2048.0, 2048.0, 40.0, 10.0))
    };
    install_bootstrap_assets(&mut core);
    core.load_game_internal(source.to_str().unwrap())
        .unwrap_or_else(|error| panic!("could not load {}: {error}", source.display()));
    let clock = (core.time.day_index, core.time.minute_of_day);

    let reach = POND_RADIUS_M + CLEARANCE_M;
    let near = |x: f32, z: f32| {
        let (dx, dz) = (x - POND_CENTRE[0], z - POND_CENTRE[1]);
        dx * dx + dz * dz <= reach * reach
    };
    // One-time preparation, not a frame path: a scan of the city's nodes and buildings.
    for id in 0..core.region_graph.node_count() as u32 {
        let pos = core.region_graph.node(id).pos;
        assert!(!near(pos.x, pos.z), "road node {id} stands in the pond's disc");
    }
    for building in &core.allocator.buildings {
        assert!(
            !near(building.center_x, building.center_y),
            "a building stands in the pond's disc"
        );
    }

    let centre = Vector2::new(POND_CENTRE[0], POND_CENTRE[1]);
    core.level_terrain_internal(centre, POND_RADIUS_M, -POND_DEPTH_M, 1.0);
    let begun = core
        .begin_world_lake_fill_preview_internal(centre, POND_SURFACE_M + 1.0)
        .expect("lake fill preview");
    assert!(begun.filled_cells > 0);
    let preview = core
        .update_world_lake_fill_preview_internal(POND_SURFACE_M)
        .expect("lake fill update");
    assert!(preview.is_valid(), "the pond's fill must stay in its basin");
    core.commit_world_lake_fill_preview_internal()
        .expect("lake fill commit");
    assert_eq!((core.time.day_index, core.time.minute_of_day), clock);

    let path = directory.join(pond_name(residents));
    let temporary = directory.join(format!(".{}.{}.tmp", pond_name(residents), std::process::id()));
    core.save_game_internal(temporary.to_str().unwrap(), None)
        .unwrap_or_else(|error| panic!("could not save the pond: {error}"));
    // The save's own clock stamp is the source's, so the recipe's output is the same bytes on
    // every run (the SHA-256 of record).
    let stamp: i64 = rusqlite::Connection::open(&source)
        .unwrap()
        .query_row("SELECT saved_at_unix FROM save_meta LIMIT 1", [], |row| row.get(0))
        .unwrap();
    rusqlite::Connection::open(&temporary)
        .unwrap()
        .execute("UPDATE save_meta SET saved_at_unix = ?1", [stamp])
        .unwrap();
    std::fs::rename(&temporary, &path).unwrap();

    // The save holds the pond: a fresh load reads the same water back.
    let mut loaded = SimCore {
        benchmark_mode: true,
        ..SimCore::new(WorldConfig::new(2048.0, 2048.0, 40.0, 10.0))
    };
    install_bootstrap_assets(&mut loaded);
    loaded.load_game_internal(path.to_str().unwrap()).unwrap();
    let wet = |core: &SimCore| {
        let (x, z) = (POND_CENTRE[0], POND_CENTRE[1]);
        core.watermap.visible_depth_world(x, z)
    };
    let depth = wet(&loaded);
    assert!(
        (depth - (POND_DEPTH_M + POND_SURFACE_M)).abs() < 0.01,
        "the pond's centre is {depth} m deep"
    );
    eprintln!(
        "STREAM_POND {}",
        serde_json::json!({
            "source": source.file_name().unwrap().to_str(),
            "source_sha256": sha256_hex(&source),
            "pond": path.file_name().unwrap().to_str(),
            "pond_sha256": sha256_hex(&path),
            "save_version": SAVE_VERSION,
            "filled_cells": preview.filled_cells,
            "centre_depth_m": depth,
        })
    );
}
