# In-engine documentation

Two separate things, and they should not be confused. Godot's own F1 class
reference is built and takes any picked folder; that section is at the end. The
gym, the zoo, and the museum are a different design, still open, and the rest of
this file is that. [`README2-ONBOARDING.md`](README2-ONBOARDING.md) states the
convention in one section; this owns the mechanism. Tracked as `GYM-01` and
`GYM-02` in [`roadmap.md`](roadmap.md), both open.

## The law: rot tracks how much is baked

Documentation decays at a rate set by how much of it was frozen at authoring
time. Images rot fastest, then saved scenes, then code-driven simulation, and
something generated from a registry effectively never rots at all.

Wube state it about their own game. Their Tips and Tricks "are still using
images, which means they become outdated as we update things", and the fix was
live rendering a real simulation of the entities inside the GUI. Factoriopedia
went further: "all we needed to do was just aggregate the useful information in
one GUI", which covers modded content for free, because nothing was written
per entity in the first place.

That is the whole design constraint. **Nothing is authored per system.** A hall
of hand-built scenes is a second thing to maintain, so the moment a system
changes, the scene demonstrating it is wrong and nobody notices. The floor plan
is the one authored thing, and it is data.

## The four kinds

Quoted directly from Robin-Yann Storm's talk.

**Gym** is "for character controllers, for player movement, and for character
animation", with colour-coded metrics: "Green is a good distance, that's easy.
Orange is hard but possible. Red is too difficult." A system under load and
against its edges, with the thresholds visible.

**Zoo** is "for 3D art assets, items, and NPCs, for VFX, audio, materials, for
vignettes, and art directors", grounded in knolling, the arrangement practice
coined by Andrew Kromelow in the 1980s. One of everything, laid out in a grid,
at real scale, under the same light.

**Museum** is "for technology and systems, for shaders and rendering, and for
physics and prefabs". The cases that were once broken, in a state that still
reproduces them.

**Spatial documentation** is the fourth he names: in-world notes and markers,
where "the world itself becomes the documentation".

Storm systematises an existing practice. His own 1990s-origin claim is
unverified and he says so, having emailed Valve's Jeff Lane and Erik Kirchmer
without reply. The strongest confirmed prior art is
`de_nuke_zoo`, shipped in the CS:GO SDK in 2016 by environment artist Robert
Briscoe, nine years before the talk.

## The generator

One scene, built at load from a registry. Adding a system means adding a
registry entry, never a scene.

The build walks the registry and, for each system, produces an exhibit: a
placard read from the system's own source, a demonstration it can run, and a
plot on the floor. Wings are assigned per entry (gym for things under load, zoo
for things compared side by side, museum for defects), and placement is knolled
within each wing so the layout never depends on iteration order.

This shape is built and certified in 2.5D_engine as `museum_node.gd`, 443 lines
with a 477-line spike, golden `0x7B36B7B1`, coverage 160 of 160 with the museum
exhibiting itself. Its companion `registry_audit.gd` is the CI failure for a
system with no exhibit, standing at zero. The port brings it over; see
`METRUM_RISE.md` in the engine repo for the conversion spec.

## What has to exist here first

The engine's generator keys on four things, and metrum_rise has none of them
in a form it can walk.

| What the generator needs | Engine | metrum_rise |
|---|---|---|
| An enumerable set of systems | `_node.gd` files in one directory | Rust modules, not enumerable at runtime |
| A placard per system | the MANIFEST description, read from source | manifests exist on touched files only |
| A result to surface | a committed golden hash per node | pass/fail tests, no hashes |
| A demonstration per system | a `_spike.gd` beside each node | six spikes, all junction work |

**The registry is the blocker, and it is the same one the Rust port has.**
Rust has no class-by-name lookup and no directory to walk, so both the engine
port and this need one link-time registry: a macro that declares a system also
submits its entry (name, wing, placard, how to demonstrate it), collected via
`inventory` or `linkme`, the same mechanism godot-rust already uses to register
classes. Adding a module adds its entry, exactly as dropping a file in a
directory did. Building it once serves both.

## The manifest index is the first registry

`godot/addons/file_browser/index.gd` walks the project, reads whatever header
each file declares, and answers questions about the set. That is a registry in
the sense this design needs: it knows every file, what each one is, and which
sections it holds, without anyone maintaining a list.

