// SPDX-License-Identifier: GPL-2.0-only

//! Reusable authored-world persistence for blank-world authoring.
//!
//! A `WorldDefinition` is intentionally narrower than a city save:
//! it stores only the authored world metadata and source terrain needed to
//! start a fresh game or continue editing the world later.

use crate::simulation::core::config::WorldConfig;
use crate::simulation::resources::{COAL_RESOURCE_ID, ResourceDepositSystem};
use crate::simulation::terrain::TerrainSystem;
use rusqlite::{Connection, params};
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::Path;

const WORLD_DEFINITION_FORMAT_VERSION: i64 = 5;

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
CREATE TABLE world_lake_fills(
    world_x REAL NOT NULL,
    world_z REAL NOT NULL,
    surface_elevation_m REAL NOT NULL
);
CREATE TABLE world_open_water_fills(
    world_x REAL NOT NULL,
    world_z REAL NOT NULL,
    surface_elevation_m REAL NOT NULL
);
CREATE TABLE world_resource_deposit_chunks(
    resource_id TEXT NOT NULL,
    chunk_x INTEGER NOT NULL,
    chunk_z INTEGER NOT NULL,
    width_samples INTEGER NOT NULL,
    height_samples INTEGER NOT NULL,
    richness_blob_u16_le BLOB NOT NULL,
    PRIMARY KEY(resource_id, chunk_x, chunk_z)
);
"#;

/// One authored lake fill record persisted in a reusable world definition.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AuthoredLakeFill {
    /// World-space X seed position in metres.
    pub world_x: f32,
    /// World-space Z seed position in metres.
    pub world_z: f32,
    /// Target authored water surface elevation in rendered world metres.
    pub surface_elevation_m: f32,
}

/// One authored edge-connected open-water fill record persisted in a reusable world definition.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AuthoredOpenWaterFill {
    /// World-space X seed position in metres.
    pub world_x: f32,
    /// World-space Z seed position in metres.
    pub world_z: f32,
    /// Target authored water surface elevation in rendered world metres.
    pub surface_elevation_m: f32,
}

/// Borrowed view of one authored world ready for persistence.
pub(crate) struct WorldDefinitionView<'a> {
    /// User-facing authored world name.
    pub name: &'a str,
    /// Authoritative world config for the authored world.
    pub config: &'a WorldConfig,
    /// Authoritative terrain source data for the authored world.
    pub terrain: &'a TerrainSystem,
    /// Authored lake fills for the world.
    pub lake_fills: &'a [AuthoredLakeFill],
    /// Authored edge-connected open-water fills for the world.
    pub open_water_fills: &'a [AuthoredOpenWaterFill],
    /// Authored natural-resource deposit layers for the world.
    pub resource_deposits: &'a ResourceDepositSystem,
}

/// Fully loaded blank-world definition ready to instantiate as runtime state.
pub(crate) struct LoadedWorldDefinition {
    /// User-facing authored world name.
    pub name: String,
    /// Authoritative world config for the authored world.
    pub config: WorldConfig,
    /// Source terrain loaded into sparse runtime storage.
    pub terrain: TerrainSystem,
    /// Authored lake fills loaded from the world definition.
    pub lake_fills: Vec<AuthoredLakeFill>,
    /// Authored edge-connected open-water fills loaded from the world definition.
    pub open_water_fills: Vec<AuthoredOpenWaterFill>,
    /// Authored natural-resource deposit layers loaded from the world definition.
    pub resource_deposits: ResourceDepositSystem,
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
    validate_resource_deposit_dimensions(view.config, view.resource_deposits)?;
    validate_authored_water(view.config, view.lake_fills, view.open_water_fills)?;

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

    let mut lake_stmt = tx.prepare(
        "INSERT INTO world_lake_fills(world_x, world_z, surface_elevation_m) VALUES (?1, ?2, ?3)",
    )?;
    for lake in view.lake_fills {
        lake_stmt.execute(params![
            lake.world_x,
            lake.world_z,
            lake.surface_elevation_m
        ])?;
    }
    drop(lake_stmt);

