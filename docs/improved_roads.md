## Improved Roads

This document defines the replacement road renderer for `Metrum Rise`.

It is based on the `graph-road-renderer` proof of concept Veeti Reis made for this project, not on the
existing junction patching system.

### Goal

Replace the current road top-surface renderer with a geometry model that is robust at arbitrary
angles and simple enough to reason about.

The renderer must guarantee:

- no sidewalk triangles inside junction asphalt
- no edge-vs-node seam logic based on inferred polygon ownership
- no special-case dependence on 90-degree intersections
- clean dead-ends, bends, T-junctions, merges, and 4-way junctions

### Core Idea

Do not build the visible road by:

- emitting independent road ribbons
- emitting independent sidewalk ribbons
- constructing a custom junction polygon between them
- trying to trim or patch the overlaps afterward

That is the old system and it is the root of the failures.

Instead, treat the visible road as a dilation of the road graph skeleton:

- each road edge contributes a thick road strip
- each junction node contributes a filled road disk
- sidewalks are the same construction at a wider radius

This is the exact `graph-road-renderer` concept adapted to this codebase.

### Scope

This renderer replaces the **top surfaces** of the visible surface-network renderer.

It should preserve:

- the existing Godot material contract
- the separate lane-marking mesh
- bridge and tunnel support
- dead-end round endpoints

The first goal is correctness and robustness. Perfect non-circular junction silhouettes are not
the first milestone of this renderer.

### Geometry Model

#### Edge Road Strip

For a road edge:

- `road_half = edge.width * 0.5`
- emit one road strip for each centerline segment in the polyline
- emit round joins only at interior polyline corners

#### Edge Sidewalk Base

For a standard, ramp, or bridge road:

- `outer_half = road_half + SIDEWALK_WIDTH`
- emit one **wider sidewalk-colored strip** for each centerline segment
- emit round joins only at interior polyline corners
- do not try to clip the sidewalk base around the junction throat

The visible sidewalk comes from drawing the asphalt surface on top of this wider base.

#### Junction Node Road Fill

For a junction node:

- collect all incident road edges except tunnels
- `road_radius = max(edge.width * 0.5)` of incident edges
- emit a tessellated disk centered at the node with radius `road_radius`

#### Junction Node Sidewalk Fill

For a junction node with at least one surface road:

- `outer_radius = max(edge.width * 0.5 + sidewalk_width)` across standard/ramp/bridge edges
- emit a tessellated **outer disk** in sidewalk color
- emit the road disk on top afterward

This guarantees the visible contract:

- asphalt is one continuous filled area through the junction center
- sidewalk is only **visibly** outside the asphalt radius
- there is no contour clipping or node/edge ownership math at the junction throat

### Node Classification

Not every node gets a junction disk.

#### Terminal node

- one incident surface road edge
- emit the same circular disk primitive as a junction, sized from the only incident edge
- this is the implementation's round road end-cap

#### Pass-through split

- exactly two incident road edges
- directions are nearly anti-parallel
- no node disk
- edges meet with butt ends only

This avoids the circular bubble on a straight split road.

#### Junction node

- any node with 2+ incident roads that is not a pass-through split
- emit node road disk
- emit node outer sidewalk disk underneath it
- edge strips continue through the node; overdraw resolves the visible ownership

### Overdraw Rule

The new renderer does **not** use edge-vs-node trims for the visible top surface.

Instead:

- sidewalk-colored geometry is emitted first as the wider base
- road-colored geometry is emitted second on a slightly higher layer
- lane markings are emitted third on a separate overlay layer

This is the `graph-road-renderer` approach: the visible union comes from simple dilation
primitives and draw order, not from computing custom handoff contours.

### Lane Markings

Lane markings remain a separate mesh pass.

For each edge:

- trim markings by the road disk radius only at true junction nodes
- emit dashed divider lines only on the surviving interior segment
- emit no markings inside junction disks

This follows the proof-of-concept behavior: markings stop before intersections.

### Tessellation

No polygon boolean library is required for phase 1.

The renderer uses tessellated primitives:

- strip quads for edge interiors
- disk fans for road junction fills
- disk fans for sidewalk junction fills
- narrow overlay quads for lane markings

The tessellation density should be radius-aware but small:

- minimum 8 sectors
- higher for larger radii if needed

### Godot Material Contract

Keep the current shader expectations where they still matter:

- road top surface uses `COLOR.a < 0.1`
- sidewalk uses `COLOR.a > 0.9`
- lane markings stay on the separate marking mesh

For the new sidewalk base geometry:

- world-space texturing still comes from `world_pos.xz`
- `UV.y` is no longer treated as a road-edge contract
- sidewalk vertices are written with `UV.y = 1` everywhere to disable the old curb-gap assumption

### Bridge And Tunnel Handling

This rewrite replaces the broken standard-road junction mesher.

Bridge and tunnel support remain, but with the same primitive model:

- bridge deck top surface uses the same widened-base plus asphalt-overdraw model
- bridges also emit a separate concrete strip mesh for the deck/body
- tunnel road top surfaces are skipped in this renderer path
- foot edges render as simple sidewalk-colored strips

### Tests

The new tests should be black-box geometry tests, not implementation-shape tests.

Must cover:

- straight road keeps terminal asphalt and sidewalk caps
- pass-through split does not create a circular center bubble
- arbitrary-angle bend keeps asphalt connected and sidewalk outside
- obtuse bend keeps asphalt connected and sidewalk outside
- T-junction center is asphalt-owned
- 4-way junction center is asphalt-owned
- editor-path diagonal merge has no sidewalk inside the junction core

Tests that depend on the old exact contour shape should be removed or rewritten.

### Explicit Non-Goals For Phase 1

This renderer does **not** attempt:

- exact polygonal intersection silhouettes
- curb-perfect sidewalk booleans
- non-circular multi-arm plaza shaping

Those can be a later phase. Phase 1 is the robust graph-road-renderer style surface solve.

### Replacement Rule

The current active road junction contour builder in `road.rs` is not to be patched further.

The replacement renderer should:

- remove the inferred contour / boundary / band ownership code from the active path
- build the visible top surface from widened strips, road strips, and node disks
- rely on overdraw and layer ordering instead of edge-to-node clipping
- keep only low-level mesh emission helpers and minimal bridge/tunnel handling
