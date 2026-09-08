#!/usr/bin/env python3
"""Import logged straight-road locations and audit diagnostic terrain replays.

Geometry dumps describe staged graph output, not mouse input. Import only the
unambiguous degree changes of two-point strokes; preserve rollbacks separately.
The SQLite reference is opened read-only and never used as a desired Y profile.
"""

import argparse
from collections import Counter
import hashlib
import json
import math
from pathlib import Path
import re
import sqlite3
import struct


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_FIXTURE = ROOT / "benchmarks/fixtures/kuopio-terrain"
DEFAULT_WORLD = ROOT / "godot/bootstrap/worlds/kuopio_324km2_10m.sqlite"


def sha256(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def read_attempts(path):
    attempts, current, block = [], None, None
    with Path(path).open() as stream:
        for number, line in enumerate(stream, 1):
            if line.startswith("[DEBUG:road] commit_segment_detail "):
                if current is not None:
                    attempts.append(current)
                fields = dict(re.findall(r"(\w+)=([^ ]+)", line.strip()))
                if fields.get("points") != "2" or fields.get("committed") != "true":
                    raise ValueError(f"line {number}: only dispatched two-point strokes supported")
                current = {"line": number, "validation": fields, "accepted": True}
            elif "ROAD_GEOMETRY_DUMP_BEGIN" in line:
                if current is None or block is not None or "dump" in current:
                    raise ValueError(f"line {number}: unpaired geometry dump")
                block = []
            elif line.strip() == "ROAD_GEOMETRY_DUMP_END":
                if block is None:
                    raise ValueError(f"line {number}: unmatched dump end")
                current["dump"] = json.loads("".join(block))
                block = None
            elif line.startswith("[DEBUG:road] road_commit_rejected "):
                if current is None:
                    raise ValueError("rejection without a placement")
                current["accepted"] = False
                current["rejection"] = line.strip()
            elif block is not None and not line.startswith("[DEBUG:"):
                block.append(line)
    if block is not None:
        raise ValueError("truncated geometry dump")
    if current is not None:
        attempts.append(current)
    if not attempts or any("dump" not in attempt for attempt in attempts):
        raise ValueError("missing placement geometry; cannot silently skip attempts")
    return attempts


def degrees(edges):
    return Counter(node for edge in edges.values() for node in (edge["start_node"], edge["end_node"]))


def recover_stroke(before, dump):
    after = {**before, **{edge["edge_idx"]: edge for edge in dump["edges"]}}
    previous_degree, next_degree = degrees(before), degrees(after)
    # A two-ended stroke changes parity only at its endpoints. Splitting an old
    # edge preserves old-node degrees; a new crossing has even degree four.
    endpoints = sorted(node for node, degree in next_degree.items()
                       if (degree - previous_degree[node]) % 2)
    if len(endpoints) != 2:
        raise ValueError(f"ambiguous stroke endpoints: {endpoints}")
    new_edges = [edge for key, edge in sorted(after.items()) if key not in before]
    if not new_edges:
        raise ValueError("stroke has no new edge")
    first = new_edges[0]
    if first["start_node"] == endpoints[1]:
        endpoints.reverse()
    elif first["start_node"] != endpoints[0]:
        raise ValueError("cannot recover stroke direction")
    positions = {}
    for edge in dump["edges"]:
        points = edge["physical_geometry_world_precise"]
        positions[edge["start_node"]] = points[0]
        positions[edge["end_node"]] = points[-1]
    return {
        "start_xz": [positions[endpoints[0]][0], positions[endpoints[0]][2]],
        "end_xz": [positions[endpoints[1]][0], positions[endpoints[1]][2]],
        "fwd_lanes": first["fwd_lanes"], "bkw_lanes": first["bkw_lanes"],
    }, after


def import_manifest(log, reference, world):
    log, reference, world = (Path(path).resolve() for path in (log, reference, world))
    attempts = read_attempts(log)
    before, cases, group_edges = {}, [], set()
    for index, attempt in enumerate(attempts, 1):
        dump = attempt["dump"]
        stroke, staged = recover_stroke(before, dump)
        touched_old = set(dump["requested_edge_ids"]) & before.keys()
        if not touched_old and not (degrees(before).keys() & degrees({e["edge_idx"]: e for e in dump["edges"]}).keys()):
            cases.append({"case_id": f"kuopio_{len(cases) + 1:02d}", "segments": []})
            group_edges = set()
        if not cases or not touched_old <= group_edges:
            raise ValueError(f"attempt {index}: cross-site edit needs explicit grouping")
        candidate_group = group_edges | (staged.keys() - before.keys())
        local = {key: staged[key] for key in candidate_group}
        stroke.update(
            attempt=index, log_line=attempt["line"], original_accepted=attempt["accepted"],
            logged_validation=attempt["validation"],
            expected_live_edges=len(local),
        )
        if attempt["accepted"]:
            cases[-1]["segments"].append(stroke)
            before, group_edges = staged, candidate_group
        else:
            # Rejected staged output is evidence, not prior state for the next edit.
            rejected = {"case_id": f"{cases[-1]['case_id']}_attempt_{index:02d}",
                        "segments": [*cases[-1]["segments"], stroke],
                        "logged_rejection": attempt["rejection"]}
            cases.insert(len(cases) - 1, rejected)
    with sqlite3.connect(reference.resolve().as_uri() + "?mode=ro", uri=True) as db:
        saved_edges = db.execute("SELECT count(*) FROM network_edges").fetchone()[0]
        source = db.execute("SELECT height_blob_f32_le FROM terrain_state").fetchone()[0]
        config = db.execute("SELECT * FROM world_config").fetchone()
        saved_geometry = {}
        for edge_id, x, y, z in db.execute("SELECT edge_id,x,y,z FROM network_edge_geometry WHERE physical=1 ORDER BY edge_id,point_index"):
            saved_geometry.setdefault(edge_id, []).append((round(x, 6), round(y, 6), round(z, 6)))
    if saved_edges != len(before):
        raise ValueError(f"log/reference mismatch: {len(before)} accepted edges vs {saved_edges} saved")
    logged_geometry = sorted(tuple(tuple(point) for point in edge["physical_geometry_world_precise"]) for edge in before.values())
    if logged_geometry != sorted(tuple(points) for points in saved_geometry.values()):
        raise ValueError("final accepted log geometry does not match the reference save")
    verify_source_terrain(world, config, source)
    return {
        "schema_version": 1,
        "input_contract": "recovered post-snap XZ endpoints of logged two-point strokes; source/visible Y resampled; not exact mouse replay",
        "log_sha256": sha256(log), "reference_save_sha256": sha256(reference),
        "source_world_sha256": sha256(world), "source_terrain_sha256": hashlib.sha256(source).hexdigest(),
        "reference_save": str(reference.relative_to(ROOT)), "source_world": str(world.relative_to(ROOT)),
        "world_config": list(config), "attempt_count": len(attempts),
        "accepted_attempt_count": sum(attempt["accepted"] for attempt in attempts),
        "reference_live_edges": saved_edges,
        "quality_limits": {"max_grade": 1.0, "max_abs_source_delta_m": 5.0, "max_pitch_change_deg": 30.0},
        "cases": cases,
    }


def verify_source_terrain(world, config, source):
    """Check every source sample, not merely the map's human-readable name."""
    width, height = (math.floor(config[axis] / config[2] + 0.5) + 1 for axis in (0, 1))
    chunk_size = math.ceil(config[3] / config[2])
    dense = bytearray(struct.pack("<f", config[4] / 20.0) * (width * height))
    with sqlite3.connect(world.resolve().as_uri() + "?mode=ro", uri=True) as db:
        world_config = db.execute("SELECT width_m,height_m,terrain_cell_m,terrain_chunk_m,terrain_base_elevation_m,env_cell_m,zone_cell_m FROM world_definition_meta").fetchone()
        if tuple(config) != world_config:
            raise ValueError("reference/source world configuration mismatch")
        for cx, cz, w, h, blob in db.execute("SELECT * FROM world_terrain_chunks"):
            for row in range(h):
                offset = ((cz * chunk_size + row) * width + cx * chunk_size) * 4
                dense[offset:offset + w * 4] = blob[row * w * 4:(row + 1) * w * 4]
    if dense != source:
        raise ValueError("reference/source terrain samples differ")


def profile_metrics(points):
    if len(points) < 2 or any(len(p) != 4 or any(not isinstance(v, (float, int)) or not math.isfinite(v) for v in p) for p in points):
        raise ValueError("profile must contain finite [x,y,z,source_y] samples")
    result = {"max_grade": 0.0, "max_abs_source_delta_m": 0.0, "max_pitch_change_deg": 0.0,
              "max_fill_m": 0.0, "max_cut_m": 0.0, "max_source_grade": 0.0}
    last_pitch = None
    for index, point in enumerate(points):
        delta = point[1] - point[3]
        result["max_fill_m"] = max(result["max_fill_m"], delta)
        result["max_cut_m"] = max(result["max_cut_m"], -delta)
        if abs(delta) > result["max_abs_source_delta_m"]:
            result["max_abs_source_delta_m"] = abs(delta)
            result["worst_delta_at"] = point
        if index == 0:
            continue
        previous = points[index - 1]
        run = math.hypot(point[0] - previous[0], point[2] - previous[2])
        dy = point[1] - previous[1]
        if run == 0:
            if dy != 0:
                raise ValueError(f"vertical road span at {point}")
            continue
        grade = abs(dy) / run
        if grade > result["max_grade"]:
            result["max_grade"] = grade
            result["worst_grade_span"] = [previous, point]
        result["max_source_grade"] = max(result["max_source_grade"], abs(point[3] - previous[3]) / run)
        pitch = math.degrees(math.atan2(dy, run))
        if last_pitch is not None:
            result["max_pitch_change_deg"] = max(result["max_pitch_change_deg"], abs(pitch - last_pitch))
        last_pitch = pitch
    return result


def audit_snapshot(snapshot, limits):
    rows = []
    if not isinstance(snapshot, dict) or not isinstance(snapshot.get("edges"), list) or not snapshot["edges"]:
        raise ValueError("missing/nonexistent committed profile snapshot")
    for edge in snapshot["edges"]:
        row = {"edge_id": edge["edge_id"], **profile_metrics(edge["points"])}
        row["failures"] = [key for key, limit in limits.items() if row[key] > limit]
        rows.append(row)
    return rows


def report_capture(data):
    if data.get("benchmark") != "road_terrain_replay" or data.get("schema_version") != 1:
        raise ValueError("not a road terrain replay capture")
    failures, rows = [], []
    if not data.get("completed"):
        failures.append("pipeline incomplete")
    if data.get("capture_error") or data.get("write_error"):
        failures.append("artifact capture failed")
    expected = data.get("expected_case_ids", [])
    actual = [case["case_id"] for case in data.get("cases", [])]
    if not expected or len(set(actual)) != len(actual) or actual != expected:
        failures.append("missing, duplicate or unexpected cases")
    reference = data.get("reference", {})
    if not reference.get("wait", {}).get("ok"):
        failures.append("reference rendering did not settle")
    limits = data.get("quality_limits", {})
    if not limits:
        failures.append("missing quality limits")
    try:
        reference_rows = audit_snapshot(reference.get("snapshot"), limits)
    except ValueError as error:
        failures.append(f"reference: {error}")
        reference_rows = []
    for row in reference_rows:
        row.update(case_id="reference_save", attempt=0)
    for case in data.get("cases", []):
        if not case.get("ok") or len(case.get("steps", [])) != case.get("expected_steps"):
            failures.append(f"{case['case_id']}: placement/render failure")
        for step in case.get("steps", []):
            if not step.get("ok"):
                failures.append(f"{case['case_id']} attempt {step['attempt']}: failed operation/topology")
            if "snapshot" not in step:
                failures.append(f"{case['case_id']}: missing committed geometry")
                continue
            try:
                audited = audit_snapshot(step["snapshot"], limits)
            except ValueError as error:
                failures.append(f"{case['case_id']} attempt {step['attempt']}: {error}")
                continue
            for row in audited:
                row.update(case_id=case["case_id"], attempt=step["attempt"])
                rows.append(row)
                if row["failures"]:
                    failures.append(f"{case['case_id']} attempt {step['attempt']} edge {row['edge_id']}: {', '.join(row['failures'])}")
    return {"success": not failures, "limits": limits, "failures": failures, "profiles": rows,
            "reference_profiles": reference_rows,
            "coverage": "longitudinal physical profiles only; screenshots require visual review; no watertightness certification"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    action = parser.add_mutually_exclusive_group(required=True)
    action.add_argument("--import-log", type=Path)
    action.add_argument("--report", type=Path)
    parser.add_argument("--map", type=Path, default=DEFAULT_FIXTURE / "kuopio-terrain-map.sqlite")
    parser.add_argument("--world", type=Path, default=DEFAULT_WORLD)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("output already exists; choose a new path")
    result = (import_manifest(args.import_log, args.map, args.world) if args.import_log
              else report_capture(json.loads(args.report.read_text())))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x") as stream:
        json.dump(result, stream, indent=2, allow_nan=False)
        stream.write("\n")
    if args.import_log:
        print(f"Imported {result['attempt_count']} attempts into {len(result['cases'])} cases")
    else:
        print(f"Terrain audit: {len(result['profiles'])} profiles, {len(result['failures'])} failures")
        for failure in result["failures"][:20]:
            print(failure)
        return 0 if result["success"] else 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
