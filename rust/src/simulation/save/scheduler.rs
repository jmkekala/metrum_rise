// SPDX-License-Identifier: GPL-2.0-only

//! Scheduler state (`SIM-02`): the timing wheel and its counters, saved as generic rows so
//! this module does not depend on the simulation's event types.

use super::{SaveLoadError, SaveLoadResult, i64_to_u16, i64_to_u64, u64_to_i64};
use rusqlite::{Connection, Transaction, params};

/// The timing wheel and its counters as saved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SavedScheduler {
    /// First slot the wheel has not popped.
    pub(crate) next_slot: u64,
    /// Next event insertion sequence.
    pub(crate) next_event_seq: u64,
    /// Next queued demand spawn id.
    pub(crate) next_spawn_id: u64,
    /// `(slot, seq, kind, a)` per pending event, in firing order; `kind` and `a` are owned by
    /// the simulation's event type.
    pub(crate) events: Vec<(u64, u64, u16, u64)>,
}

pub(super) fn save(tx: &Transaction, scheduler: &SavedScheduler) -> SaveLoadResult<()> {
    tx.execute(
        "INSERT INTO scheduler_state(singleton, next_slot, next_event_seq, next_spawn_id) VALUES (1, ?1, ?2, ?3)",
        params![
            u64_to_i64(scheduler.next_slot)?,
            u64_to_i64(scheduler.next_event_seq)?,
            u64_to_i64(scheduler.next_spawn_id)?,
        ],
    )?;
    let mut events =
        tx.prepare("INSERT INTO timed_events(slot, seq, kind, a) VALUES (?1, ?2, ?3, ?4)")?;
    for &(slot, seq, kind, a) in &scheduler.events {
        events.execute(params![
            u64_to_i64(slot)?,
            u64_to_i64(seq)?,
            i64::from(kind),
            u64_to_i64(a)?,
        ])?;
    }
    Ok(())
}

pub(super) fn load(conn: &Connection) -> SaveLoadResult<SavedScheduler> {
    let (next_slot, next_event_seq, next_spawn_id): (i64, i64, i64) = conn.query_row(
        "SELECT next_slot, next_event_seq, next_spawn_id FROM scheduler_state WHERE singleton = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let mut scheduler = SavedScheduler {
        next_slot: i64_to_u64(next_slot)?,
        next_event_seq: i64_to_u64(next_event_seq)?,
        next_spawn_id: i64_to_u64(next_spawn_id)?,
        events: Vec::new(),
    };
    let mut stmt = conn.prepare("SELECT slot, seq, kind, a FROM timed_events ORDER BY slot, seq")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        scheduler.events.push((
            i64_to_u64(row.get(0)?)?,
            i64_to_u64(row.get(1)?)?,
            i64_to_u16(row.get(2)?)?,
            i64_to_u64(row.get(3)?)?,
        ));
    }
    if scheduler
        .events
        .iter()
        .any(|&(slot, seq, _, _)| slot < scheduler.next_slot || seq >= scheduler.next_event_seq)
    {
        return Err(SaveLoadError::custom(
            "timed event outside the saved scheduler window",
        ));
    }
    Ok(scheduler)
}
