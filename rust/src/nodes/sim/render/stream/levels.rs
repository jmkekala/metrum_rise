// SPDX-License-Identifier: GPL-2.0-only

//! The graded tiles' desired levels and their travel buckets (`TERRAIN-13` part 1, round G1;
//! `docs/terrain.md` "LOD and distant tiles", T26), moved from `terrain_graded_tiles.gd`
//! (`desired_level`, `_level_tile`, `_level_margin`, `_move_camera`) unchanged in behaviour.
//!
//! Every tile a resident patch lists has the level of its distance from the view, with
//! hysteresis around the level it is installed at (held). A tile's distance changes by at most
//! the camera's travel, so a tile levelled at travel `t`, whose distance is `m` from the nearest
//! distance where its level can change, keeps its level until the travel reaches `t + m`: it
//! waits in bucket `floor((t + m) / LEVEL_BUCKET_M)`, and a camera move re-levels the buckets the
//! travel reached since the last move, the last one included. Exact: every desired tile has its
//! distance's level after every move. A move costs `O(tiles due + buckets passed)`, a still
//! frame nothing; desiring, dropping or holding a tile is `O(1)`.

use super::view::StreamView;
use crate::simulation::terrain::graded::{Level, TileKey};
use godot::prelude::*;
use std::collections::HashMap;

type Map<K, V> = HashMap<K, V, foldhash::fast::FixedState>;

/// Where `GL0`, `GL1` and `GL2` end, metres from the camera (T5, TD10; tuned by the GPU idle
/// frame in `TERRAIN-07-D` round 8).
pub(crate) const LEVEL_END_M: [f64; 3] = [64.0, 128.0, 2000.0];
/// Where `GL3` ends at each step (the patch LOD rule's 2 to 5 km, 4 to 12 km, 8 beyond).
pub(crate) const GL3_END_M: [f64; 3] = [5000.0, 12000.0, f64::INFINITY];
/// `GL3`'s steps in base cells, in the order of [`GL3_END_M`].
pub(crate) const GL3_STEPS: [u8; 3] = [2, 4, 8];
// A level's end moves this far out for a tile held at that level or a finer one.
const LEVEL_HYSTERESIS_M: f64 = 32.0;
const LEVEL_BUCKET_M: f64 = 8.0;
// The distances are f64 from f32 positions; their rounding is far below this.
const LEVEL_MARGIN_SLACK_M: f64 = 1e-6;

/// Godot's level code (0-2 `GL0`-`GL2`, 3 `GL3`) and `GL3` step (0 below `GL3`).
pub(crate) fn level_code(level: Level) -> (i64, i64) {
    match level {
        Level::Gl0 => (0, 0),
        Level::Gl1 => (1, 0),
        Level::Gl2 => (2, 0),
        Level::Gl3 { step } => (3, i64::from(step)),
    }
}

/// The level of Godot's `code` and `step`, `None` for no level.
pub(crate) fn level_from_code(code: i64, step: i64) -> Option<Level> {
    match code {
        0 => Some(Level::Gl0),
        1 => Some(Level::Gl1),
        2 => Some(Level::Gl2),
        3 => GL3_STEPS
            .into_iter()
            .find(|&s| i64::from(s) == step)
            .map(|step| Level::Gl3 { step }),
        _ => None,
    }
}

// The end of level `index` (0-2) for a tile held at level code `held`.
fn level_end(index: usize, held: Option<u8>) -> f64 {
    let end = LEVEL_END_M[index];
    if held.is_some_and(|code| usize::from(code) <= index) {
        end + LEVEL_HYSTERESIS_M
    } else {
        end
    }
}

/// The level a tile takes at `distance` from the camera, with hysteresis around `held`, the
/// level code it is installed at (`None`: not installed).
pub(crate) fn desired_level(distance: f64, held: Option<u8>) -> Level {
    match (0..3).find(|&index| distance < level_end(index, held)) {
        Some(0) => Level::Gl0,
        Some(1) => Level::Gl1,
        Some(_) => Level::Gl2,
        None => {
            let index = GL3_END_M
                .iter()
                .position(|&end| distance < end)
                .unwrap_or(GL3_STEPS.len() - 1);
            Level::Gl3 {
                step: GL3_STEPS[index],
            }
        }
    }
}

