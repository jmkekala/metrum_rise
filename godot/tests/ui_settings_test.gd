# SPDX-License-Identifier: GPL-2.0-only

## Verifies settings normalization/default recovery and coherent UI scale refreshes.
## The original settings file is restored; benchmark setup and assertions stay outside timing.
extends SceneTree

const Settings = preload("res://scripts/core/game_settings.gd")
const UIStyle = preload("res://scripts/ui/ui_style.gd")
var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	var had_config := FileAccess.file_exists(Settings.CFG_PATH)
	var original := FileAccess.get_file_as_bytes(Settings.CFG_PATH) if had_config else PackedByteArray()
	var cfg := ConfigFile.new()
	cfg.set_value(Settings.SECTION_ACCESSIBILITY, Settings.KEY_UI_SCALE, 1.25)
	cfg.set_value("layout/probe", "window_width", 9999)
	_expect(Settings.save_config(cfg) == OK, "Fixture settings must save")
	var style: Script = UIStyle
	for argument in OS.get_cmdline_user_args():
		if argument.begins_with("--style-path="):
			style = load(argument.trim_prefix("--style-path="))
	if "--benchmark-ui-scale" in OS.get_cmdline_user_args():
		_benchmark(style)
	else:
		for invalid in [NAN, INF, -INF]:
			_expect(Settings.normalized_ui_scale(invalid) == Settings.DEFAULT_UI_SCALE, "Non-finite scale must use the default")
		for pair in [[-1.0, 0.8], [0.8, 0.8], [1.24, 1.25], [1.5, 1.5], [2.0, 1.5]]:
			_expect(is_equal_approx(Settings.normalized_ui_scale(pair[0]), pair[1]), "Finite scale retains clamping and 0.05 steps")
		# ConfigFile keeps values parsed before an error; recovery must reset the whole object.
		Settings._write_defaults(cfg)
		_expect(not cfg.has_section("layout/probe"), "Defaults must discard partially parsed layout state")
		_expect(Settings.load_config().has_section("layout/probe"), "Valid config loads must preserve unrelated layout state")
		_expect(Settings.save_ui_scale(NAN) == OK and Settings.get_ui_scale() == Settings.DEFAULT_UI_SCALE, "Saving an invalid scale must never persist NaN")
		_test_refresh(style)
	if had_config:
		var file := FileAccess.open(Settings.CFG_PATH, FileAccess.WRITE)
		file.store_buffer(original)
		file.close()
	else:
		DirAccess.remove_absolute(ProjectSettings.globalize_path(Settings.CFG_PATH))
	print("ui_settings_test: %s" % ("PASS" if _failures == 0 else "FAIL"))
	quit(_failures)

func _test_refresh(style: Script) -> void:
	var viewport := SubViewport.new()
	viewport.size = Vector2i(1920, 1080)
	root.add_child(viewport)
	var window := Window.new()
	window.visible = false
	viewport.add_child(window)
	var label := Label.new()
	window.add_child(label)
	var small := Label.new()
	label.add_child(small)
	_expect(Settings.save_ui_scale(1.25) == OK, "Scale must save before refresh")
	style.set_window_base_size(window, Vector2i(400, 300), Vector2i(200, 120), viewport)
	style.set_font_size(label, 13)
	style.set_font_size(small, 4)
	style.refresh_scaled_font_sizes(viewport)
	_expect(label.get_theme_font_size("font_size") == 16 and small.get_theme_font_size("font_size") == 8, "Refresh retains rounding and minimum font size through nested nodes")
	_expect(window.size == Vector2i(500, 375) and window.min_size == Vector2i(250, 150), "Refresh preserves base and minimum window scaling")
	_expect(Settings.save_ui_scale(1.5) == OK, "A later scale must save")
	style.refresh_scaled_font_sizes(viewport)
	_expect(label.get_theme_font_size("font_size") == 20 and window.size == Vector2i(600, 450), "A later refresh reads the changed setting rather than cached state")
	_expect(Settings.save_ui_scale(0.8) == OK, "A smaller scale must save")
	style.refresh_scaled_font_sizes(viewport)
	_expect(label.get_theme_font_size("font_size") == 10 and window.size == Vector2i(600, 450), "Smaller text retains the player's larger window")
	viewport.free()

func _benchmark(style: Script) -> void:
	for count in [16, 256, 4096]:
		var container := Node.new()
		root.add_child(container)
		var labels: Array[Label] = []
		for index in count:
			var label := Label.new()
			label.set_meta(UIStyle.FONT_SIZE_META, 13)
			container.add_child(label)
			labels.append(label)
		style.refresh_scaled_font_sizes(container)
		var samples: Array[float] = []
		for sample in 9:
			var start := Time.get_ticks_usec()
			style.refresh_scaled_font_sizes(container)
			samples.append(float(Time.get_ticks_usec() - start))
			_expect(labels.all(func(label: Label): return label.get_theme_font_size("font_size") == 16), "Every benchmark label must keep the expected font size")
		samples.sort()
		print("UI_SCALE_BENCH " + JSON.stringify({"labels": count, "median_us": samples[4], "samples_us": samples}))
		container.free()
