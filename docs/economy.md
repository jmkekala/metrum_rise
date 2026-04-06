# Metrum Rise — Economy Design Spec

## Architectural Philosophy: The "NiFi" Model
Metrum Rise implements a **Data-Flow Economy** based on the principles of **Apache NiFi**. The economy is not calculated as a global balance, but as a series of distributed **Processors** connected by **Physical Pipes**.

### 1. Processors (Entities)
Each building and agent acts as a stateful processor:
- **Agents:** Consume "Needs Fulfillment" → Produce "Labor" + "Cash Flow."
- **Industrial:** Consumes "Labor + Raw Materials" → Produces "Goods."
- **Commercial:** Consumes "Goods" → Produces "Needs Fulfillment" (Sold to Agents).

### 3. Visual Economy Interface (The Management Shell)
The "NiFi" vision is primarily realized through a **Godot-based Node Editor (`GraphEdit`)**:
- **Canvas View:** A full-city management layer where economic relationships are "wired" by the player.
- **Node-Based Policy:** Instead of code-side constants, the player places **Controller Nodes** (e.g. "Tax Rate", "Freight Priority") and draws connections between **Asset Categories** (e.g. "Industrial:Grain" → "Commercial:Bakery").
- **Real-Time Command-Bus:** Adjusting a node in the UI sends a `SimCommand` to the Rust background thread. This immediately updates the weights inside the `AgentSystem` utility scorers and building production logic across all 1,000,000 agents.
- **District Scoping:** Connections can be **Global** (All Residential) or **Scoped** (Only District A) to provide granular control over the logistics mesh.

### 2. Controllers (The Brains)
Global "valves" that govern the flows between different entity types. They do not hold state—they calculate the **Delta** between needs and supply.
- **LaborController:** Manages the transfer of money from buildings to agents based on vacancy and skills.
- **MarketController:** Sets prices based on the balance of local stock vs. agent demand. High demand exerts **Back-pressure** on the system, signaling higher prices and faster production.

---

## Multimodal Physical Logistics
Metrum Rise moves away from "abstract" goods flow. All economic exchange requires a **Carrier** (Physical Agent) traversing the **RegionGraph**.

### 1. Carriers (The FlowFiles)
- **Trucks (Default):** The primary last-mile delivery agent.
- **Trains/Ships:** Bulk transporters for high-volume, long-distance trade.
- **Airplanes:** Low-volume, high-value couriers.

### 2. Unit Compression (Scaling Strategy)
To support **1,000,000 agents**, logistics must be hierarchical:
- **The "Container" Rule:** One 1,000-ton cargo ship is **1 agent** in the `AgentSystem` SoA. 
- **The "Unpacking":** Large carriers (Ships/Trains) terminate at **Terminals** (Harbors/Depots), where they are "unpacked" into smaller agents (Trucks) for city delivery. 

### 3. Traffic-Economy Feedback Loop
Because and delivery is physical:
- **Congestion = Inflation:** Traffic jams delay trucks → Shops run out of stock → Prices rise → Agent happiness falls.
- **Logistics as Gameplay:** The player's success depends on building a robust transport network that avoids these physical bottlenecks.

---

## Agent Needs: Simplified Hierarchy (Shelter & Rest)
Agents are driven by a simplified two-level utility scoring model. This minimizes the "Logistics Noise" of 1,000,000 individual shopping trips, prioritizing **Building-to-Building Logistics** over individual agent errands.

| Level | Need Type | Simulation Driver | Agent Action | Logistic Driver |
| :--- | :--- | :--- | :--- | :--- |
| **1** | **Survival** | Shelter & Rest | `IDLE (At Home)` | **Truck Agents** delivering goods to Residential Buildings (Building Stock). |
| **2** | **Stability** | Income (Money) | `TRANSIT (To Work)` | Factories/Commercial requiring **Labor Agents**. |
| **3** | **Quality** | Social & Leisure | `TRANSIT (To Park/POI)` | Optional (Low-frequency) trips for entertainment. |

### The "Residential Supply" Model:
- **Consolidated Logistics:** Instead of 1,000,000 agents buying food individually, a much smaller fleet of **Trucks** delivers goods directly to **Residential Buildings**.
- **Building Inventory:** Residential buildings gain `stock: f32`. Agents "consume" from their residence's internal stock while idle at home to satisfy happiness/rest requirements.
- **Back-pressure:** If a building's stock runs out (due to a traffic jam or poor wiring in the NiFi shell), residents lose happiness. This creates a critical "Logistics Challenge" for the player.

---

## Implementation Phases
1. **Phase 1 (Truck-Based):** Initial economy. Buildings produce stock → Spawn truck agent → Deliver to shop.
2. **Phase 2 (Visual Editor):** Godot `GraphEdit` integration to set global/local production and consumption flows.
3. **Phase 3 (Transfer Terminals):** Harbors and Depots allow "bulk-to-truck" transfers.
4. **Phase 4 (Global Region View):** Statistical modeling of inter-city trade via border nodes.