// The distance from `distance` to the nearest distance where `desired_level(_, held)` changes.
fn level_margin(distance: f64, held: Option<u8>) -> f64 {
    let ends = (0..3)
        .map(|index| level_end(index, held))
        .chain(GL3_END_M);
    ends.map(|end| (distance - end).abs())
        .fold(f64::INFINITY, f64::min)
}

// One desired tile: its level, the level code it is held at, and its place in the buckets.
#[derive(Clone, Copy, Debug)]
struct Desired {
    level: Level,
    held: Option<u8>,
    bucket: i64,
    slot: u32,
}

// A tile in no bucket: being re-levelled, its bucket taken out.
const NO_BUCKET: i64 = -1;

/// The desired tiles with their levels, bucketed by the travel at which each can next change.
#[derive(Default)]
pub(crate) struct LevelBook {
    view: StreamView,
    tiles: Map<TileKey, Desired>,
    // Bucket -> its tiles; a tile's `slot` is its index there (swap-remove keeps them exact).
    buckets: Map<i64, Vec<TileKey>>,
    // Emptied bucket vectors, reused so a warm move allocates nothing.
    spare: Vec<Vec<TileKey>>,
    bucket_first: i64,
    // Desired tiles per level code.
    by_code: [usize; 4],
    // The last move's tiles whose level changed, and the tiles and buckets it visited.
    changed: Vec<TileKey>,
    relevelled: usize,
    buckets_visited: usize,
}

impl LevelBook {
    /// Adds `tile` (or levels it again) held at `held`. Returns its level.
    pub fn desire(&mut self, tile: TileKey, held: Option<u8>) -> Level {
        if let Some(old) = self.tiles.get(&tile).copied() {
            self.unbucket(old.bucket, old.slot);
            self.by_code[level_code(old.level).0 as usize] -= 1;
        }
        let level = desired_level(self.view.tile_distance(tile), held);
        self.by_code[level_code(level).0 as usize] += 1;
        self.tiles.insert(
            tile,
            Desired {
                level,
                held,
                bucket: NO_BUCKET,
                slot: 0,
            },
        );
        self.level(tile);
        level
    }

    /// Drops `tile`. Returns whether it was desired.
    pub fn undesire(&mut self, tile: TileKey) -> bool {
        let Some(old) = self.tiles.remove(&tile) else {
            return false;
        };
        self.unbucket(old.bucket, old.slot);
        self.by_code[level_code(old.level).0 as usize] -= 1;
        true
    }

    /// Records that `tile` is installed at level code `held` (`None`: evicted) and levels it
    /// again, the hysteresis having moved. Returns its level and whether that changed; `None`
    /// for a tile not desired.
    pub fn hold(&mut self, tile: TileKey, held: Option<u8>) -> Option<(Level, bool)> {
        let entry = self.tiles.get_mut(&tile)?;
        entry.held = held;
        let before = entry.level;
        self.level(tile);
        let level = self.tiles[&tile].level;
        Some((level, level != before))
    }

    /// The level of `tile`, `None` for a tile not desired.
    pub fn level_of(&self, tile: TileKey) -> Option<Level> {
        self.tiles.get(&tile).map(|entry| entry.level)
    }

    /// The level the rule gives `tile` held at `held` from the current camera, evaluated
    /// directly (tests compare the buckets against it).
    pub fn rule_level(&self, tile: TileKey, held: Option<u8>) -> Level {
        desired_level(self.view.tile_distance(tile), held)
    }

    /// Moves the camera and re-levels the tiles due. Returns the tiles whose level changed, in
    /// the order they were re-levelled. `O(tiles due + buckets passed)`; nothing when the
    /// camera did not move.
    pub fn move_camera(&mut self, camera: [f64; 3]) -> &[TileKey] {
        self.changed.clear();
        self.relevelled = 0;
        self.buckets_visited = 0;
        if self.view.move_to(camera).is_none() {
            return &self.changed;
        }
        let last = (self.view.travel() / LEVEL_BUCKET_M).floor() as i64;
        for bucket in self.bucket_first..=last {
            self.buckets_visited += 1;
            let Some(mut due) = self.buckets.remove(&bucket) else {
                continue;
            };
            // A tile due again within this bucket goes into a new one, levelled next move.
            for tile in due.drain(..) {
                let entry = self.tiles.get_mut(&tile).expect("a bucketed tile is desired");
                entry.bucket = NO_BUCKET;
                let before = entry.level;
                self.level(tile);
                self.relevelled += 1;
                if self.tiles[&tile].level != before {
                    self.changed.push(tile);
                }
            }
            self.spare.push(due);
        }
        self.bucket_first = last;
        &self.changed
    }

