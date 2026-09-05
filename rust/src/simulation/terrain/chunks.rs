// SPDX-License-Identifier: GPL-2.0-only

//! Terrain chunk asset manifest and disk loader.
//!
//! The first terrain-import slice bakes one directory per chunk under
//! `terrain/<world>/import/chunks_512m/cx_####_cz_####/`. Each directory contains a
//! `chunk.toml` manifest plus one or more little-endian `f32` height buffers for runtime LODs.
//!
//! This module owns the Rust-side contract for reading that format. It performs strict manifest
//! validation, validates payload sizes against the declared raster dimensions, and exposes the
//! resulting row-major height buffers as ordinary `Vec<f32>` allocations.

use serde::Deserialize;
use std::{
    collections::HashSet,
    fmt, fs, io,
    path::{Component, Path, PathBuf},
};

const CURRENT_TERRAIN_CHUNK_ASSET_VERSION: u32 = 1;
const SUPPORTED_RASTER_SEMANTICS: &str = "pixel_is_area";
const SUPPORTED_SAMPLE_FORMAT: &str = "float32_le";
const FLOAT32_BYTES: usize = std::mem::size_of::<f32>();
const DIMENSION_EPSILON_M: f32 = 0.001;

/// Errors produced while parsing or validating `chunk.toml`.
#[derive(Debug)]
pub enum TerrainChunkManifestError {
    /// TOML syntax or schema mismatch reported by the `toml` crate.
    TomlParse(toml::de::Error),
    /// Structural or semantic validation failure in the chunk metadata.
    Validation(String),
}

impl fmt::Display for TerrainChunkManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TomlParse(err) => write!(f, "chunk manifest TOML parse error: {err}"),
            Self::Validation(msg) => write!(f, "chunk manifest validation error: {msg}"),
        }
    }
}

impl std::error::Error for TerrainChunkManifestError {}

impl From<toml::de::Error> for TerrainChunkManifestError {
    fn from(err: toml::de::Error) -> Self {
        Self::TomlParse(err)
    }
}

/// Errors produced while loading a terrain chunk asset directory from disk.
#[derive(Debug)]
pub enum TerrainChunkLoadError {
    /// One of the manifest or payload files could not be read from disk.
    Io {
        /// File path that failed to read.
        path: PathBuf,
        /// Underlying I/O error.
        source: io::Error,
    },
    /// The `chunk.toml` file failed to parse or validate.
    Manifest {
        /// Path to the manifest file that failed.
        path: PathBuf,
        /// Underlying parse or validation error.
        source: TerrainChunkManifestError,
    },
    /// Payload bytes or resolved paths failed structural validation after the manifest parsed.
    Validation(String),
}

impl fmt::Display for TerrainChunkLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(
                    f,
                    "could not read terrain chunk file '{}': {source}",
                    path.display()
                )
            }
            Self::Manifest { path, source } => {
                write!(
                    f,
                    "could not load terrain chunk manifest '{}': {source}",
                    path.display()
                )
            }
            Self::Validation(msg) => write!(f, "terrain chunk load validation error: {msg}"),
        }
    }
}

impl std::error::Error for TerrainChunkLoadError {}

