# SPDX-License-Identifier: GPL-3.0-or-later

## Shared mod-pack activation defaults and persistence helpers.
##
## Missing config means a fresh player profile and enables the default packs. Once the
## player saves a selection, even an empty enabled list is treated as intentional. The game
## bundles no building pack; a development run also enables the repository's test building pack.
extends RefCounted

const CFG_PATH := "user://active_packs.cfg"
const BUNDLED_ENABLED_PACK_IDS := ["parking"]
# Seeded from res://tests/mods by UserDataBootstrap; an exported game has neither.
const DEV_ENABLED_PACK_IDS := ["kenney"]

static func default_enabled_pack_ids() -> Array:
	if OS.has_feature("template"):
		return BUNDLED_ENABLED_PACK_IDS.duplicate()
	return DEV_ENABLED_PACK_IDS + BUNDLED_ENABLED_PACK_IDS

static func load_enabled_pack_ids() -> Array:
	if not FileAccess.file_exists(CFG_PATH):
		return default_enabled_pack_ids()
	var cfg := ConfigFile.new()
	var err := cfg.load(CFG_PATH)
	if err != OK:
		push_warning("Could not read active pack config '%s' (error %d)." % [CFG_PATH, err])
		return []
	var enabled = cfg.get_value("packs", "enabled", [])
	return enabled if enabled is Array else []

static func save_enabled_pack_ids(enabled_pack_ids: Array) -> Error:
	var cfg := ConfigFile.new()
	cfg.set_value("packs", "enabled", enabled_pack_ids.duplicate())
	return cfg.save(CFG_PATH)

static func seed_default_config_if_missing() -> void:
	if FileAccess.file_exists(CFG_PATH):
		return
	var err := save_enabled_pack_ids(default_enabled_pack_ids())
	if err != OK:
		push_warning(
			"Could not write default active pack config '%s' (error %d)." % [CFG_PATH, err]
		)
