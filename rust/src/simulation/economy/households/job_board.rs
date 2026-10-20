// SPDX-License-Identifier: GPL-2.0-only

//! The open workplaces job searches read (`SIM-02-J`), as a rolling snapshot of building handles
//! (`docs/simulation.md#snapshots`).
//!
//! The hour's first slot freezes the building handle capacity `H`; chunk `k` folds the buildings
//! in handle slots `[k·H/S, (k+1)·H/S)`, with `S` the hour's slots, that have an open funded job
//! at its slot, and the hour's last slot publishes them, in handle order, as the board of the
//! next hour's searches. A search reads each member's live funded jobs and workers, so a job
//! filled since its chunk is no option and one opened since waits for the next sweep. A sweep
//! that misses a slot (the clock set directly) is abandoned; an hour without a board builds one
//! at once.

use std::ops::Range;

use rayon::prelude::*;

use super::employment::open_job;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::core::handles::Handle;
use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
use crate::simulation::economy::definitions::RuntimeEconomyCatalog;

// Building handle slots per parallel task of a whole fold.
const HANDLES_PER_TASK: usize = 1_024;

/// Saved sweep in progress.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SavedJobBoardSweep {
    /// Building handle slots frozen at the hour's first slot.
    pub(crate) building_handles: u64,
    /// Absolute hour whose searches read the finished board.
    pub(crate) hour: u64,
    /// Chunk the sweep expects next.
    pub(crate) next_chunk: u64,
    /// Members folded so far, in handle order.
    pub(crate) members: Vec<Handle<Building>>,
}

/// Saved state of a [`JobBoardRoll`], so a loaded game continues it identically.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SavedJobBoard {
    /// The absolute hour the board serves and its members, in handle order.
    pub(crate) front: Option<(u64, Vec<Handle<Building>>)>,
    /// The sweep in progress.
    pub(crate) sweep: Option<SavedJobBoardSweep>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Sweep {
    building_handles: u64,
    hour: u64,
}

/// Front and back of the job board.
#[derive(Clone, Debug, Default)]
pub(crate) struct JobBoardRoll {
    // The board and the absolute hour whose searches read it.
    front: Option<(u64, Vec<Handle<Building>>)>,
    sweep: Option<Sweep>,
    next_chunk: u64,
    // Members folded so far, in handle order.
    back: Vec<Handle<Building>>,
}

impl JobBoardRoll {
    /// Slot `slot`'s chunk of the hour's sweep; the caller registers appended buildings'
    /// handles first. O(H/S) and allocation-free once the back has room; the hour's last slot
    /// also publishes the board, O(M) for M members.
    pub(super) fn advance(
        &mut self,
        slot: u64,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
    ) {
        let chunk = slot % SLOTS_PER_HOUR;
        if chunk == 0 {
            self.sweep = Some(Sweep {
                building_handles: allocator.building_handles.capacity() as u64,
                hour: slot / SLOTS_PER_HOUR + 1,
            });
            self.back.clear();
        } else if chunk != self.next_chunk {
            self.sweep = None;
        }
        let Some(sweep) = self.sweep else {
            return;
        };
        self.next_chunk = chunk + 1;
        fold(
            part(chunk, sweep.building_handles),
            allocator,
            catalog,
            &mut self.back,
        );
        if chunk == SLOTS_PER_HOUR - 1 {
            self.front = Some((sweep.hour, self.back.clone()));
            self.sweep = None;
        }
    }

    /// The board of absolute hour `hour`'s searches, built at once when no sweep finished one
    /// for it; the caller registers appended buildings' handles first. O(1) with a board. A
    /// sweep in progress keeps going and replaces it at the hour's end.
    pub(super) fn members_for(
        &mut self,
        hour: u64,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
    ) -> &[Handle<Building>] {
        if self
            .front
            .as_ref()
            .is_none_or(|(front_hour, _)| *front_hour != hour)
        {
            self.front = Some((hour, fold_all(allocator, catalog)));
        }
        &self.front.as_ref().unwrap().1
    }

    /// The board's members, empty before the first board.
    pub(super) fn members(&self) -> &[Handle<Building>] {
        self.front.as_ref().map_or(&[], |(_, members)| members)
    }

    /// Saved form; [`Self::restore`] rebuilds a roll that continues identically.
    pub(crate) fn saved(&self) -> SavedJobBoard {
        SavedJobBoard {
            front: self.front.clone(),
            sweep: self.sweep.map(|sweep| SavedJobBoardSweep {
                building_handles: sweep.building_handles,
                hour: sweep.hour,
                next_chunk: self.next_chunk,
                members: self.back.clone(),
            }),
        }
    }

    /// Rebuilds a saved roll.
    pub(crate) fn restore(saved: SavedJobBoard) -> Result<Self, String> {
        let mut roll = Self {
            front: saved.front,
            ..Self::default()
        };
        if let Some(sweep) = saved.sweep {
            if sweep.next_chunk == 0 || sweep.next_chunk >= SLOTS_PER_HOUR {
                return Err("job board sweep chunk outside the hour".to_string());
            }
            roll.sweep = Some(Sweep {
                building_handles: sweep.building_handles,
                hour: sweep.hour,
            });
            roll.next_chunk = sweep.next_chunk;
            roll.back = sweep.members;
        }
        Ok(roll)
    }
}

// Appends the buildings in handle slots `slots` with an open job now to `members`, in slot
// order; a free slot adds nothing. O(slots).
fn fold(
    slots: Range<usize>,
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    members: &mut Vec<Handle<Building>>,
) {
    for slot in slots {
        if let Some(idx) = allocator.building_handles.dense_at(slot)
            && open_job(allocator, catalog, idx).is_some()
        {
            members.push(allocator.building_handles.handle(idx));
        }
    }
}

// Every handle slot's member, folded on Rayon in fixed parts and merged in slot order.
// O(handles).
fn fold_all(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
) -> Vec<Handle<Building>> {
    let handles = allocator.building_handles.capacity();
    let parts: Vec<Vec<Handle<Building>>> = (0..handles.div_ceil(HANDLES_PER_TASK))
        .into_par_iter()
        .map(|task| {
            let start = task * HANDLES_PER_TASK;
            let mut members = Vec::new();
            fold(
                start..(start + HANDLES_PER_TASK).min(handles),
                allocator,
                catalog,
                &mut members,
            );
            members
        })
        .collect();
    parts.concat()
}

// Handle slots folded by chunk `chunk` of a sweep over `handles` slots.
fn part(chunk: u64, handles: u64) -> Range<usize> {
    (chunk * handles / SLOTS_PER_HOUR) as usize..((chunk + 1) * handles / SLOTS_PER_HOUR) as usize
}
