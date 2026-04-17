#!/usr/bin/env python3
"""
Build chunked terrain assets from a scanned GeoTIFF world import.

This is the first importer slice for the terrain pipeline:

- reads `mosaic_plan.toml` and the linked `import_manifest.toml`
- extracts one or more canonical terrain chunks
- mosaics overlapping GeoTIFF tiles with GDAL
- writes one internal chunk-asset directory per exported chunk

Output chunk asset layout:

    chunks_512m/
      cx_0011_cz_0070/
        chunk.toml
        height_lod0_2m.f32
        height_lod1_4m.f32
        height_lod2_8m.f32
        height_lod3_32m.f32

The raw height files are little-endian row-major float32 rasters. Chunk metadata
in `chunk.toml` describes dimensions, bounds, nodata, and source provenance.
"""

from __future__ import annotations

import argparse
import math
import sys
import tomllib
from datetime import datetime
from pathlib import Path

import numpy as np

try:
    from osgeo import gdal
except ImportError as exc:  # pragma: no cover - environment-dependent
    raise SystemExit(
        "Python GDAL bindings are required. Install GDAL with Python support first."
    ) from exc


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_LODS = (
    ("lod0", 2.0),
    ("lod1", 4.0),
    ("lod2", 8.0),
    ("lod3", 32.0),
)


def resolve_repo_path(value: str) -> Path:
    path = Path(value)
    if path.is_absolute():
        return path
    return REPO_ROOT / path


def toml_str(value: str) -> str:
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'


def sample_dims(width_m: float, height_m: float, sample_m: float) -> tuple[int, int]:
    width = int(round(width_m / sample_m))
    height = int(round(height_m / sample_m))
    if width <= 0 or height <= 0:
        raise ValueError(
            f"invalid sample dimensions width={width} height={height} for {width_m}x{height_m} @ {sample_m}m"
        )
    return width, height


def intersect_aabb(
    a_min_x: float,
    a_min_y: float,
    a_max_x: float,
    a_max_y: float,
    b_min_x: float,
    b_min_y: float,
    b_max_x: float,
    b_max_y: float,
) -> bool:
    return not (
        a_max_x <= b_min_x
        or a_min_x >= b_max_x
        or a_max_y <= b_min_y
        or a_min_y >= b_max_y
    )


def chunk_bounds(plan: dict, chunk_x: int, chunk_z: int) -> tuple[float, float, float, float]:
    chunk_m = float(plan["terrain_chunk_m"])
    world_min_e = float(plan["world_min_e_m"])
    world_min_n = float(plan["world_min_n_m"])
    world_max_e = float(plan["world_max_e_m"])
    world_max_n = float(plan["world_max_n_m"])
    min_e = world_min_e + chunk_x * chunk_m
    min_n = world_min_n + chunk_z * chunk_m
    max_e = min(min_e + chunk_m, world_max_e)
    max_n = min(min_n + chunk_m, world_max_n)
    return min_e, min_n, max_e, max_n


def overlapping_tiles(tiles: list[dict], min_e: float, min_n: float, max_e: float, max_n: float) -> list[dict]:
    result: list[dict] = []
    for tile in tiles:
        if intersect_aabb(
            min_e,
            min_n,
            max_e,
            max_n,
            float(tile["min_e_m"]),
            float(tile["min_n_m"]),
            float(tile["max_e_m"]),
            float(tile["max_n_m"]),
        ):
            result.append(tile)
    return result


def load_plan(path: Path) -> dict:
    with path.open("rb") as f:
        return tomllib.load(f)


def load_manifest(path: Path) -> dict:
    with path.open("rb") as f:
        return tomllib.load(f)