    let mut open_water_stmt = tx.prepare(
        "INSERT INTO world_open_water_fills(world_x, world_z, surface_elevation_m) VALUES (?1, ?2, ?3)",
    )?;
    for water in view.open_water_fills {
        open_water_stmt.execute(params![
            water.world_x,
            water.world_z,
            water.surface_elevation_m
        ])?;
    }
    drop(open_water_stmt);

    let coal_dense = view.resource_deposits.clone_coal_richness_dense();
    let (resource_w, resource_h) = view.resource_deposits.grid_dimensions();
    let mut resource_stmt = tx.prepare(
        "INSERT INTO world_resource_deposit_chunks(resource_id, chunk_x, chunk_z, width_samples, height_samples, richness_blob_u16_le) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    let chunk_cols = resource_w.div_ceil(chunk_size);
    let chunk_rows = resource_h.div_ceil(chunk_size);
    for chunk_z in 0..chunk_rows {
        for chunk_x in 0..chunk_cols {
            let origin_x = chunk_x * chunk_size;
            let origin_z = chunk_z * chunk_size;
            let width_samples = (resource_w - origin_x).min(chunk_size);
            let height_samples = (resource_h - origin_z).min(chunk_size);
            let mut payload = Vec::with_capacity(width_samples * height_samples);
            let mut touched = false;

            for local_z in 0..height_samples {
                let row_start = (origin_z + local_z) * resource_w + origin_x;
                let row_end = row_start + width_samples;
                let row = &coal_dense[row_start..row_end];
                if !touched && row.iter().any(|value| *value != 0) {
                    touched = true;
                }
                payload.extend_from_slice(row);
            }

            if !touched {
                continue;
            }

            resource_stmt.execute(params![
                COAL_RESOURCE_ID,
                chunk_x as i64,
                chunk_z as i64,
                width_samples as i64,
                height_samples as i64,
                pack_u16_slice(&payload),
            ])?;
        }
    }
    drop(resource_stmt);

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
    if !(1..=WORLD_DEFINITION_FORMAT_VERSION).contains(&format_version) {
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

    let mut lake_fills = Vec::new();
    let mut open_water_fills = Vec::new();
    if format_version >= 2 {
        let mut lake_stmt = conn.prepare(
            "SELECT world_x, world_z, surface_elevation_m FROM world_lake_fills ORDER BY rowid",
        )?;
        let mut lake_rows = lake_stmt.query([])?;
        while let Some(row) = lake_rows.next()? {
            lake_fills.push(AuthoredLakeFill {
                world_x: row.get(0)?,
                world_z: row.get(1)?,
                surface_elevation_m: row.get(2)?,
            });
        }
    }
    if format_version >= 3 {
        let mut open_water_stmt = conn.prepare(
            "SELECT world_x, world_z, surface_elevation_m FROM world_open_water_fills ORDER BY rowid",
        )?;
        let mut open_water_rows = open_water_stmt.query([])?;
        while let Some(row) = open_water_rows.next()? {
            open_water_fills.push(AuthoredOpenWaterFill {
                world_x: row.get(0)?,
                world_z: row.get(1)?,
                surface_elevation_m: row.get(2)?,
            });
        }
    }
    let mut resource_deposits = ResourceDepositSystem::from_world_config(&config);
    if format_version >= 5 {
        let (resource_w, resource_h) = resource_deposits.grid_dimensions();
        let mut resource_stmt = conn.prepare(
            "SELECT chunk_x, chunk_z, width_samples, height_samples, richness_blob_u16_le FROM world_resource_deposit_chunks WHERE resource_id = ?1 ORDER BY chunk_z, chunk_x",
        )?;
        let mut resource_rows = resource_stmt.query([COAL_RESOURCE_ID])?;
        while let Some(row) = resource_rows.next()? {
            let chunk_x = i64_to_usize(row.get(0)?)?;
            let chunk_z = i64_to_usize(row.get(1)?)?;
            let width_samples = i64_to_usize(row.get(2)?)?;
            let height_samples = i64_to_usize(row.get(3)?)?;
            let blob: Vec<u8> = row.get(4)?;

            if width_samples == 0 || height_samples == 0 {
                return Err(WorldDefinitionError::custom(
                    "world definition resource chunk has zero-sized payload",
                ));
            }
            if width_samples > chunk_size || height_samples > chunk_size {
                return Err(WorldDefinitionError::custom(
                    "world definition resource chunk exceeds canonical chunk sample span",
                ));
            }

            let origin_x = chunk_x
                .checked_mul(chunk_size)
                .ok_or_else(|| WorldDefinitionError::custom("resource chunk_x overflow"))?;
            let origin_z = chunk_z
                .checked_mul(chunk_size)
                .ok_or_else(|| WorldDefinitionError::custom("resource chunk_z overflow"))?;
            if origin_x + width_samples > resource_w || origin_z + height_samples > resource_h {
                return Err(WorldDefinitionError::custom(
                    "world definition resource chunk falls outside world bounds",
                ));
            }

            let payload = unpack_u16_blob(&blob, width_samples * height_samples)?;
            for local_z in 0..height_samples {
                let row_start = local_z * width_samples;
                let row_end = row_start + width_samples;
                let payload_row = &payload[row_start..row_end];
                for (local_x, value) in payload_row.iter().enumerate() {
                    if *value == 0 {
                        continue;
                    }
                    resource_deposits.set_coal_richness_at(
                        origin_x + local_x,
                        origin_z + local_z,
                        *value,
                    );
                }
            }
        }
    }
    validate_resource_deposit_dimensions(&config, &resource_deposits)?;
    validate_authored_water(&config, &lake_fills, &open_water_fills)?;

    Ok(LoadedWorldDefinition {
        name,
        config,
        terrain,
        lake_fills,
        open_water_fills,
        resource_deposits,
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

fn validate_resource_deposit_dimensions(
    config: &WorldConfig,
    resource_deposits: &ResourceDepositSystem,
) -> WorldDefinitionResult<()> {
    let expected_w = config.terrain_grid_width();
    let expected_h = config.terrain_grid_height();
    let (resource_w, resource_h) = resource_deposits.grid_dimensions();
    if resource_w != expected_w || resource_h != expected_h {
        return Err(WorldDefinitionError::custom(format!(
            "resource deposit size mismatch: got {}x{}, expected {}x{}",
            resource_w, resource_h, expected_w, expected_h
        )));
    }
    if (resource_deposits.cell_size_m() - config.terrain_cell_m).abs() > f32::EPSILON {
        return Err(WorldDefinitionError::custom(
            "resource deposit cell size must match terrain_cell_m",
        ));
    }
    Ok(())
}

fn validate_authored_water(
    config: &WorldConfig,
    lake_fills: &[AuthoredLakeFill],
    open_water_fills: &[AuthoredOpenWaterFill],
) -> WorldDefinitionResult<()> {
    for lake in lake_fills {
        validate_world_position(config, lake.world_x, lake.world_z, "authored lake fill")?;
        if !lake.surface_elevation_m.is_finite() {
            return Err(WorldDefinitionError::custom(
                "authored lake fill surface_elevation_m must be finite",
            ));
        }
    }
    for water in open_water_fills {
        validate_world_position(
            config,
            water.world_x,
            water.world_z,
            "authored open water fill",
        )?;
        if !water.surface_elevation_m.is_finite() {
            return Err(WorldDefinitionError::custom(
                "authored open water fill surface_elevation_m must be finite",
            ));
        }
    }
    Ok(())
}

fn validate_world_position(
    config: &WorldConfig,
    world_x: f32,
    world_z: f32,
    label: &str,
) -> WorldDefinitionResult<()> {
    if !world_x.is_finite() || !world_z.is_finite() {
        return Err(WorldDefinitionError::custom(format!(
            "{label} position must be finite"
        )));
    }
    let half_w = config.width_m * 0.5;
    let half_h = config.height_m * 0.5;
    if world_x < -half_w || world_x > half_w || world_z < -half_h || world_z > half_h {
        return Err(WorldDefinitionError::custom(format!(
            "{label} position falls outside world bounds"
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

fn pack_u16_slice(values: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * std::mem::size_of::<u16>());
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

fn unpack_u16_blob(blob: &[u8], expected_len: usize) -> WorldDefinitionResult<Vec<u16>> {
    let expected_bytes = expected_len
        .checked_mul(std::mem::size_of::<u16>())
        .ok_or_else(|| WorldDefinitionError::custom("u16 blob size overflow"))?;
    if blob.len() != expected_bytes {
        return Err(WorldDefinitionError::custom(format!(
            "u16 blob length mismatch: got {}, expected {}",
            blob.len(),
            expected_bytes
        )));
    }

    let mut out = Vec::with_capacity(expected_len);
    for chunk in blob.chunks_exact(2) {
        out.push(u16::from_le_bytes([chunk[0], chunk[1]]));
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
        let mut resource_deposits = ResourceDepositSystem::from_world_config(&config);
        resource_deposits.set_coal_richness_at(2, 2, 450);
        resource_deposits.set_coal_richness_at(7, 6, 900);

        save_world_definition_to_sqlite(
            &path,
            WorldDefinitionView {
                name: "Blank Test World",
                config: &config,
                terrain: &terrain,
                lake_fills: &[AuthoredLakeFill {
                    world_x: 0.0,
                    world_z: 0.0,
                    surface_elevation_m: 44.0,
                }],
                open_water_fills: &[AuthoredOpenWaterFill {
                    world_x: -20.0,
                    world_z: 15.0,
                    surface_elevation_m: 41.5,
                }],
                resource_deposits: &resource_deposits,
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
        assert_eq!(
            loaded.lake_fills,
            vec![AuthoredLakeFill {
                world_x: 0.0,
                world_z: 0.0,
                surface_elevation_m: 44.0,
            }]
        );
        assert_eq!(
            loaded.open_water_fills,
            vec![AuthoredOpenWaterFill {
                world_x: -20.0,
                world_z: 15.0,
                surface_elevation_m: 41.5,
            }]
        );
        assert_eq!(
            loaded.resource_deposits.clone_coal_richness_dense(),
            resource_deposits.clone_coal_richness_dense()
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
        let resource_deposits = ResourceDepositSystem::from_world_config(&config);

        save_world_definition_to_sqlite(
            &path,
            WorldDefinitionView {
                name: "Sparse Save",
                config: &config,
                terrain: &terrain,
                lake_fills: &[],
                open_water_fills: &[],
                resource_deposits: &resource_deposits,
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

    #[test]
    fn version_one_world_definition_still_loads_as_dry_world() {
        let path = temp_path("v1_load");
        let conn = Connection::open(&path).expect("temp sqlite should open");
        conn.execute_batch(
            r#"
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
"#,
        )
        .expect("v1 schema should create");
        conn.execute(
            "INSERT INTO world_definition_meta(format_version, name, width_m, height_m, terrain_cell_m, terrain_chunk_m, terrain_base_elevation_m, env_cell_m, zone_cell_m) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![1_i64, "Legacy World", 100.0_f32, 100.0_f32, 10.0_f32, 32.0_f32, 0.0_f32, 40.0_f32, 10.0_f32],
        )
        .expect("v1 metadata should insert");
        drop(conn);

        let loaded =
            load_world_definition_from_sqlite(&path).expect("v1 world definition should load");

        assert_eq!(loaded.name, "Legacy World");
        assert!(loaded.lake_fills.is_empty());
        assert!(loaded.open_water_fills.is_empty());
        assert!(loaded.resource_deposits.coal_is_empty());
        std::fs::remove_file(path).ok();
    }
}
