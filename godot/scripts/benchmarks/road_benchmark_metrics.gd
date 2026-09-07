# SPDX-License-Identifier: GPL-2.0-only

## Benchmark-only statistics; never pool different road operations into one latency distribution.
extends RefCounted

const LATENCIES := [
	"preview_ready_ms", "preview_ms", "commit_dispatch_ms", "generation_ready_ms",
	"render_ack_ms", "ghost_ready_ms", "first_idle_ms", "settle_tail_ms", "commit_ms",
	"pointer_idle_to_ready_ms",
]

static func distribution(values: Array) -> Dictionary:
	if values.is_empty():
		return {"count": 0, "p50": null, "p95": null, "p99": null, "max": null}
	var ordered := values.duplicate()
	ordered.sort()
	var count := ordered.size()
	var median := float(ordered[count / 2])
	if count % 2 == 0:
		median = (float(ordered[count / 2 - 1]) + median) * 0.5
	var total := 0.0
	for value in ordered:
		total += float(value)
	# Require at least five upper-tail observations for p95 and ten for p99. These are
	# descriptive quantiles, not confidence intervals or independent process-level replicates.
	return {
		"count": count, "mean": total / count, "p50": median,
		"p95": ordered[int(ceil(count * 0.95)) - 1] if count >= 100 else null,
		"p99": ordered[int(ceil(count * 0.99)) - 1] if count >= 1000 else null,
		"min": ordered.front(), "max": ordered.back(),
	}

static func summarize(fixtures: Array) -> Dictionary:
	var grouped := {}
	for fixture in fixtures:
		if bool(fixture.get("warmup", false)) or not bool(fixture.get("ok", false)):
			continue
		var case_id: String = fixture["case_id"]
		if not grouped.has(case_id):
			grouped[case_id] = {"totals": [], "segments": {}}
		var group: Dictionary = grouped[case_id]
		group.totals.append(fixture["total_ms"])
		for index in range(fixture.segments.size()):
			var segment: Dictionary = fixture.segments[index]
			var role := "initial_road" if index == 0 else "edit_%d" % index
			if not group.segments.has(role):
				group.segments[role] = {}
			var samples: Dictionary = group.segments[role]
			for metric in LATENCIES:
				if segment.has(metric):
					if not samples.has(metric):
						samples[metric] = []
					samples[metric].append(segment[metric])
	var cases := {}
	for case_id in grouped:
		var group: Dictionary = grouped[case_id]
		var segments := {}
		for role in group.segments:
			segments[role] = {}
			for metric in group.segments[role]:
				segments[role][metric] = distribution(group.segments[role][metric])
		cases[case_id] = {
			"fixture_total_ms": distribution(group.totals), "segments": segments,
		}
	return {
		"cases": cases,
		"quantile_policy": "p95 requires 100 observations; p99 requires 1000; null means insufficient data",
		"replicate_unit": "fixture repetition within this process; use separate paired runs for speedup claims",
	}
