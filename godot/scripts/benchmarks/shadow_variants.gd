# SPDX-License-Identifier: GPL-2.0-only

## The sun-shadow variants the idle and stream benchmarks measure (`RENDER-15`). Every variant
## starts from the High setting as `SceneLighting.apply_shadow_quality` sets it, then removes or
## cheapens one part: `none` the sun's shadow, `filter2` / `filter3` the soft filter at quality 2
## or 3 (High is the project's 4), `no_pcss` the variable penumbra (angular distance 0, a plain
## PCF lookup), `no_blend` the blending between splits, `low` the game's Low setting,
## `no_cloud_shadows` the receivers' cloud lookup, and `no_casters` every caster (the light's
## `shadow_caster_mask` 0): the shadow maps draw nothing while the receivers still read them, so
## against `full` it is the drawing plus the penumbra filter the blocker search then skips.
## Benchmark-only: nothing in the game calls it.
extends RefCounted

const NAMES: Array[String] = ["full", "none", "filter2", "filter3", "no_pcss", "no_blend", "low",
	"no_cloud_shadows", "no_casters"]
# Light3D's default caster mask: every layer.
const ALL_LAYERS := 0xFFFFFFFF

## Applies `variant` (one of NAMES; any other name is `full`). `lighting` is the SceneLighting node,
## untyped so a benchmark copied over an older tree's scripts still loads.
static func apply(lighting: Variant, sun: DirectionalLight3D, variant: String) -> void:
	lighting.apply_shadow_quality(sun, variant == "low")
	sun.shadow_enabled = variant != "none"
	sun.shadow_caster_mask = 0 if variant == "no_casters" else ALL_LAYERS
	lighting.cloud_shadows_allowed = variant != "no_cloud_shadows"
	match variant:
		"filter2":
			RenderingServer.directional_soft_shadow_filter_set_quality(RenderingServer.SHADOW_QUALITY_SOFT_LOW)
		"filter3":
			RenderingServer.directional_soft_shadow_filter_set_quality(RenderingServer.SHADOW_QUALITY_SOFT_MEDIUM)
		"no_pcss":
			sun.light_angular_distance = 0.0
		"no_blend":
			sun.directional_shadow_blend_splits = false
