# Metrum Rise — Algorithmic and Data-Structure Analysis

**Audience**: Reader with expertise in graph theory.
**Scope**: Critical evaluation of every major algorithm and data structure currently in the simulation, planned successors, and a comparative assessment of alternatives with respect to the 1,000,000-concurrent-agent scale target.

---

## 1. Core Data Structures

### 1.1 `DataGrid<T>` — Environmental and Spatial Grids

**What it is.**
A flat `Vec<T>` of length `width × height`, row-major, with stride `width`. Random access is `data[y * width + x]`, O(1). Bilinear interpolation is implemented for `DataGrid<f32>`. Used for terrain heightmap (1000 × 1000), pollution, noise, and desirability (all 500 × 500 at current `ENV_GRID_SIZE`).

**Why this choice.**
Row-major flat layout gives contiguous memory, which means:
- A single `rayon::par_chunks_mut(width)` call slices the data into cache-aligned row spans with zero extra allocation. Each thread works on its own row, no cross-thread writes.
- Prefetcher behaviour is optimal for the 4-neighbour stencil used in diffusion: the three in-row neighbours (left, right, self) are in the same cache line or the adjacent one; only the top/bottom neighbours require a separate prefetch.
- Bilinear interpolation is a direct read of four adjacent elements.

**Alternatives and why they are worse here.**
*2D `Vec<Vec<T>>`*: pointer indirection per row, no guarantee of contiguous allocation, breaks Rayon row-slicing. Rejected.
*Sparse representation (e.g. `HashMap<(u16,u16), f32>`)* : viable only when the field is near-zero almost everywhere. Pollution, noise, and desirability are dense after any activity at all. HashMap overhead (~40–60 ns per lookup) is catastrophic for a 250,000-cell stencil update at 10 Hz. Rejected.
*Octree / quadtree*: appropriate for highly non-uniform resolutions. The environmental grids are intentionally coarse (500 × 500 = 40 m/cell), so non-uniform refinement buys nothing and adds pointer-chasing. Rejected.

**Open issue.**
The diffusion tick calls `self.grid.clone()` at the start of every tick to create a read-only snapshot to diffuse from. This is a **heap allocation of ~1 MB per grid (3 MB total) per tick**. At 10 Hz this is 30 MB/s of allocator pressure. The standard solution is double-buffering: pre-allocate two grids at startup and swap pointers. This would reduce hot-path allocation to zero.

---

### 1.2 `TransitGraph` — Road Network

**Structure.**
Two parallel flat `Vec`s: `nodes: Vec<Node>` and `edges: Vec<Edge>`, each indexed by a `u32`/`usize` ID. This is a compact adjacency-list graph. Three auxiliary acceleration structures co-exist:

| Structure | Key | Value | Purpose |
|-----------|-----|-------|---------|
| `spatial_edge_grid` | `(i32, i32)` 512 m chunk | `Vec<usize>` edge IDs | AABB-overlap edge queries |
| `spatial_node_grid` | `(i32, i32)` 16 m chunk | `Vec<u32>` node IDs | Proximity snapping in `find_or_add_node` |
| `adjacency` | `u32` node ID | `Vec<usize>` edge IDs | O(1) outgoing-edge lookup |

Soft deletion: edges are never physically removed; `edge.deleted = true` marks them logically absent. `compact_edges()` does a one-shot remap with `old_to_new: HashMap<usize, usize>`, updating agents and zoning. The CCH contracted graph is rebuilt from scratch on topology changes rather than incrementally remapped.

**Turn restrictions.**
Each `Node` carries `lane_connections: HashMap<(from_edge, from_lane), Vec<(to_edge, to_lane)>>`. The absence of a key means all turns are permitted. This is a sparse representation appropriate when most junctions are unrestricted.

**Why flat Vecs.**
Random access by integer ID is O(1) with trivially predictable cache behaviour. Pathfinding indexes `nodes[id]` millions of times per second; any pointer-chasing (linked list, tree node) would be catastrophic.

**The HashMap problem.**
`adjacency` and the two spatial grids are all `HashMap<K, V>`. For a city with 100,000 nodes, `adjacency` stores 100,000 keys. At typical HashMap load factor and Robin Hood probing, a lookup costs ~10–15 ns when hot in cache, but ~40–100 ns when the table is cold. Since pathfinding reads `adjacency` for every node expansion, this dominates inner-loop cost.

The natural fix for `adjacency` is a `Vec<Vec<usize>>` indexed by node ID (since node IDs are dense integers from 0 to `nodes.len()-1`). Lookup becomes a bounds-check and a pointer dereference — a 3–5× speedup on the pathfinding inner loop. This would not require any interface change.

**Alternatives.**
*Compressed Sparse Row (CSR)*: the gold standard for static graphs. Store outgoing edges as a flat array with a prefix-sum offset array. Adjacency query is `edges[offsets[u]..offsets[u+1]]` — O(1) with a single cache line. Insert is O(E) (full rebuild). Given that road edits are rare interactive events and pathfinding runs every tick, CSR is strictly superior for the read-heavy workload. The cost is that every `add_road` or `split_edge` triggers a full rebuild — this is acceptable because CCH already requires a topology rebuild on every road edit.

*Boost-style property maps or DOD ECS*: not applicable here without an external dependency and significant restructural cost.

---

### 1.3 SoA Agent Layout — `AgentSystem`

**Structure.**
All agent state is stored as parallel `Vec<T>` fields indexed by agent index `i` in `[0, count)`. Representative fields: `pos_x: Vec<f32>`, `pos_y: Vec<f32>`, `transit: Vec<u8>`, `happiness: Vec<f32>`, `money: Vec<f32>`, `current_path: Vec<Vec<u32>>`, etc. Memory footprint is ~120 bytes/agent, so 1M agents = 120 MB.

**Why SoA over AoS.**
When the tick loop processes a single field across all agents (e.g. advance all positions, apply all pollution deltas), SoA provides full cache-line utilisation — every byte fetched from memory is useful data. AoS would load each agent's full 120-byte struct into cache to read 4 bytes of `pos_x`.

For Rayon parallelism, SoA is essential: `pos_x.par_chunks_mut(CHUNK)` divides agents across cores with no false sharing. AoS would require per-agent locks or atomic operations.

**SoA weakness.**
When the loop needs many fields simultaneously per agent (the general case in the FSM), SoA causes multiple independent cache misses — one per Vec. The AoSoA (Array of Structures of Arrays) layout, where `N`-wide SIMD batches are the unit of storage, would allow AVX2 to process 8 agents at once for arithmetic operations. This is the layout used in production AAA engines (EA SEED, Frostbite) and would be the right direction post-v0.01.

**Kill operation.**
Swap-and-pop in O(1): the dead agent's index is filled with the last agent's data, `count` is decremented. All parallel Vecs must be swapped in sync, which `AgentSystem::remove_agent` does. This is correct and O(1), but it does mean agent IDs are not stable across ticks — any external system holding an agent ID must be invalidated or use a generation counter.

---

### 1.4 `HpaGraph` — Current Pathfinding Precompute (v0.1 blocker: replaced by CCH)

**Current structure.** The live implementation uses HPA* (Botea et al., 2004): `abstract_edges: HashMap<u32, Vec<AbstractEdge>>` stores pairwise intra-chunk costs; `concrete_adj: Vec<Vec<(u32, usize, f32)>>` is the hot-path adjacency cache (Vec-indexed, not HashMap). See §2.2 for the full algorithm description and known weaknesses. This structure is removed when CCH (§2.9) lands at v0.1.

### 1.5 `CchGraph` — Planned CCH Data Structure (v0.1)

