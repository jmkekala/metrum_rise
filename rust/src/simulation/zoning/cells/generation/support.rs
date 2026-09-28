// SPDX-License-Identifier: GPL-2.0-only

//! Bounded rectangular frontage support after direct cell conflict resolution.

use super::*;

/// Tests at most four six-cell columns without allocation or connected-component traversal.
/// Frontage intervals must be sorted by boundary and start, matching the persistent store.
pub(super) fn has_road_column(
    key: CellKey,
    depths: [u8; 4],
    survives: impl Fn(CellKey) -> bool,
    frontages: &HashMap<CellKey, Vec<CellRoadFrontage>>,
    graph: &RegionGraph,
) -> bool {
    for ((boundary, dx, dy), mut mask) in [
        (CellFrontage::MinY, 0, -1),
        (CellFrontage::MaxX, 1, 0),
        (CellFrontage::MaxY, 0, 1),
        (CellFrontage::MinX, -1, 0),
    ]
    .into_iter()
    .zip(depths)
    {
        let mut cell = key;
        // Only recorded supplier depths can terminate a column. Intervening cells still
        // all have to survive, including cells contributed by a compatible second road.
        while mask != 0 {
            if mask & 1 != 0
                && let Some(links) = frontages.get(&cell)
            {
                let mut end = 0;
                for link in links.iter().filter(|link| {
                    link.boundary == boundary && !graph.edge(link.edge).no_building_spawn
                }) {
                    if link.start > end {
                        break;
                    }
                    end = end.max(link.end);
                }
                if end == super::super::CELL_FRONTAGE_UNITS {
                    return true;
                }
            }
            mask >>= 1;
            if mask == 0 {
                break;
            }
            let (Some(x), Some(y)) = (cell.x.checked_add(dx), cell.y.checked_add(dy)) else {
                break;
            };
            cell.x = x;
            cell.y = y;
            if !survives(cell) {
                break;
            }
        }
    }
    false
}
