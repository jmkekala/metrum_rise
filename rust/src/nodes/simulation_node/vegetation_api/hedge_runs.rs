// SPDX-License-Identifier: GPL-2.0-only

//! Straight hedge runs for the distant hedge level: the modules of one row merged into boxes.

use super::*;
use placement::HEDGE_MODULE_M;

// Longest box one run draws. A straight box between two ground heights cannot follow the
// ground between them, so a run is cut where a sheared box would start to float or sink.
const MAX_RUN_M: f32 = 16.0;
// How far the ground under a run's modules may leave the straight line between its end modules.
// A graded yard bends where its flat lawn meets its banks, and a box sheared across the bend
// floats or sinks by as much.
const GROUND_TOLERANCE_M: f32 = 0.1;
// Modules of one row share their yaw exactly and stand on one line; this absorbs only the f32
// rounding of their positions along that line.
const ROW_TOLERANCE_M: f32 = 0.01;

// One module, in its row's frame.
struct RunModule {
    hedge: u8,
    yaw: f32,
    across: f32,
    along: f32,
    ground: f32,
}

/// Merges the hedge modules standing in the patch at `origin` into straight runs, packed seven
/// floats each: centre x, ground height at the centre, centre z, yaw, length, the ground's rise
/// from the run's start to its end, and the hedge index (low, medium, tall). A module belongs to the patch that contains its centre, as in the
/// scatter, so no run is drawn twice. Modules of one row join while they face the same way,
/// stand on one line and lie at most one module apart, which every row `line_at` lays does,
/// and while the ground under them stays straight enough for one sheared box. Heights are the
/// lawn as drawn, so a graded yard's hedge stands on its grading. O(m log m) for the m modules
/// in the patch's edited cells, the straightness test adding at most 16 per module; an unedited
/// patch costs a few block tests. Deterministic: modules are ordered by a total key before they are merged.
pub(crate) fn hedge_runs(core: &SimCore, origin: Vector2, span: f32) -> Vec<f32> {
    let (cell_m, _) = grid(core, VegetationLayer::Canopy);
    let first = cell_at(origin, VegetationLayer::Canopy, cell_m);
    let last = cell_at(origin + Vector2::splat(span), VegetationLayer::Canopy, cell_m);
    let height = |p: Vector2| authored_ground_height(core, p);
    let mut modules = Vec::new();
    if !core.vegetation_edits.any_in_cell_range(
        VegetationLayer::Canopy,
        (first.x, last.x),
        (first.z, last.z),
    ) {
        return Vec::new();
    }
    for z in first.z..=last.z {
        for x in first.x..=last.x {
            let cell = VegetationCell {
                layer: VegetationLayer::Canopy,
                x,
                z,
            };
            for plant in core.vegetation_edits.cell(cell).1 {
                let Some(hedge) = brush::hedge_index(plant.species, plant.variant) else {
                    continue;
                };
                if plant.x < origin.x
                    || plant.z < origin.y
                    || plant.x >= origin.x + span
                    || plant.z >= origin.y + span
                {
                    continue;
                }
                let along = Vector2::new(plant.yaw.cos(), -plant.yaw.sin());
                let pos = Vector2::new(plant.x, plant.z);
                modules.push(RunModule {
                    hedge,
                    yaw: plant.yaw,
                    across: pos.dot(along.orthogonal()),
                    along: pos.dot(along),
                    ground: height(pos),
                });
            }
        }
    }
    // Rows sort together: same hedge, same yaw, then across the row before along it. Rounding
    // the offset across the row to the tolerance keeps one row's modules adjacent in the order.
    let row_key = |m: &RunModule| (m.hedge, m.yaw.to_bits(), (m.across / ROW_TOLERANCE_M).round() as i64);
    modules.sort_by(|a, b| {
        row_key(a)
            .cmp(&row_key(b))
            .then(a.along.total_cmp(&b.along))
    });
    // Whether every module of a run stands within the tolerance of the line between its ends.
    let straight = |run: &[RunModule]| {
        let (head, tail) = (&run[0], &run[run.len() - 1]);
        let rise = (tail.ground - head.ground) / (tail.along - head.along).max(f32::EPSILON);
        run.iter().all(|m| {
            (head.ground + rise * (m.along - head.along) - m.ground).abs() <= GROUND_TOLERANCE_M
        })
    };
    let mut packed = Vec::new();
    let mut emit = |run: &[RunModule]| {
        let (head, tail) = (&run[0], &run[run.len() - 1]);
        let along = Vector2::new(head.yaw.cos(), -head.yaw.sin());
        let across = along.orthogonal() * head.across;
        let start = across + along * (head.along - HEDGE_MODULE_M * 0.5);
        let end = across + along * (tail.along + HEDGE_MODULE_M * 0.5);
        let centre = (start + end) * 0.5;
        packed.extend_from_slice(&[
            centre.x,
            height(centre),
            centre.y,
            head.yaw,
            start.distance_to(end),
            height(end) - height(start),
            f32::from(head.hedge),
        ]);
    };
    let mut begin = 0;
    for i in 1..=modules.len() {
        let joins = i < modules.len() && {
            let (prev, next) = (&modules[i - 1], &modules[i]);
            row_key(prev) == row_key(next)
                && next.along - prev.along <= HEDGE_MODULE_M + ROW_TOLERANCE_M
                && next.along - modules[begin].along + HEDGE_MODULE_M <= MAX_RUN_M
                && straight(&modules[begin..=i])
        };
        if !joins && i > begin {
            emit(&modules[begin..i]);
            begin = i;
        }
    }
    packed
}
