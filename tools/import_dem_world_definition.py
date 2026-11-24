#!/usr/bin/env python3
"""
Build one `WorldDefinition` SQLite asset from DEM GeoTIFF tiles.

This importer is editor-time only. It is designed for real-world terrain imports
such as the National Land Survey of Finland `Korkeusmalli 2 m` tiles placed under:

    maps/raw/Kuopio/324km2/

The output matches the live `WorldDefinition` schema used by the Rust runtime.
The importer resamples the DEM into the authored terrain grid, keeps the heights in
metres (the runtime's unit since world format 6), chunks the source terrain, and
writes a ready-to-open `.sqlite` world asset.

By default the world covers the source tiles' extent. `--size-m` with `--centre-e` and
`--centre-n` cuts a square world of that side around a source-coordinate centre instead:
sample (i, j) lies at E = centre_e - size/2 + i * cell, N = centre_n + size/2 - j * cell,
which is world X = E - centre_e and world Z = centre_n - N (north is -Z).

`--hydrography` adds lakes and rivers from a national hydrography GeoPackage in the same
coordinate system (layers `jarvi`, `virtavesialue`, `matalikko`); see `dem_water.py` for
the bed, shore and fill rules. The output is deterministic: the same inputs give the same
bytes.
"""

from __future__ import annotations

import argparse
import json
import math
import sqlite3
import sys
from pathlib import Path

import numpy as np

gdal = None


def require_gdal():
    # Imported on use so the pure helpers stay testable without GDAL.
    global gdal
    if gdal is None:
        try:
            from osgeo import gdal as module
        except ImportError as exc:  # pragma: no cover - environment-dependent
            raise SystemExit(
                "Python GDAL bindings are required. Install GDAL with Python support first."
            ) from exc
        module.UseExceptions()
        gdal = module
    return gdal


