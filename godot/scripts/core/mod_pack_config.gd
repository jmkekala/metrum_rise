# SPDX-License-Identifier: GPL-2.0-only

## Shared mod-pack activation defaults and persistence helpers.
##
## Missing config means a fresh player profile and enables the bundled starter pack. Once the
## player saves a selection, even an empty enabled list is treated as intentional.
extends RefCounted

const CFG_PATH := "user://active_packs.cfg"
const DEFAULT_ENABLED_PACK_IDS := ["kenney"]

static func load_enabled_pack_ids() -> Array:
	if not FileAccess.file_exists(CFG_PATH):
		return DEFAULT_ENABLED_PACK_IDS.duplicate()
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
	var err := save_enabled_pack_ids(DEFAULT_ENABLED_PACK_IDS)
	if err != OK:
		push_warning(
			"Could not write default active pack config '%s' (error %d)." % [CFG_PATH, err]
		)
