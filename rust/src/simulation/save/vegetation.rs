// SPDX-License-Identifier: GPL-2.0-only

//! Deterministic serialization of the sparse vegetation delta; revisions are runtime-only.

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
    // Yard records in key order, each in the order its yard laid or planted it.
    for (table, records) in [
        (
            "yard_hedge_modules",
            edits.yard_hedges().collect::<Vec<_>>(),
        ),
        ("yard_planting_plants", edits.yard_planting().collect()),
    ] {
        let mut yard = tx.prepare(&format!(
            "INSERT INTO {table} VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"
        ))?;
        for (&(parcel_id, build_generation), plants) in records {
            for (cell, plant) in plants {
                yard.execute(params![
                    parcel_id as i64,
                    build_generation,
                    cell.layer as i64,
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
    let mut stmt = conn.prepare(
        "SELECT layer, cell_x, cell_z, x, z, yaw, scale, species, variant \
         FROM vegetation_additions ORDER BY layer, cell_z, cell_x, rowid",
    )?;
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
    for (table, planting) in [
        ("yard_hedge_modules", false),
        ("yard_planting_plants", true),
    ] {
        let mut stmt = conn.prepare(&format!(
            "SELECT parcel_id, build_generation, layer, cell_x, cell_z, x, z, yaw, scale, species, \
             variant FROM {table} ORDER BY rowid"
        ))?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let parcel_id: i64 = row.get(0)?;
            let key = (parcel_id as u64, row.get(1)?);
            let cell = VegetationCell {
                layer: layer(row.get(2)?)?,
                x: row.get(3)?,
                z: row.get(4)?,
            };
            let plant = AuthoredPlant {
                x: row.get(5)?,
                z: row.get(6)?,
                yaw: row.get(7)?,
                scale: row.get(8)?,
                species: row.get(9)?,
                variant: row.get(10)?,
            };
            if planting {
                edits.record_yard_planting(key, vec![(cell, plant)]);
            } else {
                edits.record_yard_hedge(key, vec![(cell, plant)]);
            }
        }
    }
    Ok(edits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::save::schema::SCHEMA;
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
        load(&conn).unwrap()
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
        // Rock models six variants, so a biased pin of seven names a mesh that does not exist.
        edits.add(cell(2), plant(3, 7));
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        let tx = conn.transaction().unwrap();
        save(&tx, &edits).unwrap();
        tx.commit().unwrap();
        assert!(load(&conn).is_err());
    }
}
