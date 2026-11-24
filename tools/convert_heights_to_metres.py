#!/usr/bin/env python3
"""
One-off conversion of the Kuopio buildability fixture's terrain samples from the legacy height
unit (20 m) to metres.

`benchmarks/fixtures/kuopio-terrain/residential-buildability.json` holds raw source samples that
its tests write straight into the heightmap, which takes metres since save format 131 and world
format 6. Saves and worlds of older formats are converted by their loaders and need no tool.
The conversion is exact: every sample becomes `f32(raw) * 20` rounded once to `f32`, the product
an `f32` multiply gives (a 24-bit mantissa times 20 is exact in a double, so one rounding to `f32`
is the correctly rounded `f32` product). The file is rewritten in place.

    tools/convert_heights_to_metres.py PATH
"""

from __future__ import annotations

import json
import struct
import sys
from pathlib import Path


def f32(value: float) -> float:
    return struct.unpack("<f", struct.pack("<f", value))[0]


def to_metres(raw: float) -> float:
    return f32(f32(raw) * 20.0)


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    path = Path(sys.argv[1])
    fixture = json.loads(path.read_text())
    fixture["samples"] = [[to_metres(value) for value in row] for row in fixture["samples"]]
    path.write_text(json.dumps(fixture, separators=(",", ":")))
    print(f"{path}: {len(fixture['samples'])} sample rows")
    return 0


if __name__ == "__main__":
    sys.exit(main())
