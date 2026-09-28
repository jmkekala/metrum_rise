# SPDX-License-Identifier: GPL-2.0-only

"""Measure isolated cell-zoning heap phases with the installed glibc memusage tool.

Run after `cargo test --offline --release --manifest-path rust/Cargo.toml --lib --no-run`.
This reads memusage's native 24-byte records on little-endian 64-bit Linux. It measures
successful malloc/calloc/realloc/free events and requested live heap, excluding markers.
It does not measure RSS, GPU memory, direct mmap, aligned-allocation APIs or latency.
Format: https://sourceware.org/git/?p=glibc.git;a=blob;f=malloc/memusage.c
"""

import argparse
import datetime
import gzip
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import struct
import subprocess
import sys


ROOT = Path(__file__).resolve().parent.parent
PREFIX = "nodes::sim::core::tests::road_plan_scaling::memory::"
FIXTURES = ("populated_cell_memory", "large_cell_fill_memory")
ENTRY = struct.Struct("<QQII")
CALIBRATION = {
    "calibrate_empty": [],
    "calibrate_retained": [12345],
    "calibrate_resize": [257, 256, -385, -128],
}


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def read_phases(log):
    text = re.sub(r"\x1b\[[0-9;]*m", "", log.read_text())
    phases = [json.loads(line.split("CELL_HEAP_PHASE ", 1)[1])
              for line in text.splitlines() if "CELL_HEAP_PHASE " in line]
    if not phases or len({p["marker_bytes"] for p in phases}) != len(phases):
        raise ValueError("Missing or duplicate phase manifests")
    if "test result: ok. 1 passed" not in text:
        raise ValueError("Fixture did not finish successfully")
    for kind in ("malloc", "calloc", "realloc"):
        match = re.search(rf"^\s*{kind}\|\s+\d+\s+\d+\s+(\d+)", text, re.MULTILINE)
        if match is None or int(match[1]) != 0:
            raise ValueError(f"Missing allocator summary or failed {kind} calls")
    return phases


