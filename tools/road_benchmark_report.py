#!/usr/bin/env python3
"""Validate schema-4 road captures or compare matched, unprofiled process runs.

Uses raw per-operation samples, not pooled fixture summaries. Each paired process
contributes one median per operation; repetitions inside a process are not treated
as independent runs. Negative deltas mean faster, not statistical significance.
"""

import argparse
import json
import math
from pathlib import Path
from statistics import median


def validate_capture(data):
    if data.get("benchmark") == "road_preview_latency":
        return validate_preview_capture(data)
    if data.get("schema_version") != 4 or data.get("success") is not True:
        raise ValueError("capture must be successful schema 4")
    definitions = {case["case_id"]: case for case in data["matrix_cases"]}
    expected = {
        (case, warmup, repetition)
        for case in definitions
        for warmup, count in [(False, data["repetitions"]), (True, data["warmup_repetitions"])]
        for repetition in range(count)
    }
    seen = set()
    products = {}
    for fixture in data["fixtures"]:
        key = fixture["case_id"], fixture["warmup"], fixture["repetition"]
        if key not in expected or key in seen or fixture.get("ok") is not True:
            raise ValueError(f"duplicate, unexpected, or failed fixture: {key}")
        seen.add(key)
        if len(fixture["segments"]) != len(definitions[key[0]]["segments"]):
            raise ValueError(f"incomplete segment list: {key}")
        for index, segment in enumerate(fixture["segments"]):
            if segment.get("ok") is not True:
                raise ValueError(f"failed segment: {key}")
            expected_rejection = definitions[key[0]]["segments"][index].get("expected_preview_invalid_reason")
            if segment.get("expected_preview_rejection"):
                if not expected_rejection or segment.get("invalid_reason") != expected_rejection or segment.get("committed") is not False:
                    raise ValueError(f"unexpected rejection: {key}")
                continue
            if expected_rejection:
                raise ValueError(f"missing expected rejection: {key}")
            generation = segment["generation_before"] + 1
            command = segment["state_after"]["command"]
            if not (
                generation == segment["generation_after"] == segment["generation_expected"]
                == command["generation"] and command["committed"]
            ):
                raise ValueError(f"stale or incomplete command metrics: {key}")
            if data["runtime"]["fixture_isolation"] == "reset_each_fixture":
                operation = key[0], index, fixture["anchor_x"], fixture["anchor_z"]
                product = tuple(segment["state_after"][name] for name in ("live_edges", "edge_slots", "nodes", "lanes"))
                if products.setdefault(operation, product) != product:
                    raise ValueError(f"non-repeatable road output: {key}")
            times = [segment[name] for name in (
                "commit_dispatch_ms", "generation_ready_ms", "render_ack_ms",
                "first_idle_ms", "commit_ms",
            )]
            if any(not math.isfinite(value) or value < 0 for value in times) or times != sorted(times):
                raise ValueError(f"invalid commit milestone order: {key}: {times}")
            if segment.get("preview_mode") != "immediate" and not 0 <= segment["preview_ready_ms"] <= segment["preview_ms"]:
                raise ValueError(f"invalid preview milestone order: {key}")
            if not 0 <= segment["settle_tail_ms"] <= segment["commit_ms"]:
                raise ValueError(f"invalid settlement fence: {key}")
    if seen != expected or not expected:
        raise ValueError("missing fixtures or empty workload")
    return data


