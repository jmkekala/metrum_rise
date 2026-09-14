// SPDX-License-Identifier: GPL-2.0-only

//! Deterministic serialization of the sparse vegetation delta; revisions are runtime-only.

use super::{SaveLoadError, SaveLoadResult};
use crate::simulation::vegetation::edits::{
    AuthoredPlant, VegetationCell, VegetationEdits, VegetationLayer,
};
use rusqlite::{Connection, Transaction, params};

/// Writes delta rows in stable cell order, preserving authored insertion order.
pub(super) fn save(tx: &Transaction<'_>, edits: &VegetationEdits) -> SaveLoadResult<()> {
    let mut removal = tx.prepare("INSERT INTO vegetation_removals VALUES (?1, ?2, ?3)")?;
    let mut addition =
        tx.prepare("INSERT INTO vegetation_additions VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)")?;
    for cell in edits.sorted_cells() {
        let (removed, added) = edits.cell(cell);
        let layer = cell.layer as i64;
        if removed {
            removal.execute(params![layer, cell.x, cell.z])?;
        }
        // Insertion order is authoritative within a cell and survives load/save unchanged.
        for plant in added {
            addition.execute(params![
                layer,
                cell.x,
                cell.z,
                plant.x,
                plant.z,
                plant.yaw,
                plant.scale,
                plant.species
            ])?;
        }
    }
    Ok(())
}

fn layer(value: i64) -> SaveLoadResult<VegetationLayer> {
    match value {
        0 => Ok(VegetationLayer::Canopy),
        1 => Ok(VegetationLayer::Understory),
        _ => Err(SaveLoadError::custom("invalid vegetation layer")),
    }
}

/// Restores the authoritative delta with fresh runtime patch generations.
pub(super) fn load(conn: &Connection) -> SaveLoadResult<VegetationEdits> {
    let mut edits = VegetationEdits::default();
    let mut stmt = conn.prepare(
        "SELECT layer, cell_x, cell_z FROM vegetation_removals ORDER BY layer, cell_z, cell_x",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        edits.set_removed(
            VegetationCell {
                layer: layer(row.get(0)?)?,
                x: row.get(1)?,
                z: row.get(2)?,
            },
            true,
        );
    }
    let mut stmt = conn.prepare("SELECT layer, cell_x, cell_z, x, z, yaw, scale, species FROM vegetation_additions ORDER BY layer, cell_z, cell_x, rowid")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let cell = VegetationCell {
            layer: layer(row.get(0)?)?,
            x: row.get(1)?,
            z: row.get(2)?,
        };
        let plant = AuthoredPlant {
            x: row.get(3)?,
            z: row.get(4)?,
            yaw: row.get(5)?,
            scale: row.get(6)?,
            species: row.get(7)?,
        };
        if ![plant.x, plant.z, plant.yaw, plant.scale]
            .into_iter()
            .all(f32::is_finite)
            || plant.scale <= 0.0
            || plant.species > 3
        {
            return Err(SaveLoadError::custom("invalid authored vegetation plant"));
        }
        edits.add(cell, plant);
    }
    Ok(edits)
}
