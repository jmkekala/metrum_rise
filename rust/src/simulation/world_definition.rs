//! Reusable authored-world persistence for blank-world authoring.
//!
//! A `WorldDefinition` is intentionally narrower than a city save:
//! it stores only the authored world metadata and source terrain needed to
//! start a fresh game or continue editing the world later.

use crate::simulation::core::config::WorldConfig;
use crate::simulation::terrain::TerrainSystem;
use rusqlite::{Connection, params};
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::Path;

const WORLD_DEFINITION_FORMAT_VERSION: i64 = 1;

const WORLD_DEFINITION_SCHEMA: &str = r#"
CREATE TABLE world_definition_meta(
    format_version INTEGER NOT NULL,
    name TEXT NOT NULL,
    width_m REAL NOT NULL,
    height_m REAL NOT NULL,
    terrain_cell_m REAL NOT NULL,
    terrain_chunk_m REAL NOT NULL,
    terrain_base_elevation_m REAL NOT NULL,
    env_cell_m REAL NOT NULL,
    zone_cell_m REAL NOT NULL
);
CREATE TABLE world_terrain_chunks(
    chunk_x INTEGER NOT NULL,
    chunk_z INTEGER NOT NULL,
    width_samples INTEGER NOT NULL,
    height_samples INTEGER NOT NULL,
    source_height_blob_f32_le BLOB NOT NULL,
    PRIMARY KEY(chunk_x, chunk_z)
);
"#;

/// Borrowed view of one authored world ready for persistence.
pub(crate) struct WorldDefinitionView<'a> {
    /// User-facing authored world name.
    pub name: &'a str,
    /// Authoritative world config for the authored world.
    pub config: &'a WorldConfig,
    /// Authoritative terrain source data for the authored world.
    pub terrain: &'a TerrainSystem,
}

/// Fully loaded blank-world definition ready to instantiate as runtime state.
pub(crate) struct LoadedWorldDefinition {
    /// User-facing authored world name.
    pub name: String,
    /// Authoritative world config for the authored world.
    pub config: WorldConfig,
    /// Source terrain loaded into sparse runtime storage.
    pub terrain: TerrainSystem,
}

/// Error produced while reading or writing one authored world definition.
#[derive(Debug)]
pub(crate) struct WorldDefinitionError(String);

type WorldDefinitionResult<T> = Result<T, WorldDefinitionError>;

impl WorldDefinitionError {
    fn custom(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl Display for WorldDefinitionError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for WorldDefinitionError {}

impl From<rusqlite::Error> for WorldDefinitionError {
    fn from(value: rusqlite::Error) -> Self {
        Self(value.to_string())
    }
}

impl From<std::io::Error> for WorldDefinitionError {
    fn from(value: std::io::Error) -> Self {
        Self(value.to_string())
    }
}

/// Saves one authored blank-world definition to a single-file SQLite asset.
pub(crate) fn save_world_definition_to_sqlite(
    path: &Path,
    view: WorldDefinitionView<'_>,
) -> WorldDefinitionResult<()> {
    validate_world_name(view.name)?;
    validate_world_config(view.config)?;
    validate_terrain_dimensions(view.config, view.terrain)?;

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    if path.exists() {
        fs::remove_file(path)?;
    }

    let mut conn = Connection::open(path)?;
    conn.execute_batch(WORLD_DEFINITION_SCHEMA)?;
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO world_definition_meta(format_version, name, width_m, height_m, terrain_cell_m, terrain_chunk_m, terrain_base_elevation_m, env_cell_m, zone_cell_m) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            WORLD_DEFINITION_FORMAT_VERSION,
            view.name.trim(),
            view.config.width_m,
            view.config.height_m,
            view.config.terrain_cell_m,
            view.config.terrain_chunk_m,
            view.config.terrain_base_elevation_m,
            view.config.env_cell_m,
            view.config.zone_cell_m
        ],
    )?;