def open_chunk_array(
    source_paths: list[str],
    min_e: float,
    min_n: float,
    max_e: float,
    max_n: float,
    width: int,
    height: int,
    nodata: float | None,
) -> np.ndarray:
    gdal.UseExceptions()
    warp_kwargs = dict(
        format="MEM",
        outputBounds=(min_e, min_n, max_e, max_n),
        width=width,
        height=height,
        resampleAlg="near",
        outputType=gdal.GDT_Float32,
    )
    if nodata is not None:
        warp_kwargs["srcNodata"] = nodata
        warp_kwargs["dstNodata"] = nodata
    ds = gdal.Warp("", source_paths, **warp_kwargs)
    if ds is None:
        raise RuntimeError("GDAL Warp returned no dataset")
    band = ds.GetRasterBand(1)
    array = band.ReadAsArray()
    if array is None:
        raise RuntimeError("GDAL failed to read warped chunk data")
    return np.asarray(array, dtype=np.float32)


def downsample_mean(array: np.ndarray, factor: int, nodata: float | None) -> np.ndarray:
    if factor == 1:
        return np.array(array, copy=True)
    height, width = array.shape
    if height % factor != 0 or width % factor != 0:
        raise ValueError(
            f"cannot downsample {width}x{height} by factor {factor}; dimensions are not divisible"
        )
    reshaped = array.reshape(height // factor, factor, width // factor, factor)
    if nodata is None:
        return reshaped.mean(axis=(1, 3), dtype=np.float32)

    valid = reshaped != nodata
    sums = np.where(valid, reshaped, 0.0).sum(axis=(1, 3), dtype=np.float64)
    counts = valid.sum(axis=(1, 3))
    out = np.full((height // factor, width // factor), nodata, dtype=np.float32)
    mask = counts > 0
    out[mask] = (sums[mask] / counts[mask]).astype(np.float32)
    return out


def array_min_max(array: np.ndarray, nodata: float | None) -> tuple[float | None, float | None]:
    if nodata is None:
        return float(array.min()), float(array.max())
    valid = array != nodata
    if not np.any(valid):
        return None, None
    return float(array[valid].min()), float(array[valid].max())


def write_f32(path: Path, array: np.ndarray) -> None:
    path.write_bytes(array.astype("<f4", copy=False).tobytes(order="C"))


def export_chunk(
    plan: dict,
    manifest: dict,
    chunk_x: int,
    chunk_z: int,
    output_root: Path,
    overwrite: bool,
    allow_nodata: bool,
) -> Path:
    min_e, min_n, max_e, max_n = chunk_bounds(plan, chunk_x, chunk_z)
    width_m = max_e - min_e
    height_m = max_n - min_n
    source_sample_m = float(plan["source_sample_m"])
    world_id = str(plan["world_id"])
    nodata = manifest.get("nodata_value")
    nodata = float(nodata) if isinstance(nodata, (int, float)) else None

    width_px, height_px = sample_dims(width_m, height_m, source_sample_m)
    tiles = overlapping_tiles(manifest["tiles"], min_e, min_n, max_e, max_n)
    if not tiles:
        raise RuntimeError(f"chunk ({chunk_x},{chunk_z}) does not overlap any source tiles")

    source_paths = [str(resolve_repo_path(tile["relative_path"])) for tile in tiles]
    base_array = open_chunk_array(source_paths, min_e, min_n, max_e, max_n, width_px, height_px, nodata)
    if base_array.shape != (height_px, width_px):
        raise RuntimeError(
            f"unexpected base array shape {base_array.shape}; expected {(height_px, width_px)}"
        )

    nodata_pixels = 0
    if nodata is not None:
        nodata_pixels = int(np.count_nonzero(base_array == nodata))
        if nodata_pixels > 0 and not allow_nodata:
            raise RuntimeError(
                f"chunk ({chunk_x},{chunk_z}) contains {nodata_pixels} nodata pixels; "
                "rerun with --allow-nodata if you want to export it as-is"
            )

    chunk_dir = output_root / f"cx_{chunk_x:04d}_cz_{chunk_z:04d}"
    if chunk_dir.exists():
        if not overwrite:
            raise RuntimeError(f"{chunk_dir} already exists; rerun with --overwrite to replace it")
        for path in chunk_dir.iterdir():
            if path.is_file():
                path.unlink()
            elif path.is_dir():
                raise RuntimeError(f"refusing to overwrite nested directory {path}")
    else:
        chunk_dir.mkdir(parents=True, exist_ok=True)

    lod_entries: list[dict] = []
    for lod_name, sample_m in DEFAULT_LODS:
        factor = int(round(sample_m / source_sample_m))
        if not math.isclose(source_sample_m * factor, sample_m, rel_tol=0.0, abs_tol=1e-6):
            raise RuntimeError(f"LOD sample {sample_m}m is not an integer multiple of source sample {source_sample_m}m")
        lod_array = downsample_mean(base_array, factor, nodata)
        lod_min, lod_max = array_min_max(lod_array, nodata)
        filename = f"height_{lod_name}_{int(sample_m)}m.f32"
        write_f32(chunk_dir / filename, lod_array)
        lod_entries.append(
            {
                "lod_name": lod_name,
                "sample_m": sample_m,
                "width_px": lod_array.shape[1],
                "height_px": lod_array.shape[0],
                "relative_path": filename,
                "min_height_m": lod_min,
                "max_height_m": lod_max,
            }
        )

    base_min, base_max = array_min_max(base_array, nodata)
    chunk_meta_path = chunk_dir / "chunk.toml"
    lines: list[str] = []
    lines.append("chunk_asset_version = 1")
    lines.append(f"world_id = {toml_str(world_id)}")
    lines.append(f"chunk_x = {chunk_x}")
    lines.append(f"chunk_z = {chunk_z}")
    lines.append(f"terrain_chunk_m = {float(plan['terrain_chunk_m']):.1f}")
    lines.append(f"source_sample_m = {source_sample_m:.1f}")
    lines.append(f"epsg = {int(manifest['epsg'])}")
    lines.append(f"crs_name = {toml_str(str(manifest['crs_name']))}")
    lines.append('raster_semantics = "pixel_is_area"')
    lines.append('sample_format = "float32_le"')
    lines.append(f"min_e_m = {min_e:.3f}")
    lines.append(f"min_n_m = {min_n:.3f}")
    lines.append(f"max_e_m = {max_e:.3f}")
    lines.append(f"max_n_m = {max_n:.3f}")
    lines.append(f"width_m = {width_m:.3f}")
    lines.append(f"height_m = {height_m:.3f}")
    lines.append(f"base_width_px = {width_px}")
    lines.append(f"base_height_px = {height_px}")
    if nodata is not None:
        lines.append(f"nodata_value = {nodata:.1f}")
    lines.append(f"nodata_pixel_count = {nodata_pixels}")
    if base_min is not None:
        lines.append(f"min_height_m = {base_min:.6f}")
    if base_max is not None:
        lines.append(f"max_height_m = {base_max:.6f}")
    lines.append(f"overlap_tile_count = {len(tiles)}")
    lines.append("")
    lines.append("source_tiles = [")
    for tile in tiles:
        lines.append(f"  {toml_str(str(tile['filename']))},")
    lines.append("]")
    lines.append("")
    for lod in lod_entries:
        lines.append("[[lods]]")
        lines.append(f"lod_name = {toml_str(lod['lod_name'])}")
        lines.append(f"sample_m = {lod['sample_m']:.1f}")
        lines.append(f"width_px = {lod['width_px']}")
        lines.append(f"height_px = {lod['height_px']}")
        lines.append(f"relative_path = {toml_str(lod['relative_path'])}")
        if lod["min_height_m"] is not None:
            lines.append(f"min_height_m = {lod['min_height_m']:.6f}")
        if lod["max_height_m"] is not None:
            lines.append(f"max_height_m = {lod['max_height_m']:.6f}")
        lines.append("")
    chunk_meta_path.write_text("\n".join(lines), encoding="utf-8")
    return chunk_dir


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Build internal terrain chunk assets from scanned GeoTIFF sources")
    parser.add_argument(
        "--plan",
        default="terrain/kuopio/import/mosaic_plan.toml",
        help="Path to the terrain mosaic plan TOML (repo-relative by default)",
    )
    parser.add_argument(
        "--chunk",
        nargs=2,
        type=int,
        action="append",
        metavar=("CHUNK_X", "CHUNK_Z"),
        help="Export one chunk by chunk-grid coordinate; may be repeated",
    )
    parser.add_argument(
        "--all",
        action="store_true",
        help="Export every chunk in the plan (can be large and slow)",
    )
    parser.add_argument(
        "--output-root",
        default=None,
        help="Output directory for generated chunk assets; defaults to a sibling chunks_512m dir beside the plan",
    )
    parser.add_argument(
        "--overwrite",
        action="store_true",
        help="Replace existing chunk output directories if they already exist",
    )
    parser.add_argument(
        "--allow-nodata",
        action="store_true",
        help="Allow chunk export even if the base raster contains nodata cells",
    )
    parser.add_argument(
        "--quiet",
        action="store_true",
        help="Suppress per-chunk path output; useful for --all batch exports",
    )
    parser.add_argument(
        "--progress-every",
        type=int,
        default=100,
        help="When --quiet is set, print one progress line every N exported chunks (default: 100)",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    plan_path = resolve_repo_path(args.plan)
    plan = load_plan(plan_path)
    manifest_path = resolve_repo_path(str(plan["source_manifest"]))
    manifest = load_manifest(manifest_path)

    output_root = (
        resolve_repo_path(args.output_root)
        if args.output_root
        else plan_path.parent / f"chunks_{int(float(plan['terrain_chunk_m']))}m"
    )
    output_root.mkdir(parents=True, exist_ok=True)

    chunk_cols = int(plan["chunk_grid_columns"])
    chunk_rows = int(plan["chunk_grid_rows"])
    requested_chunks: list[tuple[int, int]] = []
    if args.all:
        requested_chunks.extend((cx, cz) for cz in range(chunk_rows) for cx in range(chunk_cols))
    if args.chunk:
        requested_chunks.extend((cx, cz) for cx, cz in args.chunk)
    if not requested_chunks:
        raise SystemExit("specify at least one --chunk X Z or use --all")

    seen: set[tuple[int, int]] = set()
    deduped_chunks: list[tuple[int, int]] = []
    for chunk in requested_chunks:
        if chunk in seen:
            continue
        seen.add(chunk)
        deduped_chunks.append(chunk)

    for chunk_x, chunk_z in deduped_chunks:
        if chunk_x < 0 or chunk_x >= chunk_cols or chunk_z < 0 or chunk_z >= chunk_rows:
            raise SystemExit(
                f"chunk ({chunk_x},{chunk_z}) is out of bounds for {chunk_cols}x{chunk_rows} chunk grid"
            )

    total = len(deduped_chunks)
    started = datetime.now()
    for index, (chunk_x, chunk_z) in enumerate(deduped_chunks, start=1):
        chunk_dir = export_chunk(
            plan=plan,
            manifest=manifest,
            chunk_x=chunk_x,
            chunk_z=chunk_z,
            output_root=output_root,
            overwrite=args.overwrite,
            allow_nodata=args.allow_nodata,
        )
        if not args.quiet:
            print(chunk_dir.relative_to(REPO_ROOT).as_posix())
        elif index == 1 or index == total or (args.progress_every > 0 and index % args.progress_every == 0):
            elapsed = datetime.now() - started
            print(f"[{index}/{total}] exported chunk ({chunk_x},{chunk_z}) elapsed={elapsed}")

    elapsed = datetime.now() - started
    print(
        f"exported {total} terrain chunk(s) to {output_root.relative_to(REPO_ROOT).as_posix()} "
        f"in {elapsed}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