def validate_preview_capture(data):
    """Reject incomplete modes, stale correlations and invalid timing boundaries."""
    schema = data.get("schema_version")
    if schema not in (1, 2) or data.get("success") is not True:
        raise ValueError("preview capture must be successful schema 1 or 2")
    # Older captures ran both modes and do not record them.
    expected = {(name, mode) for name in data["expected_cases"] for mode in data.get("modes", (0, 1))}
    seen = set()
    for case in data["cases"]:
        key = case["case_id"], case["mode"]
        if key not in expected or key in seen or not case["ok"] or case["dropped"]:
            raise ValueError(f"missing, duplicate, failed or truncated preview case: {key}")
        seen.add(key)
        if len(case["stationary"]) != data["samples"] or not case["moving"]:
            raise ValueError(f"incomplete preview samples: {key}")
        target_ms = case["scheduled_input_count"] * 1000 / case["scheduled_input_hz"]
        if not target_ms <= case["moving_interval_ms"] <= target_ms + case["frame_ms"]["max"] + 1:
            raise ValueError(f"moving input cadence drift: {key}")
        if schema == 2:
            validate_preview_lifecycle(case, key)
        ids = set()
        for row in case["stationary"] + case["moving"] + [case["final"]]:
            if row["request_id"] in ids:
                raise ValueError(f"duplicate displayed request: {key}")
            ids.add(row["request_id"])
            times = [row[field] for field in ("input_us", "dispatch_us", "received_us", "installed_us", "frame_us")]
            if times != sorted(times) or any(not math.isfinite(t) or t < 0 for t in times):
                raise ValueError(f"invalid preview milestone order: {key}")
            if row["full_terrain"] != (case["mode"] == 1):
                raise ValueError(f"wrong preview mode: {key}")
            boundary = "headless_process_frame" if data["mode"] == "headless" else "frame_post_draw"
            if row["frame_boundary"] != boundary:
                raise ValueError(f"wrong render boundary: {key}")
            if abs(row["input_to_frame_ms"] - (row["frame_us"] - row["input_us"]) / 1000) > 0.001:
                raise ValueError(f"inconsistent latency: {key}")
            native = row["native"]
            diagnostic = data.get("runtime", {}).get("diagnostic_environment", {}).get("METRUM_DEBUG_PERF") == "1"
            if diagnostic and not native:
                raise ValueError(f"missing diagnostic worker stages: {key}")
            if native:
                fields = ("context_ms", "road_ms", "earthworks_ms", "core_wait_ms", "capture_ms", "terrain_ms", "retained_ms")
                if abs(sum(native[field] for field in fields) - native["worker_ms"]) > 0.001:
                    raise ValueError(f"unaccounted worker stages: {key}")
                if any(not math.isfinite(v) or v < 0 for v in native.values()):
                    raise ValueError(f"invalid native timing: {key}")
        if not case["final"]["exact_at_frame"] or any(not row["exact_at_frame"] for row in case["stationary"]):
            raise ValueError(f"stationary/final pose did not converge: {key}")
        if key == ("residency_retry", 1) and any(row["terrain_retries"] < 3 for row in case["stationary"]):
            raise ValueError("residency workload did not exercise retries")
        for field in ("generation", "live_edges", "edge_slots", "nodes", "lanes", "agents", "buildings"):
            if case["state_before"][field] != case["state_after"][field]:
                raise ValueError(f"preview mutated authoritative state: {key}: {field}")
    if not expected or expected != seen:
        raise ValueError("incomplete preview mode matrix")
    return data