    let chunk_size = authored_chunk_cells(view.config);
    let source_dense = view.terrain.clone_source_dense();
    let chunk_cols = view.terrain.width.div_ceil(chunk_size);
    let chunk_rows = view.terrain.height.div_ceil(chunk_size);
    let mut stmt = tx.prepare(
        "INSERT INTO world_terrain_chunks(chunk_x, chunk_z, width_samples, height_samples, source_height_blob_f32_le) VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for chunk_z in 0..chunk_rows {
        for chunk_x in 0..chunk_cols {
            let origin_x = chunk_x * chunk_size;
            let origin_z = chunk_z * chunk_size;
            let width_samples = (view.terrain.width - origin_x).min(chunk_size);
            let height_samples = (view.terrain.height - origin_z).min(chunk_size);
            let mut payload = Vec::with_capacity(width_samples * height_samples);
            let mut touched = false;

            for local_z in 0..height_samples {
                let row_start = (origin_z + local_z) * view.terrain.width + origin_x;
                let row_end = row_start + width_samples;
                let row = &source_dense[row_start..row_end];
                if !touched
                    && row
                        .iter()
                        .any(|value| *value != view.config.terrain_base_elevation_m)
                {
                    touched = true;
                }
                payload.extend_from_slice(row);
            }

            if !touched {
                continue;
            }

            stmt.execute(params![
                chunk_x as i64,
                chunk_z as i64,
                width_samples as i64,
                height_samples as i64,
                pack_f32_slice(&payload),
            ])?;
        }
    }
    drop(stmt);

    tx.commit()?;
    Ok(())
}

/// Loads one authored blank-world definition from a single-file SQLite asset.
pub(crate) fn load_world_definition_from_sqlite(
    path: &Path,
) -> WorldDefinitionResult<LoadedWorldDefinition> {
    let conn = Connection::open(path)?;
    let (format_version, name, width_m, height_m, terrain_cell_m, terrain_chunk_m, terrain_base_elevation_m, env_cell_m, zone_cell_m): (
        i64,
        String,
        f32,
        f32,
        f32,
        f32,
        f32,
        f32,
        f32,
    ) = conn.query_row(
        "SELECT format_version, name, width_m, height_m, terrain_cell_m, terrain_chunk_m, terrain_base_elevation_m, env_cell_m, zone_cell_m FROM world_definition_meta LIMIT 1",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
            ))
        },
    )?;
    if format_version != WORLD_DEFINITION_FORMAT_VERSION {
        return Err(WorldDefinitionError::custom(
            "world definition version mismatch",
        ));
    }
    validate_world_name(&name)?;
    let config = WorldConfig::new(width_m, height_m, env_cell_m, zone_cell_m)
        .with_terrain_resolution(terrain_cell_m)
        .with_chunking(terrain_chunk_m, terrain_base_elevation_m);
    validate_world_config(&config)?;

    let mut terrain = TerrainSystem::from_world_config(&config);
    let chunk_size = authored_chunk_cells(&config);
    let mut stmt = conn.prepare(
        "SELECT chunk_x, chunk_z, width_samples, height_samples, source_height_blob_f32_le FROM world_terrain_chunks ORDER BY chunk_z, chunk_x",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let chunk_x = i64_to_usize(row.get(0)?)?;
        let chunk_z = i64_to_usize(row.get(1)?)?;
        let width_samples = i64_to_usize(row.get(2)?)?;
        let height_samples = i64_to_usize(row.get(3)?)?;
        let blob: Vec<u8> = row.get(4)?;

        if width_samples == 0 || height_samples == 0 {
            return Err(WorldDefinitionError::custom(
                "world definition terrain chunk has zero-sized payload",
            ));
        }
        if width_samples > chunk_size || height_samples > chunk_size {
            return Err(WorldDefinitionError::custom(
                "world definition terrain chunk exceeds canonical chunk sample span",
            ));
        }

        let origin_x = chunk_x
            .checked_mul(chunk_size)
            .ok_or_else(|| WorldDefinitionError::custom("chunk_x overflow"))?;
        let origin_z = chunk_z
            .checked_mul(chunk_size)
            .ok_or_else(|| WorldDefinitionError::custom("chunk_z overflow"))?;
        if origin_x + width_samples > terrain.width || origin_z + height_samples > terrain.height {
            return Err(WorldDefinitionError::custom(
                "world definition terrain chunk falls outside world terrain bounds",
            ));
        }

        let payload = unpack_f32_blob(&blob, width_samples * height_samples)?;
        for local_z in 0..height_samples {
            let row_start = local_z * width_samples;
            let row_end = row_start + width_samples;
            let payload_row = &payload[row_start..row_end];
            for (local_x, value) in payload_row.iter().enumerate() {
                if *value == config.terrain_base_elevation_m {
                    continue;
                }
                terrain.set_height(origin_x + local_x, origin_z + local_z, *value);
            }
        }
    }
    terrain.reset_visuals_from_source();

    Ok(LoadedWorldDefinition {
        name,
        config,
        terrain,
    })
}

fn validate_world_name(name: &str) -> WorldDefinitionResult<()> {
    if name.trim().is_empty() {
        return Err(WorldDefinitionError::custom(
            "world definition name must not be empty",
        ));
    }
    Ok(())
}

