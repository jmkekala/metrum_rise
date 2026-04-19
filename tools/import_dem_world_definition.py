#!/usr/bin/env python3
"""
Build one `WorldDefinition` SQLite asset from DEM GeoTIFF tiles.

This importer is editor-time only. It is designed for real-world terrain imports
such as the National Land Survey of Finland `Korkeusmalli 2 m` tiles placed under:

    maps/raw/Kuopio/324km2/

The output matches the live `WorldDefinition` schema used by the Rust runtime.
The importer resamples the DEM into the authored terrain grid, converts it into
the current runtime's pre-`HEIGHT_SCALE` sample space, chunks the source terrain,
and writes a ready-to-open `.sqlite` world asset.
"""

from __future__ import annotations

import argparse
import math
import sqlite3
import sys
from pathlib import Path

import numpy as np

try:
    from osgeo import gdal
except ImportError as exc:  # pragma: no cover - environment-dependent
    raise SystemExit(
        "Python GDAL bindings are required. Install GDAL with Python support first."
    ) from exc


REPO_ROOT = Path(__file__).resolve().parents[1]
WORLD_DEFINITION_FORMAT_VERSION = 1
WORLD_DEFINITION_SCHEMA = """
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
"""
HEIGHT_SCALE = 20.0
DEFAULT_ENV_CELL_M = 40.0
DEFAULT_ZONE_CELL_M = 10.0
DEFAULT_TERRAIN_CHUNK_M = 512.0
DEFAULT_BASE_ELEVATION_M = 0.0


def resolve_repo_path(value: str) -> Path:
    path = Path(value)
    if path.is_absolute():
        return path
    return REPO_ROOT / path


def validate_positive(value: float, label: str) -> None:
    if not math.isfinite(value) or value <= 0.0:
        raise ValueError(f"{label} must be finite and > 0")


def discover_tifs(input_dir: Path) -> list[Path]:
    tifs = sorted(input_dir.rglob("*.tif"))
    if not tifs:
        raise ValueError(f"no .tif files found under {input_dir}")
    return tifs


def source_metadata(paths: list[Path]) -> tuple[float, float, float, float, float, float]:
    gdal.UseExceptions()
    sample_px = None
    epsg = None
    nodata = None
    min_e = float("inf")
    min_n = float("inf")
    max_e = float("-inf")
    max_n = float("-inf")

    for path in paths:
        ds = gdal.Open(str(path))
        if ds is None:
            raise RuntimeError(f"GDAL failed to open {path}")
        gt = ds.GetGeoTransform(can_return_null=True)
        if gt is None:
            raise ValueError(f"{path} has no geotransform")
        if abs(gt[2]) > 1e-6 or abs(gt[4]) > 1e-6:
            raise ValueError(f"{path} uses rotated/skewed geotransform; this importer only supports north-up rasters")
        if gt[1] <= 0.0 or gt[5] >= 0.0:
            raise ValueError(f"{path} has unsupported pixel size/orientation {gt[1]}, {gt[5]}")

        band = ds.GetRasterBand(1)
        if band is None:
            raise ValueError(f"{path} has no raster band 1")
        if ds.RasterCount != 1:
            raise ValueError(f"{path} is not single-band")
        path_nodata = band.GetNoDataValue()
        if path_nodata is None or not math.isfinite(path_nodata):
            raise ValueError(f"{path} is missing a finite NoData value")

        proj = ds.GetSpatialRef()
        if proj is None:
            raise ValueError(f"{path} has no spatial reference")
        path_epsg = proj.GetAuthorityCode(None)
        if path_epsg is None:
            raise ValueError(f"{path} has no EPSG authority code")

        px = float(gt[1])
        if sample_px is None:
            sample_px = px
            epsg = int(path_epsg)
            nodata = float(path_nodata)
        else:
            if abs(px - sample_px) > 1e-6:
                raise ValueError(f"{path} pixel size {px} does not match {sample_px}")
            if int(path_epsg) != epsg:
                raise ValueError(f"{path} EPSG:{path_epsg} does not match EPSG:{epsg}")
            if abs(float(path_nodata) - nodata) > 1e-6:
                raise ValueError(f"{path} nodata {path_nodata} does not match {nodata}")

        min_x = gt[0]
        max_y = gt[3]
        max_x = min_x + ds.RasterXSize * gt[1]
        min_y = max_y + ds.RasterYSize * gt[5]
        min_e = min(min_e, min_x, max_x)
        min_n = min(min_n, min_y, max_y)
        max_e = max(max_e, min_x, max_x)
        max_n = max(max_n, min_y, max_y)

    assert sample_px is not None
    assert epsg is not None
    assert nodata is not None
    return min_e, min_n, max_e, max_n, sample_px, nodata


