// SPDX-License-Identifier: GPL-2.0-only

//! Staged tiles for the road tool's preview poses, the commit and undo (`TERRAIN-07-E`,
//! `docs/terrain.md` "Preview, commit, undo and load"): baked tiles held with the inputs they
//! were baked from, and the one rule the three paths share. A wanted tile whose inputs equal a
//! held tile's at the same level is that tile (adopted); every other wanted tile is baked.
//! Inputs decide reuse, never time, order or edit history, so a staged tile is bitwise the
//! fresh bake of the inputs it holds (invariant 9). Nothing here outlives its caller's stage:
//! no cache.

use std::sync::Arc;
use std::time::Instant;

use super::select::{SelectScratch, dirty_tiles_for_road_changes_where, road_change_extent};
use super::sites::SiteSource;
use super::tile::{BakeError, Level, TILE_M, TileKey, TilePayload, bake_unless_held};
use crate::simulation::network::roadbed::{RoadbedSystem, TerrainChanges};
use crate::simulation::terrain::TerrainSystem;

/// One baked tile at one level, shared between the stages that hold it.
#[derive(Clone, Debug)]
pub struct StagedTile {
    /// The level the tile was baked at.
    pub level: Level,
    /// The tile.
    pub tile: TileKey,
    /// Its bake, with the inputs it came from.
    pub payload: Arc<TilePayload>,
}

/// A set of baked tiles in `(level, tile)` order: a preview pose's staging, a commit's tiles
/// or an undo record's kept tiles.
#[derive(Clone, Debug, Default)]
pub struct TileStage {
    tiles: Vec<StagedTile>,
}

impl TileStage {
    /// The tiles in `(level, tile)` order.
    pub fn tiles(&self) -> &[StagedTile] {
        &self.tiles
    }

    /// The held tile at `level`, if any. `O(log n)`.
    pub fn get(&self, level: Level, tile: TileKey) -> Option<&Arc<TilePayload>> {
        self.tiles
            .binary_search_by(|held| (held.level, held.tile).cmp(&(level, tile)))
            .ok()
            .map(|index| &self.tiles[index].payload)
    }

    /// Number of tiles.
    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    /// Whether the stage holds no tile.
    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// The tiles of `wanted` (`(level, tile)` sorted and unique) that `over` holds, else those
    /// `under` holds; a wanted tile neither holds is left out, and so is every tile outside
    /// `wanted` (`TERRAIN-07-E` T27 (d): a preview frame's newly staged tiles over the tiles it
    /// showed). The payloads are shared. `O(wanted log n)`.
    pub fn overlay(wanted: &[(Level, TileKey)], over: &Self, under: &Self) -> Self {
        debug_assert!(wanted.is_sorted(), "wanted tiles in (level, tile) order");
        let tiles = wanted
            .iter()
            .filter_map(|&(level, tile)| {
                let payload = over.get(level, tile).or_else(|| under.get(level, tile))?;
                Some(StagedTile {
                    level,
                    tile,
                    payload: Arc::clone(payload),
                })
            })
            .collect();
        Self { tiles }
    }

    /// Bytes of the held texels and rows: what a stage keeps at rest.
    pub fn payload_bytes(&self) -> usize {
        self.tiles
            .iter()
            .map(|held| {
                let payload = &held.payload;
                16 * (payload.texels.len() + payload.seams.len() + payload.edges.len())
            })
            .sum()
    }
}

/// What one staging did, with its phases' wall-clock times.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StageStats {
    /// Tiles wanted at every level.
    pub wanted: usize,
    /// Wanted tiles taken from the held stage, inputs equal.
    pub adopted: usize,
    /// Wanted tiles baked.
    pub baked: usize,
    /// The dirty tile selection, milliseconds (0 when the caller supplies the tiles).
    pub select_ms: f64,
    /// The wanted tiles' bakes, milliseconds: one pass whose bakes gather each tile's inputs
    /// first and stop for an adopted tile (`TERRAIN-07-E` round 3).
    pub bake_ms: f64,
}

impl StageStats {
    /// The stage's wall-clock time, milliseconds.
    pub fn total_ms(&self) -> f64 {
        self.select_ms + self.bake_ms
    }
}