def validate_preview_lifecycle(case, key):
    """Validate exhaustive scheduled/sampled pose accounting for schema-2 captures."""
    lifecycle = case.get("scheduled_input_lifecycle", [])
    summary = case.get("sampled_input_deadline", {})
    if len(lifecycle) != case["scheduled_input_count"] or summary.get("scheduled_count") != len(lifecycle):
        raise ValueError(f"incomplete scheduled input lifecycle: {key}")
    if [row.get("schedule_id") for row in lifecycle] != list(range(1, len(lifecycle) + 1)):
        raise ValueError(f"non-contiguous scheduled input IDs: {key}")
    allowed = {
        "coalesced_before_sampling", "sampled_but_superseded", "stale_displayed",
        "exact_on_time", "late_exact",
    }
    sampled = []
    sampled_ids = []
    disposition_counts = {}
    for row in lifecycle:
        disposition = row.get("disposition")
        if disposition not in allowed:
            raise ValueError(f"unresolved input disposition: {key}: {disposition}")
        disposition_counts[disposition] = disposition_counts.get(disposition, 0) + 1
        scheduled_us, emitted_us = row.get("scheduled_us"), row.get("emitted_us")
        if not all(isinstance(value, (int, float)) and math.isfinite(value) for value in (scheduled_us, emitted_us)):
            raise ValueError(f"invalid scheduled input time: {key}")
        if emitted_us < scheduled_us:
            raise ValueError(f"input emitted before its deadline: {key}")
        if disposition == "coalesced_before_sampling":
            if "sampled_us" in row or "strict_16_7_met" in row:
                raise ValueError(f"coalesced input was counted as sampled: {key}")
            continue
        sampled.append(row)
        sampled_us = row.get("sampled_us")
        if not isinstance(sampled_us, (int, float)) or not math.isfinite(sampled_us) or sampled_us < emitted_us:
            raise ValueError(f"pose sampled before it was emitted: {key}")
        sampled_input_id = row.get("sampled_input_id")
        if type(sampled_input_id) is not int or sampled_input_id <= 0:
            raise ValueError(f"invalid sampled input ID: {key}")
        sampled_ids.append(sampled_input_id)
        met = row.get("strict_16_7_met")
        if not isinstance(met, bool) or met != (disposition == "exact_on_time"):
            raise ValueError(f"incorrect strict deadline classification: {key}")
        resolution = row.get("sample_to_resolution_ms")
        if not isinstance(resolution, (int, float)) or not math.isfinite(resolution) or resolution < 0:
            raise ValueError(f"invalid sampled-pose resolution time: {key}")
        if disposition in ("exact_on_time", "late_exact"):
            latency = row.get("sample_to_frame_ms")
            if not isinstance(latency, (int, float)) or not math.isfinite(latency) or latency < 0:
                raise ValueError(f"invalid exact frame latency: {key}")
            frame_us = row.get("frame_us")
            if not isinstance(frame_us, (int, float)) or not math.isfinite(frame_us):
                raise ValueError(f"invalid exact frame timestamp: {key}")
            if abs(latency - (frame_us - sampled_us) / 1000) > 0.001:
                raise ValueError(f"inconsistent exact frame latency: {key}")
            if (latency < 16.7) != met:
                raise ValueError(f"exact frame deadline mismatch: {key}")
        elif disposition == "stale_displayed":
            latency = row.get("sample_to_stale_frame_ms")
            if not isinstance(latency, (int, float)) or not math.isfinite(latency) or latency < 0:
                raise ValueError(f"invalid stale frame latency: {key}")
            if met:
                raise ValueError(f"stale display passed exact deadline: {key}")
            frame_us = row.get("first_stale_frame_us")
            if not isinstance(frame_us, (int, float)) or not math.isfinite(frame_us):
                raise ValueError(f"invalid stale frame timestamp: {key}")
            if abs(latency - (frame_us - sampled_us) / 1000) > 0.001:
                raise ValueError(f"inconsistent stale frame latency: {key}")
    if sampled_ids != sorted(sampled_ids) or len(sampled_ids) != len(set(sampled_ids)):
        raise ValueError(f"sampled input IDs must be unique and ordered: {key}")

    observed = {}
    for row in case["moving"]:
        observed.setdefault(row.get("input_id"), []).append(row)
    for row in sampled:
        disposition = row["disposition"]
        if disposition in ("exact_on_time", "late_exact"):
            matches = [candidate for candidate in observed.get(row["sampled_input_id"], [])
                       if candidate.get("request_id") == row.get("exact_request_id")
                       and candidate.get("input_us") == row.get("sampled_us")
                       and candidate.get("frame_us") == row.get("frame_us")
                       and candidate.get("exact_at_frame") is True]
            if len(matches) != 1:
                raise ValueError(f"exact lifecycle has no matching observed result: {key}")
        elif disposition == "stale_displayed":
            matches = [candidate for candidate in observed.get(row["sampled_input_id"], [])
                       if candidate.get("request_id") == row.get("first_stale_request_id")
                       and candidate.get("input_us") == row.get("sampled_us")
                       and candidate.get("frame_us") == row.get("first_stale_frame_us")
                       and candidate.get("exact_at_frame") is False]
            if len(matches) != 1:
                raise ValueError(f"stale lifecycle has no matching observed result: {key}")
    expected = {
        "scheduled_count": len(lifecycle),
        "sampled_count": len(sampled),
        "exact_frame_count": sum(row["disposition"] in ("exact_on_time", "late_exact") for row in sampled),
        "met_count": sum(row["disposition"] == "exact_on_time" for row in sampled),
        "miss_count": sum(row["disposition"] != "exact_on_time" for row in sampled),
        "disposition_counts": disposition_counts,
    }
    for field, value in expected.items():
        if summary.get(field) != value:
            raise ValueError(f"incorrect sampled deadline summary {field}: {key}")
    if summary.get("budget_ms") != 16.7 or "every sampled pose" not in summary.get("contract", ""):
        raise ValueError(f"wrong sampled deadline contract: {key}")
    frames = case.get("frame_wall_ms")
    if not isinstance(frames, list) or len(frames) != case["frame_ms"]["count"]:
        raise ValueError(f"missing raw wall-time frames: {key}")
    if any(not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0 for value in frames):
        raise ValueError(f"invalid raw wall-time frame: {key}")
    if frames and abs(max(frames) - case["frame_ms"]["max"]) > 0.001:
        raise ValueError(f"raw frame maximum mismatch: {key}")
    if case.get("frames_over_16_7ms") != sum(value > 16.7 for value in frames):
        raise ValueError(f"raw frame deadline count mismatch: {key}")


def load_capture(path):
    return validate_capture(json.loads(Path(path).read_text()))


