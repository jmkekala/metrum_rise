<!-- ======================================================================
MANIFEST
===========================================================================
script_name: ENGINE_CONVERSION.md
script_path: docs/ENGINE_CONVERSION.md
module_name: Engine Conversion
version: 0.20.0
author: [BantedHam]
description: Converting Metrum Rise onto the 2.5D engine workflow: the
          transplant state, the simulation boundary, and the renderer
          conversion map.
kind: guide
spec: none
internal_dependencies: []
external_dependencies: [2.5D_engine, FILE_browser, GOAT_bus, SPEECH_socket]
features: [transplant, boundary, conversion-map]
api_version: metrum-v1.0.0
last_updated: 2026-08-31
======================================================================= -->

# Engine conversion

Metrum Rise converts onto the 2.5D engine workflow easily: no authored
meshes, evaluated fields raymarched on compute, and the one secondary
path of surfaces frozen to meshes and instanced by graph hash through
MultiMesh.

## Transplant state

All four addons mount by directory junction from the 2.5D_engine
repo: `2.5D_engine`, `GOAT_bus`, `SPEECH_socket`, and `FILE_browser`
under `godot/addons/` are junctions, not copies, so engine updates
land live with no sync pass and byte-identity holds by construction.
This repo tracks adapters, hooks, and renderer branches only, never
engine files, which is also the legal shape: from the junction commit
forward the game side carries no engine code. Each engine component
carries a LICENSE.md boundary marker: all rights reserved today, with
SPEECH_socket and FILE_browser stating the author's intent to release
as MIT once polished enough to merge into the game repo seamlessly.
The engine has no repo of its own; everything ships to the private
experimental repo when the push comes, and only the GPL side of the
line reaches the main fork, which is what the junctions and the
gitignore are for. A clone needs the
engine repo beside this one and a one-time junction step. The editor
writes generated .uid files through the junctions into the engine
tree, where they are the engine's own to commit. The four plugins are
enabled; they manage their own autoloads on activation, so none are
hand-registered. The global class cache carries the 266 addon class
registrations, which is what kept the first scan free of the
SPEECH_socket parse flood: the import scan logged zero addon script
errors.

## The simulation boundary

The Rust economy and the engine's fields keep their own ontologies and
meet at two arrays and a tick boundary.

- Down: once per tick the economy hands one batched position array and
  receives field samples per requested field. No per-agent calls.
- Up: agent actions aggregate into per-cell deposit buffers that enter
  the engine as measured rows, which override derived defaults exactly
  as the engine's measured-anchor rule already provides. No bespoke
  nodes, no special-cased laws.
- The coupling closes with one tick of lag: fields read at N, deposits
  land at N+1. Both sides drill against recorded boundary fixtures and
  neither test suite needs the other side running.

Events ride the GOAT bus through one gateway node in the Godot shell,
the Rust core's voice on the bus: batched event lists out, collected
subscriptions in as next-tick input. No Rust bus counterpart;
duplicating queue, replay, and backpressure semantics in a second
language is drift with a bridge in the middle. Bus replay of gateway
traffic re-drives the Rust sim for debugging.

Per-point kernels port to Rust one at a time as twins of the GDScript
reference, promoted only when the engine's golden audit holds, the same
gate the GLSL twins pass. Transcendental math policy decides bit-exact
versus tolerance comparison once, before the first kernel.

## Renderer conversion map

Six renderers under `godot/scripts/renderers/` are the whole surface,
11,098 lines.

| Renderer | Today | Converts to |
|---|---|---|
| terrain.gd | heightmap mesh | evaluated field, raymarched |
| water.gd | authored fills + border shader | field overlay |
| buildings.gd | pack meshes in MultiMesh | frozen evaluated geometry in the same MultiMesh plumbing |
| network_renderer.gd | road meshes | evaluated network field, frozen where interacted |
| agents.gd | vehicle/character models | engine actor path |
| zoning_overlay.gd | overlay draw | field visualisation |

buildings.gd is the beachhead: it already runs MultiMesh instancing fed
by Rust transform frames, so its conversion is a mesh-source swap from
authored packs to engine-frozen geometry while the Rust frame plumbing
survives untouched. Terrain and water are the deep rewrites. Authored
assets under `assets/models` and `assets/textures` retire per renderer
as its conversion lands.

## Order of work

1. DONE. engine_boundary.gd + spike_boundary.gd: batched sampling at
   f64 (a 32-bit boundary truncated samples and the spike caught it),
   deposits through the engine's signed-16-bit grid format read back
   cell-exact by heightmap_node, fixtures recorded with bit patterns.
   Ten checks green. Rust-side fixture tests staged in rust/tests.