/// Every level a tile can take, finest first.
pub const ALL_LEVELS: [Level; 6] = [
    Level::Gl0,
    Level::Gl1,
    Level::Gl2,
    Level::Gl3 { step: 2 },
    Level::Gl3 { step: 4 },
    Level::Gl3 { step: 8 },
];

/// The renderer's view an edit's tiles are staged for (`TERRAIN-07-E`, T42): the camera and
/// where each level ends, as the renderer's distance rule reads them (`docs/terrain.md`
/// "Residency and eviction").
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LevelView {
    /// The camera's XZ, with its height above the ground under it as the middle value.
    pub camera: [f64; 3],
    /// The distances where `GL0`, `GL1` and `GL2` end.
    pub level_end_m: [f64; 3],
    /// Where `GL3` at each step (2, 4, 8) ends; the last should be infinite.
    pub gl3_end_m: [f64; 3],
}

/// The level each tile of an edit is staged at (`TERRAIN-07-E`, T42): the renderer's distance
/// rule from its last reported camera, without the hysteresis it keeps around a held tile's
/// level, so the preview, the commit and the renderer's next request agree on a tile's level;
/// without a renderer (`None`: tests and benchmarks) every tile is staged at `GL0`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LevelRule {
    /// The renderer's view, if one reported it.
    pub view: Option<LevelView>,
}

impl LevelRule {
    // The camera's distance to the tile's centre on the ground, as the renderer measures it.
    fn distance(view: &LevelView, tile: TileKey) -> f64 {
        let [x, z] = tile.origin();
        let [cx, height, cz] = view.camera;
        let (dx, dz) = (x + 0.5 * TILE_M - cx, z + 0.5 * TILE_M - cz);
        (dx * dx + height * height + dz * dz).sqrt()
    }

    // The level at `distance` and the distances where it starts and ends.
    fn level_at(view: &LevelView, distance: f64) -> Level {
        let mut index = 0;
        while index < ALL_LEVELS.len() - 1 {
            let end = match index {
                0..3 => view.level_end_m[index],
                _ => view.gl3_end_m[index - 3],
            };
            if distance < end {
                break;
            }
            index += 1;
        }
        ALL_LEVELS[index]
    }

    /// The level `tile` is staged at.
    pub fn level_of(&self, tile: TileKey) -> Level {
        match &self.view {
            Some(view) => Self::level_at(view, Self::distance(view, tile)),
            None => Level::Gl0,
        }
    }

    /// Whether a tile whose square lies within `rect` may take `level`: the level's distance
    /// interval meets the rectangle's. `O(1)`.
    pub fn may_take(&self, level: Level, rect: [f64; 4]) -> bool {
        let Some(view) = &self.view else {
            return level == Level::Gl0;
        };
        let [cx, height, cz] = view.camera;
        let near_x = (rect[0] - cx).max(cx - rect[2]).max(0.0);
        let near_z = (rect[1] - cz).max(cz - rect[3]).max(0.0);
        let far_x = (cx - rect[0]).abs().max((rect[2] - cx).abs());
        let far_z = (cz - rect[1]).abs().max((rect[3] - cz).abs());
        let near = (near_x * near_x + height * height + near_z * near_z).sqrt();
        let far = (far_x * far_x + height * height + far_z * far_z).sqrt();
        // The levels are monotone in distance: those from the nearest point's to the farthest's.
        (Self::level_at(view, near)..=Self::level_at(view, far)).contains(&level)
    }
}

/// Reused buffers of the staging. Payloads a stage drops are kept until nothing else shares
/// them and their buffers reused by later bakes, so a drag allocates no texel buffer once warm.
/// A stage frees no payload on its thread (`TERRAIN-07-G` round 9): what the bound below leaves
/// over is freed on the background pool.
#[derive(Default)]
pub struct StageScratch {
    select: SelectScratch,
    dirty: Vec<TileKey>,
    wanted: Vec<(Level, TileKey)>,
    adopted: Vec<Option<Arc<TilePayload>>>,
    // The bake buffers: a stage's bakes take the first slots, one per wanted tile; the slots
    // past them, and an adopted tile's, keep their buffers for later bakes.
    baked: Vec<TilePayload>,
    retired: Vec<Arc<TilePayload>>,
}

