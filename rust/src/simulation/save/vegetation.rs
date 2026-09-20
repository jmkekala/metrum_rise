// SPDX-License-Identifier: GPL-2.0-only

//! Deterministic serialization of the sparse vegetation delta; revisions are runtime-only.

use super::schema::VEGETATION_VARIANT_SAVE_VERSION;
use super::{SaveLoadError, SaveLoadResult};
use crate::simulation::vegetation::edits::{
    AuthoredPlant, VegetationCell, VegetationEdits, VegetationLayer, variant_in_range,
};
use rusqlite::{Connection, Transaction, params};

/// Writes delta rows in stable cell order, preserving authored insertion order.
pub(super) fn save(tx: &Transaction<'_>, edits: &VegetationEdits) -> SaveLoadResult<()> {
    let mut removal = tx.prepare("INSERT INTO vegetation_removals VALUES (?1, ?2, ?3)")?;
    let mut addition =
        tx.prepare("INSERT INTO vegetation_additions VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)")?;
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
                plant.species,
                plant.variant
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
pub(super) fn load(conn: &Connection, version: i64) -> SaveLoadResult<VegetationEdits> {
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
    // A save written before the pin has no such column, so it selects a literal zero in its
    // place, which is the unpinned value every plant in it was authored with.
    let variant = if version >= VEGETATION_VARIANT_SAVE_VERSION {
        "variant"
    } else {
        "0"
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT layer, cell_x, cell_z, x, z, yaw, scale, species, {variant} \
         FROM vegetation_additions ORDER BY layer, cell_z, cell_x, rowid"
    ))?;
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
            variant: row.get(8)?,
        };
        if ![plant.x, plant.z, plant.yaw, plant.scale]
            .into_iter()
            .all(f32::is_finite)
            || plant.scale <= 0.0
            || plant.species > 3
            || !variant_in_range(plant.species, plant.variant)
        {
            return Err(SaveLoadError::custom("invalid authored vegetation plant"));
        }
        edits.add(cell, plant);
    }
    Ok(edits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::save::schema::{SCHEMA, VEGETATION_VARIANT_SAVE_VERSION};
    use crate::simulation::vegetation::edits::VARIANT_FROM_SEED;

    fn cell(x: i32) -> VegetationCell {
        VegetationCell {
            layer: VegetationLayer::Canopy,
            x,
            z: -3,
        }
    }

    fn plant(species: u8, variant: u8) -> AuthoredPlant {
        AuthoredPlant {
            x: 12.5,
            z: -6.25,
            yaw: 1.5,
            scale: 0.8,
            species,
            variant,
        }
    }

    fn round_trip(edits: &VegetationEdits) -> VegetationEdits {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        let tx = conn.transaction().unwrap();
        save(&tx, edits).unwrap();
        tx.commit().unwrap();
        load(&conn, VEGETATION_VARIANT_SAVE_VERSION).unwrap()
    }

    #[test]
    fn a_pinned_variant_survives_a_save_and_the_unpinned_one_stays_unpinned() {
        let mut edits = VegetationEdits::default();
        edits.add(cell(1), plant(0, 6));
        edits.add(cell(1), plant(1, VARIANT_FROM_SEED));
        let loaded = round_trip(&edits);
        let (_, added) = loaded.cell(cell(1));
        assert_eq!(
            added,
            [plant(0, 6), plant(1, VARIANT_FROM_SEED)],
            "the pin is authored state and must come back exactly, in authored order"
        );
    }

    #[test]
    fn a_pin_past_the_renderer_s_variants_is_rejected_rather_than_drawn_as_nothing() {
        let mut edits = VegetationEdits::default();
        // Bush models six variants, so a biased pin of seven names a mesh that does not exist.
        edits.add(cell(2), plant(2, 7));
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        let tx = conn.transaction().unwrap();
        save(&tx, &edits).unwrap();
        tx.commit().unwrap();
        assert!(load(&conn, VEGETATION_VARIANT_SAVE_VERSION).is_err());
    }

    #[test]
    fn a_save_written_before_the_pin_loads_every_plant_unpinned() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        // The column the older schema lacks, so the load must not read it back.
        conn.execute(
            "INSERT INTO vegetation_additions VALUES (0, 4, -3, 1.0, 2.0, 0.5, 1.25, 1, NULL)",
            [],
        )
        .unwrap();
        let loaded = load(&conn, VEGETATION_VARIANT_SAVE_VERSION - 1).unwrap();
        let (_, added) = loaded.cell(cell(4));
        assert_eq!(added.len(), 1);
        assert_eq!(
            added[0].variant, VARIANT_FROM_SEED,
            "a plant authored before the pin existed keeps the renderer's own choice"
        );
    }
}
