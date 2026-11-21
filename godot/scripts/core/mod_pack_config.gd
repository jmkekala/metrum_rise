# SPDX-License-Identifier: GPL-3.0-or-later

## Shared mod-pack activation defaults and persistence helpers.
##
## Missing config means a fresh player profile and enables the default packs. Once the
## player saves a selection, even an empty enabled list is treated as intentional. The game
## bundles no building pack; a development run also enables the repository's test building pack.
extends RefCounted

const CFG_PATH := "user://active_packs.cfg"
const BUNDLED_ENABLED_PACK_IDS := ["parking", "vehicles"]
# Bundled packs a saved selection has already been offered. A pack bundled later is enabled once
# in a profile that saved its selection before it shipped, so an existing player gets it like a
# fresh install; disabled afterwards, it stays disabled. A selection saved before this list
# existed was offered `parking` only.
const OFFERED_KEY := "bundled_offered"
const OFFERED_BEFORE_LIST := ["parking"]
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
	enabled = enabled if enabled is Array else []
	var offered = cfg.get_value("packs", OFFERED_KEY, OFFERED_BEFORE_LIST)
	offered = offered if offered is Array else OFFERED_BEFORE_LIST
	var newly := BUNDLED_ENABLED_PACK_IDS.filter(func(id: String) -> bool: return not id in offered)
	if not newly.is_empty():
		for id in newly:
			if not id in enabled:
				enabled.append(id)
		save_enabled_pack_ids(enabled)
	return enabled

static func save_enabled_pack_ids(enabled_pack_ids: Array) -> Error:
	var cfg := ConfigFile.new()
	cfg.set_value("packs", "enabled", enabled_pack_ids.duplicate())
	cfg.set_value("packs", OFFERED_KEY, BUNDLED_ENABLED_PACK_IDS.duplicate())
	return cfg.save(CFG_PATH)

static func seed_default_config_if_missing() -> void:
	if FileAccess.file_exists(CFG_PATH):
		return
	var err := save_enabled_pack_ids(default_enabled_pack_ids())
	if err != OK:
		push_warning(
			"Could not write default active pack config '%s' (error %d)." % [CFG_PATH, err]
		)
