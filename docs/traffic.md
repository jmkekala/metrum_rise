# Metrum Rise - Traffic Movement

This document owns lane-bound vehicle movement behavior: car following, junction connector
traversal, lane changes, conservative overtaking, render smoothing, and traffic debug output.

It does not own:

- road surface / roadbed geometry; see [`roads.md`](roads.md)
- building entrance planning and exact attach / detach selection; see
  [`entrance_and_exit.md`](entrance_and_exit.md)
- stable constants tables and buffer formats; see [`reference.md`](reference.md)
- future public transport mode design; see `TRANSIT-*` rows in [`roadmap.md`](roadmap.md)

Implementation lives mainly in:

- `rust/src/simulation/economy/agents/tick.rs`
- `rust/src/simulation/economy/agents/data.rs`
- `rust/src/simulation/network/lanes/vehicle_junctions.rs`
- `rust/src/nodes/sim/render/lane_pose.rs`
- `rust/src/nodes/sim/core/snapshot.rs`
- `rust/src/nodes/sim/render/agents.rs`
- `godot/scripts/renderers/agents.gd`

## Physical parking foundation (PARK-01)

Status: merged onto branch `port-park-merge` at save format 118, not landed on `dev`. The sections
down to [Current Scope](#current-scope) are the parking branch's text as it came: its save numbers
(110-116), macOS logs and timings describe that branch's builds, not this machine or this merge.
The rewrite with fresh acceptance comes before landing.

Round 1 ships building anchors, the standalone lot, deterministic edit towing, persistent parked
cars, saves and parked chunk rendering on `upstream/dev`. Agent travel is unchanged. Household
ownership and resident/freight/immigration itineraries belong to `PARK-02`, after
the shared traffic fixes. The deterministic fleet is created only by the
explicit `debug_fill_parking()` bridge command; ordinary gameplay does not populate it.

`simulation/parking.rs` owns SoA place/vehicle rows with the existing generational handle tables,
per-building/anchor source lists and signed, floor-divided 512 m chunks. Each place has one
occupant or reservation. Cars retain model, one of five colours, occupied/reserved handles and
a home point. The test fleet supplies that point in this round; resident ownership is later.

| Operation | Cost and boundary |
| --- | --- |
| Resolve, reserve, occupy, release | O(1), allocation-free; dirty capacity prepared with supply |
| Candidate query | O(9 + local candidates × quote cost), allocation-free, no candidate cap |
| Source reconciliation | O(old log old + new) per anchor source; O(old × new anchors) building dispatch, plus local tow queries |
| Off-map retry | O(W log W + W × local candidates), W local waiting cars; supply/release events |
| Parked publication | Per dirty chunk O(Pc + C log C) for Pc places and C parked cars, a copy of cached poses on the sim thread, parallel over chunks; plus one support solve per car on a place with no pose for its model. Nothing when no chunk is dirty |
| Sleeping parked cars | No simulation-tick iteration |

Queries reject straight-line distances over 400 m before quoting, and quoted walks over 400 m.
Ranking is walking seconds, driving seconds, then stable handle. Foundation towing uses Euclidean
walking estimates. `PARK-02` must use walking-route time with zebra-aware crossings, locally rebuilt
per-destination candidate caches and free-flow driving costs through `nearest_with`; retries are
scheduled events. The demand gate is a journaled counter maintained by placement mutators.
Dense-area employment must continue to work for carless households.

### Kerbside parking lanes (PARK-05)

A road edge carries `parking: RoadParking { left, right }`, its kerbs looking along the forward
direction. Each parking lane is `PARKING_LANE_WIDTH_M` (2.5 m) and is part of `edge.width`
(`Edge::cross_section_width_m`); the traffic lanes keep their widths and
`Edge::lane_group_offset` moves them half a parking lane off a one-sided parking kerb. Only a
road with a car lane may park (`parking_requires_vehicle_road`), and a kerb only with a sidewalk
on its own side (`layout_parking_needs_sidewalk`, D4): a kerb bay's walking access is on its own
side, so on a shoulder it would be supply nobody reaches. `check_layout` and
`Edge::validate_parking` hold both rules side by side, on the stroke, in place (a sidewalk cannot
be taken from under its parking lane) and on load. Splits copy the setting; no graph operation
reverses an edge. Save format 118 stores it as `network_edges.parking` (bit 0 left, bit 1 right).
A parking lane weighs 0.2 per side in `road_weight`, between a sidewalk's 0.1 and a lane
([road costs](economy.md#road-costs-road-101)).

The road builder (`ROAD-101`) has a Parking lane part, allowed only at a kerb next to a sidewalk
(`layout_parking_at_kerb`; outermost on a side with no sidewalk it is
`layout_parking_needs_sidewalk`), so at most one per side; a type's parking reaches the road tool
through `set_road_tool_parking` and travels with the stroke through preview, commit and the
refusal report like the median. Road types store it as the `parking` part kind. A clicked
Parking tile always lands at a kerb: the one on the selected slot's side, the right on a tie, the
other when that kerb already parks. Picking a part selects the slot on its kerb side, so the pick
survives the click outside that closes its popup. A dragged tile lands where it is dropped.

On the roadbed the parking lane is part of the carriageway bands: the same ground-level asphalt,
so kerbs, junction mouths, crossings and the roadbed queries need no new band kind, and the
reserved `BandKind::Parking` stays unused. The markings set it off: a solid 0.10 m line a parking
lane's width in from each parking kerb, drawn on every road with parking (an unmarked 1 + 1
street included), and the lane dividers split only the traffic lanes' share of the carriageway.
It stops where a ring entry's give-way row begins, as the dividers do (`ROAD-62`, R7d's triangles),
and it stops at the taper to a turn pocket on its side: the span ends there, the taper lying in
the transition's node piece, and a pocket edge paints no line on a kerb its pocket moves
(`Edge::pocket_moves_kerb`, D5).
The asphalt shader gets the parking sides and the median's width (half metres) in spare bits of
the lane-count channels (`road_space_color`), so the parking lane draws as the same asphalt, not
as the gravel shoulder past the traffic lanes' width, and wheel paths stay in the traffic lanes.
That repack is the one product exception to `dev`, accepted as a fix (D6: the far carriageway of a
divided road drew as shoulder). Measured on real meshes
(`the_repack_moves_only_a_divided_roads_green_channel`): a road with no median and no parking is
bit for bit what it was; a divided road moves green alone, on every carriageway vertex of its
spans, by the median's width in half metres, with red, blue and alpha untouched and junction
asphalt (zero alpha) unchanged. The R8 probe holds no median, so it does not see this.

Kerbside bays are places in the shared pool (`parking/kerb.rs`), one `PlaceSource::Kerb { edge,
right }` per parking lane, saved with the other places. Each is 6 m long (`KERB_BAY_LENGTH_M`)
and 2.5 m wide, centred half a parking lane in from the carriageway's edge of the span's
`RoadSection` bands, never `edge.width` or mesh triangles. Bays run end to end, whole bays only,
centred in each free stretch. A stretch starts 5 m past the span's first section or its start
crossing strip and ends 5 m before the last section or its end crossing strip, because Finnish
rules forbid parking within 5 m of a junction or crossing. A turn pocket takes the kerb it moves
(D5): a pocket edge holds no bays on that side, a divided road's left pocket, which takes the
median, moves neither kerb, and the far kerb keeps its bays. The base road's bays end 5 m short of
the span's end, so short of the taper. A pocket added under existing bays goes through the normal
edit path: the rebuilt owners reconcile, the bays go and their cars are towed; undo returns them.
A building's driveways and its
street-entered lot bays cut a gap of their half width plus 1 m each side on their own kerb; a
building without driveway anchors keeps a 3 m drive clear opposite its main entrance, where its
deliveries and drop-offs pull in, so parked cars never wall off a frontage
(`KerbBlocker`, indexed per
building and per edge, derived and rebuilt by the load reconcile). Cars park facing the traffic
beside them: a two-way road's left kerb faces backward, and a one-way road's lanes serve both
kerbs. Access is the bay's own side (`SameSideOnly`) when its direction has lanes, both sides
otherwise, entered from the lane beside the bay's rear along the bay's swept path.
`SimCore::reconcile_parking_frontages` reconciles the kerbs of every touched edge, and of every
edge whose blockers a building change moved, after the lots, through `replace_source`: an
unmoved bay keeps its car, and a removed one tows it. Load validation rejects a kerb place
whose edge is missing, deleted or has no parking on that side.

Road Properties changes parking in place (`set_edge_cross_section`'s `parking` code, checked by
`Edge::validate_cross_section` with the other in-place rules): a picked road type brings its
parking, and the Parking lane row's Left and Right boxes switch one kerb. The edit rebuilds the
roadbed, and its publication reconciles the kerb's bays like any road edit.

Still to come: bay paint and signs (`PARK-04`). The local `parking-kerbside-ref` branch
preserves an earlier superseded implementation for reference. Pull-out uses
`lane_attach_slot_clear` plus `claim_lane_entry` (the `TRAFFIC-11` hold) and pull-in uses
`try_network_detach`, as for lot bays.

### Anchor lots, edits and rendering

Each fitting `parking` anchor is public supply. Its `position` is the bay's entry edge and the bay
runs `length_m` along `forward`, as asset validation and the asset editor draw it. Existing
building transforms, paving, structure footprints and validated frontage driveways are reused. A
bay whose entry edge lies on the lot's street edge (within 0.6 m) and points into the lot opens
straight onto the street: the car turns in from the road and needs no `driveway` anchor. Other
bays use fixed driveway–aisle–bay paths that check full 1.8 × 4.5 m translated rectangles and
conservative 4.85 m turn envelopes squared to the bay, so a lot on an off-axis road keeps the
envelope inside its yard. Coverage ignores an uncovered remainder under 0.01 m², the float
slivers left where the yard abuts the road surface along an off-axis seam. The envelope is required at the driveway and at the aisle
corner only when the path bends there by more than 30°; through a slighter bend the two
corridors, each extended 2.25 m past the corner, already cover the car's sweep. Paved support
allows 5 cm at each bay edge. Rejections carry plain-language reasons naming the failing part
(the way to the front of the spot, the way into it, or either turn) and its cause (unpaved,
structure or another bay), with the room that part needs. They are shown in the asset editor,
which also draws the failing shape for the selected spot and lists each rejected spot as a
validation issue that blocks export, and in the building inspector. Unsupported layouts
publish diagnostics and no bay. `loading_bay` anchors are excluded. Bays painted side by side
may share an edge: the bay-on-bay check uses the same 5 cm inset. Mesh parts no taller than
0.15 m (`GROUND_DRESSING_MAX_HEIGHT_M`: paint, kerbs) are ground dressing and block neither
cars nor yard hedges.

The bundled lots follow Finnish practice: 2.5 × 5 m bays at right angles to a central 7 m
two-way aisle, painted separators, kerb stones at the bay heads, one two-socket engine-heater
post per pair of bays on the strip behind the kerb, and a P sign at the entrance. The small lot
(20 × 20 m) holds 12 bays and the larger one (20 × 30 m) 20. Both are explicitly placed sites
with zero workers/households and no economy profile; their site surfaces render the asphalt.

Coverage retains exact ordered polygon differences, filtering support polygons by AABB before
any overlay. With S support vertices and K overlapping contours, each footprint costs
O(S + K overlay cost); no overlap performs no overlay or contour allocation. Bays, swept paths
and turn envelopes share the filter. Structure checks already use the site-cell index and
`PolygonFootprint` AABB rejection before exact overlay, with an early exit once blocked.
For A bays and D driveways the bound is O(A² D + A D (S + L + K overlay cost)), L local
structure vertices. This is edit/load work and has no dependency on total city buildings.

Road and building edits reconcile affected building sources through existing SimCore publication
and edit hooks. Splits/frontage changes, road demolition/detachment, asset reload and undo all
revalidate lots through the existing site journal and site index. Graph, topology,
road planner and network-save files have no parking diff. Stable handles order towing; removed
claims are cancelled and occupied places tow to free supply near home, else store cars off-map.
Supply/release events retry local home chunks, including a newly restored lot. Undo reconciles
live occupancy without restoring an old fleet snapshot or refusing an edit.

Dirty chunks publish immutable versioned model/colour buffers, stable car IDs and 12-float
MultiMesh transforms. Unchanged chunks send nothing. Shared `vehicle_resources.gd` keeps moving
meshes/materials identical; parked meshes omit lamps/beams.

**The render path (`PARK-02`).** Moving cars and parked chunks reach Godot from one
`RenderSnapshot`, and the render path takes no core lock for them:

- The sim thread rebuilds dirty chunks while it builds the snapshot, in the same lock as the
  agent poses (`SimCore::publish_parked_chunks`), and the snapshot lists every chunk Godot has not
  acknowledged as shared `Arc` buffers. `get_vehicle_frame` reads cars and chunks under one read
  of that snapshot, so a car crossing a bay's edge is in exactly one of them on every frame Godot
  draws, including when it skips snapshots: a skipped chunk stays listed. There is no `busy`
  answer and no epoch to compare; this replaced the branch's `try_lock_core` frame, which froze
  every moving car whenever the simulation held the lock.
- Godot answers through atomics, never the lock. `acknowledge_parked_cars(epoch, serial)` says it
  installed everything up to a publication serial; an answer from a replaced world is dropped and
  a serial only rises, so a late answer cannot discard a newer chunk. A renderer reconnect sets a
  flag, and the next snapshot lists every chunk once more. The `parked` key is absent from the
  frame when nothing changed, and the listed `Arc` vector is rebuilt only when a chunk or an
  acknowledgement changes it, `O(listed chunks)` pointer copies.
- One off-lane path still try-locks the core: `smooth_car_render_data` re-grounds interpolated
  off-lane cars. It is `dev`'s, and failing it draws the snapshot's own validated pose, so the car
  still updates.

**A parked car stands on its own street or lot.** `SimCore::parked_ground_pose` reads its own
surface, not `dev`'s visible one: a bay on the ground takes the road on the ground
(`grounded_height_at`) or the graded ground, so a deck overhead never lifts it; a kerb bay on a
bridge or in a tunnel takes the road piece nearest its own height (`height_nearest_at`), so it
stands on its deck or tunnel floor. Same contacts and one gather, so the cost of the exact solve;
moving cars keep `dev`'s solve. A car in its pull-in or pull-out under a deck is such a moving
car, and takes `dev`'s top surface for those seconds.

**Parked poses are cached.** Each place keeps one grounded 12-float pose and the model it was
solved for (`Place::ground`, 49 bytes a bay, 4.9 MB at 100k bays), derived and never saved. A
pose belongs to a model, because each model has its own contacts, so a car of another model
arriving at a bay takes one solve, and a rebuild is otherwise a copy: a custody change solves
nothing for the other cars of its chunk. A full table of the five models would remove that one
solve per arrival for 241 bytes a bay and five solves per bay at every publication; an arrival
costs a car no more than one frame of off-lane grounding, so it is not worth that.

A pose goes stale only if the ground under it moves, and a bay is not re-added for that: the
pose key of `replace_source` holds the bay's own position and heading, not what a neighbouring
piece, pad or base edit does to the surface its contacts read. So invalidation does not rely on
reconcile. The four records of the graded edit journal (`record_graded_road_edit`, `_site_edit`,
`_base_edit`, `_reset`), which every graded ground change passes, forget the poses within the
change's reach plus the longest car's contacts and dirty the chunks with a car there. Road changes
cover every piece, bridges and tunnels too; a reset covers a load, a new world and a dense terrain
undo. Two paths outside the journal call the same invalidation: the undo of a bulldoze, whose
restored site records no site change, and a new car profile at mesh load. The next snapshot
solves what was forgotten, so the first publication after a load rebuilds every parked pose.
The resident moving/parked identity handoff is implemented in round 2a below; external vehicles remain for round 2b.

Save format **110** (upstream 109 + 1, no migration) stores handle generations, building-anchor
places, cars, occupancy and reservations. Edge rules/lineage are absent. Load validates reciprocal
custody and building owners, rebuilding source/chunk/waiting/render indices. Detached explicit
sites retain their rigid pose so losing a driveway road disables supply without breaking saves.
The pinned Kuopio fixture is re-recorded for the combined parking/gridlock schema.

### PARK-01 acceptance (round 1b, 2026-10-31)

Fresh runs use Rust 1.98.1, upstream Godot 4.7.2, macOS/Apple M2 Pro, the supplied `TMPDIR`,
`SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk` and `CARGO_INCREMENTAL=0`.
Production code is `bdf31d2b`; `0e64f778` corrects the bridge fixture's snapped demolition target.
The benchmark's surface-publication setup correction is test-only. Earlier round-1 acceptance
was against `road` with kerbside and does not validate this scope.

```sh
cd rust
cargo test --lib parking
cargo test --lib
cargo clippy --all-targets
cargo doc --no-deps
cargo test --release --lib --no-run
cargo build --release
```

- Targeted parking: **10 passed, 1 ignored**, both optimized-debug and release. The unchanged
  `network_undo_falls_back_to_owner_recompile_when_surface_capture_is_dirty` release test passes.
- Full `cargo test --lib`: **1,858 passed, 180 ignored, 2 failed**, 417.36 s. The failures are
  `nodes::sim::core::tests::road_reference_probe::reference_junction_products_match_the_probe`
  and `simulation::network::roadbed::node::tests::probe::roadbed_fixture_products_match_the_probe`.
  Both independently reproduce on a freshly compiled `upstream/dev` (`727dd7f2`) release control.
- Clippy: **zero warnings**. Rustdoc: **zero missing-doc warnings**, ten private/unresolved link
  warnings whose messages exactly match a fresh `upstream/dev` documentation build.
- The existing ignored `record_kuopio_reference_save` recorder passes; the new fixture has no
  edge parking sources. Full-suite load/save and continuation tests pass with it.
- Seven Godot scripts each pass in their own isolated-profile process with zero `ERROR:` or
  `SCRIPT ERROR:` lines: `parking_test`, `building_asset_reload_test`,
  `network_tool_chunk_renderer_test`, `road_benchmark_metrics_test`, `road_preview_metrics_test`,
  `road_junction_preview_test`, `road_preview_stream_test`. The parking bridge compares all 25
  moving vehicle mesh variants bit-for-bit with the retained pre-extraction source and checks
  the parked meshes omit lamps/beams. Its first demolition attempt used the unsnapped input
  coordinate; the corrected test uses the published snapped centre and passes towing/undo.

```sh
# Unique project identity isolates user://, including on macOS.
perl -e 'alarm 900; exec @ARGV' godot --headless --path "$ISOLATED_PROJECT" --import
rm -f godot/bin/libmetrum_rise.dylib
cp "$RELEASE_LIBRARY" godot/bin/libmetrum_rise.dylib
perl -e 'alarm 900; exec @ARGV' godot --headless --path "$ISOLATED_PROJECT" --script "res://tests/${TEST}.gd"
python3 tools/capture_parking.py --headless --output "$ARTIFACTS/capture-setup"
python3 tools/capture_parking.py
```

The single windowed capture writes `captures/parking/lot-0900.png`. It was opened and inspected:
six cars on the asphalt, clear driveway/aisle, no kerbside cars, zero stored cars and no asset
diagnostics. Headless setup also passes. All captures/logs remain local and uncommitted; the
player's saves and mods were not changed. Imported metadata churn was removed after the runs.

#### Short PERF-06 sanity pair

One unprofiled release pair, control first, eight Rayon workers, inherited CPU affinity,
no concurrent build, correctness run or Godot process:

```sh
RAYON_NUM_THREADS=8 "$TEST_BINARY" simulation::economy::agents::tick::contract::benchmark_agent_tick_contract --exact --ignored --nocapture --test-threads=1
```

Control `upstream/dev` `727dd7f2`, SHA-256
`21093e8644ca9693302a151ea7bb6d4ccee4d4aeff721e37c2c68caa40b95073`;
parking production code `bdf31d2b`, SHA-256
`ec936ba5d68278698d67900e2fe39f119c1762d2dbba4048e2f332d8cf5d670f`.
Per-tier work counts match exactly; agent-row work is 3× and lane work stays constant,
within the 3.6× bound. These single timing samples establish no speedup.

| Mode | Agents | dev, ms | parking, ms |
| --- | ---: | ---: | ---: |
| network | 10,000 | 0.483209 | 0.487125 |
| network | 30,000 | 0.973625 | 0.844292 |
| access egress | 10,000 | 0.824167 | 0.836167 |
| access egress | 30,000 | 1.689333 | 1.803084 |
| access ingress | 10,000 | 0.517625 | 0.380958 |
| access ingress | 30,000 | 0.532375 | 0.667667 |

#### Populated lot/anchor publication locality

The existing 800 m populated neighbourhood stays fixed; remote buildings, parcels, housed
agents and 480 m road blocks grow through 100k. Twelve real six-bay lots front a separate fixed
600 m local road. Setup, road surface publication and initial supply publication are excluded
from timing. Each tier measures nine repeated frontage reconciliations, nine site-journal
publications and 108 serial individual `lot_bays` rebuilds. Full bay geometry matches across
tiers, and bay geometry plus stable handles match after every reconciliation/publication.
Diagnostics are checked for the measured lots; unsupported anchors on background fixture
assets remain diagnosed and supply no bays, as required by the validator.

```sh
cargo test --release --lib --no-run
METRUM_LOCALITY_TIERS=0,1000,10000,100000 RAYON_NUM_THREADS=8 "$TEST_BINARY" nodes::sim::core::populated_city::commit_locality::benchmark_parking_locality --exact --ignored --nocapture --test-threads=1
```

Unprofiled, eight workers, inherited CPU affinity, no competing builds/tests/Godot. Production
code is `bdf31d2b` plus this revision's test-only setup correction; release test SHA-256
`ccc1a86f4eb4061f50e8fe9e96d12f021126f5ba6a9875a34b920a82790e0d9a`. The final run passes in 5.84 s including setup. Initial harness attempts
stopped before measurement: the new frontage needed surface publication, and the diagnostic
assertion needed to cover the twelve measured lots rather than unrelated background content.

| Background | Buildings / parcels | Agents | Roads | Setup, s | Per-lot median, ms | Maximum lot, ms | 12-lot frontage rebuild, ms | 12-lot site publication, ms |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | 1,276 / 1,264 | 0 | 84 | 0.397 | 0.152916 | 0.222000 | 0.456542 | 0.457625 |
| 1,000 | 2,276 / 2,264 | 1,000 | 109 | 0.375 | 0.147875 | 0.213250 | 0.450959 | 0.444667 |
| 10,000 | 11,276 / 11,264 | 10,000 | 334 | 0.714 | 0.142042 | 0.206209 | 0.458541 | 0.426667 |
| 100,000 | 101,276 / 101,264 | 100,000 | 2,584 | 4.186 | 0.144125 | 0.212458 | 0.497084 | 0.463541 |

All tiers publish the same 72 measured places. Individual six-bay validation stays below
0.223 ms even at 100k background; twelve-lot publication stays below 0.50 ms. Repeated cost
follows the fixed neighbourhood, while setup remains separate. No city-wide parking rebuild
or per-tick parking pass is introduced. Logs and immutable binaries remain under `$ARTIFACTS`.

#### Shared-file formatting audit

Every changed pre-existing source file was checked against `upstream/dev`: no formatting-only
hunks remain. Rust paths below are relative to `rust/src/`; the populated-city change is an
appended parking benchmark only. No formatter was run on these shared files.

| Shared file | Result |
| --- | --- |
| `godot/scenes/Main.tscn` | Semantic changes only |
| `godot/scripts/core/mod_pack_config.gd` | Semantic changes only |
| `godot/scripts/renderers/agents.gd` | Semantic changes only |
| `nodes/sim/core.rs` | Semantic changes only |
| `nodes/sim/core/populated_city/commit_locality.rs` | Semantic changes only |
| `nodes/sim/core/road_tool_context.rs` | Semantic changes only |
| `nodes/sim/core/state.rs` | Semantic changes only |
| `nodes/sim/core/tests.rs` | Semantic changes only |
| `nodes/sim/editing.rs` | Semantic changes only |
| `nodes/sim/save_load.rs` | Semantic changes only |
| `nodes/sim/undo.rs` | Semantic changes only |
| `nodes/sim/world_definition.rs` | Semantic changes only |
| `nodes/simulation_node.rs` | Semantic changes only |
| `simulation/buildings/allocator/lifecycle.rs` | Semantic changes only |
| `simulation/buildings/allocator/mod.rs` | Semantic changes only |
| `simulation/buildings/allocator/placement.rs` | Semantic changes only |
| `simulation/buildings/allocator/site/query.rs` | Semantic changes only |
| `simulation/mod.rs` | Semantic changes only |
| `simulation/save/mod.rs` | Semantic changes only |
| `simulation/save/schema.rs` | Semantic changes only |
| `simulation/save/tests.rs` | Semantic changes only |
| `simulation/save/tests/cell_zoning.rs` | Semantic changes only |
| `simulation/save/world.rs` | Semantic changes only |

`simulation/network/graph/data.rs`, `simulation/network/road_edit.rs`,
`simulation/network/road_edit/validation.rs`, `simulation/network/topology.rs`,
`simulation/save/network.rs`, `nodes/sim/render/vehicle_ground.rs` and the Godot Road Properties
script have **zero diff** against `upstream/dev`. No parking hook remains in network-owned files.
SimCore's road-tool publication, demolition/node edits and undo call
`reconcile_parking_frontages` to keep lot supply and live cars consistent with compiled frontage.

## Resident parking trips (PARK-02 round 2a)

Pass p06 exposes each rejected anchor and its exact reason in the asset editor and the
placed building inspector, alongside the building's valid-place count. Both call the same
Rust bay/support/structure/swept-path validator. The editor validates the asset-local path;
only the placed inspector can validate the road-to-driveway segment and neighbouring structures.


Residents now retain one household vehicle handle through an outing. The zoning registry's
classification of `Building.zone_profile_runtime_id` determines ownership: low density 100%,
medium 75%, high 50%. Explicit homes without a zoning profile use low density. The saved mobility
seed and stable household handle select the ownership bucket deterministically; dense storage
indices and worker scheduling do not participate. Household removal releases claims and deletes
the vehicle. Children cannot drive, including in a household with an unused car.

New cars reserve the first free fitting home catchment place before driving in, ordered by walking-route seconds,
then fixed free-flow bay maneuver time, then place handle. A single destination list cannot
order equal walks by an origin-dependent road trip; the driveway/bay maneuver is its fixed drive
tie-break. The complete departure quote separately prices the actual free-flow road route.
Without a place the car stays off-map and unavailable. Supply/release events wake only nearby
waiting home chunks. Resident recovery uses the same walking catchments; the explicit foundation
test fleet retains its original geometric recovery.

Each destination publishes a candidate list from two bounded directed pedestrian-lane searches.
Connector lengths and zebra links participate; 400 m means `400 / AGENT_WALK_SPEED_MS` seconds.
Euclidean distance only rejects impossible candidates before the walking search. Existing site
cells find affected doors after building/place/footpath publication. A new placement is included
immediately, without rebuilding the economy's lazy vacancy/chunk index. Occupancy never rebuilds
these lists, and departures never route-query each candidate.

| Operation | Bound / retained storage |
| --- | --- |
| Local catchment rebuild | O(local doors × ((local lanes + connectors) log local lanes + local places log local places)); independent doors use Rayon |
| Destination choice | O(k), first free fitting entry, no route query or allocation |
| Complete departure quote | O(k + Q), three exact leg quotes after selecting one place |
| Ordinary movement | O(1) additional phase/access work per agent |
| Eligibility / ordered resource phase | O(agents + transitions × Q); exclusive resource commits use deterministic dense agent order |
| Sleeping parked vehicles | No simulation-tick vehicle iteration |
| Candidate storage at 100k nonempty doors | Approximately 9.6 MB map/header storage + 1.6 MB per mean candidate capacity + 0.4 MB per mean road-component count + 1.6 MB per mean distinct car target count; about 38 MB at 12/2/4 including Arc headers; empty lists omitted |

`quote_parking_trip` proves walk-to-car, drive and walk-to-door before departure, with both swept
maneuvers included in the drive quote. The walk to the actual car may exceed 400 m; exact
pedestrian reachability determines that leg. The 400 m bound applies to destination parking.
Returning home releases custody even if the car remains at a distant bay. The walking tie-break and 180-second
`CAR_MODE_CHOICE_OVERHEAD_S` remain unchanged. `reserve_trip` acquires exclusive custody and the
destination in the ordered transition phase. A failed claim walks or waits for a saved timing-wheel
event. Route-buffer preparation uses the next eligible event slot; unavailable routes use 30
simulation seconds. No per-agent retry countdown polls for supply.

The itinerary is `WalkToCar → PullOut → Drive → PullIn → WalkToDoor` inside the ordinary
access/network/access FSM. Pull-out uses `lane_attach_slot_clear` and `claim_lane_entry`, including
`ACCESS_LANE_ATTACH_PENDING`. Pull-in uses the existing `try_network_detach`. Both use the lot's
driveway curb, followed by the retained four-point swept path outside the network. No resident
car-mode leg finishes at a door; arrival asserts this in debug builds. The
`TRAFFIC-13` last-visit rule and `TRAFFIC-15` lane-end reachability are authoritative and
now apply to these driving legs through the shared detach function.

Removing a reserved bay cancels its claim. A bounded three-handle reverse link retains disabled
endpoint geometry only while its borrower needs it; it is excluded from usable supply. Every
recovery that rewrites the itinerary drops the links it no longer names, in O(1)
(`retain_retired_places`), so a superseded destination gives its slot back. One edit can still
tow a borrowed car through more bays than there are slots with no recovery between; with every
slot taken the newest bay goes unretained, since no trip names a bay it was towed through and a
destination recovery replaces anyway. Evicting the oldest instead would drop the bay the walker
is heading for. Removal never panics on a full set. Recovery
walks to the car's actual new place, replans from its actual lane position, or reverses an unfinished
pull-in to the driveway before choosing replacement supply. It never moves a travelling car by
towing. The owned-car watchdog requests this ordered recovery rather than teleporting.

A door with no foot lane (a car-only building, `ROAD-107` D1) reaches nothing on foot, so its own
bays are always candidates (D4), at the straight walk across the lot: `BuildingEntrance::lot_door`
marks a hosted bay's access, `in_lot_candidate` quotes door to bay or bay to bay when no lane
joins them (O(1), only after the lane quote fails), and the leg has no lanes, which is how
`walk_across_lot` knows it: at walking pace from where the walker stands, then the usual arrival
or `ready`. Cost: O(1) more per place in a catchment rebuild, nothing on a door with foot lanes.
A car-only building without a bay of its own has no catchment: its car stays off the map and its
residents stay in, retrying every 30 s, the console saying why. That is a city without supply,
`PARK-03`'s question.

Moving and parked cars come from one snapshot (see the render path above), so neither renderer
can run ahead of a transition. Model and colour persist across the switch. Origin occupancy is released after pull-out reaches the network; destination reservation
becomes occupancy before the last walk. Save version **110** persists ownership, custody, trip phase,
claims, pending household/release events and endpoint recovery links. Loading rebuilds catchments,
access endpoints and the route buffers' saved logical preparation budgets; allocator spare capacity
cannot change a departure. No migration is provided.

Employment, shopping and service visits quote the same physical itinerary as departure. Jobs
reuse the existing component buckets and shared exact CCH quotes. Publication caches each door's
union of bay road components, distinct car arrival targets and maximum node slack; hourly searches
never route-query all its parking candidates. Batch keys distinguish actual household cars. The
bounded home/work/mode cache stores the chosen source/destination handles and geometry stamp;
occupancy or a local bay move replaces that value without adding availability-version keys.
Shopping and service folds share immutable component arrays without per-entry allocation. Their
walking proofs stay O(1); car fallbacks prove an outward itinerary and a hypothetical return from
the proposed destination bay, with the departure bay available on return. Accepted shopping-leg
timeouts use the same complete physical cost. Save version 110 includes these quote stamps and
rolling destination memberships; the unshipped format changes together, without a migration.

Pass p06 move-ins reserve one free fitting home-catchment place at paced departure, then use
`Drive → PullIn → WalkToDoor`. With no free place they walk from the border and keep the car
off-map. Carless households use the same pedestrian arrival. Ownership uses the saved seed and
stable carrier identity before a household record exists; adoption preserves that decision and
the original vehicle. Save version 111 adds this in-flight ownership decision, with no migration.

Stored cars never become occupied places at supply publication or load. New lots, vacated bays,
cancelled reservations and towing queue deduplicated chunk events. Each event considers at most
eight cars in stable handle order across the nine neighbouring home chunks, continuing at the
next eligible timing-wheel slot until that finite wake cycle ends. Failed cars sleep until the
next supply event. Ordered sets reuse the existing home-chunk index; the continuation cursor
lives in the existing parking chunk, and both cursor and scheduled event survive saves.

Per event the bound is O(8 × (9 log W + k + BQ) + log S): W local waiting cars, k cached home
candidates, B connected external borders, Q one exact route query, S scheduled events. A car
selects one cached candidate before trying borders in stable order, reserves before spawning,
and uses `Drive → PullIn`. The delivery-only carrier retires at the bay through the normal
agent lifecycle, without entering a building. The household cannot use the car until pull-in
commits occupancy. Supply release remains O(1), allocation-free. The shared logistics topology
refresh is unchanged O(nodes + edges) once per revision, prepared before delivery events;
there is no additional spatial or connectivity index. Save version 111 also stores delivery
custody and wake cursors; SQLite stores opaque event payloads as signed bit patterns so negative
chunk coordinates round-trip without numeric overflow.

Only new supply wakes waiting cars: an added or reactivated bay wakes its own chunk, and a
catchment rebuild wakes the doors whose list of reachable places changed. An unchanged bay or
catchment wakes nothing, so the full frontage reconcile and catchment rebuild after a load
schedule no arrivals the uninterrupted game did not. A household's shopping car flag changes
only at its hourly recount, while car custody changes every tick; save version **112** stores
the flag instead of rebuilding it from the agents at load. Both keep a loaded game saving the
same bytes and continuing the same census as the uninterrupted one.

A place's cached driveway access keeps its edge, side, stations and positions. Its lane ids are
derived live from that edge's current lanes in O(lanes on the edge). A terrain edit or a dense
undo renumbers every lane while republishing only the places near the edit, so cached lane ids
would point at other lanes or past the end of the lane table.

Round 2b retains freight and wider watchdog/lifecycle changes. Resident
household removal and resident edit/watchdog recovery are already included here.

### Traffic integration after round 2a (PARK-02, 2026-11-01)

Merged `upstream/dev` `c2d49902` into parking parent `6531b187`, preserving both histories.
The five conflicts were the Kuopio SQLite fixture, `agents/data.rs`, `agents/tests.rs`,
`tick/access.rs` and `save/schema.rs`. Agent state retains both parking retries and the gridlock
checker; both test modules and both access exports remain. Save version stays **110**, exactly
`dev` 109 + 1, without migration. An in-memory schema comparison verifies that every table and
column from both parents remains, including `parking_state` and `gridlock_state`. The existing
`record_kuopio_reference_save` recorder passes; the pinned version-110 fixture contains both rows.

The shared `try_network_detach` keeps the upstream lane-end reach bound and final-route-visit
rule unchanged. Gridlock detours now receive the same parking entrance override as ordinary
network replans, preserving the reservation and driveway station instead of routing to the door.
Replacement also clamps the saved route preparation budget. Additional work is O(1),
allocation-free per attempted detour; the existing one route query and route allocation remain.
No per-tick parked/stored-car pass or new spatial index is added.

Five new integration regressions pass:

- `parking_pull_in_reaches_the_driveway_at_the_lane_end`: the real resident itinerary detaches
  at the 500 m lane-end driveway and retains its destination claim.
- `parking_pull_in_waits_for_the_last_destination_edge_visit`: the real itinerary drives the
  U-turn loop before pulling in on its second visit, at the planned station.
- `parking_gridlock_excludes_parked_and_stored_cars`: both vehicles stay out of lane buckets,
  waiting-head stamps and cycle cars across consecutive checks.
- `parking_gridlock_treats_a_held_pull_out_as_an_entry_wait`: an actually blocked handoff has
  `ACCESS_LANE_ATTACH_PENDING`, no current lane and no gridlock across consecutive checks.
  The swept pull-out has completed, so its saved logical phase is `Drive` during this entry wait.
- `parking_gridlock_detour_keeps_the_reserved_driveway_and_never_teleports`: a confirmed cycle
  detours the parking-bound head, retaining its borrower and claim, unchanged lane/position bits
  at replacement, bounded route budget and ordinary movement to occupancy of the reserved bay.
  Its driveway is 80 m away from the target door, exposing an incorrect door-based replan.

The isolated loop fixture services only the existing zero-delay route-preparation requests at
its next tick boundary; it does not use a full gameplay constructor or change retry semantics.
Fresh optimized-debug parking tests: **30 passed, 2 ignored**, 1.16 s; fresh release parking tests:
**30 passed, 2 ignored**, 0.86 s. Full `cargo test --lib`: **1,885 passed, 186 ignored, 2 failed**,
359.78 s, the two reference probes compared with plain `dev` below. Fresh
`cargo clippy --all-targets`: zero warnings. `cargo doc --no-deps`: zero missing-doc diagnostics
and ten link warnings, also compared below. Builds use Rust 1.98.1, macOS SDK 26.5,
`CARGO_INCREMENTAL=0`, the supplied `TMPDIR` and eight Rayon workers.

Each of these eleven Godot 4.7.2 scripts passes in its own isolated-profile headless process,
launched through `perl -e 'alarm 900; exec @ARGV' godot ...`, with zero `ERROR:` or
`SCRIPT ERROR:` lines: `parking_test`, `agent_inspector_test`, `vehicle_ground_support_test`,
`building_asset_reload_test`, `network_tool_chunk_renderer_test`, `road_benchmark_metrics_test`,
`road_preview_metrics_test`, `road_junction_preview_test`, `road_preview_stream_test`,
`road_rejection_report_test` and `gridlock_notice_test`. The parking resource comparison uses
`e84b1136^`'s pre-extraction renderer. The gridlock fixture comes from the existing
`record_gridlock_saturation_fixture` recorder. The isolated import has zero errors. The old
library is removed before copying the new release library; generated import metadata is removed.
Logs and immutable release executables remain local under `$ARTIFACTS`.

Fresh plain `upstream/dev` `c2d49902` was built in a detached control worktree with the same
profiles and environment. Its `cargo test --lib` result is **1,855 passed, 184 ignored, 2 failed**,
284.80 s. Both builds fail exactly these same probes:

- `nodes::sim::core::tests::road_reference_probe::reference_junction_products_match_the_probe`
- `simulation::network::roadbed::node::tests::probe::roadbed_fixture_products_match_the_probe`

Plain `dev` also has zero clippy warnings and zero missing-doc diagnostics. Its ten rustdoc
link-warning messages match the parking build exactly. These are fresh comparisons, not the
historical round-2a results below; the unrelated probe expectations are unchanged.

The fresh commands in each crate are `cargo test --lib`, `cargo clippy --all-targets` and
`cargo doc --no-deps`; parking additionally runs `cargo test --lib parking`, the existing ignored
Kuopio recorder, `cargo build --release` and the release parking tests. Both release test
executables are built with `cargo test --release --lib --no-run` and retained before switching
builds. One unprofiled PERF-06 pair runs control first, parking second, serially, with eight Rayon
workers, inherited CPU affinity and no concurrent build, correctness test or Godot process:

```sh
RAYON_NUM_THREADS=8 "$TEST_BINARY" simulation::economy::agents::tick::contract::benchmark_agent_tick_contract --exact --ignored --nocapture --test-threads=1
```

Release test SHA-256: `dev` `5775e886182ae3f0fb3821b778f0ba2273e4065ddf650f573e98ca3068c96426`; parking `17c8fce914d2f2573cf6ea8f0c9813c68040ebe29ab8e9a50c3e6e17257c3476`.

| Mode | Agents | dev, ms | parking, ms |
| --- | ---: | ---: | ---: |
| network | 10,000 | 0.767959 | 0.672708 |
| network | 30,000 | 1.034875 | 1.164667 |
| access_egress | 10,000 | 0.859042 | 1.016458 |
| access_egress | 30,000 | 1.865333 | 2.135333 |
| access_ingress | 10,000 | 0.475750 | 0.516000 |
| access_ingress | 30,000 | 0.736459 | 0.775750 |

All common work counts match exactly. Agent-row counts grow exactly 3×; frontage lane work stays
at 16 and the zero-count phases remain zero. The two parking-only eligibility/transition passes
also grow exactly 3×. Every phase satisfies the 3.6× bound and repeated fixtures have identical
counts. These six single timing samples are a sanity check and establish no speedup.

Round 2b brief: the shared detach/gridlock dependencies are merged on `feat/parking`; retain the
last-visit/lane-end rules, held-entry exclusion, reserved driveway override and route-budget clamp.
Freight/immigration custody, outside-door deliveries and their wider lifecycle/watchdog work remain
for round 2b. The earlier PERF-03…05 economy counter debt remains open; this short PERF-06 pair
does not revalidate those whole-economy measurements. Keep the two independently reproduced
reference-probe failures separate from parking regressions.

### Round 2a verification (2026-11-01)

The control is round 1b `d56e9826`; the final production implementation is `d5d93371`.
Both release test binaries use the same Cargo profile, the supplied `TMPDIR`, macOS SDK 26.5,
`CARGO_INCREMENTAL=0`, eight Rayon workers and inherited CPU affinity on the Apple M2 Pro.
Setup/snapshot generation is separate from timing. Logs, binaries and snapshots remain local
under `$ARTIFACTS`; images remain uncommitted in `captures/parking/`.

```sh
export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk
export CARGO_INCREMENTAL=0 RAYON_NUM_THREADS=8
cd rust
cargo test --lib parking
cargo test --lib record_kuopio_reference_save -- --ignored --nocapture
cargo test --lib
cargo clippy --all-targets
cargo doc --no-deps
cargo test --release --lib --no-run
cargo build --release
```

- Parking regressions: **25 passed, 2 ignored**, optimized-debug 0.44 s and release 0.37 s.
  They cover all five legs, one borrower, conservation, failed claims, removed reservations,
  walk-time/local rebuilds, long walks to the actual car, SQLite save/load at every phase,
  shared economy quotes and exactly-once handoff rendering.
- Full suite: **1,873 passed, 181 ignored, 2 failed**, 359.40 s. Only the pre-existing
  `reference_junction_products_match_the_probe` and `roadbed_fixture_products_match_the_probe`
  fail. The later lint cleanup changes only an allowance, test type alias and formatting.
- Fresh clippy after that cleanup: **zero warnings**. Rustdoc: **zero missing-doc warnings**,
  the same ten pre-existing private/unresolved-link warnings. The separate release deferred
  quote-publication regression also passes.
- The existing reference-save recorder passes; the committed fixture is version 110 and
  includes the final destination/commute-cache schema. No migration is written.

#### Matched agent timing and economy contract

Release test SHA-256: control `ccc1a86f4eb4061f50e8fe9e96d12f021126f5ba6a9875a34b920a82790e0d9a`; parking `0a2721f3762f27c9324301445fdcfecedb2def4ad9ad07cac6d1b1bb9646615c`.
The 21 pairs alternate control/parking and parking/control; no build, Godot process or other
correctness test runs during timing. The command in each half is:

```sh
RAYON_NUM_THREADS=8 "$TEST_BINARY" simulation::economy::agents::tick::contract::benchmark_agent_tick_contract --exact --ignored --nocapture --test-threads=1
```

| PERF-06 state | Agents | Control median, ms | Parking median, ms | Control IQR, ms | Parking IQR, ms |
| --- | ---: | ---: | ---: | --- | --- |
| network | 10000 | 0.516000 | 0.599584 | 0.486750–0.529417 | 0.568250–0.658459 |
| network | 30000 | 1.050583 | 1.142125 | 0.969333–1.099708 | 1.021958–1.195875 |
| access_egress | 10000 | 0.774500 | 0.970666 | 0.730167–0.810292 | 0.922458–1.018000 |
| access_egress | 30000 | 1.681417 | 2.254958 | 1.650167–1.703416 | 2.222625–2.317958 |
| access_ingress | 10000 | 0.431041 | 0.550125 | 0.394459–0.496083 | 0.523708–0.609250 |
| access_ingress | 30000 | 0.732458 | 0.871000 | 0.662291–0.774500 | 0.786750–0.972625 |

Every pair repeats the same work counts. All old counters match the control. Two additional
states, parking eligibility and ordered transition classification, each visit N agent rows;
they scale exactly 3× from 10k to 30k. All PERF-06 states remain within the 3.6× bound. This
measures ordinary network/access rows; complete trip quotes are exercised by the economy run.

```sh
python3 tools/perf_contract.py --binary "$TEST_BINARY" --prepare --artifacts "$ARTIFACTS/after-economy"
python3 tools/perf_contract.py --binary "$TEST_BINARY" --artifacts "$ARTIFACTS/after-economy"
```

Both sides freshly prepare/verify healthy and fiscal 10k/30k snapshots outside timing. The
parking build uses fixture revision v6 and passes exact continuation at all four boundaries.
The cached complete runner costs **83.00 s → 109.50 s**, including repeated snapshot loads;
initial generation and compilation are excluded. Complete reports retain all counters and
setup/load times. Each workload independently keeps its original limits.

| Workload / requested residents | Control hourly, ms | Parking hourly, ms | Control daily, ms | Parking daily, ms |
| --- | ---: | ---: | ---: | ---: |
| baseline / 10,000 | 0.930200 | 0.979105 | 0.001500 | 0.001625 |
| baseline / 30,000 | 3.921435 | 2.849197 | 0.001458 | 0.001834 |
| fiscal_stress / 10,000 | 6.167288 | 6.207324 | 0.001292 | 0.004333 |
| fiscal_stress / 30,000 | 26.237354 | 21.876636 | 0.001375 | 0.001791 |

All hourly/daily timing budgets pass (40/10 ms at 10k, 120/30 ms at 30k). Isolated phase
samples, with loading excluded, are:

| Phase | Control 10k / 30k, ms | Parking 10k / 30k, ms |
| --- | --- | --- |
| census | 7.466083 / 23.511334 | 12.314667 / 24.959959 |
| planning | 1.574209 / 4.503292 | 1.777250 / 3.520208 |
| execution | 0.290500 / 1.670084 | 0.536750 / 1.119750 |
| logistics | 0.518750 / 1.784584 | 0.991584 / 2.838416 |
| lots | 2.195666 / 1.916584 | 2.211209 / 2.632625 |
| settlement | 4.000459 / 11.752583 | 5.784333 / 13.998708 |
| routing | 12.894958 / 29.612583 | 13.197125 / 31.372167 |

The 10k census now quotes 16,944 supplier routes against 8,670 in the control, and runs 2,204
staffing trials against 1,625. These are different physical-mobility/economy states after the
same deterministic warm-up. Census building counts remain almost identical; the counters
separate changed work from pure per-row cost.

| Bounded counter | Baseline control 10k → 30k | Baseline parking 10k → 30k | Fiscal control 10k → 30k | Fiscal parking 10k → 30k |
| --- | ---: | ---: | ---: | ---: |
| census_buildings | 177,504 → 488,636 | 177,503 → 488,682 | 174,772 → 471,423 | 174,674 → 479,889 |
| reservation_rows_copied | 0 → 0 | 0 → 0 | 0 → 0 | 0 → 0 |
| owner_slots_prepared | 3,816,336 → 12,033,511 | 3,838,516 → 10,873,260 | 3,796,960 → 10,383,845 | 3,796,262 → 10,444,343 |
| buyer_sites_built | 0 → 379 | 10 → 54 | 716 → 2,774 | 722 → 2,148 |
| spawn_parcels_scanned | 547,304 → 1,506,788 | 547,304 → 1,506,788 | 547,304 → 1,506,788 | 547,304 → 1,506,788 |
| staffing_trials | 40,004 → 237,964 | 41,395 → 154,449 | 229,836 → 580,032 | 231,028 → 683,162 |
| supplier_searches | 13,452 → 68,012 | 14,404 → 63,720 | 0 → 0 | 0 → 0 |
| supplier_routes | 227,712 → 1,498,647 | 238,866 → 1,152,757 | 0 → 0 | 0 → 0 |
| job_search_keys | 0 → 17,982 | 132 → 486 | 4,880 → 14,873 | 6,667 → 18,467 |
| cch_searches | 95,460 → 312,899 | 69,449 → 231,640 | 5,835 → 27,315 | 2,505 → 5,695 |

The unchanged household-normalized counter limit is 3.4305× in the control and 3.4327× in
parking. The control exits 1 with seven failures. Parking exits 1 with five: baseline buyers,
staffing trials, supplier searches/routes and job keys. All five categories already fail in
the control; the two fiscal failures clear. Fiscal churn covers every one of 24 intervals.
This change does not close the existing PERF-03…05 simulation scaling debt.

#### Dense-district employment

```sh
METRUM_PERF_CACHE="$ARTIFACTS/after-economy/snapshots" RAYON_NUM_THREADS=8 "$TEST_BINARY" nodes::sim::core::populated_city::contract::parking_employment_reach --exact --ignored --nocapture --test-threads=1
```

The retained round-1b executable gets the same test-only diagnostic harness with its original
commute API; parking uses the complete resident itinerary. Both use the 10k district's day-12
snapshot, a fixed stride of 97 working-age agents, at most 32 workers, and a fixed 900 physical
second budget. Growth and employment are allowed to change, so the sampled population and
workplaces are reported rather than treated as identical people.

| Product | Round 1b | Round 2a |
| --- | ---: | ---: |
| Residents | 10,671 | 10,673 |
| Households | 5,310 | 5,307 |
| Employed | 7,221 | 7,169 |
| Funded jobs | 13,881 | 13,841 |
| Sample workers with an available car | 32 / 32 | 17 / 32 |
| Workplaces | 971 | 967 |
| Worker/workplace pairs within 900 s | 31,072 / 31,072 | 14,453 / 30,944 |

Employment decreases by **52 (0.72%)**, with 40 fewer funded jobs. Sampled job reach decreases
from **100% to 46.707%**, or **53.293 percentage points**. This is the measured district impact
of actual ownership, custody, parking and complete trip costs together, including the different
warm-up economy; it is not an isolated causal estimate of the density percentages.

#### Candidate locality, engine checks and captures

```sh
METRUM_LOCALITY_TIERS=0,1000,10000,100000 RAYON_NUM_THREADS=8 "$TEST_BINARY" nodes::sim::core::populated_city::commit_locality::benchmark_parking_locality --exact --ignored --nocapture --test-threads=1
```

The fixed neighbourhood has twelve lots, 72 places and **828 candidate entries**, identical
at every background tier. Nine repetitions measure each local operation; setup is excluded.
The first fixture attempt had inserted its street through the geometry-only helper, leaving
all twelve doors without lane IDs. `df1cf178` fixes only this ignored fixture to use the normal
road commit and publish its lane/entrance inputs before timing. Its test executable SHA-256 is
`8ff315f1b78ffccbeb7535d414394ae60f9e2c6c32e6126d1820f7d285f10413`;
production code is unchanged from the agent/economy comparison.

| Background buildings / agents | Total buildings / parcels / roads | Setup, s | Twelve lists, ms | Road-source publication, ms | Site publication, ms | Per-lot validation, ms |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 0 | 1,276 / 1,264 / 84 | 0.739 | 0.753333 | 20.943334 | 19.929333 | 0.148709 |
| 1,000 | 2,276 / 2,264 / 109 | 0.782 | 0.753333 | 20.726125 | 19.849208 | 0.142542 |
| 10,000 | 11,276 / 11,264 / 334 | 1.392 | 0.741167 | 20.776250 | 19.754250 | 0.141709 |
| 100,000 | 101,276 / 101,264 / 2,584 | 9.351 | 0.745666 | 20.620042 | 19.655417 | 0.144333 |

The lists stay at 0.741–0.753 ms and their products match bit-for-bit. Complete road/site
publication also rebuilds other nearby door catchments and costs about 20 ms, flat with growing
background; that new local walking work is separate from the 0.142–0.149 ms lot geometry solve.
Final clippy including the fixture fix again reports zero warnings. The candidate memory
estimate above excludes temporary bounded-search/publication buffers; pending door handles
use eight bytes each before vector spare capacity.

```sh
rm -f godot/bin/libmetrum_rise.dylib
cp "$RELEASE_LIBRARY" godot/bin/libmetrum_rise.dylib
perl -e 'alarm 900; exec @ARGV' godot --headless --path "$ISOLATED_PROJECT" --import
perl -e 'alarm 900; exec @ARGV' godot --headless --path "$ISOLATED_PROJECT" --script "res://tests/${TEST}.gd"
python3 tools/capture_parking.py --headless --output "$ARTIFACTS/capture-setup"
python3 tools/capture_parking.py
```

The latest production library passes each script in its own process under a unique isolated
profile: `agent_inspector_test`, `building_asset_reload_test`, `parking_test`,
`network_tool_chunk_renderer_test`, `road_benchmark_metrics_test`, `road_preview_metrics_test`,
`road_junction_preview_test`, `road_preview_stream_test` and `vehicle_ground_support_test`
(4,840 poses, minimum clearance 0.02 m). All have zero engine/script errors.
`simulation_speed_input_test` also passes here, despite its historical failure listing.
`asset_selection_test` exits 1 for its three known Ctrl+click checks, with zero script errors.
Generated UID/import-newline changes are removed after proving their exact generated contents.
The pre-existing screenshot/LFS changes remain untouched.

Two short windowed capture runs use private copies of neukku and kuopio and nearby standalone
lots. The final ten views are `district`, `slabs`, `houses`, `slab-lot` and `house-lot`, each
at 09:00 and 18:00 in `captures/parking/`. The explicit capture homes retain their authored
household capacities and use the documented unprofiled/low-density fallback; they do not run
an unrelated growth economy. These images were captured with the same physical itinerary
implementation before the economy-query follow-up; later changes do not alter that fixture.

The inspected lot views show pedestrians between buildings and bays, an arriving orange car,
a red car pulling out through the house-lot aisle at 18:00, and parked blue/grey/yellow cars.
No car visibly enters a building. Telemetry reports eight vehicles, 24 places, no stored cars,
no diagnostics and **zero resident car-mode occupants inside buildings** at both hours. At
09:00 the trace includes WalkToCar, Drive, PullIn and WalkToDoor; at 18:00 it includes PullOut,
Drive and WalkToDoor. Native tests cover the complete sequence and each rendering handoff.
The pictures record discrete visible states; native regressions exercise complete routes.

`TRAFFIC-13` can still detach before a looping route finishes; `TRAFFIC-15` can still prevent
lane-end detach. Resident PullIn uses that unchanged function, so both remain relevant here;
no parking-specific bypass is added. Round 2b still moves freight and immigration onto outside
vehicle custody and delivery walks, with their broader lifecycle/watchdog/edit recovery.
Resident deletion and resident recovery are already included in round 2a.

The required external diff review remains pending: automatic approval review rejected exporting
the private diff/plans to the reviewer CLI without user authorization. All runnable implementation,
checks, captures and measurements above are complete; this review is not reported as passed.


## Current Scope

### Shared lane occupancy contract (TRAFFIC-02)

All live lane acquisitions use `agents/lane_state.rs`: building egress and ingress rollback,
network entry, connector entry/exit, direct continuations, lane changes, road-edit reattachment,
building-origin recovery, and debug spawning. The shared writer validates the lane ID and travel
mode, finite nonnegative distance, and connector junction ownership before publishing lane,
edge, node, distance, and transit state together. Routing, access legality, and traffic claims
remain the responsibility of the requesting subsystem. Leaving a lane and clearing/replanning
a route remain separate operations.

The contract costs O(1) per acquisition and allocates nothing on success. It uses safe disjoint
field references, including at the existing parallel movement boundary; it adds no unsafe blocks.
A violated runtime invariant panics at the caller with agent/mode, previous occupancy, requested
lane and network revision. It does not silently change travel mode or relocate the agent.
`./run.sh --release` enables the existing crash diagnostics by default, so a rejected transition
is captured with its caller and the simulation flight recorder under `logs/` (see
[`debug.md`](debug.md)). This is a fail-fast invariant, including in release builds.
Save and load share the mode/owner checks. A rejected save retains the previous file through the
existing transactional replacement mechanism.

The `zoned-city-26-days.sqlite` investigation found 14 walkers assigned to vehicle lanes; lane 61
held five walkers ahead of nine cars. Their pedestrian destination plans and saved positions
confirm inconsistent runtime occupancy. Nearby road additions are a possible trigger, but the
original corrupting transition has not been reproduced. This contract refactor does not repair
that already-invalid snapshot or establish the historical root cause.

Performance acceptance (2026-10-14): matched unprofiled release executables, baseline
`abb37aa5` (SHA-256 `eabe4dc799995762e66b84552deb5865133b31fc19d59b8f5dee797da221136d`)
and the contract working tree (`11e08f2a3a59097cca794527c1741edbcbfa2451b6d8d0d53ba0f2678167c3a3`),
each run as `RAYON_NUM_THREADS=8 taskset -c 0,2,4,6,8,10,12,14 "$TEST_BINARY" <test> --exact
--ignored --nocapture --test-threads=1` in alternating order with nothing else running.

- `benchmark_lane_transitions` (five pairs): transition counts match in every scenario. At
  65,536 groups the median deltas are clear +1.3%, merge +1.6%, blocked +1.4%, cruise +2.9% and
  lateral −4.0% (at most +0.08 ms); smaller tiers stay within pair-to-pair noise.
- `benchmark_agent_tick_contract` (`PERF-06`, 25 pairs): work counts are identical. Medians at
  10k/30k: network 0.228→0.237/0.494→0.494 ms, egress 0.478→0.471/1.415→1.381 ms; ingress
  (0.230→0.265/0.443→0.361 ms) has overlapping interquartile ranges in both directions.
- `python3 tools/perf_contract.py --binary "$TEST_BINARY" --artifacts "$ARTIFACTS"` on both
  executables reports the same pre-existing `PERF-03`–`PERF-05` budget and scaling failures,
  with every non-timing field identical. Daily ticks are 6.98→6.62/34.45→34.56 ms
  (baseline) and 22.01→21.72/73.88→70.96 ms (fiscal stress) at 10k/30k.

The shared contract therefore adds no measurable cost beyond about 2% on transition-dominated
microbenchmarks and leaves simulation work unchanged.

Save format 58 stores one identity row per live lane: road edge, direction and local lane index,
or junction node plus incoming/outgoing road lanes. Current and planned agent references resolve
through these identities after graph compaction and lane rebuilds. Tombstoned lanes are excluded.
Loading rebuilds final visible-surface lane heights before restoring distances and references,
and validates each current lane against its owner and travel mode. A live session lifts lanes the
same way without waiting for the renderer: rebuilds only append lanes, so `LaneSystem` keeps a
watermark of lanes already on the visible surface, and `TransitNetwork::sync_new_lane_heights`
lifts the lanes past it at the start of every simulation frame and after each road mesh
precompute (O(new lane points); a frame with nothing pending returns at once). Before this, a lane
rebuilt outside a drawn road chunk, such as a crosswalk connector rebuilt by a building edit, kept
its flat build heights and so a shorter length until the next load, and a session continued
differently after save and reload. Pedestrian lanes remain active
through load, including final frontage legs with an exhausted graph-node path.
An empty network path can also be a pending immediate replan after demolition. With valid
node, edge and destination references, loading preserves its planned target and path cursor
even when retry time and failure count are zero. An agent in network transit at a valid node
stays there and replans from the node whatever its mode, as the running game does: a road edit
that rebuilds the junction lane a walker is crossing leaves it at the node with no edge (found in
`SIM-02-I` part 3, when loading sent such walkers straight into their destination). Delayed
retries retain their backoff; agents without a usable destination or pending retry still
receive orphan repair.

Version 57 saves recover sidewalk direction from the saved road/node and connector identity from
the saved junction, outgoing route and authoritative position (including the pedestrian offset).
Recovery only inspects that edge or junction's indexed lanes and rejects unmatched connector
poses. Equivalent matching route prefixes use stable lane order. New saves use explicit identities.
This work occurs only during save/load; current-lane pose sampling remains allocation-free O(log P)
in the lane's point count, and agent restoration uses local owner lookups rather than city-wide scans.

The current traffic model is intentionally local and deterministic. Cars are independent agents
travelling along lane centerlines with per-lane occupancy buckets used for local gap checks.
There is no global microscopic traffic assignment, signal timing, parking search, multi-car
negotiation, or stochastic driver personality model yet.

Supported live behavior:

- car following with simplified IDM
- road speed limits
- junction connector lanes with smooth cubic geometry
- curve-based junction turn speed caps
- acceleration and braking limits
- connector entry spacing so multiple cars can use a junction when safely separated
- planned same-edge lane changes for reaching the correct destination frontage lane
- conservative same-edge overtaking and return-to-cruise-lane behavior
- render-side position and rotation smoothing by stable car render ID
- `./run.sh --debug traffic` logging and visual traffic debug overlays

## Core State

Traffic state is stored in the agent SoA. The traffic-owned continuation fields are:

- `current_lane_id`: the lane currently governing longitudinal movement
- `lane_distance`: metres travelled along `current_lane_id`
- `speed`: current car speed in m/s
- `lane_change_from_lane_id`: source lane for an active S-curve lane change, or `u32::MAX`
- `lane_change_start_d`: lane distance where the active lane change started
- `lane_change_length_m`: longitudinal length of the active lane-change S-curve
- `overtake_blocked_time_s`: time spent held below free-flow speed by lane traffic
- `overtake_cooldown_s`: cooldown before another discretionary overtake / return lane change

Save format 75 preserves speed, `lane_change_*` and `overtake_*`: they affect occupied lanes,
arrival times and later transactions. Active lane-change origins are remapped through saved lane
identities. New benchmark agents may still start with inactive defaults.

## Lane Buckets

The buckets hold `(lane_distance, agent_index)` per lane, sorted by distance with an index
tiebreak. The tick keeps them across ticks: the end of each tick fills them from every live lane
agent on the shared pool (agents grouped by lane range in one pass over 4,096-agent chunks, then
each lane range fills, sorts and overlap-corrects its own lanes in agent order), appended agents
join at the next tick start, and swap-removes patch them in place (`SIM-02-K` part 4,
[simulation.md](simulation.md#sim-02-k-part-4--agent-tick-2026-10-20-first-commit)). A bucket is
sorted again after an overlap clamp, so the snapshot is always the function of the agents' state
that a rebuild gives. These buckets are the hot-path structure for:

- nearest-car-ahead gap lookup
- connector entry slot checks
- lane-change target gap checks
- post-movement overlap correction

The bucket fill reuses retained scratch buffers and allocates nothing per agent once they have
grown. This is required for the 1M-agent scale target; traffic logic must not allocate per agent
in the tick hot path.

Active lane-changing cars are inserted into their target lane bucket and, before movement, also
into their source lane bucket while the S-curve is still active. This lets cars in both lanes react
to the crossing car during the IDM speed pass. Post-movement overlap correction clamps the current
authoritative lane.

Removing the last agent also retires dirty lane occupancy before the
empty-agent tick returns (`AUDIT-01-A5`). Cleanup reuses the existing dirty-lane list and
buffers. Once these are empty, later empty ticks skip the occupancy rebuild. The separate
low-frequency frontage-delay cache continues its existing decay from live agent speeds. Its
once-a-simulated-second update notes each agent's lane in parallel, then sums each lane range's
cars in agent order on the pool, so the delays are bit-identical for any worker count
(`SIM-02-K` part 5).
Save format 75 preserves that smoothed delay and its fractional update clock, restoring lane
values through saved lane identities; see [frontage delay persistence](entrance_and_exit.md#exact-frontage-congestion-penalty-cache).

Claim preparation classifies each agent independently into the retained byte per agent
(`AUDIT-01-A4`, `AUDIT-01-A12`). Large collections use contiguous 4,096-agent Rayon batches; smaller collections
and single-worker execution use the same direct loop. Lane-change selection is shared with
movement (`AUDIT-01-A12`): it inspects the current edge's lanes and sorted local occupancy, with
O(D + log K) work for D local lanes and K occupants. The byte distinguishes ordinary parallel
movement, fixed lateral candidates and dynamic handoffs, so ordinary movement skips a second
lane-change decision. There are no per-agent allocations or additional per-agent buffers.

A lateral move that cannot reach another lane end or frontage handoff this tick reserves its
target in the existing per-lane array. Atomic minimum selects the lowest agent index independently
of worker scheduling; these cars then move in parallel. Dynamic handoffs execute afterward in
stable agent-index order and use the remaining unreserved lanes. Fixed lateral reservations
therefore have priority over connector/frontage arrivals for that tick. Reusing one's own
reservation is allowed, including entry followed by a frontage detach.

Each lane stores one owner index (`usize::MAX` when free), replacing the former boolean. On a
64-bit target this adds seven bytes per retained lane; reset remains O(L) in retained lanes, in
parallel 4,096-lane chunks from 65,536 lanes.
Connector choices retain their existing `stable_index` algorithm and seeds.

## Car Following

Cars use a simplified Intelligent Driver Model in `idm_new_speed()`:

- free-flow speed is the current road edge speed limit, or the connector turn speed in junctions
- gap is bumper-to-bumper distance to the next car in the lane bucket
- the full approach-speed IDM term is not implemented yet because per-agent lead speed is not
  tracked

Current constants:

- `IDM_A_MAX = 6.0 m/s^2`
- `IDM_B = 6.0 m/s^2`
- `IDM_T_HEAD = 0.5 s`
- `IDM_S_MIN = 0.1 m`
- `CAR_LENGTH = 2.6 m`

After IDM proposes a target speed, `limit_speed_change()` clamps the change by acceleration and
comfortable braking. This prevents cars from snapping instantly between stopped and free-flow
speed.

The speed pass writes each result directly to that agent's SoA slot (`AUDIT-01-A7`). This is
independent because the shared lane snapshot contains distances and the current IDM model reads
no other agent's speed. The duplicate speed array and its final serial copy are removed. Non-car
and inactive agents retain their current speed while their existing traffic-timer rules run.

If a car is approaching a blocked connector or a required lane change whose target gap is blocked,
the speed pass computes a braking speed for the remaining distance. This makes the car slow before
the conflict instead of teleporting or stopping only at the exact handoff point.

## Road Speeds

The current urban road presets use `50 km/h`, stored as:

- `DEFAULT_URBAN_ROAD_SPEED_MS = 13.89 m/s`

The simulation stores speeds in m/s. `KMH_TO_MPS = 1 / 3.6`.

Normal road bends do not currently receive a curvature speed cap. A car on a curved road lane
keeps the road speed limit unless traffic or upcoming connector / lane-change constraints slow it.

## Junction Connectors

Vehicle junction movement uses explicit connector lanes generated in
`network/lanes/vehicle_junctions.rs`.

For every allowed inbound-to-outbound vehicle lane pair at a node, the lane system creates a
connection lane. A pair is allowed by the node's whitelist when it has user vehicle connections,
else by the [default junction lane use](roads.md#default-junction-lane-use-road-94) (`ROAD-94`):
one connector per pair the approach lane's movement share and the target fan allow, not every
pair. Each connector has:

- `edge_id = usize::MAX`
- `lane_type = LaneType::Vehicle`
- `node_id = junction node`
- `next_lanes = [target road lane]`
- geometry from the inbound lane's end to the outbound lane's start, as below
- samples at most `1 m` apart along the path and at most `0.15 rad` apart along an arc

**Lane ends come from the roadbed's mouth frames (`TRAFFIC-12`).** Every vehicle lane ends at a
node on its arm's `MouthFrame` (`roadbed/walkways.rs`): the arm's cross-section line at the edge's
clip there (`start_clip` / `end_clip`, the station every system that maps edge stations to lanes
reads), built from the arm's own knots exactly as the junction layout collects them
(`NodeArms::single`, `ArmInput::section_line`). The graph sets the clip to the layout's mouth
station at a bend or junction, where the node piece ends and the span begins (rounded to `f32`, so
a lane end lies within a micrometre of the mouth row), and to zero at a terminal, a pass-through
node and a node whose layout fails. A lane's end point is its lateral offset on that line (the
lane's offset, centred on one-way and uneven roads by `Edge::lane_group_offset`, `ROAD-100` in
`roads.md`), at the centreline height there; the lane polyline ends exactly on it (`LaneEnd::Mouth`), keeping only
the points more than `1 mm` beyond the station, since the roadbed profile puts a knot at every
mouth station that would otherwise stay as a zero-length end piece. The connector leaves and joins
along the frame's direction, square to the line, and never reads a lane polyline's end piece:
those pieces were zero-length, millimetres long or reversed at mouths and sharp bends, and a
zero-length one sent the old connector off along `+x`. Building a frame is `O(edge points)`, once
per edge end in the lane rebuild and once per incident edge end in the connector builder.

**Curve.** One construction that always exists (`vehicle_junctions/turn.rs`, `VehicleTurn`), of
straights and circular arcs only, so the heading is continuous and the curvature piecewise constant:

- **Tangent arc**, where the entry direction and the exit direction meet ahead of both ends: a
  straight along the entry, one arc and a straight along the exit, a fillet of the two tangent
  lines. Its radius is the largest that fits between the ends, reduced by bisection (24 halvings)
  until the arc keeps half a lane from every kerb corner of the layout (the fitted carriageway-edge
  circles of its concave sectors): at a right-angle kerb-lane turn that is the arc concentric with
  the kerb, and a large fillet no longer cuts a skewed junction's obtuse corner. The longer lead
  keeps its straight. The arc never reaches past the tangent lines' intersection, so it neither
  overshoots nor counter-steers. Construction and kerb bound from Veeti's road-chain branch
  (`1ae3b22f`).
- **Biarc** otherwise: straight on, a lateral shift, directions that meet behind an end, a U-turn,
  or a tangent arc that no radius keeps off the kerbs (none on the swept maps). Two arcs with a
  common tangent at their joint; the joint is the equal-tangent choice: both ends' tangent lines
  are cut at the same distance `d > 0`, the joint is the midpoint of the two cut points and its
  tangent runs between them. `d` exists for every pair of ends except parallel directions with the
  exit level with or behind the entry, which no junction produces; coincident ends (a user
  connection through a degree-two node) give a zero-length connector.

There is no cubic and no fallback; the old junction-span cubic, whose handles (`0.55` of the
footprint span) overshot short turns, is gone. Building a connector is `O(c)` for `c` kerb corners at
the node, `O(24 c)` when the radius is reduced, only on a lane rebuild, and allocates nothing but
its polyline.

**Check.** Each connector is checked once per lane rebuild against the node's kerb corners
(`NodeWalkways::collect_with_kerbs`, from the same layout as the walkways): the
smallest distance from a corner's centre to the connector's straights and arcs, less the corner's
radius, must not be negative (`O(c)`). A violation is logged on the `traffic` debug channel with the
node and both lane ids and is counted by the save sweep; nothing clamps or repairs the curve.

Acceptance record (`TRAFFIC-12`, 2026-10-31; commands and results; the artifact folders stay
local):

- Save sweep: `junction_connector_save_sweep` (ignored; `METRUM_SWEEP_SAVE=<save> $TEST_BINARY
  nodes::sim::core::tests::junction_connector_sweep --ignored --nocapture`, optional
  `METRUM_SWEEP_OUT=<file>` for one line per failing connector and the node layouts) loads a save,
  whose load rebuilds every lane, and measures every vehicle connector at every node with
  `lanes::trajectory_tests::measure_connectors`: the farthest probe (every `0.25 m`) outside the
  compiled carriageway, a step heading against the chord (against both lane directions in turns
  sharper than a right angle, whose exit heads against the chord), a counter-steer where the arm
  directions meet ahead of both ends (beyond `1 mrad` and the f32 resolution of the two steps: the
  demo save lies `8.5 km` from the origin, where f32 rounds points by `0.5 mm`), and a kink (a turn
  over `0.2 rad` between steps or against a lane's end piece); and every lane end against its mouth
  frame. `dev` against the fix (failing connectors, then outside / backward / counter-steer / kinks):

  | Map | `dev` | Fix |
  | --- | --- | --- |
  | Demo save copy (39 nodes) | 117 of 156: 44 (max 4.75 m) / 31 / 89 / 103 | 0 of 161 |
  | Kuopio reference save (80 nodes) | 335 of 440: 80 (max 5.65 m) / 65 / 206 / 288 | 0 of 440 |
  | Saved 100k city (2,220 nodes) | 8,282 of 24,834: 0 / 0 / 8,272 / 8,282 | 0 of 24,834 |

  Causes on `dev` (first match): a lane end piece under `3.2 mm`, so the builder used `+x` as its
  direction (82 demo, 179 Kuopio; the knot the profile puts at every mouth station, cut on exactly);
  an end piece off its arm by more than `0.05 rad` (4 demo); a lane end more than `1 cm` off its mouth
  line, the trim by 3D lane length on curved and sloped arms (14 demo, 62 Kuopio; up to `0.71 m`);
  and the junction-span cubic with good lane ends (17 demo, 94 Kuopio, every failure of the 100k city:
  its right turns counter-steer and kink by `0.24 rad`). With the fix every lane end lies on its mouth
  line, measured against the compiled roadbed's own mouth station (largest distance `0.0000 m`,
  tolerance `1 cm`), its end piece within
  `0.0077 rad` of the arm (tolerance `0.05 rad`) and at least `9.5 mm` long; no kerb violation is
  logged; the largest step is `0.150 rad`; the tightest non-U-turn is a `1.73 m` tangent arc across a
  pass-through node's slight kink (demo node 33) and the tightest junction turn a `3.58 m` biarc at a
  50-degree bend (Kuopio node 29). RJ's six fixtures (`trajectory_tests`) are clean. The 45-degree
  bend now gets two connectors: `dev`'s `+x` end directions made its inner lanes look collinear and
  overlapping, so they were linked straight across the node piece. Span lanes (the mitred offset
  inside a span): no vehicle lane of the demo or Kuopio save turns more than `0.014 rad` between steps
  of at least `1 cm`, so no mitre spike or reversal was found inside a span.
- Product probes: only the lanes component changes (20 of 20 reference entries, 133 of 141 roadbed
  fixture entries); graph, nodes, layouts, spans, walkways and queries hashes stay bitwise equal.
- Rendered top-down shots (`road_junction_reference_test.gd` with `METRUM_ROAD_JUNCTION_CONNECTORS=1`
  and `METRUM_ROAD_JUNCTION_SAVE` / `METRUM_ROAD_JUNCTION_SAVE_FOCI` on a copy of the demo save,
  gamescope headless, isolated profile) of a 90-degree bend with a zebra on one arm (node 19), the
  skewed X (node 5) and a T (node 7): on `dev` connectors hook over the zebras and the sidewalk and
  loop at both acute corners; with the fix every connector stays inside the carriageway between
  the zebras' outer edges.
- Gates: `cargo clippy --all-targets` warning set identical to `dev`; `./run.sh --test` exit 0 with
  the debug library suite at 1,851 passed, 0 failed, 179 ignored, no Godot script skipped.
- Road-edit preview: the connector build does not run in the preview path (lanes rebuild on
  commit). `benchmark_road_preview_stages` (`RAYON_NUM_THREADS=4`, cores `0,2,..,14`, `dev`, fix,
  fix, `dev` in one session): identical vertex counts; total p50 `sloped_t` 2.920, 2.770 ms (`dev`)
  against 2.973, 2.891 ms, `sloped_branch` 2.787, 2.520 against 2.556, 2.603 ms.
- `PERF-06` (`tools/perf_contract.py --agents`, 8 workers, one run each in the same session; its
  fixture has no junction): identical work counts; network 10k 0.271 against 0.245 ms, 30k 0.861
  against 0.631 ms; egress 0.729 / 1.436 against 0.567 / 1.474 ms; ingress 0.307 / 0.272 against
  0.313 / 0.290 ms; single runs at a load average near 2 (another round shared the machine), so
  within noise and not a speed claim.
- 100k lane audit (`traffic07_lane_entry_audit`, day 12 at 32×, 8 workers, each engine its own
  city): `dev` reproduces the `TRAFFIC-11` record exactly (0 lanes over capacity and 0 stacked cars
  at every sample; 2,579 and 3,385 vehicles held at nodes, 96 and 234 vehicles at attach points at
  00:00 and 24:00). The fix: 0 lanes over capacity and 0 stacked cars at every sample; 2,682 and
  3,664 held at nodes, 140 and 192 at attach points (a different city, so the holds are not
  attributed). This city was prepared with the fix's first build, whose lane ends took the layout's
  `f64` mouth station instead of the edge's `f32` clip (at most a micrometre apart).
- Envelope and determinism (`tools/sim02_gate.py envelope determinism --bursts
  departures,settlements`, `RAYON_NUM_THREADS=6`, both engines in one session): both PASS,
  determinism identical (repeat, 1 and 3 threads, reload). Both raise the same 13 notes of the stale
  baseline; the fix moves them by at most 2.6% apart from tiny rates (`dev` against fix): 10k
  baseline out-of-stock 0.1944 / 0.1943; 10k fiscal stress deliveries 8.89 / 8.69, employment
  0.07446 / 0.07450, final employment 0.00602 / 0.00592, net income 27.31 / 27.28, out-of-stock
  0.8062 / 0.8080; 30k baseline bankruptcies 1.610 / 1.652, deliveries 128.5 / 129.4, employment
  0.6004 / 0.6021, final employment 0.5418 / 0.5436, net income 8.53 / 8.58; 30k fiscal stress
  employment 0.07682 / 0.07686, final employment 0.00624 / 0.00684. The 30k arrivals workload
  moves by at most 3% on its main metrics (out-of-stock 0.218 / 0.225).

Allowed turns are derived from `Node::lane_connections`:

- if the node has no user vehicle connections, all non-U-turn outbound lanes are open
- if the node has any user vehicle connection, the node is in whitelist mode and unspecified turns
  are blocked
- terminal nodes may connect back to the same edge

Strict degree-two corridor splits may skip materializing a vehicle connector and directly link the
aligned incoming physical lane to the outgoing physical lane. This direct pass-through is allowed
only when the node has exactly two incident vehicle arms, no user-authored vehicle lane connections,
their XZ tangents are near-collinear, and the lane mouths are coincident or overlap (on the inside of
a slight bend the outgoing lane starts behind the incoming lane's end, so any connector would run
backwards).

The pass-through degree here counts only arms with vehicle lanes (`TRAFFIC-03`), so a footpath
joining a road leaves it a pass-through for cars. The U-turn rule is the shared
`open_node_allows_u_turn` (`NET-07`), which counts every arm: a road ending in a footpath is not a
dead end, and cars do not turn around there. Counting the footpath once turned such a node into a three-arm junction: the road's
coincident lane ends got a junction-span Bézier connector that ran about 3 m backwards, so cars
turned around for a moment. Saves holding such a connector still load: a saved connector whose
incoming lane now links straight to its outgoing lane is dropped, and agents on it replan. True junctions and all
user-authored vehicle connection nodes must keep explicit connector lanes so junction speed,
spacing, and whitelist semantics still apply.

Cars enter connector lanes through `TRANSIT_INTERSECTION`. Connector lanes are lane-bucketed like
road lanes, so multiple cars may occupy the same connector when they respect separation. Connector
entry also uses a per-tick claim to avoid two cars grabbing the same zero-distance entry slot.
Car exits check outgoing-lane occupancy and acquire the same reservation (`AUDIT-01-A11`). A
blocked car waits at the connector end with its route intact until the outgoing entry clears.

**Tried and rejected: don't block the box (`TRAFFIC-14`, round R14C, 2026-10-31).** A rule that
let a car onto a connector only while the connector's exit lane had room, from its start to its
tail car, for every car already on the connector plus this one (tail distance at least `(cars on
the connector + 1) × (CAR_LENGTH + IDM_S_MIN)`, an empty exit always passing), tested in the
connector claim and in the road-lane brake of the speed pass, was built and measured, then
dropped by the user's decision. It reads positions only, so a moving exit queue whose tail has
just entered counts as full: in flow a connector admits a car only after the previous one has
cleared `(cars on the connector + 1)` slots, and a car waits at the stop line and starts from 0
instead of rolling up the connector. Envelope against the same-session `dev` control
(`bcbb148f`), rule alone: 30k baseline deliveries 135.7 to 79.8 (-41%), net income 9.89 to 4.69,
bankruptcies 1.26 to 2.48, out-of-stock 0.21 to 0.46; 10k baseline deliveries 173.4 to 153.2
and bankruptcies 0.03 to 0.57; 19 notes against 14. The claim test alone (no brake) still cut
30k deliveries to 88.6 (-35%). On the 1-day 100k audit 40-46k cars stood unmoved per 6 hours
against 29-30k. With it the 100k city formed no cycle at all to day 12 (68,436 cars on road,
3,652 held at nodes, 371 at attach points; with `TRAFFIC-16` as well 70,575 / 3,710 / 376). A
rule against a standing exit queue only, which needs the exit's speeds at the tick's start, is
recorded under `TRAFFIC-09`; it is not built.

A car that starts at a node (border-origin freight, an arrival carrier, a replan at a node, or a
zero-hop route onto its planned detach lane) enters its first lane under the same two checks
(`TRAFFIC-07`): `lane_entry_slot_clear` on the candidate lanes of the first edge, starting at
the stable choice and wrapping, then `claim_lane_entry` on the first clear one. A car that finds
no lane it may enter holds at its node with its path, access plan, freight shipment, target and
scheduler state untouched, and stays in the raw network counts. The claim classification marks a
car with a valid plan, no lane and no clear first lane `HeldEntry`: it skips the serial handoff
pass and is not processed at all that tick (speed 0), so a queue at a border costs one gap test per
lane of the first edge, per held car and tick (O(node degree + lanes on the first edge) lookups
and one O(log bucket) test per lane, no allocation), done in the parallel classification. The
first tick the slot is clear it classifies `Dynamic` again and takes the entry in the serial
phase. An occupancy snapshot taken at the start of the tick decides both the hold and the entry,
so the result does not depend on the worker count. Pedestrians keep entering unchecked.

Acceptance record (`TRAFFIC-07`, 2026-10-30; commands and results; the artifact folders stay local):

- Regression: `movement_pass::tests::node_origin_cars_wait_for_an_entry_gap` puts 64 trucks at the
  start node of each of 80 one-lane roads (the 5,120 agents cross the parallel classification
  chunk) and ticks 24 times with 1 and 8 workers. On `dev` all 64 trucks of a road stack at
  distance 0; with the hold none do, the waiting trucks keep path, plan, shipment id and speed 0,
  and the lane state is bit-identical for 1 and 8 workers. Debug `cargo test --lib`: 1,847 passed,
  178 ignored, 0 failed.
- Lane audit: `frame_budget::lane_entry_audit::traffic07_lane_entry_audit` (ignored, release)
  loads a prepared snapshot, runs one game day and counts, every `METRUM_AUDIT_EVERY_MINUTES`
  (default 240), the lanes holding more cars than fit (`floor(length / 2.7) + 1`), the cars within
  0.5 m of a lane's start, and the vehicles held at nodes; `METRUM_AUDIT_TRACE=1` lists every car that
  steps onto a lane that was already full in the previous frame, with its previous state. Each engine
  builds its own 100k city (`perf_contract_prepare`, `RAYON_NUM_THREADS=8`, cores `0,2,..,14`), day
  12 at 32×. (Its freight count and its trace's "truck" label tested `freight_shipment_id != 0`,
  but a vehicle with no shipment holds `u64::MAX`, so every vehicle counted as a truck; fixed in
  round R14A. The held and traced counts below are vehicles.) `dev`: 11 lanes over capacity at 00:00 and 14 at 24:00 (one lane held 868 cars where
  197 fit, then 925), 685 and 754 cars within 0.5 m of one lane start, 2,872 and 3,708 cars
  stacked on 14 and 15 lanes, 12 and 5 vehicles held. With the hold: 0 lanes over capacity and 0
  stacked cars at 00:00, 2 lanes over at 24:00 (200 cars where 140 fit; 81 cars stacked), 2,636 and
  3,463 vehicles held at nodes. None of the 81 entries that took a lane above capacity came from node
  entry (on `dev` 255 of the first 400 entries onto a full lane did, within 40 game minutes); all
  81 were vehicles that left the access-egress state onto one lane in one tick (22 and 59 vehicles at
  a single attach point), tracked as `TRAFFIC-11`.
- `PERF-06` agent contract (`tools/perf_contract.py --agents`, 3 alternating `dev`/fix pairs, cores
  `0,2,..,14`, 8 workers): identical work counts; 10k network tick 0.255, 0.238, 0.255 ms
  (`dev`) against 0.239, 0.240, 0.235 ms; 30k network 0.718, 0.533, 0.468 against 0.663, 0.462,
  0.545 ms. Same fix-built 100k city, first two game hours at 1×, 3 alternating pairs: mean agent
  tick 5.065–5.091 ms with `HeldEntry` against 5.300–5.315 ms with held cars left on the serial
  pass (the earlier prototype's approach), 0.22 ms less. Each engine's own city at 1×: 4.925 and
  4.948 ms (`dev`, 12 held vehicles) against 5.044 and 5.026 ms (fix, 2,636 held); the cities
  differ, so that pair does not isolate the hold.
- `./run.sh --test`: Rust suite plus the headless and rendered Godot scripts.
- Envelope (`tools/sim02_gate.py envelope determinism --bursts departures,settlements`,
  `RAYON_NUM_THREADS=6`, fresh cities): PASS with no failures; determinism identical (repeat, 1
  and 3 threads, reload). The committed baseline is stale on `dev`: the unmodified `dev` tip raises
  13 notes of its own (10k baseline out-of-stock 0.173 against a band of 0.055–0.114; 30k baseline
  employment 0.600, net income 8.45 and deliveries 127 against 0.65–0.69, 12.2–13.6 and
  164–177), so the comparison is fix against `dev` in one session. At 30k the main metrics move
  by 1.5% or less in the baseline workload apart from out-of-stock (+3.8%), by at most 5.1% in
  fiscal stress and by at most 2.2% in the arrivals workload; at 10k the fix raises the
  out-of-stock rate in all three city sizes (0.97×, 1.00×, 1.03×) by +0.021, +0.021, +0.017 in the
  baseline workload (0.173 to 0.194 at 1.00×) and by +0.024, +0.020, +0.018 in fiscal stress (0.786
  to 0.806 at 1.00×, which crosses its band edge at 0.787): the two 10k out-of-stock notes of the
  earlier prototype. Cause: the hold shows the border's real throughput. One single-lane trunk per
  10,000 residents cannot pass the imports; the fix-built 10k city has 1,404 vehicles waiting at its
  border at 00:00 of day 12 and 1,919 at 24:00 (the road holds 1,100–2,600 vehicles), which the
  stacking used to hide. Border throughput stays a fixture question (`TRAFFIC-07` does not change
  it).

A vehicle placed on the road outside the movement pass waits at its attach point the same way
(`TRAFFIC-11`). `AgentVec::hold_for_lane_attach` leaves it off every lane (`TRANSIT_NETWORK`, no
lane, edge and origin node of the attach lane) with the attach lane and distance in
`planned_attach_lane_id` / `planned_attach_lane_d` and `ACCESS_LANE_ATTACH_PENDING` set. The claim
classification marks it `HeldEntry` while `lane_attach_slot_clear` fails at that distance (one
O(log bucket) test per held car and tick, no allocation, off the serial pass) and `Dynamic` once
the slot is clear; the serial phase then claims the lane with `claim_lane_entry` and attaches at
the driveway speed, exactly as egress does, so at most one car takes a lane per tick. Three
placements use it: a building removal's evacuation (`start_building_origin_plan_on_network`), a
road edit's reattachment when a rebuilt lane's spot is already taken by a car reattached earlier
in the same pass (every car on the edit's lanes is detached, so those cars are the lane's whole
occupancy; O(reattached × log) per edit), and a car whose ingress plan turns invalid on its
driveway (it returns to its detach point). Walkers keep entering at once. Trip-plan clears and
replans keep the hold with its attach point; a hold whose lane no longer exists, or no longer
belongs to the car's edge, is dropped and the car repairs from the lane's origin node.

Diagnosis (`TRAFFIC-11`, 2026-10-30). On the `TRAFFIC-07` branch (`3532b612`) the traced 100k
audit (`METRUM_AUDIT_TRACE=1`, which now also records the buildings removed in the frame and each
car's previous building) shows both bulk entries in a frame in which the economy removed buildings:
at game minute 16,024, 14 removals, 22 vehicles step onto lane 13,362 (edge 2,227, 380 m, 141 cars
where 140 fit) at one distance, 296.98 m; at 16,504, 13 removals, 59 vehicles onto lane 13,357 (edge
2,226) at 343.02 m. Every vehicle of a group was in `TRANSIT_ACCESS_EGRESS` at the same building
(57,760 and 57,711) the frame before, has its home as its new target and the driveway speed 3 m/s.
That is the evacuation of a removed building: `remove_evacuated_building`
(`buildings/allocator/lifecycle.rs:870`) calls `evacuate_referencing`
(`agents/building_refs.rs:415`), which plans each occupant's trip home and calls
`start_building_origin_plan_on_network` (`building_refs.rs:481`, `:509`, defined at `:601`). Its
`enter_lane` (`building_refs.rs:644`) put every occupant on the building's attach lane at the
attach distance without `lane_attach_slot_clear` or `claim_lane_entry`. The vehicles were the egress
queue waiting at that full attach slot, so a business failing beside a jammed road dumps its whole
queue onto one point. The same unchecked placement existed in the road-edit reattachment
(`agents/remap.rs:331`) and the invalid-ingress rollback (`movement/access_state/ingress.rs:168`);
the audit day has no road edit and showed no ingress rollback.

Acceptance record (`TRAFFIC-11`, 2026-10-30; the artifact folders stay local):

- Regression: `building_refs::tests::evacuated_egress_queue_waits_for_attach_gaps` puts 64 egress
  trucks at an origin building on each of 80 one-way 1 km roads (5,120 agents, across the parallel
  classification chunk), evacuates every origin and ticks 40 times with 1 and 8 workers. With
  `TRAFFIC-07` alone all 64 trucks of a road stand on the attach point after the evacuation
  ("cars stacked at 50 and 50 (64 on it)"); with the hold none does, the held trucks keep target,
  shipment id and speed 0, every lane keeps the static gap at every tick, and 1 and 8 workers give
  bit-identical lane state. `remap::tests::reattach_holds_a_car_whose_spot_is_taken` covers the
  road-edit hold.
- Save and load: `save::agents::lanes::tests::loaded_cars_held_at_an_attach_point_keep_waiting`.
  Load validation used to clear any access plan with a flag bit in `0xE0` and the plan fields of
  plan-less network cars; the first 100k prepare with the hold failed its exact save/load
  continuation check (`employed` 47,618 against 47,619 one hour after the reload) until load kept
  the hold like the running game does.
- Lane audit, same command as `TRAFFIC-07` (`perf_contract_prepare` then
  `traffic07_lane_entry_audit`, `RAYON_NUM_THREADS=8`, cores `0,2,..,14`, day 12 at 32×, the
  fix-built city): 0 lanes over capacity and 0 cars stacked at a lane start at 00:00, 06:00,
  12:00, 18:00 and 24:00; 2,579 vehicles held at nodes at 00:00 (2,349 of them carry freight,
  recounted in round R14A) and 3,385 at 24:00; 96 vehicles
  held at an attach point at 00:00 and 234 at 24:00. The trace lists 51 entries onto a lane that
  was full in the previous frame, all onto lanes of at most 100 m (connectors), at most 2 per lane
  and frame, none leaving a lane over capacity. On the `TRAFFIC-07` branch's own city the same
  binary ends the day with 0 lanes over capacity and 0 stacked cars (`TRAFFIC-07` alone: 2 lanes,
  81 stacked) and 136 vehicles held at attach points. The committed tree gives the same counts on
  the fix-built city; its held vehicles wait on 20 lanes at 00:00 and 27 at 24:00, only one of
  which holds as many cars as fit, so they mostly wait for a gap at the attach point itself (a
  queue standing through it) rather than for lane capacity. Whether any of them waits
  indefinitely is not established.
- `PERF-06` agent contract (`tools/perf_contract.py --agents`, 5 alternating pairs, cores
  `0,2,..,14`, 8 workers; control: release test binary of `3532b612` built in the same session,
  SHA-256 `b17c717a…`; fix `506daff0…`): identical work counts in all ten runs. The timings are
  not comparable this session: another round's runs shared the machine (load average 4-7) and the
  samples scatter 2-5× above the quiet `TRAFFIC-07` session in both binaries (10k network
  0.33-1.22 ms control, 0.40-1.12 ms fix; medians 0.62 against 0.55 ms at 10k and 1.06 against
  0.97 ms at 30k network, 0.67 against 0.80 and 1.39 against 1.92 ms egress, 0.30 against 0.48
  and 0.49 against 0.38 ms ingress). The contract's fixture holds no car at an attach point; the
  added per-tick work is one lane test per network car in `handle_network_movement` and a flag
  test per lane-less network car in the classification. Same `TRAFFIC-07` 100k city, first two
  game hours at 1×, 2 alternating pairs: mean agent tick 5.038 and 5.054 ms (control) against
  5.078 and 5.097 ms (fix), +0.04 ms (0.8%) in both pairs. The two runs diverge at the window's
  first evacuation, so the pair mixes the check's cost with different traffic.
- Envelope (`tools/sim02_gate.py envelope determinism --bursts departures,settlements`,
  `RAYON_NUM_THREADS=6`, fresh cities): PASS, determinism identical (repeat, 1 and 3 threads,
  reload). The same 13 notes as `TRAFFIC-07`'s run; the 10k and 30k fiscal-stress values are
  unchanged, and 30k baseline moves by at most 1.1% (bankruptcies 1.593 to 1.610 per 1,000
  buildings and day, deliveries 129.15 to 128.52, employment 0.6013 to 0.6004, net income 8.57
  to 8.53).
- `./run.sh --test` (headless, isolated profile): exit code 0; debug library suite 1,850 passed,
  0 failed, 178 ignored; no Godot script skipped and no script error.

### Walkers follow their route's sidewalks (`ROAD-74`)

A walking route is a node path; it is legal when some chain of sidewalk lanes and pedestrian
connectors walks it into the exit lane (`path_has_connectors_from`). Movement used to take any
connector onto the next road (the shortest, seeded among ties). With crossings off a junction
joins only some sidewalks to some arms, so that choice could leave a walker on a sidewalk from
which the rest of its route has no connector, and it waited at the mouth for ever (round R6b
trace: agent 4990, route 85, 86, 87, at node 86 the shortest connector kept it on sidewalk idx
100 of edge 84, and only idx -100 reaches its exit lane at node 87, a junction without
crossings).

- **The witness.** Each walk carries its route's lane witness, `AgentVec::walk_lanes`: one
  `Lane::lane_idx` per road of `current_path`, the cheapest chain of sidewalks along the node
  path into the planned exit lane by connector length (both sidewalks of a road cost the same to
  walk), ties within 1 mm broken by the agent's schedule seed (`tick/walk_lanes.rs`). It is built
  once per route, at its first junction from the lane the walker arrives on (or at node entry, for
  a walker with no lane yet), and cleared with every new route. At a node the walker takes the
  connector into the witness's sidewalk, never another one. The choice at a node depends only on
  the lane the walker arrives on and the rest of its route, so a witness rebuilt from any later
  lane of the chain repeats it: it is not saved, and a loaded walker continues the same walk.
- **No step at the node.** Where the witness names no connector from the walker's lane (a
  crossing removed under it without the editor), the walker waits at the mouth and replans from
  its own lane through the normal planner (`plan_network_replan` with the lane as
  `RouteStart::Lane`): a `NoTurnToEdge` or `NoTurnToLane` record with outcome `Replans`, and it
  walks the new route from the next tick. Walker replans from a road lane elsewhere (the editor
  clears every route through a toggled node, then the access-plan gate replans it; a stale exit)
  also start from that lane, not from any sidewalk of its road.
- **No route: home, or stranded (the user's option (c), 2026-11-01).** A walker on a road lane
  whose replan finds no route to its destination gives the trip up and plans a walk home from
  that lane (one `NoRoute` record, outcome `WalksHome`), then walks home: activity home, as the
  watchdog's move home sets it, but from where it stands. When home is out of reach too, or it
  was already walking home, it is stranded (`ACCESS_WALK_STRANDED`, one `NoRoute` record with
  outcome `Stranded`): path and exit lane cleared, it walks to the end of its lane and waits
  there, and its replan gate plans home from its lane after 5 s, 30 s, then every 2 min
  (`walk_home::STRANDED_RETRY_S`; the step in `network_replan_failures`, the time in
  `next_replan_time`), with no record per retry. It walks home as soon as a plan succeeds. No
  teleport, no crossing of its own, and the watchdog's move home does not apply. Cost: one route
  query per stranded walker per retry (at most one every 2 min once settled), O(1) per stranded
  walker and tick in between. A walker without a home keeps trying its destination on the same
  backoff.
- **No teleport for walkers (`TRAFFIC-19`, 2026-11-02).** The stuck-agent watchdog
  (`movement/replan_watchdog.rs`, a move home or to a border node after three failed replans)
  never moves a walker. Every site that hands it a failed replan sends a walker to
  `walk_home::walker_lost` instead: the walker plans its destination again from where it stands
  (one record with the site's cause, outcome `Replans`), and with no route gives the trip up for a
  walk home or is stranded, as above, with no retries of the watchdog's. A walker on a crossing is
  stranded at once (one record) and plans home from the end of the sidewalk it reaches. Vehicles
  keep the watchdog unchanged (`TRAFFIC-20`).
  - *A walker with no lane* (left at a node by a dropped lane, or off the network when a road edit
    removes its road and finds no sidewalk to reattach it to) takes the sidewalk it stands on
    before any replan gate (`walk_home::stand_on_sidewalk`): of the foot lanes into its node, the
    one whose end is nearest its position, ties by lane id, entered at its end, when that end is
    within a road edit's own reattach reach (`LANE_REATTACH_MAX_DIST_M`, 30 m). Its position is
    not changed; it is drawn at the lane's end from the next pose (0.1 to 4.2 m away for the 26
    walkers at a node among the bulldoze probe's 40 sampled events, below). Its route is dropped
    (its exit kept) and planned again from that sidewalk by the rules above. With no sidewalk end
    in reach it plans from its node when it stands within 30 m of it, and with no route is
    stranded there (no lane, reported at that node, the same backoff, its plans home from the
    node). Farther away, off every sidewalk, it walks to the nearest one (*Off the network*,
    below), or, with none within 200 m, is stranded where it stands, one `NoPosition` record,
    and its backoff only looks for a sidewalk in reach: it is never planned from a node it is not
    at. Before, all of these planned from the node: a walker whose road was
    bulldozed stood 128 to 351 m from the nearest sidewalk end at its node (7 walkers in an
    earlier build of the same probe) and was drawn there by its first lane, or after three
    failures moved home; a walker at a node could enter either sidewalk of its first road, or an
    exit lane across the road from it, with no crossing. A walker with no trip keeps standing.
    Cost: O(arms × lanes per arm) once, then the replan; a stranded walker off the network repeats
    the lookup once per backoff step.
  - *End of path.* A walker whose route ends at a node that is not its exit keeps its lane and
    plans again from its end (`junction/walk.rs::replan_from_lane`; no record when a route is
    found) instead of dropping the lane and planning from the node. One with no plan waits at the
    end of its lane; its replan gate plans from there once it has a destination. Its lane end has
    moved its route cursor one past its (usually empty) path, and a save keeps it there: the loader
    took that cursor for a stale route and dropped the walker's lane, then the orphan repair sent
    it home, so a loaded game diverged from the unsaved one (`PERF-13`, fixed 2026-11-04). A
    walker whose trip was taken from it on the way (its job ends mid-walk: `assign_work_building`
    clears its destination, exit and route) reaches its lane end with no plan, no destination and
    no trip to recover. With a home it plans home from that lane end and walks home
    (`walk_home::head_home_without_trip`, `TRAFFIC-24`, 2026-11-04): the give-up rule's walk home
    from where it stands, along sidewalks and crossings as any walk, with no record since no trip
    failed; with no route home it is stranded there on the stranded backoff (one `NoRoute` record,
    outcome `Stranded`). With no home it keeps waiting at its lane end for its household's next
    home. Before, nothing gave such a walker a destination while its home stood, so it waited for
    good and the walkers behind it queued there. The rule fires at the lane-end event, never on a
    scan: one route query per such walker, once (it leaves with a plan or stranded), and O(1) per
    tick for a homeless walker waiting there; walkers already waiting in an older save go home on
    their first tick. The 100k lane audit (one day at 32x on the `dev` snapshot) counts 24 to 26
    walkers with a home waiting so on `dev` and 0 with the fix (29 walk home), and none without a
    home in either run, so stepping off the lane while waiting is not needed. Tests:
    `a_walker_whose_job_ends_on_the_way_walks_home_from_its_lane_end_and_frees_the_sidewalk`
    (fails without the fix: the walker never sets off) and
    `a_walker_with_no_home_whose_job_ends_on_the_way_waits_at_its_lane_end`. A walker standing
    with no lane and no trip (the `stand_on_sidewalk` early return) is unchanged. Traced case
    (`TRAFFIC-19` probe, 100k baseline, default crossings): resident 140399 walks home from a shop
    when its household loses that home (`evict_household_member_home`), which clears its
    destination, exit and route on road 2365. It walks to the lane's end at node 790, where the
    path ends, and the old rule dropped the lane: it stood lane-less at the node with no trip, and
    once rehoused would have planned from the node (with the watchdog's three retries and move
    home behind a failed plan).
  - *Off the network* (round T19b, 2026-11-02, the user's decision; `walk_home::walk_to_sidewalk`).
    A walker off every sidewalk, its road removed under it and no sidewalk end into its node
    within 30 m, looks for the nearest sidewalk point within `OFF_NETWORK_WALK_MAX_M` (200 m,
    under 2.5 minutes at walking pace): the road edit's own reattach rule (the nearest point of
    any foot lane, ties by lane id) over the edges the road-edge R-tree finds in a box 250 m
    round it (`remap::nearest_sidewalk`; the box adds 50 m, more than any road's half width and
    sidewalk, because the tree holds centrelines). Independent of the tree's visiting order; no
    city scan, no allocation. It then walks there across the ground, one `NoPosition` record
    with outcome "walks N m to the nearest one": straight towards the point at the building access
    walk's pace (`AGENT_WALK_SPEED_MS`, 1.4 m/s, stepped by the access walk's
    `advance_along_local_access_path`), its speed that pace, never further in a tick. The access
    and egress walks themselves are tied to a building's door and entrance, so the walk is its
    own state: `ACCESS_WALK_TO_SIDEWALK` (access-flag bit 0x80, unused until now) with the target
    lane and distance in the attach-plan columns and the lane's edge as its edge. Its target is
    the lane's pose there (with the walker's sideways offset), so it steps onto the lane where it
    stands, at that distance, and plans its trip from the lane by the rules above: walks on, gives
    the trip up and walks home, or is stranded at the lane's end; with no trip it waits on the
    lane. No teleport: it is never placed anywhere it did not walk. A stranded walker off the
    network whose backoff runs out looks again, so a road built within 200 m later reaches it.
    The walk survives a trip-plan clear and a save (the bit and its columns are saved; both load
    validators keep it, as they keep a car's lane-attach hold); a road edit drops it, and a walk
    whose lane is gone or no longer on its edge is dropped at its next step, and the walker looks
    again from where it stands. Cost: one bounded R-tree query per walker that leaves the network
    (O(log edges + edges in the box × their lanes' vertices)), then O(1) a tick while it walks
    and one route query on arrival; every other agent pays one flag test a tick.
  - *Waiting for a home* (round T19b). The economy's rule for a household that loses its home
    (`households/relocation.rs::evict_household`, after failing its stay reserve for
    `stay_failure_days_before_eviction` with no affordable home nearby, or with no independent
    member) is: unhoused, it asks for a home through the hourly housing queue
    (`affordable_home`), and when none is found for `persistent_exit_max_unhoused_days` (7), or
    `persistent_exit_destitute_unhoused_days` (2) when destitute, it becomes exit-eligible and
    leaves through the departures queue, paced by the daily exit credit
    (`HouseholdExitPolicy`, `serve_departures`), which removes the household and every member
    wherever it is (`remove_households`, `kill_agent`).
    No walk or drive out exists for a departing household. So a walker whose trip home an
    eviction cancels (`evict_household_member_home`) waits at the end of its lane, as before,
    until one of those two happens: rehoused, it walks to the new home
    (`relocate_household_member_home` gives it the new home as its destination; the traced case
    test); departing, it leaves with its household as every member does. No new housing policy.
    The wait is no longer silent: the eviction puts the walker in the stranded set, and the report
    counts it as waiting for a home (a walker with no home, no destination and no next one, not
    stranded) until it is rehoused or gone; the notice adds "N residents waiting for a home"
    (`get_stranded_walkers_report`'s `waiting_for_home`). Cost: O(log set) per evicted walker,
    and the same report rebuild. A walker with no home whose job ends on the way
    (`assign_work_building` cancels its trip, `TRAFFIC-24`) waits at its lane end the same way and
    is put in the set the same way (2026-11-04, report-only: the economy envelope is
    byte-identical).
- **Measured (`TRAFFIC-19`, 2026-11-02; release test binaries of the control `3de19c2b` and the
  branch, each with R6's uncommitted crossings-off variant and the probe, one session; FRESH).**
  The probe is `populated_city::watchdog_probe` (ignored; `TRAFFIC19_RESIDENTS`, `TRAFFIC19_DAYS`,
  `TRAFFIC19_TOGGLES`, `TRAFFIC19_BULLDOZE`): a test-only tally of every trip-failure record and
  watchdog verdict by agent kind, lane state, site and outcome, off unless the probe turns it on,
  so no other run pays for it.
  - Fixtures, 3 game days after the build, `RAYON_NUM_THREADS=6`, 10k, 30k and 100k, default
    crossings and the variant: no watchdog verdict of any kind (walker, car or freight) and no
    trip-failure record, in both binaries. Vehicles reaching a path end that is not their exit:
    2,022, 4,105 and 9,733 (default), 2,034, 4,032 and 9,946 (variant), identical in the branch
    where it ran (all but the 100k variant). Walkers reaching one: only the traced case (100k
    default), which the branch keeps on its lane. The 100k variant run overflowed a background
    worker's 2 MB stack in the control (`metrum-bg-0`, while the variant toggled its 1,900
    crossing arms at build) and passed with `RUST_MIN_STACK=268435456`: a separate finding, not
    this round's.
  - Crossings turned off and on through the editor, a sixth of the four-way junctions every 30
    game minutes (13,500 toggles over 3 days at 10k): no watchdog verdict and no walker record in
    either binary.
  - A road bulldozed every 2 game hours (33 over 3 days at 10k): the control's walker verdicts
    were 2 retries and 1 move home (walker 9880, left 123 m out on a bulldozed road with its node
    87 behind it, planning from the node with no route). The branch has none: walkers left on
    bulldozed roads, 123 to 347 m along them from their nodes, are stranded where they stand and
    reported (14 `NoPosition` records from 7 walkers; a walker is recorded again when a later edit
    clears its plan), and the 26 walkers left at a node took their sidewalks 0.1 to 4.2 m away.
    Vehicles, the `TRAFFIC-20` evidence: control 65 cars sent home and 1,083 freight carriers sent
    to a border node (1,062 of them from the immigration bootstrap), branch 64 and 1,035; the runs
    diverge after the first walker's different outcome.
  - Envelope and determinism (`tools/sim02_gate.py envelope determinism --bursts
    departures,settlements`, `RAYON_NUM_THREADS=6`): all four runs PASS with the same 13 notes,
    deterministic (repeat, 1 and 3 threads, reload), no trip failure with default crossings and
    the variant's 0.014 and 0.007 per 1,000 residents a day at 30k in both binaries. The walks
    shift slightly after the first walker whose trip an eviction clears on the road: the branch
    keeps it on its lane at the path end and plans from that sidewalk once it has a destination,
    the control dropped the lane and planned from the node (14-day 10k probe: one such walker,
    agent 2750 on day 10, where the envelope's 10k baseline first differs; vehicle path ends then
    15,485 against 15,502). Control against branch, default: 10k baseline deliveries 172.26 to
    172.47 per 1,000 buildings a day, other metrics equal; 10k fiscal stress identical; 30k
    baseline deliveries 122.38 to 121.87, employment 0.6112 to 0.6101, net income 8.82 to 8.79,
    bankruptcies 1.77 to 1.80; 30k arrivals deliveries 127.44 to 126.40, net income 9.27 to 9.22;
    30k fiscal stress within 0.1%. Variant: 10k identical; 30k baseline deliveries 119.05 to
    120.10, net income 8.06 to 8.17, bankruptcies 1.91 to 1.92; 30k arrivals deliveries 119.2 to
    118.8.
  - `PERF-06` (`benchmark_agent_tick_contract`, C F F C, 5 samples a slot, `RAYON_NUM_THREADS=8`,
    P-cores, both locks, load 1.28): work counts identical in all 20 runs. Median ms per slot at
    10k/30k: network 0.49/0.90, 0.39/0.73, 0.59/0.78, 0.36/0.88; egress 0.49/1.60, 0.69/1.69,
    0.71/1.80, 0.79/2.26; ingress 0.42/0.45, 0.23/0.47, 0.31/0.64, 0.28/0.38. The slots scatter
    within each binary: no claim either way. Added per tick: one walker test per network agent
    without a lane before the gates; the rest runs only for a walker that has lost its lane or its
    route. No memory added: no column, no save change.
- **Stranded report.** `AgentVec`'s stranded walkers are a set (`tick/stranded.rs`): a walker
  enters with the record that strands it (O(new strandings) per tick) and leaves when the report
  finds it no longer stranded. The report, the junctions where stranded walkers wait and how
  many, is rebuilt from the set in the `gridlock check` slot (once a game minute, O(stranded ×
  log)), published in the render snapshot and read by `get_stranded_walkers_report`; the game
  shows "N pedestrians stranded at M junctions: no way home" under the gridlock notice and rings
  those junctions (`stranded_display.gd`, the gridlock display's poll and rings). Residents
  waiting on the road for a home after an eviction are a category of their own (round T19b):
  "N residents waiting for a home", joined to the first by a semicolon when both show; the
  display updates the notice whenever its text changes, the rings only when the junctions do. The stranded
  bit, step and time are saved columns already (no new column, save format unchanged at 110;
  the loader now keeps the bit, which it used to clear as unused); loading rebuilds the set and
  the report from them (one O(agents) scan).
- **Cost.** The witness is O(roads × sidewalks per road × connectors per lane) once per route,
  in thread-local scratch, with no allocation once the scratch and the walker's column have grown
  to its longest route; a junction step is one scan of the lane's connectors. Memory: one 24-byte
  `Vec` header per agent plus one byte per road of a walker's route, about 2.4 MB of headers and
  roughly 1-2 MB of witnesses at 100k agents (39,000-46,000 walkers on routes of a few dozen
  roads, `SIM-02-E`).
- **Measured (round R6b, 2026-11-01; release test binaries of the control `987c0ab6` and the
  branch, each with R6's uncommitted crossings-off variant behind `METRUM_R6_CROSSINGS_OFF`;
  `tools/sim02_gate.py envelope determinism --bursts departures,settlements`,
  `RAYON_NUM_THREADS=6`, one session).** Variant (every third four-way junction's crossings
  off, 208 toggles at 10k): trip-failure records per 1,000 residents a day, control against
  branch, 12,172 / 0 (10k baseline), 9,519 / 0 (10k fiscal stress), 14,570 / 0.014 (30k
  baseline), 12,028 / 0.007 (30k fiscal stress), 14,043 / 0 (30k arrivals). No walker fails:
  the branch's remaining 6 and 3 records over 14 days come three at a time on single days, as
  the control's do with default crossings (6, 3 and 3 at 30k baseline, fiscal stress and
  arrivals). An 11-day run of the 30k variant city logging every record (an uncommitted probe)
  finds the 6: two cars, each two retries and the watchdog's recovery, one a freight carrier
  with no road path to its border node, `u32::MAX`, sent to a border node, and one travelling
  with no plan, sent home; a 3-day run at 10k logs none. The variant's trip-failure notes are
  gone; its other notes are the default runs' 13.
  Default crossings: both pass with the same 13 notes, deterministic (repeat, 1 and 3 threads,
  reload); the branch has no trip failure (control 6, 3 and 3 at 30k) and walks shift: 30k
  baseline deliveries 117.3 to 122.4 per 1,000 buildings a day, employment 0.601 to 0.611,
  net income 8.27 to 8.82 per resident a day, bankruptcies 1.90 to 1.77; 10k fiscal-stress
  deliveries 8.48 to 8.87. Cause (inferred, not traced walker by walker): the witness takes the
  cheapest sidewalk chain to the exit (crossing where the route needs it) instead of the
  shortest connector at each node, so a walk's connector total is never longer than the old
  choice's, and the default network has incomplete junctions too (a degree-two node has one
  crossing, D8), where the old choice could also strand a walker; no other per-agent rule
  differs while a route exists.
- **`PERF-06` (same session, `RAYON_NUM_THREADS=8`, P-cores, load 1.37, C F F C).** Work counts
  identical between the binaries and exactly 3× from 10k to 30k (lane counts 16). Network tick
  ms at 10k/30k: control 0.69/1.90 and 0.44/0.49, branch 0.29/0.58 and 0.25/0.75; access egress
  0.54/1.41, 0.64/1.69 against 0.45/1.37, 0.59/1.24; ingress 0.24/0.45, 0.36/0.47 against
  0.34/0.53, 0.25/0.42. Single samples, scatter within each binary: no claim either way. The
  contract's fixture has no junction, so the witness itself is not timed there; its cost is the
  bound above, once per route.
- **Tests.** `simulation::economy::agents::tests::walk_witness`: the traced case (a walker whose
  exit needs the far sidewalk takes the longer connector across the road end; it fails under the
  old choice), a removed crossing replans round a road end without a teleport, walks identical
  for 1 and 8 workers. `tests::walk_home` (a block whose crossings are all off splits its inner
  and outer sidewalks): a walker whose shop becomes unreachable gives up and walks home; with no
  way home it is stranded, waits at its lane's end on the 5 s / 30 s / 2 min backoff with one
  record, the report names its junction, and it walks home once the crossings return; a walker
  already walking home is stranded at once; the report counts walkers per junction, identical for
  1 and 8 workers. `test_walking_missing_crosswalk_route_waits_at_junction_mouth`: a walker with
  no destination and no home is stranded where it stands, one record, on its backoff.
  `nodes::sim::core::tests::stranded_fixture`: the stranded block city saved mid-backoff and
  loaded continues frame for frame as the unsaved one (walkers, timers, report); its ignored
  recorder writes the fixture `godot/tests/stranded_notice_test.gd` loads into the real game
  scene and `SimulationNode` (`./run.sh --test`).
- **Tests (`TRAFFIC-19`; each fails at `3de19c2b`).** `tests::walk_home`: a walker at a node with
  no lane takes the sidewalk it stands on (not moved), gives up the unreachable shop and walks
  home, one record; a walker at a node with no sidewalk is stranded there, reported at that node,
  plans home after 5 s, 30 s and 2 min, and walks home once the road gets sidewalks; a walker off
  every sidewalk is stranded where it stands, never moved, one `NoPosition` record; the traced
  case, an evicted walker that keeps its lane at its path end and, rehoused, walks from it to its
  new home; a crossing-free block with walkers left at every node and on sidewalks, no watchdog
  move and no step over 2 m on the road, identical for 1 and 8 workers. `stranded_fixture`:
  walkers stranded at nodes without a lane (two that took their sidewalks, one at a node with
  none) saved mid-backoff and loaded continue frame for frame; since `PERF-13`, two walkers whose
  trip ended mid-walk, one waiting at its lane's end and one behind it, do too
  (`walkers_kept_at_their_path_end_continue_across_a_save_file`, fails at `d0b29ce6`). The
  watchdog's own test now drives a car, and
  `trips::test_walking_agent_recovers_from_stale_edge_after_road_split` expects its walker at a
  dead end to turn back through the node's connector from the sidewalk it stands on.
- **Measured (round T19b, 2026-11-02; FRESH, one session; release test binaries: the control
  `br4`, `edb8a46f` with R6's variant and the probe, whose non-test Rust equals `8baa904f`'s, and
  the branch at `d4d577fb` (built before one doc comment was rewrapped) with the same; `RAYON_NUM_THREADS=6`, the default thread stack).**
  - Bulldoze probe (10k, a road every 2 game hours, 33 roads, 3 game days): the control strands
    7 walkers off the network (14 `NoPosition` records, walkers 123 to 347 m along their deleted
    roads); the branch has none: those 7 walk to a sidewalk (7 starts, 7 arrivals) and plan from
    it (walker 9880, the traced one, then walks home). At the end no walker is stranded with no
    lane, at a node or off the network, none is still walking and none waits for a home. No
    walker teleport in either binary's watchdog tally. Vehicles (`TRAFFIC-20`): control 64 cars
    sent home and 1,035 freight carriers sent to a border; branch 66 and 1,035, the runs diverging
    after the first walk.
  - 100k crossings-off variant (2,572 crossing arms toggled at build, 3 game days): runs to the
    end on the default stack (`CODE-18`; the control, run once under gdb this session, overflows
    `metrum-bg-0` at build, 156 rebuilds deep), with 1,260 routing rebuilds at build (the
    `TRAFFIC-19` control's 256 MB-stack run, historical: 2,520; superseded deferred starts run
    nothing); no watchdog verdict, no trip-failure record, 9,946 vehicle path ends short of the
    exit (that historical run: 9,946), 0 stranded, walking or waiting walkers.
  - `PERF-06` (`benchmark_agent_tick_contract`, C F F C, 5 samples a slot, `RAYON_NUM_THREADS=8`,
    P-cores, both locks, 1-minute load 1.75): work counts identical in all 20 runs. Median ms per
    slot at 10k/30k: network 0.40/0.75, 0.28/0.63, 0.33/0.72, 0.49/1.11; egress 0.84/1.44,
    0.65/1.95, 0.77/1.68, 1.05/1.46; ingress 0.25/0.53, 0.43/0.38, 0.40/0.31, 0.22/0.38. Scatter
    within each binary: no claim either way. Added per tick: one flag test per network agent.
  - Envelope and determinism (`tools/sim02_gate.py envelope determinism --bursts
    departures,settlements`, `RAYON_NUM_THREADS=6`), default crossings and the variant: all four
    runs PASS with the same 13 notes and the same values, deterministic (repeat, 1 and 3 threads,
    reload), and every day's fingerprint of every workload is identical between control and
    branch (trip failures 0 with default crossings; 6 and 3 at 30k with the variant in both). No
    envelope workload removes a road under a walker, and the waiting-for-home report is derived,
    so the economy is unchanged.
- **Tests (round T19b).** `tests::walk_home`: the bulldoze case, a walker 50 m from both nodes of
  its removed road, more than 30 m from every sidewalk end, walks to the nearest sidewalk at no
  more than 1.4 m/s × 0.1 s a tick (over 30 m), steps onto a foot lane and walks on to its shop,
  one record, no teleport (fails at `8baa904f`: stranded in place); a walker 300 m from every
  sidewalk, beyond the bound, is stranded where it stands, never moved, reported; the traced
  eviction case now also checks the report counts the waiting walker as waiting for a home, not
  stranded, and drops it once rehoused (the category does not exist at `8baa904f`).
  `stranded_fixture`: a walker off every sidewalk in the middle of the block, saved mid-walk and
  loaded, continues frame for frame until it is on its sidewalk. `pure_job`: 10,000 chained jobs
  on a one-worker pool with a 2 MB stack run one at a time (the control aborts with a stack
  overflow). Godot `stranded_notice_test.gd`: the notice's "N residents waiting for a home"
  alone and beside the stranded count, and Rust's `waiting_for_home` on the block (0).

### Detach rule (`TRAFFIC-15`, `TRAFFIC-13`)

A car leaves the network for its destination's ingress in `try_network_detach`
(`tick/movement/network/detach.rs`), under one rule:

- It is on its planned detach lane, with a valid access plan.
- Its movement step has reached the planned detach distance. A step that ends inside the lane
  reaches the car's new `lane_d`. A step that reaches the lane's end passes every point left on
  the lane, so it reaches `length + ACCESS_LANE_D_TOLERANCE_M` (1e-4 m), the same bound the
  planner's access legality (`access/legality.rs`) accepts for `planned_detach_lane_d`. Every
  legal detach point is therefore reachable, the lane's end included, and the end-of-lane try
  runs before `handle_lane_end`.
- The route is on its last visit to the lane: no node of `current_path` is left beyond the lane's
  terminal node (`path_idx` names the next node, the lane's terminal while the car drives toward
  it, the same reading as `planned_next_connector`). A route that leaves its detach lane and comes
  back to a door behind the car does not detach on the first visit.
- A car claims the detach slot (`claim_lane_entry` on the detach lane). While another car holds
  it, the car waits at `min(planned_detach_lane_d, length)` with speed 0.

The speed pass does not brake for a detach point, mid-lane or at the lane's end: a car detaches in
the step that reaches the point, at the speed it has. Road-lane junction braking does not apply to
a car on its last visit to its detach lane either, since `planned_next_connector` finds no next
connector for it. Cost: O(1) per detach try (one terminal-node lookup), no allocation; the
end-of-lane try runs once per road-lane end event.

Before this rule a step had to end strictly inside the lane, so a door that projects onto the
lane's end (a corner store at `frontage_t` 0 seen from the opposite lane), or a step that crossed
the detach point and the lane's end together, was never reached: the car left the lane with a
finished route, replanned at the end node and drove round the block for ever. And a route that
came back to a door behind the car detached on its first visit and jumped back to the door.

Acceptance record (`TRAFFIC-15`, `TRAFFIC-13`, round R14B, 2026-10-31; commands and results, the
artifact folders stay local):

- Regressions (`agents::tests::detach`, debug test profile): `corner_store_door_at_the_lane_end_is_reached`
  drives 8 shoppers on each of 64 streets toward a corner store at `frontage_t` 0 of a 30 m
  street, whose door is the end of the lane they arrive on (`planned_detach_lane_d` = length).
  On `dev` the first shopper leaves that lane without detaching ("left its door's lane without
  detaching"); with the rule all 512 detach from that lane, and 1 and 8 workers give
  bit-identical outcomes. `looping_route_detaches_only_on_its_last_visit` attaches at 950 m of
  a 1 km lane with its door at 50 m of the same lane, route `[1, 0]`: on `dev` it detaches on
  the first tick; with the rule it drives to node 1, back to node 0 and detaches on the return.
- 100k built from scratch to day 12 (`lane_entry_audit::rings::traffic14_ring_formation`,
  release test binaries, `RAYON_NUM_THREADS=8`, cores `0,2,..,14`, one run at a time; wait-for
  cycles checked every 30 game minutes from day 1 08:00). Control: `dev` code with this round's
  diagnostics. The `TRAFFIC-15`/`TRAFFIC-13` commit alone:
  - `dev`: a 334-car ring from day 9 21:00 (absent at the day 10 07:30 check, back at 08:30,
    present at day 12 00:00). At day 12, 2,375 vehicles on the road plan a detach at their
    lane's end, which `dev` cannot reach (top targets: buildings 57645 578, 57646 555, 57644
    547, 57650 284, 57654 155); 68,955 vehicles on the road, 2,822 held at nodes, 140 at attach
    points.
  - With the rule: **0 cycles** at every check to day 12 00:00. At day 12, 491 vehicles plan a
    lane-end detach, all reachable (0 plan one beyond the end); 66,759 vehicles on the road,
    2,027 held at nodes, 60 at attach points.
- Looping routes (`TRAFFIC-13`), vehicles on the road whose remaining route visits the detach
  lane before its last visit, counted at each check: 0-2 during the `dev` build and 2-4 per
  6-hour checkpoint of the `dev` 100k day; 13 at day 12 with the rule (they now keep their
  route instead of detaching early).
- Envelope and determinism (`tools/sim02_gate.py envelope determinism --bursts
  departures,settlements`, `RAYON_NUM_THREADS=6`, same session as the `dev` control): PASS,
  determinism identical (repeat, 1 and 3 threads, reload). Doors are reached, so shopping and
  trips shift. 10k baseline out-of-stock 0.194 to 0.111, back inside its band (the `TRAFFIC-07`
  note is gone). 30k baseline: employment 0.602 to 0.623, final employment 0.544 to 0.583, net
  income 8.58 to 9.89, bankruptcies 1.65 to 1.26, deliveries 129.4 to 135.7, out-of-stock
  0.236 to 0.207, which is below its band: a new note in the good direction. 30k fiscal stress
  deliveries 10.97 to 10.90, a new note 0.1% under its band edge (10.91). Arrivals-30k:
  employment +2.3%, net income +7.4%, bankruptcies -10.2%, out-of-stock +7.4% (0.225 to
  0.242). 10k fiscal stress moves by at most 0.5% on its main metrics apart from final
  employment (0.0059 to 0.0052). 14 notes against `dev`'s 13, all of the stale baseline.
- `PERF-06` agent contract (`tools/perf_contract.py --agents`, 8 workers, cores `0,2,..,14`,
  exclusive timing lock, load average 1.7 at the start; control `dev` then the branch with both
  this rule and `TRAFFIC-16`, then the branch, then control): identical work counts in all four
  runs. Single samples, ms, control / branch / branch / control: network 10k 0.666 / 0.839 /
  0.280 / 1.226, 30k 0.840 / 0.638 / 0.620 / 2.046; egress 10k 0.540 / 0.475 / 0.485 / 0.795,
  30k 1.252 / 1.281 / 1.699 / 1.312; ingress 10k 0.705 / 0.295 / 0.521 / 0.194, 30k 0.340 /
  0.273 / 0.318 / 0.493. They scatter within each binary as widely as between them: no speed
  claim either way. The contract's fixture has no connector and no lane-end detach; the added
  work is one O(1) detach try per road-lane end event.

### Give-way on entry to a roundabout (`ROAD-62`)

Round R7c (2026-11-06, branch `port-r7c-giveway`, landed on `dev` 2026-11-06). A car entering a
[roundabout](roads.md#roundabouts-road-62) gives way to the ring, without waiting for `TRAFFIC-06`;
every other junction still has no right of way.

**Data.** The connector builder gives each connector from an arm onto a ring lane a `RingEntry
{ circulating, ring_in }`: the connector from the ring lane into the same ring lane (a two-lane
ring's lanes run 1:1) and the ring lane it leaves. Both sit at the entry's node, so the rebuild that
replaces one replaces the entry too. The pair lives in a table on `LaneSystem`
(`LaneSystem::ring_entry(id)`) and `Lane::ring_entry` is a four-byte index into it: two ids on `Lane`
took every lane from 104 to 112 bytes and cost the 65,536-group transition rows 2-3.5% in L1d
misses, ring or no ring, where the index fits the struct's padding (the crosswalks' reasoning: few
lanes need it). Never saved: lanes are rebuilt on load. A retired connector clears its index; the
table's stale rows go with the next whole rebuild, as tombstoned lanes do. It is the fork's
`ControlledMovement.yields_to` restricted to ring entries; `TRAFFIC-06` can later fill the same
shape for priority roads and yield signs.

**Rule** (`ring_entry_blocker`, `tick/traffic/occupancy.rs`). An entry waits while, in last tick's
lane buckets:

- a car is on the circulating connector;
- the ring lane's head is arriving: within `CAR_LENGTH + IDM_S_MIN + 20 km/h x IDM_T_HEAD` (5.48 m)
  of the lane's end and not standing at it;
- the ring lane's entry slot is blocked ("room ahead": `lane_entry_slot_clear`, the test a connector
  head makes at its exit, not the box rule the user dropped in round R14C).

`ring_entry_clear` is the same test as a yes or no. It is a pure function of the buckets: `O(log
bucket)`, no allocation, one read of the lane's index on an ordinary connector. The gap uses the ring's limit,
not the head's speed: the speed pass writes the speed column while it runs this test, so another
car's speed is not a stable input.

**Three readers.** The entry claim (`claim_connector_entry`'s candidate filter, so every connector
entry passes through it), the gridlock check (`entry_state`, which names the lane in the way) and the
speed pass's lane-end hold, which reads `entry_state` as well: a one-lane arm fans onto both lanes of a
two-lane ring, so the hold must test the claim's candidates, not the first connector. A car with no
lane at a ring node takes a road lane directly, as at any node (`first_entry_is_held`): no connector
is involved and nothing yields. Nothing but a replan after a failure puts a car there.

**Why it adds no lock.** An entry stands only for the ring lane it enters (the third hold, and the
first when the circulating car itself waits for that slot) or for a moving car (the second hold, and
the first otherwise). That is the wait a plain junction's connector head has at its exit, moved back
to the stop line, so a yielding head adds no edge to the wait-for graph that a plain junction lacks. The
head **standing** at the ring lane's end is excluded for that reason. Round R7c's first build followed
the plan's text, any head within the gap, and the populated day locked: with rings a block apart, a
link's head yielded to a ring head that was itself waiting to leave onto the opposite link, and the
pair of 3 m links, their exit connectors and the two ring lanes formed a cycle around nothing. The
check confirmed and detoured it 2,749 times in the day with 0 saturation reports, and the city
delivered 92 loads: a day that met the gate's two numbers and did not run. A standing head waits
either for the circulating connector or the ring lane's slot, which the rule tests anyway, or for
its exit, in which case it is not coming.

The ring itself can still fill: a ring locks when every head on it wants to go on and the next lane
is full, or when its exits back up. Those are ordinary `TRAFFIC-14` cycles of ring lanes and
circulating connectors, detoured through an exit or reported as usual. Giving way does not prevent
them, and does not cause them.

**Starvation** is real behaviour and is reported, not gated (the watchdog never moves a car): under
steady circulation an entry waits, and route search is free flow (`TRAFFIC-17`), so a starved entry
does not reroute. Crossing movements on the ring itself have no conflict test: the inner lane's exit
crosses the outer lane as a left turn crosses at any junction today (`TRAFFIC-09`).

Acceptance record (round R7c, 2026-11-06; release test binary of `51d68ccc`,
`traffic07_lane_entry_audit` one day at 32x, 60-minute samples, `METRUM_PERF_ROUNDABOUTS=8` on the
10k city and its 4-Lane variant, each against a ring-free run of the same city and binary; the
knob's rings go through the tool's transaction, and the prepared snapshots pass their continuation
check; artifact folders stay local):

| 10k day | 1+1, 8 rings | 1+1, no ring | 4-Lane, 8 rings (two-lane) | 4-Lane, no ring |
| --- | --- | --- | --- | --- |
| Saturation reports / detours / cycles (peak) | 0 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| Trip failures, all / with a ring in the cause or at the car | 0 / 0 | 0 / - | 0 / 0 | 0 / - |
| Freight deliveries | 710 | 1,385 | 965 | 1,863 |
| Held at nodes, peak / end of day | 104 / 84 | 260 / 260 | 486 / 438 | 163 / 140 |
| Vehicles on lanes, peak | 2,418 | 2,869 | 3,006 | 3,082 |
| Missed turns | 0 | 0 | 65 | 10 |
| Stands at an entry: count, p50 / p95 / max (simulated s) | 1,424, 1.1 / 10.1 / 63.5 | - | 1,193, 1.1 / 12.8 / 68.8 | - |

The gate is the first two rows; the rest are measurements (`TRAFFIC-17`). A game day is 1,440
simulated seconds. The audit's oracle (`wait_cycles`, which follows a yielding head by its own test)
agrees: 0 cycles at every sample. 1, 4 and 8 workers give the same 51 report lines and the same hash
of every car's lane, distance and speed. Three days from the same snapshots: 0 / 0 / 0 and 0 / 0 on
both cities. `n` swept over 1, 2, 4, 8, 16 and 32 on both cities: 0 saturation reports, 0 detours, 0
cycles and 0 trip failures in all twelve, the 1+1 city's rings a block apart with 3.3 m links from
n = 3.

Deliveries roughly halve, and nearly all of it is one ring: the first junction in the knob's order is
the border trunk's, and that ring alone gives 769 (1+1) and 1,034 (4-Lane); 32 rings give 543 and
1,003. All border freight takes one path through it (`TRAFFIC-17`), and a plain junction has no
conflict test to pay for. It does not catch up: over three days the ring-free cities deliver 4,383 and
4,449 and clear their border queues (1 held car at the end), the 8-ring cities 2,137 and 2,851, with
66 and 588 held. A single-lane ring at 20 km/h on the only border trunk is a capacity limit of this
fixture, not a lock ([measured](#what-a-ring-carries-road-62-round-r7c2)).

Most of that is the ring, not the yielding. A measurement build with the rule switched off: 864
deliveries with the one ring, 605 with eight (710 with give-way), 1,333 on the 4-Lane city (965), and
the rings then do form cycles the check has to break: 74 and 117 detours on the 1+1 city, with held
peaks of 376 and 437 against 117 and 104. So give-way costs 11% at the one ring and 28% on the two-lane
rings, gains 17% across eight one-lane rings, and buys a ring that does not cut itself off.

Tests: `give_way_tests` (the three holds and the standing head; an ordinary connector owes nothing; a
head that enters only after the circulating car passes, 1 and 4 workers equal; the gridlock check's
wait-for edge; the fork's drain test, six uneven arms and 36 cars the long way round, on a one-lane
and a two-lane ring at 60 m, the fork's 70 m being over the conversion cap, 1 and 4 workers equal),
`roundabouts::tests` (every entry marked on 1+1, 2+2 and fan arms; an incremental rebuild),
`road_roundabouts` (a load; queues crossing two neighbouring rings both ways, the lock above).

Timing (2026-11-06 06:49, `.run.lock` and the exclusive timing lock, cores `0,2,..,14`, no cargo,
rustc, godot or restic, load 10.8 falling from the gate runs; control `6e1ebef7`, branch `51d68ccc`;
`$ARTIFACTS`-local tables): `PERF-06` (`tools/perf_contract.py --agents`, 8 workers, C B B C twice, then
12 alternating pairs of the test itself): work counts identical in every run, every row's ranges
overlap; medians of the 12 pairs, control / branch, ms: network 0.327 / 0.326 and 0.798 / 0.778 at
10k / 30k, egress 0.903 / 0.618 and 1.839 / 2.002, ingress 0.259 / 0.321 and 0.531 / 0.445. Its
fixture has no connector, so it does not reach this code. `benchmark_lane_transitions` (4 workers, 3
pairs, transition counts equal in every row), medians at 65,536 groups: clear +0.4%, merge +0.5%,
turn +1.4%, cruise -3.9%, inside the control's range or touching it; **blocked +1.8% and lateral +4.4%
outside it**. `benchmark_claim_preparation` checksums equal, 1,048,576 mixed 0.560 / 0.552 / 0.493 /
0.492 ms (C B C B). `perf_contract_day` on one private set of snapshots prepared by the control
(the control with the branch's fixture file, whose bytes key the snapshots), 8 workers, C B B C: record and work
counts identical; hourly 0.86 / 0.90 / 0.87 / 0.85 ms at 10k, 4.34 / 4.33 / 4.38 / 4.26 ms at 30k;
total 3.04 / 3.05 / 3.05 / 3.02 s and 10.60 / 10.57 / 10.57 / 10.55 s.

The rows outside spread, fixed-iteration `perf stat` of each row alone (the same test-only row filter on
both sides, C B B C): blocked 3.93 / 3.95 G instructions, 2.15-2.18 / 2.16-2.18 G cycles, overlapping;
lateral 7.19 / 7.18 G instructions, 1.176 / 1.181 G branches, 35.6 / 35.1 M L1d misses, but 2.95-2.96 /
3.05-3.07 G cycles. So lateral does no more work and takes 3.5% more cycles: a cycle profile puts
+360 of the +377 samples on `sibling_vehicle_lane_with_idx`, a function this round did not touch.
Round R7c2 tested the field shift below and it is not the cause. The whole benchmark runs 92.81 /
92.86 G instructions, cycles overlapping (39.97-40.64 / 40.20-40.51 G).

**The lateral row (round R7c2, 2026-11-06, fresh; lock, cores and workers as above).** `offset_of!`:
`Lane` is 104 bytes on both sides; only `is_fwd`, `lane_type` and `lane_idx` move, 96 / 97 / 98 to
100 / 101 / 102, with `ring_entry` at 96. A `repr(C)` build with the control's offsets and
`ring_entry` at 100 does **not** bring the row back: 65,536 groups, medians of six runs each in one
session (C, R7c's binary, the branch with this round's test code, the variant, in mirrored order):
5.234 / 5.540 / 5.231 / 5.476 ms. So the control's offsets are slow in one binary, and the moved
offsets are as fast as the control in another whose non-test source equals R7c's, with the same
instructions in the function. 17 binaries of this hot code fall into two groups, 5.16-5.22 ms and
5.34-5.56 ms, and both offset layouts appear in both (the three real control builds fast, the
control-offset variant slow, branch builds on either side); the function's address modulo 64 does
not sort them. The variant was dropped. Fixed-iteration counters on R7c's filtered builds, a fast
control and a slow branch binary (C B B C): cycles +2%, L1 load misses -2%, MITE uops
-13%, DSB uops -1%, store-forward blocks -5%, but stall cycles on L1d misses +5.6% and load-bound
cycles +5.3%. So it is no front-end loss and no extra work: the same loads overlap less well in
some link layouts. Which placement does that is not known. The row is link-layout variance of
about 5% on identical source, not a cost of the field, and it says little about either side.

### What a ring carries (`ROAD-62`, round R7c2)

Round R7c2 (2026-11-06, branch `port-r7c2-capacity`, landed on `dev` 2026-11-06). Fixture `ring_capacity_tests`: no
city, four arms, a source on every entry lane that places a car whenever the lane's entry slot is
clear, a sink near the end of each exit arm. Flows are cars per **simulated minute**, 600 s after a
120 s warm-up; the median and range of nine starts, because the symmetrical fixture settles into
different periodic orbits (one case: 72 to 82). Turns are by exit in the ring's sense of travel:
first = right, third = left. 1x is the agent step at game speed 1 (1/60 s), 32x the step of the day
runs (0.533 s). `$TEST_BINARY simulation::economy::agents::tick::ring_capacity_tests::measure_capacity
--ignored --nocapture`, `METRUM_CAPACITY_TRACE=1` for the lanes. All numbers fresh on `409272b9`;
the rule-off columns come from a measurement build that is not committed.

What the constants allow. The steady bumper gap at speed `v` under limit `v0` is `(s0 + vT) / sqrt(1
- (v / v0)^4)`, so a lane's flow `v / (L + gap)` tops out at:

| Limit | 13.89 m/s (50 km/h) | 6.00 (straight connector) | 5.56 (ring) | 4.47 (right turn) | 4.07 (ring entry and exit, 36 m) |
| --- | --- | --- | --- | --- | --- |
| Top of the curve, cars/min | 70.5 | 49.6 | 47.7 | 42.5 | 40.3 |

A queue starting from rest does not reach that on a free road: it discharges at 55.1 at 50 km/h and
40.7 (40.1-41.4) at 20 km/h. A connector is too short to settle, so a short turn runs slightly over
its figure. A ring of `a` arms whose cars each use `s` of its `a` sections carries at most `a x
lane / s`: with a lane of 49, 98 for through traffic, 65 for the third exit.

| Cars/min, all arms queued unless noted | 1x | 32x | No give-way, 1x | No give-way, 32x |
| --- | --- | --- | --- | --- |
| Road 1+1 at 50 / 20 km/h, one lane | 55.1 / 40.7 | 56.3 / 35.4 | | |
| Road 2+2 at 50 / 20 km/h, both lanes | 110.2 / 81.3 | 112.5 / 70.7 | | |
| Junction 1+1 through, one arm / four | 49.7 / 199.0 | 49.8 / 199.0 | | |
| Junction 1+1 right / left, four arms | 177.9 / 188.8 | 168.1 / 179.8 | | |
| Junction 1+1 mixed (arm's own discharge, no queue) | 220.5 | 225.0 | | |
| Junction 2+2 through, one arm / four | 99.0 / 396.2 | 95.7 / 383.1 | | |
| Junction 2+2 right / left (one lane each) | 180.1 / 197.3 | 168.9 / 192.0 | | |
| Junction 2+2 mixed (no queue) | 440.9 | 450.0 | | |
| Ring 36 m one lane, one arm through | 44.3 | 37.5 | 44.3 | 40.2 |
| through, four arms | 72.7 (72-82) | 66.4 (62-67) | 97.9 | 8.7 (8-12) |
| first exit | 140.0 (130-140) | 146.3 (135-146) | 177.1 | 160.7 |
| third exit | 51.9 (51-58) | 43.5 (43-47) | 10.5 (8-12) | 8.3 |
| mixed, a third each | 66.7 (66-68) | 56.6 (56-57) | 97.9 | 9.6 (8-40) |
| Ring 50 m two lanes, one arm through | 90.5 | 82.7 | 92.3 | 89.2 |
| through, four arms | 143.2 (138-146) | 132.3 (124-133) | 140 (108-194) | 41.5 (20-80) |
| first exit (arm's own discharge, no queue) | 220.4 | 225.0 | 220.5 | 225.0 |
| third exit (inner lane only) | 54.3 (54-58) | 39.4 (34-42) | 11.1 | 8.4 |
| mixed | 132.0 (132-135) | 117.0 (115-122) | 157 (141-172) | 138 (126-146) |
| Ring 50 m, 2+2 and 1+1 arms (the fan), through / mixed | 152.7 / 115.7 | 128.7 / 103.4 | 191 / 177 | 113 / 77 |

**One stream.** A single arm on a ring is what a plain right turn is: 44.3 against 44.5, because
every car entering a ring makes a right turn on a 4.07 m/s connector. Removing either the connector
speed or give-way alone changes nothing at 1x, removing both gives 49.0, a straight connector's
figure: the third hold, read at the line 9.5 m before the ring lane, lands on the same headway.

**All arms.** Without give-way a ring reaches its bound (97.9, a lane of 49). With it the ring lanes
are not full: in the mixed case they run 33.8 cars/min at 5.0 m/s, 71% of a lane, and the entries
stand. A trace of the heads at the line, through traffic at 1x: 82% of the ticks wait for a car on
the circulating connector, 13% for an arriving head, 4% for their own connector, 1% free. That
first hold closes for the whole connector, 10.75 m on the 36 m ring and 15-17.5 m on the 50 m, 2 to
3 s a car. Each hold switched off alone, in a measurement build, through / mixed at 1x:

| One lane | Rule | No circulating hold | No arriving-head hold | No slot hold | No rule |
| --- | --- | --- | --- | --- | --- |
| through | 72.7 | 88.3 | 87.5 | 81.4 | 97.9 |
| mixed | 66.7 | 89.9 | 77.5 | 78.4 | 97.9 |
| first exit | 140.0 | 140.0 | 177.2 | 158.7 | 177.1 |

So the rule takes a quarter to a third of a saturated ring and is what keeps it running: without it
the third exit falls to 10, and at 32x the one-lane ring's through and mixed cases fall to about 9. The first
exit's whole loss is the arriving head that leaves anyway (R7c's decision 8).

The city is less sensitive to any one hold than the fixture: a one-ring day from the same snapshot with one hold off gives 791, 830 and 841 deliveries on 1+1
(rule 769) and 1,114, 1,171 and 1,075 on 4-Lane (rule 1,034), 0 detours each; all three off 471 with
17 detours, and 1,243. R7c's rule-off day (864) was prepared without the rule as well, so the two
differ; days with the rule off are not stable. No hold brings the trunk near the ring-free 1,385 and
1,863.

What was ruled out, each on the same fixture: entry and exit connectors raised to 6 m/s: mixed 67.8,
through 82 (66-82), ranges overlapping; braking at ring nodes removed: 74.6 and 66.8, inside the ranges; lane-end
claims: 0 lost at 1x, 60 of 2,300 at 32x; a connector head waiting at the ring lane's slot: 5,464
ticks over 2,888 exits at 1x, 0.03 s a car; the fan: no loss of its own. **No defect was found on
ring lanes or connectors**, so nothing outside test code changed. Ring connectors do allow 6 m/s, 21.6
km/h, over 40% of the ring, above the ring's limit; it does not limit anything.

**Two lanes against one.** A two-lane ring carries twice a one-lane ring: 2.04 for one stream, 1.97
through, 1.98 mixed at 1x; 2.2, 2.0, 2.1 at 32x. The third exit, on one lane by construction, gains
nothing.

**What a plain junction does.** Four crossing through streams carry 199.0, exactly four times one
stream: they drive through each other. That is the missing conflict of `TRAFFIC-09`, not capacity.
Only two opposing streams can run at once, 99.4, before any lost time, so a one-lane ring's 72.7
through is about three quarters of what a crossing with conflicts could carry at best.

**The game speed.** At 32x the step is 0.533 s and a slot test reads the last tick, so one stream
onto a 4.07 m/s connector takes exactly three ticks a car: 37.5. At 6 m/s it gets 47.0. Speeds 1 to
16 give 44.3 to 46.9, 32 gives 37.5. A plain through stream does not lose (49.7, 49.8).

**The city.** `METRUM_AUDIT_JUNCTION=<nodes>` on `traffic07_lane_entry_audit` counts cars into each
connector by game hour (fresh, 32x, R7c's snapshots; a game day is 24 simulated minutes):

| Trunk junction, one day | 1+1, plain | 1+1, one ring | 4-Lane, plain | 4-Lane, one ring |
| --- | --- | --- | --- | --- |
| Deliveries | 1,385 | 769 | 1,863 | 1,034 |
| Those whose carrier crossed it | 1,221 (88%) | 610 (79%) | 1,703 (91%) | 875 (85%) |
| Cars through it (ring: entries), per minute | 2,853, 119 | 1,517, 63 | 3,751, 156 | 1,934, 81 |
| In from the trunk | 1,090 | 580 | 1,655 | 519 |

Every ring-free delivery but the local ones crosses this one node. With a ring the entries are flat
at every hour of the day, the trunk's lane holds 136-159 cars at all 24 hours (the 4-Lane city's
141-168) and the row's arm up to 122: **the ring is saturated all day**, at 63 cars a minute against
the fixture's 57-66 at 32x, and 70.5 at 1x against 67-73. The fixture and the city agree. Offering
the ring-free day's movements to the fixture in steps (`measure_city_movements`) gives the ceiling for
that traffic: the plain junction takes all of 140 to 150 a minute, a one-lane ring 47-50 at 1x and
39-40 at 32x, a two-lane ring with lanes used evenly about 110-125 at 1x and at best 110 at 32x,
unstable above that, against 119 and 156 wanted.
40% of the plain day's movements are third exits, and in the city routes then shift off them (free
flow costs the longer ring): the row's arm, 37 cars a day plain, sends 423 with a ring.

**Why two lanes lose as much as one in the city.** They are not used. 4-Lane trunk, one ring: the inner
lane carries 436 entries with a queue of 141-168, the outer 83 with a queue of at most 11. The ring's
exits to a two-lane arm run 1:1 (D10), so `preferred_entry_lanes` follows the route through the ring
to the next turn off it, and the block after the first exit needs the left lane. At a plain junction
the turn fans onto both target lanes and the choice ends there. The plan expected no preference. A
measurement build that stops the preference at a ring: both trunk lanes queue (782 and 473 entries),
ring entries 2,791 a day, 116 a minute, the fixture's figure and 1.84 times the one-lane ring;
deliveries 1,246; but 3,298 missed turns against 20, since those cars then stand in the wrong lane on a
short block. So not a fix: the capacity exists and the lane rule keeps it out of reach on this grid.

**Without the trunk junction.** `METRUM_PERF_RINGS_SKIP_TRUNK=1` leaves the junctions next to a border
plain; D17's default is unchanged (fresh: 710 and 965 again, hashes `9d1150dd...`, `14f11455...`).
One day at 32x, 8 workers:

| 10k day | 1+1, 1 ring | 1+1, 8 rings | 1+1, no ring | 4-Lane, 1 ring | 4-Lane, 8 rings | 4-Lane, no ring |
| --- | --- | --- | --- | --- | --- | --- |
| Saturation reports / detours / cycles | 0 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| Trip failures, all / ring | 0 / 0 | 0 / 0 | 0 / - | 0 / 0 | 0 / 0 | 0 / - |
| Freight deliveries | 1,419 | 1,421 | 1,385 | 1,759 | 1,909 | 1,863 |
| Held at nodes, peak / end | 283 / 283 | 321 / 321 | 260 / 260 | 163 / 147 | 129 / 62 | 163 / 140 |
| Vehicles on lanes, peak | 2,896 | 3,134 | 2,869 | 3,098 | 3,202 | 3,082 |
| Missed turns | 0 | 0 | 0 | 148 | 199 | 10 |
| Cars into the rings | 51 | 1,704 | - | 126 | 1,094 | - |
| Stands at an entry: count, p50 / p95 / max (s) | 6, 0.5 / 0.5 / 0.5 | 557, 1.1 / 5.9 / 97.0 | - | 12, 1.1 / 3.2 / 4.8 | 193, 1.1 / 5.3 / 29.9 | - |

So the rings elsewhere cost nothing; the loss is the one ring on the one junction the city's freight
crosses. The single ring takes almost no traffic in either run, so that column says little. The
4-Lane city's missed turns rise, 148 and 199 against 10: rings on a grid of 30-40 m links, a
measurement.

**Game speed in the day.** R7c's table is at 32x. The same snapshots at 1x (86,400 frames): 1,386 no
ring, 826 one ring, 780 eight on 1+1; 1,889, 1,138, 1,062 on 4-Lane. The ring-free city moves 0-1.4%,
the rings 7-10%, so the 32x table understates a ring by that much, and the ratio is still 56-60%.

Not known: how the rule's windows would behave at other ring sizes (only 36 m and 50 m ran); what a
junction with `TRAFFIC-09` conflicts would carry on the trunk (the 99.4 is an upper bound, not a
measurement); what `TRAFFIC-17` routing would do to the trunk's load, since a single border trunk has no
other way in.

### Gridlock check (`TRAFFIC-14`)

A deterministic wait-for cycle check (`tick/gridlock.rs`, `AgentSystem::check_gridlock`) runs in
the SIM-02 fixed slot `gridlock check`, slot 4 of every game minute
([`simulation.md`](simulation.md#scheduler)), after the frame's agent tick. Contract:

- **Graph.** A lane waits when its head car (the last bucket entry, not a lane-change ghost) is a
  car in network or junction transit standing at the lane's end (within 0.05 m; a waiting head is
  placed at the end exactly) and the entry it needs next fails the test movement makes:
  - a connector head waits for its exit lane while the exit's entry slot is blocked
    (`exit_connector_lane`'s test);
  - a road-lane head reads its next target from its route as the speed pass does
    (`planned_next_target`, the road edge of its next path node or its zero-hop detach lane). A
    direct pass-through lane is tested by its entry slot; otherwise the connectors leading there
    (only the shortest ones toward an edge, as the claim keeps them) are tested in successor
    order: the head waits only when every one fails, for the first one's connector. A connector's
    test is its entry slot (`lane_entry_slot_clear`), as the claim's, and on a ring entry the
    [give-way test](#give-way-on-entry-to-a-roundabout-road-62) too; a head that yields waits for
    the lane that test names, the ring's, not for its own connector.

  Each lane waits for at most one lane, so the graph is functional: one walk per waiting lane,
  stamped with the check number and its start lane, finds every cycle once. Inputs are the lane
  buckets the last tick left (`rebuild_lanes` at the tick's end); the check is skipped while the
  bucket snapshot does not describe the current lanes. Parked and stored-off-map vehicles have
  no lane-bucket entries. A pull-out held by `ACCESS_LANE_ATTACH_PENDING` has no current lane;
  it is an entry wait and cannot supply a wait-for edge or confirm a cycle by itself.
- **Confirmation.** A cycle's identity is its lanes, rotated to start at the lowest lane id, plus
  every car on them as (agent handle, lane distance bits) in bucket order. A cycle is confirmed
  when the previous check held one with the same lanes and the same cars at the same distances:
  the same lanes, and no car in it moved. Exact comparison, no hash.
- **Detour.** For each confirmed cycle, in the order the walk found them, the road-lane heads of
  the cycle are ordered by the slot their head was first seen waiting (per-lane stamps carried
  from check to check while the same head waits), then by agent index. The first of them with a
  way out detours: another successor of its lane, toward an edge other than its planned one,
  whose entry passes the normal entry test now, queried in successor order, gets one route
  query from that exit's end to the car's destination (`plan_network_replan` for a
  building, `plan_border_network_replan` for a border-bound truck), with the exit as the incoming
  edge so turn rules apply. On success the remaining route becomes the junction, the exit's end
  and the planned route, with the planned detach; the car keeps standing and enters on a later
  tick through the normal speed pass and claim. Nothing is moved, despawned or timed out.
  Parking-bound cars use the existing parking endpoint override to retain the reserved bay's
  driveway, custody and reservation. Route replacement also clamps the saved preparation budget
  to the new route's guaranteed storage. This adds O(1) endpoint work per attempted detour,
  allocation-free; the check's scan/walk bounds and single route query remain unchanged.
  Connector heads never detour. A connector exists only for a turn the node's lane rules allow,
  so a detour never takes a banned turn.
- **Report.** A confirmed cycle with no such head is genuine saturation: it stays locked and its
  junctions (connector nodes and the terminal nodes of its road lanes) are reported, ascending
  and unique, in `GridlockReport::junctions`, with the counts of waiting lanes, cycles,
  confirmed and saturated cycles and the totals of detours and saturation reports. The bridge
  exposes it as `SimulationNode::get_gridlock_report()` (`junctions`, `positions` and the counts),
  read from the render snapshot (each frame copies the counts and the live junctions with their
  positions into the snapshot's recycled buffers, O(reported junctions), so the bridge never
  waits for the core). The game's `GridlockDisplay` (`godot/scripts/renderers/gridlock_display.gd`)
  reads it twice a second and, only when the junctions change, rings each reported junction and
  shows the HUD notice "Gridlock: traffic is locked at N junctions with no way out"
  (`godot/scripts/ui/gridlock_notice.gd`); GDScript decides nothing.
- **Cost.** Per check O(lanes / p) for the head scan (Rayon, 4,096 lanes a task; O(node degree)
  per head at its lane's end; the waiting heads' stamps are updated in the same pass), one
  serial O(lanes) pass over the flat waited-for array to start the walks, O(waiting lanes) for
  the walks, O(cars in cycles) for the fingerprints and one route query per detour. The
  per-lane arrays (waited-for lane, walk stamps, head stamps) are sized when the lane count or
  `LaneSystem::revision` changes, which also forgets the previous cycles; scratch keeps its
  capacity, so a steady state allocates nothing. A detour allocates its new route.
- **Save** (save format 109, [`simulation.md`](simulation.md#save-bytes-road-68-k16)). One
  `gridlock_state` row holds what the next check needs, never the per-lane arrays: the report,
  the stamps of the lanes whose head waited at the last check (lane, head handle, slot first seen
  waiting) and that check's cycles (lanes per cycle; lane and car count per cycle lane; handle
  and distance bits per car), packed little-endian. Lane ids are saved live and mapped on load
  through the lane-reference restore's map (a missing lane fails the load); each cycle is
  rotated back to start at its lowest loaded lane, its per-lane car groups with it; junctions
  are saved in saved node ids. A checker bound to other lanes than the save writes (a topology
  edit since the last check) saves no heads and no cycles, as its next check would forget them.
  Save O(lanes) for the stamps plus O(heads + cycle lanes + cycle cars) bytes; load O(heads +
  cycle lanes + cycle cars), one map lookup per saved lane; the arrays are sized when the loaded
  lanes are bound (`prepare_loaded_tick`).

Acceptance record (`TRAFFIC-14`, round R14D, 2026-10-31; branch `port-r14d`: `TRAFFIC-16`, the
check without the box rule, save format 109, the notice; release test binaries,
`RAYON_NUM_THREADS=8`, cores `0,2,..,14`, one run at a time, nothing else building or running;
control `dev` `bcbb148f`; the artifact folders stay local):

- Ring fixture: `traffic14_ring_with_exits_detours_out_of_its_lock` 13 detours in 60 checks, no
  cycle confirmed at two checks in a row, cars moving after every detour, 0 saturation reports,
  1 and 8 workers bit-identical; `traffic14_ring_without_exits_stays_locked_and_reports_saturation`
  0 detours, corners 0-3 reported (51 saturation reports), nobody moves after the lock, no car
  removed; `traffic14_gridlock_check_continues_across_a_reload` (now through the save file's
  `gridlock_state` row) matches the continuous run. Whole city:
  `traffic14_saturated_ring_continues_across_a_save_file` (a ring of four one-way roads in a 2 km
  world, 550 trucks, locked and reported at frame 1,103) saved one frame after a check, loaded,
  equals the unsaved city in reports and every car's lane, distance and speed bits at the next
  three checks.
- 100k city from scratch to day 12 (`traffic14_ring_formation`): the check commit's build and the
  final build print the same 523 `TRAFFIC14` lines, field for field apart from timings, as round
  R14C's build without the box rule: the R14B column cycle forms at day 10 and every confirmed
  one is detoured (21 detours, 18 on day 10 and 3 on day 11), 0 saturation reports, no cycle at
  day 12; day 12 71,336 cars on road, 1,485 held at nodes, 196 at attach points (`dev`: 66,759 /
  2,027 / 60, no cycle).
- Check cost at 100k (184,092 lanes, up to 5,300 waiting at a check), in frame
  (`FrameStepReport::gridlock_ms`, the final build's formation run, quiet machine): mean 0.21 ms
  on day 1, 0.32 on day 3, 0.63 on day 5, 0.68-0.71 ms on days 6-12; worst single check 1.51 ms
  (day 3), 6 of 15,302 checks took more than 1 ms (1.01-1.51 ms). Isolated
  (`traffic14_gridlock_check_cost` on a day-12 snapshot the final build prepared, 64 checks on the
  standing city, load average 3.5): median 0.28 ms, min 0.19, max 0.58. One profile (`perf record`,
  2,000 checks on the same city, median 0.32 ms, max 0.64 ms): of the check's own samples, reading
  the waiting heads' next targets (`planned_next_target`, `head_target`) takes 48%, the head scan
  and walks (`check_gridlock`) 32%, the entry tests (`entry_state`) 19%. No isolated check
  reaches 1 ms, so the six in-frame checks over it are not reproduced by the check's own work;
  they are unexplained (contention with the frame's other work is the likely cause, not shown).
- 1-day 100k audit of the snapshot `dev` built (`traffic07_lane_entry_audit`, 32×, 6-hour
  checkpoints; a format-108 snapshot, so the check commit's build, whose simulation the final
  build's equals), check build / `dev`: 0 lanes over capacity and 0 stacked cars at every
  checkpoint in both; unmoved cars reporting more than 0.5 m/s 35-56 (connector heads at their
  connector's end but one; heads that lost an entry claim in the sampled frame, `TRAFFIC-18`) /
  1,648-1,728; cars unmoved per 6 hours 26,152-28,428 / 28,904-30,327; 0 cycles at every
  checkpoint.
- Envelope and determinism (`tools/sim02_gate.py envelope determinism --bursts
  departures,settlements`, `RAYON_NUM_THREADS=6`, same session, final build against `dev`): both
  PASS and deterministic (repeat, 1 and 3 threads, reload); the control's metrics equal round
  R14C's control and the final build's equal R14C's build without the box rule, workload for
  workload. 13 notes against 14, the same metrics, every shift the `TRAFFIC-16` one (heads start
  from 0 at saturated junctions): 30k baseline deliveries 135.7 to 117.3, employment 0.623 to
  0.601, final employment 0.583 to 0.520, net income 9.89 to 8.27, bankruptcies 1.26 to 1.90 (all
  outside the band on `dev` already), out-of-stock 0.207 to 0.265 (back inside the band: the
  14th note goes); 30k arrivals deliveries 126.2 to 122.4; 10k fiscal stress deliveries 8.67 to
  8.48; 10k baseline deliveries 173.4 to 174.0; the other metrics move by at most 0.4%.
- `PERF-06` agent contract (`tools/perf_contract.py --agents`, 8 workers, cores `0,2,..,14`,
  exclusive timing lock, load average 1.5 at 19:46; control, final, final, control): identical
  work counts in all four runs. Single samples, ms: network 10k 0.320 / 0.243 / 0.452 / 0.505,
  30k 0.705 / 0.935 / 0.613 / 0.689; egress 10k 0.904 / 1.278 / 0.796 / 0.851, 30k 2.399 /
  1.293 / 1.658 / 1.651; ingress 10k 0.955 / 0.409 / 0.302 / 0.202, 30k 0.614 / 0.622 / 0.590 /
  0.365. They scatter within a binary as widely as between binaries: no speed claim. The
  contract's fixture has no connector, so `TRAFFIC-16`'s brake does not run in it, and the
  gridlock check is not part of the agent tick.
- Gates: `cargo clippy --all-targets` warning set equal to `dev` `bcbb148f`'s (59, diffed by file
  and message); `./run.sh --test` exit 0 (debug `cargo test --lib` 1,858 passed, 0 failed, 184
  ignored; every headless and rendered Godot script passed, `gridlock_notice_test` included).

Earlier record (round R14C, 2026-10-31; branch `port-r14c`, kept as evidence; release test
binaries, `RAYON_NUM_THREADS=8`, cores `0,2,..,14`, one run at a time; the artifact folders stay
local). Builds: control `dev` `bcbb148f`; box rule; box rule + `TRAFFIC-16`; final = box rule +
`TRAFFIC-16` + the check; and a variant without the box rule (`TRAFFIC-16` + the check).

- Ring fixture (`movement_pass::ring_tests`): `traffic14_ring_with_exits_detours_out_of_its_lock`
  (an escape spur at every corner, 60 one-minute checks): the ring locks at the fourth check, is
  confirmed and detoured at the fifth, and keeps re-forming as the corners refill it; 20 detours,
  each confirmed cycle detoured at its confirming check (no cycle confirmed at two checks in a
  row), cars moving in the interval after each detour, 0 saturation reports, 1 and 8 workers
  bit-identical. `traffic14_ring_without_exits_stays_locked_and_reports_saturation`: confirmed
  from the fifth check, 0 detours, corners 0-3 reported, nobody moves, no car removed.
  `traffic14_gridlock_check_continues_across_a_reload`: a restore between two checks matches the
  continuous run. Without the box rule the fixture's exits test detours 13 times and the locked
  test reports corners 0-3 as well.
- 100k city built from scratch to day 12 (`traffic14_ring_formation`, 30-minute checks):

  | build | wait-for cycles | detours / saturation reports | day 12: on road / held at nodes / held at attach points |
  | --- | --- | --- | --- |
  | `dev` control | 0 at every check | n/a | 66,759 / 2,027 / 60 |
  | box rule | 0 | n/a | 68,436 / 3,652 / 371 |
  | box rule + `TRAFFIC-16` | 0 | n/a | 70,575 / 3,710 / 376 |
  | final | 0 | 0 / 0 | 70,575 / 3,710 / 376 |
  | `TRAFFIC-16` + check, no box rule | the 1,690-car column cycle of round R14B forms at day 10 10:00-23:30 and keeps re-forming while it crawls; every confirmed one is detoured | 21 / 0 (18 on day 10, 3 on day 11) | 71,336 / 1,485 / 196 |

  The final build's check lines are identical, field for field apart from timings, before and
  after the cost refactor below. In the variant the column ring is a slowly moving cycle: the
  diagnostic sees it at 25 half-hour samples, the check confirms it only when nobody moved for a
  minute, and detours it then, so no locked cycle outlives two check intervals.
- Check cost at 100k (184,092 lanes, 5,000-6,600 waiting at a check): in frame
  (`FrameStepReport::gridlock_ms`, the formation run), variant without the box rule, a quiet
  machine: mean 0.21 ms on day 1, 0.69-0.74 ms on days 6-12, worst single check 1.64 ms. Final
  build: 0.24 ms on day 1, 0.79-1.14 ms on days 5-12 with worst checks of 2.9-4.6 ms; that run
  overlapped two release builds and its timings are not acceptance. Isolated
  (`traffic14_gridlock_check_cost`, the day-12 snapshot, 64 checks on the standing city, load
  average 5.6): median 0.34 ms, min 0.25, max 0.93; before the cost refactor (head stamps moved
  into the parallel scan, lane fields tested before agent columns) 0.52 / 0.36 / 1.39. The mean
  meets the 1 ms target; single checks do not always.
- 1-day 100k audit of the snapshot the control built (`traffic07_lane_entry_audit`, 32×, 6-hour
  checkpoints), control / box rule / final: 0 lanes over capacity and 0 stacked cars at every
  checkpoint in all three; unmoved cars reporting more than 0.5 m/s 1,648-1,728 / 2,355-2,625 /
  1-2; cars unmoved per 6 hours 28,904-30,327 / 40,201-45,785 / 40,058-45,882; 0 cycles.
- Envelope and determinism (`tools/sim02_gate.py envelope determinism --bursts
  departures,settlements`, `RAYON_NUM_THREADS=6`, same session): PASS and deterministic
  (repeat, 1 and 3 threads, reload) for control, box rule and final. The box rule's cost is
  above. `TRAFFIC-16` on top of it: 30k baseline deliveries 79.8 to 75.8, arrivals 60.9 to 70.1.
  Variant without the box rule (envelope only): 13 notes; 10k baseline unchanged (deliveries
  173.4 to 174.0); 30k baseline deliveries 135.7 to 117.3, net income 9.89 to 8.27,
  bankruptcies 1.26 to 1.90, final employment 0.583 to 0.520, the `TRAFFIC-16` shift round R14B
  measured.
- `PERF-06` agent contract (`tools/perf_contract.py --agents`, 8 workers, cores `0,2,..,14`,
  exclusive timing lock, load average 1.6; control, final, final, control): identical work
  counts in all four runs. Single samples, ms: network 10k 0.556 / 0.395 / 0.252 / 0.293, 30k
  1.007 / 0.647 / 0.730 / 0.599; egress 10k 0.598 / 0.487 / 0.483 / 0.490, 30k 1.441 / 1.687 /
  1.275 / 1.233; ingress 10k 0.249 / 0.300 / 0.365 / 0.264, 30k 0.310 / 0.509 / 0.568 / 0.427.
  They scatter within a binary as widely as between binaries: no speed claim. The contract's
  fixture has no connector, so the box rule's test does not run in it.

The `junction_connectors` debug view's stall detector (`agents/junction_stalls.rs`) is a separate
observer: it reports single cars that stand on a connector, not cycles, and runs only while the
view is on.

## Junction Speeds

Junction movement uses a cap separate from the parent road speed:

- `CAR_JUNCTION_SPEED_MS = 6.0 m/s`
- `CAR_JUNCTION_LATERAL_ACCEL_MS2 = 2.2 m/s^2`
- `CAR_JUNCTION_MIN_SPEED_MS = 2.0 m/s`

`connector_turn_speed()` estimates turn severity from the angle between the connector's first and
last non-degenerate tangents. For straight or very gentle connectors, the cap is
`CAR_JUNCTION_SPEED_MS`. For tighter connectors, approximate radius is:

```text
radius = connector_length / turn_angle_radians
```

The turn speed cap is:

```text
sqrt(CAR_JUNCTION_LATERAL_ACCEL_MS2 * radius)
```

clamped to `CAR_JUNCTION_MIN_SPEED_MS..CAR_JUNCTION_SPEED_MS`.

`CAR_JUNCTION_MIN_SPEED_MS` is the lower bound for the curvature-derived cap. It is not a rule that
boosts a stopped or slow car up to crawl speed when entering a connector.

Road-lane speed prediction looks ahead to the next connector and brakes early if the connector is
tight or blocked. Once in `TRANSIT_INTERSECTION`, the car remains capped by the connector's
curvature speed until it exits to the target road lane.

A connector head (no car ahead on its connector) brakes for its exit lane's entry slot the same
way (`TRAFFIC-16`): while `lane_entry_slot_clear` fails for the connector's exit lane (the lane
`exit_connector_lane` enters), its target speed is capped at `braking_speed_for_distance(0,
distance to the connector's end)`, so it stands at the end at speed 0 until the exit clears, and
starts from 0. One O(log bucket) slot test per connector head and tick, no allocation. Before,
connector cars followed only their own bucket, and the head standing at a full exit kept its
turn speed (the movement step re-clamps to the turn speed but never raises it).

Acceptance record (`TRAFFIC-16`, round R14B, 2026-10-31; the artifact folders stay local):

- Ring fixture (`movement_pass::ring_tests::traffic16_connector_heads_stop_at_a_blocked_exit`):
  the locked four-lane ring's four connector heads read 0; on `dev` all four report their turn
  speed. `traffic14_full_ring_locks` still locks (no ring rule yet) and stays ignored.
- 1-day 100k audit of the `dev`-built snapshot (`traffic07_lane_entry_audit`, day 12 at 32×,
  `RAYON_NUM_THREADS=8`, cores `0,2,..,14`): unmoved cars reporting more than 0.5 m/s at the
  6-hour checkpoints, `dev` against the fix: 2,003 / 10, 2,027 / 23, 2,119 / 36, 2,140 / 29
  (`dev`: 1,992-2,124 connector heads at the connector's end; fix: 10-34 connector heads and up
  to 2 road-lane heads). The two connector heads followed frame by frame (`METRUM_AUDIT_FOLLOW`)
  found their exit slot clear at the tick's start, accelerated (3.2 m/s) and lost the entry to
  another car in that frame (`TRAFFIC-18`); the next frame they read 0. The road-lane heads were
  not followed. Both engines: 0 lanes over capacity and 0 stacked
  cars at every checkpoint; held at nodes 2,682 at 00:00 (2,210 of them freight) and 3,742
  (fix) / 3,664 (`dev`) at 24:00. The snapshot already holds `dev`'s locked ring, which neither
  fix releases.
- **Traffic cost at saturation.** A head that stands at a blocked exit now starts from 0 when
  the slot clears; it used to leave at its turn speed from a standstill. The exit lane's entry
  slot therefore clears later and a saturated junction discharges more slowly. The 100k city
  built from scratch with both fixes forms a 1,690-car wait-for cycle at day 10 10:30: two
  columns of 35 full 30 m cross-street lanes, two full 380 m row lanes and their connectors.
  It is absent at two later checks, present without a break from day 10 19:30 and at all five
  checkpoints of that city's next day (47,630-50,086 cars unmoved per 6 hours, against
  33,400-36,865 in the `dev` city). With `TRAFFIC-15`/`TRAFFIC-13` alone the same build has no
  cycle. Envelope (both fixes, deterministic, PASS, 13 notes) against `TRAFFIC-15` alone: 30k
  baseline deliveries 135.7 to 117.3, employment 0.623 to 0.601, net income 9.89 to 8.27,
  bankruptcies 1.26 to 1.90, out-of-stock 0.207 to 0.265; 30k fiscal-stress deliveries 10.90
  to 10.79; the other workloads move by at most 2.3% on their main metrics. Landing this before
  the junction-box and gridlock rules of `TRAFFIC-14` is the user's decision (round R14B
  handoff).
- **Landed with the gridlock check (round R14D, 2026-10-31)**, the envelope shift accepted by
  the user; results in the [gridlock check record](#gridlock-check-traffic-14).
- **Round R14C (2026-10-31).** Built with the `TRAFFIC-14` gridlock check, the 100k city's
  column cycle still forms at day 10, but every confirmed one is detoured (21 detours to day 12,
  0 saturation reports) and day 12 ends with no cycle; with the box rule as well no cycle forms
  at all. The 1-day audit of the final build finds 1-2 unmoved cars over 0.5 m/s per checkpoint
  (box rule alone: 2,355-2,625, `dev` 1,648-1,728). The envelope shift is the one round R14B
  measured: heads start from 0 at saturated junctions, so 30k baseline deliveries fall 13.5%.
  Results in the [gridlock check record](#gridlock-check-traffic-14).

## Planned Lane Changes

Planned lane changes solve a different problem from overtaking: a car may need to move to a sibling
lane on the same edge to reach the exact `planned_detach_lane_id` for building access, or to a
lane that makes its next junction movement (`ROAD-94`).

### Advance lane choice (`ROAD-94`)

Junction lane use and target pairing are owned by
[`roads.md`](roads.md#default-junction-lane-use-road-94); this section owns how a car gets onto
a lane that makes its turn. Ported from Veeti's road-chain branch (`6ec0b858`, `1ae3b22f`),
re-derived onto `dev`'s lane-end handling.

- **Window.** A car on a road lane flagged by the lane use (`LaneSystem::needs_turn_preparation`)
  prepares its turn in the last `70 m x (count - 1) + 30 m` of the lane, `count` the lanes in its
  direction: 100 m on a 2+2 road, 170 m on a 3+3, 240 m on a 4+4. Fixed for every road speed
  (PLAN-R4 D2): 70 m is the longest lane-change S-curve, 30 m the 12 m road-end buffer plus room
  to start. The guard (`lane_nav.rs` `junction_approach_node`) reads the flag first, so a car on
  any other lane (every lane of a 1+1 road) pays one read of its lane (`Lane::solid_zone_m`, on
  the lane's own cache line since decision Q3-1 of `TRAFFIC-23`).
- **Selector.** Inside the window `junction_approach_lane` takes where the car goes next from
  `planned_next_target` (the next path edge, or the zero-hop detach lane's edge; one function
  with the route preview and the connector choice, D1) and picks the nearest same-direction lane
  that continues onto it (a direct pass-through or a connector), ties by `(|index delta|,
  |index|, lane id)`. The car's own lane wins when it already makes the movement, so it never
  weaves. Claim classification (`claims.rs` `agent_turn_lane_target`) and the movement step
  (`movement/network/lane_change.rs` `turn_lane_target`) call the same selector on the same
  inputs, both out of line behind the flag read.
- **The move.** `choose_lane_change` moves one adjacent lane toward the selected lane through the
  planned-change machinery (lateral claim, owner `fetch_min`, gap check, reservation, S-curve);
  no new agent column. A pending frontage change keeps priority, and no discretionary overtake or
  return starts inside a window. The budget divides the distance left before the solid zone over
  the steps still needed: `min(speed x 3.5 s, (available - 12 m) / steps - 0.25 m)`, never under
  the 18 m minimum (`turn_lane_change_length`).
- **Entry-lane choice (`TRAFFIC-23` (a)).** A car entering a road with more than one lane in its
  direction takes a lane from which it can follow its route without a lane change up to the
  route's next lane-selective movement (`lane_nav.rs` `preferred_entry_lanes`): the walk follows
  the path and narrows the usable ranks by the default lane use's group for the movement at each
  node (`lanes::approach_group`), carries them across every group that maps 1:1 onto the next
  road (straight on between equal counts, a pass-through, a one-lane turn onto a one-lane road),
  and stops at the first group that does not (a turn onto a wider road, a fan), at the planned
  detach lane on the road or its zero-hop turn, at a whitelisted node or a U-turn, or where the
  next group shares no lane with the ranks kept (the car must then change lanes on a later,
  longer road). No preference when every lane is usable. The lane end's connector choice keeps
  only the connectors onto a preferred lane when any reaches one, then the shortest as before
  (`collect_connector_lanes_to_edge`); a preferred connector that is occupied makes the car wait,
  as for any connector. The node entry (`prepare_lane_entry`, decision S1) rotates its seed over
  the preferred lanes only and waits while they are busy; the zero-hop fallback prefers the
  detach lane's own rank; the route preview passes the same inputs, so the inspector shows the
  lane the tick takes. It works because a single-lane turn group fans onto every lane of its
  target ([`roads.md`](roads.md#default-junction-lane-use-road-94)). The per-tick selector is
  unchanged (one node ahead, decision S6).
- **Turn pockets (`ROAD-95`, round R5a3).** A pocket lane is its turn's only lane, and the other
  lanes of the pocket edge do not make that turn ([`roads.md`](roads.md#turn-pockets-road-95), D4).
  The fork into it is a connector at the lane transition (D5: each base lane keeps its rank and the
  lane beside the pocket lane also forks into it), so a car picks the pocket at the fork and turns
  without a lane change. A pocket edge is one taper and a body long, about 30 m of lane at a 40 m
  body: too short to change lanes on. So the lane choice reads through the transition: the
  entry-lane walk continues past a transition's fan into a pocket edge to the movement at the
  pocket's junction, and a base approach of more than one lane to a transition is flagged when the
  pocket edge's approach needs a lane choice (`transition_needs_lane_choice`): its window selector
  targets the base lanes whose connectors reach the lanes making the movement
  (`pocket_fork_sources`), S8 keeps it there, and its last 20 m before the transition are a solid
  zone like any flagged approach (the paint reads the same rule). A 1+1 base road has one forward
  lane and needs none of this. A car that joins the base road from a frontage too close to the
  transition to change lanes misses the turn and replans (the populated record is in
  [`roads.md`](roads.md#pockets-in-traffic-round-r5a3)). Cost: at entry events the walk adds
  `O(degree + lanes)` at a transition; the window selector reads one edge (whether the next road
  has a pocket) on every other approach; the flag is written at connector rebuilds.
- **Discretionary changes keep the route's lanes (S8).** On a flagged lane a discretionary
  overtake or return whose target lies outside the lanes `preferred_entry_lanes` gives for the
  current road (`route_lane_group`, the route read as `planned_next_target` reads it) is refused:
  an overtake out of the chosen lane in a standing queue may find no gap to return before the
  junction. Evaluated only after every other condition of the change has passed (the target is
  useful and its gap clear), never otherwise; a 1+1 road is never flagged.
- **Solid approach zone.** No change of any kind (frontage, turn, overtake, return) starts in
  the last `APPROACH_SOLID_LENGTH_M` = 20 m of a flagged lane (`lane_change_end`); the turn budget
  extends its 12 m end buffer to the zone (8 m more) rather than charging both, and the window
  guard stops at the zone, so a queue at the stop line runs no selector. The lane lines there are
  painted solid ([`roads.md`](roads.md#default-junction-lane-use-road-94)). The route preview's
  preparation step uses the same zone.
- **Missed turn (D8).** A car that reaches the lane end on a lane without a connector to its
  planned road, while a sibling lane has one (a lane-use miss, not a missing road turn), takes its
  lane's first successor in stable order onto another road (`junction/enter.rs`
  `follow_missed_turn`), drops its node path and logs `[MISSED_TURN]` on `traffic`; a wait for
  that connector keeps the car and its path at the lane end. Right after that entry, the lane-end
  handler replans from the end of the road it entered (`replan.rs` `replan_after_missed_turn`:
  `plan_network_replan` with the car's access flags, or `plan_border_network_replan` for
  freight). **Lane-aware (`TRAFFIC-23` (b)):** the onward road is the first successor (stable
  order) whose lane can host a turn preparation (`lane_hosts_turn_preparation`: one lane, or the
  18 m change and the end buffers fit before its solid zone from its start, more than 38.25 m on
  a flagged two-lane lane), else the first that is not the wanted one (S2); when the road the car
  entered cannot host a preparation, the replan's first hop is restricted to the roads its lane
  reaches (`plan_network_replan`'s `first_lane`, S3): `TripQueryScratch::network_path` builds the
  predicate (`vehicle_lane_reaches_edge`) and `CchGraph::find_path_in` seeds the origin per
  accepted departure, as a restricted origin; that forward sweep is never retained, so the next
  unrestricted query from the node is unchanged (S7). Every other caller passes `None` and is
  unchanged. A failed replan only sets the replan backoff; the car drives on and `dev`'s
  empty-path replan at the next lane end is the last resort, so nothing teleports. A destination
  turn may enter its road on any lane (a connector to the detach edge) and reach the exact detach
  lane by frontage changes. A missed turn is not a trip failure; a genuine missing connection
  keeps `dev`'s failure report.
- **Bounds.** Per tick, per car on a road lane: outside a window, one byte read in claims and
  one in movement, plus two compares in `choose_lane_change`; no other column read, no
  allocation, no route query. Inside a window: `O(D + L x C)` twice (`D` the node degree, `L`
  the lanes in the car's direction, `C` their successors), the existing `O(L)` adjacent-lane
  lookup and one `O(log K)` gap test. At a lane end, `follow_missed_turn` costs `O(L x C + C)`
  lookups only on a missing connection, and a missed turn costs one route query (an event, not
  per tick). The entry-lane choice costs `O(hops x degree)` per entry event on a multi-lane road
  (one edge read on a one-lane road), the S8 check the same walk only on a tick where a
  discretionary change on a flagged lane would otherwise start (a refused car is asked again on
  the next such tick while it stays blocked beside a clear, useful lane), the restricted replan one `O(successors)` predicate
  call per departure of the origin; no allocation, no extra route query, nothing per tick.

Acceptance record (`ROAD-94`, 2026-11-04, rounds R4a2 and R4a3; artifact folders stay local):

- `benchmark_lane_transitions` (4 workers, P-cores, alternating pairs against `dev` `9f2808bc`
  with the same test-only `turn` scenario): the five guard-only rows within noise after two
  inlining fixes (R4a2: the missed-turn recovery had made `enter_next_edge_connector` recursive
  and the added code had moved the movement pass's per-agent steps into other codegen units, so
  `handle_network_movement` called them out of line; `#[inline]` on the steps and a non-recursive
  entry restored it, and the fork's per-tick missed-turn gate moved to the lane end). The
  blocked row's remaining +2.6% (+0.16 ms at 65,536 groups) follows the codegen-unit partition: a
  test-only edit applied to both sides flips its sign (roads.md question 36). The `turn` row,
  the only measurement of the window's cost: 65,536 approach arms of 2+2 four-ways, every car
  in its window on the wrong lane: +12.1 ms, about 0.09 us per car per tick, the worst case
  (131,072 lane changes against the control's none). R4a3's session (12:52, `.run.lock` and the
  exclusive timing lock, P-cores, load 1.4, the same control binary against the final code, three
  alternating pairs, medians in ms at 65,536 groups): clear 5.10 / 5.04, merge 5.44 / 5.53,
  blocked 6.10 / 6.28 (+2.8%, as R4a2's +2.6%), lateral 5.20 / 5.15, cruise 3.66 / 3.85 (+5.2%;
  ranges 3.51-3.75 and 3.71-3.96 overlap), turn 3.52 / 15.31. Transition counts equal in every
  row. The cruise rise is not attributed: the zone adds two compares to every
  `choose_lane_change` call, and question 36's partition effect was not profiled this round.
- Economy envelope and determinism (`tools/sim02_gate.py envelope determinism --bursts
  departures,settlements`, `RAYON_NUM_THREADS=6`, the gate's tiers: baseline and fiscal_stress
  10k and 30k, arrivals 30k): branch and control (`dev` plus the same test-only harness) both
  PASS with the same 13 stale-baseline notes; all 70 `SIM02_DAY` records byte-identical; the
  built cities' lanes hashes (`SIM02_LANES`) equal (10k `9537bde0101110fd`, 30k
  `726f721b92d2abbc`): the envelope cities are 1+1 and the lane use changes none of their
  connectors. Determinism: no difference (repeat, 1 and 3 threads, reload) on both.
- Locality: `incremental_lane_rebuild_scaling` local products equal at 0 to 100,000 background
  edges and equal to the control's (`90ceffa5f4b95fd1`), median 0.030-0.034 ms, flat;
  `populated_road_plan_scaling` identical local products at 4 to 100,004 parcels, compile p50
  0.19-0.46 ms with no trend in size (the control alternates 0.17-0.51 ms the same way).
- Connector sweeps: the re-recorded Kuopio save 324 connectors (94 U-turns, `dev` 440), 0
  failing; the generated 100k city 24,834 (10 U-turns), 0 failing, equal to the control (a 1+1
  city).
- 100k lane audit (the prepared city, one day at 32x, 8 workers): every `TRAFFIC07_AUDIT` and
  `TRAFFIC14_CYCLES` line equal to the control's: 0 lanes over capacity, 0 stacked, held at
  node 1,841-2,614, 0 gridlock cycles at all 7 samples, 4,374 freight deliveries, 9 trip
  failures, 0 `[MISSED_TURN]`, 0 missing connections.
- **10k 4-Lane variant (`METRUM_PERF_ARTERIALS=4lane`, test builds only: the populated city's
  cross streets and border trunk as 2+2): failing, open as `TRAFFIC-23`.** One day at 32x:
  68,635 missed turns (2,994 cars, 1,435 of them ten times or more, at most 113), 0 missing
  connections, 0 trip failures, **19 freight deliveries against the control's 2,073**, about
  5,900 vehicles on lanes all day against 940-1,850, 300 lanes over their fit against 0-5, and
  557 cars waiting more than an hour at the border node. 98.5% of the misses are on 2+2
  approaches of 30-40 m, the cross streets' 50 m blocks between their mouths: a lane change
  needs `18 m + 12 m` (`38 m` with the solid zone's extra 8 m), and a car turning onto a cross
  street enters the lane the target fan gives it (a right turn the outer lane, D10), so a turn
  off on the other side one block later cannot be prepared; the replan from the next node often
  needs another such turn, and the circling cars saturate the grid. The same city without the
  solid zone (a diagnostic build) misses 54,839 turns with 24 deliveries, so the zone is not the
  cause. A diagnostic of the border hold found no claim starvation: each tick a held car loses
  the lane only to the car that entered it before it in the same pass. The 100k and envelope
  cities are 1+1, so no contract tier exercises this.

`TRAFFIC-23` record (round R4a4, 2026-11-04; fresh runs, artifact folders stay local; one day at
32x, 8 workers, each binary preparing its own city; control `dev` `9f2808bc` plus the same
test-only harness, whose audit lines equal R4a3's control's exactly): the fix ((a) entry-lane
choice, (b) lane-aware recovery, S8, the permissive masks, Q3-1) **meets the missed turns but not
the deliveries; deliveries were then made a measurement, not a gate (user decision, below).** Node entry run both ways (decision S1):

| | control (`dev`) | S1: wait for a preferred lane (shipped) | S1 alternative: any clear lane |
| --- | ---: | ---: | ---: |
| `[MISSED_TURN]` / day | 0 | **10** | 1,131 |
| freight deliveries | 2,073 | **1,863** (-10.1%) | 2,088 |
| vehicles on lanes (samples 0-1440) | 937-1,841 | 1,012-1,709 | 1,002-2,052 |
| held at node (7 samples) | 0, 1, 20, 115, 11, 0, 0 | 3, 0, 21, 15, 6, **129, 140** | 1, 11, 131, 184, 1, 1, 1 |
| waits over 1 h / over 4 h (worst) | 47 / 4 | 126 / 83 | 177 / 25 |
| lanes over fit | 0-5 | 0-2 | 0-5 |
| trip failures / missing connections / gridlock cycles | 0 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| prepare continuation check | passes | passes | passes |

- **Misses attributed** (`[MISSED_TURN]` with the traffic channel, classified against the save):
  waiting, all 10 on the 373 m trunk (outer lane wanting a cross street, one per car, no
  chains): the trunk's lanes host a preparation, so each is the gap model's standing-queue limit,
  not a lane the car could not have chosen. Any clear lane: 553 on the 506 m border link (freight
  that took the other lane when its preferred lane's slot was busy) and 578 on the trunk, 479 cars,
  chains up to 12 (a car off its lane in a standing queue never merges back). A
  `RAYON_NUM_THREADS=1` repeat of the waiting run gives the same audit lines.
- **The border hold, exactly** (a test-only breakdown of the held cars at every sample, not
  committed): waiting, every held car at minutes 1200 and 1440 (129 and 140) stands at the border
  node entering the 506 m border link with the outer lane preferred: the route turns right at the
  link's far junction (a 2+2 four-way: the inner lane serves left and straight, the outer straight
  and right). The outer lane holds 116-131 cars with its entry slot occupied (nearest car
  0.8-4.9 m from the start) while the inner lane holds 3-5 cars (nearest 24-61 m). So it is the
  single right-turn lane carrying the whole southbound border flow (on `dev` both lanes turn
  right), with S1 refusing the free lane; the deliveries missing at the day's end are the queue on
  that lane and the held freight. The alternative's peak (184 at minute 720) is both lanes' queues
  of the same flow, then cleared.
- Diagnostics (not shipped, test-only builds): with S8 off the waiting run makes 1,968
  deliveries and 226 misses, holds up to 225 cars and 148 wait over 4 h, so S8 is not the cause
  of the shortfall. **With R4a3's lane masks instead of the permissive ones** (everything else
  as shipped) it makes **2,074 deliveries with 8 misses**; its border hold (the same node, lane
  and movement) peaks at 167 cars at minute 720 with 72 waiting over 4 h at minute 960 (`dev` 115
  and 4), then clears (0 from minute 1200). So the delivery shortfall comes with the permissive
  masks (user decision), which on this grid let straight traffic share the turn lanes (two-lane
  approaches with two movements, L+S / S and S / S+R, at the border cross streets' T junctions);
  the hold itself is the single right-turn lane in both.
- Other gates, fresh: economy envelope and determinism (`RAYON_NUM_THREADS=6`) PASS on both with
  the same 13 notes, all 70 `SIM02_DAY` records byte-identical to the control's (md5
  `35345ab0110821f9`, as R4a3), lanes hashes equal (10k `9537bde0101110fd`, 30k
  `726f721b92d2abbc`), no determinism difference; the 100k one-day audit identical to the
  control's line for line (4,374 deliveries, 0 missed turns); connector sweeps 0 failing: Kuopio
  352 connectors (94 U-turns; R4a3 324, `dev` 440), the 100k city 24,834 (equal to the control);
  locality: `incremental_lane_rebuild_scaling` local products `90ceffa5f4b95fd1` at 0 to 100,000
  background edges, median 0.030-0.084 ms with no trend in size (the control 0.030-0.034 ms),
  `populated_road_plan_scaling` identical local products, compile p50 0.18-0.31 ms (control
  0.19-0.34 ms).
- Timing (one session, 15:23, `.run.lock` and the exclusive timing lock, P-cores, load 3.6 after
  the gate runs with no cargo, rustc or godot; three alternating pairs against the control, 4
  workers; medians in ms at 65,536 groups): clear 5.09 / 5.13, merge 5.48 / 5.50, blocked 6.07 /
  6.11, lateral 5.16 / 5.28 (+2.3%), cruise 3.53 / 3.53, turn 3.54 / 17.07; transition counts
  equal in every row. The turn row's window cost rose from R4a3's 15.31 ms (+12%): the
  selector scans every sibling lane's successors (`O(L x C)`) and one-lane turn groups now have
  a connector to each target lane (instruction samples in `junction_approach_lane`'s filter
  2.40% against 1.63%). PERF-06 (C F F C) work counts equal; network 30k 0.49 / 0.75 / 0.81 /
  0.59 ms, sub-millisecond rows within their usual spread only loosely (noted, not a claim).
  `benchmark_claim_preparation` checksums equal, 1,048,576 mixed 0.51 / 0.54 / 0.52 / 0.55 ms.
  Decision Q3-1's check (`perf stat`, the 65,536 cruise row alone, two runs each, the same
  test-only row filter): the new code 5.676 G instructions, 0.959 G branches, 2.04-2.05 G cycles;
  R4a3's code 5.652-5.658 G, 0.953-0.954 G, 2.05-2.09 G; `dev` 5.843 G, 0.983 G, 2.11 G. The
  new binary carries all of this round's code, so the field's own saving is not isolated: the
  row runs 0.4% more instructions than R4a3's and 1% fewer cycles, 2.9% fewer instructions and
  3% fewer cycles than `dev`.
- **Decision (user, 2026-11-04): on this grid deliveries are a measurement, not a gate.** Route
  search is free flow (`TRAFFIC-17`), so all border freight takes one path, and the comparison with
  `dev`'s every-lane connectors measures that one movement's lane capacity, not the lane rules.
  `TRAFFIC-23` is accepted on missed turns, missing connections, gridlock cycles, lanes over fit,
  determinism, 1+1 cities unchanged, the sweeps and the per-tick cost; deliveries are re-measured
  once `TRAFFIC-17` lets traffic spread over routes. Two-day runs (fresh, test-only diagnostic
  builds whose first-day audit lines equal the shipped code's and the control's exactly; deliveries
  counted from the start): `dev` 2,073 after day one and 3,113 after day two, the shipped code
  1,863 and **3,156**, R4a3's masks (a comparator only) 2,074 and 3,130. The shipped code's border
  queue (140 held at midnight, 98 at day two's first sample) clears early on day two (0-2 held at
  the later samples), with 22 missed turns over the two days. So the day-one shortfall is the
  queue's timing at the funnel, not lost throughput. Open, not attributed: the `PERF-06` network
  30,000 row above; the turn row's sibling scan has a candidate fix (a per-lane set of the exits
  each lane reaches, built with the connectors).

Ownership split:

- [`entrance_and_exit.md`](entrance_and_exit.md) owns choosing `planned_detach_lane_id` and
  `planned_detach_lane_d`
- this document owns how the car physically changes lanes once that plan requires it

Rules:

- only cars in `TRANSIT_NETWORK` may start a planned lane change
- source and target lanes must be same-edge, same-direction vehicle lanes
- movement advances one adjacent lane at a time toward the final planned detach lane
- the target lane must have a speed-scaled safe gap at the current distance
- the car must own the target lane's reservation before starting the maneuver
- clear target lane means no intentional speed penalty
- blocked target lane is traffic and may force braking before the detach point
- planned lane changes take priority over discretionary overtaking

Lane-change length is speed-scaled:

```text
length = speed * 3.5 s
```

clamped to:

- minimum `18 m`
- maximum `70 m`

The car's authoritative longitudinal lane becomes the target lane when the maneuver starts, while
rendering blends from the source lane to the target lane over the S-curve. This keeps traffic gap
checks and destination detach checks tied to the lane the car is entering, without creating a
temporary lane object per car.

## Conservative Overtaking

Overtaking is intentionally conservative and local. It exists to make multi-lane roads feel alive
without adding a full driver AI.

Rules:

- only cars in `TRANSIT_NETWORK`
- only same-edge, same-direction sibling vehicle lanes
- never while a planned lane change is pending
- never while already lane-changing
- never near the current edge end
- never near a planned building detach point
- target lane must have a safe speed-scaled gap at the current distance
- target lane must have meaningfully more space ahead
- one adjacent lane at a time
- cooldown after each discretionary overtake / return

Current constants:

- stuck time before passing: `2.0 s`
- cooldown: `8.0 s`
- minimum speed-gain condition: `2.0 m/s`
- minimum gap gain over current lane: `12 m`
- target ahead gap for passing: `30 m`
- target ahead gap for returning: `20 m`
- edge-end buffer: `12 m`
- detach buffer: `25 m`

Lane direction convention:

- forward lanes use non-negative lane indices; lane `0` is closest to the road center
- backward lanes use negative lane indices; lane `-1` is closest to the road center
- overtaking moves toward the center lane
- return-to-cruise moves outward

This rule intentionally avoids weaving:

- a car must be traffic-blocked before passing
- a car returns only when the outward lane is clear and the overtake cooldown has expired
- planned destination-lane changes override overtaking

## Render Movement

Rust produces car transforms from lane poses in `nodes/sim/core/snapshot.rs`; the former direct
car/pedestrian exporters are removed. `nodes/sim/render/agents.rs` owns path debug overlays only.
The snapshot samples an active car lane-change curve only when source and destination lanes have
matching road, direction and lane type. Invalid/stale sources use the current lane pose. This
reuses the existing allocation-free lane sampler; it adds one bounded source-lane lookup and, for
active changes, source-lane sampling, not a scan over agents or roads. The snapshot regression
`car_snapshot_keeps_lane_change_curve_in_both_directions` checks start/midpoint/end positions,
sloped lane heights, lateral orientation, ownership flags and mismatched-source rejection.

The final audit's `lane_change_snapshot_benchmark` measures 100/1,000/10,000 cars on two sloped
lanes, with fixture setup excluded, ten warmups and 100 recycled snapshots per workload. Release
p50 for inactive versus all-active lane changes is 0.0316/0.0363, 0.1189/0.1507 and 0.8272/0.9872
ms respectively (`RAYON_NUM_THREADS=24`). This isolates the snapshot sampling cost; it is not a
gameplay frame-time benchmark.

Lane pose sampling in `nodes/sim/render/lane_pose.rs`:

- samples position by distance along lane geometry
- samples tangent using a `2 m` look-behind / look-ahead window
- falls back to the local segment tangent when the look window degenerates
- samples active lane-change S-curves by smoothstep blending source-lane and target-lane positions
- derives S-curve tangent from blended lane tangents plus the lateral blend derivative

Render-side car interpolation runs in Rust (`nodes/sim/render/car_visual.rs`, `RENDER-14`).
Each frame `godot/scripts/renderers/agents.gd` passes `get_car_render_data()` to
`smooth_car_render_data(car_data, delta)` and uploads the returned per-bucket buffers:

- cars have stable render IDs from Rust; drawn poses are main-thread presentation state on
  `SimulationNode`, never saved or read by the simulation
- pedestrian and vehicle MultiMeshes use the shared dynamic shadow-caster policy
- origin interpolation smooths per-tick position updates
- basis interpolation smooths rotation (Godot's `Basis.slerp` formula, reimplemented on `glam`;
  godot-rust's `Quaternion::slerp` is an engine call per car)
- large jumps above `80 m` snap instead of interpolating across the map
- a change between lane and off-lane height owners snaps instead of blending
- off-lane cars are grounded by the shared support solve after interpolation; when the core is
  busy they show their validated snapshot pose, never an unsupported interpolated one
- a car absent from one frame is forgotten; degenerate or non-finite snapshot poses pass through
  unchanged and are not remembered
- transform uploads happen every render frame so fast sim multipliers do not quantize cars to the
  simulation tick rate

Current render interpolation constants:

- `INTERPOLATION_RATE = 24.0`
- `ROTATION_INTERPOLATION_RATE = 18.0`
- `SNAP_DISTANCE_M = 80.0`

**Cost (`RENDER-14`, 2026-09-30).** Smoothing is O(visible cars) with one hash lookup and insert
per car. Buckets of at least 2,048 cars interpolate in parallel on the global Rayon pool, the pool
the support solve already used from the main thread. A populated-city run should confirm that the
main thread does not wait on a pool the simulation tick is saturating.

Matched A/B, `godot/tests/vehicle_ground_support_test.gd -- --benchmark-vehicle-support`: 100 timed
`update_swarm` frames with cars advancing every frame. Baseline is `HEAD` `eedd0094` GDScript;
both sides load the same release library (SHA-256 `92a4c47b…94841fda`), whose only change is the
new function. Runs were interleaved, two per side, headless and windowed 640×360, with
`RAYON_NUM_THREADS=4` on an i9-12900K / RX 7900 XTX. Windowed p50 ranges in milliseconds:

| Cars | Lane cars | Off-lane cars |
| --- | --- | --- |
| 1,000 | 1.22 → 0.074 | 1.87–1.96 → 0.60–0.63 |
| 5,000 | 6.06–6.12 → 0.34–0.49 | 8.74–9.28 → 2.23–2.37 |
| 20,000 | 24.9–25.3 → 1.19–1.25 | 34.8–35.3 → 8.22–8.26 |

Headless numbers agree (lane cars at 20,000: 24.6–24.8 → 0.82–0.88 ms). Off-lane cars keep the
unchanged native support solve, about 0.4 µs per car, which is now their dominant cost. Real
off-lane counts are limited to cars in an access phase. The Rust-only kernel
(`car_smoothing_frame_cost`, ignored test) measures 0.045 / 0.17 / 0.66 ms for 1,000 / 5,000 /
20,000 lane cars. After the runs, a clippy cleanup (`as_chunks` in place of `chunks_exact`, with no
behaviour change) produced the deployed `godot/bin` library `7a09021d…bb31156e`. The native tests
and the headless and windowed vehicle grounding test were rerun on it.

**Correctness checks:**
- Native `car_visual` tests cover:
  - a first sighting shows the snapshot pose;
  - origin and heading close their gaps at 24/s and 18/s, and the next frame starts from the drawn
    pose;
  - long jumps and height-owner changes snap;
  - absent cars are forgotten;
  - degenerate and non-finite poses pass through unrecorded;
  - a busy support solve restores only the off-lane snapshot poses.
- The 156 related release vehicle, car and render tests pass. Rustdoc reports no warnings. Clippy
  reports nothing new in the touched files.
- `vehicle_ground_support_test.gd` drives the production `agents.gd` update through the native
  call with real vehicle meshes, and passes headless and windowed. It checks 4,840 poses: no buried
  contact, windowed MultiMesh buffers equal to the drawn pose, and the height-owner snap within the
  snap distance.

### Vehicle models and lamps (`RENDER-26`)

Cars are the Kenney models (`godot/assets/models/vehicles/civilian/` sedan, sports sedan, SUV,
luxury SUV; `freight/` delivery van) with their colormap atlas variants. Generic Finnish models
were tried on this branch and withdrawn as not good enough to ship; they are kept off-tree. The
Kenney meshes carry no lamp data, so `vehicle_lamps.gd` holds a measured table of front and rear
lamp anchors per model (left lamps mirror X); procedural cars derive theirs from the AABB.

At night (sun below 4 to -2 degrees, as the street lights):

- `vehicle_lamps.gd` adds a camera-facing glow quad per
  lamp (`vehicle_lamp_glow.gdshader`) that keeps at least 1.6 pixels, so traffic shows across the
  map; headlights show from ahead, tail lights from behind;
- the 12 drawn cars nearest the camera within 100 m get a real `SpotLight3D` each
  (`vehicle_headlights.gd`, 60 m range, inverse-square decay, 35 degrees with a soft rim, no
  shadows; see below), chosen in Rust by
  `nearest_car_poses` (O(drawn cars · log 12) with a bounded heap). They light the road, trees,
  walls, walkers and the car ahead by their own colour;
- every car carries a light box (`vehicle_headlight_beam.gdshader`) that rebuilds each covered
  pixel from the depth buffer and adds a small red tail glow behind, and for cars from 70-100 m
  out (where the spot lights fade) the same cone the real headlight casts: Godot's SpotLight3D
  falloff with its 60 m range, inverse-square decay, 35° angle and rim attenuation 3, lit as level
  ground. This light is added blind to surface colour, acceptable only at that distance. Near the camera the box shrinks in the vertex shader
  to the tail region; by day and past 450 m it collapses and draws nothing.

Measured 2026-10-09 on the M2 Pro, `no-headlights` save (7 cars), release build, 120 frames,
old (Kenney models, no lamps) and new alternated three times: by day the new models cost under
0.1 ms. At 23:00, with real lights already on for the street lights: saved view +1.15 ms,
eye level +0.8 ms, 32 m overhead +0.4 ms with a 45 m spot range. Without the spot lights the
lamps cost 0.1-0.35 ms; the spot lights' cost follows their reach, not their count (6 or 12 the
same), and a 28 m range saved 0.45 ms in the saved view (about +0.85 ms over old in total), so
the range is 30 m. The 30 m setting itself was checked visually, not re-timed.

Near and far headlights did not match: up close the lighting shrank and dimmed. The real lights
were about 100 times weaker than the far light box on ordinary surfaces (their light is scaled by
albedo, the box's is added blind to it), fell off linearly instead of with the square of distance,
and reached 30 m against the box's 45. `tools/bake_headlight_projector.py` calibrates the
two against each other and bakes the box's own low-beam pattern into a spot-light projector
(`godot/assets/textures/vehicles/headlight_projector.png`). Godot 4.7 applies a projector only to
a shadowed light (`SpotLight3D` warns so; `storage_rd/light_storage.cpp` builds the projector
matrix only for shadowed lights), and four shadowed lights cost 1.9-2.9 ms a frame on the M2 Pro,
two 1.3-2.1 ms; hard shadows or an empty caster mask saved little, so the cost is per-pixel
shading. They are off (`SHADOWED := 0`), and every light is a plain cone: 60 m range,
inverse-square decay, 35 degrees with a steep rim falloff, energy 1200, matching the box's hot
spot near 20 m and overlighting the first few metres. Against the old 30 m lights (same save,
23:00, 120 frames, alternated twice): close view -0.1 ms, saved view +0.4 ms, overhead +0.15 ms.
Shots: `screenshots/2026-10-09-roads/headlights-4-shadowed.png` and
`headlights-shadowless-cone.png`.

Far and near headlights still did not match (2026-10-14): from 70 m out a car lit a wide white
block with straight edges, and close up a narrow soft cone. The box drew its own low-beam pattern,
still strong at the box's front face (45 m) and side faces (16 m), and lit each pixel with a
normal rebuilt from depth derivatives, which broke at every kerb and depth step into bright dashes
and straight-edged patches. The box now draws the real light's cone (above), lights everything as
level ground, and reaches 60 m ahead and 42 m (tan 35° × 60 m) to each side, where that cone is
spent; it fades out below the box's 8 m top. Its energy (62) and a ground tint (1.07, 1.0, 0.85)
were fitted to the mean light the real headlights add to the same views (`light-road` save at
01:00, cars 60 and 110 m away, all-real against all-box: 86-110 % per channel before the tint).
Frame cost was not measurable apart from the street-light change it shipped with (`ROAD-59`).

## Debugging

Run:

```bash
./run.sh --debug traffic
```

or:

```bash
./run.sh --debug-traffic
```

Traffic debug logging goes to stdout through `traffic_log!` on the `traffic` debug channel, each
line prefixed `[DEBUG:traffic]`. The channel can also be switched on in game from the debug
console; see [`debug.md`](debug.md).

Important log markers:

- `[JUNCTION_ENTER]`
- `[JUNCTION_BYPASS]`
- `[JUNCTION_EXIT]`
- `[JUNCTION_WAIT]`
- `[JUNCTION_MISSING_CONN]`
- `[JUNCTION_MISSING_EDGE]`
- `[JUNCTION_MISSING_EXIT]`
- `[LANE_CHANGE_START]`
- `[LANE_CHANGE_WAIT]`
- `[OVERTAKE_START]`
- `[OVERTAKE_RETURN]`
- `[ACCESS_EGRESS_ATTACH]`
- `[ACCESS_INGRESS_DETACH]`
- `[ACCESS_INGRESS_WAIT]`

When traffic debug is enabled, the agent path overlay (`P`) also shows each car's lane and
junction labels and the connector it will take. To see every connector at nearby junctions,
colored by whitelist, turn speed or occupancy, and to log cars stuck on a connector, use the
`junction_connectors` visual mode ([debug.md](debug.md#junction-connectors)).

## Performance Contract

Traffic code is a hot path. New traffic behavior must preserve these rules:

- no allocation inside per-agent tick loops
- reuse existing lane buckets and scratch buffers
- use `rayon` through existing dispatch helpers for independent agent work
- keep per-agent decisions O(1) or bounded by tiny per-edge lane counts
- do not add a new spatial structure when lane buckets, edge lanes, the road graph, or existing
  indices answer the query
- keep route planning and traffic movement separate; local traffic behavior must not trigger
  per-tick CCH pathfinding

### Short agent-tick contract (`PERF-06`)

Routine traffic changes use the ignored release test
`simulation::economy::agents::tick::contract::benchmark_agent_tick_contract`. It warms each isolated
lane fixture once, then records one setup-free timed tick for the 10k and 30k agent tiers. The
test emits one `PERF_AGENT` JSON line per mode and tier with deterministic row counts for lane
bucket preparation, speed updates, lane claims, movement, the end-of-tick lane fill and frontage-delay
aggregation. The fixture uses one long straight lane, fixed 16 m spacing and no route-planning
work; construction and buffer allocation stay outside the timed tick.

The bounded counts are the contract's first diagnostic. Every emitted phase count must grow by at
most `1.2 × (30k / 10k) = 3.6×`; a zero count at 10k must remain zero at 30k. A failed count is
investigated before any timing comparison.
The one timing sample per tier is a routine regression signal, not a speedup claim. Keep a
release baseline executable beside the current executable and report both build identities when
the contract runs. Matched alternating process pairs remain reserved for milestone comparisons
and claimed speedups. The short contract is run serially with no concurrent build or measurement;
the 30-minute Criterion `agent_benchmark` is no longer a routine gate.

Movement now owns route-query scratch by shards that each take every shards-th chunk of 256
agents (`SIM-02-K` part 4), retaining the existing
small-city serial threshold and separate ordered dynamic movement. Scoped TLS access restores
the previous scratch on return or unwind; this preserves planner entry points while making
memoization independent of which Rayon worker executes a range. Storage is prepared outside
agent loops and bounded by worker count and graph size. The 2026-10-14 owned-scratch build
(`c7ca3b0bf9514d00a0e89d5c7f02a16b47fc0489da9536418073c08de8a2d66a`) passes the short
agent contract: network 0.205/0.587 ms, egress 0.472/1.334 ms, ingress 0.404/0.427 ms at
10k/30k. Agent work is exactly 3× and lane work stays constant. These single samples are
routine validation, not speedup evidence. See `economy.md` for repeatable whole-day CCH counts.

Fresh infrastructure check (2026-10-13): the release test above passed with eight Rayon
workers and inherited CPU affinity, run serially as
`RAYON_NUM_THREADS=8 "$TEST_BINARY" simulation::economy::agents::tick::contract::benchmark_agent_tick_contract --exact --ignored --nocapture --test-threads=1`.
Executable SHA-256: `b660a61e02aa320874e141994a55a518dcdf78efd60569bdba60694dd111c84e`
(working tree based on `395b810f6575ac8c73eee0ff1cbc1d5c5b2b9ad3`).
Network ticks measured 0.417/1.077 ms at 10k/30k, access egress 0.556/1.368 ms,
and access ingress 0.211/0.316 ms. All exercised agent-row counts grew exactly 3×,
lane counts stayed at 16, and fresh-fixture counter repeats matched. The complete test
took 0.05 seconds including setup. These single samples validate the short runner and
work bounds; they establish no speedup against the retained baseline.

`PERF-06` acceptance (2026-10-14): the complete performance runner passes all six agent samples
and deterministic repeats with eight Rayon workers pinned to CPUs `0,2,4,6,8,10,12,14`.
Current executable SHA-256 is `06e8aa0a4a2899a89cff7ad78bcfea5a6c2eb32ee2bdabbd3f2c33449347074c`;
the retained baseline executable's verified SHA-256 is
`b34a0bedb12609c2b1fb8382d2e4f2cabc5e0102553fcd59fca46944fd88d546`.
Network ticks measure 0.242/0.507 ms, access egress 0.458/1.342 ms and ingress 0.143/0.313 ms
at 10k/30k. Every exercised agent count is exactly 3×; lane counts remain 16. The 36 targeted
agent-tick regression tests pass (eight manual tests ignored). These are routine contract
samples, not a measured speedup against the baseline.

Current known bounded scans:

- adjacent lane lookup scans the current edge's lane list, whose count is tiny for supported road
  presets
- connector selection scans `next_lanes` from the current lane, bounded by junction fan-out
- direct pass-through detection scans only the candidate `next_lanes` of the current lane, bounded
  by degree-two split fan-out
- lane bucket gap checks use sorted vectors and `partition_point`

### Claim Preparation Audit Measurements (2026-09-12)

Five alternating unprofiled release process pairs used eight physical cores (`taskset -c
0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`); three pairs used CPU 0 and one worker. The isolated
SoA fixture includes idle agents, walking/car egress, normal road travel, imminent lane endings,
stopped cars, invalid lanes and junction travel. One lane is sufficient for these classification
branches; placement, route planning, fixture allocation and checksum validation are outside timing.
Each process warms up three times, then records 21 samples of eight preparations. Every output
flag matches across versions and worker counts.

| Agents | Eight-core idle before → after, ms | Eight-core mixed before → after, ms |
| --- | --- | --- |
| 1,024 | 0.000631 → 0.000693 | 0.001533 → 0.001574 |
| 16,384 | 0.009895 → 0.010659 | 0.025071 → 0.024581 |
| 131,072 | 0.080190 → 0.014952 | 0.200441 → 0.030323 |
| 1,048,576 | 0.648757 → 0.094635 | 1.621333 → 0.225566 |

| Agents | Single-core idle before → after, ms | Single-core mixed before → after, ms |
| --- | --- | --- |
| 1,024 | 0.000631 → 0.000694 | 0.001604 → 0.001548 |
| 16,384 | 0.009894 → 0.010864 | 0.024039 → 0.024992 |
| 131,072 | 0.080474 → 0.089581 | 0.199360 → 0.200038 |
| 1,048,576 | 0.648518 → 0.687894 | 1.607550 → 1.636986 |

The accepted tradeoff is a small serial cost: the largest single-worker idle case adds about
0.039 ms and the mixed case adds 0.029 ms per preparation. Small-city differences remain below
0.001 ms in these runs. These are classification timings, not full-frame or full-movement timings.
The existing lane-claim order, actual movement, legal detach checks and lane-gap rules are unchanged.

Run `BINARY --exact simulation::economy::agents::tick::claims::tests::benchmark_claim_preparation
--ignored --nocapture --test-threads=1` with the affinity/worker settings above. Both hot predicates
explicitly retain their original in-loop execution in the accepted build.

### Speed Update Audit Measurements (2026-09-12)

The isolated speed-phase fixture uses one straight road lane and one straight connector, with
fixed 16 m spacing and the final agent at least 1,000 m from the lane end. It exercises road cars,
a mix of road cars/walkers/inactive agents/junction cars, and an all-inactive case. Placement,
route planning, occupancy construction, allocation and result hashing are outside timing. Each
process performs three warmups and 21 samples of eight updates; the same fixed sequence produces
identical speed, blocked-time and cooldown bits across builds and worker counts.

Five alternating release process pairs use eight physical cores (`taskset -c
0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`); three pairs use CPU 0 with one worker.

| Agents | Eight-core road before → after, ms | Eight-core mixed before → after, ms |
| --- | --- | --- |
| 1,024 | 0.013677 → 0.012956 | 0.012443 → 0.011602 |
| 16,384 | 0.081637 → 0.068452 | 0.055334 → 0.043820 |
| 131,072 | 0.613947 → 0.551550 | 0.335940 → 0.280316 |
| 1,048,576 | 5.971705 → 6.180697 | 3.360615 → 2.515615 |

At 1,048,576 agents, single-core road timing is 43.944670 → 46.577261 ms and mixed timing is
31.389801 → 24.839429 ms. The fully occupied single-lane stress case therefore has an accepted
3.5% eight-core / 6% single-core cost; the change is not a universal timing improvement. Idle
work retains comparable timings.

The change removes one retained `f32` per speed-buffer slot (4 MiB in the largest fixture) and an
O(A) serial copy. Existing per-agent gap lookup remains O(log K) in that lane's occupancy, with
bounded per-edge lane queries. This fixture intentionally concentrates traffic in very long
lanes; it measures the speed phase rather than gameplay frame time or ordinary city density.

Run `BINARY --exact simulation::economy::agents::tick::speed::tests::benchmark_speed_update
--ignored --nocapture --test-threads=1` with the settings above. The regression
independently reverses agent storage and runs with one/eight workers, comparing every speed
and timer bit after six updates.

### Lane Reservation Audit Measurements (2026-09-12)

`AUDIT-01-A11/A12` closes collisions at connector exits and lateral lane changes. The movement
fixture isolates dispatch, including claim preparation, with 512, 8,192 and 65,536 independent
lane groups. Each connector group has two 10 m connectors entering one 100 m road lane; cases
cover an available exit, competing exits and a stationary blocker. Lateral groups have three
100 m road lanes, competing return/overtake maneuvers into the middle lane and a stopped leader.
Cruising groups have three cars 20 m apart with no maneuver. Cars move at 4 m/s in 0.25 s steps.
Fixture creation, occupancy construction and resets are outside timing. Each process uses three
warmups and 21 single-step samples; no road-end route query occurs in this minimal lane fixture.

Five alternating unprofiled release pairs use eight physical cores (`taskset -c
0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`); three pairs use CPU 0 and one worker. Source and
binaries remain fixed, with no concurrent builds or other benchmark jobs. Median process medians:

| Workload | Agents or retained lanes | Eight-core before → after, ms | Single-core before → after, ms |
| --- | --- | --- | --- |
| Clear connector exits | 131,072 agents | 4.176683 → 4.231970 | 7.400009 → 7.498916 |
| Competing connector exits | 131,072 agents | 5.973477 → 5.260708 | 7.990074 → 7.279029 |
| Blocked connector exits | 196,608 agents | 6.598414 → 6.290669 | 10.158671 → 9.745280 |
| Competing lateral moves | 196,608 agents | 2.500647 → 3.442970 | 13.851123 → 19.403899 |
| Cruising | 1,536 agents | 0.026580 → 0.055582 | 0.051212 → 0.066385 |
| Cruising | 24,576 agents | 0.216818 → 0.237956 | 0.834715 → 1.090162 |
| Cruising | 196,608 agents | 2.064773 → 2.301867 | 11.618839 → 13.034764 |
| Idle classification | 1,048,576 agents | 0.093443 → 0.078447 | 0.633645 → 0.558090 |
| Mixed classification | 1,048,576 agents | 0.234366 → 0.224720 | 1.670352 → 1.637971 |
| Empty occupancy/claim reset | 1,048,576 lanes | 0.122143 → 0.188869 | 0.122626 → 0.202618 |

The corrected contested cases intentionally admit fewer cars: one winner instead of two, or
zero when blocked. Those timing differences are not equivalent-output throughput improvements.
The runner verifies the expected transition counts separately for each build. Classification
checksums match across builds and worker counts; that fixture has no eligible lateral moves.
Classification/reset timings use 21 samples of eight preparations after three warmups. The
empty-agent reset fixture warms all buffers and isolates the wider owner array; reset is serial
under both worker settings.

The accepted eight-core cost is about 0.24 ms for 196,608 cruising agents and 0.94 ms for the
extreme lateral case, which attempts 131,072 lane changes at once. Small cruising batches show
a 0.029 ms increase; the 24,576-agent single-core case costs 30.6% more. These are movement-phase
measurements, not full-city frame timings. The retained classification buffer stays one byte
per agent. On 64-bit targets, replacing a boolean claim with an owner adds seven bytes per
retained lane (7 MiB at 1,048,576 lanes), with O(L) reset work and no per-agent allocation.

The first correct implementation serialized all lane changes and increased the eight-core
lateral case from 2.601286 to 13.154782 ms; it was rejected. The accepted version reserves fixed
lateral targets in parallel using minimum agent ID, then executes those moves in parallel.
Three classification modes in the existing byte avoid repeating the decision for ordinary
movement. Dynamic handoffs retain stable serial order.

The measured ignored tests are
`tick::movement_pass::tests::benchmark_lane_transitions`,
`tick::claims::tests::benchmark_claim_preparation`, and
`tick::movement_pass::tests::benchmark_empty_lane_claim_reset` under
`simulation::economy::agents`, with `METRUM_DEBUG=0` and the worker settings above. Rust is
1.98.1 (`48a229cea`, 2026-09-01). Baseline binary SHA-256
`ed8ee8df8150244d3637cbebbf74318d1e0676a887e6c2e36ab17df7f280e138`; accepted binary SHA-256
`71402a5b38110a6ce1b15ff553e012d0d9814dc52530fe9a0ba292cac6ca18fc`.

## Known Limits

- normal road bends do not yet use curvature speed limits
- full IDM approach-speed interaction is not implemented because lead-vehicle speed is not tracked
- overtaking has no driver personality, urgency, emergency behavior, or multi-car prediction
- there are no traffic lights, stop signs, yield priorities, or priority-road rules yet
  (`TRAFFIC-06` plans junction priority, yield and stop control); a roundabout's entries give way
  to the ring ([round R7c](#give-way-on-entry-to-a-roundabout-road-62)), nothing else does
- lane changes are centerline S-curves, not full swept-body collision geometry
- resident parking trips, curb queues and building entrance reservations are not yet modeled; physical supply and parked test cars are described above
- connector lanes are generated from lane necks; if road geometry changes materially, lane rebuild
  must keep connector lanes and `next_lanes` in sync
- a border queue of held cars is not shortened by the entry hold: the borders' throughput (one
  single-lane trunk per 10,000 residents in the perf fixture) is a separate fixture question
- walkers keep car spacing (2.7 m) in lane buckets, connector entry claims and overlap correction