def fill_border_nodata(array: np.ndarray, nodata: float) -> np.ndarray:
    out = np.array(array, copy=True)
    height, width = out.shape
    changed = True
    while changed:
        changed = False
        if width > 1 and np.any(out[:, 0] == nodata):
            mask = out[:, 0] == nodata
            out[mask, 0] = out[mask, 1]
            changed = True
        if width > 1 and np.any(out[:, -1] == nodata):
            mask = out[:, -1] == nodata
            out[mask, -1] = out[mask, -2]
            changed = True
        if height > 1 and np.any(out[0, :] == nodata):
            mask = out[0, :] == nodata
            out[0, mask] = out[1, mask]
            changed = True
        if height > 1 and np.any(out[-1, :] == nodata):
            mask = out[-1, :] == nodata
            out[-1, mask] = out[-2, mask]
            changed = True
    return out


def import_dem_array(
    source_paths: list[Path],
    width_m: float,
    height_m: float,
    min_e: float,
    min_n: float,
    max_e: float,
    max_n: float,
    terrain_cell_m: float,
    nodata: float,
) -> np.ndarray:
    width_samples = int(round(width_m / terrain_cell_m)) + 1
    height_samples = int(round(height_m / terrain_cell_m)) + 1
    bounds = (
        min_e - terrain_cell_m * 0.5,
        min_n - terrain_cell_m * 0.5,
        max_e + terrain_cell_m * 0.5,
        max_n + terrain_cell_m * 0.5,
    )
    ds = gdal.Warp(
        "",
        [str(path) for path in source_paths],
        format="MEM",
        outputBounds=bounds,
        width=width_samples,
        height=height_samples,
        resampleAlg="bilinear",
        outputType=gdal.GDT_Float32,
        srcNodata=nodata,
        dstNodata=nodata,
    )
    if ds is None:
        raise RuntimeError("GDAL Warp returned no dataset")
    band = ds.GetRasterBand(1)
    array = band.ReadAsArray()
    if array is None:
        raise RuntimeError("GDAL failed to read warped DEM")
    array = np.asarray(array, dtype=np.float32)
    if array.shape != (height_samples, width_samples):
        raise RuntimeError(
            f"unexpected warped array shape {array.shape}; expected {(height_samples, width_samples)}"
        )

    nodata_pixels = int(np.count_nonzero(array == nodata))
    if nodata_pixels > 0:
        array = fill_border_nodata(array, nodata)
        nodata_pixels = int(np.count_nonzero(array == nodata))
    if nodata_pixels > 0:
        raise RuntimeError(
            f"imported DEM still contains {nodata_pixels} nodata pixels after border fill; "
            "crop the source tiles or use a complete source coverage"
        )
    return array


def pack_f32_slice(values: np.ndarray) -> bytes:
    return np.asarray(values, dtype="<f4").tobytes(order="C")


