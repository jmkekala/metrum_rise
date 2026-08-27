extends RefCounted
class_name SpikeRecord

## Records a spike run to JSON so a later run can be compared against it.
##
## A spike prints numbers and they scroll away. Two weeks later nobody can say
## whether 69 cars through a junction was the number last time, so a regression
## that halves throughput reads as normal. This writes each run to
## `user://spike_runs/<name>.json` and diffs it against the previous one.
##
## The record is the baseline for the gym layer: the spikes already know how to
## build the scenario and measure it, and this is what lets a run be checked
## against what the system used to do rather than against memory.
##
## Usage inside a spike:
##
##     var rec := SpikeRecord.new("left_turn")
##     rec.check("cars cleared the junction", entered > 0)
##     rec.measure("overlapping_pairs", pairs)
##     rec.finish()   # writes the file, prints the comparison, returns exit code

const DIR := "user://spike_runs"

var spike_name: String
var checks: Array = []
var metrics: Dictionary = {}
var failures := 0

func _init(name: String) -> void:
	spike_name = name

## Records a pass/fail assertion and prints it.
func check(label: String, ok: bool) -> void:
	checks.append({"label": label, "ok": ok})
	if ok:
		print("  ok    %s" % label)
	else:
		failures += 1
		print("  FAIL  %s" % label)

## Records a number worth comparing between runs.
##
## Metrics are not pass/fail. A junction passing 69 cars is neither right nor
## wrong on its own; it is only meaningful against what it did last time.
func measure(key: String, value) -> void:
	metrics[key] = value

func _path() -> String:
	return "%s/%s.json" % [DIR, spike_name]

func _load_previous() -> Dictionary:
	if not FileAccess.file_exists(_path()):
		return {}
	var f := FileAccess.open(_path(), FileAccess.READ)
	if f == null:
		return {}
	var parsed = JSON.parse_string(f.get_as_text())
	f.close()
	return parsed if parsed is Dictionary else {}

## Writes the run, prints how it compares to the previous one, returns the exit
## code the spike should quit with.
func finish() -> int:
	var previous := _load_previous()

	var run := {
		"spike": spike_name,
		"checks_total": checks.size(),
		"checks_failed": failures,
		"checks": checks,
		"metrics": metrics,
	}

	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(DIR))
	var f := FileAccess.open(_path(), FileAccess.WRITE)
	if f != null:
		f.store_string(JSON.stringify(run, "  "))
		f.close()
		print("\n  recorded to %s" % ProjectSettings.globalize_path(_path()))
	else:
		print("\n  could not write %s" % _path())

	if previous.is_empty():
		print("  no previous run to compare against; this one is the baseline")
	else:
		_print_comparison(previous)

	if failures == 0:
		print("\nPASS: %d checks, 0 failures" % checks.size())
		return 0
	print("\nFAIL: %d checks, %d failures" % [checks.size(), failures])
	return 1

## Prints every metric that moved, and any check whose result flipped.
func _print_comparison(previous: Dictionary) -> void:
	print("  compared against the previous run:")

	var old_metrics: Dictionary = previous.get("metrics", {})
	var moved := 0
	for key in metrics:
		if not old_metrics.has(key):
			print("    %s: %s (new)" % [key, str(metrics[key])])
			moved += 1
			continue
		var was = old_metrics[key]
		var now = metrics[key]
		if str(was) != str(now):
			print("    %s: %s -> %s" % [key, str(was), str(now)])
			moved += 1
	if moved == 0:
		print("    every metric identical")

	# A check that used to pass and now fails is the interesting case, and it is
	# worth calling out separately from the raw fail count.
	var old_by_label := {}
	for c in previous.get("checks", []):
		if c is Dictionary and c.has("label"):
			old_by_label[c["label"]] = c.get("ok", false)
	for c in checks:
		var label: String = c["label"]
		if old_by_label.has(label) and old_by_label[label] and not c["ok"]:
			print("    REGRESSION: '%s' passed last run and fails now" % label)
