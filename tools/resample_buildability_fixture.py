#!/usr/bin/env python3
"""
Regenerates the ground of the Kuopio hillside buildability fixture from the shipped world.

`benchmarks/fixtures/kuopio-terrain/residential-buildability.json` holds a window of terrain
samples that its tests write straight into the heightmap, beside roads, nodes and parcels that
are stored authority (as a save holds them) and are not touched here. This tool replaces only the
window: the shipped world's stored samples (metres, `f32`) over world x -9024..-8456 and
z 1952..3048, with the window's `width`, `height`, `cell_m`, `x0` and `z0`. On the 18,432 m, 8 m
Kuopio that is 72 x 138 samples from sample (24, 1396); it covers node 1 with a margin. Other keys
keep their values and order; the file is rewritten in place on one line, as before.

    tools/resample_buildability_fixture.py [--check]

`--check` writes nothing and exits 1 when the committed fixture differs from the regenerated one.
"""

from __future__ import annotations

import argparse
import json
import math
import sqlite3
import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FIXTURE = ROOT / "benchmarks/fixtures/kuopio-terrain/residential-buildability.json"
WORLD = ROOT / "godot/bootstrap/worlds/kuopio_18432m_8m.sqlite"
# The window in world metres: the hillside area of the recorded session, edges inclusive.
WINDOW_X_M = (-9024.0, -8456.0)
WINDOW_Z_M = (1952.0, 3048.0)


def world_samples(world: Path) -> tuple[list[float], int, float, float]:
    # Dense row-major samples from the world's chunk table, `base` where no chunk is stored.
    with sqlite3.connect(world.as_uri() + "?mode=ro", uri=True) as db:
        width_m, height_m, cell, chunk_m, base = db.execute(
            "SELECT width_m, height_m, terrain_cell_m, terrain_chunk_m, terrain_base_elevation_m "
            "FROM world_definition_meta"
        ).fetchone()
        width, height = (math.floor(v / cell + 0.5) + 1 for v in (width_m, height_m))
        chunk = math.ceil(chunk_m / cell)
        dense = [base] * (width * height)
        for cx, cz, w, h, blob in db.execute(
            "SELECT chunk_x, chunk_z, width_samples, height_samples, source_height_blob_f32_le "
            "FROM world_terrain_chunks"
        ):
            values = struct.unpack(f"<{w * h}f", blob)
            for row in range(h):
                offset = (cz * chunk + row) * width + cx * chunk
                dense[offset:offset + w] = values[row * w:(row + 1) * w]
    return dense, width, cell, width_m / 2


def regenerate(text: str) -> str:
    dense, width, cell, half = world_samples(WORLD)
    x0, x1 = (int((x + half) / cell) for x in WINDOW_X_M)
    z0, z1 = (int((z + half) / cell) for z in WINDOW_Z_M)
    fixture = json.loads(text)
    fixture.update(
        width=width,
        height=len(dense) // width,
        cell_m=cell,
        x0=x0,
        z0=z0,
        samples=[[dense[z * width + x] for x in range(x0, x1 + 1)] for z in range(z0, z1 + 1)],
    )
    out = json.dumps(fixture, separators=(",", ":"))
    return out + "\n" if text.endswith("\n") else out


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--check", action="store_true", help="compare only; write nothing")
    args = parser.parse_args()
    text = FIXTURE.read_text()
    regenerated = regenerate(text)
    if args.check:
        if regenerated != text:
            print(f"{FIXTURE.relative_to(ROOT)} differs from its regenerated ground", file=sys.stderr)
            return 1
        print(f"{FIXTURE.relative_to(ROOT)} matches its regenerated ground")
        return 0
    FIXTURE.write_text(regenerated)
    return 0


if __name__ == "__main__":
    sys.exit(main())
