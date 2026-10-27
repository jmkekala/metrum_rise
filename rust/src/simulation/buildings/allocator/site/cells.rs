// SPDX-License-Identifier: GPL-2.0-only

//! The building sites by the 64 m cells their bounds meet (`TERRAIN-07-F`, `docs/terrain.md`
//! T37 (b), T40 (a)): the graded terrain's site reads visit the sites near their window, not
//! every building of the 512 m chunks it meets. Derived from the site clients and changed with
//! them, in the same call; never saved. Within a cell the order follows the changes, which no
//! reader depends on: the graded terrain orders its sites by content.

use super::model::BuildingSiteClient;
use std::collections::HashMap;

// Cell span, metres: a graded terrain tile, so a point query's window (its rectangle grown by
// the 64 m cap) meets three or four cells on each axis.
const SITE_CELL_M: f32 = 64.0;

type CellMap = HashMap<(i32, i32), Vec<CellEntry>, foldhash::fast::FixedState>;

// One site in a cell: its building index and the bounds of its footprint and surfaces
// (`BuildingSiteClient::bounds`), `[min_x, min_z, max_x, max_z]`.
#[derive(Clone, Copy, Debug)]
struct CellEntry {
    site: u32,
    bounds: [f32; 4],
}

/// The site clients by the 64 m cells their bounds meet.
#[derive(Clone, Debug, Default)]
pub(crate) struct SiteCells {
    cells: CellMap,
}

fn cell_of(value: f32) -> i32 {
    (value / SITE_CELL_M).floor() as i32
}

// The cells `[min x, min z, max x, max z]` that closed bounds meet; `None` for empty bounds (a
// site without corners), which meet no rectangle.
fn cell_range(bounds: [f32; 4]) -> Option<[i32; 4]> {
    (bounds[0] <= bounds[2] && bounds[1] <= bounds[3]).then(|| bounds.map(cell_of))
}

fn site_bounds(site: &BuildingSiteClient) -> [f32; 4] {
    let (min_x, min_z, max_x, max_z) = site.bounds();
    [min_x, min_z, max_x, max_z]
}

impl SiteCells {
    /// Indexes every site client by its building index. `O(sites x cells each meets)`.
    pub(crate) fn rebuild(&mut self, sites: &[BuildingSiteClient]) {
        self.cells.clear();
        for (index, site) in sites.iter().enumerate() {
            self.insert(index, site);
        }
    }

    pub(crate) fn clear(&mut self) {
        self.cells.clear();
    }

    /// Adds the site at building index `index`. `O(cells it meets)`.
    pub(crate) fn insert(&mut self, index: usize, site: &BuildingSiteClient) {
        let bounds = site_bounds(site);
        self.for_each_cell(bounds, |cells, key| {
            cells.entry(key).or_default().push(CellEntry {
                site: index as u32,
                bounds,
            });
        });
    }

    /// Removes the site at building index `index`, as it was indexed. `O(cells it meets x
    /// sites in each)`.
    pub(crate) fn remove(&mut self, index: usize, site: &BuildingSiteClient) {
        self.for_each_cell(site_bounds(site), |cells, key| {
            let Some(list) = cells.get_mut(&key) else {
                return;
            };
            if let Some(position) = list.iter().position(|entry| entry.site == index as u32) {
                list.swap_remove(position);
            }
            if list.is_empty() {
                cells.remove(&key);
            }
        });
    }

    /// Renames the site at building index `from`, as it was indexed, to `to`: a swap-remove's
    /// move of the last building. `O(cells it meets x sites in each)`.
    pub(crate) fn rename(&mut self, from: usize, to: usize, site: &BuildingSiteClient) {
        self.for_each_cell(site_bounds(site), |cells, key| {
            if let Some(entry) = cells
                .get_mut(&key)
                .and_then(|list| list.iter_mut().find(|entry| entry.site == from as u32))
            {
                entry.site = to as u32;
            }
        });
    }

    /// Visits once each site whose bounds meet the closed rectangle `[min_x, min_z, max_x,
    /// max_z]`, from the cell holding the lower corner of the overlap. `O(cells the rectangle
    /// meets + sites in them)`.
    pub(crate) fn visit(&self, rect: [f32; 4], mut visit: impl FnMut(usize)) {
        let Some([x0, z0, x1, z1]) = cell_range(rect) else {
            return;
        };
        for x in x0..=x1 {
            for z in z0..=z1 {
                let Some(list) = self.cells.get(&(x, z)) else {
                    continue;
                };
                for entry in list {
                    let b = entry.bounds;
                    if b[0] <= rect[2] && b[2] >= rect[0] && b[1] <= rect[3] && b[3] >= rect[1] {
                        // The overlap's lower corner lies in one cell both meet.
                        let first = (cell_of(rect[0].max(b[0])), cell_of(rect[1].max(b[1])));
                        if first == (x, z) {
                            visit(entry.site as usize);
                        }
                    }
                }
            }
        }
    }

    fn for_each_cell(&mut self, bounds: [f32; 4], mut visit: impl FnMut(&mut CellMap, (i32, i32))) {
        let Some([x0, z0, x1, z1]) = cell_range(bounds) else {
            return;
        };
        for x in x0..=x1 {
            for z in z0..=z1 {
                visit(&mut self.cells, (x, z));
            }
        }
    }
}