2. DONE. rust_gateway.gd + spike_gateway.gd: outbound batches publish
   under rust/, watched events collect inbound, drain hands one tick's
   batch. The bus requires system_registry and config_manager before it
   delivers, and queued delivery rides its pump. Five checks green.
3. DONE. engine_mesh_source.gd behind the workflow toggle in
   buildings.gd's one loader function, packs as fallback: frozen
   evaluated shells, cached by key, deterministic, distinct per key.
   Seven checks green.
4. DONE. engine_terrain_source.gd in terrain.gd's height upload: RF
   bytes from the field, endpoint-inclusive so abutting patches share
   edges exactly. Five checks green. Full raymarching stays future work
   riding the engine's own compute path.
5. DONE for water (depth = level minus the same evaluated ground, so
   water fills the field's basins), network (road decks conform to the
   evaluated ground in network_tool's chunk upload), and agents (frozen
   evaluated forms, static until the actor path lands, VAT walk stays
   authored). Zoning needed NO change: its overlay renders economy
   data over whichever ground draws, already source-agnostic.
6. DONE. The fBm twin in rust/src/engine_twin/fbm.rs PROMOTED under
   the decided math policy: BIT-EXACT f64, operation order preserved as
   contract, no transcendentals in the kernel; a future kernel using
   libm escalates the policy before porting. rust/tests/fbm_twin.rs
   matched the reference's recorded bit patterns on every fixture
   position, first run, alongside both Rust-side boundary tests.

Every hook sits in Metrum Rise's own tree behind one toggle
(user://engine_meshes.cfg) with the Rust payloads as fallback; the three
addons are byte-identical to their source.

## System integrations

Each system consumes the engine at the boundary, drilled headless.

- Sound, DERIVED: acoustic_node's modal profiles (frequencies and decay
  from composition, geometry, wetness) rendered by damped modal
  synthesis in engine_sound_source.gd; engine_sound_player.gd mounts
  anywhere and strike() plays the material. Eight engine materials,
  audible deterministic decaying strikes, material identity audible,
  wetness damping measured. Eight checks green.
- Physics: engine_physics_source.gd, ground contact from the field's
  own gradient, no collision meshes: height, unit normal, slope.
- Minerals: engine_mineral_source.gd, channel-salted strata of the same
  world noise, coal scaled by biomineral_node's carbon accounting so
  burial is what coal is.
- Social: engine_social_source.gd, parcel desirability from flatness
  and the shore band of the same evaluated ground, batched at parcel
  positions. Fourteen system checks green across the three.

## Integration register

Every evaluator node, decided. The rule: no source without a consumer,
no consumer without a source; wiring an awaiting node later is a source
plus a drill, never a port.

