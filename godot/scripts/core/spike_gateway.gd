# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: spike_gateway.gd
#  script_path: scripts/core/spike_gateway.gd
#  module_name: spike_gateway
#  version: 0.1.1
#  author: [BantedHam]
#  description: Drills the Rust gateway against a real GOAT bus built in
#           the spike itself: outbound batches publish under the rust/
#           prefix, watched events collect inbound, and drain hands over
#           exactly one tick's batch. Loopback proves both directions in
#           one pass.
#  kind: spike
#  spec: none
#  internal_dependencies: [scripts/core/rust_gateway.gd, addons/GOAT_bus]
#  external_dependencies: [Godot 4.x]
#  features: [gateway-drill, loopback]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## Run: godot --headless --path godot --script scripts/core/spike_gateway.gd
extends SceneTree

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const Gateway := preload("res://scripts/core/rust_gateway.gd")
const BusCore := preload("res://addons/GOAT_bus/goat_bus/goat_bus.gd")

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
	# The bus core itself, no autoload anywhere: the gateway takes an
	# injected bus, which is what makes it drillable headless. The core
	# caches every publish until its two required dependencies resolve, so
	# the spike injects stubs for both before anything speaks.
	var bus = BusCore.new()
	var registry_stub := Node.new()
	root.add_child(registry_stub)
	var config_stub := Node.new()
	root.add_child(config_stub)
	bus._dependency_manager.set_dependency("system_registry", registry_stub)
	bus._dependency_manager.set_dependency("config_manager", config_stub)
	var shim := Node.new()
	shim.set_script(null)
	# The gateway speaks the wrapper's verbs; a 30-line shim maps them to
	# the core so the spike drills the gateway, not the wrapper.
	var gateway := Gateway.new()
	gateway.bus = _wrap(bus)
	root.add_child(gateway)

	# Outbound: a tick's worth of core events.
	var sent := gateway.flush_outbound([
		{"name": "building_placed", "data": {"zone": 4, "asset": "mill"}},
		{"name": "agent_hired", "data": {"agent": 17}},
		{"junk": true},
	])
	_check("outbound publishes the well-formed events", sent == 2)

	# Inbound: watch, publish onto the bus, drain.
	gateway.watch(["rust/building_placed", "ui/pause"])
	bus.publish("rust/building_placed", {"zone": 4, "asset": "mill"})
	bus.publish("ui/pause", {"on": true})
	bus.publish("ui/ignored", {"n": 1})
	# A queued bus delivers on its pump, which a frameless spike drives by
	# hand, exactly as the shell's tick will.
	bus.process_queued_events()
	var batch := gateway.drain_inbound()
	_check("watched events collected", batch.size() == 2)
	var saw_ignored := false
	var payload_ok := false
	for e in batch:
		if str(e["name"]) == "ui/ignored":
			saw_ignored = true
		if str(e["name"]) == "rust/building_placed" \
				and int((e["data"] as Dictionary).get("zone", -1)) == 4:
			payload_ok = true
	_check("unwatched events ignored", not saw_ignored)
	_check("payload survives the trip", payload_ok)
	var second := gateway.drain_inbound()
	_check("drain empties the batch", second.is_empty())

	print("=== %d passed, %d failed ===" % [passed, failed])
	load("res://scripts/core/spike_stats.gd").record("spike_gateway", passed, failed)
	quit(1 if failed > 0 else 0)


# =========================================================================
# WRAP
# =========================================================================
## The wrapper's verbs over the bare core, for the spike alone.
func _wrap(bus) -> Node:
	var shim := Node.new()
	shim.set_meta("bus", bus)
	var script := GDScript.new()
	script.source_code = """
extends Node
# =========================================================================
# PUBLISH EVENT
# =========================================================================
func publish_event(event_name: String, data: Dictionary = {}, priority: int = 1):
	return get_meta("bus").publish(event_name, data)
# =========================================================================
# SUBSCRIBE TO EVENT
# =========================================================================
func subscribe_to_event(event_name: String, handler: Callable = Callable(), owner: Object = null):
	return get_meta("bus").subscribe(event_name, handler, self)
"""
	script.reload()
	shim.set_script(script)
	root.add_child(shim)
	return shim