    /// The tiles the last move re-levelled and the buckets it visited (both 0 on a still
    /// frame).
    pub fn last_move_work(&self) -> (usize, usize) {
        (self.relevelled, self.buckets_visited)
    }

    /// The desired tiles.
    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    /// The desired tiles per level code (0-3).
    pub fn by_code(&self) -> [usize; 4] {
        self.by_code
    }

    /// Every desired tile, in no particular order (tests and diagnostics).
    pub fn tiles(&self) -> impl Iterator<Item = TileKey> + '_ {
        self.tiles.keys().copied()
    }

    /// The desired tiles whose level differs from the rule evaluated directly at their held
    /// level: 0 by construction. `O(desired tiles)`, for tests.
    #[cfg(test)]
    pub fn mismatches(&self) -> usize {
        self.tiles
            .iter()
            .filter(|(tile, entry)| entry.level != self.rule_level(**tile, entry.held))
            .count()
    }

    /// Drops every tile and the camera.
    pub fn clear(&mut self) {
        self.tiles.clear();
        for (_, mut tiles) in self.buckets.drain() {
            tiles.clear();
            self.spare.push(tiles);
        }
        self.view = StreamView::default();
        self.bucket_first = 0;
        self.by_code = [0; 4];
        self.changed.clear();
    }

    // Levels `tile` at the camera's distance and puts it in the bucket of the travel at which
    // its level can first change.
    fn level(&mut self, tile: TileKey) {
        let distance = self.view.tile_distance(tile);
        let entry = self.tiles[&tile];
        let level = desired_level(distance, entry.held);
        if level != entry.level {
            self.by_code[level_code(entry.level).0 as usize] -= 1;
            self.by_code[level_code(level).0 as usize] += 1;
        }
        let margin = (level_margin(distance, entry.held) - LEVEL_MARGIN_SLACK_M).max(0.0);
        let bucket = ((self.view.travel() + margin) / LEVEL_BUCKET_M).floor() as i64;
        let mut slot = entry.slot;
        if bucket != entry.bucket {
            self.unbucket(entry.bucket, entry.slot);
            let spare = &mut self.spare;
            let tiles = self
                .buckets
                .entry(bucket)
                .or_insert_with(|| spare.pop().unwrap_or_default());
            slot = tiles.len() as u32;
            tiles.push(tile);
        }
        let entry = self.tiles.get_mut(&tile).expect("levelled tiles are desired");
        entry.level = level;
        entry.bucket = bucket;
        entry.slot = slot;
    }

    // Takes the tile at `slot` out of `bucket`; the bucket's last tile takes its slot.
    fn unbucket(&mut self, bucket: i64, slot: u32) {
        if bucket == NO_BUCKET {
            return;
        }
        let tiles = self.buckets.get_mut(&bucket).expect("a tile's bucket exists");
        tiles.swap_remove(slot as usize);
        if let Some(&moved) = tiles.get(slot as usize) {
            self.tiles
                .get_mut(&moved)
                .expect("a bucketed tile is desired")
                .slot = slot;
        }
        if tiles.is_empty() {
            let tiles = self.buckets.remove(&bucket).expect("present");
            self.spare.push(tiles);
        }
    }
}

// Godot's level as `Vector2i(code, step)`, `(-1, -1)` for none.
fn level_vector(level: Option<Level>) -> Vector2i {
    level.map_or(Vector2i::new(-1, -1), |level| {
        let (code, step) = level_code(level);
        Vector2i::new(code as i32, step as i32)
    })
}

fn held_code(held: i64) -> Option<u8> {
    u8::try_from(held).ok().filter(|&code| code < 4)
}

fn tile_key(tile: Vector2i) -> TileKey {
    TileKey {
        x: tile.x,
        z: tile.y,
    }
}

