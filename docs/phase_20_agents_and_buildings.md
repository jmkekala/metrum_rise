# Phase 20: Building Allocator & Instancing

Now that the simulation possesses **Time**, **Zoning**, and **Desirability**, it is time for the city to grow. Phase 20 introduces the `BuildingAllocator`, responsible for fulfilling zoning demand by spawning physical structures on the terrain.

## Proposed Changes

### [Backend] City Demographics & Demand (`simulation/economy/demand.rs`)
- [NEW] Create a `DemandSystem` to track global demand for R, C, and I zones. 
- [NEW] Demand is simple for now: +10 Demand to all zones globally. As buildings spawn, they consume this demand.

### [Backend] The Building Allocator (`simulation/buildings/allocator.rs`)
- [NEW] Introduce a `Building` struct containing its grid position, `ZoneType`, and level.
- [NEW] In the global network `simulate_tick()`, the `BuildingAllocator` will scan random sections of the `ZoningSystem`.
- [NEW] **Spawn Condition**: If a cell is currently Zoned, has NO existing building, and the local `Desirability` > 50, a Building is spawned *if* there is Demand for its type.
- [NEW] **Mixed Zones**: For `ZoneType::Mixed`, the allocator will check if there is *either* Residential OR Commercial demand. It spawns a specific `Mixed-Use` building type that consumes a bit of both demands, and serves as both housing and jobs!
- [NEW] **Throttling**: Limit spawning to ~5 buildings per tick to create an organic "growing" effect rather than popping the whole city into existence instantly.

### [Backend] Godot Integration (`nodes/simulation_node.rs`)
- [NEW] Expose a function `get_building_transforms(zone_type: u8) -> PackedFloat32Array`.
- [NEW] The supported types will be: Residential (1), Commercial (2), Industrial (3), and Mixed (4). This returns a flattened array of 3D Transforms (Position, Rotation, Scale).

### [Frontend] MultiMesh Instancing (`godot/scenes/Main.tscn` & `godot/scripts/buildings.gd`)
- [NEW] In games like Cities: Skylines, there are tens of thousands of buildings. Spawning individual Godot `Node3D` scenes for each house will instantly crash the framerate.
- [NEW] Create a new `MultiMeshInstance3D` node in Godot for Residential, Commercial, and Industrial.
- [NEW] **MultiMesh Pipeline**: Every second in Godot `_process()`, call `get_building_transforms`. Take the raw array and pipe it into `MultiMesh.buffer`, rendering thousands of low-poly houses in a single draw call.

## Verification Plan

### Manual Verification
1. **Spawning**: Zone a block of Residential next to a road. Press Play. Watch as small boxes (or primitive house models) randomly pop up over time across the green zone until it is full.
2. **Desirability Block**: Paint Industrial right next to Residential. Let the pollution spread. Verify that the polluted Residential grass refuses to spawn any new houses because the `Desirability` drops below 50.
