// SPDX-License-Identifier: GPL-2.0-only

//! What a resident patch draws (`RENDER-32-J`; T53 (a), decided by the user 2026-10-28), ported
//! from `terrain_graded_tiles.gd` (`_draw_state`, `_apply_shown`, `_previews_in`) and the
//! checks the tests and the drag probe read every frame (`partial_sets`, the state half of
//! `tile_draw_violations`).
//!
//! A patch draws its PlaneMesh only while its list is known to name no tile and no preview tile
//! lies in it; otherwise its set once the set is built and shows every tile its list names, and
//! nothing until then. A listed tile that cannot show (refused, or its pool exhausted) draws its
//! base cell. A set that drew once in its patch's residency (`complete`) keeps drawing while its
//! list is refreshed. Bounds: `O(the patch's list)` while the set is not complete, else `O(1)`,
//! plus the preview's tiles.

use super::cells::{CELL_BASE, DRAW_NONE, DRAW_PLANE, DRAW_SET};
use super::state::{Effect, Graded, PatchKey, TILE_M, Tile};

impl Graded {
    /// `_previews_in`: whether a preview tile meets the patch rectangle `rect`.
    pub(crate) fn previews_in(&self, rect: [f32; 4]) -> bool {
        if self.preview.len() == 0 {
            return false;
        }
        let first = [rect[0], rect[1]].map(|value| (value / TILE_M).floor() as i32);
        let last = [rect[0] + rect[2], rect[1] + rect[3]].map(|value| (value / TILE_M).ceil() as i32 - 1);
        self.preview
            .keys()
            .any(|tile| tile.x >= first[0] && tile.z >= first[1] && tile.x <= last[0] && tile.z <= last[1])
    }

    /// `_draw_state`: what resident patch `key` draws.
    pub(crate) fn draw_state(&self, key: PatchKey) -> u8 {
        let patch = self.resident.get(&key).expect("a resident patch");
        let list = self.lists.get(&key);
        if list.is_none() && !self.lists_refused.contains_key(&key) {
            return DRAW_NONE;
        }
        let tiles: &[Tile] = list.map_or(&[], |(_, tiles)| tiles);
        let set = self.cells.by_patch.get(&key).map(|&set| &self.cells.sets[set as usize]);
        let built = set.is_some_and(|set| set.is_built());
        let graded = built && set.is_some_and(|set| set.graded_cells() > 0);
        let refreshing = list.is_some_and(|(token, _)| *token != patch.generation);
        let previews = self.previews_in(patch.rect);
        if tiles.is_empty() && !(refreshing && graded) && !previews {
            return DRAW_PLANE;
        }
        if graded && set.is_some_and(|set| set.complete) {
            return DRAW_SET;
        }
        let mut showing = previews || (refreshing && graded);
        for &tile in tiles {
            if self.shown_in(tile, key).is_some() {
                showing = true;
            } else if !self.refused_at_level(tile) {
                return DRAW_NONE;
            }
        }
        if !showing {
            return DRAW_PLANE;
        }
        let Some(set) = set.filter(|_| graded && built) else {
            return DRAW_NONE;
        };
        // Every installed tile the patch draws is copied into its block's page: a border tile's
        // second copy may arrive frames after the first, and the two halves must not differ.
        for &tile in &set.tiles {
            if self.installed.contains_key(&tile) && self.shown_in(tile, key).is_none() && !self.refused_at_level(tile) {
                return DRAW_NONE;
            }
        }
        DRAW_SET
    }

    /// `_apply_shown`: patch `key` draws what `draw_state` says; a set starting or stopping to
    /// draw rewrites the cells across its sides in the same frame.
    pub(crate) fn apply_shown(&mut self, key: PatchKey) {
        if !self.resident.contains_key(&key) {
            return;
        }
        let state = self.draw_state(key);
        self.out.push(Effect::Draw { key, state });
        let set = self.cells.by_patch.get(&key).copied();
        let built = set.is_some_and(|set| self.cells.sets[set as usize].is_built());
        let listed = self.lists.get(&key).is_some_and(|(_, tiles)| !tiles.is_empty());
        if state == DRAW_NONE && !built && listed {
            self.cells.builds_urgent.insert(key, ());
        }
        let Some(set) = set else {
            return;
        };
        let shown = self.cells.sets[set as usize].shown;
        if state == DRAW_PLANE {
            self.cells.sets[set as usize].complete = false;
        }
        if (state == DRAW_SET) == shown {
            return;
        }
        self.cells.frame[1] += 1;
        self.border_changed(key);
        if shown {
            self.hide(set);
            return;
        }
        let cell_set = &mut self.cells.sets[set as usize];
        cell_set.shown = true;
        cell_set.complete = true;
        cell_set.base_mesh = Some(super::cells::base_code(cell_set.step).min(6));
        self.out.push(Effect::CellShow { set, key });
    }

    /// `partial_sets`: drawing sets whose list names a tile they do not show (not installed,
    /// not previewed, not refused). For tests of a streaming run without edits.
    pub(crate) fn partial_sets(&self) -> usize {
        self.resident
            .keys()
            .filter(|key| {
                self.cells
                    .by_patch
                    .get(key)
                    .is_some_and(|&set| self.cells.sets[set as usize].shown)
                    && self.lists.get(key).is_some_and(|(_, tiles)| {
                        tiles
                            .iter()
                            .any(|&tile| self.shown_in(tile, **key).is_none() && !self.refused_at_level(tile))
                    })
            })
            .count()
    }

    /// The state half of `tile_draw_violations`: per drawing built set of a resident patch, each
    /// of its tiles must be one instance in the mesh of what it shows there, its custom data
    /// what it would be written as now, a graded instance's slot holding the tile's copy in the
    /// page the set binds. Returns (holes, stitches: tiles with only a stale side code).
    pub(crate) fn draw_violations(&self) -> (usize, usize) {
        let (mut holes, mut stitches) = (0, 0);
        for (&key, patch) in self.resident.iter() {
            let Some(&set) = self.cells.by_patch.get(&key) else {
                continue;
            };
            let set = &self.cells.sets[set as usize];
            if !set.shown || !set.is_built() {
                continue;
            }
            if set.counts.iter().sum::<u32>() as usize != set.tiles.len() {
                holes += 1;
            }
            for &tile in &set.tiles {
                let (mesh, index) = set.at[set.place(tile)];
                let want_mesh = self.cell_mesh(tile, key);
                if mesh as usize != want_mesh
                    || index < 0
                    || index as u32 >= set.counts[mesh as usize]
                    || set.tile_at[mesh as usize][index as usize] != tile
                {
                    holes += 1;
                    continue;
                }
                let custom = set.custom[mesh as usize][index as usize];
                let want = self.cell_custom(tile, set.first, patch, key);
                if want_mesh != CELL_BASE {
                    let entry = self.shown_in(tile, key).expect("a graded cell shows an entry");
                    let page = self.cells.bound[usize::from(entry.level)]
                        .get(&self.block_of(key, entry.level))
                        .map_or(-1, |&page| i64::from(page));
                    if set.pages[usize::from(entry.level)] != page
                        || page < 0
                        || entry.copy_in(page as u32).is_none()
                        || custom[1] as i64 != want[1] as i64
                    {
                        holes += 1;
                        continue;
                    }
                }
                if custom != want {
                    if custom[0] as i64 != want[0] as i64 || (custom[2] as i64) & 7 != (want[2] as i64) & 7 {
                        holes += 1;
                    } else {
                        stitches += 1;
                    }
                }
            }
        }
        (holes, stitches)
    }
}