def observations(data, metric):
    result = {}
    for fixture in data["fixtures"]:
        if fixture["warmup"]:
            continue
        for index, segment in enumerate(fixture["segments"]):
            if segment.get("expected_preview_rejection"):
                continue
            source = segment
            field = metric
            if metric.startswith("command."):
                source = segment["state_after"]["command"]
                field = metric.removeprefix("command.")
            if field not in source:
                continue  # e.g. an immediate click has no completed-preview latency
            value = source[field]
            if not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0:
                raise ValueError(f"invalid {metric}: {value}")
            result.setdefault((fixture["case_id"], index), []).append(value)
    return {key: median(values) for key, values in result.items()}


def workload_signature(data):
    runtime = data["runtime"]
    runtime_fields = (
        "godot", "cpu", "logical_cpus", "video_adapter", "max_fps", "vsync",
        "rayon_num_threads_env", "world_sha256", "world_contract", "fixture_isolation",
        "simulation_speed", "input_contract", "harness_sha256", "viewport_size", "configured_renderer",
        "metrics_helper_sha256", "diagnostic_environment",
    )
    cardinalities = ("live_edges", "edge_slots", "nodes", "lanes", "agents", "buildings", "rayon_threads")
    fixtures = []
    for fixture in data["fixtures"]:
        state = fixture["state_before"]
        fixtures.append((
            fixture["case_id"], fixture["warmup"], fixture["repetition"],
            fixture["anchor_x"], fixture["anchor_z"],
            tuple(state[name] for name in cardinalities),
            [(
                segment["start"], segment["end"],
                tuple(segment["state_after"][name] for name in cardinalities) if "state_after" in segment else None,
            ) for segment in fixture["segments"]],
        ))
    return (
        data["mode"], data["matrix_cases"],
        {field: runtime[field] for field in runtime_fields}, fixtures,
    )


def compare_pairs(baselines, candidates, metric):
    if not baselines or len(baselines) != len(candidates):
        raise ValueError("provide equal, nonempty baseline/candidate process-run lists in paired order")
    reference = workload_signature(baselines[0])
    changes = {}
    absolute = {}
    for baseline, candidate in zip(baselines, candidates):
        for capture in (baseline, candidate):
            validate_capture(capture)
            if capture["runtime"]["profiled"]:
                raise ValueError("profiled captures are diagnostic, not headline A/B latency measurements")
            if workload_signature(capture) != reference:
                raise ValueError("incompatible workload, anchors, state, harness, hardware, or cadence")
        before, after = observations(baseline, metric), observations(candidate, metric)
        if before.keys() != after.keys():
            raise ValueError("mismatched operations")
        for key in before:
            if before[key] <= 0:
                raise ValueError(f"zero baseline for {key}; choose a measurable metric")
            changes.setdefault(key, []).append((after[key] / before[key] - 1) * 100)
            absolute.setdefault(key, []).append((before[key], after[key]))
    if not changes:
        raise ValueError(f"no measured operations contain metric {metric}")
    rows = []
    for key, deltas in sorted(changes.items()):
        pairs = absolute[key]
        rows.append({
            "case": key[0], "segment": key[1], "process_pairs": len(deltas),
            "baseline_ms": median(pair[0] for pair in pairs),
            "candidate_ms": median(pair[1] for pair in pairs),
            "paired_delta_percent_median": median(deltas),
            "paired_delta_percent_range": [min(deltas), max(deltas)],
        })
    warnings = ["Measure unchanged-build A/A noise before accepting a speedup; command stages can also be noisy."]
    if len(baselines) < 3:
        warnings.append("Fewer than three process pairs: exploratory comparison, not regression evidence.")
    return {"metric": metric, "rows": rows, "warnings": warnings,
            "interpretation": "descriptive paired process medians; negative is faster; no significance claim"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--validate", metavar="CAPTURE")
    parser.add_argument("--baseline", nargs="+")
    parser.add_argument("--candidate", nargs="+")
    parser.add_argument("--metric", default="first_idle_ms")
    args = parser.parse_args()
    try:
        if args.validate:
            capture = load_capture(args.validate)
            print(f"Validated {len(capture.get('fixtures', capture.get('cases', [])))} complete road fixtures")
        else:
            if not args.baseline or not args.candidate:
                parser.error("use --validate, or both --baseline and --candidate")
            paths = [Path(path).resolve() for path in args.baseline + args.candidate]
            if len(set(paths)) != len(paths):
                parser.error("each capture must come from a distinct process run; do not reuse files as replicates")
            report = compare_pairs(
                [load_capture(path) for path in args.baseline],
                [load_capture(path) for path in args.candidate], args.metric,
            )
            print(json.dumps(report, indent=2))
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.exit(1, f"Invalid road benchmark: {error}\n")


if __name__ == "__main__":
    main()
