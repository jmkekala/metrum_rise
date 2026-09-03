# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: spike_boundary.gd
#  script_path: scripts/core/spike_boundary.gd
#  module_name: spike_boundary
#  version: 0.1.1
#  author: [BantedHam]
#  description: Drills the simulation boundary both directions and records
#           the exchange as fixtures. Down: batched sampling is
#           deterministic and matches the kernel called directly. Up: a
#           deposit buffer round-trips through the engine's grid format
#           and reads back cell for cell. The fixtures are the contract
#           either side can drill against without the other running.
#  kind: spike
#  spec: none
#  internal_dependencies: [scripts/core/engine_boundary.gd]
#  external_dependencies: [Godot 4.x]
#  features: [boundary-drill, fixtures]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## Run: godot --headless --path godot --script scripts/core/spike_boundary.gd
extends SceneTree

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const Boundary := preload("res://scripts/core/engine_boundary.gd")
const Fbm := preload("res://addons/2.5D_engine/evaluator/fbm_node.gd")

const FIXTURE_DIR := "res://fixtures/boundary"

var passed := 0
var failed := 0

# =========================================================================
# CHECK
# =========================================================================
func _check(name: String, ok: bool) -> void:
	if ok:
		passed += 1
		print("  ok   %s" % name)
	else:
		failed += 1
		print("  FAIL %s" % name)

# =========================================================================
# INIT
# =========================================================================
func _init() -> void:
	DirAccess.make_dir_recursive_absolute(FIXTURE_DIR)

	# DOWN: batched sampling.
	var positions := PackedVector3Array([
		Vector3(0, 0, 0), Vector3(10.5, 0, -3.25), Vector3(-200, 4, 77),
		Vector3(1000, -2, 1000), Vector3(0.001, 0, 0.001),
		Vector3(-1, 0, 1), Vector3(512, 8, -512), Vector3(3.14, 1.5, -2.7),
	])
	var a := Boundary.sample_field(positions, 0.5, 0.0, 1337)
	var b := Boundary.sample_field(positions, 0.5, 0.0, 1337)
	_check("sampling returns one value per position", a.size() == positions.size())
	_check("sampling is deterministic", a == b)
	# Probed BEFORE any differently-seeded call: the first run of this spike
	# showed the kernel answers differently after a 1338-seeded batch, which
	# is order-sensitive static state, recorded below as its own check.
	var direct := Fbm.evaluate(positions[2].x, positions[2].y, positions[2].z,
		0.5, 0.0, 1337)
	_check("batch matches the kernel called directly", a[2] == direct)
	var seeded := Boundary.sample_field(positions, 0.5, 0.0, 1338)
	_check("seed changes the field", a != seeded)
	var direct_after := Fbm.evaluate(positions[2].x, positions[2].y, positions[2].z,
		0.5, 0.0, 1337)
	_check("kernel is order-independent across seeds", direct_after == direct)

	var fixture := {"positions": [], "footprint": 0.5, "t": 0.0,
		"seed": 1337, "samples": [], "sample_bits": []}
	for p in positions:
		(fixture["positions"] as Array).append([p.x, p.y, p.z])
	for v in a:
		(fixture["samples"] as Array).append(v)
		# JSON's decimal text loses last bits; the little-endian byte hex
		# is the golden the twin gate compares, decimal is for human eyes.
		(fixture["sample_bits"] as Array).append(
			PackedFloat64Array([v]).to_byte_array().hex_encode())
	var sf := FileAccess.open(FIXTURE_DIR + "/sample.json", FileAccess.WRITE)
	sf.store_string(JSON.stringify(fixture, "\t"))
	sf.close()
	_check("sampling fixture recorded",
		FileAccess.file_exists(FIXTURE_DIR + "/sample.json"))

	# UP: a deposit buffer through the engine's own grid format. Whole
	# metres, because the format is signed 16-bit metres.
	var w := 4
	var h := 3
	var grid := PackedFloat32Array([
		0, 12, -7, 100,
		32767, -32768, 5, 1,
		-1, 250, 4000, -4000,
	])
	var raw := FIXTURE_DIR + "/deposit.raw"
	_check("deposit writes", Boundary.write_deposit(grid, w, h,
		10.0, 50.0, 1.0 / 60.0, raw))
	var opened = Boundary.open_deposit(raw)
	_check("engine opens the deposit", opened != null and opened.ok)
	if opened != null and opened.ok:
		var back_ok := true
		for row in h:
			for col in w:
				# Cell centres, from the sidecar's own georeferencing.
				var lon := 10.0 + (float(col) + 0.5) * (1.0 / 60.0)
				var lat := 50.0 - (float(row) + 0.5) * (1.0 / 60.0)
				var v: float = opened.data_elevation_m(lat, lon)
				if absf(v - grid[row * w + col]) > 0.001:
					back_ok = false
		_check("every cell reads back exactly", back_ok)
	_check("deposit fixture recorded", FileAccess.file_exists(raw)
		and FileAccess.file_exists(raw.get_basename() + ".json"))

	print("=== %d passed, %d failed ===" % [passed, failed])
	load("res://scripts/core/spike_stats.gd").record("spike_boundary", passed, failed)
	quit(1 if failed > 0 else 0)