The CCH precompute produces two persistent structures:

| Field | Type | Role |
|-------|------|------|
| `order` | `Vec<u32>` | Contraction order: `order[i]` = node contracted at step i (lowest importance first) |
| `shortcuts` | `Vec<Vec<(u32, f32, Option<u32>)>>` | Per-node upward shortcut edges: `(to, weight, via_node)` where `via_node` is the node bypassed by this shortcut |
| `upward_adj` | `Vec<Vec<u32>>` | CSR-style upward adjacency (edges to higher-ranked nodes only) — forward search |
| `downward_adj` | `Vec<Vec<u32>>` | CSR-style downward adjacency (edges from higher-ranked nodes) — backward search |
| `node_rank` | `Vec<u32>` | `node_rank[v]` = contraction step at which node v was contracted; used for upward/downward edge filtering during query |
| `max_v` | `f32` | Maximum edge speed in network; used for A*-style pruning during bidirectional search |

**Separation of topology and metrics (CRP property).** The contraction order (`order`, `upward_adj`, `downward_adj`) depends only on graph topology and is rebuilt when edges are added or removed — a rare interactive event. Edge weights (`shortcuts` field, specifically the `f32` weight component) are updated independently via the *customization phase*: propagate new weights through the elimination tree in O(E). This runs on every congestion update or speed limit change without touching the contraction order.

**Memory.** A contracted graph for 100k edges adds ~30–40% shortcut edges → ~140k edges total. At ~12 bytes/shortcut entry: ~1.7 MB. The `node_rank` Vec is V × 4 bytes = ~0.4 MB at 100k nodes. Total: ~5–10 MB, comparable to the `HpaGraph` it replaces.

---

## 2. Algorithms

### 2.1 A* on the Concrete Graph

**State key.**
The search state is `(node_id: u32, incoming_edge: usize)` rather than `node_id` alone. This is a textbook requirement for networks with arc-based constraints (turn restrictions). Without the incoming edge in the state key, the optimal substructure property of Dijkstra/A* breaks at restricted junctions: the shortest path to node `v` via edge `e₁` may not satisfy the restriction that `e₁ → e₂` is forbidden, but the path via `e₃` to `v` — which is longer — may be the only valid one. The expanded state space is O(V × E) in the worst case but bounded in practice by junction degree.

**Cost function.**
`base_cost = length / speed_limit` (time in seconds). A slope penalty `1 + (max_slope × 5)²` is applied for grades above 10%, implemented as an exponential on the polyline's worst segment. This is an asymmetric terrain-aware cost that correctly penalizes steep grades nonlinearly — a 20% grade yields a 25× penalty, a 40% grade a 400× penalty.

