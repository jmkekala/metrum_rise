# SPDX-License-Identifier: GPL-3.0-or-later

## The Graphics option "Terrain relief shading" (`RENDER-78`): its default, a round trip through
## the Options panel and the settings file, the value the shader global takes, and the console's
## session override, which wins over the stored setting and is never saved. The original
## settings file is restored.
extends SceneTree

const Settings = preload("res://scripts/core/game_settings.gd")
const GraphicsOptions = preload("res://scripts/ui/graphics_options.gd")

var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

# The global as the server holds it, where the renderer lets it be read back (the headless
# dummy may not); null when it cannot.
func _expect_global(expected: Vector2, when: String) -> void:
	var held: Variant = RenderingServer.global_shader_parameter_get(Settings.TERRAIN_RELIEF_GLOBAL)
	if held is Vector2:
		_expect(held == expected, "%s: the shader global must hold %s, not %s" % [when, expected, held])

func _stored() -> Variant:
	return Settings.load_config().get_value(Settings.SECTION_GRAPHICS, Settings.KEY_TERRAIN_RELIEF, null)

func _run() -> void:
	var had_config := FileAccess.file_exists(Settings.CFG_PATH)
	var original := FileAccess.get_file_as_bytes(Settings.CFG_PATH) if had_config else PackedByteArray()
	Settings.terrain_relief_override = null
	var cfg := ConfigFile.new()
	Settings._write_defaults(cfg)
	_expect(Settings.save_config(cfg) == OK, "fixture settings must save")
	_expect(Settings.get_terrain_relief() == 1.0, "the default is Normal, strength 1")
	_expect(Settings.terrain_relief_global() == Vector2(1.0, 0.0), "Normal is the add form at strength 1")
	_expect(Settings.terrain_relief_global() == ProjectSettings.get_setting("shader_globals/%s" % Settings.TERRAIN_RELIEF_GLOBAL)["value"],
		"the settings default and the project's shader global default must agree")

	# Through the panel, as the player does it: pick High, Apply, and the file and global follow.
	var panel: Node = GraphicsOptions.new()
	root.add_child(panel)
	await process_frame
	var cycler: Variant = panel._choices.get(Settings.KEY_TERRAIN_RELIEF)
	_expect(cycler != null and cycler.get_item_count() == Settings.TERRAIN_RELIEF_CHOICES.size(),
		"the Graphics panel must list one choice per preset")
	_expect(panel._index_for_value(Settings.KEY_TERRAIN_RELIEF, 1.0) == 2, "Normal must show as the third choice")
	for index in Settings.TERRAIN_RELIEF_CHOICES.size():
		var value: float = panel._value_for_index(Settings.KEY_TERRAIN_RELIEF, index)
		panel._set_pending(Settings.KEY_TERRAIN_RELIEF, value)
		_expect(panel.apply_changes() == OK, "Apply must save")
		_expect(_stored() == value, "the stored value must be %s after Apply, not %s" % [value, _stored()])
		_expect(Settings.get_terrain_relief() == value, "the setting must read back %s" % value)
		Settings.apply_terrain_relief()
		_expect_global(Vector2(value, 0.0), "after Apply %s" % value)
		panel.refresh()
		_expect(panel._pending[Settings.KEY_TERRAIN_RELIEF] == value, "reopened Options must show %s" % value)
	panel.queue_free()

	# A stored value that is not a preset falls back to the default.
	var odd := Settings.load_config()
	odd.set_value(Settings.SECTION_GRAPHICS, Settings.KEY_TERRAIN_RELIEF, 7.0)
	Settings.save_config(odd)
	_expect(Settings.get_terrain_relief() == 1.0, "an unknown stored strength must read as the default")

	# The console's session override wins over the stored setting and is never saved.
	var low := Settings.load_config()
	low.set_value(Settings.SECTION_GRAPHICS, Settings.KEY_TERRAIN_RELIEF, 0.5)
	Settings.save_config(low)
	Settings.terrain_relief_override = Vector2(2.0, 1.0)
	Settings.apply_terrain_relief()
	_expect(Settings.terrain_relief_global() == Vector2(2.0, 1.0), "the session override must win")
	_expect_global(Vector2(2.0, 1.0), "with the override")
	_expect(_stored() == 0.5, "the override must not reach the settings file")
	Settings.terrain_relief_override = null
	Settings.apply_terrain_relief()
	_expect(Settings.terrain_relief_global() == Vector2(0.5, 0.0), "clearing the override returns to the setting")
	_expect_global(Vector2(0.5, 0.0), "after clearing the override")

	if had_config:
		var file := FileAccess.open(Settings.CFG_PATH, FileAccess.WRITE)
		file.store_buffer(original)
		file.close()
	else:
		DirAccess.remove_absolute(Settings.CFG_PATH)
	Settings.apply_terrain_relief()
	var readable := RenderingServer.global_shader_parameter_get(Settings.TERRAIN_RELIEF_GLOBAL) is Vector2
	print("terrain_relief_setting_test: shader global read back: %s" % ("yes" if readable else "no (renderer)"))
	if _failures == 0:
		print("terrain_relief_setting_test: PASS")
	quit(1 if _failures else 0)