REPO_ROOT = Path(__file__).resolve().parents[1]
WORLD_DEFINITION_FORMAT_VERSION = 6
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
"""
DEFAULT_ENV_CELL_M = 40.0
DEFAULT_ZONE_CELL_M = 10.0
DEFAULT_TERRAIN_CHUNK_M = 512.0
DEFAULT_BASE_ELEVATION_M = 0.0


def resolve_repo_path(value: str) -> Path:
    path = Path(value)
    if path.is_absolute():
        return path
    return REPO_ROOT / path


def display_path(path: Path) -> str:
    try:
        return path.relative_to(REPO_ROOT).as_posix()
    except ValueError:
        return path.as_posix()


def validate_positive(value: float, label: str) -> None:
    if not math.isfinite(value) or value <= 0.0:
        raise ValueError(f"{label} must be finite and > 0")


def discover_tifs(input_dir: Path) -> list[Path]:
    tifs = sorted(input_dir.rglob("*.tif"))
    if not tifs:
        raise ValueError(f"no .tif files found under {input_dir}")
    return tifs


def source_metadata(paths: list[Path]) -> tuple[float, float, float, float, float, float]:
    require_gdal()
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


def source_epsg(path: Path) -> int:
    require_gdal()
    return int(gdal.Open(str(path)).GetSpatialRef().GetAuthorityCode(None))


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
    require_gdal()
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


def world_frame(
    size_m: float, centre_e: float, centre_n: float, terrain_cell_m: float
) -> tuple[float, float, float, float]:
    """Source-coordinate sample extent `(min_e, min_n, max_e, max_n)` of a square world of side
    `size_m` centred on `(centre_e, centre_n)`; the side must be a whole number of cells."""
    validate_positive(size_m, "size_m")
    validate_positive(terrain_cell_m, "terrain_cell_m")
    cells = size_m / terrain_cell_m
    if abs(cells - round(cells)) > 1e-9:
        raise ValueError(f"size_m {size_m} is not a whole number of {terrain_cell_m} m cells")
    half = size_m * 0.5
    return centre_e - half, centre_n - half, centre_e + half, centre_n + half


# ETRS89 / TM35FIN is published as EPSG:3067 (the elevation sheets) and as EPSG:10690, its
# EUREF-FIN realisation (the hydrography); the National Land Survey publishes both products in
# the same EUREF-FIN coordinates, so no transform is applied between them.
TM35FIN_EPSG_CODES = frozenset({3067, 10690})


def same_crs(a: int, b: int) -> bool:
    return a == b or (a in TM35FIN_EPSG_CODES and b in TM35FIN_EPSG_CODES)


def read_interior_point(value) -> tuple[float, float] | None:
    # `sijainti_piste` is a GeoJSON point string in the layer's own coordinates.
    if not value:
        return None
    coordinates = json.loads(value)["coordinates"]
    return float(coordinates[0]), float(coordinates[1])


def load_hydrography(path: Path, epsg: int):
    """Lakes (`jarvi`, level from `keskikorkeus` in mm), rivers (`virtavesialue`, which carry no
    level attribute: the level is the lowest vertex height, the outlet) and shoals (`matalikko`)."""
    import shapely
    from osgeo import ogr

    from dem_water import WaterBody

    ogr.UseExceptions()
    source = ogr.Open(str(path))
    if source is None:
        raise RuntimeError(f"OGR failed to open {path}")

    def features(layer_name):
        layer = source.GetLayerByName(layer_name)
        if layer is None:
            raise ValueError(f"{path} has no layer {layer_name}")
        srs = layer.GetSpatialRef()
        code = srs.GetAuthorityCode(None) if srs is not None else None
        if code is None or not same_crs(int(code), epsg):
            raise ValueError(f"{path} layer {layer_name} is EPSG:{code}, the DEM EPSG:{epsg}")
        rows = []
        for feature in layer:
            geometry = shapely.from_wkb(bytes(feature.GetGeometryRef().ExportToIsoWkb()))
            rows.append((feature, geometry))
        return rows

    bodies = []
    for feature, geometry in features("jarvi"):
        level = feature.GetField("keskikorkeus")
        if level is None:
            raise ValueError(f"lake {feature.GetField('mtk_id')} has no keskikorkeus")
        bodies.append(
            WaterBody(
                key=int(feature.GetField("mtk_id")),
                kind="lake",
                level_mm=int(level),
                geometry=shapely.force_2d(geometry),
                interior_en=read_interior_point(feature.GetField("sijainti_piste")),
            )
        )
    for feature, geometry in features("virtavesialue"):
        z = shapely.get_coordinates(geometry, include_z=True)[:, 2]
        bodies.append(
            WaterBody(
                key=int(feature.GetField("mtk_id")),
                kind="river",
                level_mm=int(round(float(z.min()) * 1000.0)),
                geometry=shapely.force_2d(geometry),
                interior_en=read_interior_point(feature.GetField("sijainti_piste")),
            )
        )
    keyed = sorted((int(f.GetField("mtk_id")), shapely.force_2d(g)) for f, g in features("matalikko"))
    shoals = [geometry for _, geometry in keyed]
    return bodies, shoals


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
    lake_fills: list[tuple[float, float, float]] = (),
    open_water_fills: list[tuple[float, float, float]] = (),
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
        conn.executemany(
            "INSERT INTO world_lake_fills(world_x, world_z, surface_elevation_m) VALUES (?, ?, ?)",
            lake_fills,
        )
        conn.executemany(
            "INSERT INTO world_open_water_fills(world_x, world_z, surface_elevation_m) VALUES (?, ?, ?)",
            open_water_fills,
        )
        conn.commit()
    finally:
        conn.close()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Import DEM GeoTIFF tiles into a WorldDefinition SQLite asset"
    )
    # No defaults name a world: the shipped Kuopio's sources (the 900 km2 elevation sheets and the
    # hydrography) are not in the repository; docs/terrain.md section 6 has its command.
    parser.add_argument(
        "--input-dir",
        required=True,
        help="Directory containing source GeoTIFF tiles (repo-relative unless absolute)",
    )
    parser.add_argument(
        "--output",
        required=True,
        help="Output WorldDefinition path (repo-relative unless absolute)",
    )
    parser.add_argument(
        "--world-name",
        required=True,
        help="User-facing world name to store in the WorldDefinition",
    )
    parser.add_argument(
        "--terrain-cell-m",
        type=float,
        default=10.0,
        help="Authored terrain sample spacing in metres (must be >= source pixel size)",
    )
    parser.add_argument(
        "--size-m",
        type=float,
        help="Side of a square world cut around --centre-e/--centre-n (default: the source extent)",
    )
    parser.add_argument("--centre-e", type=float, help="World centre easting in source coordinates")
    parser.add_argument("--centre-n", type=float, help="World centre northing in source coordinates")
    parser.add_argument(
        "--hydrography",
        help="Hydrography GeoPackage with layers jarvi, virtavesialue and matalikko",
    )
    parser.add_argument("--lake-slope", type=float, default=0.15, help="Bed slope from the shore")
    parser.add_argument("--lake-depth-cap-m", type=float, default=12.0, help="Deepest synthetic bed")
    parser.add_argument("--shoal-depth-cap-m", type=float, default=1.0, help="Deepest bed in a shoal")
    parser.add_argument(
        "--shore-margin-m",
        type=float,
        default=0.05,
        help="Height above its water that a shore or sill sample is raised to at least",
    )
    parser.add_argument(
        "--report-dir",
        help="Directory for water_report.json and water_labels.npz (per-sample polygon and group)",
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

    if args.size_m is not None:
        if args.centre_e is None or args.centre_n is None:
            raise SystemExit("--size-m needs --centre-e and --centre-n")
        frame = world_frame(args.size_m, args.centre_e, args.centre_n, args.terrain_cell_m)
        if frame[0] < min_e or frame[1] < min_n or frame[2] > max_e or frame[3] > max_n:
            raise SystemExit(
                f"the requested world {frame} is not inside the source extent "
                f"{(min_e, min_n, max_e, max_n)}"
            )
        min_e, min_n, max_e, max_n = frame
    elif args.centre_e is not None or args.centre_n is not None:
        raise SystemExit("--centre-e/--centre-n need --size-m")

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

    lake_fills = []
    open_water_fills = []
    if args.hydrography:
        from dem_water import SampleGrid, WaterRule, apply_hydrography

        rule = WaterRule(
            slope=args.lake_slope,
            depth_cap_m=args.lake_depth_cap_m,
            shoal_depth_cap_m=args.shoal_depth_cap_m,
            shore_margin_m=args.shore_margin_m,
        )
        grid = SampleGrid(
            west_e=min_e,
            north_n=max_n,
            cell_m=args.terrain_cell_m,
            width=elevation_m.shape[1],
            height=elevation_m.shape[0],
        )
        bodies, shoals = load_hydrography(resolve_repo_path(args.hydrography), source_epsg(source_paths[0]))
        water = apply_hydrography(elevation_m, grid, bodies, shoals, rule)
        for fill in water.fills:
            row = (fill.world_x, fill.world_z, fill.surface_elevation_m)
            (lake_fills if fill.kind == "lake" else open_water_fills).append(row)
        if args.report_dir:
            report_dir = resolve_repo_path(args.report_dir)
            report_dir.mkdir(parents=True, exist_ok=True)
            report = dict(water.report)
            report["fills"] = [fill.__dict__ for fill in water.fills]
            (report_dir / "water_report.json").write_text(
                json.dumps(report, indent=1, sort_keys=True) + "\n"
            )
            np.savez_compressed(
                report_dir / "water_labels.npz",
                body_of_sample=water.body_of_sample,
                group_of_sample=water.group_of_sample,
                dem_m=elevation_m,
                heights_m=water.heights,
            )
        elevation_m = water.heights
        print(
            f"water: {water.report['bodies_in_world']} polygons, "
            f"{water.report['bodies_dropped_under_min_samples']} dropped, "
            f"{len(lake_fills)} lake fills, {len(open_water_fills)} open-water fills, "
            f"{water.report['water_samples']} water samples"
        )

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
        source_heights_runtime=elevation_m,
        lake_fills=lake_fills,
        open_water_fills=open_water_fills,
    )

    width_samples = int(round(width_m / args.terrain_cell_m)) + 1
    height_samples = int(round(height_m / args.terrain_cell_m)) + 1
    print(
        f"imported {len(source_paths)} DEM tile(s) from {display_path(input_dir)} "
        f"into {display_path(output_path)} "
        f"({width_m/1000.0:.1f} km x {height_m/1000.0:.1f} km, "
        f"{width_samples} x {height_samples} samples @ {args.terrain_cell_m:.1f} m)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
