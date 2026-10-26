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

use super::select::{SelectScratch, dirty_tiles_for_road_changes};
use super::sites::SiteSource;
use super::tile::{BakeError, Level, TileInputs, TileKey, TilePayload, bake_tiles, gather_inputs};
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
    /// The wanted tiles' input gathers and the comparison, milliseconds.
    pub inputs_ms: f64,
    /// The bakes, milliseconds.
    pub bake_ms: f64,
}

impl StageStats {
    /// The stage's wall-clock time, milliseconds.
    pub fn total_ms(&self) -> f64 {
        self.select_ms + self.inputs_ms + self.bake_ms
    }
}

/// Reused buffers of the staging. Payloads a stage drops are kept until nothing else shares
/// them and their buffers reused by later bakes, so a drag allocates no texel buffer once warm.
#[derive(Default)]
pub struct StageScratch {
    select: SelectScratch,
    dirty: Vec<TileKey>,
    wanted: Vec<(Level, TileKey)>,
    inputs: Vec<TileInputs>,
    adopted: Vec<Option<Arc<TilePayload>>>,
    rebake: Vec<TileKey>,
    baked: Vec<TilePayload>,
    retired: Vec<Arc<TilePayload>>,
}

// Retired payloads kept for reuse at most: a drag's few poses' worth of `GL0` tiles.
const RETIRED_MAX: usize = 64;

impl StageScratch {
    /// The tiles a road change from `before` to `after` can alter at each of `levels` (the
    /// dirty tile selection, [`dirty_tiles_for_road_changes`]), in `(level, tile)` order, kept
    /// in the scratch for [`stage_tiles`]. `levels` must be sorted.
    #[allow(clippy::too_many_arguments)]
    pub fn select_road_edit(
        &mut self,
        terrain: &TerrainSystem,
        before: &RoadbedSystem,
        after: &RoadbedSystem,
        sites: &impl SiteSource,
        changes: &TerrainChanges,
        levels: &[Level],
    ) -> &[(Level, TileKey)] {
        debug_assert!(levels.is_sorted(), "levels in order");
        self.wanted.clear();
        for &level in levels {
            dirty_tiles_for_road_changes(
                terrain,
                before,
                after,
                sites,
                changes,
                level,
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
            self.retired.drain(..excess);
        }
    }

    // Moves the retired payloads nothing else shares into the bake buffers.
    fn recycle(&mut self) {
        self.baked.clear();
        let mut kept = 0;
        for index in 0..self.retired.len() {
            let payload = std::mem::take(&mut self.retired[index]);
            match Arc::try_unwrap(payload) {
                Ok(payload) => self.baked.push(payload),
                Err(shared) => {
                    self.retired[kept] = shared;
                    kept += 1;
                }
            }
        }
        self.retired.truncate(kept);
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
/// `sites`: each wanted tile's inputs are gathered ([`gather_inputs`]); a tile `held` holds at
/// the same level with equal inputs is adopted (the same payload, shared), the others are
/// baked, one batch per level. The result is bitwise the bake of every wanted tile, whatever
/// `held` holds. `O(wanted gathers + rebaked bakes)`, in parallel on the shared pool.
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
    let inputs_start = Instant::now();
    gather_inputs(terrain, roadbed, sites, wanted, &mut scratch.inputs)?;
    scratch.adopted.clear();
    scratch.adopted.extend(
        wanted
            .iter()
            .zip(&scratch.inputs)
            .map(|(&(level, tile), inputs)| {
                held.get(level, tile)
                    .filter(|payload| payload.inputs == *inputs)
                    .cloned()
            }),
    );
    stats.inputs_ms = inputs_start.elapsed().as_secs_f64() * 1000.0;

    let bake_start = Instant::now();
    scratch.recycle();
    let mut tiles = Vec::with_capacity(wanted.len());
    let mut start = 0;
    while start < wanted.len() {
        let level = wanted[start].0;
        let end = start + wanted[start..].partition_point(|&(other, _)| other == level);
        scratch.rebake.clear();
        scratch.rebake.extend(
            (start..end)
                .filter(|&index| scratch.adopted[index].is_none())
                .map(|index| wanted[index].1),
        );
        if !scratch.rebake.is_empty() {
            bake_tiles(
                terrain,
                roadbed,
                sites,
                &scratch.rebake,
                level,
                &mut scratch.baked,
            )?;
        }
        let mut baked = scratch.baked.drain(..);
        let run = scratch.adopted[start..end].iter_mut();
        for ((&(_, tile), adopted), inputs) in wanted[start..end]
            .iter()
            .zip(run)
            .zip(&scratch.inputs[start..end])
        {
            let payload = match adopted.take() {
                Some(payload) => {
                    stats.adopted += 1;
                    payload
                }
                None => {
                    stats.baked += 1;
                    let payload = baked.next().expect("one bake per rebaked tile");
                    debug_assert_eq!(
                        payload.inputs, *inputs,
                        "a bake records the inputs its gather finds"
                    );
                    Arc::new(payload)
                }
            };
            tiles.push(StagedTile {
                level,
                tile,
                payload,
            });
        }
        drop(baked);
        start = end;
    }
    stats.bake_ms = bake_start.elapsed().as_secs_f64() * 1000.0;
    Ok((TileStage { tiles }, stats))
}
