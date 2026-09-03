# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: spike_stats.gd
#  script_path: scripts/core/spike_stats.gd
#  module_name: spike_stats
#  version: 0.1.0
#  author: [BantedHam]
#  description: The benchmark ledger every spike registers into: one JSON
#           entry per run with the verdict, the run's wall time from
#           process start, and the machine it ran on, so any box's
#           numbers can stand beside any other's. Appends to
#           benchmarks.json at the project root; the ledger is data, the
#           spikes stay the authority on pass and fail.
#  kind: module
#  spec: none
#  internal_dependencies: []
#  external_dependencies: [Godot 4.x]
#  features: [benchmark-ledger, machine-stats]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-09-01
# =========================================================================

## One JSON entry per spike run: verdict, wall time, machine.
extends RefCounted

# =========================================================================
# THE DECLARATIONS
# =========================================================================

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const LEDGER_PATH := "res://benchmarks.json"


# =========================================================================
# RECORD
# =========================================================================
# =========================================================================
# RECORD
# =========================================================================
## Append this run's entry. Wall time is process start to this call,
## which for a headless spike is the whole run, boot included.
static func record(spike: String, passed: int, failed: int,
		extra: Dictionary = {}) -> void:
	var entries := []
	if FileAccess.file_exists(LEDGER_PATH):
		var parsed = JSON.parse_string(FileAccess.get_file_as_string(LEDGER_PATH))
		if parsed is Array:
			entries = parsed
	var entry := {
		"spike": spike,
		"utc": Time.get_datetime_string_from_system(true),
		"passed": passed,
		"failed": failed,
		"ms": Time.get_ticks_msec(),
		"cpu": OS.get_processor_name(),
		"cores": OS.get_processor_count(),
		"ram_mb": int(OS.get_memory_info().get("physical", 0) / 1048576),
		"godot": Engine.get_version_info()["string"],
	}
	for k in extra:
		entry[k] = extra[k]
	entries.append(entry)
	var fh := FileAccess.open(LEDGER_PATH, FileAccess.WRITE)
	if fh == null:
		push_error("spike_stats: cannot open %s" % LEDGER_PATH)
		return
	fh.store_string(JSON.stringify(entries, "\t"))
	fh.close()
