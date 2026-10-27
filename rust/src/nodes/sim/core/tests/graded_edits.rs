// SPDX-License-Identifier: GPL-2.0-only

//! `TERRAIN-07-D` item 5: a committed road edit journals what it changed for the graded tiles,
//! and the journal's own snapshots select off the lock exactly the tiles the live core's
//! selection does at `GL0`-`GL2` (a superset at `GL3`, which reads no site). An undo on the
//! roadbed's swap path journals the commit's tiles back.

use super::SimCore;
use super::road_edit_order::{Stroke, committed};
use super::road_reference_probe::reference_fixtures;
use crate::nodes::sim::core::graded_edits::{EDIT_LEVELS, GradedEditKind};
use crate::nodes::sim::core::road_commit::RoadCommitRequest;
use crate::simulation::network::roadbed::{PieceId, PieceMesh, RoadbedSystem};
use crate::simulation::terrain::graded::{
    Level, SelectScratch, TileKey, bake_tiles, dirty_tiles_for_road_changes,
};
use godot::prelude::Vector3;
use std::collections::BTreeMap;

pub(super) fn commit_stroke(core: &mut SimCore, (ends, lanes): Stroke) {
    let points = ends
        .map(|point| {
            Vector3::new(
                point.x,
                core.get_world_surface_height_internal(point),
                point.y,
            )
        })
        .to_vec();
    let outcome = core.commit_road(
        RoadCommitRequest {
            points,
            fwd_lanes: lanes,
            bkw_lanes: lanes,
            snap_to_existing_roads: true,
            preview: None,
        },
        std::time::Instant::now(),
        Default::default(),
    );
    assert!(outcome.committed, "{}", outcome.rejection);
}

// The journal holds one road edit whose selection at every level equals the live selection
// from `before` to the core's roadbed (`GL3`: contains it). Returns the edit's selection.
fn assert_journaled(core: &mut SimCore, before: &RoadbedSystem) -> Vec<(Level, Vec<TileKey>)> {
    let edits = core.take_graded_edits();
    let kinds: Vec<&str> = edits
        .iter()
        .map(|edit| match edit.kind {
            GradedEditKind::Roads { .. } => "roads",
            GradedEditKind::Sites { .. } => "sites",
            GradedEditKind::Base(_) => "base",
            GradedEditKind::All => "all",
        })
        .collect();
    let [edit] = edits.as_slice() else {
        panic!("one edit, got {kinds:?}");
    };
    let GradedEditKind::Roads { sites, .. } = &edit.kind else {
        panic!("a road edit, got {kinds:?}");
    };
    assert!(sites.is_some(), "the site index is ready");
    let mut scratch = SelectScratch::default();
    let selection = edit.select(&mut scratch);
    assert!(!selection.all);
    assert_eq!(selection.levels.len(), EDIT_LEVELS.len());
    let mut live = Vec::new();
    for (level, tiles) in &selection.levels {
        dirty_tiles_for_road_changes(
            &core.heightmap,
            before,
            &core.transit_network.roadbed,
            &core.allocator,
            &core.last_road_terrain_changes,
            *level,
            &mut scratch,
            &mut live,
        );
        assert!(!live.is_empty(), "{level:?}: the edit changes tiles");
        if matches!(level, Level::Gl3 { .. }) {
            assert!(
                live.iter().all(|tile| tiles.binary_search(tile).is_ok()),
                "{level:?}: a superset of the live selection"
            );
        } else {
            assert_eq!(tiles, &live, "{level:?}");
        }
    }
    selection.levels
}

#[test]
fn a_commit_journals_its_dirty_tiles() {
    let (_, strokes, _) = reference_fixtures().swap_remove(0);
    let mut core = committed(&strokes, &[0], true);
    assert!(
        core.take_graded_edits().is_empty(),
        "the journal starts empty"
    );
    let before = core.transit_network.roadbed.clone();
    commit_stroke(&mut core, strokes[1]);
    assert_journaled(&mut core, &before);
}

// Every published piece's mesh address.
fn piece_meshes(roadbed: &RoadbedSystem) -> BTreeMap<PieceId, *const PieceMesh> {
    let everywhere = [f64::MIN, f64::MIN, f64::MAX, f64::MAX];
    roadbed
        .pieces_in_bounds(everywhere)
        .map(|(id, _)| {
            let mesh = match id {
                PieceId::Span(edge) => roadbed.span_piece(edge).map(|span| span.mesh()),
                PieceId::Node(node) => roadbed.node_piece(node).map(|piece| piece.mesh()),
            };
            (id, std::ptr::from_ref(mesh.expect("listed pieces are published")))
        })
        .collect()
}

// Each tile's payload on `roadbed` with the core's sites, as `Debug` text (equal text, equal
// bits).
fn baked(core: &SimCore, roadbed: &RoadbedSystem, tiles: &[TileKey], level: Level) -> Vec<String> {
    let mut out = Vec::new();
    bake_tiles(&core.heightmap, roadbed, &core.allocator, tiles, level, &mut out)
        .expect("the tiles bake");
    out.iter().map(|payload| format!("{payload:?}")).collect()
}

// The undo of a commit nothing rebuilt since takes the roadbed's swap path (`ROAD-68-E`: the
// pre-edit pieces themselves come back). The journal records it as a road edit from the
// commit's roadbed to the restored one, which selects the commit's own tiles at `GL0`-`GL2`, and
// those tiles bake to the bytes they had before the commit.
#[test]
fn an_undo_on_the_swap_path_journals_the_commits_tiles_back() {
    let (_, strokes, _) = reference_fixtures().swap_remove(0);
    let mut core = committed(&strokes, &[0], true);
    // Benchmark mode keeps no undo state.
    core.benchmark_mode = false;
    assert!(core.take_graded_edits().is_empty());
    let before = core.transit_network.roadbed.clone();
    commit_stroke(&mut core, strokes[1]);
    let commit = assert_journaled(&mut core, &before);
    let after = core.transit_network.roadbed.clone();

    assert!(core.undo_action_internal());
    assert_eq!(
        piece_meshes(&core.transit_network.roadbed),
        piece_meshes(&before),
        "the swap path: the pre-edit pieces themselves"
    );
    let undo = assert_journaled(&mut core, &after);
    for ((level, committed), (_, undone)) in commit.iter().zip(&undo) {
        if matches!(level, Level::Gl3 { .. }) {
            continue;
        }
        assert_eq!(undone, committed, "{level:?}: the undo dirties the commit's tiles");
        let restored = baked(&core, &core.transit_network.roadbed, undone, *level);
        assert_eq!(
            restored,
            baked(&core, &before, undone, *level),
            "{level:?}: the tiles bake as before the commit"
        );
        assert_ne!(
            restored,
            baked(&core, &after, undone, *level),
            "{level:?}: the commit had changed them"
        );
    }
}