/// The graded tile renderer's desired levels (`terrain_graded_tiles.gd`, `TERRAIN-13` part 1):
/// the tiles its resident patches list, each at its distance's level with hysteresis around the
/// level it is installed at, re-levelled by travel bucket as the camera moves. Levels are
/// `Vector2i(code, step)`: codes 0-3 for `GL0`-`GL3`, the `GL3` step in base cells (0 below
/// `GL3`), `(-1, -1)` for a tile not desired. Held levels are codes, -1 for none.
#[derive(GodotClass)]
#[class(init, base = RefCounted)]
pub struct GradedTileLevels {
    book: LevelBook,
}

#[godot_api]
impl GradedTileLevels {
    /// Desires `tile` held at level code `held`; returns its level. `O(1)`.
    #[func]
    pub fn desire(&mut self, tile: Vector2i, held: i64) -> Vector2i {
        level_vector(Some(self.book.desire(tile_key(tile), held_code(held))))
    }

    /// Drops `tile`; false when it was not desired. `O(1)`.
    #[func]
    pub fn undesire(&mut self, tile: Vector2i) -> bool {
        self.book.undesire(tile_key(tile))
    }

    /// Records that `tile` is now installed at level code `held` (-1: evicted) and levels it
    /// again. Returns `Vector3i(code, step, changed)`, `changed` 1 where the level changed;
    /// `(-1, -1, 0)` for a tile not desired. `O(1)`.
    #[func]
    pub fn hold(&mut self, tile: Vector2i, held: i64) -> Vector3i {
        match self.book.hold(tile_key(tile), held_code(held)) {
            Some((level, changed)) => {
                let level = level_vector(Some(level));
                Vector3i::new(level.x, level.y, i32::from(changed))
            }
            None => Vector3i::new(-1, -1, 0),
        }
    }

    /// The level of `tile`. `O(1)`.
    #[func]
    pub fn level_of(&self, tile: Vector2i) -> Vector2i {
        level_vector(self.book.level_of(tile_key(tile)))
    }

    /// Whether `tile` is desired. `O(1)`.
    #[func]
    pub fn has(&self, tile: Vector2i) -> bool {
        self.book.level_of(tile_key(tile)).is_some()
    }

    /// Moves the camera (XZ, with its height above the ground as `y`) and re-levels the tiles
    /// due. Returns the tiles whose level changed as flat `(x, z)` pairs (the first camera and
    /// an unchanged one re-level nothing). `O(tiles due + buckets passed)`.
    #[func]
    pub fn move_camera(&mut self, camera: Vector3) -> PackedInt32Array {
        let changed = self
            .book
            .move_camera([camera.x, camera.y, camera.z].map(f64::from));
        if changed.is_empty() {
            return PackedInt32Array::new();
        }
        changed.iter().flat_map(|tile| [tile.x, tile.z]).collect()
    }

    /// The tiles the last move re-levelled.
    #[func]
    pub fn relevelled(&self) -> i64 {
        self.book.last_move_work().0 as i64
    }

    /// The desired tiles.
    #[func]
    pub fn size(&self) -> i64 {
        self.book.len() as i64
    }

    /// The desired tiles per level code (0-3).
    #[func]
    pub fn count_by_code(&self) -> PackedInt32Array {
        self.book.by_code().map(|count| count as i32).into_iter().collect()
    }

    /// Every desired tile as flat `(x, z)` pairs, in no particular order. `O(desired tiles)`,
    /// for tests and diagnostics.
    #[func]
    pub fn tiles(&self) -> PackedInt32Array {
        self.book.tiles().flat_map(|tile| [tile.x, tile.z]).collect()
    }

    /// The level the rule gives `tile` held at level code `held` (-1: none) from the current
    /// camera, evaluated directly. For tests.
    #[func]
    pub fn rule_level(&self, tile: Vector2i, held: i64) -> Vector2i {
        level_vector(Some(self.book.rule_level(tile_key(tile), held_code(held))))
    }

    /// Where `GL0`-`GL2` end, for the road preview's view (`set_road_preview_terrain_view`).
    #[func]
    pub fn level_end_m() -> PackedFloat64Array {
        PackedFloat64Array::from(LEVEL_END_M.as_slice())
    }

    /// Where `GL3` ends at steps 2, 4 and 8 (the last infinite).
    #[func]
    pub fn gl3_end_m() -> PackedFloat64Array {
        PackedFloat64Array::from(GL3_END_M.as_slice())
    }

    /// Drops every tile and the camera, as the renderer resets.
    #[func]
    pub fn clear(&mut self) {
        self.book.clear();
    }
}

#[cfg(test)]
mod tests;
