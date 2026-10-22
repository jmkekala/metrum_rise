# SPDX-License-Identifier: GPL-2.0-only

## Tools > Weather and Tools > Looks dispatch the scenario index the Rust bridge expects and pin
## the hour the Time of Day menu shows.
extends SceneTree

const TopMenu = preload("res://scripts/ui/top_menu.gd")
const DayCycleConfig = preload("res://scripts/core/day_cycle.gd")

class SceneRoot extends Node:
	var hours: Array = []
	var weather: Array = []
	var automatic := true
	func menu_set_time_of_day(hour: float) -> void:
		hours.append(hour)
	func menu_set_weather(index: int) -> void:
		weather.append(index)
		automatic = false
	func menu_set_weather_automatic(enabled: bool) -> void:
		automatic = enabled
	func menu_weather_automatic() -> bool:
		return automatic

var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	var scene := SceneRoot.new()
	root.add_child(scene)
	var menu := TopMenu.new()
	menu.scene_kind = TopMenu.SCENE_GAMEPLAY
	scene.add_child(menu)
	await process_frame
	# The labels follow the Rust WeatherScenario order: clear, fair cumulus, overcast, storm.
	menu._on_tools_menu_pressed(TopMenu.ActionId.TOOLS_WEATHER_FIRST + 3)
	_expect(scene.weather == [3], "Weather > Storm must request scenario 3, got %s" % [scene.weather])
	_expect(scene.hours.is_empty(), "choosing a weather must leave the hour alone")
	var popup_weather: PopupMenu = menu._weather_popup
	var automatic_index := popup_weather.get_item_index(TopMenu.ActionId.TOOLS_WEATHER_AUTOMATIC)
	_expect(not popup_weather.is_item_checked(automatic_index), "choosing a weather must uncheck Automatic")
	menu._on_tools_menu_pressed(TopMenu.ActionId.TOOLS_WEATHER_AUTOMATIC)
	_expect(scene.automatic and popup_weather.is_item_checked(automatic_index), "Automatic must hand the weather back to the schedule")
	scene.automatic = false
	menu._sync_weather_automatic()
	_expect(not popup_weather.is_item_checked(automatic_index), "opening the menu must show the schedule's state")
	for index in range(TopMenu.WEATHER_SCENARIOS.size()):
		var listed := popup_weather.get_item_index(TopMenu.ActionId.TOOLS_WEATHER_FIRST + index) >= 0
		_expect(listed == (index != TopMenu.HIDDEN_RAIN_SCENARIO),
			"Weather must hide only Rain and keep the other scenarios' action IDs")
	var looks: PopupMenu = menu.find_child("Looks", true, false)
	_expect(looks != null, "Tools must retain its Looks menu")
	if looks != null:
		for index in range(TopMenu.LOOK_PRESETS.size()):
			var visible := looks.get_item_index(TopMenu.ActionId.TOOLS_LOOK_FIRST + index) >= 0
			_expect(visible == (TopMenu.LOOK_PRESETS[index][2] != 4),
				"Looks must hide only the rain preset and preserve other action IDs")
	for index in range(TopMenu.LOOK_PRESETS.size()):
		var look: Array = TopMenu.LOOK_PRESETS[index]
		_expect(DayCycleConfig.PRESET_HOURS.has(look[1]), "look %s must name a Time of Day preset" % look[0])
		_expect(look[2] >= 0 and look[2] < TopMenu.WEATHER_SCENARIOS.size(), "look %s must name a weather" % look[0])
		scene.hours.clear()
		scene.weather.clear()
		menu._on_tools_menu_pressed(TopMenu.ActionId.TOOLS_LOOK_FIRST + index)
		_expect(scene.hours == [DayCycleConfig.PRESET_HOURS[look[1]]], "look %s must pin its hour" % look[0])
		_expect(scene.weather == [look[2]], "look %s must set its weather" % look[0])
		var popup: PopupMenu = menu._time_of_day_popup
		var checked := popup.get_item_index(TopMenu.ActionId.TOOLS_TIME_OF_DAY_FIRST
			+ DayCycleConfig.PRESET_HOURS.keys().find(look[1]))
		_expect(popup.is_item_checked(checked), "look %s must check its hour in Time of Day" % look[0])
	print("weather_menu_test: %s" % ("PASS" if _failures == 0 else "FAIL"))
	quit(_failures)