// Retired payloads, and spare bake buffers, kept for reuse at most: a drag's few poses' worth
// of `GL0` tiles.
const RETIRED_MAX: usize = 64;

// Frees `items` on the background pool. A payload freed on a stage's thread can have the
// allocator hand its pages back to the system there, inside the frame (`TERRAIN-07-G` round 9:
// glibc trimmed the heap under the stage, 1.2 ms of one profiled 30k undo).
fn free_off_thread<T: Send + 'static>(items: Vec<T>) {
    if !items.is_empty() {
        crate::thread_pools::spawn(move || drop(items));
    }
}

impl StageScratch {
    /// The tiles a road change from `before` to `after` can alter, each at the level `rule`
    /// gives it (the dirty tile selection, [`super::dirty_tiles_for_road_changes`], at each
    /// level some candidate may take, over the candidates that take it), in `(level, tile)`
    /// order, kept in the scratch for [`stage_tiles`]. `O(one selection)`: each candidate is
    /// tested once, at its own level.
    #[allow(clippy::too_many_arguments)]
    pub fn select_road_edit(
        &mut self,
        terrain: &TerrainSystem,
        before: &RoadbedSystem,
        after: &RoadbedSystem,
        sites: &impl SiteSource,
        changes: &TerrainChanges,
        rule: &LevelRule,
    ) -> &[(Level, TileKey)] {
        self.wanted.clear();
        let cell_m = f64::from(terrain.cell_size_m());
        for level in ALL_LEVELS {
            let Some(extent) = road_change_extent(changes, level, cell_m) else {
                break;
            };
            if !rule.may_take(level, extent) {
                continue;
            }
            dirty_tiles_for_road_changes_where(
                terrain,
                before,
                after,
                sites,
                changes,
                level,
                |tile| rule.level_of(tile) == level,
                &mut self.select,
                &mut self.dirty,
            );
            self.wanted
                .extend(self.dirty.iter().map(|&tile| (level, tile)));
        }
        &self.wanted
    }

    /// The tiles the last [`Self::select_road_edit`] selected, in `(level, tile)` order.
    pub fn selected(&self) -> &[(Level, TileKey)] {
        &self.wanted
    }

    /// Hands a stage that is no longer held to the scratch: its tiles that `kept` does not hold
    /// too are reused once nothing else shares them.
    pub fn retire(&mut self, stage: TileStage, kept: &TileStage) {
        for held in stage.tiles {
            let shared = kept
                .get(held.level, held.tile)
                .is_some_and(|other| Arc::ptr_eq(other, &held.payload));
            if !shared {
                self.retired.push(held.payload);
            }
        }
        if self.retired.len() > RETIRED_MAX {
            let excess = self.retired.len() - RETIRED_MAX;
            free_off_thread(self.retired.drain(..excess).collect());
        }
    }

    // Moves the retired payloads nothing else shares into the bake buffers, after the buffers
    // the last stage left unused. `O(bake buffers + retired)`; frees nothing here.
    fn recycle(&mut self) {
        // A slot whose payload a bake took holds no buffer: dropping it frees nothing.
        self.baked.retain(|payload| payload.texels.capacity() > 0);
        let unshared = self
            .retired
            .extract_if(.., |payload| Arc::get_mut(payload).is_some());
        self.baked.extend(unshared.filter_map(Arc::into_inner));
        if self.baked.len() > RETIRED_MAX {
            free_off_thread(self.baked.split_off(RETIRED_MAX));
        }
    }
}

/// Stages the tiles the last [`StageScratch::select_road_edit`] selected over the terrain,
/// `roadbed` and `sites`: [`stage_tiles`] with those tiles.
pub fn stage_selected(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    sites: &impl SiteSource,
    held: &TileStage,
    scratch: &mut StageScratch,
) -> Result<(TileStage, StageStats), BakeError> {
    let wanted = std::mem::take(&mut scratch.wanted);
    let staged = stage_tiles(terrain, roadbed, sites, &wanted, held, scratch);
    scratch.wanted = wanted;
    staged
}