WIRED, a game system reads them: fbm (terrain, veins, physics ground),
heightmap (deposits), acoustic (sound), weather (conditions), fire
(danger), orbital + sealevel (tides, breathing the water level through
the EngineTick autoload), mind (FINISHED: a roster of living
instances, spawn/tick/state on the node's own fixed step, each
carrying a certified creature and its drives; the policy table was
removed upstream with the layer it certified, so the intake's policy
channel is gone with it, and the actor path mounts bodies on these
minds when it lands; connectome arbitration rides inside the node),
director (FINISHED: pacing from gateway traffic through the seeded
variety walk, thresholds ruling and the chain filling only the freedom
they leave, population and threat multipliers per state), hydrology +
flood (standing water, basins), strata + biomineral (minerals),
habitability (desirability with the city's own buildability),
soundscape (the derived wind bed), gait (walk phase per stride),
contagion (outbreaks for the health services), snowpack (winters on
the weather seed), cartography (the overview's projection), plant
(grown flora, cached per plot), vehicle (Pacejka tires, the friction
circle, powertrain pull, and the rollover verdict for traffic), with
GOAT_bus as the event trunk pumped each tick. electrokinetics returns
to awaiting: it is ion physics, not lightning.

TRANSITIVE, consumed inside wired calls: pressure, circulation, cloud,
coriolis and the rest of weather's interior; mineral's catalog under
strata; lens under strata's config; ctx and coupling wherever nodes
compose.

AWAITING A CONSUMER, present and unwired: the water interior (drainage,
oceancurrent, waterchem, openwater, gully, karst, snowpack, avalanche,
droplet, cavitation), land interior (orogeny, isostasy, plate,
volcanism, landform, soiltexture, avalanche, delta, front), life
(creature, anatomy, limb, gait, plant, biosphere, evolution, migration,
population, society, contagion, symbiosis, trophic, metabolism,
ontogeny, lineage, maturation, trait, habitability, nicheconstruct,
nichematrix, colonize, neurotaxonomy, perception, intent), sound's
sequels (music, soundscape, voice, sympathy, pluck, surfacesynth,
godverb, cue, avsync), city and content generation (urban, fractalcity,
road, pavement, wfc, grammar, markov, lore, cartography, museum,
showcase_planet), rendering interior (footprint, light, optics, lens,
godray, shimmer, color, paint, pixelate, camera, frame, framechain,
microscale, microslice, mosaic, sdf, sdfbrick, primitive, scatter,
branch, graft, phyllotaxis, membrane, softbody, coating, metamorph,
fracture, stress, striker, collide, explosion, impact, chain,
coupled_chain, conduit, edge, vortex, flight, traverse), the far shelf
(cosmology, cosmos, stellar, quantum, arcanum, spellgraph, invocation,
speculate, worldcomp, worldplace, worldpolicy, worldscale, planner,
director, grammar, honesty, attribution, budget, lockstep, recording,
export, export_driver, import_route, ingest, rasteringest, geotiff,
index, profile, preset, plugin, hook, player, vehicle, entity,
element, custom_element, material, variance, potential, phase,
reaction, mixing, thermogas, airchem, electrokinetics, electromech,
emfield, biolight, carboncycle, decay, criticality, accrete, adpm,
aeolian, affordance, arcanum, avsync, cavitation, circulation,
disperse, drainage, droplet, fieldstep, gully, isostasy, karst,
landform, lifeground, orientation, pinned).

The awaiting list is a decision record, not a backlog: each name moves
up only when a game system starts reading it.

## The Rust intake, stage one

The core now has an intake: engine_api.rs holds a revisioned store the
shell fills through set_engine_inputs (desirability and the three
mineral channels, one call per delivery; the policy channel left with
the layer that produced it), with summary
and revision accessors, and engine_inputs_snapshot() for the sim thread
to read without holding a lock across a tick. EngineTick delivers every
120 ticks on a probe grid AIMED AT THE CITY: engine_parcel_bounds hands
up the parcels' world bounding box and the grid spans it plus a 200 m
margin (50 m spacing floor so a one-parcel city never collapses it),
with the listener-centred layout as the fallback before any city
exists. The storage round-trip test passes and the fBm twin
still matches bit for bit after the rebuild; the deployed DLL carries
the API. Stage two's first consumer is LIVE: the land value grid's
base is the delivered engine desirability wherever the probe covers
(bilinear, at the simulation layer, one snapshot per tick so no lock
crosses the parallel loop) and the flat 50 wherever it does not, so no
delivery means byte-identical old behaviour. The save loader passes the
same config.

The coal loop is CLOSED the same way: the extractor reserve treats a
painted deposit cell as a measured row that wins outright, and an
unpainted cell falls back to the delivered engine coal channel through
the same bilinear geometry, clamped to full richness, zero outside
coverage. The snapshot rides in as a parameter so the reserve is one
delivery's coherent view and the tests never race the global store;
the fallback is computed live at commit time and never baked into a
save, so derived values cannot masquerade as authored ones. Iron and
stone are delivered and waiting in the store; they get consumers when
the economy catalog grows extractors for them. The paint overlay still
shows authored cells only; engine coal is invisible on it until a
survey view exists.

Ground truth reconciliation is BUILT: the shell's ground is
Fbm.evaluate(wx, 0, wz, 0.5, 0, 0x2E5D) * 8.0 (engine_terrain_source),
and apply_engine_ground on the core derives every terrain sample still
at the base elevation from the promoted twin at that sample's world
position, zero boundary traffic, into both height buffers, with the
same road-surface dirtying a sculpt runs. Sculpted samples stay as
measured overrides and nothing is undoable, because derivation is not
an edit. EngineTick calls it once per session when the workflow toggle
is on, passing the terrain source's own constants so the parameters
cannot drift from what the renderer draws. Sim slopes, placement,
raycasts, and clipping then agree with the drawn ground. Terrain fill
test green; cache-free parse gate green. Both game load paths reset
the flag through EngineTick.reapply_ground(), so a loaded world's
untouched cells re-derive, and derivation is gated to Main.tscn: it
never runs in the editors, where a filled cell would export as
authored world data. Remaining caveat: a savegame SAVED from a filled
game session stores the derived cells (savegames persist full state);
on load they read as sculpted and stop re-deriving, which is stable
but freezes that world's ground at the fill-time field.

## The windowed proof, the world scale, and the pixel posture

spike_engine_live.gd runs the real Main scene in a rendered window,
the traffic-spike way: toggle on, boot, then every insertion point
answers through the same objects the game uses, with screenshots. Its
first run caught what headless never could: the ground sources fed
raw metres into a field whose base wavelength is one unit, so the
world was needle terrain with 80 degree slopes and desirability
scored an honest zero everywhere. The fixes, each one law:

- WORLD_SCALE_M (1000 m per field unit) in engine_terrain_source, and
  ground_m() is THE ground: terrain, water, network, and physics all
  read it, so the scale can never fork. The Rust apply_engine_ground
  takes the same scale from the shell.
- The footprint is a band limit in field units, authored in metres,
  so it rides the same division; raw it faded out every octave finer
  than half a kilometre and a 64 m patch measured flat. Every sampler
  passes its own spacing as its band limit: the sim its 10 m cell,
  each render patch its texel step, because finer octaves can only
  alias on the grid that reads them. The fill parallelizes by row
  through rayon. Boot on this box went 684 s to 353 s across the
  three fixes; the remaining floor is per-texel GDScript patch
  evaluation, whose successor is the compute path under step 4.
  Drawn micro-detail below the sim grid diverges by the fine
  octaves' amplitude, about 12 cm worst case, which a 10 m grid
  could never represent anyway.
- Habitability answers per-biome suitabilities, a winner, and
  moisture; a scalar score key never existed, and reading one scored
  every parcel 0.0. The social source now takes the settlement read:
  authored game weights over the engine's biomes (grassland and
  temperate found cities, arid and rainforest resist), normalised by
  the suitabilities' own mass. The windowed mean landed at 0.5303.
- Pixel art is all the way on: EngineTick sets the viewport's render
  scale to one art pixel per pixel_size screen pixels from the
  pixelate node's own config, the cheap end of SPEC 13.9's dial until
  the dial itself is built, and the largest performance lever this
  box has.

Four windowed runs green, 13 checks each, screenshots kept under
screenshots/. Benchmark context for the 353 s boot: AMD A9-9425, two
cores, 7.5 GB RAM, dev-profile Rust at opt-level 0; the ground fill
covers the 20 km world at 10 m cells, 2001 x 2001 = 4,004,001
samples, parallel over both cores; the second apply's scan answers
in 1.4 s.

The projection, from the author's math: PassMark puts the A9-9425 at
1,521 and a Ryzen 7 7800X3D at 34,277, so the average Steam survey
device (an RTX 3080, 16 GB of RAM, an 8-core CPU) should handle the
CPU-only render in 10 to 15 seconds at most, and with the GPU twin
taking part of the load it renders almost instantly. This box is the
floor, not the estimate.

## Open integration work

Testing, in rough order of what blocks what:

- Terrain deformation: convicted by inspection before any run. With
  the toggle on the renderer draws the pure field while sculpts and
  earthworks write the sim heightmap, so deformation works in the sim
  and never shows on screen. The fix is the same law as everywhere:
  the drawn ground reads field plus measured overrides. Then the live
  drill: sculpt in-window, sim height, drawn patch, and road conform
  move together.
- The coal loop live: place an extractor over engine-rich ground,
  commit the polygon, watch reserve, production, and the pit mask.
- Desirability steering growth: zone across a grassland-to-arid
  gradient, assert the growth differential.
- The parcel-bounds re-aim with parcels actually existing.
- Save and load of a filled world, the fill-freeze caveat included.
- Frozen building shells and the sound path in a rendered window.
- Sustained frame rate with agents, not just boot.

Boundary work still open:

- Resource scale: DONE for the vein channel (VEIN_SCALE_M 250 m,
  authored ore-body taste; ten metres reads the same body, a
  kilometre spreads 0.33, drilled). Strata, biomineral, and
  habitability turned out to vary at engine-native scales already,
  so raw coordinates were only ever wrong in the direct Fbm
  samplers. Iron and stone still await economy-catalog extractors.
- The ground is one fBm field; terrain appropriate for mining,
  farming, and flooding means composing it from more of the engine
  (drainage, landform, hydrology carving), design work on the ground
  function.
- Deposits upward: write_deposit is drilled but no game action writes
  one, so the engine never feels the city yet.
- The director's population and threat multipliers have no sim
  consumer.
- PRE-EXISTING metrum bug, exposed by the placement probes and
  independent of the engine work: on a flat world with a nonzero
  base elevation (blank world at base 5 m, no engine fill), building
  placement fails with "no nearby road frontage can fit this
  building". Flat-zero worlds mask it because every height
  convention agrees at zero. Recorded for the author's ruling; it
  predates the transplant.

Editor work, game side:

- The asset creator/editor updates for alley frontage and the new
  road shapes; frontage_class and VehicleFrontageAccess already exist
  in the core, so the editor catching up is mechanical once the alley
  semantics are confirmed.
- The engine's tiny-glade-style editor ports into Metrum Rise onto it's
  current asset editor, adding a building editor as a planned feature, 
  riding the same evaluated fields the game draws.

Commit push target for 2.5D_engine is `metrum_rise_experimental`,
pending the engine boundary and license conversation. All other work is
safe to push to the main fork and PR.

## Screenshots

Windowed runs write into a dated `screenshots/spikes_<range>/` folder
under the naming convention this project keeps:
`spikeNN_<subject>[_pixelart_filter]_<W/100>x<H/100>.png`. The
resolution collapses two zeros (1604x881 reads 16x9), the filter is
named only when it is on, and nothing overwrites anything.

- Spike 1, world scale and the pixel posture: kilometre-scale ground for the first time.
  ![worldscale_boot_and_live](../screenshots/spikes_30.8.26-2.9.26/spike01_0b91ab8d_worldscale_boot_and_live_pixelart_filter_16x9.png)
- Spike 1, mid-boot.
  ![worldscale_booting](../screenshots/spikes_30.8.26-2.9.26/spike01_0b91ab8d_worldscale_booting_pixelart_filter_16x9.png)
- Spike 2, the benchmark ledger and the first deformation, drawn as a mound before the drill was corrected to dig.
  ![ledger_sculpt_live](../screenshots/spikes_30.8.26-2.9.26/spike02_489afec2_ledger_sculpt_live_pixelart_filter_16x9.png)
- Spike 3, the stored-height convention: hills at their authored gentleness, and the coal loop closing.
  ![storedconvention_live](../screenshots/spikes_30.8.26-2.9.26/spike03_d618e61f_storedconvention_live_pixelart_filter_16x9.png)
- Spike 4, the dig beside the placed mine and its road, with the terrain deadlock fenced.
  ![dig_and_deadlock](../screenshots/spikes_30.8.26-2.9.26/spike04_47c6dd01_dig_and_deadlock_pixelart_filter_16x9.png)
- Spike 5, the city grown on delivered land value.
  ![city_grown](../screenshots/spikes_30.8.26-2.9.26/spike05_497394ff_city_grown_pixelart_filter_16x9.png)

## The batch before the push

Every spike registers one JSON entry per run in benchmarks.json
(verdict, wall time, CPU, cores, RAM, Godot version), so any box's
numbers stand beside any other's.

What the batch proved and learned, each finding measured before it
was written:

- The city grows on delivered land value, witnessed windowed: five
  family houses under construction on zoned parcels, the coal loop
  banking 6,979.99 units, the dig drawing, a strike playing, 49
  ms/frame sustained at speed five. Demand needs an outside world (a
  Border node with a road edge lifts pressure; a living economy needs
  the route too, because households, workers, and production all
  waited on a connected border road), opened through
  check_border_candidate and set_border_connection within 3 m of the
  map's true edge.
- The save round trip holds exactly: dig, fill, buildings, and parcel
  bounds return bit-for-bit, and the fill-freeze caveat measures as
  recorded (a post-load re-apply finds zero untouched cells).
- The boundary's upward half is closed: extraction aggregates into
  hundred-metre deposit cells on each delivery, written in the
  engine's converted-grid format (north-up: latitude decreases with
  row) and read back cell-exact by the engine's own heightmap node.
- The director paces arrivals through the game's own dial: the
  population multiplier drives border openness, surfaced as the
  twelfth fiscal control the presentation layer was built around.
- Iron and stone have consumers (profiles, delivered-channel
  fractions, engine-only reserve walking) and await only building
  assets from the asset-editor pass.
- The terrain process loop deadlocks on the state a stepped sim
  leaves behind (six reproductions, renderer lineup conviction, 0.00 s
  CPU over eight); it stays fenced in the live spike, and the
  diagnosis owns a task. Every windowed launch embeds a log-stall
  watchdog.
