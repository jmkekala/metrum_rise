# SPDX-License-Identifier: GPL-2.0-only

## Headless native vegetation edit, persistence, tool routing and preview contract.
## Uses explicit failures because headless GDScript assertions do not terminate a test.
extends SceneTree

const VegetationTool := preload("res://scripts/tools/vegetation_tool.gd")
const InputManager := preload("res://scripts/core/input_manager.gd")
var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

# One ctrl-held wheel notch, which is the tool's radius control.
func _wheel(button: int) -> InputEventMouseButton:
	var event := InputEventMouseButton.new()
	event.button_index = button
	event.pressed = true
	event.ctrl_pressed = true
	return event

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	var host := Node3D.new()
	root.add_child(host)
	var simulation := SimulationNode.new()
	simulation.name = "SimulationNode"
	host.add_child(simulation)
	simulation.set_simulation_speed(0.0)
	_expect(simulation.create_blank_world(1020.0, 1020.0, 10.0, 510.0, 20.0), "fixture world must be created")
	var layout: Dictionary = simulation.get_terrain_patch_layout()
	var span: float = float(layout["terrain_cell_m"]) * float(layout["patch_interval_cells"])
	_expect(span == 510.0, "native patch layout must match the fixture")
	var left := Vector2(-510.0, 0.0)
	var right := Vector2(0.0, 0.0)
	var before_left := simulation.get_decorative_tree_patch(left, span, true)
	var before_right := simulation.get_decorative_tree_patch(right, span, true)
	# Captured before any edit, and canopy-only, so every record is known to be a generated
	# candidate of a known layer. Reading a position back out of an edited patch cannot tell the
	# two apart: an authored plant is packed into the same payload, in the same cell order.
	var generated := simulation.get_decorative_tree_patch(right, span, false)
	var target := Vector2(0.0, 17.0)
	_expect(simulation.add_vegetation_at(target, 3), "clear ground must accept a plant")
	_expect(simulation.get_vegetation_patch_generation(Vector2i(1, 1)) == 1, "only the containing render patch advances")
	_expect(simulation.get_vegetation_patch_generation(Vector2i(0, 1)) == 0, "the adjacent patch stays unchanged")
	_expect(simulation.get_decorative_tree_patch(left, span, true) == before_left, "boundary addition must not leak into the left patch")
	var authored := simulation.get_decorative_tree_patch(right, span, true)
	_expect(authored.size() == before_right.size() + 6, "boundary addition must appear exactly once")
	_expect(not simulation.add_vegetation_at(Vector2(510.0, 0.0), 0), "world edge must refuse placement")
	_expect(not simulation.add_vegetation_at(target, 4), "unknown species must be refused")
	_expect(simulation.paint_vegetation(target, 32.0, -1) == 0, "brush must refuse unknown species")
	var path := OS.get_temp_dir().path_join("metrum_vegetation_edit_%d.sqlite" % OS.get_process_id())
	_expect(simulation.save_game(path), "authored vegetation must save")
	_expect(simulation.remove_vegetation_at(target, 0.01) == 1, "authored plant removal must report one change")
	_expect(simulation.remove_vegetation_at(target, 0.01) == 0, "repeated removal must report no change")
	_expect(simulation.load_game(path), "authored vegetation must load")
	_expect(simulation.get_decorative_tree_patch(right, span, true) == authored, "load must reproduce every packed plant bit")
	_expect(generated.size() >= 12, "fixture must contain generated canopy")
	if generated.size() >= 12:
		var first := Vector2(generated[0], generated[2])
		_expect(simulation.remove_vegetation_at(first, 0.01) == 1, "a generated plant must be addressable")
		var second := Vector2(generated[6], generated[8])
		_expect(simulation.remove_vegetation_at(second, 0.01) == 1, "canopy removal must tombstone exactly one tree")
		_expect(simulation.paint_vegetation(second, 0.01, 0) == 1, "repaint must restore that generated tree")
		# This tiny disc covers only the generator candidate: its standing tree must not
		# acquire a duplicate when repainted.
		_expect(simulation.paint_vegetation(second, 0.01, 0) == 0, "a standing tree must not be repainted")
	_expect(simulation.paint_vegetation(Vector2(-80.0, -80.0), 64.0, 1) > 750, "brush must fill its dense lattice across existing forest")
	_expect(simulation.paint_vegetation(Vector2(-80.0, -80.0), 64.0, 1) == 0, "repeated brush must not stack plants")

	var tool := VegetationTool.new()
	tool.name = "VegetationTool"
	host.add_child(tool)
	tool.set_process(false)
	var manager := InputManager.new()
	# Exercise the same activation dispatcher without constructing unrelated gameplay tools.
	manager.vegetation_tool = tool
	manager._activate_tool_logic(InputManager.Tool.VEGETATION, true)
	_expect(tool.active, "input routing must activate the vegetation tool")
	# The radius chooses the point mutator or the brush inside one plant mode, so both dispatches
	# are exercised without a third mode to select them.
	tool.mode = VegetationTool.Mode.PLANT
	tool.species = 2
	tool.radius = VegetationTool.MIN_RADIUS_M
	_expect(tool.apply_at(Vector2(11.0, 13.0)) == 1, "the minimum radius must call the native point mutator")
	tool.mode = VegetationTool.Mode.REMOVE
	tool.radius = 0.01
	_expect(tool.apply_at(Vector2(11.0, 13.0)) == 1, "remove mode must forward its radius")
	tool.mode = VegetationTool.Mode.PLANT
	tool.radius = 32.0
	_expect(tool.apply_at(Vector2(96.0, -96.0)) > 0, "a raised radius must call the native planting brush")
	# Ctrl and the wheel are the only radius control, and stepping back down has to land on the
	# minimum exactly, because that value is what selects the single-plant dispatch above.
	tool.radius = VegetationTool.MIN_RADIUS_M
	tool._unhandled_input(_wheel(MOUSE_BUTTON_WHEEL_UP))
	_expect(tool.radius > VegetationTool.MIN_RADIUS_M, "ctrl and wheel up must widen the brush")
	for _i in range(8):
		tool._unhandled_input(_wheel(MOUSE_BUTTON_WHEEL_DOWN))
	_expect(tool.radius == VegetationTool.MIN_RADIUS_M, "wheel down must clamp back to the point radius")
	tool.radius = 1024.0
	_expect(tool.radius == 256.0, "plant radius must match the native stamp bound")
	tool.mode = VegetationTool.Mode.REMOVE
	tool.radius = 1024.0
	_expect(tool.radius == 1024.0, "removal keeps its existing radius bound")
	tool.mode = VegetationTool.Mode.PLANT
	_expect(tool.radius == 256.0, "switching to planting must clamp the preview radius")
	_expect(simulation.paint_vegetation(Vector2.ZERO, 256.01, 0) == 0, "native brush must reject oversized stamps")
	manager._activate_tool_logic(InputManager.Tool.VEGETATION, false)
	tool._process(0.0)
	_expect(not tool.active and not tool.preview.visible, "deactivation must hide the ground preview")
	_expect(tool.preview.mesh is TorusMesh, "tool must provide a ground-ring preview")
	manager.free()
	var scene := load("res://scenes/Main.tscn") as PackedScene
	_expect(scene != null, "main scene must load with the vegetation tool")
	if scene != null:
		var state := scene.get_state()
		var found := false
		for i in range(state.get_node_count()):
			if state.get_node_name(i) == &"VegetationTool":
				found = state.get_node_type(i) == &"Node3D"
		_expect(found, "main scene must own a VegetationTool Node3D")
	host.free()
	DirAccess.remove_absolute(path)
	if _failures == 0:
		print("vegetation_edit_test: PASS")
	quit(1 if _failures > 0 else 0)
