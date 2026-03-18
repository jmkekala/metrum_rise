# Phase 19: Environmental Simulation & Time Engine

Now that the player can build networks and paint zones, the game needs a concept of **Time** to simulate cause-and-effect. 

## Proposed Changes

### [Backend] Global Tick Engine (`simulation/core/time.rs`)
- [NEW] Implement a `TimeSystem` that ticks forward at a fixed timestep (e.g., 1 in-game day per 2 real seconds).
- [NEW] Support Play, Pause, and Fast-Forward controls sent from Godot.

### [Backend] Cellular Automata Grids (`grid/pollution.rs` & `grid/noise.rs`)
- [NEW] **Emission**: During a tick, all painted Industrial zones emit +5 Pollution. Commercial/Highways emit +3 Noise.
- [NEW] **Diffusion (Spread)**: Implement a cellular automata pass (like a Gaussian blur or heat equation) where each cell shares a small percentage of its pollution/noise with its 4 orthogonal neighbors.
- [NEW] **Decay**: Apply a small global decay factor (`value * 0.99`) so pollution dissipates if the source is removed.

### [Backend] Desirability Calculation (`grid/desirability.rs`)
- [NEW] Create a `DesirabilitySystem` that reads the underlying grids.
- [NEW] Formula: `Desirability = Base_Proximity_Value - (Pollution * 2.0) - (Noise * 1.5)`. This dictates where Residential/Commercial buildings are allowed to spawn.

### [Frontend] Overlay Rendering (`terrain.gdshader`)
- [MODIFY] Add Godot hotkeys (e.g., `8` for Pollution, `9` for Noise, `0` for Desirability) to switch the active texture sent from Rust to the terrain shader, creating a heat-map visual feedback system for the player.

## Verification Plan

### Manual Verification
- **Pollution Diffusion Test**: Paint an Industrial zone, press Play. Switch to the Pollution overlay and watch the "Heat" slowly expand outward over time.
- **Decay Test**: Paint over the Industrial zone with 'Erase'. Unpause. Watch the Pollution heat map slowly fade back to zero.
