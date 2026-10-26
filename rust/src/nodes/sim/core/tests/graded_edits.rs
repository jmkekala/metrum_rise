// SPDX-License-Identifier: GPL-2.0-only

//! `TERRAIN-07-D` item 5: a committed road edit journals what it changed for the graded tiles,
//! and the journal's own snapshots select off the lock exactly the tiles the live core's
//! selection does at `GL0`-`GL2` (a superset at `GL3`, which reads no site).

use super::SimCore;
use super::road_edit_order::{Stroke, committed};
use super::road_reference_probe::reference_fixtures;
use crate::nodes::sim::core::graded_edits::{EDIT_LEVELS, GradedEditKind};
use crate::nodes::sim::core::road_commit::RoadCommitRequest;
use crate::simulation::network::roadbed::RoadbedSystem;
use crate::simulation::terrain::graded::{Level, SelectScratch, dirty_tiles_for_road_changes};
use godot::prelude::Vector3;

fn commit_stroke(core: &mut SimCore, (ends, lanes): Stroke) {
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
// from `before` to the core's roadbed (`GL3`: contains it).
fn assert_journaled(core: &mut SimCore, before: &RoadbedSystem) {
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
