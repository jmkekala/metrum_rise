// SPDX-License-Identifier: GPL-2.0-only

//! Isolated reservation-index cost for rejecting a stale farm demolition inverse.

use super::*;
use crate::simulation::zoning::cells::{CellKey, CellSelectionShape};
use glam::DVec2;
use std::time::Instant;

#[test]
#[ignore = "undo reservation locality; run release alone without profiling"]
fn rejected_field_undo_reservation_scaling() {
    let (mut core, farm, polygon) = farm_fixture();
    core.commit_field_polygon_internal(farm, polygon).unwrap();
    assert!(core.push_building_removal_undo(farm));
    assert!(core.allocator.remove_building_for_bulldoze(
        farm,
        &mut core.zoning,
        &mut core.agents,
        &mut core.households,
        &mut core.logistics,
        &mut core.treasury.balance,
    ));
    core.publish_pending_building_site_changes();
    core.seal_building_removal_undo(0.0);
    assert!(core.prepare_cell_chunk_internal((0, 0)));
    let point = DVec2::new(120.0, 42.0);
    let grid = core.zoning.cells.pick(point).unwrap().grid;
    let mut previous = 0;
    for background in [0, 1_000, 10_000, 100_000] {
        // Populate the same reservation indices used by placement. As in the road-planning
        // field fixture, distant polygons need no running production/household simulation.
        for index in previous..background {
            assert!(core.zoning.cells.restore_saved_cell(
                CellKey {
                    grid,
                    x: 200 + (index % 700) as i32,
                    y: 200 + (index / 700) as i32,
                },
                1,
            ));
            core.allocator.field_clearance.set(
                index + 1,
                &rectangle(
                    -8_000.0 + (index % 400) as f32 * 12.0,
                    2_000.0 + (index / 400) as f32 * 12.0,
                    8.0,
                    4.0,
                ),
            );
        }
        previous = background;
        let remote_paint = core.zoning.cells.saved_cells();
        for blocker in ["field", "parcel", "paint"] {
            let parcel = match blocker {
                "field" => {
                    core.allocator
                        .field_clearance
                        .set(usize::MAX, &rectangle(100.0, 40.0, 40.0, 20.0));
                    None
                }
                "parcel" => Some(
                    core.zoning
                        .place_or_rezone_parcel_at(120.0, 8.0, 0, 20.0, 80.0, &core.region_graph)
                        .unwrap(),
                ),
                "paint" => {
                    let preview =
                        core.preview_cell_selection_internal(CellSelectionShape::Cell, &[point]);
                    assert!(!preview.selection.cells.is_empty());
                    // Exercise the retained demolition journal against newer cell authority,
                    // independently of the UI's ordering of separate paint undo entries.
                    core.zoning
                        .paint_cells(&preview.selection, 1, |_| false)
                        .unwrap();
                    None
                }
                _ => unreachable!(),
            };
            let dependencies = core.cell_tool_dependencies();
            let paint = core.zoning.cells.saved_cells();
            let balance = core.treasury.balance;
            let mut samples = Vec::with_capacity(21);
            for batch in 0..24 {
                let start = Instant::now();
                for _ in 0..64 {
                    assert!(!std::hint::black_box(core.undo_action_internal()));
                }
                let us = start.elapsed().as_secs_f64() * 1_000_000.0 / 64.0;
                if batch >= 3 {
                    samples.push(us);
                }
            }
            assert_eq!(core.cell_tool_dependencies(), dependencies);
            assert_eq!(core.zoning.cells.saved_cells(), paint);
            assert_eq!(core.treasury.balance, balance);
            assert_eq!(core.undo_stack.len(), 1);
            assert!(core.allocator.buildings.is_empty());
            assert!(core.agriculture.sites().is_empty());
            samples.sort_by(f64::total_cmp);
            println!(
                "FIELD_UNDO_RESERVATION_SCALING {}",
                serde_json::json!({
                    "background_cells": background, "background_field_footprints": background,
                    "blocker": blocker, "p50_us": samples[10], "p95_us": samples[19],
                    "batches": 21, "calls_per_batch": 64,
                    "rayon_threads": rayon::current_num_threads(),
                    "unchanged_authority": true, "journal_retained": true,
                })
            );
            match blocker {
                "field" => core.allocator.field_clearance.set(usize::MAX, &[]),
                "parcel" => {
                    assert_eq!(
                        core.zoning
                            .remove_parcels_by_raw_ids(&HashSet::from([parcel.unwrap().raw()])),
                        1
                    );
                }
                "paint" => {
                    let selection = core.zoning.cells.select(CellSelectionShape::Cell, &[point]);
                    core.zoning.paint_cells(&selection, 0, |_| false).unwrap();
                }
                _ => unreachable!(),
            }
            assert_eq!(core.zoning.cells.saved_cells(), remote_paint);
        }
    }
    assert!(
        core.undo_action_internal(),
        "released land permits the retained inverse"
    );
}
