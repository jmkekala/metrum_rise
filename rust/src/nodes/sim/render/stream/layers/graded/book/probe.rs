// SPDX-License-Identifier: GPL-2.0-only

//! The graded book's part of the stream benchmark's drawn probe (`RENDER-32-N`,
//! [`crate::nodes::sim::render::stream::probe`]): its resident patches with what they draw, and
//! its desired tiles with what is installed for them. Benchmark only.

use super::super::cells::DRAW_NONE;
use super::super::state::TILE_M;
use super::GradedTileBook;
use crate::nodes::sim::render::stream::levels::level_code;
use crate::nodes::sim::render::stream::probe::{COVERED, DRAWN, READY, push_chunk};
use godot::prelude::PackedFloat32Array;

impl GradedTileBook {
    /// The resident patches: drawn while their draw state is not `DRAW_NONE` (T53 (a): the
    /// patch draws its set or its plane), ready when every tile their list names is installed or
    /// cannot show (the patch waits for its set's build only). `O(resident patches' lists)`.
    pub(crate) fn probe_patches(&self) -> PackedFloat32Array {
        let state = &self.state;
        let mut out = Vec::with_capacity(
            state.resident.len() * crate::nodes::sim::render::stream::probe::LANES,
        );
        for (&key, patch) in state.resident.iter() {
            let drawn = state.draw_state(key) != DRAW_NONE;
            let ready = state.lists.get(&key).is_some_and(|(_, tiles)| {
                tiles
                    .iter()
                    .all(|tile| state.installed.contains_key(tile) || state.refused_at_level(*tile))
            });
            let flags = if drawn { DRAWN } else { 0 } | if ready { READY } else { 0 };
            let [x, z, width, depth] = patch.rect;
            push_chunk(
                &mut out,
                [key.0, key.1, 0],
                flags,
                [x, z, x + width, z + depth],
            );
        }
        PackedFloat32Array::from(out.as_slice())
    }

    /// The desired tiles: drawn when installed at their desired level, covered when installed at
    /// another (the tile shows, at the other level's detail). `O(desired tiles)`.
    pub(crate) fn probe_tiles(&self) -> PackedFloat32Array {
        let state = &self.state;
        let mut out = Vec::new();
        for tile in state.levels.tiles() {
            let Some(level) = state.levels.level_of(tile) else {
                continue;
            };
            let (code, step) = level_code(level);
            let flags = state.installed.get(&tile).map_or(0, |entry| {
                let at_level =
                    i64::from(entry.level) == code && (code < 3 || i64::from(entry.step) == step);
                if at_level { DRAWN } else { COVERED }
            });
            let [x, z] = [tile.x as f32 * TILE_M, tile.z as f32 * TILE_M];
            push_chunk(
                &mut out,
                [tile.x, tile.z, code as i32],
                flags,
                [x, z, x + TILE_M, z + TILE_M],
            );
        }
        PackedFloat32Array::from(out.as_slice())
    }
}
