# Agent Behavior and Rules Documentation

This document outlines the simulation rules governing agent life cycles, daily routines, and transportation behavior within Metrum Rise.

## 1. Life Cycle and Immigration

### Arrival (Immigration)
- **Border Spawning**: New agents (Immigrants) spawn at the city's highway border nodes.
- **Car Arrival**: All immigrants arrive by car.
- **Capacity Cap**: Immigration is capped by the city's total residential capacity. No new immigrants will spawn if the current population exceeds `Total Residential Capacity * 1.1`.

### Finding a Home
- **Homeless State**: Initially, immigrants have no home (`home_building = usize::MAX`).
- **Housing Search**: Immigrants drive into the city center and periodically search for residential buildings with available capacity (currently 6 agents per plot).
- **Settling**: Once a home is found, the agent assigns themselves to it and transitions into the standard daily routine.

---

## 2. Daily Routine (Activities)

Agents cycle through three primary activities based on time and random probability:

1.  **Home (Activity 0)**: Agents stay at home to rest. While at home, their "Happiness" may recover.
2.  **Work (Activity 1)**: Agents travel to Industrial or Commercial buildings to earn "Money". They find jobs periodically when at home.
3.  **Shop (Activity 2)**: Agents travel to Commercial buildings to spend "Money".

---

## 3. Transportation and Transit

### Walking vs. Driving
- **Distance Threshold**: When leaving home, agents decide whether to walk or drive based on a 200m distance threshold to their target.
- **Pathway Rules**:
    - **Pedestrians**: Can traverse all roads and walkways in **both directions**, ignoring vehicle one-way restrictions.
    - **Drivers**: Must strictly follow lane directions and one-way rules.

### Vehicle Persistence
- **Has Car**: If an agent takes a car from home, they "own" that vehicle for the duration of their trip outside.
- **Mandatory Return**: If an agent arrived at a location by car, they **must** return to that car and drive it to their next destination. They cannot abandon cars at work or shops.
- **Home Parking**: Cars are only "removed" from the road network when the agent returns home.

---

## 4. Parking Rules

### Park-and-Walk System
- **Destination Arrival**: Drivers do not drive directly into buildings. They drive to the road node nearest their target building.
- **Edge Parking**: Upon reaching the target node, the driver searches connected road edges for available parking.
- **Capacity**: Every road edge has a parking capacity based on its physical length (1 car per 6 meters per side).
- **Parking Procedure**:
    1. Agent finds a spot on a nearby edge.
    2. `parking_occupied` for that edge is incremented.
    3. The agent "dismounts" (the car disappears), and they become a pedestrian.
    4. They walk the final distance to the building.

### Car Retrieval
- When an agent leaves a building to go elsewhere, they first check if they have a parked car.
- If so, they walk back to the specific edge and coordinate where they parked.
- Once they reach the car, `parking_occupied` is decremented, and they reappear in a vehicle to continue their trip.
