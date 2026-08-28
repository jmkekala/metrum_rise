extends SceneTree

## Reproducer: the simulation thread is never shut down.
##
## `SimulationNode` spawns its sim thread in `_ready` and has no `exit_tree`.
## Nothing signals the thread to stop and nothing joins it, so on quit Godot
## tears the node down while the thread is still ticking.
##
## Run:
##   godot --headless --path godot --script spike_poll.gd
##
## Expected: prints its result, returns 0.
## Actual: prints its result, then SIGSEGV during teardown. Three runs of three.
##
## Two symptoms, one cause. The node counts read 0 where an identical build
## without per-frame polling reads 2 and then 5, so the render snapshot is being
## read while it is being replaced.
##
## Upstream code, unchanged by this contribution: `get_network_nodes` is
## byte-identical to 859afaba and no `exit_tree` or `join()` exists under
## `rust/src/nodes/`.

var sim = null

func _init() -> void:
	sim = ClassDB.instantiate("SimulationNode")
	sim.name = "SimulationNode"
	root.add_child(sim)
	_go()

## Polls the node list every frame, which is what a renderer does.
func _settle(max_frames: int = 1200) -> int:
	var stable := 0
	var last := -1
	for _i in range(max_frames):
		await process_frame
		var n: int = sim.get_network_nodes().size()
		if n == last:
			stable += 1
			if stable >= 30:
				return n
		else:
			stable = 0
			last = n
	return sim.get_network_nodes().size()

func _go() -> void:
	sim.add_road(PackedVector3Array([Vector3(-300, 0, 0), Vector3(300, 0, 0)]), 2, 2)
	var a: int = await _settle()
	print("after road 1: %d nodes (expected 2)" % a)

	sim.add_road(PackedVector3Array([Vector3(0, 0, -300), Vector3(0, 0, 300)]), 2, 2)
	var b: int = await _settle()
	print("after road 2: %d nodes (expected 5)" % b)

	print("DONE - if the process now segfaults, that is the bug")
	quit(0)