`build_exhibits` groups it into the three wings. A file's `wing` field wins
outright, because a person who states where their work belongs is right;
otherwise it is inferred from `kind`, with a test going to the gym since it
exercises a system under load, a doc to the museum since it describes one, and
everything else to the zoo as a specimen. The placard is the file's own
`description` read from its header, so it cannot disagree with the file: there
is no second copy to drift.

`unexhibited` is the coverage number. A file with no manifest has nothing to
exhibit and is invisible to the docs, which is the audit the engine's
`registry_audit` runs at zero. Here it is a list you can read.

## Godot's F1 reference

Separate from the three wings, and built. Godot has its own documentation
browser, the class reference opened with F1 or by right-clicking a symbol and
choosing Open Documentation. A folder picked in the dock reaches it.

No plugin API adds a page. The class reference is built from compiled-in XML and
from scripts the editor recognizes; `EditorPlugin` has no such method, and
`ScriptEditor` exposes only `_help_tab_goto`, which navigates to a page that
already exists. Both were checked against 4.7.1.

So the dock generates one GDScript stub per indexed document, carrying
`class_name` and the manifest as `##` comments, and a stub is a real global class
the browser indexes. `@tutorial(Source)` carries the original path onto the
rendered page.

**Markdown only.** Code is what the browser is for, and it already searches by
term, author, date, kind, and any declared field. Generating a page per source
file would put several hundred module stubs in front of the documents somebody
opened F1 to find, so `PAGE_EXTENSIONS` holds `md` and nothing else. The
coverage count follows the same rule: a Rust file without a header is a gap in
the browser, not in the reference, and counting it would report a failure
against files that were never going to be pages.

A file with no `res://` path cannot itself be a help page. A stub that names it
can, and that is the whole reason the generated directory exists.

`spike_file_browser.gd` drills all of it headlessly, 89 checks: the walk, the
index, search on every facet, injection and parsing across the five strategies,
autofill over a ten-file batch, the overwrite rules, an external folder, page
generation, class-name collisions between two files sharing a basename, and
stale-page removal. `spike_file_browser_live.gd` covers what needs a real tree,
running as an `EditorScript` from File > Run: the dock, the async scan, search
narrowing the live tree, and the Generate button.

It covers the Rust too. The crate already carries manifests; the index simply
could not reach them, because a Godot project sees `res://` and nothing else.
Adding a folder in the dock walks it alongside the project, and a file's own
`script_path` is its identity there, so `control.rs` lists as
`rust/src/simulation/network/graph/control.rs`, and an absolute path that means
nothing on another machine never appears.

The link-time registry described above is still the answer for the engine port,
where a Rust module has no file the index can read. It is not needed here.

## What already exists

`godot/spike_record.gd` is the measurement half. It records pass/fail checks,
records metrics that are not pass/fail, writes a JSON baseline per spike under
`user://spike_runs`, and prints the comparison against the previous run. Storm's
colour-coded metrics are a threshold band over a metric it already collects, so
green, orange, and red are a presentation layer over existing machinery.

The six spikes under `godot/spike_*.gd` are the raw material for the gym's
first wing. Each was written against a live failure and each stays because the
system it exercises still needs checking after the next change: they build
cross junctions, spawn traffic, install two-phase signals, group junction arms
into streets by geometry, cluster car positions, and count overlapping vehicles.

They are `SceneTree` scripts run from the command line, not scenes a person
loads from the running game. Making them exhibits means the generator invokes
them, which is what the engine's generator already does with its own spikes.

## The open decision

An engine exhibit surfaces a golden hash, because every node there has one. A
metrum_rise exhibit has no hash to show. Either the golden mechanism arrives
early with the port so exhibits here carry hashes too, or exhibits surface a
weaker result (suite pass, recorded metric against baseline) until it lands.
Undecided.

## GYM-02: the API surface

The three wings cover systems you can watch. The rest of the surface, the 180
`#[func]` methods on `SimulationNode`, uses Godot's own class reference:
right-click a node or property in the editor, choose "open documentation", and
the entry opens in the editor, with no browser involved.

godot-rust exports Rust doc comments into that reference, so a `///` line above
a `#[func]` method becomes the answer a contributor gets without leaving the
editor. This is close to free once the doc comments exist, and it needs no
registry. Entries link out to the owning document here when the full design
context matters.
