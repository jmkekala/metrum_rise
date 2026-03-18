# City Scale Traffic & Pathfinding Architecture

This document outlines the planned architecture for scaling Metrum Rise to support Cities: Skylines-level agent simulation (millions of concurrent agents) using cost-aware pathfinding and Rust parallelism.

## 1. Cost-Aware Pathfinding Engine

To simulate realistic traffic, agents will evaluate routes based on "Cost" rather than purely geometric distance.

### Cost Modifiers:
*   **Speed Limits**: Base time cost is calculated as `Distance / Speed Limit`. A long highway is "cheaper" than a short dirt road.
*   **Incline (The Hills)**: A "Slope Cost" penalty will be applied to segments with steep vertical gradients. Heavy vehicles (e.g., cargo trucks) will incur higher penalties for steep grades, forcing them to find longer but flatter bypasses around mountains.
*   **Traffic Density**: The engine will track `current_congestion` on each Edge. As traffic volume increases, dynamic cost multipliers will encourage new agents to seek alternative routes to avoid traffic jams.

## 2. Scaling to Millions: Advanced Routing

Standard A* or Dijkstra algorithms are insufficient for city-scale routing on the main thread. We will utilize the following techniques in the Rust backend:

### Hierarchical Pathfinding (HPA*)
*   Divide the city map into "Districts" (macro-cells).
*   Pre-calculate the cost to move between district borders on a low-resolution graph.
*   Agents formulate a high-level plan across districts, and only run expensive high-resolution pathfinding for the local district they currently occupy.

### Flow Fields / Cost Maps
*   For shared destinations (e.g., "Commute to Industrial Zone" or "Go to commercial"), we will calculate a single gravity-like "Cost Field" originating from the destination.
*   Instead of computing 10,000 individual paths for 10,000 citizens, every citizen simply queries the pre-calculated Flow Field for their current position to determine which direction represents "downhill" towards their destination.

### Multithreaded Execution
*   The `TransitGraph` lives entirely in Rust, independent of the Godot render thread.
*   Utilize Rayon (`par_iter()`) to parallelize pathfinding and cost-update calculations across all available CPU cores, ensuring traffic simulation scales without impacting game framerate.

## 3. Weighted Voronoi Zoning

The road network can organically define the city's zoning topology:
*   Using flood-fill algorithms originating from the road segments, we can assign terrain pixels to their nearest road.
*   **Commercial/Industrial Zoning**: Fostered around roads with high traffic capacity, high connectivity, and high speed limits.
*   **Residential Zoning**: Fostered around low-cost, low-speed roads (e.g., cul-de-sacs and neighborhood streets).

*Drafted: March 2026*


Manual Verification
Unit Tests: Write Rust unit tests to verify a 10km highway is calculated as "cheaper" than a 5km dirt road.
Slope Avoidance Test: Write a unit test where an agent has two routes: a short 41% grade hill, and a long flat bypass. Verify the agent chooses the bypass.
Flow Field Timing: Benchmark the Dijkstra Flow Field generation over a 1000-node graph to ensure it takes < 5ms.