/// On-disk `chunk.toml` metadata for one baked terrain chunk.
#[derive(Debug, Clone, Deserialize)]
pub struct TerrainChunkManifest {
    /// Asset schema version for this internal chunk format.
    pub chunk_asset_version: u32,
    /// Stable world identifier that links the chunk back to its imported source dataset.
    pub world_id: String,
    /// Chunk coordinate on the world X axis.
    pub chunk_x: i32,
    /// Chunk coordinate on the world Z axis.
    pub chunk_z: i32,
    /// Canonical chunk width target in metres. Border chunks may be smaller in actual extent.
    pub terrain_chunk_m: f32,
    /// Authoritative source sample spacing from the imported DEM.
    pub source_sample_m: f32,
    /// Projected CRS EPSG code used by the chunk bounds.
    pub epsg: u32,
    /// Human-readable CRS label copied from the importer metadata.
    pub crs_name: String,
    /// Raster interpretation label. The current format only supports `pixel_is_area`.
    pub raster_semantics: String,
    /// Binary height payload encoding. The current format only supports `float32_le`.
    pub sample_format: String,
    /// Minimum projected easting covered by this chunk.
    pub min_e_m: f32,
    /// Minimum projected northing covered by this chunk.
    pub min_n_m: f32,
    /// Maximum projected easting covered by this chunk.
    pub max_e_m: f32,
    /// Maximum projected northing covered by this chunk.
    pub max_n_m: f32,
    /// Actual chunk width in metres after clipping to the imported world bounds.
    pub width_m: f32,
    /// Actual chunk height in metres after clipping to the imported world bounds.
    pub height_m: f32,
    /// Raster width of the authoritative base import LOD.
    pub base_width_px: usize,
    /// Raster height of the authoritative base import LOD.
    pub base_height_px: usize,
    /// Nodata sentinel copied from the importer. Payloads may or may not contain it.
    pub nodata_value: f32,
    /// Count of nodata pixels present in the authoritative base LOD.
    pub nodata_pixel_count: usize,
    /// Minimum height value recorded across the authoritative base LOD.
    pub min_height_m: f32,
    /// Maximum height value recorded across the authoritative base LOD.
    pub max_height_m: f32,
    /// Number of source GeoTIFF tiles that contributed samples to this chunk.
    pub overlap_tile_count: usize,
    /// Source GeoTIFF filenames that overlapped this chunk during import.
    pub source_tiles: Vec<String>,
    /// Baked runtime LOD payloads for this chunk.
    pub lods: Vec<TerrainChunkLodManifest>,
}