/// Stages `wanted` (`(level, tile)` sorted and unique) over the terrain, `roadbed` and
/// `sites` in one pass ([`bake_unless_held`]): each wanted tile's bake gathers its inputs first;
/// a tile `held` holds at the same level with equal inputs is adopted (the same payload,
/// shared) and not sampled, the others are baked. The result is bitwise the bake of every
/// wanted tile, whatever `held` holds. `O(wanted gathers + rebaked bakes)`, in parallel on the
/// shared pool.
pub fn stage_tiles(
    terrain: &TerrainSystem,
    roadbed: &RoadbedSystem,
    sites: &impl SiteSource,
    wanted: &[(Level, TileKey)],
    held: &TileStage,
    scratch: &mut StageScratch,
) -> Result<(TileStage, StageStats), BakeError> {
    debug_assert!(wanted.is_sorted(), "wanted tiles in (level, tile) order");
    let mut stats = StageStats {
        wanted: wanted.len(),
        ..StageStats::default()
    };
    let bake_start = Instant::now();
    scratch.adopted.clear();
    scratch.adopted.extend(
        wanted
            .iter()
            .map(|&(level, tile)| held.get(level, tile).cloned()),
    );
    scratch.recycle();
    bake_unless_held(
        terrain,
        roadbed,
        sites,
        wanted,
        &mut scratch.adopted,
        &mut scratch.baked,
    )?;
    let mut tiles = Vec::with_capacity(wanted.len());
    let staged = wanted
        .iter()
        .zip(&mut scratch.adopted)
        .zip(&mut scratch.baked);
    for ((&(level, tile), adopted), baked) in staged {
        let payload = match adopted.take() {
            Some(payload) => {
                stats.adopted += 1;
                payload
            }
            None => {
                stats.baked += 1;
                Arc::new(std::mem::take(baked))
            }
        };
        tiles.push(StagedTile {
            level,
            tile,
            payload,
        });
    }
    stats.bake_ms = bake_start.elapsed().as_secs_f64() * 1000.0;
    Ok((TileStage { tiles }, stats))
}

#[cfg(test)]
impl StageScratch {
    // The texel buffers of the bake buffers held for later bakes, in slot order.
    fn spare_texels(&self) -> Vec<*const [f32; 4]> {
        self.baked
            .iter()
            .filter(|payload| payload.texels.capacity() > 0)
            .map(|payload| payload.texels.as_ptr())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::NoSites;
    use super::super::tests::{compiled, fixtures};
    use super::*;

    // `TERRAIN-07-G` round 9 (the 30k undo stall): a stage frees no bake buffer on its thread.
    // The buffers of retired tiles that a stage leaves unused, because it adopts every tile,
    // stay in the scratch for later bakes through any number of such stages; freeing them in the
    // stage had glibc trim the heap there (madvise, 1.2 ms in one profiled undo).
    #[test]
    fn stages_that_adopt_every_tile_keep_the_spare_bake_buffers() {
        let (_, graph, terrain) = fixtures().remove(0);
        let roadbed = compiled(&graph);
        let wanted: Vec<(Level, TileKey)> = (-2..1)
            .flat_map(|x| (-2..1).map(move |z| (Level::Gl0, TileKey { x, z })))
            .collect();
        let mut scratch = StageScratch::default();
        let none = TileStage::default();
        let stage = |held: &TileStage, scratch: &mut StageScratch| {
            stage_tiles(&terrain, &roadbed, &NoSites, &wanted, held, scratch).unwrap()
        };
        let (first, _) = stage(&none, &mut scratch);
        let (second, stats) = stage(&none, &mut scratch);
        assert_eq!(stats.baked, wanted.len());
        scratch.retire(first, &second);
        let (third, stats) = stage(&second, &mut scratch);
        assert_eq!(stats.adopted, wanted.len());
        let spares = scratch.spare_texels();
        assert_eq!(spares.len(), wanted.len(), "the retired tiles' buffers");
        scratch.retire(second, &third);
        for _ in 0..3 {
            let (next, stats) = stage(&third, &mut scratch);
            assert_eq!(stats.adopted, wanted.len());
            assert!(
                scratch.spare_texels().starts_with(&spares),
                "the spares are kept"
            );
            scratch.retire(next, &third);
        }
    }
}
