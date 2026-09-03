# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: rust_gateway.gd
#  script_path: scripts/core/rust_gateway.gd
#  module_name: rust_gateway
#  version: 0.1.0
#  author: [BantedHam]
#  description: The Rust core's one voice on the GOAT bus. Outbound, the
#           core's per-tick event list publishes as bus events under the
#           rust/ prefix. Inbound, watched events collect into a batch
#           the core drains as next-tick input. One choke point, one
#           marshalling site, one tick of lag; no Rust bus counterpart,
#           because duplicating queue and replay semantics in a second
#           language is drift with a bridge in the middle.
#  kind: module
#  spec: none
#  internal_dependencies: [addons/GOAT_bus]
#  external_dependencies: [Godot 4.x]
#  features: [gateway, batched-events, one-tick-lag]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## Everything the Rust core says or hears goes through this node.
@tool
extends Node

## The prefix every core-originated event carries on the bus.
const OUT_PREFIX := "rust/"

## The bus, injected or found. Injection is what lets a spike hand in a
## bus it built itself, with no autoload in sight.
var bus: Node = null
var _inbound: Array[Dictionary] = []


func _ready() -> void:
	if bus == null:
		bus = get_node_or_null("/root/GoatBusSystem")


## Publish the core's tick output: an array of {"name": String,
## "data": Dictionary}. Returns how many were accepted by the bus.
func flush_outbound(events: Array) -> int:
	if bus == null:
		return 0
	var sent := 0
	for e in events:
		var ev := e as Dictionary
		if ev == null or not ev.has("name"):
			continue
		var ok = bus.publish_event(OUT_PREFIX + str(ev["name"]),
			ev.get("data", {}) as Dictionary)
		if ok == null or ok:
			sent += 1
	return sent


## Watch bus events the core wants to hear. Each arrival lands in the
## inbound batch with its name, never dispatched into the core mid-tick.
func watch(event_names: Array) -> void:
	if bus == null:
		return
	for event_name in event_names:
		var name_str := str(event_name)
		bus.subscribe_to_event(name_str, _on_watched.bind(name_str))


func _on_watched(data, event_name: String) -> void:
	_inbound.append({"name": event_name,
		"data": data if data is Dictionary else {"value": data}})


## Hand the collected batch to the core and start the next one: the
## one-tick lag, applied to discrete facts.
func drain_inbound() -> Array[Dictionary]:
	var batch := _inbound
	_inbound = []
	return batch