impl TerrainChunkManifest {
    /// Parses and validates a `chunk.toml` document.
    pub fn from_str(s: &str) -> Result<Self, TerrainChunkManifestError> {
        let manifest: Self = toml::from_str(s)?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validates this chunk manifest against the current internal asset contract.
    pub fn validate(&self) -> Result<(), TerrainChunkManifestError> {
        if self.chunk_asset_version != CURRENT_TERRAIN_CHUNK_ASSET_VERSION {
            return Err(TerrainChunkManifestError::Validation(format!(
                "chunk ({}, {}): chunk_asset_version {} is not supported (expected {})",
                self.chunk_x,
                self.chunk_z,
                self.chunk_asset_version,
                CURRENT_TERRAIN_CHUNK_ASSET_VERSION
            )));
        }

        if self.world_id.trim().is_empty() {
            return Err(TerrainChunkManifestError::Validation(format!(
                "chunk ({}, {}): world_id must not be empty",
                self.chunk_x, self.chunk_z
            )));
        }

        validate_positive_f32(
            self.terrain_chunk_m,
            "terrain_chunk_m",
            self.chunk_x,
            self.chunk_z,
        )?;
        validate_positive_f32(
            self.source_sample_m,
            "source_sample_m",
            self.chunk_x,
            self.chunk_z,
        )?;
        validate_positive_f32(self.width_m, "width_m", self.chunk_x, self.chunk_z)?;
        validate_positive_f32(self.height_m, "height_m", self.chunk_x, self.chunk_z)?;

        if self.width_m > self.terrain_chunk_m + DIMENSION_EPSILON_M {
            return Err(TerrainChunkManifestError::Validation(format!(
                "chunk ({}, {}): width_m {} exceeds terrain_chunk_m {}",
                self.chunk_x, self.chunk_z, self.width_m, self.terrain_chunk_m
            )));
        }
        if self.height_m > self.terrain_chunk_m + DIMENSION_EPSILON_M {
            return Err(TerrainChunkManifestError::Validation(format!(
                "chunk ({}, {}): height_m {} exceeds terrain_chunk_m {}",
                self.chunk_x, self.chunk_z, self.height_m, self.terrain_chunk_m
            )));
        }

        if self.raster_semantics != SUPPORTED_RASTER_SEMANTICS {
            return Err(TerrainChunkManifestError::Validation(format!(
                "chunk ({}, {}): raster_semantics '{}' is not supported (expected '{}')",
                self.chunk_x, self.chunk_z, self.raster_semantics, SUPPORTED_RASTER_SEMANTICS
            )));
        }
        if self.sample_format != SUPPORTED_SAMPLE_FORMAT {
            return Err(TerrainChunkManifestError::Validation(format!(
                "chunk ({}, {}): sample_format '{}' is not supported (expected '{}')",
                self.chunk_x, self.chunk_z, self.sample_format, SUPPORTED_SAMPLE_FORMAT
            )));
        }

        if self.max_e_m <= self.min_e_m {
            return Err(TerrainChunkManifestError::Validation(format!(
                "chunk ({}, {}): max_e_m must be greater than min_e_m",
                self.chunk_x, self.chunk_z
            )));
        }
        if self.max_n_m <= self.min_n_m {
            return Err(TerrainChunkManifestError::Validation(format!(
                "chunk ({}, {}): max_n_m must be greater than min_n_m",
                self.chunk_x, self.chunk_z
            )));
        }

        validate_extent_matches_dimension(
            self.max_e_m - self.min_e_m,
            self.width_m,
            "easting extent",
            self.chunk_x,
            self.chunk_z,
        )?;
        validate_extent_matches_dimension(
            self.max_n_m - self.min_n_m,
            self.height_m,
            "northing extent",
            self.chunk_x,
            self.chunk_z,
        )?;

        if self.base_width_px == 0 || self.base_height_px == 0 {
            return Err(TerrainChunkManifestError::Validation(format!(
                "chunk ({}, {}): base raster dimensions must be greater than zero",
                self.chunk_x, self.chunk_z
            )));
        }

        validate_raster_dimensions(
            self.width_m,
            self.source_sample_m,
            self.base_width_px,
            "base_width_px",
            self.chunk_x,
            self.chunk_z,
        )?;
        validate_raster_dimensions(
            self.height_m,
            self.source_sample_m,
            self.base_height_px,
            "base_height_px",
            self.chunk_x,
            self.chunk_z,
        )?;

        if self.max_height_m < self.min_height_m {
            return Err(TerrainChunkManifestError::Validation(format!(
                "chunk ({}, {}): max_height_m {} is less than min_height_m {}",
                self.chunk_x, self.chunk_z, self.max_height_m, self.min_height_m
            )));
        }

        if self.overlap_tile_count != self.source_tiles.len() {
            return Err(TerrainChunkManifestError::Validation(format!(
                "chunk ({}, {}): overlap_tile_count {} does not match source_tiles length {}",
                self.chunk_x,
                self.chunk_z,
                self.overlap_tile_count,
                self.source_tiles.len()
            )));
        }

        if self.source_tiles.iter().any(|tile| tile.trim().is_empty()) {
            return Err(TerrainChunkManifestError::Validation(format!(
                "chunk ({}, {}): source_tiles must not contain empty filenames",
                self.chunk_x, self.chunk_z
            )));
        }

        if self.lods.is_empty() {
            return Err(TerrainChunkManifestError::Validation(format!(
                "chunk ({}, {}): at least one LOD entry is required",
                self.chunk_x, self.chunk_z
            )));
        }

        let mut seen_lod_names = HashSet::new();
        let mut previous_sample_m = 0.0_f32;
        for (lod_index, lod) in self.lods.iter().enumerate() {
            lod.validate(self.chunk_x, self.chunk_z)?;

            if !seen_lod_names.insert(lod.lod_name.clone()) {
                return Err(TerrainChunkManifestError::Validation(format!(
                    "chunk ({}, {}): duplicate lod_name '{}'",
                    self.chunk_x, self.chunk_z, lod.lod_name
                )));
            }

            if lod.sample_m < self.source_sample_m - DIMENSION_EPSILON_M {
                return Err(TerrainChunkManifestError::Validation(format!(
                    "chunk ({}, {}): LOD '{}' sample_m {} is finer than source_sample_m {}",
                    self.chunk_x, self.chunk_z, lod.lod_name, lod.sample_m, self.source_sample_m
                )));
            }

            if lod_index == 0 {
                if !approx_eq(lod.sample_m, self.source_sample_m) {
                    return Err(TerrainChunkManifestError::Validation(format!(
                        "chunk ({}, {}): first LOD sample_m {} must equal source_sample_m {}",
                        self.chunk_x, self.chunk_z, lod.sample_m, self.source_sample_m
                    )));
                }
                if lod.width_px != self.base_width_px || lod.height_px != self.base_height_px {
                    return Err(TerrainChunkManifestError::Validation(format!(
                        "chunk ({}, {}): first LOD dimensions {}x{} do not match base raster {}x{}",
                        self.chunk_x,
                        self.chunk_z,
                        lod.width_px,
                        lod.height_px,
                        self.base_width_px,
                        self.base_height_px
                    )));
                }
                if !approx_eq(lod.min_height_m, self.min_height_m)
                    || !approx_eq(lod.max_height_m, self.max_height_m)
                {
                    return Err(TerrainChunkManifestError::Validation(format!(
                        "chunk ({}, {}): first LOD min/max heights must match chunk min/max heights",
                        self.chunk_x, self.chunk_z
                    )));
                }
            } else if lod.sample_m <= previous_sample_m + DIMENSION_EPSILON_M {
                return Err(TerrainChunkManifestError::Validation(format!(
                    "chunk ({}, {}): LOD sample_m values must increase strictly (found {} after {})",
                    self.chunk_x, self.chunk_z, lod.sample_m, previous_sample_m
                )));
            }

            validate_raster_dimensions(
                self.width_m,
                lod.sample_m,
                lod.width_px,
                "lod.width_px",
                self.chunk_x,
                self.chunk_z,
            )?;
            validate_raster_dimensions(
                self.height_m,
                lod.sample_m,
                lod.height_px,
                "lod.height_px",
                self.chunk_x,
                self.chunk_z,
            )?;

            previous_sample_m = lod.sample_m;
        }

        Ok(())
    }
}

/// Metadata for one baked terrain LOD payload.
#[derive(Debug, Clone, Deserialize)]
pub struct TerrainChunkLodManifest {
    /// Stable importer-assigned LOD label such as `lod0`.
    pub lod_name: String,
    /// Sample spacing in metres for this raster payload.
    pub sample_m: f32,
    /// Raster width in samples.
    pub width_px: usize,
    /// Raster height in samples.
    pub height_px: usize,
    /// Relative path to the little-endian `f32` payload file within the chunk directory.
    pub relative_path: String,
    /// Minimum height value recorded in this LOD payload.
    pub min_height_m: f32,
    /// Maximum height value recorded in this LOD payload.
    pub max_height_m: f32,
}

impl TerrainChunkLodManifest {
    fn validate(&self, chunk_x: i32, chunk_z: i32) -> Result<(), TerrainChunkManifestError> {
        if self.lod_name.trim().is_empty() {
            return Err(TerrainChunkManifestError::Validation(format!(
                "chunk ({}, {}): lod_name must not be empty",
                chunk_x, chunk_z
            )));
        }
        validate_positive_f32(self.sample_m, "lod.sample_m", chunk_x, chunk_z)?;
        if self.width_px == 0 || self.height_px == 0 {
            return Err(TerrainChunkManifestError::Validation(format!(
                "chunk ({}, {}): LOD '{}' dimensions must be greater than zero",
                chunk_x, chunk_z, self.lod_name
            )));
        }
        if self.max_height_m < self.min_height_m {
            return Err(TerrainChunkManifestError::Validation(format!(
                "chunk ({}, {}): LOD '{}' max_height_m {} is less than min_height_m {}",
                chunk_x, chunk_z, self.lod_name, self.max_height_m, self.min_height_m
            )));
        }
        validate_relative_payload_path(&self.relative_path, chunk_x, chunk_z, &self.lod_name)?;
        Ok(())
    }
}

/// Fully loaded terrain chunk asset with all declared LOD payloads resident in memory.
#[derive(Debug, Clone)]
pub struct TerrainChunkAsset {
    /// Validated on-disk manifest that describes this chunk.
    pub manifest: TerrainChunkManifest,
    /// Loaded height payloads in the same order as [`TerrainChunkManifest::lods`].
    pub lods: Vec<TerrainChunkLodAsset>,
}

impl TerrainChunkAsset {
    /// Loads a chunk asset directory containing `chunk.toml` and its referenced payload files.
    pub fn load_from_dir(path: impl AsRef<Path>) -> Result<Self, TerrainChunkLoadError> {
        let dir = path.as_ref();
        let manifest_path = dir.join("chunk.toml");
        let manifest_source =
            fs::read_to_string(&manifest_path).map_err(|source| TerrainChunkLoadError::Io {
                path: manifest_path.clone(),
                source,
            })?;
        let manifest = TerrainChunkManifest::from_str(&manifest_source).map_err(|source| {
            TerrainChunkLoadError::Manifest {
                path: manifest_path.clone(),
                source,
            }
        })?;

        let mut lods = Vec::with_capacity(manifest.lods.len());
        for lod_manifest in &manifest.lods {
            lods.push(load_lod_payload(
                dir,
                lod_manifest,
                manifest.chunk_x,
                manifest.chunk_z,
            )?);
        }

        Ok(Self { manifest, lods })
    }