**Heuristic.**
`h(v) = euclidean_distance(v, goal) / max_v` where `max_v` is precomputed at graph build time as the maximum `speed_limit` in the network. This is *admissible* (never overestimates true time cost since Euclidean ≤ arc-length and max_v ≥ any edge's speed) and *consistent* (satisfies the triangle inequality). Consistency guarantees A* never re-expands a node, yielding O((V + E) log V) worst-case complexity.

**Comparison to alternatives.**
Dijkstra (h = 0) is admissible and consistent but explores far more nodes. Landmark-based heuristics (ALT algorithm) using triangle inequality on pre-computed landmark distances can give tighter heuristics in practice, especially for long-distance urban queries. This would be a meaningful improvement over Euclidean/max_v, particularly when the road network has strong directional structure (one-way arterials, motorways).

---

### 2.2 HPA* — Current Implementation (v0.1 blocker: replaced by CCH, §2.9)

**Theory.**
HPA* (Botea et al., 2004) hierarchically abstracts the search space. The map is divided into rectangular chunks. Nodes on chunk boundaries become *abstract nodes*; intra-chunk Dijkstra computes pairwise costs between all abstract nodes within each chunk, yielding an *abstract graph* on which long-distance A* can route without expanding every concrete node.

**Build phase.**
For each chunk, run Dijkstra from every abstract entry node to every other abstract entry node within the chunk. Store `(cost, inner_path)` as `AbstractEdge`. Complexity: O(K × B² × (V_chunk + E_chunk) log V_chunk) where K = number of chunks, B = abstract nodes per chunk boundary. Given 512 m chunks on a 20 km map: K ≈ 40² = 1,600; B is typically 2–8 per boundary side.

**Query phase — three phases.**
1. **Local boundary search from start**: Dijkstra within the start chunk from `start` to all abstract boundary nodes.
2. **Backward local boundary search from goal**: same, reversed adjacency.
3. **Abstract A***: connects Phase A and Phase B results via `abstract_edges`. Heuristic is `dist(v, goal) / max_v`. Search is **unidirectional** — the backward adjacency (`concrete_rev_adj`) is not used in Phase C.

**Known weaknesses (all resolved by CCH).**
- Phase C accumulates `dist + 0.0` — abstract distance is not tracked through the contracted graph; returned `dist` is inaccurate until concrete reconstruction runs.
- Abstraction completeness: edges entirely within one 512 m chunk have no abstract representation. On small graphs, HPA* degrades to full concrete A*.
- Unidirectional abstract search misses the ~2× speedup from bidirectional meet-in-the-middle.
- Chunk size (512 m) is calibrated for road networks; rail lines with 500 m–5 km inter-station gaps produce few or no abstract nodes, degrading rail pathfinding to full A*.

This section is retained as historical context. The algorithm is removed when CCH (§2.9) lands.

### 2.9 CCH / CRP — Planned Replacement (v0.1 blocker)

**Algorithm family.** Customizable Contraction Hierarchies (Dibbelt et al., 2016) / Customizable Route Planning (Vetter et al., 2009). CCH separates three phases: topology preprocessing (once per graph structure change), metric customization (once per weight change — congestion, speed limits), and query (per routing request).

**Topology phase — contraction.**
Nodes are contracted in order of increasing *importance* (edge-difference heuristic: importance(v) = added shortcuts − removed edges when v is contracted). Contracting node v: for every pair of neighbours (u, w) where the shortest u→w path passes through v, add a shortcut edge u→w with cost `cost(u,v) + cost(v,w)` if no alternative path of equal or lesser cost exists. After all contractions, the graph has V levels; a node at level k was contracted at step k.

The key property: every shortest path in the original graph corresponds to an *upward-then-downward* path in the contracted graph — forward search only follows edges to higher-ranked nodes, backward search only follows edges from higher-ranked nodes. The two frontiers meet at the highest-ranked node on the optimal path.

Complexity: O(E log E) preprocessing. For a 100k-edge city graph: ~140k edges after shortcuts, ~0.4 s preprocessing time (estimated from RoutingKit benchmarks at this scale).

**Metric customization.**
Edge weights (speed limits, congestion multipliers) can change without altering the contraction order. The customization phase propagates new weights through the elimination tree bottom-up in O(E): for each shortcut edge, recompute `weight = min(weight, cost(u,v_contracted) + cost(v_contracted,w))`. This is the CRP property that makes CCH suitable for interactive editing: a road speed change triggers only the O(E) customization, not a full O(E log E) topology rebuild.

When a road edit adds or removes an edge (topology change), the contraction order is recomputed in full. This is O(E log E) and acceptable because road edits are rare interactive events — the same justification that makes CSR acceptable for the base graph.

**Query — bidirectional Dijkstra.**
Forward search from source s expands only upward edges (to nodes with higher rank). Backward search from goal t expands only downward edges (from nodes with higher rank). The searches meet at the node u that minimises `dist_forward(u) + dist_backward(u)`. Complexity: O(√E log √E) — empirically 100–1,000× faster than A* on city-scale graphs.

**Turn restriction compatibility.**
The `(node, incoming_edge)` state key used in §2.1 must be preserved. CCH with turn restrictions expands state (node, incoming_edge) instead of node alone; the contracted graph encodes turn-restriction-aware shortcut costs. This increases the state space by a factor of average junction degree (~3–4×) but does not change the asymptotic complexity.

**Modal filtering.**
`allowed_mask: u8` (v0.01 goal 4) filters edges during both topology preprocessing and query. A CCH graph can be built per modal mask (one for CAR, one for FOOT, etc.) or masks can be applied at query time. Per-mask precomputation is preferred at scale — it produces smaller contracted graphs and faster queries.

**RegionGraph compatibility.**
CCH operates on whatever graph is present. A single `RegionGraph` containing one city tile produces a CCH contracted graph for that city. When a second city tile is added (with inter-city highway/rail/ship/air edges), the topology phase rebuilds on the full region graph and the contraction hierarchy naturally promotes inter-city connectors to the top of the hierarchy — they have high betweenness centrality and are contracted last. Cross-city queries immediately escalate to the inter-city edge level and route there, without exploring local streets of the intermediate cities.

**Flow fields.**
Flow fields (§3.1) and CCH are complementary. CCH handles heterogeneous (agent-specific, novel-destination) routing. Flow fields handle homogeneous (shared-destination) bulk routing — one reverse Dijkstra per active destination zone type, O(1) per-agent lookup. Retain CCH for immigration and one-off novel destinations; flow fields for the common daily-cycle trips (home→work→shop→home).

---

### 2.3 Diffusion PDE — Pollution, Noise, Desirability

**Formulation.**
The pollution update is:

```
u_new[i,j] = (u[i,j] × 0.60 + avg₄(u) × 0.40) × 0.995 + emission[i,j]
```

This is a first-order explicit finite-difference discretisation of the advection-diffusion-decay PDE:

```
∂u/∂t = D ∇²u − λu + S
```

where `D` is the diffusion coefficient, `λ` is the decay rate, and `S` is the source term.

With the 4-neighbour stencil and the above weights, the effective discrete diffusion coefficient per time step is approximately 0.40/4 = 0.10 per cell per step. The decay factor 0.995 per tick yields a half-life of `ln(2) / ln(1/0.995) ≈ 138 ticks`. The stability condition for explicit FD on the diffusion equation is `D Δt / Δx² ≤ 0.5`; with the chosen weights this is satisfied by construction (0.10 < 0.5) regardless of tick rate.

**Parallelism.**
The diffusion step uses `new_grid.data.par_chunks_mut(width).enumerate()`, processing one row per Rayon thread. Rows are independent (only same-grid neighbours are read, from the old snapshot), so there are no data races. This is a textbook embarrassingly parallel pattern.

**Desirability.**
The formula `50 − pollution × 2 − noise × 1.5` is a linear composite score. This is a simplification of hedonic pricing models. At 1M-agent scale, the desirability score influences building spawning and agent happiness, so precision matters less than consistency and speed. The current formula is O(1) per cell and parallelisable.

**Alternatives.**
*Alternating Direction Implicit (ADI)*: unconditionally stable, allows arbitrarily large Δt. Overkill for a 500 × 500 grid at 10 Hz where explicit stability is trivially satisfied.
*FFT convolution*: O(N log N) per step with an arbitrary kernel. More accurate for long-range diffusion but unnecessary at this resolution.
*Multigrid methods*: O(N) but complex to implement. The grid is small enough that this is not warranted.

---

### 2.4 Agent FSM and Movement

**State machine.**
Six states: IDLE, DEPARTING, ON_ROAD, ARRIVING, IMMIGRATING, INTERSECTION. Transitions are probabilistic (`gen_bool(0.05 × delta)` for IDLE→travel) or deterministic (arrival detection). The probabilistic departure means agents do not synchronise, spreading pathfinding load across ticks.

**Movement model.**
On-road movement uses a sub-tick distance budget `remaining_dist = speed × delta`. Each iteration of the inner `while remaining_dist > 0` loop advances the agent one polyline segment or until the budget is exhausted, whichever comes first. This gives smooth, frame-rate-independent movement with correct partial-segment positioning.

Lane offsets are computed from edge width, lane count, and a lane index drawn at departure. This is kinematically correct (agents stay within their lane cross-section) but agents do not interact — there is no collision avoidance, no car-following model, and no capacity constraint per lane. This is a deliberate simplification for v0.01.

**Critical scalability issue: O(N) building scans.**
In the IDLE→DEPARTING transition, every agent searching for a job scans the entire `allocator.buildings` Vec:

```rust
let jobs: Vec<usize> = allocator.buildings.iter().enumerate()
    .filter(|(_, b)| b.zone_type == ZoneType::Industrial || ...)
    .map(|(idx, _)| idx).collect();
```

This is O(B) per agent activation, allocating a new `Vec<usize>` on the heap each time. At 500,000 buildings and 10,000 activations per tick this is 5 × 10⁹ comparisons per tick — the single largest algorithmic bottleneck in the entire simulation, worse than pathfinding.

The fix is an inverted index: `Vec<Vec<usize>>` indexed by `ZoneType` (or equivalently, a `HashMap<ZoneType, Vec<usize>>`), maintained incrementally when buildings are added or removed. Building lookup becomes O(1) (index into zone list) + O(1) random selection. This is a critical prerequisite for any meaningful scale-up.

---

### 2.5 Slope Penalty in Edge Cost

The cost calculator applies `1 + (max_slope × 5)²` for slopes above 10%. This is a max-slope rather than integral-slope formulation — a single steep segment can dominate the cost of an otherwise flat road. For routing purposes, the integral of slope squared along the polyline (total elevation change weighted by steepness) would be more physically accurate and route-fair, but the max formulation is conservative in the desired direction: it avoids steep roads aggressively.

---

### 2.6 Car Collision and Traffic Flow

**Current state.**
Agents move at a fixed speed with no interaction. Two cars on the same edge and lane pass through each other. There is no capacity constraint per lane and no congestion feedback to the routing system. This is intentionally deferred past v0.01 but is a correctness failure at any meaningful scale.

**Why the problem is 1D, not 2D.**
Cars are constrained to discrete lanes on polyline edges. The collision problem does not require a 2D spatial structure. A car only needs to know the position of the single vehicle directly ahead on the same edge and lane. This reduces the problem to a sorted 1D list per lane — a fundamentally simpler and cheaper structure than any 2D spatial index.

**Per-lane occupancy lists.**
The planned structure is a transient `Vec<Vec<u32>>` indexed by `edge_idx * MAX_LANES + lane_idx`. Each sub-Vec is sorted by `edge_progression` (the polyline index already tracked in the SoA). Building this structure each tick from the existing SoA fields costs O(A) total — one pass over all agents. It is thrown away at the end of the tick; no inter-tick state is required beyond the agent's own `speed` field. Finding the car directly ahead is O(1): it is the adjacent element in the sorted sub-Vec.

**Grade separation.** Cars on a bridge and cars in a tunnel directly below it are on different edges and therefore in different lane lists. There is no cross-level interaction to handle and no modification to the per-lane structure is required when bridges or tunnels are introduced.

**Intelligent Driver Model (IDM).**
The IDM (Treiber et al., 2000) is the standard microscopic traffic model for car-following. Each car computes its acceleration from two inputs: the gap to the car ahead `gap` and the relative speed `Δv`:

```
a = a_max × [ 1 − (v / v_max)⁴ − (s*(v, Δv) / gap)² ]

s*(v, Δv) = s_min + v · T + v · Δv / (2 √(a_max · b))
```

where `T` is the desired time headway, `b` is the comfortable deceleration, and `s_min` is the minimum jam gap. This is O(1) per car given the car-ahead lookup, and produces emergent stop-and-go waves, capacity saturation, and jam dissolution without any explicit collective state.

IDM requires one new SoA field: `speed: Vec<f32>` (~4 MB at 1M agents). The current model uses a hardcoded speed constant; IDM replaces this with a dynamic per-agent value that the model updates each tick.

**Intersection queuing.**
A car at the last polyline segment of its edge whose target node has no accepted entry slot this tick sets `speed = 0`. A simple capacity model is one entry per incoming lane per tick. This is sufficient to produce realistic queue build-up without requiring explicit intersection state machines.

**Congestion write-back.**
After the movement pass, the average speed per edge relative to the edge's `speed_limit` is written into `Edge::current_congestion`. This feeds directly into the HPA* traversal cost (`base_cost × (1 + current_congestion)`) and into the v1.0 congestion heatmap. This closes the feedback loop: congested roads become more expensive in the routing cost function, which diverts future agents — the minimal model of traffic assignment.

**Parallelism.**
The per-lane lists are independent: no agent in lane `(e, l)` reads or writes any agent in lane `(e', l')`. The IDM pass over all lanes is trivially parallelisable with Rayon. The only shared write is the per-edge `current_congestion` value, which can be accumulated into a temporary `Vec<f32>` per thread and merged after the parallel phase.

**Alternatives.**

*Nagel-Schreckenberg (NaSch) cellular automaton*: discretises the road into fixed cells (typically 7.5 m each, one car per cell) and uses integer speeds with simple probabilistic rules (accelerate, check gap, brake, randomise). O(1) per cell, perfectly parallelisable, and trivially implementable. The trade-off is reduced realism — NaSch does not produce the smooth speed–density relationship of IDM and is poorly suited to roads with varying speed limits or mixed vehicle types. It is a viable alternative if IDM proves difficult to integrate with the polyline-progression movement model.

*ORCA / Reciprocal Velocity Obstacles*: a 2D collision avoidance algorithm. O(A²) naive, O(A log A) with spatial indexing. Entirely inappropriate for lane-constrained on-road vehicles — it solves a harder problem than required and at greater cost.

*Fixed headway model*: each car simply maintains `gap ≥ MIN_HEADWAY × speed`, braking when the gap closes. Simpler than IDM, but loses realistic wave behaviour and the smooth speed–density curve. Appropriate as a first implementation step before full IDM.

---

### 2.7 Grade Separation: Bridges and Tunnels

**What the current engine handles without modification.**
Edge geometry is stored as `Vec<Vector3>` — a 3D polyline. Elevated or depressed roads (bridges, tunnels, ramps) are natively representable; the engine stores whatever Y coordinates are given. The following systems require no changes:

- *Pathfinding*: A* and HPA* operate on graph topology and scalar edge costs. Bridge and tunnel edges are indistinguishable from ground-level edges in the graph. The slope penalty in `cost.rs` already applies a cost multiplier to steep bridge approaches.
- *Node snapping*: `find_or_add_node` uses 3D Euclidean distance. A bridge abutment node and a ground-level node at the same XZ position will not snap as long as their vertical separation exceeds `SNAP_TOLERANCE` (1 m). Real bridges have ≥ 4 m clearance; this condition is always satisfied.
- *Car collision*: per-lane lists are keyed by `edge_idx`. A car on a bridge and a car on the road below are on different edges and will never appear in the same lane list. No cross-level interaction occurs.
- *512 m spatial chunk index*: the chunk grid is XZ-only, so a bridge edge and an underpass edge at the same XZ location land in the same chunk bucket. This is harmless — all chunk queries use graph topology (edge indices) and never compare Y coordinates.

**What the current engine does not handle: `EdgeClass`.**
The renderer and zoning system currently assume all edges are ground-level. A single `EdgeClass` enum field on `Edge` is the minimal change needed to branch these two systems:

| `EdgeClass` | Renderer behaviour | Zoning behaviour |
|-------------|-------------------|-----------------|
| `Standard` | Road mesh follows terrain surface (current) | Normal zoning on both sides |
| `Bridge` | Floating deck mesh at geometry Y; no terrain deformation | Zoning disabled; ground cells below the deck must not be blocked |
| `Tunnel` | Portal entrance meshes at endpoints only; road hidden between portals | Zoning disabled |

The `EdgeClass` field fits in existing `Edge` struct padding — zero memory cost. No simulation logic changes beyond the renderer and zoning obstruction check.

**The zoning interaction.**
`is_cell_obstructed` tests whether a cell is occupied by road asphalt using 5-point sampling. A bridge deck at Y = 8 m passes directly above ground-level cells. Without a fix, those ground cells would register the bridge geometry as an obstruction and refuse zoning. The fix is a clearance check: if the nearest edge is classified `Bridge` and the cell's Y position is more than a threshold below the edge's geometry Y, treat the cell as unobstructed by that edge.

**Structural implications for gameplay.**
Bridges and tunnels are expensive infrastructure. Even without a full cost model, tagging edges with `EdgeClass` enables future systems to:
- Apply construction and maintenance costs per edge class.
- Set load limits (bridge tonnage) that restrict heavy vehicles.
- Generate visual assets appropriate to the structure type (bridge piers, tunnel portals, parapet walls).

None of these require changes to the simulation algorithms described in this document.

---

## 3. Planned Algorithms and Their Assessment

### 3.1 Flow Fields (v0.2 target)

Dijkstra from each destination produces a `DataGrid<f32>` cost map. Agent routing becomes a grid lookup: O(1) per agent per tick. This is the single most impactful planned change for scale.

**Limitation**: flow fields on the graph naturally produce next-node recommendations, not grid-cell recommendations. The backlog refers to a `DataGrid<f32>` cost map — this implies projecting the graph cost onto the spatial grid. The projection is lossy (multiple graph nodes may map to the same grid cell) and must handle one-way constraints carefully. An alternative is to store a `next_node` map per destination, indexed by node ID rather than grid cell.

**Interaction with CCH**: flow fields and CCH are complementary, as described in §2.9. Flow fields handle homogeneous (shared-destination) bulk routing; CCH handles the exceptional cases (immigration, novel destinations, inter-city travel).

### 3.2 Parallelised Agent Tick (v0.2 target)

The plan is `rayon::par_iter_mut` over agent chunks with `AtomicU32` for `parking_occupied`. The SoA layout makes this straightforward for most fields, but several operations are not straightforwardly parallelisable:

- **Pathfinding calls** (CCH query) require read-only access to `CchGraph` and `RegionGraph`. Both are read-only during the agent tick, so shared references across threads are safe. No issue here.
- **`building.parking_occupied` increments**: must use `AtomicU32` or per-building `Mutex`. AtomicU32 with `fetch_add` is O(1) and contention is low (many buildings).
- **`home_building` / `work_building` assignment** during immigration: currently uses interior mutation. With `par_iter_mut` over agents, cross-agent writes are ruled out by the borrow checker. This needs careful restructuring: immigration decisions should be batched and applied after the parallel phase, or buildings should use atomic vacancy counters.

The key architectural issue is that `AgentSystem::tick` currently borrows `allocator` immutably and `graph` mutably. With Rayon, mutable access to `graph` must be restructured out of the parallel loop — the agent tick should never mutate the road graph, and currently it does not (the `graph` borrow is used only for reads in practice). Removing the `&mut TransitGraph` parameter from `tick` is a prerequisite.

### 3.3 CCH / CRP (v0.1 blocker — supersedes §3.3 as a v1.0 target)

The full algorithmic description is in §2.9. This section summarises the assessment against the alternatives and the rationale for moving the milestone.

**Why CCH instead of plain CH.** Pure Contraction Hierarchies require a full O(E log E) topology rebuild on every road edit. Road edits are frequent interactive events. CRP/CCH separates topology (rebuilt on edge add/remove) from metrics (O(E) customization on weight change). The query complexity is identical to CH: O(√E log √E).

**Why moved from v1.0 to v0.1 blocker.** Two reasons:
1. The multi-city region model (§3.6) requires a single `RegionGraph` covering all city tiles. CCH's contraction hierarchy naturally promotes inter-city connectors (highways, rail, air) to the top of the hierarchy — this property is fundamental to the region architecture, not an optimisation. Deferring CCH to v1.0 would require implementing the region model on top of HPA*, which degrades badly on rail networks (§2.2 known weakness) and has no bidirectional search.
2. The `RegionGraph` rename (removing city-scope from `TransitGraph`) is a prerequisite for CCH data structure placement and should happen while the codebase is small.

**Performance at city scale.** For a single 20 km city with O(10⁵) edges: CCH query is ~1–5 µs vs ~5–50 µs for HPA* (estimated from RoutingKit benchmarks; HPA* estimate from §6.4 Wall 3). The ~10× improvement matters at 100k agents with 1% activation rate: 1,000 CCH queries/tick × 5 µs = 5 ms vs 50 ms for HPA*. At multi-city scale the gap widens to the 100–1,000× range.

### 3.4 Car Collision — IDM (v0.2 target)

The IDM implementation described in §2.6 introduces one new SoA field (`speed: Vec<f32>`) and one transient per-tick structure (per-lane sorted agent lists). The critical prerequisite is that `AgentSystem::tick` must be parallelised first (§3.2): building and consuming the per-lane lists inside a single-threaded loop has acceptable overhead, but writing `Edge::current_congestion` from a parallel pass requires either a temporary accumulator Vec or atomic floats. The congestion write-back should be structured as a separate sequential reduce step after the parallel IDM pass, consistent with the pattern planned for building vacancy counters.

The NaSch cellular automaton (§2.6) is the recommended fallback if the polyline-progression movement model makes IDM integration difficult: NaSch's discrete cell model maps cleanly onto integer `edge_progression` indices and avoids the continuous acceleration update.

### 3.5 Agent Level-of-Detail (v1.0 target)

Three tiers:
- Full FSM + rendering for camera-visible agents (~50k)
- Flow-field-only routing within 2 km (~500k): no individual pathfinding, no FSM, just grid-cell advection
- Statistical counts only beyond 2 km (~450k): no individual agents, just aggregate supply/demand numbers per zone

This is the standard LoD approach used in all large-scale agent simulations (transport microsimulation, crowd simulation). The transition boundaries must be carefully managed to prevent agents appearing or disappearing at LoD boundaries, and statistical agents must be promoted to full agents when they enter the camera radius without creating discontinuities in city-level statistics.

### 3.6 Multi-City Region Model (v0.2 target)

**Architecture.** The long-term scale target is ≥ 1,000,000 agents across a multi-city region, not a single city. Each city tile is a 20 km × 20 km map; tiles are connected by highway, rail, ship, and air edges in a single unified `RegionGraph`. Only one city tile runs full agent simulation at a time; all others are represented by a statistical model of ~15 numbers per tile.

**Statistical background model.** An inactive city tile exposes: population count, employment capacity, demand by zone type (R/C/I/O/Mixed), and throughput per inter-city connection link. These update on a coarse schedule (~1/s game time). No individual agents are simulated for background cities. The `RegionGraph` contains all tiles' nodes and edges so CCH routing through background cities works correctly — but agents on those edges are counters, not FSM objects.

**Border crossing.** When an agent leaves the active city onto an inter-city edge it is *demoted*: replaced by a statistical entry in a queue on that edge, with an estimated arrival time derived from the CCH path cost. On arrival at the destination city boundary it is *promoted*: spawned as a full FSM agent at the border node. If the destination city is inactive, the agent is absorbed into that tile's statistical population counter. `NodeType::Border` is already declared in `types.rs`.

**CCH and the regional hierarchy.** CCH's contraction order naturally elevates inter-city connectors to the top of the hierarchy because they have high betweenness centrality — thousands of shortest paths cross them. A cross-city query (agent in City A routing to City B) expands ~10–20 nodes before reaching the highway/rail level; the rest of the query runs on shortcut edges without touching local streets. This property is not engineered — it emerges from the graph topology.

**Scaling.** At 1M agents distributed across N city tiles with one active tile and N−1 statistical tiles, the active tile's simulation budget is unchanged: ~1M/N full agents plus border-crossing arrivals. The statistical model for N−1 background tiles is O(N) per tick — negligible. The `RegionGraph` grows with N but the CCH query time grows only as O(√E_total log √E_total) which is sub-linear in tile count.

**Interaction with Agent LoD (§3.5).** The LoD system (§3.5) and the regional model are complementary tiers of the same concept: both replace full FSM agents with cheaper representations beyond a distance or boundary threshold. Within the active city: LoD tiers by camera radius. At the city boundary: promotion/demotion to/from statistical queue.

---

## 4. Summary: Bottleneck Ranking for 1M Agents

The following ranks all identified bottlenecks by estimated impact at the 1M-agent, 500k-building scale target, in order of severity:

| Rank | Location | Issue | Complexity | Impact at 1M agents |
|------|----------|-------|-----------|---------------------|
| 1 | `tick.rs` IDLE transition | O(B) linear scan for job/shop assignment per activating agent | O(A_active × B) | ~5 × 10⁹ ops/tick at 1% activation rate and 500k buildings. Simulation-halting. |
| 2 | `tick.rs` | Single-threaded agent loop | O(A) on 1 core | ~10–100 ms/tick at 1M agents. Target is ≤ 33 ms total. |
| 3 | `decisions.rs` | 2 pathfinding queries per mode-decision per activation (currently HPA*, v0.1: CCH) | O(A_active × query_cost) | 20k queries/tick at 1% activation rate. 5–50 ms with HPA*; ~0.1–5 ms with CCH. Flow fields (§3.1) reduce this further to O(M) for shared destinations. |
| 4 | `tick.rs` | No car-following model — cars pass through each other | Missing system | Physically incorrect at any scale. Breaks congestion feedback loop. Fix: IDM + per-lane sorted lists (§2.6). |
| 5 | `pollution.rs` et al. | `grid.clone()` allocation per tick per grid | O(W × H) allocation | 30 MB/s allocator pressure at 10 Hz. Avoidable with double-buffering. |
| 6 | `graph.rs` | `adjacency: HashMap<u32, Vec<usize>>` | +10–50 ns per lookup vs Vec | ~5–25% pathfinding overhead on current HPA*. Fixable by converting to Vec<Vec<usize>> (v0.1 blocker, prerequisite for CCH build phase). |
| 7 | `tick.rs` | `current_path: Vec<Vec<u32>>` nested heap allocation | Allocation per path update | Cache pressure at 1M agents. Fixable with pool allocator or path arena. |

Items 1–4 are prerequisites for the 1M scale target. Items 5–7 are secondary optimisations that become significant only after items 1–4 are addressed. Rows 7 and 8 from the previous ranking (HPA* abstract distance bug and HPA* abstraction completeness failure) are eliminated by CCH and no longer appear.

---

## 5. Multi-Modal Transport: Compatibility Analysis

The following examines each transport mode against the current engine architecture, identifying what already works, what needs extension, and what requires fundamental restructuring.

### 5.1 Existing Foundation

The type system in `network/types.rs` already declares the full modal vocabulary:

```rust
enum TransitType  { Road, Rail, Ship, Air, Foot }
enum NodeType     { Junction, Station, Harbor, Airport, Transfer, Frontage }
struct TransitFlags {
    FOOT: u8 = 1 << 0,   CAR:  u8 = 1 << 1,
    RAIL: u8 = 1 << 2,   SHIP: u8 = 1 << 3,
    AIR:  u8 = 1 << 4,
}
```

`Edge::allowed_types: u8` is a bitmask that the pathfinding query filters on during edge expansion. This infrastructure is correct for multi-modal routing under both HPA* and CCH, and requires no changes.

**What does not match this vocabulary** is the agent system. The entire movement and pathfinding pipeline is controlled by a single binary field:

```rust
is_driving: Vec<bool>   // true = car, false = walking — no other modes representable
```

Every downstream consequence is hardcoded to this boolean:

| Site | Binary assumption |
|------|------------------|
| `tick.rs` speed | `if is_driving { 20.0 } else { 4.0 }` |
| `tick.rs` lane offset | `if is_driving { road lane } else { sidewalk }` |
| pathfinding query filter | `if pedestrian { FOOT bit } else { CAR bit }` |
| `decisions.rs` | Returns `(node, bool)` — driving or not |

Adding any third mode requires replacing `is_driving: Vec<bool>` with `transit_mode: Vec<u8>` and converting all four sites above to a mode-indexed lookup table. This is a contained but pervasive change — it touches `data.rs`, `tick.rs`, `decisions.rs`, and all four query call sites. Both migrations (`transit_mode` and `allowed_mask`) are designated **v0.01 Goals** — they are cheap to make while the codebase has exactly two modes, and CCH inherits the same `allowed_mask: u8` parameter unchanged. See §6 for the full milestone rationale.

### 5.2 Bicycles

**What works.** `TransitFlags` has no BIKE bit yet, but there is room (bits 5–7 are free). `allowed_types` filtering works unchanged in both the current HPA* and the planned CCH once a bit is defined. The polyline movement model, lane offsets, and IDM car-following all generalise to bikes.

**What needs changing.**

- Add `TransitFlags::BIKE: u8 = 1 << 5` to `types.rs`.
- Replace `is_driving: Vec<bool>` with `transit_mode: Vec<u8>`. Bikes become a third mode constant alongside CAR and FOOT.
- Speed table: bike ≈ 5.5 m/s (20 km/h). Currently hardcoded to 20.0 or 4.0.
- Lane offset logic: bikes use dedicated bike lanes or the road shoulder, not the car-lane centre or the full sidewalk. The offset calculation needs a mode branch.
- The pathfinding query parameter `pedestrian: bool` must become `allowed_mask: u8` passed directly from the agent's mode. The current binary maps to either FOOT or CAR; a mask generalises to any combination including BIKE. CCH inherits this parameter unchanged.
- Bike-only edges (cycle paths) need `allowed_types = FOOT | BIKE` but not `CAR`. No structural change to `Edge`; just a new edge class to set in the editor.

**Effort level.** Moderate. No new data structures. All changes are concentrated in `is_driving` → `transit_mode` migration and the four hardcoded speed/lane/filter sites.

### 5.3 Trains and Metros

**What works.** `TransitType::Rail`, `NodeType::Station`, and `TransitFlags::RAIL` are declared. The `TransitGraph` can hold rail edges with `allowed_types = RAIL` and HPA* will filter car/pedestrian agents off them. Rail edges through tunnels work identically to the `EdgeClass::Tunnel` roads described in §2.7. The slope penalty in `cost.rs` already applies to elevated rail approaches.

**Fundamental architectural mismatch.** Trains are *shared vehicles* carrying many agents, not independent agents each moving individually. The `AgentSystem` has no concept of a vehicle that multiple agents board simultaneously. This creates four structural problems:

1. **Vehicle capacity.** A train carriage holds 50–300 passengers. The current model has one agent = one vehicle unit. Modelling every passenger as an independent rail-traversing agent would mean a single 200-passenger train generates 200 simultaneous HPA* calls and 200 individual path traces — identical in every way. This is wasteful and incorrect: agents on the same train should share one vehicle trajectory.

2. **Schedules vs on-demand travel.** The FSM currently transitions IDLE → travel immediately when an agent decides to move. Rail operates on fixed timetables. An agent at a station must *wait* for the next departure. The FSM has no WAITING state.

3. **Transfer logic.** An agent commuting by rail typically walks to a station, boards a train, rides to a destination station, then walks to the final building. This is a multi-leg journey with mode switches. The FSM has no concept of mid-journey mode change or intermediate node types. `NodeType::Transfer` is declared but has no FSM handling.

4. **Single graph assumption.** Rail shares the `RegionGraph` with roads. This is correct for topology. Unlike HPA* (which degraded on rail because its 512 m chunk size produced few abstract nodes for long inter-station edges), CCH has no fixed chunk size — the contraction hierarchy places high-betweenness rail corridor nodes near the top regardless of edge length. Rail pathfinding with CCH is not a special case.

**What a minimal rail implementation requires.**
A new `VehicleSystem` (separate from `AgentSystem`) that owns train carriages as first-class simulation objects:
- Each carriage is a vehicle that follows a schedule and traverses rail edges.
- Agents board and alight at `NodeType::Station` nodes; the vehicle carries their logical position.
- The agent's `transit_mode` becomes RAIL when boarded; their `current_node` tracks the carriage position, not their own.
- A WAITING FSM state for agents at stations.
- A timetable structure (per-line departure intervals).

This is a significant new subsystem, not an extension of the current agent model.

### 5.4 Ships

**What works.** `TransitType::Ship`, `NodeType::Harbor`, and `TransitFlags::SHIP` are all declared. Simple inter-harbor shipping (harbor A → harbor B direct edge) fits the existing model with no changes: add a SHIP-flagged edge between two `Harbor` nodes and agents with `transit_mode = SHIP` will route through it correctly via the CCH query with `allowed_mask = SHIP | FOOT`.

**What needs changing for city waterways.**
Rivers and canals within the city require navigable waterway edges in `TransitGraph`. These do not yet exist. The water simulation (`simulation/water/`) models the SWE physics of water flow but produces no navigable graph — it is purely a rendering and flooding system. Extracting a navigable graph from the water grid (by thresholding depth) is possible but non-trivial: the water grid is 500 × 500 cells at 40 m/cell resolution, which is too coarse to represent individual waterway channels.

The practical approach is to treat waterways as manually-placed edges in `TransitGraph` (the same way roads are), not derived from the water simulation. Ship edges would have `allowed_types = SHIP`, `EdgeClass::Waterway` (a new class alongside Bridge/Tunnel), and a speed limit reflecting vessel type (~3–8 m/s for canal barges).

The `is_driving` → `transit_mode` migration (§5.1) is the prerequisite: ships need a third mode constant.

### 5.5 Airplanes

**What works.** `TransitType::Air`, `NodeType::Airport`, and `TransitFlags::AIR` are declared. Within a 20 km × 20 km city, airplanes are used for inter-city connections only (inbound immigration, cargo), not intra-city commuting. The existing immigration system already spawns agents at highway border nodes; airports would be analogous border nodes with `NodeType::Airport` and AIR-flagged entry edges.

**What needs changing.** Almost nothing for the basic case. Airports as immigration sources: add `NodeType::Airport` nodes at map edges, connect them with AIR-flagged edges of zero physical length (agents materialise at the airport, then walk or drive). The `allowed_types` filter in HPA* handles the rest.

For realistic flight simulation (runway queuing, takeoff/landing sequences, airspace management): this is entirely outside the scope of the current simulation and would require a dedicated `AirTrafficSystem`. Not recommended before v1.0.

### 5.6 Buses

**What works.** Buses run on road edges — the same `CAR`-flagged `TransitGraph` edges that cars already use. No new edge types, no new graph structure. Because buses occupy road lanes, they participate directly in the IDM car-following system (§2.6): a bus in a lane list is a slow, heavy vehicle with low acceleration, creating realistic queue formation behind it without any special-casing.

**The shared-vehicle problem.** Buses carry many passengers simultaneously, which means the same architectural issue as trains (§5.3) applies: the `AgentSystem` models independent agents, not vehicles that aggregate passengers. A `VehicleSystem` is required. Each bus is a vehicle object that follows its route; agents board and alight at stops, with their logical position tracking the vehicle while aboard.

**Bus stops and Virtual Frontages.** Currently, buildings connect to the road network by physically splitting an edge at the frontage point (`split_for_frontage`), inserting a new `NodeType::Frontage` node into the graph. Applying the same mechanism to every bus stop would pollute the graph with hundreds of extra nodes and edge splits, degrading pathfinding and topology operations.

The correct solution is Virtual Frontages — promoted to a **v0.01 blocker** because the same `split_for_frontage` problem affects buildings today, not only future bus stops. Bus stops are `(EdgeID, t: f32)` T-coordinates on an edge, not physical nodes. A bus arrives at T-coordinate 0.35 on edge 412 and opens its doors; waiting agents within walking distance board without the graph being split. This makes buses a strong motivator for implementing Virtual Frontages before any other transit mode — it is a shared prerequisite for buses, and it eliminates graph pollution from building placement simultaneously.

**Scheduling.** Unlike trains on a closed track, buses on shared roads cannot maintain strict timetables because road congestion affects their speed. The minimum viable model is fixed-interval headways: a bus departs a terminus every N seconds, routes along its fixed path, and stops at each T-coordinate for a dwell time. No global clock synchronisation is needed — each bus is an independent vehicle agent with a departure timer.

**Two-phase routing for passengers.** An agent commuting by bus executes a multi-leg journey:
1. Walk (FOOT path) to the nearest bus stop on the route.
2. Wait (new WAITING FSM state) until a bus arrives.
3. Ride the bus (position tracks the vehicle, mode = `BUS_PASSENGER`).
4. Alight at the stop nearest the destination; walk the last segment.

Phase 1 requires the agent to know which bus stop is relevant — this implies a stop-lookup structure (nearest stop per zone, or a pre-computed walk-to-stop table). Phase 3 requires that the agent's `current_node` tracks the bus vehicle's position while boarded, not its own.

**Congestion interaction.** Buses stopping to board passengers block the lane they occupy for the dwell time. In the IDM model, this manifests naturally: the bus decelerates to zero at the stop, the car behind it also decelerates, and a queue forms. This is realistic and costs nothing extra — it is a consequence of the shared lane list.

**Effort level.** High. Requires `VehicleSystem`, WAITING FSM state, Virtual Frontages, and a route/stop data structure. The movement and pathfinding machinery are largely reusable.

### 5.7 Taxis

**What works.** Taxis drive on road edges exactly like private cars. Movement, pathfinding, IDM car-following, lane offsets, and congestion feedback all apply unchanged. The `transit_mode` migration (§5.1) is the only prerequisite: a taxi passenger needs a mode constant distinct from private car.

**Taxis as agents, not vehicles.** Unlike buses and trains, a taxi carries only one party (1–4 passengers) and serves them exclusively for one trip. This means a taxi does not require a `VehicleSystem` — it can be modelled as a specialised agent within the existing `AgentSystem`. A taxi agent has:
- `transit_mode = TAXI_DRIVER` while empty or en route to pickup
- Standard car movement and pathfinding
- An assigned passenger ID field (or `usize::MAX` when idle)

The passenger agent gets `transit_mode = TAXI_PASSENGER` while riding, and their position tracks the taxi agent's position rather than advancing independently.

**Dispatch.** The core algorithmic challenge for taxis is matching idle taxis to waiting passengers. The naive approach is O(T × P) — for each waiting passenger, scan all idle taxis and pick the nearest. At scale this is unacceptable.

The natural structure is a spatial lookup using the existing 16 m `spatial_node_grid` — or equivalently a `DataGrid<Vec<taxi_id>>` at coarser resolution. When a passenger requests a taxi, the dispatch system queries the spatial index for idle taxis within a search radius, expanding outward until one is found. This is O(1) amortised for uniform taxi distributions, O(r²) worst case where r is the search radius in grid cells.

**FSM extensions.** Two new transit states are needed:
- `WAITING_FOR_TAXI`: passenger has requested a taxi and is standing at their current position. Transitions to `ON_ROAD` (as `TAXI_PASSENGER`) when a taxi arrives.
- `DRIVING_TO_PICKUP`: taxi agent is routing to a passenger's location. Transitions to carrying state on arrival, then routes to the passenger's destination.

These are contained additions to the FSM — no new systems, no VehicleSystem, no schedules.

**Key distinction from buses.** Taxis are a *service* (demand-responsive, flexible, private), not *infrastructure* (fixed routes, shared, scheduled). This makes taxis architecturally much simpler than buses despite superficially similar behaviour. The absence of shared occupancy, fixed stops, and schedules means taxis avoid the three hardest parts of the bus/train architecture.

**Effort level.** Low-to-moderate. The `transit_mode` migration is the prerequisite. After that: a small dispatch spatial lookup, two new FSM states, and a passenger-tracking field on the taxi agent. No new subsystems.

### 5.8 Summary: Compatibility Matrix

| Mode | Graph support | Agent model | Pathfinding | Unique requirements | Effort |
|------|--------------|-------------|------------|---------------------|--------|
| Bicycle | Add `BIKE` bit | `transit_mode` migration | `allowed_mask: u8` (CCH inherits unchanged) | Bike lane edge type; speed table | Low |
| Bus | CAR edges (existing) | `VehicleSystem` + WAITING state | Bus routes on car graph | Virtual Frontages; route/stop structure; schedule headways | High |
| Taxi | CAR edges (existing) | Specialised agent; no `VehicleSystem` | Standard car pathfinding | Dispatch spatial lookup; 2 new FSM states | Low–Moderate |
| Train / Metro | RAIL type exists | `VehicleSystem` + WAITING state | Works for agents; trains need schedule | `VehicleSystem`, timetables, FSM WAITING, transfer logic | High |
| Ship | SHIP type exists | `transit_mode` migration | Works once mode added | `EdgeClass::Waterway`; manual waterway edges | Moderate |
| Airplane | AIR type exists | `transit_mode` migration | Works for simple case | Airport border nodes only | Minimal |

**Shared prerequisite for all modes**: replace `is_driving: Vec<bool>` with `transit_mode: Vec<u8>` and replace `pedestrian: bool` in the pathfinding query with `allowed_mask: u8`. This single change unblocks bicycles, taxis, ships, and airplanes without any further structural work. CCH inherits `allowed_mask` directly — the migration is forward-compatible.

**Shared prerequisite for buses and trains**: `VehicleSystem` (shared vehicle carrying multiple agents) and a WAITING FSM state. Virtual Frontages (v0.01 blocker) is additionally required for buses.

Taxis are the easiest new mode to add after the `transit_mode` migration. Bicycles are second. Buses and trains share the hard infrastructure and are best implemented together after `VehicleSystem` exists.

---

## 6. Implementation Roadmap

### 6.1 Milestone Structure

The project uses a five-tier milestone calibrated against agent count. Each tier has a distinct focus: earlier tiers establish correctness and the architectural contracts that later tiers depend on.

| Milestone | Agent target | Primary focus |
|-----------|-------------|---------------|
| v0.01 | 10k | Correctness and playability. Multi-modal architectural contracts (`transit_mode`, `allowed_mask`) established while the codebase is small. |
| v0.1 | 100k | CCH replaces HPA* (v0.1 blocker); `RegionGraph` rename; adjacency Vec migration. Bridge/tunnel infrastructure, spatial index improvement. |
| v0.2 | ~250k–500k | Three performance walls converge; multi-city region model (statistical background tiles, border crossing); bicycle validates multi-modal foundation. |
| v1.0 | 1M | AoSoA + GPU compute for movement; VehicleSystem for buses/trains; all transit modes; full Agent LoD tiers. |

### 6.2 Why `transit_mode` and `allowed_mask` Belong at v0.01

The `is_driving: Vec<bool>` → `transit_mode: Vec<u8>` migration and the `pedestrian: bool` → `allowed_mask: u8` migration each touch 3–4 files and approximately 20 lines of code when the agent system has exactly two modes. The cost grows non-linearly: every new system added before the migration bakes in the two-mode assumption at a new site. At v0.2, with a parallel tick, flow fields, and IDM all reading `is_driving`, the migration would touch significantly more code and require careful reasoning about atomics and parallel access patterns.

The analogy is a type system migration in a production API: changing a `bool` to an `enum` in week one is a 20-minute refactor; changing it in month six after dozens of call sites exist is a multi-day audit. The migrations are designated v0.01 Goals specifically because the opportunity cost of deferring them compounds quickly.

### 6.3 v0.2 Priority Order and Dependency Structure

The seven v0.2 items are not independent. The following dependency graph determines the only valid implementation order:

```
[v0.1 prerequisite: CCH + adjacency Vec migration already complete]

B16 fix (zone index)
    |
    +-> Parallel tick ---------------------------------> Flow fields
             |
             +-> (unlocks IDM write-back)

B18 fix (double-buffer)  --- independent -----------> (parallel with B16/tick)
IDM car collision        --- independent -----------> (can follow or run alongside)
Multi-city region        --- requires CCH (v0.1) ---> after CCH lands
Bicycle                  --- requires transit_mode + allowed_mask (v0.01 goals) ---> last
```

The longest critical path is **B16 → parallel tick → flow fields**. Reasoning:

1. **B16 first**: the inverted zone-type index must exist before the tick is parallelised because the parallel tick needs `AtomicU32` vacancy counters per building — counters that only make sense if the index (not a linear scan) is the access path. An O(B) atomic-incremented linear scan is not an improvement.

2. **Parallel tick before flow fields**: flow fields reduce per-agent pathfinding work from O(CCH query) to O(1) — but this reduction is only architecturally meaningful once the tick is parallel. In a single-threaded tick, the O(1) flow-field lookup per agent is dominated by the sequential loop itself. After parallelisation, each parallel agent slot is filled with the O(1) lookup, and the throughput improvement is the full factor.

3. **B18 is independent**: the env-grid double-buffer fix can be done in any order relative to the B16/tick work.

4. **Multi-city region after CCH**: the `RegionGraph` rename and CCH data structures are the foundation the region model builds on. Background city statistical model and border crossing spawn/despawn come after CCH is stable.

5. **Bicycle last, deliberately**: bicycle is the integration test for the multi-modal foundation. If bicycle works correctly under parallel load with IDM active on shared road edges, the architectural contracts (transit_mode bitmask routing, allowed_mask filtering, per-lane IDM for mixed vehicle types) are verified. Every subsequent mode (taxi, bus, rail) adds behaviour on top of contracts that are now proven.

### 6.4 v0.2 Performance Wall Analysis

The three walls that converge at ~200k agents:

**Wall 1 — Single-threaded iteration ceiling.** At 1 µs/agent (a rough lower bound given pathfinding), 200k agents = 200 ms/tick. The target is ≤ 33 ms total. Even at 100 ns/agent (optimistic for a full FSM step), 200k agents = 20 ms on one core. Rayon on an 8-core machine gives a theoretical 8× reduction: 200 ms → 25 ms. Without parallelism, the ceiling is roughly 30k–50k agents at acceptable frame rates.

**Wall 2 — O(B) activation scan.** The IDLE→DEPARTING transition scans all buildings to find jobs. At 200k agents, 1% activating per tick = 2,000 activations. If the city has 20k buildings, this is 40M comparisons/tick — approximately 40 ms assuming 1 ns/comparison. The inverted zone index reduces this to 2,000 O(1) lookups. Without this fix, adding agents beyond ~50k in a populated city causes the IDLE scan to dominate over all other work combined.

**Wall 3 — Per-agent routing.** CCH on a medium city graph (10k nodes) costs ~1–5 µs per query vs ~5–50 µs for HPA*. At 200k agents with 1% activating: 2,000 queries/tick × 5 µs = 10 ms. Flow fields reduce this further to one Dijkstra per active zone type (~10–100 active zones), typically 1–10 ms total, with O(1) per-agent lookup. The routing load drops by a factor of ~200k / 100 = 2,000×. CCH is necessary but not sufficient — flow fields remain the key scaling tool for shared destinations.

All three walls must be resolved together. Fixing only one or two leaves the others as the new bottleneck.

---

## 7. Comparative Table: Selected Algorithms

| Problem | Current | Near-term plan | Long-term plan | Notes |
|---------|---------|----------------|----------------|-------|
| Point-to-point routing | HPA* (concrete + abstract A*) | CCH / CRP (v0.1 blocker) | — | CCH gives CH-level query speed with edit-tolerant metric customization; supersedes HPA* entirely |
| Shared-destination routing | Per-agent HPA* | Flow fields (Dijkstra maps) — after CCH lands | Flow fields + CCH hybrid | Flow fields reduce O(A log N) to O(M log N), M << A; CCH handles residual novel-destination queries |
| Spatial edge query | Uniform 512 m grid | — | R-tree (`rstar` crate) | R-tree better for non-uniform edge lengths |
| Spatial node query | Uniform 16 m grid | — | — | Correct; 16 m matches SNAP_TOLERANCE |
| Agent data layout | SoA | — | AoSoA + SIMD | AoSoA enables AVX2 8-wide agent batches |
| Diffusion PDE | Explicit FD, 4-neighbour | Double-buffer (no clone) | — | Algorithm correct; allocation pattern is the issue |
| Agent job selection | O(B) linear scan | O(1) inverted zone index | — | Critical prerequisite for any scale-up |
| Car-following / collision | None (cars pass through) | IDM + per-lane sorted lists | IDM + NaSch fallback | Per-lane 1D lists: O(A) build, O(1) car-ahead lookup; bridges/tunnels require no modification |
| Congestion feedback | None (`current_congestion` never written) | IDM write-back to `Edge::current_congestion` | CCH metric customization on congestion change (O(E)) | Closes routing feedback loop |
| Grade separation (bridges/tunnels) | Not supported (renderer assumes ground-level) | `EdgeClass` field + renderer branch + zoning fix | Bridge cost model, load limits | Simulation geometry already 3D; only renderer and zoning need changes |
| Turn restrictions | `(node, incoming_edge)` state key in A* | — | — | Correct; must be preserved in any pathfinding replacement |
| Node aliasing (merge) | Union-find via HashMap | — | Path-compressed union-find array | Array-based union-find is O(α(N)) vs O(log N) for HashMap |
| Undo stack | VecDeque (O(1) FIFO) | — | — | Correct |