fn validate_world_config(config: &WorldConfig) -> WorldDefinitionResult<()> {
    validate_positive_f32(config.width_m, "width_m")?;
    validate_positive_f32(config.height_m, "height_m")?;
    validate_positive_f32(config.terrain_cell_m, "terrain_cell_m")?;
    validate_positive_f32(config.terrain_chunk_m, "terrain_chunk_m")?;
    validate_positive_f32(config.env_cell_m, "env_cell_m")?;
    validate_positive_f32(config.zone_cell_m, "zone_cell_m")?;
    if !config.terrain_base_elevation_m.is_finite() {
        return Err(WorldDefinitionError::custom(
            "terrain_base_elevation_m must be finite",
        ));
    }
    Ok(())
}

fn validate_positive_f32(value: f32, label: &str) -> WorldDefinitionResult<()> {
    if !value.is_finite() || value <= 0.0 {
        return Err(WorldDefinitionError::custom(format!(
            "{label} must be finite and > 0"
        )));
    }
    Ok(())
}

fn validate_terrain_dimensions(
    config: &WorldConfig,
    terrain: &TerrainSystem,
) -> WorldDefinitionResult<()> {
    let expected_w = config.terrain_grid_width();
    let expected_h = config.terrain_grid_height();
    if terrain.width != expected_w || terrain.height != expected_h {
        return Err(WorldDefinitionError::custom(format!(
            "terrain size mismatch: got {}x{}, expected {}x{}",
            terrain.width, terrain.height, expected_w, expected_h
        )));
    }
    Ok(())
}

fn authored_chunk_cells(config: &WorldConfig) -> usize {
    ((config.terrain_chunk_m / config.terrain_cell_m).ceil() as usize).max(1)
}

fn pack_f32_slice(values: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * std::mem::size_of::<f32>());
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

fn unpack_f32_blob(blob: &[u8], expected_len: usize) -> WorldDefinitionResult<Vec<f32>> {
    let expected_bytes = expected_len
        .checked_mul(std::mem::size_of::<f32>())
        .ok_or_else(|| WorldDefinitionError::custom("f32 blob size overflow"))?;
    if blob.len() != expected_bytes {
        return Err(WorldDefinitionError::custom(format!(
            "f32 blob length mismatch: got {}, expected {}",
            blob.len(),
            expected_bytes
        )));
    }

    let mut out = Vec::with_capacity(expected_len);
    for chunk in blob.chunks_exact(4) {
        out.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Ok(out)
}

fn i64_to_usize(value: i64) -> WorldDefinitionResult<usize> {
    usize::try_from(value).map_err(|_| {
        WorldDefinitionError::custom(format!("could not convert SQLite integer {value} to usize"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "metrum_rise_world_definition_{name}_{}_{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn world_definition_round_trip_preserves_config_and_source_terrain() {
        let path = temp_path("round_trip");
        let config = WorldConfig::new(120.0, 90.0, 40.0, 10.0)
            .with_terrain_resolution(10.0)
            .with_chunking(32.0, 2.0);
        let mut terrain = TerrainSystem::from_world_config(&config);
        terrain.set_height(0, 0, 3.0);
        terrain.set_height(1, 0, 4.0);
        terrain.set_height(4, 5, 7.5);
        terrain.set_height(9, 3, 1.0);

        save_world_definition_to_sqlite(
            &path,
            WorldDefinitionView {
                name: "Blank Test World",
                config: &config,
                terrain: &terrain,
            },
        )
        .expect("world definition should save");

        let loaded =
            load_world_definition_from_sqlite(&path).expect("world definition should load");

        assert_eq!(loaded.name, "Blank Test World");
        assert_eq!(loaded.config, config);
        assert_eq!(
            loaded.terrain.clone_source_dense(),
            terrain.clone_source_dense()
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn world_definition_only_persists_touched_terrain_chunks() {
        let path = temp_path("chunk_count");
        let config = WorldConfig::new(100.0, 100.0, 40.0, 10.0)
            .with_terrain_resolution(10.0)
            .with_chunking(20.0, 0.0);
        let mut terrain = TerrainSystem::from_world_config(&config);
        terrain.set_height(0, 0, 5.0);
        terrain.set_height(8, 8, 6.0);

        save_world_definition_to_sqlite(
            &path,
            WorldDefinitionView {
                name: "Sparse Save",
                config: &config,
                terrain: &terrain,
            },
        )
        .expect("world definition should save");

        let conn = Connection::open(&path).expect("saved world definition should exist");
        let chunk_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM world_terrain_chunks", [], |row| {
                row.get(0)
            })
            .expect("chunk count query should succeed");
        assert_eq!(chunk_count, 2);
        std::fs::remove_file(path).ok();
    }
}