    /// Returns the authoritative imported base LOD (`lod0`).
    ///
    /// The manifest validator guarantees at least one LOD and that the first entry matches the
    /// source sample spacing.
    pub fn base_lod(&self) -> &TerrainChunkLodAsset {
        &self.lods[0]
    }

    /// Returns the first LOD with the given importer-assigned `lod_name`, if present.
    pub fn lod_by_name(&self, lod_name: &str) -> Option<&TerrainChunkLodAsset> {
        self.lods
            .iter()
            .find(|lod| lod.metadata.lod_name == lod_name)
    }
}

/// One loaded terrain LOD payload plus its manifest metadata.
#[derive(Debug, Clone)]
pub struct TerrainChunkLodAsset {
    /// On-disk metadata for this LOD.
    pub metadata: TerrainChunkLodManifest,
    /// Row-major height buffer in metres.
    pub heights_m: Vec<f32>,
}

impl TerrainChunkLodAsset {
    /// Returns the number of samples stored in [`TerrainChunkLodAsset::heights_m`].
    pub fn sample_count(&self) -> usize {
        self.heights_m.len()
    }
}

fn load_lod_payload(
    dir: &Path,
    lod_manifest: &TerrainChunkLodManifest,
    chunk_x: i32,
    chunk_z: i32,
) -> Result<TerrainChunkLodAsset, TerrainChunkLoadError> {
    let payload_path = dir.join(&lod_manifest.relative_path);
    let bytes = fs::read(&payload_path).map_err(|source| TerrainChunkLoadError::Io {
        path: payload_path.clone(),
        source,
    })?;

    let pixel_count = lod_manifest
        .width_px
        .checked_mul(lod_manifest.height_px)
        .ok_or_else(|| {
            TerrainChunkLoadError::Validation(format!(
                "chunk ({}, {}): LOD '{}' pixel count overflow for {}x{}",
                chunk_x,
                chunk_z,
                lod_manifest.lod_name,
                lod_manifest.width_px,
                lod_manifest.height_px
            ))
        })?;
    let expected_bytes = pixel_count.checked_mul(FLOAT32_BYTES).ok_or_else(|| {
        TerrainChunkLoadError::Validation(format!(
            "chunk ({}, {}): LOD '{}' byte size overflow for {} samples",
            chunk_x, chunk_z, lod_manifest.lod_name, pixel_count
        ))
    })?;

    if bytes.len() != expected_bytes {
        return Err(TerrainChunkLoadError::Validation(format!(
            "chunk ({}, {}): LOD '{}' payload '{}' has {} bytes, expected {}",
            chunk_x,
            chunk_z,
            lod_manifest.lod_name,
            payload_path.display(),
            bytes.len(),
            expected_bytes
        )));
    }

    let mut heights_m = Vec::with_capacity(pixel_count);
    for sample_bytes in bytes.chunks_exact(FLOAT32_BYTES) {
        let value = f32::from_le_bytes([
            sample_bytes[0],
            sample_bytes[1],
            sample_bytes[2],
            sample_bytes[3],
        ]);
        if !value.is_finite() {
            return Err(TerrainChunkLoadError::Validation(format!(
                "chunk ({}, {}): LOD '{}' contains a non-finite height value",
                chunk_x, chunk_z, lod_manifest.lod_name
            )));
        }
        heights_m.push(value);
    }

    Ok(TerrainChunkLodAsset {
        metadata: lod_manifest.clone(),
        heights_m,
    })
}

fn validate_positive_f32(
    value: f32,
    field_name: &str,
    chunk_x: i32,
    chunk_z: i32,
) -> Result<(), TerrainChunkManifestError> {
    if !value.is_finite() || value <= 0.0 {
        return Err(TerrainChunkManifestError::Validation(format!(
            "chunk ({}, {}): {} must be finite and greater than zero (got {})",
            chunk_x, chunk_z, field_name, value
        )));
    }
    Ok(())
}

fn validate_extent_matches_dimension(
    extent_m: f32,
    dimension_m: f32,
    label: &str,
    chunk_x: i32,
    chunk_z: i32,
) -> Result<(), TerrainChunkManifestError> {
    if !approx_eq(extent_m, dimension_m) {
        return Err(TerrainChunkManifestError::Validation(format!(
            "chunk ({}, {}): {} {} does not match declared dimension {}",
            chunk_x, chunk_z, label, extent_m, dimension_m
        )));
    }
    Ok(())
}

fn validate_raster_dimensions(
    extent_m: f32,
    sample_m: f32,
    dimension_px: usize,
    label: &str,
    chunk_x: i32,
    chunk_z: i32,
) -> Result<(), TerrainChunkManifestError> {
    let expected = extent_m / sample_m;
    if !approx_eq(expected, dimension_px as f32) {
        return Err(TerrainChunkManifestError::Validation(format!(
            "chunk ({}, {}): {} {} does not match extent/sample ratio {} / {} = {}",
            chunk_x, chunk_z, label, dimension_px, extent_m, sample_m, expected
        )));
    }
    Ok(())
}

fn validate_relative_payload_path(
    relative_path: &str,
    chunk_x: i32,
    chunk_z: i32,
    lod_name: &str,
) -> Result<(), TerrainChunkManifestError> {
    if relative_path.trim().is_empty() {
        return Err(TerrainChunkManifestError::Validation(format!(
            "chunk ({}, {}): LOD '{}' relative_path must not be empty",
            chunk_x, chunk_z, lod_name
        )));
    }

    let path = Path::new(relative_path);
    if path.is_absolute() {
        return Err(TerrainChunkManifestError::Validation(format!(
            "chunk ({}, {}): LOD '{}' relative_path '{}' must not be absolute",
            chunk_x, chunk_z, lod_name, relative_path
        )));
    }

    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(TerrainChunkManifestError::Validation(format!(
                    "chunk ({}, {}): LOD '{}' relative_path '{}' escapes the chunk directory",
                    chunk_x, chunk_z, lod_name, relative_path
                )));
            }
        }
    }

    Ok(())
}