def write_world_definition(
    output_path: Path,
    world_name: str,
    width_m: float,
    height_m: float,
    terrain_cell_m: float,
    terrain_chunk_m: float,
    terrain_base_elevation_m: float,
    env_cell_m: float,
    zone_cell_m: float,
    source_heights_runtime: np.ndarray,
) -> None:
    if source_heights_runtime.ndim != 2:
        raise ValueError("source terrain array must be 2D")
    height_samples, width_samples = source_heights_runtime.shape
    chunk_size = max(1, int(math.ceil(terrain_chunk_m / terrain_cell_m)))

    if output_path.exists():
        output_path.unlink()
    output_path.parent.mkdir(parents=True, exist_ok=True)

    conn = sqlite3.connect(output_path)
    try:
        conn.executescript(WORLD_DEFINITION_SCHEMA)
        conn.execute(
            """
            INSERT INTO world_definition_meta(
                format_version, name, width_m, height_m, terrain_cell_m, terrain_chunk_m,
                terrain_base_elevation_m, env_cell_m, zone_cell_m
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                WORLD_DEFINITION_FORMAT_VERSION,
                world_name.strip(),
                width_m,
                height_m,
                terrain_cell_m,
                terrain_chunk_m,
                terrain_base_elevation_m,
                env_cell_m,
                zone_cell_m,
            ),
        )

        chunk_cols = math.ceil(width_samples / chunk_size)
        chunk_rows = math.ceil(height_samples / chunk_size)
        base = np.float32(terrain_base_elevation_m)
        for chunk_z in range(chunk_rows):
            for chunk_x in range(chunk_cols):
                origin_x = chunk_x * chunk_size
                origin_z = chunk_z * chunk_size
                chunk = source_heights_runtime[
                    origin_z : min(origin_z + chunk_size, height_samples),
                    origin_x : min(origin_x + chunk_size, width_samples),
                ]
                if not np.any(chunk != base):
                    continue
                conn.execute(
                    """
                    INSERT INTO world_terrain_chunks(
                        chunk_x, chunk_z, width_samples, height_samples, source_height_blob_f32_le
                    ) VALUES (?, ?, ?, ?, ?)
                    """,
                    (
                        chunk_x,
                        chunk_z,
                        int(chunk.shape[1]),
                        int(chunk.shape[0]),
                        pack_f32_slice(chunk),
                    ),
                )
        conn.commit()
    finally:
        conn.close()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Import DEM GeoTIFF tiles into a WorldDefinition SQLite asset"
    )
    parser.add_argument(
        "--input-dir",
        default="maps/raw/Kuopio/324km2",
        help="Directory containing source GeoTIFF tiles (repo-relative by default)",
    )
    parser.add_argument(
        "--output",
        default="maps/processed/Kuopio/kuopio_324km2_10m.sqlite",
        help="Output WorldDefinition path (repo-relative by default)",
    )
    parser.add_argument(
        "--world-name",
        default="Kuopio 324km2",
        help="User-facing world name to store in the WorldDefinition",
    )
    parser.add_argument(
        "--terrain-cell-m",
        type=float,
        default=10.0,
        help="Authored terrain sample spacing in metres (must be >= source pixel size)",
    )
    parser.add_argument(
        "--terrain-chunk-m",
        type=float,
        default=DEFAULT_TERRAIN_CHUNK_M,
        help="Authored terrain chunk span in metres",
    )
    parser.add_argument(
        "--env-cell-m",
        type=float,
        default=DEFAULT_ENV_CELL_M,
        help="Environmental grid cell size to store in the world config",
    )
    parser.add_argument(
        "--zone-cell-m",
        type=float,
        default=DEFAULT_ZONE_CELL_M,
        help="Zoning grid cell size to store in the world config",
    )
    parser.add_argument(
        "--terrain-base-elevation-m",
        type=float,
        default=DEFAULT_BASE_ELEVATION_M,
        help="Default base terrain sample value for untouched chunks in the output world",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    input_dir = resolve_repo_path(args.input_dir)
    output_path = resolve_repo_path(args.output)
    validate_positive(args.terrain_cell_m, "terrain_cell_m")
    validate_positive(args.terrain_chunk_m, "terrain_chunk_m")
    validate_positive(args.env_cell_m, "env_cell_m")
    validate_positive(args.zone_cell_m, "zone_cell_m")
    if not math.isfinite(args.terrain_base_elevation_m):
        raise SystemExit("terrain_base_elevation_m must be finite")

    source_paths = discover_tifs(input_dir)
    min_e, min_n, max_e, max_n, source_sample_m, nodata = source_metadata(source_paths)
    if args.terrain_cell_m + 1e-6 < source_sample_m:
        raise SystemExit(
            f"terrain_cell_m {args.terrain_cell_m}m is finer than the source pixel size {source_sample_m}m"
        )

    width_m = max_e - min_e
    height_m = max_n - min_n
    elevation_m = import_dem_array(
        source_paths=source_paths,
        width_m=width_m,
        height_m=height_m,
        min_e=min_e,
        min_n=min_n,
        max_e=max_e,
        max_n=max_n,
        terrain_cell_m=args.terrain_cell_m,
        nodata=nodata,
    )
    runtime_samples = elevation_m / HEIGHT_SCALE

    write_world_definition(
        output_path=output_path,
        world_name=args.world_name,
        width_m=width_m,
        height_m=height_m,
        terrain_cell_m=args.terrain_cell_m,
        terrain_chunk_m=args.terrain_chunk_m,
        terrain_base_elevation_m=args.terrain_base_elevation_m,
        env_cell_m=args.env_cell_m,
        zone_cell_m=args.zone_cell_m,
        source_heights_runtime=runtime_samples,
    )

    width_samples = int(round(width_m / args.terrain_cell_m)) + 1
    height_samples = int(round(height_m / args.terrain_cell_m)) + 1
    print(
        f"imported {len(source_paths)} DEM tile(s) from {input_dir.relative_to(REPO_ROOT).as_posix()} "
        f"into {output_path.relative_to(REPO_ROOT).as_posix()} "
        f"({width_m/1000.0:.1f} km x {height_m/1000.0:.1f} km, "
        f"{width_samples} x {height_samples} samples @ {args.terrain_cell_m:.1f} m)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
