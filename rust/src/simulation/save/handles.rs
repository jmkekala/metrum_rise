// SPDX-License-Identifier: GPL-2.0-only

//! Entity handle tables (`SIM-02`): saved with their stores, so a loaded game resolves every
//! handle to the same entity and reuses freed slots in the same order.

use super::{SaveLoadError, SaveLoadResult, i64_to_u32};
use crate::simulation::core::handles::{HandleTable, SavedHandleTable};
use rusqlite::{Connection, Transaction, params};

/// Saved table kinds; stable codes in `entity_handle_slots.kind`.
#[derive(Clone, Copy)]
pub(super) enum HandleKind {
    Buildings = 0,
    Households = 1,
    Agents = 2,
}

/// Writes `table` for a store of `len` entities; entities appended directly get handles first,
/// in the order the live table would give them.
pub(super) fn save<K>(
    tx: &Transaction,
    kind: HandleKind,
    table: &HandleTable<K>,
    len: usize,
) -> SaveLoadResult<()> {
    let mut registered = table.clone();
    registered.register_appended(len);
    let saved = registered.save();
    let kind = kind as i64;
    let mut slots = tx.prepare(
        "INSERT INTO entity_handle_slots(kind, slot, generation, dense) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (slot, &(generation, dense)) in saved.slots.iter().enumerate() {
        slots.execute(params![
            kind,
            slot as i64,
            i64::from(generation),
            dense.map_or(-1, i64::from),
        ])?;
    }
    let mut free =
        tx.prepare("INSERT INTO entity_handle_free(kind, position, slot) VALUES (?1, ?2, ?3)")?;
    for (position, &slot) in saved.free.iter().enumerate() {
        free.execute(params![kind, position as i64, i64::from(slot)])?;
    }
    Ok(())
}

/// Restores the table of a loaded store of `len` entities.
pub(super) fn load<K>(
    conn: &Connection,
    kind: HandleKind,
    len: usize,
) -> SaveLoadResult<HandleTable<K>> {
    let kind = kind as i64;
    let mut saved = SavedHandleTable::default();
    let mut slots = conn.prepare(
        "SELECT slot, generation, dense FROM entity_handle_slots WHERE kind = ?1 ORDER BY slot",
    )?;
    let mut rows = slots.query([kind])?;
    while let Some(row) = rows.next()? {
        if row.get::<_, i64>(0)? != saved.slots.len() as i64 {
            return Err(SaveLoadError::custom(
                "entity handle slots are not contiguous",
            ));
        }
        let dense: i64 = row.get(2)?;
        saved.slots.push((
            i64_to_u32(row.get(1)?)?,
            (dense >= 0).then(|| i64_to_u32(dense)).transpose()?,
        ));
    }
    let mut free =
        conn.prepare("SELECT slot FROM entity_handle_free WHERE kind = ?1 ORDER BY position")?;
    let mut rows = free.query([kind])?;
    while let Some(row) = rows.next()? {
        saved.free.push(i64_to_u32(row.get(0)?)?);
    }
    HandleTable::restore(&saved, len).map_err(SaveLoadError::custom)
}