def parse_trace(path, phases):
    if path.stat().st_size < 2 * ENTRY.size or path.stat().st_size % ENTRY.size:
        raise ValueError("Truncated or incompatible memusage trace")
    markers = {p["marker_bytes"]: p for p in phases}
    rows = []
    seen = set()
    active = None
    previous = 0
    with path.open("rb") as stream:
        stream.read(ENTRY.size)  # Start timestamp / aggregate peak, not an event.
        peak = ENTRY.unpack(stream.read(ENTRY.size))[0]
        if peak >= 1 << 40:
            raise ValueError("Implausible heap peak; do not accept a wrapped counter")
        while block := stream.read(ENTRY.size * 65536):
            for heap, _stack, _low, _high in ENTRY.iter_unpack(block):
                if heap > peak:
                    raise ValueError("Event exceeds final peak; trace is inconsistent")
                delta = heap - previous
                if delta in markers:
                    if active is not None or delta in seen:
                        raise ValueError("Ambiguous phase boundary")
                    seen.add(delta)
                    active = dict(markers[delta], events=0, growth_bytes=0, released_bytes=0,
                                  peak_bytes=0, baseline=heap, calibration_deltas=[])
                elif active is not None and delta == -active["marker_bytes"]:
                    active["retained_bytes"] = previous - active.pop("baseline")
                    expected = CALIBRATION.get(active["name"])
                    observed = active.pop("calibration_deltas")
                    if expected is not None and observed != expected:
                        raise ValueError(f"Calibration {active['name']}: {observed} != {expected}")
                    active["within_budget"] = (
                        active["peak_bytes"] <= active["peak_budget"]
                        and (not active["zero_events"] or active["events"] == 0)
                    )
                    rows.append(active)
                    active = None
                elif active is not None:
                    active["events"] += 1
                    active["growth_bytes"] += max(0, delta)
                    active["released_bytes"] += max(0, -delta)
                    active["peak_bytes"] = max(active["peak_bytes"], heap - active["baseline"])
                    if active["name"] in CALIBRATION:
                        active["calibration_deltas"].append(delta)
                previous = heap
    if active is not None or seen != markers.keys() or len(rows) != len(phases):
        raise ValueError("Incomplete phase boundaries")
    if [p["marker_bytes"] for p in rows] != [p["marker_bytes"] for p in phases]:
        raise ValueError("Phase order does not match the fixture manifest")
    if {p["name"] for p in rows if p["name"] in CALIBRATION} != CALIBRATION.keys():
        raise ValueError("All three real allocator calibrations are required")
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path, help="Compiled release library test binary")
    parser.add_argument("--output", required=True, type=Path, help="New directory for raw traces and results")
    parser.add_argument("--runs", type=int, default=3)
    args = parser.parse_args()
    if sys.byteorder != "little" or struct.calcsize("P") != 8 or platform.system() != "Linux":
        parser.error("This trace reader requires little-endian 64-bit Linux")
    if args.runs < 1:
        parser.error("--runs must be positive")
    binary = args.binary.resolve(strict=True)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, RAYON_NUM_THREADS="1", METRUM_DEBUG="0")
    env.pop("MEMUSAGE_TRACE_MMAP", None)
    metadata = {
        "utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "binary": {"path": str(binary), "sha256": sha(binary)},
        "compiler": subprocess.check_output(["rustc", "--version"], text=True).strip(),
        "profiler": subprocess.check_output(["memusage", "--version"], text=True).splitlines()[0],
        "env": {"RAYON_NUM_THREADS": "1", "METRUM_DEBUG": "0"},
        "scope": "Successful normal libc heap events inside calibrated markers. Setup, markers, logging and product destruction after return excluded. No profiled latency, RSS, direct mmap, aligned allocation or GPU-memory claims.",
        "commands": [],
        "source_hashes": {},
    }
    sources = subprocess.check_output(
        ["git", "ls-files", "-co", "--exclude-standard", "--", "rust/src", "rust/Cargo.toml", "rust/Cargo.lock", "benchmarks/zoning_memory.py"],
        cwd=ROOT, text=True).splitlines()
    metadata["source_hashes"] = {p: sha(ROOT / p) for p in sorted(set(sources))}
    rows = []
    for run in range(1, args.runs + 1):
        for fixture in FIXTURES:
            stem = f"{fixture}-{run}"
            trace = output / (stem + ".data")
            log = output / (stem + ".log")
            command = ["taskset", "-c", "0", "memusage", "--no-timer", "--data=" + str(trace),
                       str(binary), "--exact", PREFIX + fixture, "--ignored", "--nocapture", "--test-threads=1"]
            metadata["commands"].append(command)
            (output / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
            with log.open("w") as stream:
                subprocess.run(command, cwd=ROOT, env=env, stdout=stream, stderr=subprocess.STDOUT, check=True)
            current = parse_trace(trace, read_phases(log))
            measured = {(p["name"], p["background"]) for p in current if p["name"] not in CALIBRATION}
            if fixture == "populated_cell_memory":
                names = ("idle_lots", "idle_chunk", "pick", "cell_preview", "marquee_preview",
                         "brush_preview", "fill_preview", "paint", "erase", "undo_erase", "undo_paint")
                expected = {(name, size) for name in names for size in (0, 1000, 10000, 100000)}
            else:
                expected = {(name, size) for name in ("painted_store", "large_fill")
                            for size in (768, 3072, 11400)}
            if measured != expected or len(current) != len(expected) + len(CALIBRATION):
                raise ValueError("Fixture did not execute every required operation and background")
            rows.extend(dict(row, fixture=fixture, run=run) for row in current)
            (output / "results.json").write_text(json.dumps(rows, indent=2) + "\n")
            # Retain the complete raw event stream, including excluded setup, without keeping
            # gigabytes of highly repetitive uncompressed fixture events in the artifact directory.
            with trace.open("rb") as source, gzip.open(str(trace) + ".gz", "wb", compresslevel=1) as compressed:
                shutil.copyfileobj(source, compressed)
            trace.unlink()
            print(f"{stem}: {len(current)} calibrated phases, {sum(not p['within_budget'] for p in current)} budget failures", flush=True)
    if sha(binary) != metadata["binary"]["sha256"] or any(
        sha(ROOT / p) != expected for p, expected in metadata["source_hashes"].items()
    ):
        raise ValueError("Source or binary changed during measurement")
    failures = [p for p in rows if not p["within_budget"]]
    for row in failures:
        print(json.dumps(row, sort_keys=True))
    return int(bool(failures))


if __name__ == "__main__":
    sys.exit(main())