fn approx_eq(a: f32, b: f32) -> bool {
    (a - b).abs() <= DIMENSION_EPSILON_M
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_test_dir(prefix: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("metrum_terrain_chunk_{prefix}_{suffix}"))
    }

    fn write_lod_payload(path: &Path, heights_m: &[f32]) {
        let mut bytes = Vec::with_capacity(heights_m.len() * FLOAT32_BYTES);
        for value in heights_m {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        fs::write(path, bytes).expect("test payload should write");
    }

    fn write_test_chunk(
        dir: &Path,
        chunk_x: i32,
        chunk_z: i32,
        width_m: f32,
        height_m: f32,
        base_sample_m: f32,
        lod_samples_m: &[f32],
        min_height_m: f32,
        max_height_m: f32,
    ) {
        fs::create_dir_all(dir).expect("test chunk dir should create");

        let base_width_px = (width_m / base_sample_m) as usize;
        let base_height_px = (height_m / base_sample_m) as usize;
        let min_e_m = chunk_x as f32 * 512.0;
        let min_n_m = chunk_z as f32 * 512.0;
        let max_e_m = min_e_m + width_m;
        let max_n_m = min_n_m + height_m;

        let mut lod_manifest = String::new();
        for (index, sample_m) in lod_samples_m.iter().enumerate() {
            let width_px = (width_m / sample_m) as usize;
            let height_px = (height_m / sample_m) as usize;
            let relative_path = format!("height_lod{index}_{sample_m:.0}m.f32");
            let heights_m = vec![min_height_m; width_px * height_px];
            write_lod_payload(&dir.join(&relative_path), &heights_m);
            lod_manifest.push_str(&format!(
                r#"

[[lods]]
lod_name = "lod{index}"
sample_m = {sample_m}
width_px = {width_px}
height_px = {height_px}
relative_path = "{relative_path}"
min_height_m = {min_height_m}
max_height_m = {max_height_m}
"#
            ));
        }

        let manifest = format!(
            r#"chunk_asset_version = 1
world_id = "test_world"
chunk_x = {chunk_x}
chunk_z = {chunk_z}
terrain_chunk_m = 512.0
source_sample_m = {base_sample_m}
epsg = 3067
crs_name = "ETRS89 / TM35FIN(E,N)"
raster_semantics = "pixel_is_area"
sample_format = "float32_le"
min_e_m = {min_e_m}
min_n_m = {min_n_m}
max_e_m = {max_e_m}
max_n_m = {max_n_m}
width_m = {width_m}
height_m = {height_m}
base_width_px = {base_width_px}
base_height_px = {base_height_px}
nodata_value = -9999.0
nodata_pixel_count = 0
min_height_m = {min_height_m}
max_height_m = {max_height_m}
overlap_tile_count = 1
source_tiles = ["fixture_tile.tif"]
{lod_manifest}
"#
        );

        fs::write(dir.join("chunk.toml"), manifest).expect("test manifest should write");
    }

    fn sample_chunk_dir() -> PathBuf {
        let dir = unique_test_dir("sample");
        write_test_chunk(
            &dir,
            11,
            70,
            512.0,
            512.0,
            2.0,
            &[2.0, 4.0, 8.0, 32.0],
            123.5,
            123.5,
        );
        dir
    }

    fn border_chunk_dir() -> PathBuf {
        let dir = unique_test_dir("border");
        write_test_chunk(
            &dir,
            117,
            117,
            96.0,
            96.0,
            2.0,
            &[2.0, 4.0, 8.0, 32.0],
            77.0,
            77.0,
        );
        dir
    }

    #[test]
    fn terrain_chunk_manifest_rejects_duplicate_lod_names() {
        let manifest_toml = r#"
chunk_asset_version = 1
world_id = "kuopio"
chunk_x = 0
chunk_z = 0
terrain_chunk_m = 512.0
source_sample_m = 2.0
epsg = 3067
crs_name = "ETRS89 / TM35FIN(E,N)"
raster_semantics = "pixel_is_area"
sample_format = "float32_le"
min_e_m = 0.0
min_n_m = 0.0
max_e_m = 512.0
max_n_m = 512.0
width_m = 512.0
height_m = 512.0
base_width_px = 256
base_height_px = 256
nodata_value = -9999.0
nodata_pixel_count = 0
min_height_m = 0.0
max_height_m = 10.0
overlap_tile_count = 1
source_tiles = ["P5121E.tif"]

[[lods]]
lod_name = "lod0"
sample_m = 2.0
width_px = 256
height_px = 256
relative_path = "height_lod0_2m.f32"
min_height_m = 0.0
max_height_m = 10.0

[[lods]]
lod_name = "lod0"
sample_m = 4.0
width_px = 128
height_px = 128
relative_path = "height_lod1_4m.f32"
min_height_m = 0.0
max_height_m = 9.0
"#;

        let error = TerrainChunkManifest::from_str(manifest_toml)
            .expect_err("duplicate lod_name should fail");
        assert!(error.to_string().contains("duplicate lod_name"));
    }

    #[test]
    fn terrain_chunk_asset_loads_sample_chunk() {
        let chunk =
            TerrainChunkAsset::load_from_dir(sample_chunk_dir()).expect("sample chunk should load");

        assert_eq!(chunk.manifest.world_id, "test_world");
        assert_eq!(chunk.manifest.chunk_x, 11);
        assert_eq!(chunk.manifest.chunk_z, 70);
        assert_eq!(chunk.lods.len(), 4);

        let base_lod = chunk.base_lod();
        assert_eq!(base_lod.metadata.lod_name, "lod0");
        assert_eq!(base_lod.metadata.sample_m, 2.0);
        assert_eq!(base_lod.metadata.width_px, 256);
        assert_eq!(base_lod.metadata.height_px, 256);
        assert_eq!(base_lod.sample_count(), 256 * 256);
        assert!(base_lod.heights_m.iter().all(|value| value.is_finite()));
    }

    #[test]
    fn terrain_chunk_asset_loads_partial_border_chunk() {
        let chunk =
            TerrainChunkAsset::load_from_dir(border_chunk_dir()).expect("border chunk should load");

        assert_eq!(chunk.manifest.chunk_x, 117);
        assert_eq!(chunk.manifest.chunk_z, 117);
        assert_eq!(chunk.manifest.width_m, 96.0);
        assert_eq!(chunk.manifest.height_m, 96.0);

        let far_lod = chunk.lod_by_name("lod3").expect("lod3 should exist");
        assert_eq!(far_lod.metadata.width_px, 3);
        assert_eq!(far_lod.metadata.height_px, 3);
        assert_eq!(far_lod.sample_count(), 9);
    }
}
