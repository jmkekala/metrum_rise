# SPDX-License-Identifier: GPL-2.0-only

## Live speed input validation and simulation-command responsiveness regression.
extends SceneTree

const InputManager := preload("res://scripts/core/input_manager.gd")
var _failures := 0

class SpeedDisplay extends Control:
	var speed := 0.0
	func set_sim_speed_display(value: float) -> void:
		speed = value

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	var simulation := SimulationNode.new()
	root.add_child(simulation)
	_expect(simulation.set_simulation_speed(1.0), "valid speed must be accepted")
	await create_timer(0.1).timeout
	_expect(float(simulation.get_road_benchmark_state()["simulation_speed"]) == 1.0, "valid speed must reach the simulation thread")
	for invalid_speed in [32.01, 1.0e30, INF, -INF, NAN]:
		print("Checking simulation speed input: ", invalid_speed)
		_expect(not simulation.set_simulation_speed(invalid_speed), "unsupported speed must be rejected")
		await create_timer(0.1).timeout
		# The diagnostic acquires the core lock: invalid speed must neither poison state nor stall a tick.
		var state: Dictionary = simulation.get_road_benchmark_state()
		_expect(float(state["simulation_speed"]) == 1.0, "unsupported input must preserve the prior speed")
	var input := InputManager.new()
	var display := SpeedDisplay.new()
	input.simulation_node = simulation
	input.main_ui = display
	input.set_simulation_speed(2.0)
	for invalid_speed in [32.01, 1.0e30, INF, -INF, NAN]:
		input.set_simulation_speed(invalid_speed)
		_expect(input._simulation_speed == 2.0 and display.speed == 2.0, "rejected requests must preserve the controller and HUD speed")
	await create_timer(0.1).timeout
	_expect(float(simulation.get_road_benchmark_state()["simulation_speed"]) == 2.0, "rejected UI requests must preserve the accepted simulation speed")
	input.set_simulation_speed(-2.0)
	_expect(input._simulation_speed == 0.0 and display.speed == 0.0, "finite negative speed must pause the controller and HUD")
	await create_timer(0.1).timeout
	_expect(float(simulation.get_road_benchmark_state()["simulation_speed"]) == 0.0, "finite negative speed must retain pause clamping")
	for expected_speed in [0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 32.0]:
		input.step_simulation_speed(1)
		_expect(input._simulation_speed == expected_speed and display.speed == expected_speed, "speed-up must follow the shared steps and stop at 32x")
	await create_timer(0.1).timeout
	_expect(float(simulation.get_road_benchmark_state()["simulation_speed"]) == 32.0, "maximum supported speed must reach the simulation")
	for expected_speed in [16.0, 8.0, 4.0, 2.0, 1.0, 0.5, 0.0, 0.0]:
		input.step_simulation_speed(-1)
		_expect(input._simulation_speed == expected_speed and display.speed == expected_speed, "speed-down must follow the shared steps and stop at pause")
	input.free()
	display.free()
	simulation.queue_free()
	await process_frame
	print("Simulation speed input tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)
